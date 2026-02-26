use std::borrow::Cow;
use std::fs;
use std::io;
use std::os::unix;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use anyhow::{Result, anyhow};
use argh::FromArgs;
use oci_spec::image::{
    Arch, Config, DescriptorBuilder, Digest, ImageConfigurationBuilder, ImageIndexBuilder,
    ImageManifestBuilder, MediaType, Os, RootFsBuilder,
};
use serde_json::json;
use sha2::Sha256;
use sha2::digest::{Digest as _, Output};

#[derive(Debug, FromArgs)]
pub struct Opts {
    #[argh(option)]
    name: String,
    #[argh(option, long = "closure")]
    closures: Vec<PathBuf>,
    #[argh(option, long = "link-path")]
    paths_to_link: Vec<PathBuf>,
    #[argh(option)]
    out_path: PathBuf,
    #[argh(option)]
    tag: Option<String>,
    #[argh(option)]
    config: String,
    #[argh(option)]
    os: String,
    #[argh(option)]
    arch: String,
}

fn fmt_sha256(sha256: Output<Sha256>) -> String {
    format!("sha256:{}", hex::encode(sha256))
}

trait ToDigest {
    fn to_digest(&self) -> Result<Digest>;
}

impl ToDigest for Output<Sha256> {
    fn to_digest(&self) -> Result<Digest> {
        let id = fmt_sha256(*self);
        Ok(Digest::from_str(&id).unwrap())
    }
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
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(anyhow!("tar failed: {stderr}"));
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
fn symlink_all(src: &Path, dst: &Path) -> io::Result<()> {
    let src_meta = fs::symlink_metadata(src)?;

    if !src_meta.is_dir() {
        prepare_dst(dst)?;
        let src = if src_meta.is_symlink() {
            Cow::Owned(fs::read_link(src)?)
        } else {
            Cow::Borrowed(src)
        };
        unix::fs::symlink(src, dst)?;
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
        symlink_all(&entry.path(), &dst.join(entry.file_name()))?;
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

pub fn main() -> Result<()> {
    let opts: Opts = argh::from_env();
    let architecture = Arch::from(opts.arch.as_str());
    let os = Os::from(opts.os.as_str());

    let temp_dir = Path::new(concat!("/tmp/", env!("CARGO_PKG_NAME")));
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
                    symlink_all(&src, &dst)?;
                }
            }
        }

        let meta = write_layer_from_dir(&layer_path, &blob_dir)?;
        layer_meta.push(meta);
        fs::remove_dir_all(&layer_path)?;
    }

    let user_config = serde_json::from_str::<Config>(&opts.config)?;
    let config = ImageConfigurationBuilder::default()
        .architecture(architecture)
        .os(os)
        .config(user_config)
        .rootfs(
            RootFsBuilder::default()
                .typ("layers")
                .diff_ids(
                    layer_meta
                        .iter()
                        .map(|(sha256, _)| fmt_sha256(*sha256))
                        .collect::<Vec<_>>(),
                )
                .build()?,
        )
        .build()?;

    let config_bytes = serde_json::to_vec(&config)?;
    let (config_hash, config_size) = write_blob(&config_bytes, &blob_dir)?;

    let manifest = ImageManifestBuilder::default()
        .schema_version(2u32)
        .media_type(MediaType::ImageManifest)
        .config(
            DescriptorBuilder::default()
                .media_type(MediaType::ImageConfig)
                .digest(config_hash.to_digest()?)
                .size(config_size as u64)
                .build()?,
        )
        .layers(
            layer_meta
                .iter()
                .map(|(sha256, size)| {
                    Ok(DescriptorBuilder::default()
                        .media_type(MediaType::ImageLayer)
                        .digest(sha256.to_digest()?)
                        .size(*size as u64)
                        .build()?)
                })
                .collect::<Result<Vec<_>>>()?,
        )
        .build()?;

    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let (manifest_hash, manifest_size) = write_blob(&manifest_bytes, &blob_dir)?;

    let index = ImageIndexBuilder::default()
        .schema_version(2u32)
        .media_type(MediaType::ImageIndex)
        .manifests(vec![
            DescriptorBuilder::default()
                .media_type(MediaType::ImageManifest)
                .digest(manifest_hash.to_digest()?)
                .size(manifest_size as u64)
                .annotations([(
                    "org.opencontainers.image.ref.name".to_string(),
                    format!(
                        "{}:{}",
                        &opts.name,
                        opts.tag.as_ref().map(String::as_str).unwrap_or("latest")
                    ),
                )])
                .build()?,
        ])
        .build()?;

    let index_bytes = serde_json::to_vec(&index)?;
    fs::write(temp_dir.join("index.json"), &index_bytes)?;

    let image_layout = json!({ "imageLayoutVersion": "1.0.0" });
    let image_layout_bytes = serde_json::to_vec(&image_layout)?;
    fs::write(temp_dir.join("oci-layout"), &image_layout_bytes)?;

    copy_dir_all(&temp_dir, &opts.out_path)?;
    fs::remove_dir_all(&temp_dir)?;

    Ok(())
}
