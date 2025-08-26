use std::{
    process::{ExitStatus, Stdio},
    sync::Arc,
};

use miette::Diagnostic;
use prodash::Progress;
use tokio::{
    io::AsyncReadExt,
    process::{Child, ChildStderr, ChildStdout, Command},
};

use crate::progress;

pub struct ChildWithStdio {
    pub inner: Child,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
}

impl ChildWithStdio {
    async fn stdout(&mut self) -> Result<String, std::io::Error> {
        let mut output = String::new();
        self.stdout.read_to_string(&mut output).await?;
        Ok(output)
    }

    async fn stderr(&mut self) -> Result<String, std::io::Error> {
        let mut output = String::new();
        self.stderr.read_to_string(&mut output).await?;
        Ok(output)
    }
}

pub async fn spawn(cmd: &mut Command) -> Result<ChildWithStdio, std::io::Error> {
    let mut inner = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = inner.stdout.take().unwrap();
    let stderr = inner.stderr.take().unwrap();

    Ok(ChildWithStdio {
        inner,
        stdout,
        stderr,
    })
}

pub async fn run_with_progress<P>(
    cmd: &mut Command,
    progress: P,
) -> Result<ExitStatus, std::io::Error>
where
    P: Progress + 'static,
{
    let progress = Arc::new(progress);
    let mut child = spawn(cmd).await?;

    progress::proxy_stdio(child.stdout, Arc::clone(&progress));
    progress::proxy_stdio(child.stderr, Arc::clone(&progress));

    child.inner.wait().await
}

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum ExitError {
    #[error("IO error")]
    IO(#[from] std::io::Error),
    #[error("command failed with code '{code}': {stderr}")]
    Status { code: i32, stderr: String },
}

pub async fn run_with_output(cmd: &mut Command) -> Result<String, ExitError> {
    let mut child = spawn(cmd).await?;
    let status = child.inner.wait().await?;

    if status.success() {
        return Ok(child.stdout().await?);
    }

    let stderr = child.stderr().await?;

    Err(ExitError::Status {
        code: status.code().unwrap_or_default(),
        stderr,
    })
}
