use futures::{StreamExt, TryStreamExt, future, stream};
use miette::Diagnostic;
use oci_client::{
    Client, Reference,
    client::PushResponse,
    errors::{OciDistributionError, OciErrorCode},
    secrets::RegistryAuth,
};
use prodash::{messages::MessageLevel, tree::Item};

use crate::image::Image;

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum PushError {
    #[error("failed to push image")]
    Oci(#[from] OciDistributionError),
}

#[derive(Clone)]
pub struct Registry {
    auth: RegistryAuth,
    client: Client,
}

impl Registry {
    pub fn new(client: Client, auth: RegistryAuth) -> Self {
        Self { auth, client }
    }

    async fn try_resolve_digest(
        &self,
        reference: &Reference,
    ) -> Result<Option<String>, OciDistributionError> {
        match self
            .client
            .fetch_manifest_digest(reference, &self.auth)
            .await
        {
            Ok(digest) => Ok(Some(digest)),
            // If the manifest is not found, we assume the image does not exist
            Err(OciDistributionError::ImageManifestNotFoundError(_)) => Ok(None),
            // If the manifest is unknown, we assume the image does not exist
            Err(OciDistributionError::RegistryError { envelope, .. }) if matches!(envelope.errors.first(), Some(e) if e.code == OciErrorCode::ManifestUnknown) => {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    pub async fn push(
        &mut self,
        progress: Item,
        image_ref: &Reference,
        image: Image,
    ) -> Result<Option<PushResponse>, PushError> {
        if let Some(digest) = self.try_resolve_digest(image_ref).await? {
            // If the digest matches the image's digest, we can skip pushing
            if digest == image.digest {
                progress.message(MessageLevel::Info, "image already exists, skipping push");
                return Ok(None);
            }
        }

        progress.init(Some(image.layers.len()), None);
        progress.message(MessageLevel::Info, "pushing image");

        self.client
            .store_auth_if_needed(image_ref.resolve_registry(), &self.auth)
            .await;

        // Push blobs with cache
        stream::iter(&image.layers)
            .map(|layer| {
                let client = self.client.clone();
                let layer_desc = &image.manifest.layers;
                let progress = &progress;

                async move {
                    let digest = layer.sha256_digest();
                    let desc = layer_desc.iter().find(|l| l.digest == digest).unwrap();

                    match client
                        .pull_blob_stream_partial(image_ref, desc, 0, Some(1))
                        .await
                    {
                        Ok(_) => {
                            progress.inc();
                            Ok(())
                        }
                        Err(OciDistributionError::ServerError { code: 404, .. }) => {
                            client.push_blob(image_ref, &layer.data, &digest).await?;
                            progress.inc();
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                }
            })
            .boxed() // Workaround to rustc issue https://github.com/rust-lang/rust/issues/104382
            .buffer_unordered(16)
            .try_for_each(future::ok::<(), OciDistributionError>)
            .await?;

        let config_url = self
            .client
            .push_blob(image_ref, &image.config.data, &image.manifest.config.digest)
            .await?;
        let manifest_url = self
            .client
            .push_manifest(image_ref, &image.manifest.into())
            .await?;

        progress.message(MessageLevel::Success, "image pushed");

        Ok(Some(PushResponse {
            config_url,
            manifest_url,
        }))
    }
}
