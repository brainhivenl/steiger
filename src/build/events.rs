use std::{env, sync::Arc, time::Duration};

use reqwest::header::{HeaderMap, InvalidHeaderValue};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tags {
    #[serde(rename = "git.rev")]
    pub git_rev: String,
    #[serde(rename = "git.refname")]
    pub git_refname: String,
    #[serde(rename = "github.repo")]
    pub github_repo: Option<String>,
    #[serde(rename = "github.workflow")]
    pub github_workflow: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBuildRequest {
    pub target: String,
    pub tags: Tags,
}

#[derive(Deserialize)]
pub struct CreateBuildResponse {
    pub id: Uuid,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedEvent {
    pub elapsed: Duration,
}

#[derive(Serialize)]
pub struct ProgressEvent {
    pub phase: String,
    pub total: i64,
    pub current: i64,
}

#[derive(Serialize)]
pub struct ArtifactEvent {
    pub uri: String,
}

#[derive(Serialize)]
pub enum Event {
    Progress(ProgressEvent),
    Artifact(ArtifactEvent),
    Completed(CompletedEvent),
}

#[derive(Serialize)]
pub struct CreateEventRequest {
    pub event: Event,
}

#[derive(Debug, Deserialize, thiserror::Error)]
#[error("{error}")]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("API error: {0}")]
    Response(#[from] ErrorResponse),
    #[error("request failed: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("failed to (de)serialize: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("invalid header value: {0}")]
    Header(#[from] InvalidHeaderValue),
}

#[derive(Clone)]
pub struct Client {
    base_url: Arc<String>,
    http: reqwest::Client,
}

impl Client {
    pub fn new(base_url: String, api_token: &str) -> Result<Self, ClientError> {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {api_token}").parse()?);

        Ok(Self {
            base_url: Arc::new(base_url),
            http: reqwest::Client::builder()
                .default_headers(headers)
                .build()?,
        })
    }

    pub fn from_env() -> Option<Self> {
        let base_url = env::var("BUILD_EVENTS_ENDPOINT").ok();
        let api_token = env::var("BUILD_EVENTS_TOKEN").ok();

        match (base_url, api_token) {
            (Some(base_url), Some(api_token)) => Self::new(base_url, &api_token).ok(),
            _ => None,
        }
    }

    async fn post<I, O>(&mut self, url: &str, body: I) -> Result<O, ClientError>
    where
        I: Serialize,
        O: DeserializeOwned,
    {
        let response = self.http.post(url).json(&body).send().await?;

        if response.status().is_success() {
            return Ok(response.json().await?);
        }

        Err(response.json::<ErrorResponse>().await?.into())
    }

    pub async fn create_build(
        &mut self,
        request: &CreateBuildRequest,
    ) -> Result<CreateBuildResponse, ClientError> {
        self.post(&format!("{}/builds", self.base_url), request)
            .await
    }

    pub async fn create_event(
        &mut self,
        build_id: Uuid,
        request: &CreateEventRequest,
    ) -> Result<(), ClientError> {
        self.post(
            &format!("{}/builds/{build_id}/events", self.base_url),
            request,
        )
        .await
    }
}
