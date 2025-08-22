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
    pub platforms: HashMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Docker {
    pub context: String,
    pub dockerfile: Option<String>,
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
pub struct Service {
    pub build: Build,
}
