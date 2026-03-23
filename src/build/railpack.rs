use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use async_tempfile::TempDir;
use miette::Diagnostic;
use tokio::process::Command;

use crate::{
    build::{
        Builder, Context, Output,
        docker::{DockerBuilder, DockerError},
    },
    config::{Docker, Railpack},
    exec::{self, CommandError},
};

const RAILPACK_FRONTEND: &str = "ghcr.io/railwayapp/railpack-frontend";
const RAILPACK_CONFIG: &str = "railpack.json";

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum RailpackError {
    #[error("failed to find railpack binary")]
    Path(#[from] which::Error),
    #[error("docker error")]
    #[diagnostic(transparent)]
    Docker(#[from] DockerError),
    #[error("failed to run railpack")]
    #[diagnostic(transparent)]
    Command(#[from] CommandError),
    #[error("IO error")]
    IO(#[from] std::io::Error),
    #[error("failed to create tempdir")]
    TempDir(#[from] async_tempfile::Error),
}

#[derive(Clone)]
pub struct RailpackBuilder {
    binary: PathBuf,
    docker: DockerBuilder,
}

impl Builder for RailpackBuilder {
    type Error = RailpackError;
    type Input = Railpack;

    fn try_init() -> Result<Self, Self::Error> {
        let binary = which::which("railpack")?;
        let docker = DockerBuilder::try_init()?;
        Ok(Self { binary, docker })
    }

    async fn build(self, ctx: &mut Context, input: Self::Input) -> Result<Output, Self::Error> {
        let tmp = TempDir::new().await?;
        let plan_out = tmp.as_ref().join(RAILPACK_CONFIG);
        let existing = Path::new(&input.context).join(RAILPACK_CONFIG);

        if existing.exists() {
            ctx.progress.info("using existing railpack.json");
            tokio::fs::copy(&existing, &plan_out).await?;
        } else {
            ctx.progress.info("generating build plan");
            exec::run_with_progress(
                Command::new(&self.binary)
                    .arg("prepare")
                    .arg(&input.context)
                    .arg("--plan-out")
                    .arg(&plan_out),
                ctx.child_progress("railpack"),
            )
            .await?;
        }

        let docker_input = Docker {
            context: input.context,
            dockerfile: Some(plan_out.to_string_lossy().into_owned()),
            build_args: HashMap::from([("BUILDKIT_SYNTAX".into(), RAILPACK_FRONTEND.into())]),
            ..Default::default()
        };

        self.docker
            .build(ctx, docker_input)
            .await
            .map_err(RailpackError::Docker)
    }
}
