use std::{collections::HashMap, path::Path, sync::Arc};

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
pub enum ContextError {
    #[error("attempted to deploy to the wrong cluster (expected `{expected}`, got `{actual}`)")]
    Mismatch { expected: String, actual: String },
    #[error("no active kubernetes context found")]
    Missing,
    #[error("failed to read kubeconfig")]
    KubeConfig(#[from] kube::config::KubeconfigError),
    #[error("failed to join task")]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum Error {
    #[error("failed to read input file")]
    #[diagnostic(transparent)]
    Input(#[from] InputError),
    #[error("failed to verify kubernetes context")]
    #[diagnostic(transparent)]
    Context(#[from] ContextError),
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

async fn ensure_context(
    profile: Option<&str>,
    mappings: &HashMap<String, String>,
) -> Result<(), ContextError> {
    let Some(expected) = profile.and_then(|profile| mappings.get(profile)) else {
        return Ok(());
    };

    let kubeconfig = tokio::task::spawn_blocking(kube::config::Kubeconfig::read).await??;
    let current_context = kubeconfig.current_context.ok_or(ContextError::Missing)?;
    if current_context.ne(expected) {
        return Err(ContextError::Mismatch {
            expected: expected.to_string(),
            actual: current_context.to_string(),
        });
    }

    Ok(())
}

pub async fn run(profile: Option<&str>, config: Config, input_file: &Path) -> Result<(), Error> {
    ensure_context(profile, &config.context_mappings).await?;

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
