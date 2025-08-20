use std::{
    env,
    error::Error,
    path::{Path, PathBuf},
};

use clap::Parser;

use crate::config::Config;

mod builder;
mod cmd;
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
        #[arg(short, long, help = "OCI registry to use")]
        repo: Option<String>,

        #[arg(short, long, help = "Output file location")]
        output_file: Option<PathBuf>,
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

pub fn parse_host(path: &str) -> &str {
    path.split('/').next().unwrap_or_default()
}

async fn detect_kube_platform() -> Result<String, Box<dyn Error>> {
    let client = kube::Client::try_default().await?;
    let version = client.apiserver_version().await?;

    Ok(version.platform)
}

async fn detect_platform() -> String {
    if let Ok(platform) = detect_kube_platform().await {
        return platform;
    }

    match (env::consts::OS, env::consts::ARCH) {
        ("linux", "x86_64") => "linux/amd64".to_string(),
        ("linux", "aarch64") => "linux/arm64".to_string(),
        ("macos", "x86_64") => "darwin/amd64".to_string(),
        ("macos", "aarch64") => "darwin/arm64".to_string(),
        ("windows", "x86_64") => "windows/amd64".to_string(),
        _ => unimplemented!("unsupported platform"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let opts = Opts::parse();
    let dir = opts.dir.map(Ok).unwrap_or_else(env::current_dir)?;
    let config = read_config_file(opts.config.unwrap_or_else(|| dir.join("steiger.yml"))).await?;
    let platform = detect_platform().await;

    env::set_current_dir(&dir)?;

    match opts.cmd {
        Cmd::Build { repo, output_file } => {
            cmd::build::run(config, platform, repo, output_file).await
        }
    }
}
