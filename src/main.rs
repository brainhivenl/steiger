use std::{
    env,
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::Parser;
use docker_credential::DockerCredential;
use oci_distribution::{Client, client::ClientConfig, secrets::RegistryAuth};

use crate::{builder::MetaBuild, config::Config, registry::Registry};

mod builder;
mod config;
mod exec;
mod image;
mod progress;
mod registry;

#[derive(Parser)]
struct Opts {
    #[arg(short, long)]
    dir: Option<PathBuf>,

    #[arg(short, long)]
    config: Option<PathBuf>,

    #[clap(subcommand)]
    cmd: Cmd,
}

#[derive(Parser)]
enum Cmd {
    Build {
        #[arg(long, help = "OCI registry to use")]
        repo: Option<String>,
    },
}

#[derive(Debug, thiserror::Error)]
enum ConfigError {
    #[error("I/O error: {0}")]
    IO(#[from] std::io::Error),
    #[error("failed to deserialize: {0}")]
    Yaml(#[from] serde_yml::Error),
}

async fn read_config_file(path: impl AsRef<Path>) -> Result<Config, ConfigError> {
    let contents = tokio::fs::read(path).await?;
    Ok(serde_yml::from_slice(&contents)?)
}

fn parse_host(path: &str) -> &str {
    path.split('/').next().unwrap_or_default()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let opts = Opts::parse();
    let dir = opts.dir.map(Ok).unwrap_or_else(env::current_dir)?;
    let config = read_config_file(opts.config.unwrap_or_else(|| dir.join("steiger.yml"))).await?;

    env::set_current_dir(&dir)?;

    match opts.cmd {
        Cmd::Build { repo } => {
            let root = progress::tree();
            let handle = progress::setup_line_renderer(&root);
            let builder = MetaBuild::new(Arc::new(config));
            let output = builder.build(Arc::clone(&root)).await?;

            match repo {
                Some(repo) => {
                    let host = parse_host(&repo);
                    let auth = match docker_credential::get_credential(host)? {
                        DockerCredential::IdentityToken(_) => unimplemented!(),
                        DockerCredential::UsernamePassword(user, pass) => {
                            RegistryAuth::Basic(user, pass)
                        }
                    };

                    let client = Client::new(ClientConfig::default());
                    let mut registry = Registry::new(Arc::clone(&root), client, auth);

                    for (artifact, images) in output.artifacts {
                        for image in images {
                            registry.push(&repo, &artifact, image).await?;
                        }
                    }
                }
                None => {
                    println!("no repo set, skipping push");
                }
            }

            handle.shutdown_and_wait();
            Ok(())
        }
    }
}
