use std::{env, sync::Arc, time::Duration};

use base64::{Engine, prelude::BASE64_STANDARD};
use reqwest::header::{HeaderMap, InvalidHeaderValue};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum TagDiscoveryError {
    #[error("discovery failed: {0}")]
    Discover(#[from] gix::discover::Error),
    #[error("failed to find reference: {0}")]
    FindReference(#[from] gix::reference::find::existing::Error),
    #[error("detached head")]
    DetachedHead,
}

#[derive(Debug, Serialize)]
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

impl Tags {
    pub fn try_discover() -> Result<Tags, TagDiscoveryError> {
        let repo = gix::discover(".")?;
        let head = repo.head()?;
        let id = head.id().ok_or(TagDiscoveryError::DetachedHead)?;

        let git_rev = id.to_string();
        let git_refname = head
            .try_into_referent()
            .ok_or(TagDiscoveryError::DetachedHead)?
            .name()
            .as_bstr()
            .to_string();
        let github_repo = env::var("GITHUB_REPOSITORY").ok();
        let github_workflow = env::var("GITHUB_WORKFLOW").ok();

        Ok(Tags {
            git_rev,
            git_refname,
            github_repo,
            github_workflow,
        })
    }
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

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    Progress {
        phase: String,
        total: i64,
        current: i64,
    },
    Artifact {
        uri: String,
    },
    Completed {
        elapsed: Duration,
    },
}

#[derive(Serialize)]
pub struct CreateEventRequest<'a> {
    pub event: &'a Event,
}

#[derive(Debug, Deserialize, thiserror::Error)]
#[error("{error}")]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, miette::Diagnostic, thiserror::Error)]
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
        headers.insert(
            "authorization",
            format!("Basic {}", BASE64_STANDARD.encode(api_token)).parse()?,
        );

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

    async fn post<I, O>(&self, url: &str, body: I) -> Result<O, ClientError>
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
        &self,
        request: &CreateBuildRequest,
    ) -> Result<CreateBuildResponse, ClientError> {
        self.post(&format!("{}/builds", self.base_url), request)
            .await
    }

    pub async fn create_event(&self, build_id: &Uuid, event: &Event) -> Result<(), ClientError> {
        self.post(
            &format!("{}/builds/{build_id}/events", self.base_url),
            CreateEventRequest { event },
        )
        .await
    }
}
