use std::{collections::HashMap, path::PathBuf, sync::Arc};

use docker_credential::{CredentialRetrievalError, DockerCredential};
use oci_distribution::{Client, Reference, client::ClientConfig, secrets::RegistryAuth};
use tokio::fs;

use crate::{
    builder::{BuildError, MetaBuild},
    config::Config,
    parse_host, progress,
    registry::{PushError, Registry},
    tag::{self, TagError},
};

mod skaffold {
    use serde::Serialize;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Build {
        pub image_name: String,
        pub tag: String,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Output {
        pub builds: Vec<Build>,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("failed to write to file")]
    IO(#[from] std::io::Error),
    #[error("failed to serialize output")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to tag")]
    Tag(#[from] TagError),
    #[error("failed to build")]
    Build(#[from] BuildError),
    #[error("failed to push")]
    Push(#[from] PushError),
    #[error("failed to write output")]
    WriteOutput(#[from] WriteError),
    #[error("failed to retrieve registry credentials")]
    Credential(#[from] CredentialRetrievalError),
    #[error("failed to parse reference")]
    Parse(#[from] oci_distribution::ParseError),
}

pub async fn run(
    config: Config,
    platform: String,
    repo: Option<String>,
    output_file: Option<PathBuf>,
) -> Result<(), Error> {
    let root = progress::tree();
    let handle = progress::setup_line_renderer(&root);
    let builder = MetaBuild::new(Arc::new(config));
    let output = builder.build(root.add_child("build"), platform).await?;
    let tag = tag::resolve().await?;

    match repo {
        Some(repo) => {
            let host = parse_host(&repo);
            let auth = match docker_credential::get_credential(host)? {
                DockerCredential::IdentityToken(_) => unimplemented!(),
                DockerCredential::UsernamePassword(user, pass) => RegistryAuth::Basic(user, pass),
            };

            let client = Client::new(ClientConfig::default());
            let registry = Registry::new(Arc::clone(&root), client, auth);
            let mut artifacts = HashMap::new();

            for (artifact, images) in output.artifacts {
                for image in images {
                    let image_ref = Reference::try_from(format!("{repo}/{artifact}:{tag}"))?;
                    let output_ref = format!("{repo}/{artifact}:{tag}@{}", image.digest);

                    registry.push(&artifact, &image_ref, image).await?;
                    artifacts.insert(artifact.clone(), output_ref);
                }
            }

            handle.shutdown_and_wait();

            println!("\nPushed artifacts:");

            for (artifact, image_ref) in artifacts.iter() {
                println!("- {artifact}: {}", image_ref);
            }

            if let Some(path) = output_file {
                let output = skaffold::Output {
                    builds: artifacts
                        .into_iter()
                        .map(|(image_name, tag)| skaffold::Build { image_name, tag })
                        .collect(),
                };

                let data = serde_json::to_vec(&output).map_err(WriteError::Serde)?;
                fs::write(path, data).await.map_err(WriteError::IO)?;
            }

            Ok(())
        }
        None => {
            handle.shutdown_and_wait();
            println!("no repo set, skipping push");
            Ok(())
        }
    }
}
