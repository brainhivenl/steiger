use std::{collections::HashMap, path::PathBuf, process::ExitStatus};

use prodash::{
    NestedProgress, Progress, messages::MessageLevel, progress::DoOrDiscard, tree::Item,
};
use tokio::process::Command;

use crate::{
    builder::{Builder, Output},
    config::Bazel,
    exec::{self, ExitError},
    image,
};

#[derive(Debug, thiserror::Error)]
pub enum BazelError {
    #[error("failed to find bazel binary: {0}")]
    Path(#[from] which::Error),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("failed to run 'bazel build': {0:?}")]
    Build(ExitStatus),
    #[error("failed to parse image: {0}")]
    Image(#[from] image::ImageError),
    #[error("failed to query for output: {0}")]
    Exit(#[from] ExitError),
    #[error("failed to deserialize cquery output: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("unable to find artifact for target: {0}")]
    MissingArtifact(String),
}

#[derive(Clone)]
pub struct BazelBuilder {
    binary: PathBuf,
}

impl BazelBuilder {
    pub async fn get_files_output(
        &self,
        targets: impl Iterator<Item = &String>,
    ) -> Result<HashMap<String, String>, BazelError> {
        // Output the target and it's output
        let output = exec::run_with_output(
            Command::new(&self.binary)
                .arg("cquery")
                .arg(
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

    async fn build(
        self,
        mut pb: DoOrDiscard<Item>,
        service_name: String,
        input: Self::Input,
    ) -> Result<Output, Self::Error> {
        pb.set_name(service_name);
        pb.message(MessageLevel::Info, "starting builder".to_string());

        let status = exec::run_with_progress(
            Command::new(&self.binary)
                .arg("build")
                .args(input.targets.values()),
            pb.add_child("bazel"),
        )
        .await?;

        if !status.success() {
            pb.message(
                MessageLevel::Failure,
                format!(
                    "build failed with exit code: {}",
                    status.code().unwrap_or_default()
                ),
            );

            return Err(BazelError::Build(status));
        }

        pb.message(MessageLevel::Success, "build finished".to_string());
        pb.message(MessageLevel::Info, "gathering output".to_string());

        let cquery = self.get_files_output(input.targets.values()).await?;
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
