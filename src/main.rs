use std::{env, error::Error, path::PathBuf};

use async_tempfile::TempFile;
use clap::Parser;
use miette::Diagnostic;

use crate::config::ConfigError;

mod builder;
mod cmd;
mod config;
mod deploy;
mod exec;
mod image;
mod progress;
mod registry;
mod tag;

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
    /// Build all artifacts
    Build {
        /// OCI registry to use
        #[arg(short, long)]
        repo: Option<String>,

        /// Output file location
        #[arg(short, long)]
        output_file: Option<PathBuf>,

        /// Platform selector (e.g. linux/amd64)
        #[arg(long)]
        platform: Option<String>,

        /// Profile name
        #[arg(short, long)]
        profile: Option<String>,
    },

    /// Deploy artifacts based on the output-file of the build command
    Deploy {
        /// Input file location
        #[arg(short, long)]
        input_file: PathBuf,

        /// Profile name
        #[arg(short, long)]
        profile: Option<String>,
    },

    /// Run the build and deploy commands in sequence
    Run {
        /// OCI registry to use
        #[arg(short, long)]
        repo: String,

        /// Platform selector (e.g. linux/amd64)
        #[arg(long)]
        platform: Option<String>,

        /// Profile name
        #[arg(short, long)]
        profile: Option<String>,
    },
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

#[derive(Debug, Diagnostic, thiserror::Error)]
enum AppError {
    #[error("I/O error")]
    IO(#[from] std::io::Error),
    #[error("failed to read config")]
    #[diagnostic(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Build(Box<cmd::build::Error>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Deploy(#[from] cmd::deploy::Error),
    #[error("failed to get current dir")]
    CurrentDir(std::io::Error),
    #[error("failed to set current dir")]
    SetCurrentDir(std::io::Error),
    #[error("failed to create temp file")]
    TempFile(#[from] async_tempfile::Error),
}

impl From<cmd::build::Error> for AppError {
    fn from(e: cmd::build::Error) -> Self {
        AppError::Build(Box::new(e))
    }
}

async fn run(opts: Opts) -> Result<(), AppError> {
    let dir = opts
        .dir
        .map(Ok)
        .unwrap_or_else(env::current_dir)
        .map_err(AppError::CurrentDir)?;
    let config_path = opts.config.unwrap_or_else(|| dir.join("steiger.yml"));
    let detected_platform = detect_platform().await;

    env::set_current_dir(&dir).map_err(AppError::SetCurrentDir)?;

    match opts.cmd {
        Cmd::Build {
            profile,
            repo,
            output_file,
            platform,
        } => {
            let config = config::load_from_path(profile.as_deref(), config_path).await?;
            cmd::build::run(
                config,
                platform.unwrap_or(detected_platform),
                repo,
                output_file.as_deref(),
            )
            .await?;
        }
        Cmd::Deploy {
            profile,
            input_file,
        } => {
            let config = config::load_from_path(profile.as_deref(), config_path).await?;
            cmd::deploy::run(config, &input_file).await?;
        }
        Cmd::Run {
            profile,
            repo,
            platform,
        } => {
            let dest = TempFile::new().await?;
            let config = config::load_from_path(profile.as_deref(), config_path).await?;

            cmd::build::run(
                config.clone(),
                platform.unwrap_or(detected_platform),
                Some(repo),
                Some(dest.file_path()),
            )
            .await?;

            dest.sync_all().await?;

            cmd::deploy::run(config, dest.file_path()).await?;
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> miette::Result<()> {
    let opts = Opts::parse();
    run(opts).await?;

    Ok(())
}
