use std::{path::PathBuf, process::ExitStatus};

use async_tempfile::TempDir;
use gix::progress::MessageLevel;
use prodash::tree::Item;
use tokio::process::Command;

use crate::{
    builder::{Builder, Context, Output},
    config::Ko,
    exec, image,
};

#[derive(Debug, thiserror::Error)]
pub enum KoError {
    #[error("IO error")]
    IO(#[from] std::io::Error),
    #[error("failed to find ko binary")]
    Path(#[from] which::Error),
    #[error("failed to create tempdir")]
    TempDir(#[from] async_tempfile::Error),
    #[error("failed to parse image")]
    Image(#[from] image::ImageError),
    #[error("failed to run 'ko build': {0:?}")]
    Build(ExitStatus),
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
        mut progress: Item,
        Context {
            service_name,
            platform,
            input,
        }: Context<Self::Input>,
    ) -> Result<Output, Self::Error> {
        progress.set_name(&service_name);
        progress.message(MessageLevel::Info, "starting builder");

        let dest = TempDir::new_with_name(&service_name).await?;
        let status = exec::run_with_progress(
            Command::new(&self.binary)
                .arg("build")
                .arg("--push=false")
                .arg("--platform")
                .arg(&platform)
                .arg("--oci-layout-path")
                .arg(dest.as_os_str())
                .arg(input.import_path.as_deref().unwrap_or(".")),
            progress.add_child(format!("{service_name} › ko")),
        )
        .await?;

        if !status.success() {
            progress.message(
                MessageLevel::Failure,
                format!(
                    "build failed with exit code: {}",
                    status.code().unwrap_or_default()
                ),
            );

            return Err(KoError::Build(status));
        }

        progress.message(MessageLevel::Success, "build finished".to_string());

        let images = image::load_from_path(dest).await?;

        Ok(Output {
            artifacts: vec![(service_name, images)].into_iter().collect(),
        })
    }
}
