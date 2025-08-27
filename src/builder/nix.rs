use std::{collections::HashMap, path::PathBuf, process::ExitStatus, sync::Arc};

use aho_corasick::AhoCorasick;
use miette::Diagnostic;
use once_cell::sync::Lazy;
use prodash::{Progress, messages::MessageLevel, tree::Item};
use serde::Deserialize;
use serde_repr::Deserialize_repr;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    task::JoinSet,
};
use which::which;

use crate::{
    builder::Context,
    builder::{Builder, Output},
    config::Nix,
    exec::{self, ExitError},
    image, progress,
};

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum NixError {
    #[error("failed to find nix binary")]
    Path(#[from] which::Error),
    #[error("IO error")]
    IO(#[from] std::io::Error),
    #[error("failed to join worker tasks")]
    Join(#[from] tokio::task::JoinError),
    #[error("failed to parse image")]
    #[diagnostic(transparent)]
    Image(#[from] image::ImageError),
    #[error("failed to query for output")]
    Exit(#[from] ExitError),
    #[error("failed to deserialize nix message")]
    Serde(#[from] serde_json::Error),
    #[error("failed to evaluate: {0}")]
    Eval(String),
    #[error("failed to run nix build: {0}")]
    Build(ExitStatus),
    #[error("invalid platform: {0}")]
    InvalidPlatform(String),
    #[error("failed to convert nix system to platform: {0}")]
    UnsupportedPlatform(String),
    #[error("unable to find artifact for target: {0}")]
    MissingArtifact(String),
}

type OutPaths = HashMap<String, PathBuf>;

static ANSI_REPLACER: Lazy<AhoCorasick> =
    Lazy::new(|| AhoCorasick::new([r"\u001b", r"\033", r"\x1b", r"\e"]).unwrap());

fn unescape_ansi(text: &str) -> String {
    ANSI_REPLACER.replace_all(text, &["\x1b"; 4])
}

fn try_system(platform: &str) -> Result<String, NixError> {
    let Some((os, arch)) = platform.split_once("/") else {
        return Err(NixError::InvalidPlatform(platform.to_string()));
    };

    let arch = match arch {
        "arm64" => "aarch64",
        "amd64" => "x86_64",
        _ => return Err(NixError::UnsupportedPlatform(platform.to_string())),
    };

    Ok([arch, os].join("-"))
}

#[derive(Debug, Deserialize_repr, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
enum Verbosity {
    Error,
    Warn,
    Notice,
    Info,
    Talkative,
    Chatty,
    Debug,
    Vomit,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
enum BuildAction {
    Result {
        fields: Vec<serde_json::Value>,
        #[serde(rename = "type")]
        ty: u8,
    },
    Msg {
        level: Verbosity,
        msg: String,
    },
    Start {},
    Stop {},
}

impl BuildAction {
    const FILE_LINKED: u8 = 100;
    const BUILD_LOG_LINE: u8 = 101;
    const UNTRUSTED_PATH: u8 = 102;
    const CORRUPTED_PATH: u8 = 103;
    const SET_PHASE: u8 = 104;
    const _PROGRESS: u8 = 105;
    const _SET_EXPECTED: u8 = 106;
    const POST_BUILD_LOG_LINE: u8 = 107;

    fn report(&self, progress: &Item) -> Option<()> {
        match self {
            Self::Result { fields, ty, .. } => match *ty {
                Self::BUILD_LOG_LINE | Self::POST_BUILD_LOG_LINE => {
                    let text = fields[0].as_str()?;
                    progress.info(unescape_ansi(text));
                }
                Self::FILE_LINKED => {
                    let output_path = fields[0].as_str()?;
                    let store_path = fields[1].as_str()?;
                    progress.done(format!("✓ Linked {output_path} → {store_path}"));
                }
                Self::UNTRUSTED_PATH => {
                    let path = fields[0].as_str()?;
                    progress.fail(format!("⚠ Untrusted: {path}"));
                }
                Self::CORRUPTED_PATH => {
                    let path = fields[0].as_str()?;
                    let corrupted_msg = format!("✗ Corrupted: {path}");
                    progress.fail(corrupted_msg);
                }
                Self::SET_PHASE => {
                    let phase = fields[0].as_str()?;
                    progress.set_name(phase);
                    progress.info(format!("→ Entering {phase}"));
                }
                _ => {}
            },
            Self::Msg { level, msg } => {
                if !msg.is_empty() && level <= &Verbosity::Info {
                    progress.info(msg.to_string());
                }
            }
            _ => {}
        }
        Some(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalResult {
    attr: String,
    attr_path: Vec<String>,
    drv_path: Option<String>,
    error: Option<String>,
    outputs: Option<HashMap<String, String>>,
}

impl EvalResult {
    async fn build(
        mut self,
        nix_binary: Arc<PathBuf>,
        mut progress: Item,
    ) -> Result<OutPaths, NixError> {
        if let Some(error) = self.error.take() {
            progress.message(MessageLevel::Failure, &error);
            return Err(NixError::Eval(error));
        }

        let mut out_paths = OutPaths::new();

        if let (Some(drv_path), Some(out_path)) = (
            self.drv_path.as_ref(),
            self.outputs.as_ref().and_then(|o| o.get("out")),
        ) {
            progress.info(format!("starting build for package: {}", &self.attr));

            let mut root_cmd = Command::new(nix_binary.as_ref());
            let cmd = root_cmd
                .arg("build")
                .arg("--no-link")
                .arg("--log-format")
                .arg("internal-json")
                .arg([drv_path, "out"].join("^"));

            let mut child = exec::spawn(cmd).await?;

            let progress = progress.add_child(&self.attr);
            let reader = BufReader::new(child.stderr);
            let mut lines = reader.lines();

            while let Some(line) = lines.next_line().await? {
                let Some(json) = line.strip_prefix("@nix ") else {
                    continue;
                };

                let action: BuildAction = serde_json::from_str(json)?;
                action.report(&progress);
            }

            let status = child.inner.wait().await?;
            progress.inc();

            if status.success() {
                progress.done(format!("successfully built package: {}", self.attr));
                out_paths.insert(self.attr, PathBuf::from(out_path));
            } else {
                let exit_code = status.code().unwrap_or_default();
                progress.fail(format!("build failed with exit code: {exit_code}"));
                return Err(NixError::Build(status));
            }
        }

        Ok(out_paths)
    }
}

#[derive(Clone)]
pub struct NixBuilder {
    nix_binary: Arc<PathBuf>,
    eval_binary: PathBuf,
}

impl NixBuilder {
    async fn eval(
        &self,
        mut progress: Item,
        set: &mut JoinSet<Result<OutPaths, NixError>>,
        platform: &str,
        systems: &[String],
        packages: &HashMap<String, String>,
    ) -> Result<(), NixError> {
        let system = try_system(platform)?;
        let Some(system) = systems.iter().find(|s| s == &&system) else {
            return Ok(());
        };

        let mut root_cmd = Command::new(&self.eval_binary);
        let cmd = root_cmd
            .arg("--verbose")
            .arg("--log-format")
            .arg("internal-json")
            .arg("--gc-roots-dir")
            .arg(std::env::temp_dir())
            .arg("--flake")
            .arg(format!(".#packages.{system}"));

        progress.message(MessageLevel::Info, format!("using platform: {system}"));

        let child = exec::spawn(cmd).await?;
        progress::proxy_stdio(child.stderr, progress.add_child("nix").into());

        let reader = BufReader::new(child.stdout);
        let mut lines = reader.lines();

        while let Some(line) = lines.next_line().await? {
            let drv: EvalResult = serde_json::from_str(&line)?;
            let attr_path = drv.attr_path.join(".");

            if packages.values().any(|v| v == &attr_path) {
                progress.init(Some(set.len() + 1), None);
                let binary = Arc::clone(&self.nix_binary);
                let progress = progress.add_child(format!("{attr_path} › nix"));
                set.spawn(drv.build(binary, progress));
            }
        }

        Ok(())
    }

    async fn detect_systems(&self, flake_path: &str) -> Result<Vec<String>, NixError> {
        let mut root_cmd = Command::new(self.nix_binary.as_os_str());
        let cmd = root_cmd
            .arg("eval")
            .arg([flake_path, "packages"].join("#"))
            .arg("--apply")
            .arg("builtins.attrNames")
            .arg("--json");

        let stdout = exec::run_with_output(cmd).await?;
        Ok(serde_json::from_str(&stdout)?)
    }
}

impl Builder for NixBuilder {
    type Error = NixError;
    type Input = Nix;

    fn try_init() -> Result<Self, Self::Error> {
        Ok(Self {
            nix_binary: option_env!("NIX_BINARY")
                .map(PathBuf::from)
                .unwrap_or(which("nix")?)
                .into(),
            eval_binary: option_env!("NIX_EVAL_JOBS_BINARY")
                .map(PathBuf::from)
                .unwrap_or(which("nix-eval-jobs")?),
        })
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
        progress.info("starting builder".to_string());

        let flake_path = input
            .flake
            .as_ref()
            .and_then(|path| path.to_str())
            .unwrap_or(".");
        let systems = self.detect_systems(flake_path).await?;

        let mut set = JoinSet::default();

        self.eval(
            progress.add_child("eval"),
            &mut set,
            &platform,
            &systems,
            &input.packages,
        )
        .await?;

        progress.done("evaluation finished".to_string());

        let out_paths =
            set.join_all()
                .await
                .into_iter()
                .try_fold(OutPaths::new(), |mut acc, paths| {
                    acc.extend(paths?);
                    progress.inc();
                    Ok::<_, NixError>(acc)
                })?;

        progress.done("finished building packages".to_string());

        let mut artifacts = HashMap::default();

        for (target, files) in out_paths {
            let artifact = input
                .packages
                .iter()
                .find(|(_, t)| t == &&target)
                .map(|(artifact, _)| artifact.clone())
                .ok_or(NixError::MissingArtifact(target))?;

            artifacts.insert(artifact, image::load_from_path(files).await?);
        }

        Ok(Output { artifacts })
    }
}
