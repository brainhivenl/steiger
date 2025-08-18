use std::fmt::Debug;
use std::path::Path;

use oci_distribution::{
    client::ImageLayer,
    manifest::{OciImageIndex, OciImageManifest, Platform},
};
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("failed to (de)serialize: {0}")]
    Serde(#[from] serde_json::Error),
}

pub struct Image {
    pub manifest: OciImageManifest,
    pub platform: Option<Platform>,
    pub layers: Vec<ImageLayer>,
}

impl Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("manifest", &self.manifest)
            .field("platform", &self.platform)
            .field("layer_count", &self.layers.len())
            .finish()
    }
}

async fn parse<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T, ImageError> {
    Ok(serde_json::from_slice(&tokio::fs::read(path).await?)?)
}

fn split_algo_hash(digest: &str) -> (&str, &str) {
    digest.split_once(':').unwrap_or_default()
}

pub async fn load_from_path(dir: impl AsRef<Path>) -> Result<Vec<Image>, ImageError> {
    let dir = dir.as_ref();
    let index: OciImageIndex = parse(dir.join("index.json")).await?;
    let mut images = vec![];
    let blob_dir = dir.join("blobs");

    for entry in index.manifests {
        let (alg, hash) = split_algo_hash(&entry.digest);
        let manifest: OciImageManifest = parse(blob_dir.join(alg).join(hash)).await?;
        let mut layers = vec![];

        for layer in manifest.layers.iter() {
            let (alg, hash) = split_algo_hash(&layer.digest);
            let data = tokio::fs::read(blob_dir.join(alg).join(hash)).await?;

            layers.push(ImageLayer::new(
                data,
                layer.media_type.clone(),
                layer.annotations.clone(),
            ));
        }

        images.push(Image {
            manifest,
            layers,
            platform: entry.platform,
        });
    }

    Ok(images)
}
