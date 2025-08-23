use crate::{
    builder::{bazel::BazelBuilder, docker::DockerBuilder, ko::KoBuilder},
    config::{Build, Config},
    image::Image,
};

use futures::TryFutureExt;
use miette::Diagnostic;
use prodash::{messages::MessageLevel, tree::Item};
use tokio::{task::JoinSet, time::Instant};

mod bazel;
mod docker;
mod ko;

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum BuildError {
    #[error("ko error")]
    #[diagnostic(transparent)]
    Ko(#[from] ErrorOf<KoBuilder>),
    #[error("docker error")]
    #[diagnostic(transparent)]
    Docker(#[from] ErrorOf<DockerBuilder>),
    #[error("bazel error")]
    #[diagnostic(transparent)]
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

pub struct Context<T> {
    pub service_name: String,
    pub platform: String,
    pub input: T,
}

impl<T> Context<T> {
    pub fn new(service_name: String, platform: String, input: T) -> Self {
        Self {
            service_name,
            platform,
            input,
        }
    }

    pub fn with<I>(self, input: I) -> Context<I> {
        Context {
            input,
            platform: self.platform,
            service_name: self.service_name,
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
        input: Context<Self::Input>,
    ) -> Result<Output, Self::Error>;
}

type ErrorOf<T> = <T as Builder>::Error;

use std::{collections::HashMap, sync::Arc};

fn run_builder<B>(
    var: &mut Option<B>,
    progress: Item,
    ctx: Context<B::Input>,
) -> Result<impl Future<Output = Result<Output, <B as Builder>::Error>> + use<B>, BuildError>
where
    B: Builder,
    BuildError: From<<B as Builder>::Error>,
{
    let builder = match var.clone() {
        Some(builder) => Ok(builder),
        None => {
            let builder = B::try_init()?;
            *var = Some(builder.clone());
            Ok(builder)
        }
    }?;

    Ok(builder.build(progress, ctx))
}

pub struct MetaBuild {
    config: Arc<Config>,
    ko: Option<KoBuilder>,
    bazel: Option<BazelBuilder>,
    docker: Option<DockerBuilder>,
}

impl MetaBuild {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            ko: None,
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
            let ctx = Context::new(name.clone(), platform.to_string(), ());

            match &service.build {
                Build::Ko(ko) => {
                    set.spawn(
                        run_builder(&mut self.ko, progress, ctx.with(ko.clone()))?
                            .map_err(BuildError::Ko),
                    );
                }
                Build::Bazel(bazel) => {
                    set.spawn(
                        run_builder(&mut self.bazel, progress, ctx.with(bazel.clone()))?
                            .map_err(BuildError::Bazel),
                    );
                }
                Build::Docker(docker) => {
                    set.spawn(
                        run_builder(&mut self.docker, progress, ctx.with(docker.clone()))?
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
