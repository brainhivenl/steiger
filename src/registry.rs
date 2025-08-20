use oci_distribution::{
    Client, Reference,
    client::PushResponse,
    errors::{OciDistributionError, OciErrorCode},
    secrets::RegistryAuth,
};
use prodash::{messages::MessageLevel, tree::Item};

use crate::image::Image;

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("failed to push image")]
    Oci(#[from] OciDistributionError),
}

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

        progress.message(MessageLevel::Info, "pushing image");

        let response = self
            .client
            .push(
                image_ref,
                &image.layers,
                image.config,
                &self.auth,
                Some(image.manifest),
            )
            .await?;

        progress.message(MessageLevel::Success, "image pushed");

        Ok(Some(response))
    }
}
