use std::{collections::HashMap, mem, path::Path};

use miette::Diagnostic;
use serde::Deserialize;
use serde_yml::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub services: HashMap<String, Service>,
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

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Build {
    Ko(Ko),
    Bazel(Bazel),
    Docker(Docker),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default, flatten)]
    pub vars: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub build: Build,
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

fn template(vars: &HashMap<String, String>, config: &mut Value) -> Result<(), subst::Error> {
    match config {
        Value::String(s) => {
            if s.contains('$') {
                *s = subst::substitute(s, vars)?;
            }
        }
        Value::Sequence(seq) => {
            for item in seq.iter_mut() {
                template(vars, item)?;
            }
        }
        Value::Mapping(map) => {
            for (_, value) in map.iter_mut() {
                template(vars, value)?;
            }
        }
        _ => {}
    }

    Ok(())
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
                template(&profile.vars, &mut config)?;
            }
            None => return Err(ConfigError::Profile(profile.to_string())),
        };
    }

    Ok(serde_yml::from_value(config)?)
}
