use std::sync::Arc;

use futures::TryFutureExt;
use miette::Diagnostic;
use prodash::tree::Item;
use tokio::{task::JoinSet, time::Instant};

use crate::{
    cmd::build::output::Output,
    config::{Config, Release},
    deploy::helm::HelmDeployer,
};

pub mod helm;

pub struct Context<T> {
    pub input: T,
    pub output: Arc<Output>,
}

impl<T> Context<T> {
    pub fn new(input: T, output: Arc<Output>) -> Self {
        Self { input, output }
    }
}

pub trait Deployer: Clone {
    type Error;
    type Input;

    fn try_init() -> Result<Self, Self::Error>
    where
        Self: Sized;
    async fn validate(&self, input: &Self::Input) -> Result<(), Self::Error>;
    async fn deploy(
        self,
        progress: Item,
        release: String,
        input: Context<Self::Input>,
    ) -> Result<(), Self::Error>;
}

type ErrorOf<T> = <T as Deployer>::Error;

#[derive(Debug, Diagnostic, thiserror::Error)]
#[error("one or more deployments failed")]
pub struct MultiError {
    #[related]
    pub errors: Vec<DeployError>,
}

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum DeployError {
    #[error("helm error")]
    #[diagnostic(transparent)]
    Helm(#[from] ErrorOf<HelmDeployer>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Multi(MultiError),
}

fn ensure<T: Deployer>(deploy: &Option<T>) -> T {
    match deploy {
        Some(deploy) => deploy.clone(),
        None => unreachable!(),
    }
}

pub struct MetaDeployer {
    config: Config,
    output: Arc<Output>,
    helm: Option<HelmDeployer>,
}

impl MetaDeployer {
    pub fn new(config: Config, output: Arc<Output>) -> Self {
        Self {
            config,
            output,
            helm: None,
        }
    }

    pub async fn validate(&mut self, pb: &mut Item) -> Result<(), DeployError> {
        pb.info("validating releases");

        for release in self.config.deploy.values() {
            match release {
                Release::Helm(helm) => {
                    if self.helm.is_none() {
                        self.helm = Some(HelmDeployer::try_init()?)
                    }

                    ensure(&self.helm).validate(helm).await?;
                }
            }
        }

        Ok(())
    }

    pub async fn deploy(self, mut pb: Item) -> Result<(), DeployError> {
        let instant = Instant::now();
        let mut set = JoinSet::<Result<_, DeployError>>::new();

        pb.init(Some(self.config.deploy.len()), None);
        pb.info("starting deployment");

        for (name, release) in self.config.deploy {
            let progress = pb.add_child(&name);

            match release {
                Release::Helm(helm) => {
                    set.spawn(
                        ensure(&self.helm)
                            .deploy(progress, name, Context::new(helm, Arc::clone(&self.output)))
                            .map_err(DeployError::Helm),
                    );
                }
            }
        }

        let mut errors = vec![];

        while let Some(Ok(result)) = set.join_next().await {
            pb.inc();

            if let Err(e) = result {
                pb.fail("deployment failed");
                errors.push(e);
            }
        }

        if !errors.is_empty() {
            return Err(DeployError::Multi(MultiError { errors }));
        }

        let elapsed = instant.elapsed();

        pb.done(format!("deployment completed in {elapsed:?}"));

        Ok(())
    }
}
