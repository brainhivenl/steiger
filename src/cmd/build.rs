use std::{collections::HashMap, error::Error, path::PathBuf, sync::Arc};

use docker_credential::DockerCredential;
use oci_distribution::{Client, client::ClientConfig, secrets::RegistryAuth};
use tokio::fs;

use crate::{builder::MetaBuild, config::Config, parse_host, progress, registry::Registry};

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

pub async fn run(
    config: Config,
    platform: String,
    repo: Option<String>,
    output_file: Option<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let root = progress::tree();
    let handle = progress::setup_line_renderer(&root);
    let builder = MetaBuild::new(Arc::new(config));
    let output = builder.build(root.add_child("build"), platform).await?;

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
                    artifacts.insert(
                        artifact.clone(),
                        registry.push(&repo, &artifact, image).await?,
                    );
                }
            }

            handle.shutdown_and_wait();

            println!("\nPushed artifacts:");

            for (artifact, output) in artifacts.iter() {
                println!("- {artifact}: {}", output.reference);
            }

            if let Some(path) = output_file {
                let output = skaffold::Output {
                    builds: artifacts
                        .into_iter()
                        .map(|(artifact, output)| skaffold::Build {
                            image_name: artifact,
                            tag: output.reference.to_string(),
                        })
                        .collect(),
                };

                fs::write(path, serde_json::to_vec(&output)?).await?;
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
