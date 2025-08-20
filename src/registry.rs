use std::sync::Arc;

use oci_distribution::{
    Client, Reference, client::PushResponse, errors::OciDistributionError, secrets::RegistryAuth,
};
use prodash::{messages::MessageLevel, tree::Root};

use crate::image::Image;

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("failed to parse reference")]
    Parse(#[from] oci_distribution::ParseError),
    #[error("failed to push image")]
    Oci(#[from] OciDistributionError),
}

pub struct ImageOutput {
    pub response: Option<PushResponse>,
    pub reference: Reference,
}

pub struct Registry {
    auth: RegistryAuth,
    client: Client,
    progress: Arc<Root>,
}

impl Registry {
    pub fn new(progress: Arc<Root>, client: Client, auth: RegistryAuth) -> Self {
        Self {
            auth,
            client,
            progress,
        }
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
            Err(OciDistributionError::ImageManifestNotFoundError(_)) => {
                // If the manifest is not found, we assume the image does not exist
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    pub async fn push(
        &self,
        repo: &str,
        artifact: &str,
        image: Image,
    ) -> Result<ImageOutput, PushError> {
        let reference = Reference::try_from(format!("{repo}/{artifact}:latest"))?;
        let progress = self.progress.add_child(format!("{artifact} › push"));

        if let Some(digest) = self.try_resolve_digest(&reference).await? {
            progress.message(MessageLevel::Info, "image already exists, skipping push");

            return Ok(ImageOutput {
                response: None,
                reference: Reference::with_digest(
                    reference.registry().to_string(),
                    reference.repository().to_string(),
                    digest,
                ),
            });
        }

        progress.message(MessageLevel::Info, "pushing image");

        let response = self
            .client
            .push(
                &reference,
                &image.layers,
                image.config,
                &self.auth,
                Some(image.manifest),
            )
            .await?;

        progress.message(MessageLevel::Success, "image pushed");

        Ok(ImageOutput {
            reference,
            response: Some(response),
        })
    }
}
