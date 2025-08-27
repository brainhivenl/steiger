use std::{path::PathBuf, process::ExitStatus};

use miette::Diagnostic;
use prodash::tree::Item;

use crate::{
    config::Helm,
    deploy::{Context, Deployer},
    exec::{self, CmdBuilder},
};

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum HelmError {
    #[error("failed to find helm binary")]
    Path(#[from] which::Error),
    #[error("failed to locate helm chart")]
    Chart(#[from] std::io::Error),
    #[error("helm chart at '{0}' is not a directory")]
    NotADir(String),
    #[error("failed to run 'helm upgrade': {0}")]
    Install(ExitStatus),
}

#[derive(Clone)]
pub struct HelmDeployer {
    binary: PathBuf,
}

impl HelmDeployer {
    async fn upgrade(
        &mut self,
        progress: &mut Item,
        release: &str,
        ctx: &Context<Helm>,
    ) -> Result<(), HelmError> {
        progress.info("upgrade/install helm release");

        let mut cmd = CmdBuilder::new(&self.binary);

        for build in ctx.output.builds.iter() {
            cmd.flag(
                "--set",
                format!(
                    "steiger.{}.image={}",
                    change_case::camel_case(&build.image_name),
                    build.tag
                ),
            );
        }

        if let Some(timeout) = &ctx.input.timeout {
            cmd.flag("--timeout", timeout);
        }

        if let Some(namespace) = &ctx.input.namespace {
            cmd.flag("--namespace", namespace);
        }

        for (key, value) in &ctx.input.values {
            cmd.flag("--set", format!("{key}={value}"));
        }

        for file in &ctx.input.values_files {
            cmd.flag("--values", file);
        }

        let status = exec::run_with_progress(
            cmd.arg("upgrade")
                .arg("--install")
                .arg(release)
                .arg(&ctx.input.path),
            progress.add_child(format!("{release} › helm")),
        )
        .await?;

        if !status.success() {
            progress.fail(format!(
                "deployment failed with exit code: {}",
                status.code().unwrap_or_default()
            ));

            return Err(HelmError::Install(status));
        }

        Ok(())
    }
}

impl Deployer for HelmDeployer {
    type Error = HelmError;
    type Input = Helm;

    fn try_init() -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        which::which("helm")
            .map(|binary| Self { binary })
            .map_err(|e| e.into())
    }

    async fn validate(&self, input: &Self::Input) -> Result<(), Self::Error> {
        let meta = tokio::fs::metadata(&input.path).await?;

        if !meta.is_dir() {
            return Err(HelmError::NotADir(input.path.clone()));
        }

        Ok(())
    }

    async fn deploy(
        mut self,
        mut progress: Item,
        release: String,
        ctx: Context<Self::Input>,
    ) -> Result<(), Self::Error> {
        self.upgrade(&mut progress, &release, &ctx).await?;

        progress.done("deployment finished".to_string());

        Ok(())
    }
}
