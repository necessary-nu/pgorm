//! The sweep against a live server: it reclaims a crashed run's leftovers
//! without touching anything a running process could still own.
//!
//! Run the test locally:
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:54319 cargo nextest run -p pgorm-test-support

use std::process::{Child, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use pgorm_test_support::{
    Creator, connect, create_database, drop_database, name_for, scoped_name, sweep, this_run,
};
use tokio_postgres::{Client, Error};

fn base_url() -> String {
    dotenvy::from_filename(".env.local").ok();
    dotenvy::from_filename(".env").ok();
    std::env::var("DATABASE_URL").expect("Environment variable 'DATABASE_URL' not set")
}

fn unix_now() -> u32 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past 1970")
        .as_secs();
    u32::try_from(secs).expect("the clock is before 2106")
}

/// A process this test starts and holds for its length, killed and reaped by
/// its own handle however the test ends.
struct Sleeper(Child);

impl Sleeper {
    fn start() -> Self {
        Self(
            Command::new("sleep")
                .arg("60")
                .spawn()
                .expect("sleep is spawned"),
        )
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The pid of a process this test started, which has exited and been reaped.
fn exited_pid() -> u32 {
    let mut child = Command::new("true").spawn().expect("true is spawned");
    child.wait().expect("true exits");
    child.id()
}

async fn exists(maintenance: &Client, catalogue: &str, name: &str) -> Result<bool, Error> {
    let sql = match catalogue {
        "database" => "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)",
        _ => "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)",
    };
    Ok(maintenance.query_one(sql, &[&name]).await?.get(0))
}

// [spec:pgorm:req:testing.isolation/test]    the sweep drops a database and a role
// only when their creator is gone from this host and nobody is connected; a
// running creator, one of another host or user, and an occupied database all
// keep theirs
#[tokio::test]
async fn the_sweep_reclaims_only_what_gone_runs_left() -> Result<(), Error> {
    let base_url = base_url();
    let maintenance = connect(&base_url, "postgres").await?;
    let run = this_run();
    let sleeper = Sleeper::start();
    let before_exit = unix_now();
    let gone = Creator {
        pid: exited_pid(),
        stamp: before_exit - 10,
        ..run
    };
    let running = Creator {
        pid: sleeper.0.id(),
        stamp: unix_now() + 1,
        ..run
    };
    let reused = Creator {
        pid: sleeper.0.id(),
        stamp: before_exit - 100,
        ..run
    };
    let elsewhere = Creator {
        owner: run.owner ^ 1,
        ..gone
    };

    let reclaimed = [
        name_for("sweep_gone", gone),
        name_for("sweep_reused", reused),
    ];
    let kept = [
        scoped_name("sweep_this_run"),
        name_for("sweep_running", running),
        name_for("sweep_elsewhere", elsewhere),
    ];
    let occupied = name_for("sweep_occupied", gone);
    for name in reclaimed.iter().chain(&kept).chain([&occupied]) {
        create_database(&base_url, name).await?;
    }
    let gone_role = name_for("sweep_gone_role", gone);
    let running_role = name_for("sweep_running_role", running);
    maintenance
        .batch_execute(&format!(
            "CREATE ROLE \"{gone_role}\"; CREATE ROLE \"{running_role}\""
        ))
        .await?;
    let visitor = connect(&base_url, &occupied).await?;

    sweep(&maintenance).await?;

    // Every expectation is read before anything is cleaned up, and the
    // cleanup runs whatever they say: the database recorded for another host
    // is one no sweep will ever reclaim.
    let mut wrong = Vec::new();
    for name in &reclaimed {
        if exists(&maintenance, "database", name).await? {
            wrong.push(format!("{name} survived"));
        }
    }
    for name in kept.iter().chain([&occupied]) {
        if !exists(&maintenance, "database", name).await? {
            wrong.push(format!("{name} was dropped"));
        }
    }
    if exists(&maintenance, "role", &gone_role).await? {
        wrong.push(format!("{gone_role} survived"));
    }
    if !exists(&maintenance, "role", &running_role).await? {
        wrong.push(format!("{running_role} was dropped"));
    }

    drop(visitor);
    for name in kept.iter().chain([&occupied]) {
        drop_database(&base_url, name).await?;
    }
    maintenance
        .batch_execute(&format!(
            "DROP ROLE IF EXISTS \"{gone_role}\"; DROP ROLE IF EXISTS \"{running_role}\""
        ))
        .await?;
    drop(sleeper);
    assert!(wrong.is_empty(), "{wrong:#?}");
    Ok(())
}
