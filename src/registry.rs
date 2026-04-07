use std::time::Duration;

use docker_credential::{CredentialRetrievalError, DockerCredential};
use futures::{StreamExt, TryStreamExt, stream};
use miette::Diagnostic;
use oci_client::{
    Client, Reference,
    client::{ClientConfig, ClientProtocol, PushResponse},
    errors::{OciDistributionError, OciErrorCode},
    secrets::RegistryAuth,
};
use prodash::tree::Item;

use crate::image::Image;

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum PushError {
    #[error("failed to push image")]
    Oci(#[from] OciDistributionError),
}

fn parse_host(repo: &str) -> &str {
    repo.split('/').next().unwrap_or_default()
}

pub fn load_credentials(repo: &str) -> Result<RegistryAuth, CredentialRetrievalError> {
    match docker_credential::get_credential(parse_host(repo)) {
        Ok(DockerCredential::IdentityToken(_)) => unimplemented!(),
        Ok(DockerCredential::UsernamePassword(user, pass)) => Ok(RegistryAuth::Basic(user, pass)),
        Err(
            CredentialRetrievalError::HelperFailure { .. }
            | CredentialRetrievalError::ConfigNotFound
            | CredentialRetrievalError::NoCredentialConfigured,
        ) => Ok(RegistryAuth::Anonymous),
        Err(e) => Err(e),
    }
}

#[derive(Clone)]
pub struct Registry {
    client: Client,
    use_monolithic_push: bool,
    auth: RegistryAuth,
}

impl Registry {
    pub fn with_config(
        auth: RegistryAuth,
        insecure_registies: &[String],
        use_monolithic_push: bool,
    ) -> Self {
        let config = ClientConfig {
            protocol: ClientProtocol::HttpsExcept(
                [insecure_registies, &["localhost".to_string()]].concat(),
            ),
            use_monolithic_push,
            ..ClientConfig::default()
        };

        Self {
            client: Client::new(config),
            use_monolithic_push,
            auth,
        }
    }

    async fn try_resolve_digest(
        &self,
        auth: &RegistryAuth,
        reference: &Reference,
    ) -> Result<Option<String>, OciDistributionError> {
        match self.client.fetch_manifest_digest(reference, auth).await {
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

    async fn monolithic_push(
        &mut self,
        mut progress: Item,
        image_ref: &Reference,
        image: Image,
    ) -> Result<Option<PushResponse>, PushError> {
        progress.init(None, None);
        progress.info("pushing image");

        let image_urls = self
            .client
            .push(
                image_ref,
                &image.layers,
                image.config,
                &self.auth,
                image.manifest.into(),
            )
            .await?;

        progress.done("image pushed");

        Ok(Some(PushResponse {
            config_url: image_urls.config_url,
            manifest_url: image_urls.manifest_url,
        }))
    }

    async fn layered_push(
        &mut self,
        mut progress: Item,
        image_ref: &Reference,
        image: Image,
    ) -> Result<Option<PushResponse>, PushError> {
        progress.init(Some(image.layers.len()), None);
        progress.info("pushing image");

        // Push blobs with cache
        stream::iter(image.layers)
            .map(|layer| {
                let client = self.client.clone();
                let progress = &progress;

                async move {
                    let digest = layer.sha256_digest();

                    if !client.blob_exists(image_ref, &digest).await? {
                        let mut last_err = None;

                        for i in 1..5 {
                            match client
                                .push_blob(image_ref, layer.data.clone(), &digest)
                                .await
                            {
                                Ok(_) => {
                                    last_err = None;
                                    break;
                                }
                                // Retry on digest mismatch (400) and invalid range (416) errors.
                                // Root cause unknown; we should probably look into this but retry is safe for now.
                                // Retrying on 5xx server errors is also acceptable.
                                Err(
                                    e @ OciDistributionError::ServerError {
                                        code: 400 | 416 | 500..599,
                                        ..
                                    },
                                ) => {
                                    last_err = Some(e);
                                    tokio::time::sleep(Duration::from_secs(i * 2)).await;
                                }
                                Err(e) => return Err(e),
                            }
                        }

                        if let Some(e) = last_err {
                            return Err(e);
                        }
                    }

                    progress.inc();

                    Ok(())
                }
            })
            .boxed() // Workaround to rustc issue https://github.com/rust-lang/rust/issues/104382
            .buffer_unordered(16)
            .try_for_each(|()| async { Ok(()) })
            .await?;

        let config_url = self
            .client
            .push_blob(image_ref, image.config.data, &image.manifest.config.digest)
            .await?;
        let manifest_url = self
            .client
            .push_manifest(image_ref, &image.manifest.into())
            .await?;

        progress.done("image pushed");

        Ok(Some(PushResponse {
            config_url,
            manifest_url,
        }))
    }

    pub async fn push(
        &mut self,
        mut progress: Item,
        image_ref: &Reference,
        image: Image,
    ) -> Result<Option<PushResponse>, PushError> {
        let registry = image_ref.resolve_registry();
        self.client.store_auth_if_needed(registry, &self.auth).await;

        if let Some(digest) = self.try_resolve_digest(&self.auth, image_ref).await? {
            // If the digest matches the image's digest, we can skip pushing
            if digest == image.digest {
                progress.info("image already exists, skipping push");
                return Ok(None);
            }
        }

        if self.use_monolithic_push {
            return self.monolithic_push(progress, image_ref, image).await;
        }

        self.layered_push(progress, image_ref, image).await
    }
}
