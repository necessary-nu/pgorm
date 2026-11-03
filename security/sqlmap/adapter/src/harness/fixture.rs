use super::{
    Result, Settings,
    process::{self, Process},
    read_json, write_json,
};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    time::{Instant, sleep, timeout},
};

fn secret(bytes: usize) -> Result<String> {
    let mut data = vec![0; bytes];
    File::open("/dev/urandom")?.read_exact(&mut data)?;
    Ok(data.iter().map(|b| format!("{b:02x}")).collect())
}

// [spec:pgorm:req:security.sqlmap.isolation]
// [spec:pgorm:req:security.sqlmap.fixtures]
pub struct Fixture {
    pub name: String,
    pub url: String,
    pub settings: Value,
    admin_url: String,
    /// The profile the server was started under, whose session settings every role shares.
    profile: Option<Settings>,
    adapter: Option<Process>,
    container_attempted: bool,
    artifacts: PathBuf,
}

impl Fixture {
    pub fn new(artifacts: &Path) -> Result<Self> {
        Ok(Self {
            name: format!("pgorm-sqlmap-{}", secret(6)?),
            url: String::new(),
            settings: Value::Null,
            admin_url: String::new(),
            profile: None,
            adapter: None,
            container_attempted: false,
            artifacts: artifacts.into(),
        })
    }

    /// `manifest` is the case manifest the run inventoried, from which the adapter takes each
    /// case's declared test parameter.
    pub async fn start(
        &mut self,
        pins: &Value,
        adapter: &Path,
        settings: &Settings,
        manifest: &Path,
    ) -> Result<()> {
        let password = secret(24)?;
        let admin_password = secret(24)?;
        let image = pins["postgres"]
            .as_str()
            .ok_or("missing PostgreSQL image pin")?;
        self.profile = Some(settings.clone());
        let args = container_args(&self.name, image, settings);
        let mut docker = process::command(
            "docker",
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        docker.env("POSTGRES_PASSWORD", &admin_password);
        // A cancelled docker client may still have created the named container.
        self.container_attempted = true;
        process::output(&mut docker, None, 120).await?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if process::run(
                "docker",
                &[
                    "exec",
                    &self.name,
                    "pg_isready",
                    "-h",
                    "127.0.0.1",
                    "-p",
                    "5432",
                    "-U",
                    "postgres",
                ],
            )
            .await
            .is_ok()
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err("PostgreSQL readiness deadline exceeded".into());
            }
            sleep(Duration::from_millis(250)).await;
        }
        let mapping = process::run("docker", &["port", &self.name, "5432/tcp"]).await?;
        let port: u16 = mapping
            .strip_prefix("127.0.0.1:")
            .ok_or("unexpected PostgreSQL port binding")?
            .parse()?;
        self.admin_url = format!("postgresql://postgres:{admin_password}@127.0.0.1:{port}");
        let sql = format!(
            "CREATE ROLE fixture LOGIN PASSWORD '{password}' NOSUPERUSER NOCREATEDB NOCREATEROLE;\nCREATE DATABASE harness OWNER fixture;\n{}",
            role_session("fixture", settings)
        );
        process::output(
            &mut process::command(
                "docker",
                &[
                    "exec",
                    "-i",
                    &self.name,
                    "psql",
                    "-U",
                    "postgres",
                    "-v",
                    "ON_ERROR_STOP=1",
                ],
            ),
            Some(&sql),
            120,
        )
        .await?;
        // Read settings as the role the adapter actually uses.
        let settings = process::run("docker", &["exec", &self.name, "psql", "-U", "fixture", "-d", "harness", "-Atc",
            "SELECT json_build_object('version',version(),'encoding',current_setting('server_encoding'),'search_path',current_setting('search_path'),'standard_conforming_strings',current_setting('standard_conforming_strings'),'statement_timeout',current_setting('statement_timeout'),'lock_timeout',current_setting('lock_timeout'))"]).await?;
        self.settings = serde_json::from_str(&settings)?;
        self.settings["image"] =
            process::run("docker", &["inspect", &self.name, "--format", "{{.Image}}"])
                .await?
                .into();
        let stderr = File::create(self.artifacts.join("adapter.stderr"))?;
        let mut command = process::command(adapter.to_str().ok_or("non-UTF8 adapter path")?, &[]);
        command
            .env(
                "SQLMAP_FIXTURE_URL",
                format!("postgresql://fixture:{password}@127.0.0.1:{port}/harness"),
            )
            .env("SQLMAP_EVIDENCE", self.artifacts.join("requests.jsonl"))
            .env("SQLMAP_MANIFEST", manifest)
            .stdout(Stdio::piped())
            .stderr(stderr);
        self.adapter = Some(Process::spawn(&mut command)?);
        let stdout = self
            .adapter
            .as_mut()
            .and_then(|p| p.child.stdout.take())
            .ok_or("missing adapter stdout")?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        timeout(Duration::from_secs(30), reader.read_line(&mut line))
            .await
            .map_err(|_| "adapter startup deadline exceeded")??;
        let started: Value = serde_json::from_str(&line)
            .map_err(|_| "adapter failed to start; see adapter.stderr")?;
        let port = started["port"]
            .as_u64()
            .filter(|p| *p > 0 && *p <= 65535)
            .ok_or("invalid adapter port")?;
        self.url = format!("http://127.0.0.1:{port}");
        Ok(())
    }

    pub async fn direct_regressions(&self, root: &Path) -> Result<Value> {
        // TestContext creates its own databases on this same disposable server.
        // Give its role the same session settings as the scanner fixture.
        let settings = self.profile.as_ref().ok_or("fixture was never started")?;
        process::run(
            "docker",
            &[
                "exec",
                &self.name,
                "psql",
                "-U",
                "postgres",
                "-Atc",
                &role_session("postgres", settings),
            ],
        )
        .await?;
        process::run(
            "docker",
            &[
                "exec",
                &self.name,
                "psql",
                "-U",
                "postgres",
                "-d",
                "template1",
                "-v",
                "ON_ERROR_STOP=1",
                "-c",
                "CREATE SCHEMA fixture",
            ],
        )
        .await?;
        let file = File::create(self.artifacts.join("direct-regressions.log"))?;
        let args = [
            "test",
            "--locked",
            "--test",
            "sql_security_tests",
            "--",
            "--test-threads=1",
        ];
        write_json(&self.artifacts.join("direct-command.json"), &args)?;
        let mut command = process::command("cargo", &args);
        command
            .current_dir(root)
            .env("DATABASE_URL", &self.admin_url)
            .stdout(file.try_clone()?)
            .stderr(file);
        let mut process = Process::spawn(&mut command)?;
        let status = process.wait(900).await?;
        Ok(
            json!({"pass":status.is_some_and(|s| s.success()), "returncode":status.and_then(|s| s.code()), "log":"direct-regressions.log", "database":self.settings}),
        )
    }

    pub async fn close(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(mut adapter) = self.adapter.take()
            && let Err(e) = adapter.stop().await
        {
            errors.push(format!("adapter cleanup: {e}"));
        }
        if self.container_attempted {
            // Keep server diagnostics even when startup or a scan fails.
            if let Ok(log) = process::run("docker", &["logs", &self.name]).await {
                let _ = fs::write(self.artifacts.join("postgres.log"), log);
            }
            if let Err(e) =
                process::run("docker", &["rm", "--force", "--volumes", &self.name]).await
            {
                errors.push(format!("container cleanup: {e}"));
            }
            self.container_attempted = false;
        }
        errors
    }
}

/// The disposable server's `docker run` arguments, its default statement and lock timeouts
/// the profile's.
///
/// The data volume mounts at `/var/lib/postgresql`: the 18 image keeps its
/// data under `/var/lib/postgresql/18/docker` and refuses to start with
/// anything mounted at the `.../data` path older images used.
// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn container_args(name: &str, image: &str, settings: &Settings) -> Vec<String> {
    let mut args: Vec<String> = [
        "run",
        "--detach",
        "--name",
        name,
        "--label",
        "pgorm.sqlmap=disposable",
        "--network",
        "bridge",
        "--publish",
        "127.0.0.1::5432",
        "--env",
        "POSTGRES_PASSWORD",
        "--mount",
        "type=volume,destination=/var/lib/postgresql",
        "--memory",
        "512m",
        "--cpus",
        "2",
        image,
    ]
    .map(str::to_owned)
    .into();
    args.extend([
        "-c".into(),
        format!(
            "statement_timeout={}s",
            settings.postgres_statement_timeout_seconds
        ),
        "-c".into(),
        format!("lock_timeout={}s", settings.postgres_lock_timeout_seconds),
    ]);
    args
}

/// The session settings a fixture role runs under, the profile's statement and lock timeouts
/// among them.
// [spec:pgorm:req:security.sqlmap.profiles+4]
pub fn role_session(role: &str, settings: &Settings) -> String {
    [
        format!("ALTER ROLE {role} SET search_path = fixture, pg_catalog;"),
        format!("ALTER ROLE {role} SET standard_conforming_strings = on;"),
        format!(
            "ALTER ROLE {role} SET statement_timeout = '{}s';",
            settings.postgres_statement_timeout_seconds
        ),
        format!(
            "ALTER ROLE {role} SET lock_timeout = '{}s';",
            settings.postgres_lock_timeout_seconds
        ),
    ]
    .join("\n")
        + "\n"
}

pub fn request_invariant(artifacts: &Path, case: &str) -> Result<bool> {
    let events = fs::read_to_string(artifacts.join("requests.jsonl"))?;
    let route = format!("protected/{case}");
    let mut invariant = true;
    for line in events.lines() {
        let event: Value = serde_json::from_str(line)?;
        if event["route"] == route && event["response"]["invariant"] == false {
            invariant = false;
        }
    }
    Ok(invariant)
}

pub fn scanner_report(output: &Path) -> Value {
    read_json(&output.join("scanner.json")).unwrap_or(Value::Null)
}
