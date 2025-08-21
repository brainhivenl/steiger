use crate::{
    builder::{bazel::BazelBuilder, docker::DockerBuilder},
    config::{Build, Config},
    image::Image,
};

use futures::TryFutureExt;
use prodash::{messages::MessageLevel, tree::Item};
use tokio::{task::JoinSet, time::Instant};

mod bazel;
mod docker;

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("docker error")]
    Docker(#[from] ErrorOf<DockerBuilder>),
    #[error("bazel error")]
    Bazel(#[from] ErrorOf<BazelBuilder>),
}

#[derive(Debug, Default)]
pub struct Output {
    pub artifacts: HashMap<String, Vec<Image>>,
}

impl Output {
    pub fn merge(&mut self, other: Output) {
        for (name, images) in other.artifacts {
            self.artifacts.insert(name, images);
        }
    }
}

pub trait Builder: Clone {
    type Error;
    type Input;

    fn try_init() -> Result<Self, Self::Error>
    where
        Self: Sized;
    async fn build(
        self,
        progress: Item,
        service_name: String,
        platform: String,
        input: Self::Input,
    ) -> Result<Output, Self::Error>;
}

type ErrorOf<T> = <T as Builder>::Error;

use std::{collections::HashMap, sync::Arc};

fn builder<B: Builder>(var: &mut Option<B>) -> Result<B, BuildError>
where
    BuildError: From<<B as Builder>::Error>,
{
    match var.clone() {
        Some(builder) => Ok(builder),
        None => {
            let builder = B::try_init()?;
            *var = Some(builder.clone());
            Ok(builder)
        }
    }
}

pub struct MetaBuild {
    config: Arc<Config>,
    bazel: Option<BazelBuilder>,
    docker: Option<DockerBuilder>,
}

impl MetaBuild {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            bazel: None,
            docker: None,
        }
    }

    pub async fn build(mut self, mut pb: Item, platform: &str) -> Result<Output, BuildError> {
        let instant = Instant::now();
        let config = Arc::clone(&self.config);
        let mut set = JoinSet::default();

        pb.init(Some(config.services.len()), None);
        pb.message(MessageLevel::Info, format!("detected platform: {platform}"));

        for (name, service) in config.services.iter() {
            let progress = pb.add_child(name);

            match &service.build {
                Build::Bazel(bazel) => {
                    let config = bazel.clone();

                    set.spawn(
                        builder(&mut self.bazel)?
                            .build(progress, name.to_string(), platform.to_string(), config)
                            .map_err(BuildError::Bazel),
                    );
                }
                Build::Docker(docker) => {
                    let config = docker.clone();

                    set.spawn(
                        builder(&mut self.docker)?
                            .build(progress, name.to_string(), platform.to_string(), config)
                            .map_err(BuildError::Docker),
                    );
                }
            };
        }

        let mut output = Output::default();

        while let Some(Ok(result)) = set.join_next().await {
            pb.inc();
            output.merge(result?);
        }

        let elapsed = instant.elapsed();

        pb.message(
            MessageLevel::Info,
            format!("build completed in {elapsed:?}"),
        );

        Ok(output)
    }
}
