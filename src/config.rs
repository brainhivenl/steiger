use std::{
    collections::HashMap,
    mem,
    path::{Path, PathBuf},
};

use miette::Diagnostic;
use serde::Deserialize;
use serde_yml::{Mapping, Value};

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub insecure_registries: Vec<String>,
    pub build: HashMap<String, Build>,
    #[serde(default)]
    pub deploy: HashMap<String, Release>,
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

pub async fn load_from_path(
    profile: Option<&str>,
    path: impl AsRef<Path>,
) -> Result<Config, ConfigError> {
    let data = tokio::fs::read(path).await?;
    let mut config = serde_yml::from_slice::<Value>(&data)?;

    if let Some(profile) = profile {
        match config
            .get_mut("profiles")
            .and_then(|profiles| profiles.get_mut(profile))
        {
            Some(profile) => {
                let profile = serde_yml::from_value::<Profile>(mem::take(profile))?;
                config = template(&profile.vars, config)?;
            }
            None => return Err(ConfigError::Profile(profile.to_string())),
        };
    }

    Ok(serde_yml::from_value(config)?)
}
