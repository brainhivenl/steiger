use std::{collections::HashMap, path::PathBuf};

use miette::Diagnostic;
use tokio::process::Command;
use tracing::instrument;

use crate::{
    build::{Builder, Context, Output},
    config::Bazel,
    exec::{self, CmdBuilder, CommandError, ExitError},
    image,
};

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum BazelError {
    #[error("failed to find bazel binary")]
    Path(#[from] which::Error),
    #[error("IO error")]
    IO(#[from] std::io::Error),
    #[error("failed to run 'bazel build'")]
    #[diagnostic(transparent)]
    Build(#[from] CommandError),
    #[error("failed to parse image")]
    #[diagnostic(transparent)]
    Image(#[from] image::ImageError),
    #[error("failed to query for output")]
    #[diagnostic(transparent)]
    Exit(#[from] ExitError),
    #[error("failed to deserialize cquery output")]
    Serde(#[from] serde_json::Error),
    #[error("unable to find artifact for target: {0}")]
    MissingArtifact(String),
}

#[derive(Clone)]
pub struct BazelBuilder {
    binary: PathBuf,
}

impl BazelBuilder {
    #[instrument(skip(self, targets), err(Debug))]
    pub async fn get_files_output(
        &self,
        platform: Option<&String>,
        targets: impl Iterator<Item = &String>,
    ) -> Result<HashMap<String, String>, BazelError> {
        let mut cmd = CmdBuilder::new(&self.binary);
        cmd.arg("cquery");

        if let Some(platform) = platform {
            cmd.arg(format!("--platforms={platform}"));
        }

        // Output the target and it's output
        let output = exec::run_with_output(
            cmd.arg(
                targets
                    .map(|target| format!("\"{target}\""))
                    .collect::<Vec<_>>()
                    .join(" union "),
            )
            .arg("--output=starlark")
            .arg(
                r#"--starlark:expr=json.encode([
                        '{}:{}'.format(target.label.package, target.label.name),
                        [f.path for f in target.files.to_list()][0]
                    ])"#,
            ),
        )
        .await?;

        Ok(output
            .trim()
            .lines()
            .filter(|line| !line.is_empty())
            .map(serde_json::from_str)
            .collect::<Result<HashMap<_, _>, _>>()?)
    }
}

impl Builder for BazelBuilder {
    type Error = BazelError;
    type Input = Bazel;

    fn try_init() -> Result<Self, Self::Error> {
        which::which("bazel")
            .or_else(|_| which::which("bazelisk"))
            .map(|binary| Self { binary })
            .map_err(|e| e.into())
    }

    #[instrument(skip(self, ctx, input), fields(service = %ctx.service_name, platform = %ctx.platform), err(Debug))]
    async fn build(
        self,
        ctx: &mut Context,
        input: Self::Input,
    ) -> Result<Output, Self::Error> {
        let bazel_platform = input.platforms.get(&ctx.platform);
        let mut root_cmd = Command::new(&self.binary);
        let mut cmd = root_cmd.arg("build");

        if let Some(platform) = bazel_platform {
            cmd = cmd.arg(format!("--platforms={platform}"));
            ctx.progress.info(format!("using platform: {platform}"));
        }

        exec::run_with_progress(
            cmd.args(input.targets.values()),
            ctx.child_progress("bazel"),
        )
        .await?;

        ctx.progress.info("gathering output".to_string());

        let cquery = self
            .get_files_output(bazel_platform, input.targets.values())
            .await?;
        let mut artifacts = HashMap::default();

        for (target, files) in cquery {
            let artifact = input
                .targets
                .iter()
                .find(|(_, t)| t == &&target)
                .map(|(artifact, _)| artifact.clone())
                .ok_or(BazelError::MissingArtifact(target))?;

            artifacts.insert(artifact, image::load_from_path(files).await?);
        }

        Ok(Output { artifacts })
    }
}
