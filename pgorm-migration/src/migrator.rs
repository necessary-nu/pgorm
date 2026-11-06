use futures::Future;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Display;
use std::pin::Pin;
use std::time::SystemTime;
use tracing::info;

use super::{MigrationTrait, ledger};
use pgorm::pgorm_query::{
    ColumnDef, Expr, IntoName, Order, Query, Schema, SelectStatement, SqlName, Table, TableName,
};
use pgorm::{
    ActiveModelTrait, ConnectionTrait, DatabasePool, DatabaseTransaction, Error, FromQueryResult,
    Insert, Iterable, Name, TransactionError, TransactionTrait, set,
};

/// The name of the ledger's nullable digest column, as PostgreSQL stores it.
const CHECKSUM_COLUMN: &str = "checksum";

/// The table `migration_table_name()` resolves to unless a migrator overrides it.
// [spec:pgorm:def:migration.runner+4]    the ledger's default physical name
pub const DEFAULT_LEDGER_TABLE: &str = "pgorm_migrations";

/// The name this crate inherited from SeaORM and no longer creates. A database
/// last migrated by SeaORM, or by pgorm before the rename, keeps its ledger
/// here; `install` adopts it rather than leaving it to read as unmigrated.
// [spec:pgorm:req:migration.ledger-upgrade+2]    the name that is looked for
#[doc(alias = "seaql_migrations")]
pub const LEGACY_LEDGER_TABLE: &str = "seaql_migrations";

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
/// Status of migration
// [spec:pgorm:def:migration.runner+4]    reported status vocabulary
pub enum MigrationStatus {
    /// Not yet applied
    Pending,
    /// Applied
    Applied,
}

impl Display for MigrationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status = match self {
            MigrationStatus::Pending => "Pending",
            MigrationStatus::Applied => "Applied",
        };
        write!(f, "{status}")
    }
}

pub struct Migration {
    migration: Box<dyn MigrationTrait>,
    status: MigrationStatus,
}

impl Migration {
    /// Get migration name from MigrationName trait implementation
    pub fn name(&self) -> &str {
        self.migration.name()
    }

    /// Get migration status
    pub fn status(&self) -> MigrationStatus {
        self.status
    }
}

/// Performing migrations on a database
// [spec:pgorm:def:migration.runner+4]    runner surface
// [spec:pgorm:req:migration.up-only]    no down/fresh/refresh/reset
#[async_trait::async_trait]
pub trait MigratorTrait: Send {
    /// Vector of migrations in time sequence
    fn migrations() -> Vec<Box<dyn MigrationTrait>>;

    /// Name of the migration table, it is `pgorm_migrations` by default
    ///
    /// Overriding this takes the ledger out of the crate's hands: legacy-name
    /// adoption applies to the default name alone, so a custom-named ledger is
    /// never renamed and never adopted from.
    // [spec:pgorm:req:migration.ledger-upgrade+2]    an override opts out of adoption
    fn migration_table_name() -> Name {
        ledger::Entity.into_name()
    }

    /// The schema the migration table lives in, `None` by default.
    ///
    /// `None` leaves the table's name unqualified, so it resolves through
    /// `search_path` as any unqualified name does. A schema keeps the ledger
    /// apart from the application's tables and whatever triggers or policies
    /// they carry: `install` creates the schema when it is missing, and every
    /// read and write names the table qualified by it. Legacy-name adoption
    /// applies only to the default name with no schema, so a ledger given one
    /// is never adopted into.
    // [spec:pgorm:def:migration.runner+4]    the ledger's schema
    // [spec:pgorm:req:migration.ledger-upgrade+2]    a schema opts out of adoption
    fn migration_table_schema() -> Option<Name> {
        None
    }

    /// Get list of migrations wrapped in `Migration` struct, failing if two of
    /// them answer `name()` with the same string
    // [spec:pgorm:sem:migration.name+4]    duplicates are rejected here, ahead of every other step
    fn get_migration_files() -> Result<Vec<Migration>, Error> {
        let migrations = Self::migrations();

        let mut seen: HashSet<&str> = HashSet::new();
        let mut duplicates: BTreeSet<String> = BTreeSet::new();
        for migration in &migrations {
            let name = migration.name();
            if !seen.insert(name) {
                duplicates.insert(name.to_owned());
            }
        }

        if !duplicates.is_empty() {
            let names = duplicates
                .iter()
                .map(|name| format!("'{name}'"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Error::Custom(format!(
                "Duplicate migration name(s): {names}. A migration's name is its ledger identity, \
                 so it must be unique; note that `MigrationName` uses the file stem alone, \
                 which makes two same-named files in different directories collide"
            )));
        }

        Ok(migrations
            .into_iter()
            .map(|migration| Migration {
                migration,
                status: MigrationStatus::Pending,
            })
            .collect())
    }

    /// The key this migrator's advisory lock is taken on: a stable FNV-1a hash
    /// of the ledger table's name, so two runners contend whenever they would
    /// write the same ledger, and never with an unrelated application lock that
    /// happened to pick a round number. The schema is not hashed: ledgers of one
    /// name in two schemas share a key, which orders their runs and nothing
    /// more, and the default ledger keeps the key every earlier build took.
    ///
    /// The hash is spelled out here rather than taken from `DefaultHasher`
    /// because the key has to agree between processes that need not share a
    /// compiler version, let alone a `HashMap` seed.
    // [spec:pgorm:sem:migration.up+5]
    fn lock_key() -> i64 {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

        let name = SqlName::to_string(&*Self::migration_table_name());
        let mut hash = FNV_OFFSET;
        for byte in name.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash as i64
    }

    /// Take the migration advisory lock, blocking until any concurrent runner
    /// against the same ledger has committed or rolled back.
    ///
    /// The argument is a transaction rather than any `ConnectionTrait` because
    /// `pg_advisory_xact_lock` is released at end of transaction: taken in
    /// autocommit it would be surrendered before the caller's next statement,
    /// which is the shape of the bug rather than the fix.
    // [spec:pgorm:sem:migration.up+5]
    async fn lock(db: &DatabaseTransaction<'_>) -> Result<(), Error> {
        let key = Self::lock_key();
        db.execute("SELECT pg_advisory_xact_lock($1)", &[&key])
            .await?;
        tracing::debug!("Acquired migration advisory lock {key}");
        Ok(())
    }

    /// Get list of applied migrations from database.
    ///
    /// Reads and nothing else: the ledger is located in the catalogue rather
    /// than provisioned, so a database that has never been migrated answers
    /// with an empty list instead of gaining a table. A ledger still sitting
    /// under the legacy name is read where it stands, and one predating the
    /// checksum column is read without it.
    // [spec:pgorm:req:migration.read-only+1]    a read locates the ledger, never creates it
    async fn get_migration_models(
        db: &(impl ConnectionTrait),
    ) -> Result<Vec<ledger::Model>, Error> {
        let Some(ledger) = ReadableLedger::locate(db, LedgerTable::of::<Self>()).await? else {
            return Ok(Vec::new());
        };

        let mut stmt = Query::select().table_name(ledger.table);
        stmt.order_by(ledger::Column::Version, Order::Asc);
        if ledger.has_checksum {
            stmt.columns(ledger::Column::iter().map(IntoName::into_name));
        } else {
            // The column is absent rather than empty, so it cannot be named;
            // a bound text NULL stands in, which is the same answer every row
            // of a widened ledger would have given and the same answer the
            // three-valued checksum check grandfathers.
            // [spec:pgorm:req:migration.checksum+1]
            stmt.column(ledger::Column::Version.into_name())
                .column(ledger::Column::AppliedAt.into_name())
                .expr_as(
                    Expr::val(Option::<String>::None),
                    ledger::Column::Checksum.into_name(),
                );
        }

        let (stmt, values) = stmt.build();
        ledger::Model::find_by_statement(&stmt, values)
            .all(db)
            .await
    }

    /// Get list of migrations with status
    // [spec:pgorm:sem:migration.up+5]    pending set difference + missing-file detection
    // [spec:pgorm:req:migration.checksum+1]    recorded digests are checked on every read
    // [spec:pgorm:req:migration.read-only+1]    status is computed from a read alone
    async fn get_migration_with_status(
        db: &(impl ConnectionTrait),
    ) -> Result<Vec<Migration>, Error> {
        let mut migration_files = Self::get_migration_files()?;
        let migration_models = Self::get_migration_models(db).await?;

        let applied_checksums: HashMap<String, Option<String>> = migration_models
            .into_iter()
            .map(|model| (model.version, model.checksum))
            .collect();
        let migration_in_db: HashSet<String> = applied_checksums.keys().cloned().collect();
        let migration_in_fs: HashSet<String> = migration_files
            .iter()
            .map(|file| file.migration.name().to_string())
            .collect();

        let pending_migrations = &migration_in_fs - &migration_in_db;
        let mut checksum_errors: Vec<String> = Vec::new();
        for migration_file in migration_files.iter_mut() {
            if pending_migrations.contains(migration_file.migration.name()) {
                continue;
            }
            migration_file.status = MigrationStatus::Applied;

            let recorded = applied_checksums
                .get(migration_file.migration.name())
                .and_then(Option::as_deref);
            if let (Some(recorded), Some(current)) = (recorded, migration_file.migration.checksum())
                && recorded != current
            {
                checksum_errors.push(format!(
                    "Migration '{}' was applied with checksum '{recorded}' but now reports '{current}', so its contents have changed since it was applied; correct it with a new migration rather than by editing this one",
                    migration_file.migration.name()
                ));
            }
        }

        let missing_migrations_in_fs = &migration_in_db - &migration_in_fs;
        let mut errors: Vec<String> = missing_migrations_in_fs
            .iter()
            .map(|missing_migration| {
                format!("Migration file of version '{missing_migration}' is missing, this migration has been applied but its file is missing")
            }).collect();
        errors.extend(checksum_errors);

        if !errors.is_empty() {
            Err(Error::Custom(errors.join("\n")))
        } else {
            Ok(migration_files)
        }
    }

    /// Get list of pending migrations.
    ///
    /// A read, like every accessor here: against a database with no ledger at
    /// all, every migration is pending and nothing is installed to say so.
    // [spec:pgorm:req:migration.read-only+1]
    async fn get_pending_migrations(db: &(impl ConnectionTrait)) -> Result<Vec<Migration>, Error> {
        Ok(Self::get_migration_with_status(db)
            .await?
            .into_iter()
            .filter(|file| file.status == MigrationStatus::Pending)
            .collect())
    }

    /// Get list of applied migrations.
    ///
    /// A read, like every accessor here.
    // [spec:pgorm:req:migration.read-only+1]
    async fn get_applied_migrations(db: &(impl ConnectionTrait)) -> Result<Vec<Migration>, Error> {
        Ok(Self::get_migration_with_status(db)
            .await?
            .into_iter()
            .filter(|file| file.status == MigrationStatus::Applied)
            .collect())
    }

    /// Take over a ledger left under the legacy `seaql_migrations` name, so a
    /// database migrated before the rename is not mistaken for a fresh one.
    ///
    /// The take-over is a rename, not a copy: one ledger exists at any moment,
    /// so there is no window in which two of them can disagree. It applies only
    /// when `migration_table_name()` is the default and `migration_table_schema()`
    /// is `None` — a custom name or a schema is the caller's own placement, and
    /// is left exactly where they put it — and only when the new name is absent,
    /// so a database that already has both keeps both and the legacy table is
    /// not touched.
    // [spec:pgorm:req:migration.ledger-upgrade+2]    detect, then rename in place
    #[doc(alias = "seaql_migrations")]
    async fn adopt_legacy_ledger(db: &(impl ConnectionTrait)) -> Result<(), Error> {
        if !LedgerTable::of::<Self>().adopts_legacy() {
            return Ok(());
        }

        // The steady state — for a fresh database and an upgraded one alike —
        // is that there is nothing to adopt, and this read settles it without
        // taking a lock or naming a relation the server must lock to resolve.
        let adoptable: bool = db
            .query_one(
                "SELECT to_regclass($1) IS NULL AND to_regclass($2) IS NOT NULL",
                &[&DEFAULT_LEDGER_TABLE, &LEGACY_LEDGER_TABLE],
            )
            .await?
            .get(0);
        if !adoptable {
            return Ok(());
        }

        // The condition is then re-tested inside the server, under the
        // migrator's advisory lock, because `install` is also reached from
        // accessors running in autocommit, where nothing orders two racing
        // adoptions. A `DO` block is a single statement and therefore a single
        // implicit transaction, which is exactly the extent a transaction-scoped
        // lock needs to cover; a caller already holding that lock — every `up` —
        // takes it re-entrantly and is unaffected.
        //
        // The lock orders the adopters but does not by itself settle the guard.
        // `to_regclass` resolves a name without taking a relation lock, so it
        // never processes the invalidation messages the winner's rename sent and
        // can still answer from a catalog snapshot taken before it: the loser
        // reaches the `ALTER`, whose own lock acquisition re-resolves the name
        // and finds it gone. Those two outcomes — the source renamed away, or
        // the target already there — are the race resolving itself in our
        // favour, not failures, so they are caught and the block does nothing.
        // The handler's subtransaction keeps them off a caller's transaction,
        // which inside `up` is carrying the whole migration batch.
        let key = Self::lock_key();
        db.execute(
            &format!(
                "DO $adopt$ BEGIN \
                 PERFORM pg_advisory_xact_lock({key}); \
                 IF to_regclass('{DEFAULT_LEDGER_TABLE}') IS NULL \
                 AND to_regclass('{LEGACY_LEDGER_TABLE}') IS NOT NULL THEN \
                 ALTER TABLE \"{LEGACY_LEDGER_TABLE}\" RENAME TO \"{DEFAULT_LEDGER_TABLE}\"; \
                 END IF; \
                 EXCEPTION WHEN undefined_table OR duplicate_table THEN NULL; \
                 END $adopt$"
            ),
            &[],
        )
        .await?;
        tracing::debug!("Adopted the legacy '{LEGACY_LEDGER_TABLE}' ledger");

        Ok(())
    }

    /// Create migration table `pgorm_migrations` in the database.
    ///
    /// The one provisioning entry point, and the only method here that issues
    /// DDL: it adopts a legacy-named ledger, creates the ledger's schema and
    /// then the table if either is absent, and widens a table that predates the
    /// checksum column. `up` calls it; no read does, so asking a question never
    /// needs the privileges to answer it by changing the schema.
    // [spec:pgorm:def:migration.runner+4]    self-provisioning ledger under migration_table_name(), in its schema
    // [spec:pgorm:req:migration.checksum+1]    a ledger predating the column is widened in place
    // [spec:pgorm:req:migration.ledger-upgrade+2]    adoption precedes creation, widening follows it
    // [spec:pgorm:req:migration.read-only+1]    the write path a read is split away from
    async fn install(db: &(impl ConnectionTrait)) -> Result<(), Error> {
        // Ahead of the create, or the create would answer a legacy database
        // with an empty ledger beside the populated one.
        Self::adopt_legacy_ledger(db).await?;

        let ledger = LedgerTable::of::<Self>();
        create_ledger_schema(db, &ledger).await?;

        let stmt = Table::create(ledger.qualified())
            .if_not_exists()
            .col(ColumnDef::new(ledger::Column::Version).text().not_null())
            .primary_key(ledger::Column::Version)
            .col(
                ColumnDef::new(ledger::Column::AppliedAt)
                    .big_integer()
                    .not_null(),
            )
            .col(ColumnDef::new(ledger::Column::Checksum).text().null())
            .to_owned();
        db.execute(&stmt.to_string(), &[]).await?;

        // A ledger created before the column existed is widened rather than
        // recreated, leaving its rows NULL. This runs after adoption so an
        // adopted table is widened by the same step, rather than needing one of
        // its own. The catalog is consulted first because `ADD COLUMN IF NOT
        // EXISTS` takes an ACCESS EXCLUSIVE lock even when it goes on to do
        // nothing, and this is the provisioning path a caller may reach often.
        if !has_checksum_column(db, ledger.schema_text().as_deref(), &ledger.name_text()).await? {
            let stmt = Table::alter(ledger.qualified())
                .add_column_if_not_exists(ColumnDef::new(ledger::Column::Checksum).text().null());
            db.execute(&stmt.to_string(), &[]).await?;
            tracing::debug!("Widened the migration ledger with a checksum column");
        }

        tracing::debug!("Installed");
        Ok(())
    }

    /// Check the status of all migrations.
    ///
    /// A read: it logs what it found and leaves the database exactly as it was,
    /// so a readiness probe or a CI gate can call it against a connection with
    /// no DDL privileges at all.
    // [spec:pgorm:req:migration.read-only+1]
    async fn status(db: &(impl ConnectionTrait)) -> Result<(), Error> {
        info!("Checking migration status");

        for Migration { migration, status } in Self::get_migration_with_status(db).await? {
            info!("Migration '{}'... {}", migration.name(), status);
        }

        Ok(())
    }

    /// The channel `up` notifies once a batch that applied a migration
    /// commits, so that every pool listening there clears its statement
    /// caches; `None` notifies nothing. It is
    /// [`STATEMENT_CACHE_CHANNEL`](pgorm::STATEMENT_CACHE_CHANNEL) by default,
    /// the one a pool given that channel as its
    /// [`statement_cache_channel`](pgorm::ManagerConfig::statement_cache_channel)
    /// listens on.
    // [spec:pgorm:sem:migration.up+5]    the channel the runner notifies
    fn statement_cache_channel() -> Option<String> {
        Some(pgorm::STATEMENT_CACHE_CHANNEL.to_owned())
    }

    /// Apply pending migrations, then clear the pool's cached statements if the
    /// committed batch applied any, notifying every other pool listening on
    /// [`statement_cache_channel`](MigratorTrait::statement_cache_channel).
    ///
    /// A migration whose [`commits_after`](MigrationTrait::commits_after)
    /// answers `true` is committed with its ledger row before the next one
    /// runs, and the rest run in a new transaction that takes the lock again;
    /// each commit clears and notifies as the end of a batch does.
    // [spec:pgorm:sem:migration.up+5]
    // [spec:pgorm:req:migration.commit-after]
    async fn up(db: DatabasePool, mut steps: Option<u32>) -> Result<(), Error> {
        tracing::debug!("Applying migrations");
        let channel = Self::statement_cache_channel();
        loop {
            let channel = channel.clone();
            let batch = exec_with_connection(&db, move |manager| {
                tracing::debug!("Exec up");
                let channel = channel.clone();
                Box::pin(async move {
                    let batch = exec_up::<Self>(manager, steps).await?;
                    // Sent inside the batch's transaction, the notification is
                    // delivered exactly when the batch commits, and never if it
                    // rolls back.
                    // [spec:pgorm:sem:migration.up+5]    other processes' pools hear of a committed batch
                    // [spec:pgorm:req:migration.commit-after]    and of each commit inside one
                    if batch.applied > 0
                        && let Some(channel) = channel
                    {
                        pgorm::notify(manager, &channel, "").await?;
                    }
                    Ok(batch)
                })
            })
            .await?;

            // The pool's connections may hold statements planned against the
            // schema the batch changed, and PostgreSQL does not re-plan every
            // such change: an insert prepared before a domain's default
            // changed keeps the old one. The connection the next transaction
            // takes is one of them.
            // [spec:pgorm:req:conn.pool.statement-cache.clear+1]    the runner clears after a batch commits
            // [spec:pgorm:req:migration.commit-after]    and after each commit inside one
            if batch.applied > 0 {
                db.clear_statement_caches();
            }
            match batch.resume {
                Some(rest) => steps = rest,
                None => return Ok(()),
            }
        }
    }
}

/// What one transaction of [`MigratorTrait::up`] applied, and the step bound
/// the next one starts from, when it stopped after a migration that asked to
/// be committed with migrations still to run.
// [spec:pgorm:req:migration.commit-after]
struct Batch {
    applied: usize,
    resume: Option<Option<u32>>,
}

/// Where a migrator's ledger lives: `migration_table_name()`, qualified by
/// `migration_table_schema()` when that is set.
// [spec:pgorm:def:migration.runner+4]    every read and write names the qualified table
struct LedgerTable {
    schema: Option<Name>,
    name: Name,
}

impl LedgerTable {
    fn of<M: MigratorTrait + ?Sized>() -> Self {
        Self {
            schema: M::migration_table_schema(),
            name: M::migration_table_name(),
        }
    }

    fn qualified(&self) -> TableName {
        match &self.schema {
            Some(schema) => TableName::SchemaTable(schema.clone(), self.name.clone()),
            None => TableName::Table(self.name.clone()),
        }
    }

    fn schema_text(&self) -> Option<String> {
        self.schema
            .as_ref()
            .map(|schema| SqlName::to_string(&**schema))
    }

    fn name_text(&self) -> String {
        SqlName::to_string(&*self.name)
    }

    /// Whether this is the ledger legacy adoption applies to: the default name,
    /// with no schema, so it lands wherever `search_path` puts the legacy table.
    // [spec:pgorm:req:migration.ledger-upgrade+2]    the default name in the default place
    fn adopts_legacy(&self) -> bool {
        self.schema.is_none() && self.name_text() == DEFAULT_LEDGER_TABLE
    }
}

/// Create the ledger's schema when one is configured and it is missing.
///
/// The catalogue is read first because `CREATE SCHEMA IF NOT EXISTS` checks the
/// database's `CREATE` privilege before it checks whether the schema exists: a
/// role that owns a schema made for it, without `CREATE` on the database, would
/// be refused on every run although there is nothing to create.
// [spec:pgorm:def:migration.runner+4]    the schema is created before the ledger
async fn create_ledger_schema(
    db: &impl ConnectionTrait,
    ledger: &LedgerTable,
) -> Result<(), Error> {
    let (Some(schema), Some(schema_text)) = (&ledger.schema, ledger.schema_text()) else {
        return Ok(());
    };
    let exists: bool = db
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1)",
            &[&schema_text],
        )
        .await?
        .get(0);
    if !exists {
        let stmt = Schema::create(schema.clone()).if_not_exists().to_string();
        db.execute(&stmt, &[]).await?;
        tracing::debug!("Created the migration ledger's schema '{schema_text}'");
    }
    Ok(())
}

/// Whether `table`, in `schema` or else in the current schema, already carries
/// the ledger's nullable digest column.
///
/// Both halves of the crate need the answer and neither may guess it: `install`
/// widens a table that predates the column, and a read has to know whether the
/// column can be named at all. A configured schema is named outright, so a
/// same-named table elsewhere on `search_path` cannot answer for the ledger.
// [spec:pgorm:req:migration.checksum+1]    the catalogue is what says the column exists
// [spec:pgorm:req:migration.read-only+1]    the column is looked for in the ledger's own schema
async fn has_checksum_column(
    db: &impl ConnectionTrait,
    schema: Option<&str>,
    table: &str,
) -> Result<bool, Error> {
    Ok(db
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_schema = coalesce($1, current_schema()) \
             AND table_name = $2 AND column_name = $3)",
            &[&schema, &table, &CHECKSUM_COLUMN],
        )
        .await?
        .get(0))
}

/// A ledger a read may use, as the catalogue reports it — never as a read would
/// like it to be.
///
/// This is the read path's counterpart to `install`: it answers what is there,
/// where `install` makes what should be there. The two facts a read needs are
/// which table holds the rows and whether that table has the checksum column,
/// because a ledger older than the column cannot have it projected by name.
// [spec:pgorm:req:migration.read-only+1]    reads resolve the ledger, they do not provision it
struct ReadableLedger {
    /// The table to select from: the configured ledger, or the legacy name
    /// when a ledger is still sitting there unadopted.
    table: TableName,
    /// Whether `table` has the `checksum` column yet.
    has_checksum: bool,
}

impl ReadableLedger {
    /// Locate the ledger for `configured` using catalogue reads alone.
    ///
    /// `None` means no ledger exists under either name, which a caller reads as
    /// nothing applied — the honest answer for a database that has never been
    /// migrated, and one that costs it no table.
    ///
    /// A ledger still under `LEGACY_LEDGER_TABLE` is read where it stands
    /// rather than reported as absent. Reporting it absent would recreate, in
    /// the read path, exactly the failure `migration.ledger-upgrade` exists to
    /// prevent: an upgraded deployment's entire applied history read as
    /// pending. Adoption stays `install`'s job, so the answer is the same
    /// before and after the rename and only `up` moves the table.
    ///
    /// The legacy name is consulted under adoption's own first gate — the
    /// default name with no schema — so a caller who placed their own ledger is
    /// never silently answered from someone else's.
    // [spec:pgorm:req:migration.read-only+1]
    // [spec:pgorm:req:migration.ledger-upgrade+2]    an unadopted ledger is read, not renamed
    async fn locate(
        db: &impl ConnectionTrait,
        configured: LedgerTable,
    ) -> Result<Option<ReadableLedger>, Error> {
        let schema = configured.schema_text();
        let name = configured.name_text();
        let legacy = configured.adopts_legacy().then_some(LEGACY_LEDGER_TABLE);

        // One catalogue read settles both names, and `to_regclass` takes no
        // relation lock to answer. `quote_ident` is what makes the lookup agree
        // with how the name is rendered everywhere else in the crate: a
        // mixed-case ledger is `"MyLedger"`, not the `myledger` a bare
        // identifier would fold to. A configured schema qualifies the first
        // name, so it is looked up there and nowhere else; `concat_ws` drops a
        // `NULL` schema, leaving the bare name to `search_path`. A `NULL`
        // legacy name — a ledger placed by the caller, which is never adopted
        // from — resolves to `NULL` and so never matches.
        let row = db
            .query_one(
                "SELECT to_regclass(concat_ws('.', quote_ident($1), quote_ident($2))) IS NOT NULL, \
                 to_regclass(quote_ident($3)) IS NOT NULL",
                &[&schema, &name, &legacy],
            )
            .await?;

        let ledger = if row.get::<_, bool>(0) {
            let has_checksum = has_checksum_column(db, schema.as_deref(), &name).await?;
            ReadableLedger {
                table: configured.qualified(),
                has_checksum,
            }
        } else if row.get::<_, bool>(1) {
            ReadableLedger {
                table: TableName::Table(Name::runtime(LEGACY_LEDGER_TABLE)),
                has_checksum: has_checksum_column(db, None, LEGACY_LEDGER_TABLE).await?,
            }
        } else {
            return Ok(None);
        };

        Ok(Some(ledger))
    }
}

// [spec:pgorm:sem:migration.up+5]    one connection, one transaction: the whole batch, or one part of a batch a migration splits
async fn exec_with_connection<T, F>(db: &DatabasePool, f: F) -> Result<T, Error>
where
    F: for<'b> Fn(
        &'b DatabaseTransaction<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'b>>,
{
    let mut conn = db.get().await?;
    match conn
        .transaction(async |transaction| f(transaction).await)
        .await
    {
        Ok(outcome) => Ok(outcome),
        Err(TransactionError::Connection(error) | TransactionError::Transaction(error)) => {
            Err(error)
        }
        // The connection's state is unknown once its ROLLBACK has failed, so
        // the pool does not get it back.
        Err(TransactionError::RollbackFailed { cause, .. }) => {
            conn.discard();
            Err(cause)
        }
        // A failure with no arm here leaves the connection's state as unknown
        // as a failed ROLLBACK does.
        Err(other) => {
            conn.discard();
            Err(Error::Custom(other.to_string()))
        }
    }
}

/// Applies the pending migrations inside `db`, answering how many it applied
/// and, where a migration asked to be committed before the rest, the step
/// bound left for them.
// [spec:pgorm:sem:migration.up+5]    lock, then declaration-order application, step bound, ledger append
// [spec:pgorm:req:migration.commit-after]    the lock is retaken and the pending set recomputed per transaction
async fn exec_up<M>(db: &DatabaseTransaction<'_>, mut steps: Option<u32>) -> Result<Batch, Error>
where
    M: MigratorTrait + ?Sized,
{
    // A list that names the same migration twice cannot be applied coherently,
    // so it is rejected before the lock is taken and before the ledger exists —
    // there is no reason to make a concurrent runner queue behind it.
    // [spec:pgorm:sem:migration.name+4]
    M::get_migration_files()?;

    // Before anything reads or writes the ledger: a concurrent runner is made
    // to wait here rather than collide with the ledger's primary key or race
    // this transaction's `CREATE TABLE IF NOT EXISTS`. Whoever waits recomputes
    // the pending set below, after the winner's commit is visible, and finds
    // nothing left to do. Taking the lock first also puts the legacy-name
    // adoption inside `install` under it, so a batch cannot begin against a
    // ledger another runner is in the middle of renaming.
    // [spec:pgorm:req:migration.ledger-upgrade+2]
    M::lock(db).await?;
    M::install(db).await?;

    if let Some(steps) = steps {
        info!("Applying {} pending migrations", steps);
    } else {
        info!("Applying all pending migrations");
    }

    let mut migrations = M::get_pending_migrations(db).await?.into_iter().peekable();
    if migrations.len() == 0 {
        info!("No pending migrations");
    }

    let mut applied = 0;
    while let Some(Migration { migration, .. }) = migrations.next() {
        if let Some(steps) = steps.as_mut() {
            if steps == &0 {
                break;
            }
            *steps -= 1;
        }
        info!("Applying migration '{}'", migration.name());
        migration.up(db).await?;
        info!("Migration '{}' has been applied", migration.name());
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|err| {
                Error::Custom(format!("system clock is before the Unix epoch: {err}"))
            })?;
        Insert::one(ledger::ActiveModel {
            version: set(migration.name()),
            applied_at: set(now.as_secs() as i64),
            checksum: set(migration.checksum()),
        })
        .table_name(LedgerTable::of::<M>().qualified())
        .exec(db)
        .await?;
        applied += 1;

        if migration.commits_after() {
            info!("Committing after migration '{}'", migration.name());
            let more = migrations.peek().is_some() && steps != Some(0);
            return Ok(Batch {
                applied,
                resume: more.then_some(steps),
            });
        }
    }

    Ok(Batch {
        applied,
        resume: None,
    })
}

trait QueryTable {
    type Statement;

    fn table_name(self, table_name: TableName) -> Self::Statement;
}

impl QueryTable for SelectStatement {
    type Statement = SelectStatement;

    fn table_name(mut self, table_name: TableName) -> SelectStatement {
        self.from(table_name);
        self
    }
}

impl<A, R> QueryTable for pgorm::Insert<A, R>
where
    A: ActiveModelTrait,
{
    type Statement = pgorm::Insert<A, R>;

    fn table_name(mut self, table_name: TableName) -> pgorm::Insert<A, R> {
        pgorm::QueryTrait::query(&mut self).into_table(table_name);
        self
    }
}
