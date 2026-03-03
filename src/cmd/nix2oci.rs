use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Parser;
use miette::Diagnostic;
use oci_client::config::{Architecture, Config as OciConfig, ConfigFile as OciConfigFile, Os};
use oci_client::manifest::{
    IMAGE_CONFIG_MEDIA_TYPE, IMAGE_LAYER_MEDIA_TYPE, IMAGE_MANIFEST_MEDIA_TYPE, ImageIndexEntry,
    OCI_IMAGE_INDEX_MEDIA_TYPE, OCI_IMAGE_MEDIA_TYPE, OciDescriptor, OciImageIndex,
    OciImageManifest,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use sha2::Sha256;
use sha2::digest::{Digest, Output};

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum Error {
    #[error("failed to create temporary file/directory: {0}")]
    Temp(#[from] async_tempfile::Error),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("tar command failed: {stderr}")]
    Tar { stderr: String },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Parser)]
pub struct Opts {
    #[arg(long)]
    name: String,
    #[arg(long = "closure")]
    closures: Vec<PathBuf>,
    #[arg(long = "link-path")]
    paths_to_link: Vec<PathBuf>,
    #[arg(long)]
    out_path: PathBuf,
    #[arg(long, default_value = "latest")]
    tag: String,
    #[arg(long)]
    config: String,
    #[arg(long)]
    os: String,
    #[arg(long)]
    arch: String,
}

fn write_blob(bytes: &[u8], blob_dir: &Path) -> Result<(Output<Sha256>, usize)> {
    let mut hasher = Sha256::default();
    hasher.update(bytes);
    let sha256 = hasher.finalize();

    let blob_path = blob_dir.join(hex::encode(sha256));
    fs::write(blob_path, bytes)?;

    Ok((sha256, bytes.len()))
}

fn run_tar(
    blob_dir: &Path,
    configure: impl FnOnce(&mut Command) -> &mut Command,
) -> Result<(Output<Sha256>, usize)> {
    let temp_file_path = blob_dir.join("temp");
    let temp_file = fs::File::create_new(&temp_file_path)?;

    let mut cmd = Command::new("tar");
    cmd.stdout(temp_file)
        .arg("--create")
        .arg("--hard-dereference")
        .arg("--sort=name")
        .arg("--owner=0")
        .arg("--group=0")
        .arg("--numeric-owner");

    configure(&mut cmd);

    let output = cmd.output()?;
    if !output.status.success() {
        let _ = fs::remove_file(&temp_file_path);
        return Err(Error::Tar {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let content = fs::read(&temp_file_path)?;
    let result = write_blob(&content, blob_dir)?;
    fs::remove_file(temp_file_path)?;

    Ok(result)
}

fn write_layer_from_paths(paths: &[PathBuf], blob_dir: &Path) -> Result<(Output<Sha256>, usize)> {
    run_tar(blob_dir, |cmd| cmd.args(paths))
}

fn write_layer_from_dir(path: &Path, blob_dir: &Path) -> Result<(Output<Sha256>, usize)> {
    run_tar(blob_dir, |cmd| {
        cmd.arg(format!("--directory={}", path.to_string_lossy()))
            .arg(".")
    })
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Recursively merge `src` into `dst` via symlinks, handling existing entries.
fn symlink_farm(src: &Path, dst: &Path) -> io::Result<()> {
    let src_meta = fs::symlink_metadata(src)?;

    if !src_meta.is_dir() {
        prepare_dst(dst)?;
        let src = if src_meta.is_symlink() {
            Cow::Owned(fs::read_link(src)?)
        } else {
            Cow::Borrowed(src)
        };
        std::os::unix::fs::symlink(src, dst)?;
        return Ok(());
    }

    if let Ok(dst_meta) = dst.symlink_metadata() {
        if !dst_meta.is_dir() {
            fs::remove_file(dst)?;
        }
    }
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        symlink_farm(&entry.path(), &dst.join(entry.file_name()))?;
    }
    Ok(())
}

fn prepare_dst(dst: &Path) -> io::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Ok(meta) = dst.symlink_metadata() {
        if meta.is_dir() {
            fs::remove_dir_all(dst)?;
        } else {
            fs::remove_file(dst)?;
        }
    }
    Ok(())
}

fn fmt_sha_id(sha256: Output<Sha256>) -> String {
    format!("sha256:{}", hex::encode(sha256))
}

fn deserialize_str<T: DeserializeOwned>(value: String) -> Result<T> {
    let value = serde_json::Value::String(value);
    Ok(serde_json::from_value(value)?)
}

pub fn run(opts: Opts) -> Result<()> {
    let architecture = deserialize_str::<Architecture>(opts.arch)?;
    let os = deserialize_str::<Os>(opts.os)?;
    let temp_dir = PathBuf::from("/tmp/nix2oci");
    let blob_dir = temp_dir.join("blobs/sha256");
    fs::create_dir_all(&blob_dir)?;

    let mut layer_meta = vec![];
    let mut all_drv_paths = vec![];

    for closure_path in &opts.closures {
        let closure_content = fs::read_to_string(closure_path)?;
        let paths: Vec<PathBuf> = closure_content
            .lines()
            .map(PathBuf::from)
            .filter(|path| !all_drv_paths.contains(path))
            .collect();

        if !paths.is_empty() {
            let meta = write_layer_from_paths(&paths, &blob_dir)?;
            layer_meta.push(meta);
            all_drv_paths.extend(paths);
        }
    }

    if !opts.paths_to_link.is_empty() {
        let layer_path = temp_dir.join("link-layer");
        fs::create_dir_all(&layer_path)?;

        for drv_path in &all_drv_paths {
            for path_to_link in &opts.paths_to_link {
                let path_to_link = path_to_link.strip_prefix("/").unwrap_or(path_to_link);
                let src = drv_path.join(path_to_link);
                let dst = layer_path.join(path_to_link);
                if src.symlink_metadata().is_ok() {
                    symlink_farm(&src, &dst)?;
                }
            }
        }

        let meta = write_layer_from_dir(&layer_path, &blob_dir)?;
        layer_meta.push(meta);
        fs::remove_dir_all(&layer_path)?;
    }

    let user_config = serde_json::from_str::<OciConfig>(&opts.config)?;
    let config = OciConfigFile {
        created: None,
        author: None,
        architecture,
        os,
        config: Some(user_config),
        rootfs: oci_client::config::Rootfs {
            r#type: "layers".to_string(),
            diff_ids: layer_meta
                .iter()
                .map(|(sha256, _)| fmt_sha_id(*sha256))
                .collect(),
        },
        history: None,
    };

    let config_bytes = serde_json::to_vec(&config)?;
    let (config_hash, config_size) = write_blob(&config_bytes, &blob_dir)?;

    let manifest = OciImageManifest {
        schema_version: 2,
        media_type: Some(IMAGE_MANIFEST_MEDIA_TYPE.into()),
        config: OciDescriptor {
            media_type: IMAGE_CONFIG_MEDIA_TYPE.into(),
            digest: fmt_sha_id(config_hash),
            size: config_size as i64,
            urls: None,
            annotations: None,
        },
        layers: layer_meta
            .iter()
            .map(|(sha256, size)| OciDescriptor {
                media_type: IMAGE_LAYER_MEDIA_TYPE.into(),
                digest: fmt_sha_id(*sha256),
                size: *size as i64,
                urls: None,
                annotations: None,
            })
            .collect(),
        subject: None,
        artifact_type: None,
        annotations: None,
    };

    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let (manifest_hash, manifest_size) = write_blob(&manifest_bytes, &blob_dir)?;

    let index = OciImageIndex {
        schema_version: 2,
        media_type: Some(OCI_IMAGE_INDEX_MEDIA_TYPE.into()),
        manifests: vec![ImageIndexEntry {
            media_type: OCI_IMAGE_MEDIA_TYPE.into(),
            digest: fmt_sha_id(manifest_hash),
            size: manifest_size as i64,
            platform: None,
            annotations: Some(
                [(
                    "org.opencontainers.image.ref.name".to_string(),
                    format!("{}:{}", &opts.name, &opts.tag),
                )]
                .into_iter()
                .collect(),
            ),
        }],
        artifact_type: None,
        annotations: None,
    };

    let index_bytes = serde_json::to_vec(&index)?;
    fs::write(temp_dir.join("index.json"), &index_bytes)?;

    let image_layout = json!({ "imageLayoutVersion": "1.0.0" });

    let image_layout_bytes = serde_json::to_vec(&image_layout)?;
    fs::write(temp_dir.join("oci-layout"), &image_layout_bytes)?;

    copy_dir_all(&temp_dir, &opts.out_path)?;
    fs::remove_dir_all(&temp_dir)?;

    Ok(())
}
