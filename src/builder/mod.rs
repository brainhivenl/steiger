use miette::Diagnostic;
use prodash::tree::Item;
use tokio::{task::JoinSet, time::Instant};

use crate::{
    builder::{bazel::BazelBuilder, docker::DockerBuilder, ko::KoBuilder, nix::NixBuilder},
    config::{Build, Config},
    image::Image,
};

mod bazel;
mod docker;
mod ko;
mod nix;

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum BuildError {
    #[error("ko error")]
    #[diagnostic(transparent)]
    Ko(#[from] ErrorOf<KoBuilder>),
    #[error("bazel error")]
    #[diagnostic(transparent)]
    Bazel(#[from] ErrorOf<BazelBuilder>),
    #[error("docker error")]
    #[diagnostic(transparent)]
    Docker(#[from] ErrorOf<DockerBuilder>),
    #[error("nix error")]
    #[diagnostic(transparent)]
    Nix(#[from] ErrorOf<NixBuilder>),
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

pub struct Context {
    pub service_name: String,
    pub platform: String,
    pub progress: Item,
}

impl Context {
    pub fn new(service_name: String, platform: String, progress: Item) -> Self {
        Self {
            service_name,
            platform,
            progress,
        }
    }
}

pub trait Builder: Clone {
    type Error;
    type Input;

    fn try_init() -> Result<Self, Self::Error>
    where
        Self: Sized;
    async fn build(self, ctx: Context, input: Self::Input) -> Result<Output, Self::Error>;
}

type ErrorOf<T> = <T as Builder>::Error;

use std::collections::HashMap;

fn run_builder<B>(
    var: &mut Option<B>,
    ctx: Context,
    input: B::Input,
) -> Result<impl Future<Output = Result<Output, BuildError>> + use<B>, BuildError>
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

    Ok(async { Ok(builder.build(ctx, input).await?) })
}

pub struct MetaBuild {
    config: Config,
    ko: Option<KoBuilder>,
    bazel: Option<BazelBuilder>,
    docker: Option<DockerBuilder>,
    nix: Option<NixBuilder>,
}

impl MetaBuild {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            ko: None,
            bazel: None,
            docker: None,
            nix: None,
        }
    }

    pub async fn build(mut self, mut pb: Item, platform: &str) -> Result<Output, BuildError> {
        let instant = Instant::now();
        let mut set = JoinSet::default();

        pb.init(Some(self.config.build.len()), None);
        pb.info(format!("detected platform: {platform}"));

        for (name, build) in self.config.build {
            let progress = pb.add_child(&name);
            let ctx = Context::new(name, platform.to_string(), progress);

            match build {
                Build::Ko(ko) => {
                    set.spawn(run_builder(&mut self.ko, ctx, ko)?);
                }
                Build::Bazel(bazel) => {
                    set.spawn(run_builder(&mut self.bazel, ctx, bazel)?);
                }
                Build::Docker(docker) => {
                    set.spawn(run_builder(&mut self.docker, ctx, docker)?);
                }
                Build::Nix(nix) => {
                    set.spawn(run_builder(&mut self.nix, ctx, nix)?);
                }
            };
        }

        let mut output = Output::default();

        while let Some(Ok(result)) = set.join_next().await {
            pb.inc();
            output.merge(result?);
        }

        let elapsed = instant.elapsed();

        pb.done(format!("build completed in {elapsed:?}"));

        Ok(output)
    }
}
