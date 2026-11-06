//! The registry's live-only sites: identifier render sites whose SQL is built
//! where no render closure can reach it, and is captured on its way to the
//! server instead.
//!
//! tokio-postgres logs every statement text it is handed, at debug level,
//! before encoding it: `executing statement batch: {sql}` for the simple
//! protocol and `preparing query {name}: {sql}` for the extended one. A
//! logger installed here records those lines for the current thread, so a
//! site's statements are exactly what the client would have sent. Each site
//! runs twice — once with the benign name, once with the hostile one — each
//! time on a fresh pool, so no statement cache hides a statement from the
//! capture, and the two sequences are held to the oracle's property
//! statement by statement.
//!
//! A run that cannot check out its connection has sent nothing, and an empty
//! capture would read as a site that sends nothing, so a run retries its
//! checkout for [`REACH`], and one that still cannot is reported as an
//! [`Outage`], never judged.

use std::{
    future::Future,
    pin::Pin,
    sync::{LazyLock, Mutex},
    thread::{self, ThreadId},
    time::Duration,
};

use pgorm::{
    ConnectionTrait, DatabasePool, EntityTrait, NoTls, RelationTrait, SelectModel,
    pgorm_query::{Asterisk, IntoKey, Name, Query},
    tests_cfg::{cake, fruit},
};
use pgorm_migration::{MigrationName, MigrationTrait, MigratorTrait};

use super::{
    corpus::{Hostile, NUL_NAME, corpus},
    live::{close, error_chain, open},
    oracle::{
        BENIGN, NulBehaviour, Policy, Reference, Rendered, Site, Verdict, encoder_refuses,
        judge_against, reference_of,
    },
    pins::pinned,
};

type Run = for<'a> fn(&'a str, &'a str) -> Pin<Box<dyn Future<Output = Reached> + 'a>>;

type Reached = Result<(), Outage>;

/// Why a run never reached the server: it sent nothing, so its capture says
/// nothing about the site.
pub struct Outage(String);

impl Outage {
    /// The failure this outage is, for `site` run with the name `label`.
    fn at(self, site: &str, label: &str) -> String {
        format!("site `{site}`, name `{label}`: {}", self.0)
    }
}

/// How long a run keeps trying to check out its connection. A loaded machine
/// can hold a connection's handshake past the server's
/// `authentication_timeout`, 60 s by default, which cancels it and fails the
/// checkout; the bound outlasts two such cancellations.
const REACH: Duration = Duration::from_secs(150);

/// The pause between one failed checkout and the next.
const REACH_RETRY: Duration = Duration::from_millis(250);

/// A site registered here rather than in `registry.rs`: its [`Site`] (whose
/// `kinds` cover the whole captured sequence, in order, and whose `render`
/// is never called) and the operation that makes the client build its SQL.
pub struct LiveSite {
    pub site: Site,
    /// Drive the API with the database URL and the name.
    pub run: Run,
    /// Undo what a run left behind, outside the capture, so the next name
    /// meets the same database — the ledger a 63-byte name created is the
    /// one a 64-byte name truncates onto.
    pub reset: Option<Run>,
    /// For a name longer than `NAMEDATALEN - 1` bytes, the number of
    /// statements after which the run is expected to stop, because the site
    /// also binds the name as a `name` *value* and the server refuses an
    /// over-long one (`42622`, *identifier too long*) where it would have
    /// truncated the identifier. The statements before the stop are held to
    /// the property as usual.
    pub long_name_stops_after: Option<usize>,
}

/// The live-only sites.
pub fn live_sites() -> Vec<LiveSite> {
    vec![
        LiveSite {
            site: Site {
                id: "pool/savepoint.name",
                api: "pgorm_pool::Transaction::savepoint(String), then rollback and commit",
                kinds: &[
                    "TransactionStmt.savepoint_name",
                    "TransactionStmt.savepoint_name",
                    "TransactionStmt.savepoint_name",
                    "TransactionStmt.savepoint_name",
                ],
                policy: Policy::Quoted,
                render: live_only,
            },
            run: |url, name| Box::pin(savepoint(url, name)),
            reset: None,
            long_name_stops_after: None,
        },
        LiveSite {
            site: Site {
                id: "migration/ledger.table-name",
                api: "MigratorTrait::migration_table_name() -> Name, through MigratorTrait::up",
                kinds: &[
                    "CreateStmt.relation.relname",
                    "RangeVar.relname",
                    "InsertStmt.relation.relname",
                ],
                policy: Policy::Quoted,
                render: live_only,
            },
            run: |url, name| Box::pin(ledger(url, name)),
            reset: Some(|url, name| Box::pin(drop_ledger(url, name))),
            // `CREATE TABLE`, then the catalogue lookup that binds the name:
            // the truncation note of the `security.ident-oracle` rule.
            long_name_stops_after: Some(4),
        },
        LiveSite {
            site: Site {
                id: "pgorm/cursor.table",
                api: "Cursor::new(query, Name, order columns)",
                kinds: &["RangeVar.alias.aliasname", "ColumnRef.fields[0]"],
                policy: Policy::Quoted,
                render: live_only,
            },
            run: |url, name| Box::pin(cursor(url, name)),
            reset: None,
            long_name_stops_after: None,
        },
        LiveSite {
            site: Site {
                id: "pgorm/cursor.secondary-order-by",
                api: "Cursor::set_secondary_order_by([(Name, key)])",
                kinds: &["RangeVar.alias.aliasname", "ColumnRef.fields[0]"],
                policy: Policy::Quoted,
                render: live_only,
            },
            run: |url, name| Box::pin(secondary_order(url, name)),
            reset: None,
            long_name_stops_after: None,
        },
        LiveSite {
            site: Site {
                id: "pgorm/cursor.add-secondary-order-by",
                api: "Cursor::add_secondary_order_by(Name, column)",
                kinds: &["RangeVar.alias.aliasname", "ColumnRef.fields[0]"],
                policy: Policy::Quoted,
                render: live_only,
            },
            run: |url, name| Box::pin(appended_order(url, name)),
            reset: None,
            long_name_stops_after: None,
        },
        LiveSite {
            site: Site {
                id: "pgorm/graph.cursor-by.alias",
                api: "SelectGraph::join_maybe_as(relation, Name).cursor_by(key)",
                kinds: &[
                    "ColumnRef.fields[0]",
                    "RangeVar.alias.aliasname",
                    "ColumnRef.fields[0]",
                    "ColumnRef.fields[0]",
                    "ColumnRef.fields[0]",
                    "ColumnRef.fields[0]",
                ],
                policy: Policy::Quoted,
                render: live_only,
            },
            run: |url, name| Box::pin(graph_cursor(url, name)),
            reset: None,
            long_name_stops_after: None,
        },
    ]
}

fn live_only(_: &str) -> Rendered {
    Rendered::Refused("rendered only against a live server".to_owned())
}

// ---------------------------------------------------------------------------
// The operations
// ---------------------------------------------------------------------------

/// Open a savepoint under `name` and roll back to it, then open it again and
/// release it: `SAVEPOINT`, `ROLLBACK TO`, `SAVEPOINT`, `RELEASE`.
async fn savepoint(url: &str, name: &str) -> Reached {
    let pool = pgorm_pool::Config {
        url: Some(url.to_owned()),
        ..Default::default()
    }
    .create_pool(NoTls)
    .expect("a pgorm-pool pool");
    let mut client = reach(|| pool.get()).await?;
    let Ok(mut txn) = client.transaction().await else {
        return Ok(());
    };
    if let Ok(point) = txn.savepoint(name).await {
        let _ = point.rollback().await;
    }
    if let Ok(point) = txn.savepoint(name).await {
        let _ = point.commit().await;
    }
    let _ = txn.rollback().await;
    Ok(())
}

static LEDGER: Mutex<String> = Mutex::new(String::new());

struct OracleMigrator;

struct Noop;

impl MigrationName for Noop {
    fn name(&self) -> &str {
        "m_oracle_noop"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Noop {
    async fn up(&self, _: &pgorm::DatabaseTransaction<'_>) -> Result<(), pgorm::Error> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl MigratorTrait for OracleMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Noop)]
    }

    fn migration_table_name() -> Name {
        Name::runtime(LEDGER.lock().map(|name| name.clone()).unwrap_or_default())
    }
}

/// Run the migrator, with one no-op migration, against a ledger named `name`.
async fn ledger(url: &str, name: &str) -> Reached {
    if let Ok(mut ledger) = LEDGER.lock() {
        name.clone_into(&mut ledger);
    }
    let pool = fresh_pool(url);
    // The connection checked out here goes back idle, and is the one the
    // migrator is lent, so it does not connect again.
    drop(reach(|| pool.get()).await?);
    let _ = OracleMigrator::up(pool, None).await;
    Ok(())
}

/// Drop the ledger a run created, quoting the name independently of the code
/// under test.
async fn drop_ledger(url: &str, name: &str) -> Reached {
    let pool = fresh_pool(url);
    let db = reach(|| pool.get()).await?;
    let quoted = format!("\"{}\"", name.replace('"', "\"\""));
    let _ = db
        .batch_execute(&format!("DROP TABLE IF EXISTS {quoted}"))
        .await;
    Ok(())
}

/// Page `cake` read under the alias `name` through a cursor qualified by it.
async fn cursor(url: &str, name: &str) -> Reached {
    let pool = fresh_pool(url);
    let db = reach(|| pool.get()).await?;
    let query = Query::select()
        .column(Asterisk)
        .from_as(Name::runtime("cake"), Name::runtime(name))
        .take();
    let mut cursor = pgorm::Cursor::<SelectModel<cake::Model>, pgorm::Value>::new(
        query,
        Name::runtime(name),
        cake::Column::Id,
    );
    let _ = cursor.first(1).all(&db).await;
    Ok(())
}

/// Page `cake` under the alias `name`, tiebreaking on a secondary order
/// column qualified by it.
async fn secondary_order(url: &str, name: &str) -> Reached {
    let pool = fresh_pool(url);
    let db = reach(|| pool.get()).await?;
    let query = Query::select()
        .column(Asterisk)
        .from_as(Name::runtime("cake"), Name::runtime(name))
        .take();
    let mut cursor = pgorm::Cursor::<SelectModel<cake::Model>, pgorm::Value>::new(
        query,
        Name::runtime("cake"),
        cake::Column::Id,
    );
    cursor.set_secondary_order_by(vec![(Name::runtime(name), cake::Column::Name.into_key())]);
    let _ = cursor.first(1).all(&db).await;
    Ok(())
}

/// Page `cake` under the alias `name`, tiebreaking on a typed column the
/// cursor appends qualified by it.
async fn appended_order(url: &str, name: &str) -> Reached {
    let pool = fresh_pool(url);
    let db = reach(|| pool.get()).await?;
    let query = Query::select()
        .column(Asterisk)
        .from_as(Name::runtime("cake"), Name::runtime(name))
        .take();
    let mut cursor = pgorm::Cursor::<SelectModel<cake::Model>, pgorm::Value>::new(
        query,
        Name::runtime("cake"),
        cake::Column::Id,
    );
    cursor.add_secondary_order_by(Name::runtime(name), cake::Column::Name);
    let _ = cursor.first(1).all(&db).await;
    Ok(())
}

/// Page a graph whose joined `fruit` is bound under the alias `name`, so the
/// cursor's tiebreak is qualified by it.
async fn graph_cursor(url: &str, name: &str) -> Reached {
    let pool = fresh_pool(url);
    let db = reach(|| pool.get()).await?;
    let _ = cake::Entity::graph()
        .join_maybe_as::<fruit::Entity>(cake::Relation::Fruit.def(), Name::runtime(name))
        .cursor_by(cake::Column::Id)
        .first(1)
        .all(&db)
        .await;
    Ok(())
}

/// A pool of its own for one run, so no statement cache hides a statement
/// from the capture.
fn fresh_pool(url: &str) -> DatabasePool {
    pgorm::connect(url.parse().expect("the test database's URL parses"))
}

/// Check out a connection through `checkout`, trying again after each
/// failure until [`REACH`] has passed, and then answering why it did not.
async fn reach<T, E, F, Fut>(mut checkout: F) -> Result<T, Outage>
where
    E: std::error::Error + 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let deadline = tokio::time::Instant::now() + REACH;
    let mut failed = None;
    loop {
        match tokio::time::timeout_at(deadline, checkout()).await {
            Ok(Ok(connection)) => return Ok(connection),
            Ok(Err(error)) => failed = Some(error_chain(&error)),
            Err(_) => {
                let before = failed.map_or_else(String::new, |why| format!(", after {why}"));
                return Err(Outage(format!(
                    "the run did not reach the server within {REACH:?}: \
                     its last attempt was still connecting{before}"
                )));
            }
        }
        if tokio::time::Instant::now() + REACH_RETRY >= deadline {
            let why = failed.unwrap_or_default();
            return Err(Outage(format!(
                "the run did not reach the server within {REACH:?}: {why}"
            )));
        }
        tokio::time::sleep(REACH_RETRY).await;
    }
}

// ---------------------------------------------------------------------------
// The capture
// ---------------------------------------------------------------------------

static LINES: LazyLock<Mutex<Vec<(ThreadId, String)>>> = LazyLock::new(Mutex::default);

struct Capture;

static CAPTURE: Capture = Capture;

impl log::Log for Capture {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("tokio_postgres") && metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = record.args().to_string();
        let sql = if let Some(sql) = message.strip_prefix("executing statement batch: ") {
            sql
        } else if let Some(sql) = message.strip_prefix("executing simple query: ") {
            sql
        } else if let Some(rest) = message.strip_prefix("preparing query ") {
            match rest.split_once(": ") {
                Some((_, sql)) => sql,
                None => return,
            }
        } else {
            return;
        };
        if let Ok(mut lines) = LINES.lock() {
            lines.push((thread::current().id(), sql.to_owned()));
        }
    }

    fn flush(&self) {}
}

/// Every statement tokio-postgres was handed on this thread while `run` ran,
/// but for its own catalogue lookups, or the outage that kept `run` from the
/// server.
async fn capture(run: Pin<Box<dyn Future<Output = Reached> + '_>>) -> Result<Vec<String>, Outage> {
    // Installing twice fails harmlessly: the logger is the same one.
    let _ = log::set_logger(&CAPTURE);
    log::set_max_level(log::LevelFilter::Debug);
    let me = thread::current().id();
    let drain = || {
        LINES.lock().map_or_else(
            |_| Vec::new(),
            |mut lines| {
                let (mine, theirs) = lines.drain(..).partition(|(thread, _)| *thread == me);
                *lines = theirs;
                mine.into_iter()
                    .map(|(_, sql)| sql)
                    .filter(|sql| !is_catalogue(sql) && !is_commit(sql))
                    .collect::<Vec<_>>()
            },
        )
    };
    drain();
    let reached = run.await;
    let statements = drain();
    reached.map(|()| statements)
}

/// The benign run's statements, parsed, checked against the kinds the site
/// declares for the whole sequence.
fn references(site: &Site, benign: &[String]) -> Result<Vec<Reference>, String> {
    let references = benign
        .iter()
        .map(|sql| reference_of(sql))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("site `{}`: {err}", site.id))?;
    let kinds: Vec<String> = references.iter().flat_map(Reference::kinds).collect();
    if kinds != site.kinds {
        return Err(format!(
            "site `{}`: the registry declares the name at {:?}, but the benign run puts it at \
             {kinds:?}:\n    {benign:?}",
            site.id, site.kinds
        ));
    }
    Ok(references)
}

/// Hold one live site's captured statements for one name against the benign
/// run's, statement by statement.
fn judge_sequence(
    live: &LiveSite,
    hostile: &Hostile,
    references: &[Reference],
    captured: &[String],
) -> Result<Verdict, String> {
    let site = &live.site;
    let stop = live
        .long_name_stops_after
        .filter(|_| hostile.name.len() > super::oracle::IDENTIFIER_BYTES);
    let mut verdict = Verdict::RoundTrip;
    for (index, reference) in references.iter().enumerate() {
        // The refusal fails the run's transaction closure, whose awaited
        // ROLLBACK, naming nothing, is the one statement past the stop.
        if stop == Some(index) && captured.get(index..) == Some(&["ROLLBACK".to_owned()][..]) {
            return Ok(Verdict::Refused("identifier too long"));
        }
        let Some(sql) = captured.get(index) else {
            return Err(format!(
                "site `{}`, name `{}`: the run stopped after {index} of {} statements:\n    \
                 {captured:?}",
                site.id,
                hostile.label,
                references.len()
            ));
        };
        match judge_against(site, hostile, Rendered::Sql(sql.clone()), reference)? {
            Verdict::EmptyRejected => return Ok(Verdict::EmptyRejected),
            Verdict::RoundTrip => {}
            other => verdict = other,
        }
    }
    if captured.len() != references.len() {
        return Err(format!(
            "site `{}`, name `{}`: {} statements where the benign run sent {}:\n    {captured:?}",
            site.id,
            hostile.label,
            captured.len(),
            references.len()
        ));
    }
    Ok(verdict)
}

/// Every live-only site holds every corpus name, statement by statement, and
/// keeps a NUL-bearing name out of the server.
// [spec:pgorm:req:security.ident-oracle+17/test]
// [spec:pgorm:req:security.ident-oracle.nul+2/test]
#[tokio::test]
async fn live_sites_hold_every_hostile_name() {
    let (ctx, db) = open("ident_oracle_live_capture").await;
    db.batch_execute(
        "CREATE TABLE cake (id integer PRIMARY KEY, name text); INSERT INTO cake VALUES (1, 'a'); \
         CREATE TABLE fruit (id integer PRIMARY KEY, name text, cake_id integer); \
         INSERT INTO fruit VALUES (1, 'b', 1)",
    )
    .await
    .expect("the fixture table is created");
    let url = format!(
        "{}/{}",
        std::env::var("DATABASE_URL").expect("DATABASE_URL is set"),
        ctx.db_name()
    );
    let mut failures = Vec::new();
    for live in live_sites() {
        failures.extend(hold_site(&live, &url).await);
    }
    close(ctx, db).await;
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Hold one live site to every corpus name and to the NUL name, answering
/// each failure.
async fn hold_site(live: &LiveSite, url: &str) -> Vec<String> {
    let id = live.site.id;
    let references = match capture((live.run)(url, BENIGN)).await {
        Ok(benign) => references(&live.site, &benign),
        Err(outage) => Err(outage.at(id, BENIGN)),
    };
    let references = match references {
        Ok(references) => references,
        Err(failure) => return vec![failure],
    };
    if let Some(reset) = live.reset
        && let Err(outage) = reset(url, BENIGN).await
    {
        return vec![outage.at(id, BENIGN)];
    }
    let mut failures = Vec::new();
    for hostile in corpus() {
        let captured = capture((live.run)(url, &hostile.name)).await;
        let reset = match live.reset {
            Some(reset) => reset(url, &hostile.name).await,
            None => Ok(()),
        };
        let captured = match captured.and_then(|captured| reset.map(|()| captured)) {
            Ok(captured) => captured,
            Err(outage) => {
                failures.push(outage.at(id, hostile.label));
                continue;
            }
        };
        let outcome = judge_sequence(live, &hostile, &references, &captured);
        match (outcome, pinned(id, hostile.label)) {
            (Ok(_), None) | (Err(_), Some(_)) => {}
            (Err(failure), None) => failures.push(failure),
            (Ok(verdict), Some(pin)) => failures.push(format!(
                "site `{id}`, name `{}`: pinned as `{}` but now passes ({verdict:?})",
                hostile.label, pin.node
            )),
        }
    }
    let captured = match capture((live.run)(url, NUL_NAME)).await {
        Ok(captured) => captured,
        Err(outage) => {
            failures.push(outage.at(id, "NUL"));
            return failures;
        }
    };
    match captured.iter().find(|sql| sql.contains('\0')) {
        Some(sql) if live.site.policy.nul() == NulBehaviour::Encoder => {
            if let Err(what) = encoder_refuses(sql) {
                failures.push(format!("site `{id}`, NUL: {what}"));
            }
        }
        other => failures.push(format!(
            "site `{id}`, NUL: expected the NUL byte in a statement the encoder refuses, got {other:?}"
        )),
    }
    failures
}

/// tokio-postgres's own type-lookup statements, which it prepares the first
/// time a connection meets a type it does not know. They carry no caller
/// name, and whether one runs depends on the connection's history, not on the
/// site.
fn is_catalogue(sql: &str) -> bool {
    sql.contains("pg_catalog.pg_type") || sql.contains("FROM pg_catalog.pg_enum")
}

/// pgorm-pool's commit of a transaction, `SELECT; COMMIT` in one simple
/// query (`conn.tx.aborted`): fixed text naming nothing, and two statements,
/// which no site's statement is.
fn is_commit(sql: &str) -> bool {
    sql == "SELECT; COMMIT"
}
