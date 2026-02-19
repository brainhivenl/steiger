use std::{env, error::Error, path::PathBuf};

use async_tempfile::TempFile;
use clap::Parser;
use miette::Diagnostic;
use steiger::config;

mod build;
mod cmd;
mod deploy;
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
        repo: Option<String>,

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
    Config(#[from] config::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LocateConfig(#[from] config::LocateError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Build(Box<cmd::build::Error>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Deploy(#[from] cmd::deploy::Error),
    #[error("failed to set current dir")]
    SetCurrentDir(std::io::Error),
    #[error("failed to create temp file")]
    TempFile(#[from] async_tempfile::Error),
    #[error("no repository specified")]
    #[diagnostic(help("either set in config or pass via --repo"))]
    RepoRequired,
}

impl From<cmd::build::Error> for AppError {
    fn from(e: cmd::build::Error) -> Self {
        AppError::Build(Box::new(e))
    }
}

async fn run(opts: Opts) -> Result<(), AppError> {
    let config_path = config::locate(opts.dir.as_ref(), opts.config.as_ref())?;
    let detected_platform = detect_platform().await;

    if let Some(ref dir) = opts.dir {
        env::set_current_dir(dir).map_err(AppError::SetCurrentDir)?;
    }

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

            if repo.is_none() && config.default_repo.is_none() {
                return Err(AppError::RepoRequired);
            }

            cmd::build::run(
                config.clone(),
                platform.unwrap_or(detected_platform),
                repo,
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
