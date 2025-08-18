use std::sync::Arc;

use prodash::{
    Progress,
    messages::MessageLevel,
    render::line::JoinHandle,
    tree::{Root, root::Options},
};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

pub fn tree() -> Arc<Root> {
    Arc::new(
        Options {
            message_buffer_capacity: 200,
            ..Default::default()
        }
        .into(),
    )
}

pub fn setup_line_renderer(progress: &Arc<Root>) -> JoinHandle {
    prodash::render::line(
        std::io::stderr(),
        std::sync::Arc::downgrade(progress),
        prodash::render::line::Options {
            frames_per_second: 6.0,
            initial_delay: None,
            throughput: true,
            hide_cursor: false,
            ..prodash::render::line::Options::default()
        }
        .auto_configure(prodash::render::line::StreamKind::Stderr),
    )
}

pub fn proxy_stdio<R, P>(reader: R, progress: Arc<P>)
where
    R: AsyncRead + Unpin + Send + 'static,
    P: Progress + 'static,
{
    let mut lines = BufReader::new(reader).lines();

    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            progress.message(MessageLevel::Info, line);
        }
    });
}
