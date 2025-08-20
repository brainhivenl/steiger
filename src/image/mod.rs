use std::fmt::Debug;
use std::path::Path;

use oci_client::{
    client::{Config, ImageLayer},
    manifest::{OciImageIndex, OciImageManifest, Platform},
};
use olpc_cjson::CanonicalFormatter;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::image::blob_store::BlobStore;

mod blob_store;

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("IO error")]
    IO(#[from] std::io::Error),
    #[error("failed to (de)serialize")]
    Serde(#[from] serde_json::Error),
}

pub struct Image {
    pub digest: String,
    pub config: Config,
    pub manifest: OciImageManifest,
    pub platform: Option<Platform>,
    pub layers: Vec<ImageLayer>,
}

impl Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("digest", &self.digest)
            .field("manifest", &self.manifest)
            .field("platform", &self.platform)
            .field("layer_count", &self.layers.len())
            .finish()
    }
}

fn compute_digest(manifest: &OciImageManifest) -> Result<String, serde_json::Error> {
    let mut body = vec![];
    let mut ser = serde_json::Serializer::with_formatter(&mut body, CanonicalFormatter::new());
    manifest.serialize(&mut ser)?;

    let mut hasher = Sha256::default();
    hasher.update(body);

    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

pub async fn load_from_path(dir: impl AsRef<Path>) -> Result<Vec<Image>, ImageError> {
    let dir = dir.as_ref();
    let store = BlobStore::new(dir.to_path_buf());
    let index =
        serde_json::from_slice::<OciImageIndex>(&tokio::fs::read(dir.join("index.json")).await?)?;
    let mut images = vec![];

    for entry in index.manifests {
        let manifest =
            serde_json::from_slice::<OciImageManifest>(&store.read_blob(&entry.digest).await?)?;
        let mut layers = vec![];

        for layer in manifest.layers.iter() {
            let data = store.read_blob(&layer.digest).await?;

            layers.push(ImageLayer::new(
                data,
                layer.media_type.clone(),
                layer.annotations.clone(),
            ));
        }

        let digest = compute_digest(&manifest)?;
        let data = store.read_blob(&manifest.config.digest).await?;
        let config = Config {
            data,
            media_type: manifest.config.media_type.clone(),
            annotations: manifest.config.annotations.clone(),
        };

        images.push(Image {
            manifest,
            layers,
            platform: entry.platform,
            config,
            digest,
        });
    }

    Ok(images)
}
