use std::{
    collections::HashMap,
    mem,
    path::{Path, PathBuf},
};

use miette::Diagnostic;
use serde::Deserialize;
use serde_yml::{Mapping, Value};

use crate::git;

const DEFAULT_TAG_FORMAT: &str = "${gitTag:$gitShortCommit}${gitDirty:}";

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub build: HashMap<String, Build>,
    #[serde(default)]
    pub deploy: HashMap<String, Release>,
    #[serde(default)]
    pub insecure_registries: Vec<String>,
    pub default_repo: Option<String>,
    #[serde(default)]
    pub tag_format: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bazel {
    pub targets: HashMap<String, String>,
    pub platforms: HashMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Docker {
    pub context: String,
    pub dockerfile: Option<String>,
    #[serde(default)]
    pub build_args: HashMap<String, String>,
    #[serde(default)]
    pub hosts: HashMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ko {
    pub import_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Nix {
    pub packages: HashMap<String, String>,
    pub flake: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Build {
    Ko(Ko),
    Bazel(Bazel),
    Docker(Docker),
    Nix(Nix),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Helm {
    pub path: String,
    pub namespace: Option<String>,
    pub timeout: Option<String>,
    #[serde(default)]
    pub values: HashMap<String, String>,
    #[serde(default)]
    pub values_files: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Release {
    Helm(Helm),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default, flatten)]
    pub vars: HashMap<String, String>,
}

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum ConfigError {
    #[error("I/O error")]
    IO(#[from] std::io::Error),
    #[error("substitution failed")]
    Subst(#[from] subst::Error),
    #[error("failed to deserialize")]
    Yaml(#[from] serde_yml::Error),
    #[error("failed to parse git status")]
    Git(#[from] git::GitError),
    #[error("profile '{0}' does not exist")]
    Profile(String),
}

fn template(vars: &HashMap<String, String>, config: Value) -> Result<Value, subst::Error> {
    match config {
        Value::String(s) => Ok(Value::String(if s.contains('$') {
            subst::substitute(&s, vars)?
        } else {
            s
        })),
        Value::Sequence(seq) => seq
            .into_iter()
            .map(|c| template(vars, c))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Sequence),
        Value::Mapping(map) => map
            .into_iter()
            .map(|(key, value)| Ok((template(vars, key)?, template(vars, value)?)))
            .collect::<Result<Mapping, _>>()
            .map(Value::Mapping),
        _ => Ok(config),
    }
}

fn extract_git_vars(state: git::State) -> HashMap<String, String> {
    let mut vars = HashMap::new();

    vars.insert("gitShortCommit".to_string(), state.commit[0..6].to_string());
    vars.insert("gitCommit".to_string(), state.commit);
    if let Some(tag) = state.tag {
        vars.insert("gitTag".to_string(), tag);
    }
    if state.dirty {
        vars.insert("gitDirty".to_string(), "-dirty".to_string());
    }

    vars
}

pub async fn load_from_path(
    profile: Option<&str>,
    path: impl AsRef<Path>,
) -> Result<Config, ConfigError> {
    let mut vars = extract_git_vars(git::state().await?);
    let data = tokio::fs::read(path).await?;
    let mut config = serde_yml::from_slice::<Value>(&data)?;

    if let Some(profile) = profile {
        let profile = serde_yml::from_value::<Profile>(mem::take(
            config
                .get_mut("profiles")
                .and_then(|profiles| profiles.get_mut(profile))
                .ok_or_else(|| ConfigError::Profile(profile.to_string()))?,
        ))?;

        vars.extend(profile.vars);
    }

    let mut config = serde_yml::from_value::<Config>(template(&vars, config)?)?;

    if config.tag_format.is_empty() {
        config.tag_format = subst::substitute(DEFAULT_TAG_FORMAT, &vars)?;
    }

    Ok(config)
}
