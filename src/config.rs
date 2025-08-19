use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub services: HashMap<String, Service>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bazel {
    pub targets: HashMap<String, String>,
    #[serde(default)]
    pub platforms: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Build {
    Bazel(Bazel),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub build: Build,
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub name: String,
}
