//! Names for what a live test creates on a PostgreSQL server that other runs
//! share, and the sweep that reclaims what a crashed run left behind.
//!
//! Any number of runs may use one server at once: two checkouts, two runs of
//! one checkout, the Rust crates' suites and pgorm-napi's. Each database or
//! role a test creates is named for the process that created it,
//!
//! ```text
//! {label}_{seq}__{owner}_{pid}_{stamp}
//! ```
//!
//! - `label` is the name the test asked for, cut to fit PostgreSQL's 63-byte
//!   identifiers;
//! - `seq` counts the names this process has minted, so two labels cut to one
//!   prefix, or one label asked for twice, still name different objects;
//! - `owner` is eight hex digits hashing the host name and the user, the scope
//!   within which a process id means anything;
//! - `pid` is the creating process, in decimal;
//! - `stamp` is eight hex digits holding the Unix second at which the process
//!   minted its first name, which tells the creator from a later process that
//!   was given its pid.
//!
//! No run drops a database or role it did not create, with one exception: a
//! leftover whose creator is provably gone. That is a name with this run's
//! owner whose pid no process on this host holds, or holds for a process that
//! started after the stamp, and to which no session is connected. Every
//! question this host cannot answer counts as the creator still running.
//!
//! pgorm's and pgorm-migration's live tests take this crate as a
//! dev-dependency, and pgorm-napi's `tests/support.ts` mints and sweeps the
//! same names, so each suite can reclaim the others' leftovers.
//!
//! [`ListenerClears`] counts the clears a pool's statement-cache listener
//! makes, for the tests of it in pgorm, pgorm-pool and pgorm-migration, and
//! [`PREPARED_STATEMENT`] names the statement a session holds prepared, which
//! is how a test tells whether a pool cleared it.

use std::{
    future::Future,
    process::Command,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use pgorm_driver::{Client, Config, Error, NoTls, error::SqlState};

mod clears;
mod prepared;

pub use clears::ListenerClears;
pub use prepared::PREPARED_STATEMENT;

/// The longest identifier PostgreSQL stores without truncating it.
const MAX_IDENTIFIER_LEN: usize = 63;

/// The tail every minted name ends with, as a PostgreSQL regular expression.
/// It only narrows what the catalogue returns; [`creator`] reads each
/// candidate exactly before anything is decided about it.
const TAG_PATTERN: &str = "__[0-9a-f]{8}_[0-9]+_[0-9a-f]{8}$";

/// How far after its stamp a creator's start can appear: `ps` reports elapsed
/// time in whole seconds, and the stamp is truncated to one.
const START_SLACK_SECS: u64 = 2;

/// How many times a create or drop is re-attempted while another session
/// holds what it needs.
const DDL_ATTEMPTS: u32 = 8;

/// Pause before the second attempt; each further attempt waits a multiple of
/// it, so eight attempts span a little under a second and a half.
const DDL_BACKOFF: Duration = Duration::from_millis(50);

/// The process that minted a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Creator {
    pub owner: u32,
    pub pid: u32,
    pub stamp: u32,
}

/// What `ps` says about a process id.
enum Holder {
    /// No process holds it.
    Gone,
    /// A process holds it, started at this Unix second.
    StartedAt(u64),
}

/// The connection configuration for `db_name` on the server `base_url` names.
/// It is the type `pgorm::connect` takes, since both crates name the
/// workspace's tokio-postgres.
pub fn config(base_url: &str, db_name: &str) -> Config {
    format!("{base_url}/{db_name}")
        .parse()
        .expect("DATABASE_URL is not a valid PostgreSQL connection string")
}

/// A client on `db_name`, its connection driven by a task on the current
/// tokio runtime until the client is dropped.
pub async fn connect(base_url: &str, db_name: &str) -> Result<Client, Error> {
    let (client, connection) = config(base_url, db_name).connect(NoTls).await?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            tracing::debug!("test support connection ended: {err}");
        }
    });
    Ok(client)
}

/// This process, as the names it mints record it.
pub fn this_run() -> Creator {
    static RUN: OnceLock<Creator> = OnceLock::new();
    *RUN.get_or_init(|| Creator {
        owner: owner_hash(),
        pid: std::process::id(),
        stamp: u32::try_from(unix_now()).unwrap_or(u32::MAX),
    })
}

/// A database or role name for `label`, unique to this process and to this
/// call within it.
// [spec:pgorm:req:testing.isolation+1]    the per-run name every live test object takes
pub fn scoped_name(label: &str) -> String {
    name_for(label, this_run())
}

/// A name for `label` that records `creator`, which need not be this
/// process: the sweep's tests mint names for other creators. The sequence is
/// still this process's, so no two calls here return one name.
pub fn name_for(label: &str, creator: Creator) -> String {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let Creator { owner, pid, stamp } = creator;
    let tail = format!("_{seq}__{owner:08x}_{pid}_{stamp:08x}");
    let mut cut = MAX_IDENTIFIER_LEN
        .saturating_sub(tail.len())
        .min(label.len());
    while !label.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{tail}", &label[..cut])
}

/// The creator a minted name records, or `None` for any other name.
pub fn creator(name: &str) -> Option<Creator> {
    let mut parts = name.rsplitn(4, '_');
    let (stamp, pid, owner, rest) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let (_label, seq) = rest.strip_suffix('_')?.rsplit_once('_')?;
    let decimal = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let hex = |text: &str| {
        (text.len() == 8 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
            .then(|| u32::from_str_radix(text, 16).ok())
            .flatten()
    };
    if !decimal(seq) || !decimal(pid) {
        return None;
    }
    Some(Creator {
        owner: hex(owner)?,
        pid: pid.parse().ok()?,
        stamp: hex(stamp)?,
    })
}

/// Whether the process that minted `creator` may still be running. Anything
/// this host cannot answer is a yes: a creator recorded by another host or
/// user, or a `ps` that does not answer for this very process.
pub fn may_be_running(creator: Creator) -> bool {
    let run = this_run();
    if creator.owner == 0 || creator.owner != run.owner || creator == run || !ps_answers() {
        return true;
    }
    match holder(creator.pid) {
        Some(Holder::Gone) => false,
        Some(Holder::StartedAt(start)) => start <= u64::from(creator.stamp) + START_SLACK_SECS,
        None => true,
    }
}

/// Create the database `name` for this process, first reclaiming whatever
/// crashed runs left on the server.
pub async fn create_database(base_url: &str, name: &str) -> Result<(), Error> {
    let maintenance = connect(base_url, "postgres").await?;
    sweep_once(&maintenance).await;
    let create = format!("CREATE DATABASE {}", quoted(name));
    retrying(|| maintenance.batch_execute(&create)).await
}

/// Drop the database `name`, which this process created. `FORCE` can evict
/// only this process's own sessions, since no other run knows the name.
pub async fn drop_database(base_url: &str, name: &str) -> Result<(), Error> {
    let maintenance = connect(base_url, "postgres").await?;
    let drop = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", quoted(name));
    retrying(|| maintenance.batch_execute(&drop)).await
}

/// Sweep the server once per process: under nextest that is once per test,
/// which keeps a crashed run's leftovers from outliving the next run's first
/// test, and under `cargo test` once per test binary.
async fn sweep_once(maintenance: &Client) {
    static SWEPT: AtomicBool = AtomicBool::new(false);
    if SWEPT.swap(true, Ordering::Relaxed) {
        return;
    }
    if let Err(err) = sweep(maintenance).await {
        tracing::warn!("leftovers of crashed test runs were not swept: {err}");
    }
}

/// Drop every database and role a gone creator left, in that order, so a
/// role's privileges on a leftover database's objects go with the database.
///
/// The databases are dropped without `FORCE`: a session that connected after
/// the catalogue was read makes the drop fail rather than be evicted. Each
/// failure is left for a later sweep.
// [spec:pgorm:req:testing.isolation+1]    only a provably gone creator's leftovers are dropped
pub async fn sweep(maintenance: &Client) -> Result<(), Error> {
    let databases = maintenance
        .query(
            "SELECT d.datname::text FROM pg_database d \
             WHERE d.datname ~ $1 AND NOT d.datistemplate \
             AND NOT EXISTS (SELECT 1 FROM pg_stat_activity a WHERE a.datname = d.datname)",
            &[&TAG_PATTERN],
        )
        .await?;
    for name in abandoned(databases.iter().map(|row| row.get(0))) {
        let dropped = maintenance
            .batch_execute(&format!("DROP DATABASE IF EXISTS {}", quoted(&name)))
            .await;
        if let Err(err) = dropped {
            tracing::debug!("leftover database {name:?} stays: {err}");
        }
    }
    let roles = maintenance
        .query(
            "SELECT rolname::text FROM pg_roles WHERE rolname ~ $1",
            &[&TAG_PATTERN],
        )
        .await?;
    for name in abandoned(roles.iter().map(|row| row.get(0))) {
        let dropped = maintenance
            .batch_execute(&format!("DROP ROLE IF EXISTS {}", quoted(&name)))
            .await;
        if let Err(err) = dropped {
            tracing::debug!("leftover role {name:?} stays: {err}");
        }
    }
    Ok(())
}

fn abandoned(names: impl Iterator<Item = String>) -> Vec<String> {
    names
        .filter(|name| creator(name).is_some_and(|creator| !may_be_running(creator)))
        .collect()
}

fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Re-run `step` while another session holds what it needs, such as a
/// connection to `template1` during `CREATE DATABASE`, waiting longer each
/// time; any other refusal is the statement's own and returns at once.
async fn retrying<F, Fut>(mut step: F) -> Result<(), Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(), Error>>,
{
    let mut attempt = 1;
    loop {
        match step().await {
            Err(err) if attempt < DDL_ATTEMPTS && err.code() == Some(&SqlState::OBJECT_IN_USE) => {
                tokio::time::sleep(DDL_BACKOFF * attempt).await;
                attempt += 1;
            }
            outcome => return outcome,
        }
    }
}

/// A hash of the host name and the user, or 0 when the host name cannot be
/// learned, which no sweep acts on. pgorm-napi's support module computes the
/// same value from the same sources.
fn owner_hash() -> u32 {
    let host = host_name();
    if host.is_empty() {
        return 0;
    }
    let user = ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .unwrap_or_default();
    fnv1a(format!("{host}\n{user}").as_bytes())
}

fn host_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .or_else(|| {
            let out = Command::new("hostname").output().ok()?;
            out.status
                .success()
                .then(|| String::from_utf8(out.stdout).ok())
                .flatten()
        })
        .map(|name| name.trim().to_owned())
        .unwrap_or_default()
}

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Whether `ps` answers for this process, which proves the flags
/// [`holder`] passes are understood, so its silence about another pid means
/// that no process holds it.
fn ps_answers() -> bool {
    static ANSWERS: OnceLock<bool> = OnceLock::new();
    *ANSWERS.get_or_init(|| matches!(holder(std::process::id()), Some(Holder::StartedAt(_))))
}

/// What `ps` says about `pid`, or `None` when it cannot say.
fn holder(pid: u32) -> Option<Holder> {
    let out = Command::new("ps")
        .args(["-o", "etime=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let elapsed = String::from_utf8(out.stdout).ok()?;
    let elapsed = elapsed.trim();
    if !out.status.success() {
        return (elapsed.is_empty() && out.stderr.is_empty()).then_some(Holder::Gone);
    }
    Some(Holder::StartedAt(
        unix_now().checked_sub(elapsed_secs(elapsed)?)?,
    ))
}

/// `ps`'s elapsed time, `[[dd-]hh:]mm:ss`, in seconds.
pub fn elapsed_secs(etime: &str) -> Option<u64> {
    let (days, clock) = match etime.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, etime),
    };
    let fields: Vec<&str> = clock.split(':').collect();
    if !(2..=3).contains(&fields.len()) {
        return None;
    }
    let mut secs = 0;
    for field in fields {
        secs = secs * 60 + field.parse::<u64>().ok()?;
    }
    Some(days * 86_400 + secs)
}
