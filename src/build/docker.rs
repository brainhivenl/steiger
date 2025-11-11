use std::{collections::HashMap, path::PathBuf, process::ExitStatus};

use async_tempfile::TempDir;
use miette::Diagnostic;
use tokio::process::Command;

use crate::{
    build::{Builder, Context, Output},
    config::Docker,
    exec::{self, CmdBuilder, ExitError},
    image,
};

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum DockerError {
    #[error("failed to find docker binary")]
    Path(#[from] which::Error),
    #[error("failed to list buildkit builders")]
    #[diagnostic(transparent)]
    ListBuilders(#[source] ExitError),
    #[error("failed to create buildkit builder")]
    #[diagnostic(transparent)]
    CreateBuilder(#[source] ExitError),
    #[error("IO error")]
    IO(#[from] std::io::Error),
    #[error("failed to create tempdir")]
    TempDir(#[from] async_tempfile::Error),
    #[error("failed to parse image")]
    #[diagnostic(transparent)]
    Image(#[from] image::ImageError),
    #[error("failed to parse buildkit output")]
    Serde(#[from] serde_json::Error),
    #[error("failed to run 'docker build': {0}")]
    Build(ExitStatus),
}

mod buildx {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    pub struct Builder {
        pub name: String,
    }
}

fn fmt_map(map: HashMap<String, String>, sep: char) -> Vec<String> {
    map.into_iter()
        .map(|(name, value)| format!("{name}{sep}{value}"))
        .collect::<Vec<_>>()
}

#[derive(Clone)]
pub struct DockerBuilder {
    binary: PathBuf,
}

impl DockerBuilder {
    async fn list_builders(&self) -> Result<Vec<buildx::Builder>, DockerError> {
        let output = exec::run_with_output(
            Command::new(&self.binary)
                .arg("buildx")
                .arg("ls")
                .arg("--format=json"),
        )
        .await
        .map_err(DockerError::ListBuilders)?;

        Ok(output
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<Vec<_>, _>>()?)
    }

    async fn create_builder(&self) -> Result<(), DockerError> {
        exec::run_with_output(
            Command::new(&self.binary)
                .arg("buildx")
                .arg("create")
                .arg("--driver=docker-container")
                .arg("--name=steiger"),
        )
        .await
        .map_err(DockerError::CreateBuilder)?;

        Ok(())
    }
}

impl Builder for DockerBuilder {
    type Error = DockerError;
    type Input = Docker;

    fn try_init() -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        which::which("docker")
            .map(|binary| Self { binary })
            .map_err(|e| e.into())
    }

    async fn build(
        self,
        Context {
            service_name,
            platform,
            mut progress,
        }: Context,
        input: Self::Input,
    ) -> Result<Output, Self::Error> {
        progress.set_name(&service_name);
        progress.info("starting builder");

        let builders = self.list_builders().await?;

        if !builders.iter().any(|b| b.name == "steiger") {
            progress.info("creating buildkit builder");

            match self.create_builder().await {
                Err(DockerError::CreateBuilder(ExitError::Status { code: 1, stderr }))
                    if stderr.contains("ERROR: existing instance for") =>
                {
                    progress.info("buildkit builder exists, assuming remote driver");
                }
                Err(e) => return Err(e),
                Ok(()) => {}
            }

            progress.done("buildkit builder created");
        } else {
            progress.info("using existing buildkit builder");
        }

        let mut cmd = CmdBuilder::new(&self.binary);
        cmd.arg("buildx").arg("build");

        let build_args = fmt_map(input.build_args, '=');
        let hosts = fmt_map(input.hosts, ':');

        for entry in build_args.iter() {
            cmd.flag("--build-arg", entry);
        }

        for entry in hosts.iter() {
            cmd.flag("--add-host", entry);
        }

        let dest = TempDir::new_with_name(&service_name).await?;
        let status = exec::run_with_progress(
            cmd.arg("--builder")
                .arg("steiger")
                .arg("--platform")
                .arg(&platform)
                .arg("--output")
                .arg(format!(
                    "type=oci,dest={},tar=false",
                    dest.as_os_str().to_string_lossy()
                ))
                .arg("--file")
                .arg(
                    input
                        .dockerfile
                        .as_deref()
                        .unwrap_or(&format!("{}/Dockerfile", input.context)),
                )
                .arg(&input.context),
            progress.add_child(format!("{service_name} › docker")),
        )
        .await?;

        if !status.success() {
            progress.fail(format!(
                "build failed with exit code: {}",
                status.code().unwrap_or_default()
            ));

            return Err(DockerError::Build(status));
        }

        progress.done("build finished".to_string());

        let images = image::load_from_path(dest).await?;

        Ok(Output {
            artifacts: vec![(service_name, images)].into_iter().collect(),
        })
    }
}
