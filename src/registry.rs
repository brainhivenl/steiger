use std::sync::Arc;

use oci_distribution::{
    Client, Reference, RegistryOperation, errors::OciDistributionError, secrets::RegistryAuth,
};
use prodash::{
    messages::MessageLevel,
    tree::{Item, Root},
};

use crate::image::Image;

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("failed to parse reference: {0}")]
    Parse(#[from] oci_distribution::ParseError),
    #[error("failed to push image: {0}")]
    Oci(#[from] OciDistributionError),
}

pub struct ImageOutput {
    pub config_url: String,
    pub manifest_url: String,
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

    pub async fn push(
        &mut self,
        repo: &str,
        artifact: &str,
        image: Image,
    ) -> Result<ImageOutput, PushError> {
        let reference = Reference::try_from(format!("{repo}/{artifact}:latest"))?;
        let progress = self.progress.add_child(format!("pushing {artifact}"));

        progress.message(MessageLevel::Info, format!("pushing image"));

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

        progress.message(MessageLevel::Success, format!("image pushed"));

        Ok(ImageOutput {
            config_url: response.config_url,
            manifest_url: response.manifest_url,
            reference,
        })
    }
}
