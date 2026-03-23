use std::collections::HashMap;
use std::fmt;

use miette::Diagnostic;
use prodash::tree::Item;
use tokio::task::JoinSet;

use crate::{
    build::{
        bazel::BazelBuilder, docker::DockerBuilder, ko::KoBuilder,
        nix::NixBuilder,
    },
    config::{Build, Config},
    image::Image,
};

mod bazel;
mod docker;
pub(crate) mod events;
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
    #[error("build events error")]
    #[diagnostic(transparent)]
    Events(#[from] events::ClientError),
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
    progress: Item,
}

impl Context {
    pub fn new(service_name: String, platform: String, progress: Item) -> Self {
        Self {
            service_name,
            platform,
            progress,
        }
    }

    /// Create a child progress item for subprocess output.
    pub fn child_progress(&mut self, label: &str) -> Item {
        self.progress.add_child(format!("{} › {label}", self.service_name))
    }

    fn start(&mut self) {
        self.progress.set_name(&self.service_name);
        self.progress.info("starting builder");
    }

    fn done(&mut self) {
        self.progress.done("build finished");
    }

    fn fail(&mut self, error: &impl fmt::Display) {
        self.progress.fail(format!("{error}"));
    }
}

pub trait Builder: Clone {
    type Error: fmt::Display;
    type Input;

    fn try_init() -> Result<Self, Self::Error>
    where
        Self: Sized;
    async fn build(self, ctx: &mut Context, input: Self::Input) -> Result<Output, Self::Error>;
}

type ErrorOf<T> = <T as Builder>::Error;

async fn run_builder<B>(builder: B, mut ctx: Context, input: B::Input) -> Result<Output, BuildError>
where
    B: Builder,
    BuildError: From<<B as Builder>::Error>,
{
    ctx.start();

    match builder.build(&mut ctx, input).await {
        Ok(output) => {
            ctx.done();
            Ok(output)
        }
        Err(e) => {
            ctx.fail(&e);
            Err(e.into())
        }
    }
}

fn init_builder<B: Builder>(cache: &mut Option<B>) -> Result<B, B::Error> {
    match cache.clone() {
        Some(builder) => Ok(builder),
        None => {
            let builder = B::try_init()?;
            *cache = Some(builder.clone());
            Ok(builder)
        }
    }
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
        let mut set = JoinSet::default();

        pb.init(Some(self.config.build.len()), None);
        pb.info(format!("detected platform: {platform}"));

        for (name, build) in self.config.build {
            let progress = pb.add_child(&name);
            let ctx = Context::new(name, platform.to_string(), progress);

            match build {
                Build::Ko(ko) => {
                    let builder = init_builder(&mut self.ko)?;
                    set.spawn(run_builder(builder, ctx, ko));
                }
                Build::Bazel(bazel) => {
                    let builder = init_builder(&mut self.bazel)?;
                    set.spawn(run_builder(builder, ctx, bazel));
                }
                Build::Docker(docker) => {
                    let builder = init_builder(&mut self.docker)?;
                    set.spawn(run_builder(builder, ctx, docker));
                }
                Build::Nix(nix) => {
                    let builder = init_builder(&mut self.nix)?;
                    set.spawn(run_builder(builder, ctx, nix));
                }
            };
        }

        let mut output = Output::default();

        while let Some(Ok(result)) = set.join_next().await {
            pb.inc();
            output.merge(result?);
        }

        Ok(output)
    }
}
