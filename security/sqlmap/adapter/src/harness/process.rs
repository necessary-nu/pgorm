use super::Result;
use std::{
    process::{ExitStatus, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
    time::timeout,
};

/// Own a process group so cancellation also terminates scanner descendants.
pub struct Process {
    pub child: Child,
    group: i32,
}

impl Process {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        command.process_group(0).kill_on_drop(true);
        let child = command.spawn()?;
        let group = child.id().ok_or("spawned process has no pid")? as i32;
        Ok(Self { child, group })
    }

    pub async fn stop(&mut self) -> Result<()> {
        self.signal(libc::SIGTERM)?;
        match timeout(Duration::from_secs(5), self.child.wait()).await {
            Ok(status) => {
                status?;
            }
            Err(_) => {
                self.signal(libc::SIGKILL)?;
                timeout(Duration::from_secs(5), self.child.wait()).await??;
            }
        }
        // The group can outlive its leader.
        self.signal(libc::SIGKILL)?;
        Ok(())
    }

    fn signal(&self, signal: i32) -> Result<()> {
        // SAFETY: group is a positive pid returned by our own process_group(0) spawn.
        if unsafe { libc::kill(-self.group, signal) } == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error.into());
            }
        }
        Ok(())
    }

    pub async fn wait(&mut self, seconds: u64) -> Result<Option<ExitStatus>> {
        match timeout(Duration::from_secs(seconds), self.child.wait()).await {
            Ok(result) => Ok(Some(result?)),
            Err(_) => {
                self.stop().await?;
                Ok(None)
            }
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.signal(libc::SIGKILL);
    }
}

pub fn command(program: &str, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    // The external scanner never inherits unrelated database credentials.
    for name in [
        "DATABASE_URL",
        "PGPASSWORD",
        "POSTGRES_PASSWORD",
        "SQLMAP_FIXTURE_URL",
    ] {
        command.env_remove(name);
    }
    command
}

pub async fn output(command: &mut Command, input: Option<&str>, seconds: u64) -> Result<String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut process = Process::spawn(command)?;
    let mut stdout = process.child.stdout.take().ok_or("missing child stdout")?;
    let mut stderr = process.child.stderr.take().ok_or("missing child stderr")?;
    let mut stdin = process.child.stdin.take();
    let operation = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let send = async {
            if let (Some(pipe), Some(input)) = (&mut stdin, input) {
                pipe.write_all(input.as_bytes()).await?;
                pipe.shutdown().await?;
            }
            stdin.take();
            Ok::<_, std::io::Error>(())
        };
        let (_, _, _, status) = tokio::try_join!(
            send,
            stdout.read_to_end(&mut out),
            stderr.read_to_end(&mut err),
            process.child.wait()
        )?;
        if !status.success() {
            return Err(format!(
                "command failed ({status}): {}",
                String::from_utf8_lossy(&err).trim()
            )
            .into());
        }
        Ok(String::from_utf8(out)?.trim().to_owned())
    };
    match timeout(Duration::from_secs(seconds), operation).await {
        Ok(result) => result,
        Err(_) => {
            process.stop().await?;
            Err(format!("command deadline exceeded ({seconds}s)").into())
        }
    }
}

pub async fn run(program: &str, args: &[&str]) -> Result<String> {
    output(&mut command(program, args), None, 120).await
}

pub async fn http(url: &str) -> Result<(u16, serde_json::Value)> {
    let response = run(
        "curl",
        &[
            "--silent",
            "--show-error",
            "--max-time",
            "20",
            "--noproxy",
            "*",
            "--write-out",
            "\n%{http_code}",
            url,
        ],
    )
    .await?;
    let (body, status) = response.rsplit_once('\n').ok_or("missing HTTP status")?;
    Ok((status.parse()?, serde_json::from_str(body)?))
}
