use std::{
    env,
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::Parser;

use crate::{builder::MetaBuild, config::Config};

mod builder;
mod config;
mod exec;
mod image;
mod progress;

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
    Build,
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let opts = Opts::parse();
    let dir = opts.dir.map(Ok).unwrap_or_else(env::current_dir)?;
    let config = read_config_file(opts.config.unwrap_or_else(|| dir.join("steiger.yml"))).await?;

    env::set_current_dir(&dir)?;

    match opts.cmd {
        Cmd::Build => {
            let builder = MetaBuild::new(Arc::new(config));
            let output = builder.build().await?;

            println!("{output:#?}");

            Ok(())
        }
    }
}
