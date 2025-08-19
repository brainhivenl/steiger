use crate::{
    builder::bazel::BazelBuilder,
    config::{Build, Config},
    image::Image,
};

use prodash::{messages::MessageLevel, tree::Item};
use tokio::task::JoinSet;

mod bazel;

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("bazel error: {0}")]
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

pub struct MetaBuild {
    config: Arc<Config>,
    bazel: Option<BazelBuilder>,
}

impl MetaBuild {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            bazel: None,
        }
    }

    fn bazel(&mut self) -> Result<BazelBuilder, BuildError> {
        match self.bazel.clone() {
            Some(builder) => Ok(builder),
            None => {
                let builder = BazelBuilder::try_init()?;
                self.bazel = Some(builder.clone());
                Ok(builder)
            }
        }
    }

    pub async fn build(mut self, mut pb: Item, platform: String) -> Result<Output, BuildError> {
        let config = Arc::clone(&self.config);
        let mut set = JoinSet::default();

        pb.init(Some(config.services.len()), None);
        pb.message(MessageLevel::Info, format!("detected platform: {platform}"));

        for (name, service) in config.services.iter() {
            let progress = pb.add_child(name);

            match &service.build {
                Build::Bazel(bazel) => {
                    let builder = self.bazel()?;
                    let config = bazel.clone();

                    set.spawn(builder.build(progress, name.to_string(), platform.clone(), config));
                }
            };
        }

        let mut output = Output::default();

        for result in set.join_all().await {
            pb.inc();
            output.merge(result?);
        }

        Ok(output)
    }
}
