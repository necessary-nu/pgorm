//! pgorm-migration's runner over a list a binding's program wrote: its
//! `SqlMigration`s are the crate's own, and a migration written in the
//! language is a `MigrationTrait` whose `up` hands the batch's transaction to
//! the language for as long as the object's `up` runs. The list is a
//! `pgorm_migration::Migrations`, so the ledger, the lock, the checksums,
//! `commits_after` and the statement-cache clear and notification are the
//! runner's alone.

use std::{
    future::Future,
    sync::{Arc, Mutex, PoisonError},
};

use pgorm::{DatabaseTransaction, Error, Name};
use pgorm_migration::{MigrationName, MigrationTrait, Migrations, MigratorTrait, SqlMigration};

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

/// A binding's list of migrations, its ledger placed and its channel named
/// as the program gave them.
// [spec:pgorm:req:bindings.migrations]
#[derive(Debug)]
pub struct Listed<T> {
    entries: Vec<Entry<T>>,
    table_name: Option<String>,
    table_schema: Option<String>,
    channel: Option<String>,
}

impl<T: Send + Sync + 'static> Listed<T> {
    /// The list `entries`, its ledger the table `table_name` in
    /// `table_schema` where they are given, notifying `channel`.
    pub fn new(
        entries: Vec<Entry<T>>,
        table_name: Option<String>,
        table_schema: Option<String>,
        channel: Option<String>,
    ) -> Self {
        Self {
            entries,
            table_name,
            table_schema,
            channel,
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
    fn migrator<H: Host<Object = T>>(&self, run: Option<&Arc<Run<H>>>) -> Migrations {
        let migrations = self.entries.iter().map(|entry| match entry {
            Entry::Sql(migration) => Box::new(migration.clone()) as Box<dyn MigrationTrait>,
            Entry::Written(written) => Box::new(Hosted {
                written: written.clone(),
                run: run.cloned(),
            }),
        });
        let mut migrator =
            Migrations::new(migrations).with_statement_cache_channel(self.channel.clone());
        if let Some(name) = &self.table_name {
            migrator = migrator.with_migration_table_name(Name::runtime(name));
        }
        if let Some(schema) = &self.table_schema {
            migrator = migrator.with_migration_table_schema(Name::runtime(schema));
        }
        migrator
    }

    /// The runner's migrator over this list for reading and installing its
    /// ledger, which runs no written migration.
    pub fn ledger<H: Host<Object = T>>(&self) -> Migrations {
        self.migrator::<H>(None)
    }

    /// pgorm-migration's `up` of `steps` pending migrations, or all of them,
    /// on connections of the host's pool: `Some` of what a written migration
    /// failed with in the language when one did, which the binding fails with
    /// in place of the runner's account of it.
    // [spec:pgorm:req:bindings.migrations]
    pub fn up<H: Host<Object = T>>(
        &self,
        host: H,
        steps: Option<u32>,
    ) -> impl Future<Output = Result<Option<H::Raised>, Failure>> + Send + 'static {
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
                (Ok(()), _) => Ok(None),
                (Err(_), Some(raised)) => Ok(Some(raised)),
                (Err(error), None) => Err(pool.failure(&error)),
            }
        }
    }
}

/// Each migration's name and whether the ledger records it applied, as
/// `migrator` reads them on `db`.
pub async fn statuses<C: pgorm::ConnectionTrait>(
    migrator: &Migrations,
    db: &C,
) -> Result<Vec<(String, bool)>, Error> {
    let migrations = migrator.get_migration_with_status(db).await?;
    Ok(migrations
        .iter()
        .map(|migration| {
            (
                migration.name().to_owned(),
                migration.status() == pgorm_migration::MigrationStatus::Applied,
            )
        })
        .collect())
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
}
