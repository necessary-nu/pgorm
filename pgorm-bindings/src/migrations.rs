//! pgorm-migration's runner over a list a binding's program wrote: its
//! `SqlMigration`s are the crate's own, and a migration written in the
//! language is a `MigrationTrait` whose `up` hands the batch's transaction to
//! the language for as long as the object's `up` runs. The list is a
//! `pgorm_migration::Migrations`, so the ledger, the lock, the checksums,
//! `commits_after`, the statement-cache clear and notification and the
//! handling of a migration the list does not know are the runner's alone.

use std::{
    future::Future,
    sync::{Arc, Mutex, PoisonError},
};

use pgorm::{DatabaseTransaction, Error, Name};
use pgorm_migration::{
    MigrationName, MigrationTrait, Migrations, MigratorTrait, SqlMigration, UnknownMigrations,
};

use crate::{
    Failure,
    connection::PoolState,
    transaction::{Lent, State, lend},
};

/// A migration written in the binding's language: what the runner read of
/// it once, as the list was built, and the object whose `up` it runs.
#[derive(Debug)]
pub struct Written<T> {
    /// The migration's name, which the ledger records it by.
    pub name: String,
    /// The checksum the ledger holds an applied migration to, if it has one.
    pub checksum: Option<String>,
    /// Whether the batch commits after this migration before the next runs.
    pub commits_after: bool,
    /// The language's migration object, whose `up` [`Host::up`] runs.
    pub object: T,
}

/// One migration of a binding's list.
#[derive(Debug)]
pub enum Entry<T> {
    /// A migration whose whole body is one SQL script.
    Sql(SqlMigration),
    /// A migration the language wrote.
    Written(Arc<Written<T>>),
}

/// How a written migration's `up` failed in the language.
#[derive(Debug)]
pub enum Failed<R> {
    /// It failed with `R`, which the runner's caller fails with in place of
    /// the runner's account of it; the account says the migration `said`
    /// (`"raised"`, `"rejected"`).
    With(R, &'static str),
    /// The language could not run it, as the account says it.
    Unrun(&'static str),
}

/// What one run of the runner's `up` gives a binding's written migrations.
pub trait Host: Send + Sync + 'static {
    /// The language's migration object.
    type Object: Send + Sync + 'static;
    /// What the pool's binding keeps with it ([`PoolState`]'s `O`).
    type Owner: Send + Sync + 'static;
    /// What a migration's `up` failed with in the language.
    type Raised: Send + 'static;
    /// The pool the runner takes its connections from.
    fn pool(&self) -> &Arc<PoolState<Self::Owner>>;
    /// Run `written`'s `up` on the transaction `state` is, as a handle of the
    /// language's own.
    fn up(
        self: &Arc<Self>,
        written: &Arc<Written<Self::Object>>,
        state: Arc<State<Self::Owner>>,
    ) -> impl Future<Output = Result<(), Failed<Self::Raised>>> + Send;
}

/// The host of one `up`, and what the first migration that failed in the
/// language failed with.
struct Run<H: Host> {
    host: Arc<H>,
    raised: Mutex<Option<H::Raised>>,
}

/// A written migration as the runner holds it: `run` is `None` for a
/// migrator built to read the ledger, which never runs a migration.
struct Hosted<H: Host> {
    written: Arc<Written<H::Object>>,
    run: Option<Arc<Run<H>>>,
}

impl<H: Host> MigrationName for Hosted<H> {
    fn name(&self) -> &str {
        &self.written.name
    }
}

#[pgorm_migration::async_trait::async_trait]
impl<H: Host> MigrationTrait for Hosted<H> {
    // [spec:pgorm:req:bindings.migrations]    `up` awaited in the batch's transaction
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        let name = &self.written.name;
        let Some(run) = &self.run else {
            return Err(Error::Custom(format!(
                "migration '{name}' was read to report a status, not to run"
            )));
        };
        let lent = lend(run.host.pool().clone(), tx, |state| {
            run.host.up(&self.written, state)
        })
        .await;
        let said = match lent {
            Lent::Ran(Ok(())) => return Ok(()),
            Lent::Ran(Err(Failed::With(raised, said))) => {
                run.raised
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get_or_insert(raised);
                said
            }
            Lent::Ran(Err(Failed::Unrun(said))) => said,
            Lent::Discarded => {
                "lost a command's outcome, so its transaction is in a state nobody knows"
            }
        };
        Err(Error::Custom(format!("migration '{name}' {said}")))
    }

    fn checksum(&self) -> Option<String> {
        self.written.checksum.clone()
    }

    fn commits_after(&self) -> bool {
        self.written.commits_after
    }
}

/// The spellings of [`UnknownMigrations`], which both bindings take as a
/// string.
// [spec:pgorm:req:bindings.migrations.unknown-applied]
pub const UNKNOWN_MIGRATIONS: [(&str, UnknownMigrations); 2] = [
    ("refuse", UnknownMigrations::Refuse),
    ("warn", UnknownMigrations::Warn),
];

/// A binding's list of migrations, its ledger placed, its channel named and
/// a migration it does not know refused or warned of as the program gave
/// them.
// [spec:pgorm:req:bindings.migrations]
#[derive(Debug)]
pub struct Listed<T> {
    entries: Vec<Entry<T>>,
    table_name: Option<String>,
    table_schema: Option<String>,
    channel: Option<String>,
    unknown_migrations: UnknownMigrations,
}

impl<T: Send + Sync + 'static> Listed<T> {
    /// The list `entries`, its ledger the table `table_name` in
    /// `table_schema` where they are given, notifying `channel`, and
    /// treating a migration it does not know as `unknown_migrations` says.
    pub fn new(
        entries: Vec<Entry<T>>,
        table_name: Option<String>,
        table_schema: Option<String>,
        channel: Option<String>,
        unknown_migrations: UnknownMigrations,
    ) -> Self {
        Self {
            entries,
            table_name,
            table_schema,
            channel,
            unknown_migrations,
        }
    }

    /// How many migrations the list holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the list holds no migration.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The runner's migrator over this list, its written migrations run in
    /// `run` when there is one.
    fn migrator<H: Host<Object = T>>(&self, run: Option<&Arc<Run<H>>>) -> Migrator {
        let migrations = self.entries.iter().map(|entry| match entry {
            Entry::Sql(migration) => Box::new(migration.clone()) as Box<dyn MigrationTrait>,
            Entry::Written(written) => Box::new(Hosted {
                written: written.clone(),
                run: run.cloned(),
            }),
        });
        let mut migrations = Migrations::new(migrations)
            .with_statement_cache_channel(self.channel.clone())
            .with_unknown_migrations(self.unknown_migrations);
        if let Some(name) = &self.table_name {
            migrations = migrations.with_migration_table_name(Name::runtime(name));
        }
        if let Some(schema) = &self.table_schema {
            migrations = migrations.with_migration_table_schema(Name::runtime(schema));
        }
        Migrator {
            migrations,
            warnings: Mutex::new(Vec::new()),
        }
    }

    /// The runner's migrator over this list for reading and installing its
    /// ledger, which runs no written migration.
    pub fn ledger<H: Host<Object = T>>(&self) -> Migrator {
        self.migrator::<H>(None)
    }

    /// pgorm-migration's `up` of `steps` pending migrations, or all of them,
    /// on connections of the host's pool: what a written migration failed
    /// with in the language when one did, which the binding fails with in
    /// place of the runner's account of it, and otherwise the warnings the
    /// run gave.
    // [spec:pgorm:req:bindings.migrations]
    // [spec:pgorm:req:bindings.migrations.unknown-applied]    a run's warnings reach the binding
    pub fn up<H: Host<Object = T>>(
        &self,
        host: H,
        steps: Option<u32>,
    ) -> impl Future<Output = Result<Ran<H::Raised>, Failure>> + Send + 'static {
        let run = Arc::new(Run {
            host: Arc::new(host),
            raised: Mutex::new(None),
        });
        let migrator = self.migrator(Some(&run));
        async move {
            let pool = run.host.pool().clone();
            let result = migrator.up(pool.pool.clone(), steps).await;
            let raised = run
                .raised
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            match (result, raised) {
                (Ok(()), _) => Ok(Ran::Finished(migrator.take_warnings())),
                (Err(_), Some(raised)) => Ok(Ran::Raised(raised)),
                (Err(error), None) => Err(pool.failure(&error)),
            }
        }
    }
}

/// How a binding's `up` ended, short of a failure of the runner's own.
#[derive(Debug)]
pub enum Ran<R> {
    /// The run finished, giving these warnings, each to be logged as one.
    Finished(Vec<String>),
    /// A written migration failed with `R` in the language.
    Raised(R),
}

/// The runner's migrator over a binding's list: its `Migrations`, keeping
/// each warning of a migration the list does not know rather than logging it
/// through `tracing`, which reaches no binding's program, so the binding
/// can log it through the language's own logging.
// [spec:pgorm:req:bindings.migrations.unknown-applied]
#[derive(Debug)]
pub struct Migrator {
    migrations: Migrations,
    warnings: Mutex<Vec<String>>,
}

impl Migrator {
    /// The warnings kept since they were last taken, in the order given.
    pub fn take_warnings(&self) -> Vec<String> {
        std::mem::take(&mut *self.warnings.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[pgorm_migration::async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        self.migrations.migrations()
    }

    fn migration_table_name(&self) -> Name {
        self.migrations.migration_table_name()
    }

    fn migration_table_schema(&self) -> Option<Name> {
        self.migrations.migration_table_schema()
    }

    fn statement_cache_channel(&self) -> Option<String> {
        self.migrations.statement_cache_channel()
    }

    fn unknown_migrations(&self) -> UnknownMigrations {
        self.migrations.unknown_migrations()
    }

    fn warn_unknown_migrations(&self, versions: &[String]) {
        self.warnings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(
                versions
                    .iter()
                    .map(|version| UnknownMigrations::warning(version)),
            );
    }
}

/// What a binding's status read found: each migration's name and whether
/// the ledger records it applied, and the warnings the read gave.
#[derive(Debug)]
pub struct Statuses {
    /// Each migration's name and whether it is applied, in the list's order.
    pub migrations: Vec<(String, bool)>,
    /// A warning for each applied migration the list does not know.
    pub warnings: Vec<String>,
}

/// Each migration's name and whether the ledger records it applied, as
/// `migrator` reads them on `db`, and the warnings the read gave.
// [spec:pgorm:req:bindings.migrations.unknown-applied]    a read's warnings reach the binding
pub async fn statuses<C: pgorm::ConnectionTrait>(
    migrator: &Migrator,
    db: &C,
) -> Result<Statuses, Error> {
    let migrations = migrator.get_migration_with_status(db).await?;
    Ok(Statuses {
        migrations: migrations
            .iter()
            .map(|migration| {
                (
                    migration.name().to_owned(),
                    migration.status() == pgorm_migration::MigrationStatus::Applied,
                )
            })
            .collect(),
        warnings: migrator.take_warnings(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host whose pool is never reached: the test reads the list only.
    struct Unreached;

    impl Host for Unreached {
        type Object = ();
        type Owner = ();
        type Raised = ();

        fn pool(&self) -> &Arc<PoolState<()>> {
            unreachable!("the ledger's migrator runs no written migration")
        }

        async fn up(
            self: &Arc<Self>,
            _: &Arc<Written<()>>,
            _: Arc<State<()>>,
        ) -> Result<(), Failed<()>> {
            Ok(())
        }
    }

    // [spec:pgorm:req:bindings.migrations/test]    name, checksum and commits_after read once
    #[test]
    fn a_written_migration_keeps_what_was_read() {
        let written = Arc::new(Written {
            name: "0002_written".to_owned(),
            checksum: Some("abc".to_owned()),
            commits_after: true,
            object: (),
        });
        let listed = Listed::new(
            vec![
                Entry::Sql(SqlMigration::new("0001_sql", "SELECT 1")),
                Entry::Written(written),
            ],
            Some("ledger".to_owned()),
            None,
            None,
            UnknownMigrations::Warn,
        );
        let migrations = listed.ledger::<Unreached>().migrations();
        let read: Vec<_> = migrations
            .iter()
            .map(|migration| {
                (
                    migration.name().to_owned(),
                    migration.checksum().is_some(),
                    migration.commits_after(),
                )
            })
            .collect();
        assert_eq!(
            read,
            [
                ("0001_sql".to_owned(), true, false),
                ("0002_written".to_owned(), true, true),
            ]
        );
        assert_eq!(listed.len(), 2);
    }

    // [spec:pgorm:req:bindings.migrations.unknown-applied/test]    the setting reaches the runner, and
    // a warning is kept in the runner's words rather than logged
    #[test]
    fn the_ledgers_migrator_keeps_its_warnings() {
        let listed = Listed::<()>::new(Vec::new(), None, None, None, UnknownMigrations::Warn);
        let migrator = listed.ledger::<Unreached>();
        assert_eq!(migrator.unknown_migrations(), UnknownMigrations::Warn);
        migrator.warn_unknown_migrations(&["0009_log".to_owned()]);
        assert_eq!(
            migrator.take_warnings(),
            [UnknownMigrations::warning("0009_log")]
        );
        assert!(migrator.take_warnings().is_empty());
        let refusing = Listed::<()>::new(Vec::new(), None, None, None, UnknownMigrations::Refuse);
        assert_eq!(
            refusing.ledger::<Unreached>().unknown_migrations(),
            UnknownMigrations::Refuse
        );
    }
}
