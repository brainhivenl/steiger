use std::path::PathBuf;

use async_tempfile::TempDir;
use miette::Diagnostic;
use tokio::process::Command;

use crate::{
    build::{Builder, Context, Output},
    config::Ko,
    exec::{self, CommandError},
    image,
};

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum KoError {
    #[error("IO error")]
    IO(#[from] std::io::Error),
    #[error("failed to find ko binary")]
    Path(#[from] which::Error),
    #[error("failed to create tempdir")]
    TempDir(#[from] async_tempfile::Error),
    #[error("failed to parse image")]
    #[diagnostic(transparent)]
    Image(#[from] image::ImageError),
    #[error("failed to run 'ko build'")]
    #[diagnostic(transparent)]
    Build(#[from] CommandError),
}

#[derive(Clone)]
pub struct KoBuilder {
    binary: PathBuf,
}

impl Builder for KoBuilder {
    type Error = KoError;
    type Input = Ko;

    fn try_init() -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        which::which("ko")
            .map(|binary| Self { binary })
            .map_err(|e| e.into())
    }

    async fn build(
        self,
        ctx: &mut Context,
        input: Self::Input,
    ) -> Result<Output, Self::Error> {
        let dest = TempDir::new_with_name(&ctx.service_name).await?;
        exec::run_with_progress(
            Command::new(&self.binary)
                .arg("build")
                .arg("--push=false")
                .arg("--platform")
                .arg(&ctx.platform)
                .arg("--oci-layout-path")
                .arg(dest.as_os_str())
                .arg(input.import_path.as_deref().unwrap_or(".")),
            ctx.child_progress("ko"),
        )
        .await?;

        let images = image::load_from_path(dest).await?;

        Ok(Output {
            artifacts: vec![(ctx.service_name.clone(), images)].into_iter().collect(),
        })
    }
}
