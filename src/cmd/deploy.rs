use std::{path::Path, sync::Arc};

use miette::Diagnostic;

use crate::{
    cmd::build::output::Output,
    config::Config,
    deploy::{DeployError, MetaDeployer, helm::HelmError},
    progress,
};

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum InputError {
    #[error("failed to read file")]
    IO(#[from] std::io::Error),
    #[error("failed to parse input file")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum Error {
    #[error("failed to read input file")]
    #[diagnostic(transparent)]
    Input(#[from] InputError),
    #[error("failed to deploy")]
    #[diagnostic(transparent)]
    Deploy(#[from] DeployError),
    #[error("failed to init helm deployer")]
    #[diagnostic(transparent)]
    Helm(#[from] HelmError),
}

async fn read_input(path: impl AsRef<Path>) -> Result<Output, InputError> {
    let content = tokio::fs::read(path).await?;
    Ok(serde_json::from_slice(&content)?)
}

pub async fn run(config: Config, input_file: &Path) -> Result<(), Error> {
    let input = read_input(input_file).await?;
    let root = progress::tree();
    let handle = progress::setup_line_renderer(&root);
    let mut progress = root.add_child("deploy");

    let mut deploy = MetaDeployer::new(config, Arc::new(input));

    deploy.validate(&mut progress).await?;
    deploy.deploy(progress).await?;

    handle.shutdown_and_wait();

    Ok(())
}
