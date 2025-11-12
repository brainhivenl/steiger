use std::{collections::HashMap, mem, path::Path};

use docker_credential::CredentialRetrievalError;
use miette::Diagnostic;
use oci_client::Reference;
use tokio::{fs, task::JoinSet, time::Instant};

use crate::{
    build::{
        BuildError, MetaBuild,
        events::{Client as EventsClient, CreateBuildRequest, Event, Tags},
    },
    config::Config,
    git,
    image::Image,
    progress,
    registry::{self, PushError, Registry},
};

pub mod output {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Build {
        pub image_name: String,
        pub tag: String,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Output {
        pub builds: Vec<Build>,
    }
}

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum WriteError {
    #[error("failed to write to file")]
    IO(#[from] std::io::Error),
    #[error("failed to serialize output")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Git(#[from] git::GitError),
    #[error("failed to build")]
    #[diagnostic(transparent)]
    Build(#[from] BuildError),
    #[error("failed to send build event")]
    #[diagnostic(transparent)]
    BuildEvent(#[from] crate::build::events::ClientError),
    #[error("failed to push")]
    #[diagnostic(transparent)]
    Push(#[from] PushError),
    #[error("failed to find image for platform")]
    NoImage(String),
    #[error("failed to write output")]
    #[diagnostic(transparent)]
    WriteOutput(#[from] WriteError),
    #[error("failed to retrieve registry credentials")]
    Credential(#[from] CredentialRetrievalError),
    #[error("failed to parse reference")]
    Parse(#[from] oci_client::ParseError),
}

fn find_image(mut images: Vec<Image>, platform: &str) -> Result<Image, Error> {
    if let Some(n) = images.iter().position(
        |i| matches!(i.platform, Some(ref p) if format!("{}/{}", p.os, p.architecture) == platform),
    ) {
        return Ok(images.remove(n));
    }

    images
        .into_iter()
        .find(|i| i.platform.is_none())
        .ok_or(Error::NoImage(platform.to_string()))
}

pub async fn run(
    mut config: Config,
    platform: String,
    repo: Option<String>,
    output_file: Option<&Path>,
) -> Result<(), Error> {
    let root = progress::tree();
    let handle = progress::setup_line_renderer(&root);
    let insecure_registries = mem::take(&mut config.insecure_registries);

    let (tag, default_repo) = (config.tag_format.clone(), config.default_repo.take());
    let events = EventsClient::from_env();
    let builder = MetaBuild::new(config);

    let now = Instant::now();
    let output = builder.build(root.add_child("build"), &platform).await?;

    let mut build_id = None;
    if let Some(ref client) = events
        && let Ok(tags) = Tags::try_discover()
        && let Ok(target) = std::env::var("BUILD_EVENTS_TARGET")
    {
        let response = client
            .create_build(&CreateBuildRequest { target, tags })
            .await.unwrap();

        build_id = Some(response.id);
    }

    let Some(repo) = repo.or(default_repo) else {
        handle.shutdown_and_wait();
        println!("no repo set, skipping push");
        return Ok(());
    };

    let mut progress = root.add_child("push");
    progress.init(Some(output.artifacts.len()), None);

    let auth = registry::load_credentials(&repo)?;
    let registry = Registry::with_config(auth, &insecure_registries);
    let mut artifacts = HashMap::new();
    let mut set = JoinSet::<Result<_, PushError>>::new();

    for (artifact, images) in output.artifacts {
        let image = find_image(images, &platform)?;
        let pb = progress.add_child(format!("{artifact} › push"));
        let image_ref = Reference::try_from(format!("{repo}/{artifact}:{tag}"))?;
        let output_ref = format!("{repo}/{artifact}:{tag}@{}", image.digest);
        let mut registry = registry.clone();

        set.spawn(async move {
            registry.push(pb, &image_ref, image).await?;
            Ok((artifact, output_ref))
        });
    }

    while let Some(Ok(result)) = set.join_next().await {
        let (artifact, uri) = result?;
        artifacts.insert(artifact, uri.clone());

        if let Some(ref client) = events
            && let Some(ref id) = build_id
        {
            client.create_event(id, &Event::Artifact { uri }).await?;
        }

        progress.inc();
    }

    let elapsed = now.elapsed();
    progress.done(format!("build completed in {elapsed:?}"));

    handle.shutdown_and_wait();

    if let Some(ref client) = events
        && let Some(ref id) = build_id
    {
        client
            .create_event(id, &Event::Completed { elapsed })
            .await?;
    }

    println!("\nPushed artifacts:");

    for (artifact, image_ref) in artifacts.iter() {
        println!("- {artifact}: {image_ref}");
    }

    if let Some(path) = output_file {
        let output = output::Output {
            builds: artifacts
                .into_iter()
                .map(|(image_name, tag)| output::Build { image_name, tag })
                .collect(),
        };

        let data = serde_json::to_vec(&output).map_err(WriteError::Serde)?;
        fs::write(path, data).await.map_err(WriteError::IO)?;
    }

    Ok(())
}
