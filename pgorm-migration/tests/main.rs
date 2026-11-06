mod common;

use common::setup::{
    TestContext, count_rows, count_rows_in, has_column, has_column_in, has_index, has_schema,
    has_table,
};
use pgorm_migration::migrator::MigrationStatus;
use pgorm_migration::prelude::*;

// DATABASE_URL=postgres://postgres:postgres@127.0.0.1:54329 cargo test -p pgorm-migration

/// A fresh database has nothing applied; `up` installs the tracking table and
/// runs every pending migration in order.
// [spec:pgorm:def:migration.runner+5/test]
// [spec:pgorm:sem:migration.up+6/test]
// [spec:pgorm:sem:migration.name+4/test]    asserted names are file stems
// [spec:pgorm:req:migration.ledger-upgrade+2/test]    the fresh-install half: the legacy name is never created
// [spec:pgorm:def:macros.derive+7/test]    `MigrationName` names each migration after the file stem
// [spec:pgorm:req:migration.read-only+1/test]    the accessors leave a fresh database fresh
#[tokio::test]
async fn fresh_install_applies_all_pending() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_fresh").await;
    let db = &ctx.db;

    assert!(!has_table(db, "pgorm_migrations").await?);

    let pending = common::migrator::default::Migrator
        .get_pending_migrations(&db.get().await?)
        .await?;
    assert_eq!(pending.len(), 5);
    assert_eq!(pending[0].name(), "m20220118_000001_create_cake_table");
    assert_eq!(pending[0].status(), MigrationStatus::Pending);
    // Asking did not answer by building the thing it asked about.
    assert!(!has_table(db, "pgorm_migrations").await?);

    common::migrator::default::Migrator
        .up(db.clone(), None)
        .await?;

    assert!(has_table(db, "pgorm_migrations").await?);
    assert!(has_column(db, "pgorm_migrations", "checksum").await?);
    // A database pgorm built itself never carries the inherited name.
    assert!(!has_table(db, "seaql_migrations").await?);
    assert!(has_table(db, "cake").await?);
    assert!(has_table(db, "fruit").await?);
    assert!(has_index(db, "cake", "cake_name_index").await?);
    assert!(!has_index(db, "cake", "non_existent_index").await?);

    // One row from the ActiveModel seed, one from the query-builder seed.
    assert_eq!(count_rows(db, "cake").await?, 2);
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 5);

    let applied = common::migrator::default::Migrator
        .get_applied_migrations(&db.get().await?)
        .await?;
    assert_eq!(applied.len(), 5);
    assert_eq!(applied[0].name(), "m20220118_000001_create_cake_table");
    assert_eq!(applied[0].status(), MigrationStatus::Applied);
    assert!(
        common::migrator::default::Migrator
            .get_pending_migrations(&db.get().await?)
            .await?
            .is_empty()
    );

    ctx.delete().await;
    Ok(())
}

/// Re-running `up` on an already-migrated database is a no-op: no migration is
/// re-applied and no extra ledger row is written.
// [spec:pgorm:sem:migration.up+6/test]
#[tokio::test]
async fn repeated_up_is_idempotent() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_idempotent").await;
    let db = &ctx.db;

    common::migrator::default::Migrator
        .up(db.clone(), None)
        .await?;
    let first = count_rows(db, "cake").await?;

    common::migrator::default::Migrator
        .up(db.clone(), None)
        .await?;
    common::migrator::default::Migrator
        .up(db.clone(), None)
        .await?;

    assert_eq!(count_rows(db, "cake").await?, first);
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 5);
    assert!(
        common::migrator::default::Migrator
            .get_pending_migrations(&db.get().await?)
            .await?
            .is_empty()
    );

    ctx.delete().await;
    Ok(())
}

/// `steps` bounds how many pending migrations are applied, and `status` reports
/// the split without altering it.
// [spec:pgorm:def:migration.runner+5/test]
// [spec:pgorm:sem:migration.up+6/test]
#[tokio::test]
async fn stepped_up_reports_status() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_status").await;
    let db = &ctx.db;

    // `install` alone creates the ledger without applying anything.
    common::migrator::default::Migrator
        .install(&db.get().await?)
        .await?;
    assert!(has_table(db, "pgorm_migrations").await?);
    assert!(!has_table(db, "cake").await?);

    common::migrator::default::Migrator
        .up(db.clone(), Some(0))
        .await?;
    assert!(!has_table(db, "cake").await?);

    common::migrator::default::Migrator
        .up(db.clone(), Some(1))
        .await?;
    assert!(has_table(db, "cake").await?);
    assert!(!has_table(db, "fruit").await?);

    let conn = db.get().await?;
    common::migrator::default::Migrator.status(&conn).await?;

    let applied = common::migrator::default::Migrator
        .get_applied_migrations(&conn)
        .await?;
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].name(), "m20220118_000001_create_cake_table");

    let pending = common::migrator::default::Migrator
        .get_pending_migrations(&conn)
        .await?;
    assert_eq!(pending.len(), 4);
    assert_eq!(pending[0].name(), "m20220118_000002_create_fruit_table");

    drop(conn);
    ctx.delete().await;
    Ok(())
}

/// `migration_table_name` is honoured everywhere, including by `install`.
// [spec:pgorm:def:migration.runner+5/test]
#[tokio::test]
async fn migration_table_name_is_overridable() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_table_name").await;
    let db = &ctx.db;

    common::migrator::override_migration_table_name::Migrator
        .up(db.clone(), None)
        .await?;

    assert!(has_table(db, "override_migration_table_name").await?);
    assert!(!has_table(db, "pgorm_migrations").await?);
    assert_eq!(count_rows(db, "override_migration_table_name").await?, 5);
    assert!(has_table(db, "cake").await?);

    ctx.delete().await;
    Ok(())
}

/// The whole `up` run shares one transaction, so a failing migration rolls back
/// every migration in the run along with the ledger rows.
// [spec:pgorm:sem:migration.up+6/test]
// [spec:pgorm:req:migration.up-only/test]    no rollback path other than the transaction
// [spec:pgorm:req:migration.commit-after/test]    a batch with no migration asking to be committed is one transaction
#[tokio::test]
async fn failed_migration_rolls_back_the_run() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_abort").await;
    let db = &ctx.db;

    let err = common::migrator::abort::Migrator
        .up(db.clone(), None)
        .await
        .expect_err("the final migration must fail");
    assert!(matches!(err, Error::Custom(ref msg) if msg == "Abort migration"));

    assert!(!has_table(db, "cake").await?);
    assert!(!has_table(db, "fruit").await?);
    assert!(!has_table(db, "pgorm_migrations").await?);

    ctx.delete().await;
    Ok(())
}

/// Two migrations answering `name()` with the same string share one ledger row,
/// so the list is refused — by `status` and by `up` alike — before anything is
/// installed, read or applied.
// [spec:pgorm:sem:migration.name+4/test]
#[tokio::test]
async fn duplicate_migration_names_are_rejected() -> Result<(), Error> {
    use common::migrator::pinned::{Duplicated, REPEATED};

    let ctx = TestContext::new("pgorm_migration_duplicate").await;
    let db = &ctx.db;

    let conn = db.get().await?;
    let err = Duplicated
        .status(&conn)
        .await
        .expect_err("a repeated migration name must be refused");
    let Error::Custom(msg) = &err else {
        panic!("expected Error::Custom, got {err}");
    };
    assert!(msg.contains(REPEATED), "{msg}");
    // The name that appears once is not an offender.
    assert!(!msg.contains("m20240101_000002_other"), "{msg}");
    drop(conn);

    let err = Duplicated
        .up(db.clone(), None)
        .await
        .expect_err("a repeated migration name must be refused");
    assert!(
        matches!(&err, Error::Custom(msg) if msg.contains(REPEATED)),
        "{err}"
    );

    // Refused ahead of the ledger, so nothing was installed and nothing ran.
    assert!(!has_table(db, "pgorm_migrations").await?);
    assert!(!has_table(db, REPEATED).await?);

    ctx.delete().await;
    Ok(())
}

/// The advisory key is a stable function of the ledger's name: it does not move
/// between builds, and two migrators with different ledgers do not contend.
// [spec:pgorm:sem:migration.up+6/test]
#[test]
fn lock_key_is_derived_from_the_ledger_name() {
    assert_eq!(
        common::migrator::default::Migrator.lock_key(),
        4_841_830_261_500_369_544
    );
    assert_eq!(
        common::migrator::override_migration_table_name::Migrator.lock_key(),
        -6_722_091_628_467_453_339
    );
}

/// Two runners racing one fresh database both succeed: the loser waits on the
/// advisory lock, then recomputes the pending set and finds nothing to do.
// [spec:pgorm:sem:migration.up+6/test]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_runners_queue_on_the_advisory_lock() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_concurrent").await;
    let first = ctx.db.clone();
    let second = ctx.second_pool();

    let a = tokio::spawn(async move { common::migrator::default::Migrator.up(first, None).await });
    let b = tokio::spawn(async move { common::migrator::default::Migrator.up(second, None).await });

    a.await.expect("the first runner panicked")?;
    b.await.expect("the second runner panicked")?;

    // One run's worth of ledger rows, and one run's worth of seed rows.
    assert_eq!(count_rows(&ctx.db, "pgorm_migrations").await?, 5);
    assert_eq!(count_rows(&ctx.db, "cake").await?, 2);

    ctx.delete().await;
    Ok(())
}

/// A migration edited after it was applied reports a different checksum than
/// the ledger recorded, and every entry point says so.
// [spec:pgorm:req:migration.checksum+1/test]
#[tokio::test]
async fn editing_an_applied_migration_is_detected() -> Result<(), Error> {
    use common::migrator::pinned::{Checksummed, Edited, REPEATED};

    let ctx = TestContext::new("pgorm_migration_checksum_drift").await;
    let db = &ctx.db;

    Checksummed.up(db.clone(), None).await?;
    assert!(has_table(db, REPEATED).await?);

    let conn = db.get().await?;
    let err = Edited
        .status(&conn)
        .await
        .expect_err("an edited migration must be reported");
    let Error::Custom(msg) = &err else {
        panic!("expected Error::Custom, got {err}");
    };
    assert!(msg.contains(REPEATED), "{msg}");
    assert!(msg.contains("digest-one"), "{msg}");
    assert!(msg.contains("digest-two"), "{msg}");
    drop(conn);

    let err = Edited
        .up(db.clone(), None)
        .await
        .expect_err("an edited migration must be reported");
    assert!(
        matches!(&err, Error::Custom(msg) if msg.contains("digest-two")),
        "{err}"
    );

    // The unedited migration still passes.
    Checksummed.up(db.clone(), None).await?;

    ctx.delete().await;
    Ok(())
}

/// A checksum that either side is missing is unverifiable, not drift: a row
/// recorded without one, and a migration that no longer reports one, both pass.
// [spec:pgorm:req:migration.checksum+1/test]
#[tokio::test]
async fn an_unverifiable_checksum_is_not_drift() -> Result<(), Error> {
    use common::migrator::pinned::{Checksummed, Unchecked};

    let recorded_without = TestContext::new("pgorm_migration_checksum_null").await;
    Unchecked.up(recorded_without.db.clone(), None).await?;
    // Adopting a checksum afterwards cannot verify the grandfathered row.
    Checksummed.up(recorded_without.db.clone(), None).await?;
    let conn = recorded_without.db.get().await?;
    Checksummed.status(&conn).await?;
    assert_eq!(Checksummed.get_applied_migrations(&conn).await?.len(), 1);
    drop(conn);
    recorded_without.delete().await;

    let stopped_reporting = TestContext::new("pgorm_migration_checksum_dropped").await;
    Checksummed.up(stopped_reporting.db.clone(), None).await?;
    // Absence is not evidence of a change.
    Unchecked.up(stopped_reporting.db.clone(), None).await?;
    let conn = stopped_reporting.db.get().await?;
    Unchecked.status(&conn).await?;
    assert_eq!(Unchecked.get_applied_migrations(&conn).await?.len(), 1);
    drop(conn);
    stopped_reporting.delete().await;

    Ok(())
}

/// A database last migrated under the inherited `seaql_migrations` name is
/// adopted rather than re-run: the ledger answers to the new name, keeps every
/// row it had, and gains the checksum column on the way through.
// [spec:pgorm:req:migration.ledger-upgrade+2/test]    the upgrade half
// [spec:pgorm:req:migration.checksum+1/test]    an adopted ledger is widened by the same install
#[tokio::test]
async fn a_legacy_ledger_is_adopted_not_rerun() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_legacy_adopt").await;
    let db = &ctx.db;

    // Manufacture a deployment predating both the rename and the checksum
    // column: migrate part-way, then put the ledger back the way an older
    // pgorm — or SeaORM — would have left it.
    common::migrator::default::Migrator
        .up(db.clone(), Some(2))
        .await?;
    let conn = db.get().await?;
    conn.execute("ALTER TABLE \"pgorm_migrations\" DROP COLUMN checksum", &[])
        .await?;
    conn.execute(
        "ALTER TABLE \"pgorm_migrations\" RENAME TO \"seaql_migrations\"",
        &[],
    )
    .await?;
    assert!(!has_table(db, "pgorm_migrations").await?);
    assert_eq!(count_rows(db, "seaql_migrations").await?, 2);

    // A read answers from the unadopted ledger rather than calling its history
    // pending — the same answer it will give after the rename — and it renames
    // nothing to do so.
    let pending = common::migrator::default::Migrator
        .get_pending_migrations(&conn)
        .await?;
    assert_eq!(pending.len(), 3);
    assert_eq!(pending[0].name(), "m20220118_000003_seed_cake_table");

    let applied = common::migrator::default::Migrator
        .get_applied_migrations(&conn)
        .await?;
    assert_eq!(applied.len(), 2);
    assert_eq!(applied[0].name(), "m20220118_000001_create_cake_table");
    assert_eq!(applied[1].name(), "m20220118_000002_create_fruit_table");

    assert!(!has_table(db, "pgorm_migrations").await?);
    assert!(has_table(db, "seaql_migrations").await?);
    assert!(!has_column(db, "seaql_migrations", "checksum").await?);

    // `install` is what adopts, and the run picks up where the legacy ledger
    // left off. Every row survived the move, and each is grandfathered as
    // unverifiable.
    drop(conn);
    common::migrator::default::Migrator
        .up(db.clone(), None)
        .await?;

    assert!(has_table(db, "pgorm_migrations").await?);
    assert!(!has_table(db, "seaql_migrations").await?);
    assert!(has_column(db, "pgorm_migrations", "checksum").await?);
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 5);
    assert_eq!(count_rows(db, "cake").await?, 2);

    let conn = db.get().await?;
    let unverifiable: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM \"pgorm_migrations\" WHERE checksum IS NULL",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(unverifiable, 5);
    drop(conn);

    ctx.delete().await;
    Ok(())
}

/// Adoption happens once. A later run has nothing to rename, and a legacy table
/// that reappears beside an existing ledger is left where it stands rather than
/// overwriting the ledger in use.
// [spec:pgorm:req:migration.ledger-upgrade+2/test]    idempotent, and never clobbering
#[tokio::test]
async fn legacy_adoption_is_idempotent() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_legacy_idempotent").await;
    let db = &ctx.db;

    common::migrator::default::Migrator
        .up(db.clone(), None)
        .await?;
    let conn = db.get().await?;
    conn.execute(
        "ALTER TABLE \"pgorm_migrations\" RENAME TO \"seaql_migrations\"",
        &[],
    )
    .await?;

    common::migrator::default::Migrator.install(&conn).await?;
    common::migrator::default::Migrator.install(&conn).await?;
    assert!(!has_table(db, "seaql_migrations").await?);
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 5);

    // A stray table under the old name, with the ledger already in place: the
    // ledger in use wins and the stray is not read, renamed, or dropped.
    conn.execute(
        "CREATE TABLE \"seaql_migrations\" (\
         version TEXT NOT NULL PRIMARY KEY, applied_at BIGINT NOT NULL)",
        &[],
    )
    .await?;
    conn.execute(
        "INSERT INTO \"seaql_migrations\" (version, applied_at) VALUES ('stray', 0)",
        &[],
    )
    .await?;
    drop(conn);

    common::migrator::default::Migrator
        .up(db.clone(), None)
        .await?;
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 5);
    assert_eq!(count_rows(db, "seaql_migrations").await?, 1);

    ctx.delete().await;
    Ok(())
}

/// Adoption is reached from autocommit accessors as well as from `up`, so the
/// rename is guarded on the server too: racing installers all succeed and
/// exactly one of them renames the table.
// [spec:pgorm:req:migration.ledger-upgrade+2/test]    concurrent adoption
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_installers_adopt_a_legacy_ledger_once() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_legacy_concurrent").await;

    common::migrator::default::Migrator
        .up(ctx.db.clone(), Some(2))
        .await?;
    let conn = ctx.db.get().await?;
    conn.execute("ALTER TABLE \"pgorm_migrations\" DROP COLUMN checksum", &[])
        .await?;
    conn.execute(
        "ALTER TABLE \"pgorm_migrations\" RENAME TO \"seaql_migrations\"",
        &[],
    )
    .await?;
    drop(conn);

    let racers: Vec<_> = (0..4)
        .map(|_| {
            let pool = ctx.second_pool();
            tokio::spawn(async move {
                let conn = pool.get().await?;
                common::migrator::default::Migrator.install(&conn).await
            })
        })
        .collect();
    for racer in racers {
        racer.await.expect("an installer panicked")?;
    }

    assert!(has_table(&ctx.db, "pgorm_migrations").await?);
    assert!(!has_table(&ctx.db, "seaql_migrations").await?);
    assert!(has_column(&ctx.db, "pgorm_migrations", "checksum").await?);
    assert_eq!(count_rows(&ctx.db, "pgorm_migrations").await?, 2);

    ctx.delete().await;
    Ok(())
}

/// A ledger the caller named is the caller's: a legacy table sitting beside it
/// is neither adopted from nor touched.
// [spec:pgorm:req:migration.ledger-upgrade+2/test]    an override opts out of adoption
// [spec:pgorm:req:migration.read-only+1/test]    and out of being read from
#[tokio::test]
async fn a_custom_ledger_name_is_not_adopted() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_legacy_custom").await;
    let db = &ctx.db;

    let conn = db.get().await?;
    conn.execute(
        "CREATE TABLE \"seaql_migrations\" (\
         version TEXT NOT NULL PRIMARY KEY, applied_at BIGINT NOT NULL)",
        &[],
    )
    .await?;
    conn.execute(
        "INSERT INTO \"seaql_migrations\" (version, applied_at) \
         VALUES ('m20220118_000001_create_cake_table', 0)",
        &[],
    )
    .await?;
    // A read is gated the same way the rename is: the legacy table is not this
    // migrator's ledger, so it is not read from either.
    assert!(
        common::migrator::override_migration_table_name::Migrator
            .get_applied_migrations(&conn)
            .await?
            .is_empty()
    );
    assert!(!has_table(db, "override_migration_table_name").await?);
    drop(conn);

    common::migrator::override_migration_table_name::Migrator
        .up(db.clone(), None)
        .await?;

    // The custom ledger was built from nothing, so all five ran.
    assert_eq!(count_rows(db, "override_migration_table_name").await?, 5);
    assert!(!has_table(db, "pgorm_migrations").await?);
    // The legacy table is still exactly as it was found.
    assert_eq!(count_rows(db, "seaql_migrations").await?, 1);
    assert!(!has_column(db, "seaql_migrations", "checksum").await?);

    ctx.delete().await;
    Ok(())
}

/// A ledger deployed before the checksum column existed is widened in place,
/// keeping the rows it already had.
// [spec:pgorm:req:migration.checksum+1/test]
#[tokio::test]
async fn install_widens_a_pre_checksum_ledger() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_checksum_widen").await;
    let db = &ctx.db;

    let conn = db.get().await?;
    conn.execute(
        "CREATE TABLE \"pgorm_migrations\" (\
         version TEXT NOT NULL PRIMARY KEY, applied_at BIGINT NOT NULL)",
        &[],
    )
    .await?;
    conn.execute(
        "INSERT INTO \"pgorm_migrations\" (version, applied_at) \
         VALUES ('m20220118_000001_create_cake_table', 0)",
        &[],
    )
    .await?;
    assert!(!has_column(db, "pgorm_migrations", "checksum").await?);

    common::migrator::default::Migrator.install(&conn).await?;
    assert!(has_column(db, "pgorm_migrations", "checksum").await?);

    let applied = common::migrator::default::Migrator
        .get_applied_migrations(&conn)
        .await?;
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].name(), "m20220118_000001_create_cake_table");
    assert_eq!(
        common::migrator::default::Migrator
            .get_pending_migrations(&conn)
            .await?
            .len(),
        4
    );

    drop(conn);
    ctx.delete().await;
    Ok(())
}

/// A read-named method is a read. Against a database with no ledger at all
/// every accessor answers "nothing applied", and none of them leaves a table
/// behind — so a readiness probe or a CI gate needs no DDL privilege to ask.
/// After `up` has done the provisioning the same calls agree with it.
// [spec:pgorm:req:migration.read-only+1/test]    the regression: a read that provisioned
#[tokio::test]
async fn reading_status_provisions_nothing() -> Result<(), Error> {
    let ctx = TestContext::new("pgorm_migration_read_only").await;
    let db = &ctx.db;
    let conn = db.get().await?;

    common::migrator::default::Migrator.status(&conn).await?;
    assert!(
        common::migrator::default::Migrator
            .get_applied_migrations(&conn)
            .await?
            .is_empty()
    );
    let pending = common::migrator::default::Migrator
        .get_pending_migrations(&conn)
        .await?;
    assert_eq!(pending.len(), 5);
    assert!(
        pending
            .iter()
            .all(|file| file.status() == MigrationStatus::Pending)
    );

    // The whole point: nothing was created to answer with.
    assert!(!has_table(db, "pgorm_migrations").await?);
    assert!(!has_table(db, "seaql_migrations").await?);
    drop(conn);

    common::migrator::default::Migrator
        .up(db.clone(), None)
        .await?;

    let conn = db.get().await?;
    common::migrator::default::Migrator.status(&conn).await?;
    assert_eq!(
        common::migrator::default::Migrator
            .get_applied_migrations(&conn)
            .await?
            .len(),
        5
    );
    assert!(
        common::migrator::default::Migrator
            .get_pending_migrations(&conn)
            .await?
            .is_empty()
    );
    drop(conn);

    ctx.delete().await;
    Ok(())
}

/// A ledger deployed before the checksum column is read as it stands rather
/// than widened to suit the reader. The stored digest is absent, so the
/// three-valued check grandfathers the row exactly as it would once the column
/// existed — the answer does not depend on whether the widening has happened.
// [spec:pgorm:req:migration.read-only+1/test]    no ALTER on the way to an answer
// [spec:pgorm:req:migration.checksum+1/test]    the read path's NULL stand-in
#[tokio::test]
async fn a_read_never_widens_a_pre_checksum_ledger() -> Result<(), Error> {
    use common::migrator::pinned::{Checksummed, REPEATED};

    let ctx = TestContext::new("pgorm_migration_read_pre_checksum").await;
    let db = &ctx.db;

    let conn = db.get().await?;
    conn.execute(
        "CREATE TABLE \"pgorm_migrations\" (\
         version TEXT NOT NULL PRIMARY KEY, applied_at BIGINT NOT NULL)",
        &[],
    )
    .await?;
    conn.execute(
        "INSERT INTO \"pgorm_migrations\" (version, applied_at) VALUES ($1, 0)",
        &[&REPEATED],
    )
    .await?;
    assert!(!has_column(db, "pgorm_migrations", "checksum").await?);

    // The migrator reports a digest; the ledger cannot have recorded one.
    Checksummed.status(&conn).await?;
    let applied = Checksummed.get_applied_migrations(&conn).await?;
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].name(), REPEATED);
    assert!(Checksummed.get_pending_migrations(&conn).await?.is_empty());

    assert!(!has_column(db, "pgorm_migrations", "checksum").await?);
    drop(conn);

    ctx.delete().await;
    Ok(())
}

/// A batch that changes a domain's default reaches a connection that already
/// prepared an insert through it: once the batch commits, the runner has the
/// pool clear its statements, closing the insert, which is prepared again and
/// writes the new default. A run that applies nothing leaves the insert
/// prepared.
// [spec:pgorm:sem:migration.up+6/test]    a committed batch that applied a migration clears the
// pool's statement caches, and a run that applied nothing does not
// [spec:pgorm:req:conn.pool.statement-cache.clear+1/test]    the runner clears after the DDL it applied
#[tokio::test]
async fn up_clears_the_pools_prepared_statements() -> Result<(), Error> {
    use common::migrator::domain_default::{Created, Defaulted, INSERT, prepared_insert};

    const BACKEND: &str = "SELECT pg_backend_pid()";

    let ctx = TestContext::new("pgorm_migration_clears_statements").await;
    let db = &ctx.db;

    Created.up(db.clone(), None).await?;
    let conn = db.get().await?;
    let backend: i32 = conn.query_one(BACKEND, &[]).await?.get(0);
    assert_eq!(conn.query_one(INSERT, &[&1]).await?.get::<_, i32>(0), 1);
    let prepared = prepared_insert(&conn).await?;
    assert!(prepared.is_some());
    drop(conn);

    Created.up(db.clone(), None).await?;
    let conn = db.get().await?;
    assert_eq!(
        conn.query_one(BACKEND, &[]).await?.get::<_, i32>(0),
        backend
    );
    assert_eq!(prepared_insert(&conn).await?, prepared);
    drop(conn);

    Defaulted.up(db.clone(), None).await?;
    let conn = db.get().await?;
    assert_eq!(
        conn.query_one(BACKEND, &[]).await?.get::<_, i32>(0),
        backend
    );
    assert_eq!(prepared_insert(&conn).await?, None);
    assert_eq!(conn.query_one(INSERT, &[&2]).await?.get::<_, i32>(0), 2);
    let prepared_again = prepared_insert(&conn).await?;
    assert!(prepared_again.is_some() && prepared_again != prepared);
    drop(conn);

    ctx.delete().await;
    Ok(())
}

/// Another process's pool, listening on the channel the runner notifies by
/// default, hears a batch that applied a migration and clears: its insert,
/// prepared before the batch changed the domain's default, is closed, and
/// prepared again writes the new one. A run that applies nothing sends
/// nothing, nor does a migrator that names no channel, which still clears the
/// pool it was given, as the fence each is followed by shows: notifications
/// reach a session in commit order.
// [spec:pgorm:sem:migration.up+6/test]    a batch that applied a migration notifies the channel as
// it commits, and a run that applied nothing, or a migrator naming no channel, does not
// [spec:pgorm:req:conn.pool.statement-cache.listen+1/test]    another pool listening on the default
// channel clears once the runner's batch commits
#[test]
fn up_notifies_pools_listening_elsewhere() -> Result<(), Error> {
    use common::migrator::domain_default::{
        Created, Defaulted, INSERT, Unannounced, prepared_insert,
    };
    use futures::StreamExt;
    use pgorm_migration::pgorm::{Listener, Notifications, STATEMENT_CACHE_CHANNEL, notify};
    use pgorm_test_support::ListenerClears;
    use std::time::Duration;

    const BACKEND: &str = "SELECT pg_backend_pid()";
    const ARRIVAL: Duration = Duration::from_secs(10);

    async fn payload(notifications: &mut Notifications) -> Result<String, Error> {
        let notification = tokio::time::timeout(ARRIVAL, notifications.next())
            .await
            .expect("a notification arrives in time")
            .expect("the stream has not ended")?;
        assert_eq!(notification.channel(), STATEMENT_CACHE_CHANNEL);
        Ok(notification.payload().to_owned())
    }

    let clears = ListenerClears::default();
    clears.run(async {
        let ctx = TestContext::new("pgorm_migration_notifies_listening_pools").await;
        let db = &ctx.db;
        let (listener, mut notifications) = db.listener(Listener::DEFAULT_CAPACITY).await?;
        listener.listen(STATEMENT_CACHE_CHANNEL).await?;

        Created.up(db.clone(), None).await?;
        assert_eq!(payload(&mut notifications).await?, "");

        // Opened after `Created`'s notification, so only `Defaulted`'s can
        // clear it once its `LISTEN` has.
        let elsewhere = ctx.listening_pool();
        drop(elsewhere.get().await?);
        clears.reach(1).await;
        let conn = elsewhere.get().await?;
        assert_eq!(conn.query_one(INSERT, &[&1]).await?.get::<_, i32>(0), 1);
        let prepared = prepared_insert(&conn).await?;
        assert!(prepared.is_some());
        drop(conn);
        let conn = elsewhere.get().await?;
        assert_eq!(prepared_insert(&conn).await?, prepared);
        drop(conn);

        Defaulted.up(db.clone(), None).await?;
        assert_eq!(payload(&mut notifications).await?, "");
        clears.reach(2).await;
        let conn = elsewhere.get().await?;
        assert_eq!(prepared_insert(&conn).await?, None);
        assert_eq!(conn.query_one(INSERT, &[&2]).await?.get::<_, i32>(0), 2);
        let prepared_again = prepared_insert(&conn).await?;
        assert!(prepared_again.is_some() && prepared_again != prepared);
        drop(conn);

        let conn = db.get().await?;
        let backend: i32 = conn.query_one(BACKEND, &[]).await?.get(0);
        assert_eq!(conn.query_one(INSERT, &[&3]).await?.get::<_, i32>(0), 2);
        assert!(prepared_insert(&conn).await?.is_some());
        drop(conn);
        Defaulted.up(db.clone(), None).await?;
        Unannounced.up(db.clone(), None).await?;
        let conn = db.get().await?;
        assert_eq!(
            conn.query_one(BACKEND, &[]).await?.get::<_, i32>(0),
            backend
        );
        assert_eq!(
            prepared_insert(&conn).await?,
            None,
            "the runner still clears its own pool"
        );
        assert_eq!(conn.query_one(INSERT, &[&4]).await?.get::<_, i32>(0), 3);
        drop(conn);
        notify(&db.get().await?, STATEMENT_CACHE_CHANNEL, "fence").await?;
        assert_eq!(payload(&mut notifications).await?, "fence");

        drop((listener, notifications, elsewhere));
        ctx.delete().await;
        Ok(())
    })
}

/// A ledger given a schema starts from a database holding neither: reads leave
/// it that way, `up` creates the schema and then the ledger inside it, and a
/// second run reads that ledger back and applies nothing.
// [spec:pgorm:def:migration.runner+5/test]    the schema is created, then the ledger in it
// [spec:pgorm:req:migration.read-only+1/test]    a read creates neither the schema nor the table
#[tokio::test]
async fn a_ledger_schema_is_created_from_nothing() -> Result<(), Error> {
    use common::migrator::default::Migrator;
    use common::migrator::ledger_schema::{LEDGER_SCHEMA, Separate};

    let ctx = TestContext::new("pgorm_migration_schema_fresh").await;
    let db = &ctx.db;

    let conn = db.get().await?;
    Separate.status(&conn).await?;
    assert_eq!(Separate.get_pending_migrations(&conn).await?.len(), 5);
    drop(conn);
    assert!(!has_schema(db, LEDGER_SCHEMA).await?);

    Separate.up(db.clone(), None).await?;
    assert!(has_schema(db, LEDGER_SCHEMA).await?);
    assert_eq!(
        count_rows_in(db, LEDGER_SCHEMA, "pgorm_migrations").await?,
        5
    );
    assert!(has_column_in(db, LEDGER_SCHEMA, "pgorm_migrations", "checksum").await?);
    assert!(!has_table(db, "pgorm_migrations").await?);
    assert!(has_table(db, "cake").await?);

    Separate.up(db.clone(), None).await?;
    assert_eq!(
        count_rows_in(db, LEDGER_SCHEMA, "pgorm_migrations").await?,
        5
    );
    assert_eq!(count_rows(db, "cake").await?, 2);

    let conn = db.get().await?;
    Separate.status(&conn).await?;
    let applied = Separate.get_applied_migrations(&conn).await?;
    assert_eq!(applied.len(), 5);
    assert_eq!(applied[0].name(), "m20220118_000001_create_cake_table");
    assert!(Separate.get_pending_migrations(&conn).await?.is_empty());
    // The same migrations with an unqualified ledger find none on search_path.
    assert_eq!(Migrator.get_pending_migrations(&conn).await?.len(), 5);
    drop(conn);

    assert_eq!(Separate.lock_key(), Migrator.lock_key());

    ctx.delete().await;
    Ok(())
}

/// A schema made ahead of time for the migrating role is used as it stands.
/// The role owns the schema but holds no `CREATE` on the database, which
/// `CREATE SCHEMA IF NOT EXISTS` demands even of a schema that exists, so the
/// run succeeds only because the catalogue is read before it.
// [spec:pgorm:def:migration.runner+5/test]    an existing schema is not created again
#[tokio::test]
async fn an_existing_ledger_schema_is_used_as_found() -> Result<(), Error> {
    use common::migrator::ledger_schema::{LEDGER_SCHEMA, Separate};

    let ctx = TestContext::new("pgorm_migration_schema_existing").await;
    let role = ctx.create_role("pgorm_migration_schema_owner").await;

    let conn = ctx.db.get().await?;
    conn.batch_execute(&format!(
        "CREATE SCHEMA \"{LEDGER_SCHEMA}\" AUTHORIZATION \"{role}\"; \
         GRANT CREATE, USAGE ON SCHEMA public TO \"{role}\""
    ))
    .await?;
    let can_create: bool = conn
        .query_one(
            "SELECT has_database_privilege($1, current_database(), 'CREATE')",
            &[&role],
        )
        .await?
        .get(0);
    assert!(!can_create);
    drop(conn);

    let owner = ctx.pool_as(&role);
    Separate.up(owner.clone(), None).await?;
    Separate.up(owner.clone(), None).await?;
    drop(owner);

    assert_eq!(
        count_rows_in(&ctx.db, LEDGER_SCHEMA, "pgorm_migrations").await?,
        5
    );
    assert_eq!(count_rows(&ctx.db, "cake").await?, 2);

    ctx.delete().await;
    ctx.drop_role(&role).await;
    Ok(())
}

/// Tables named like the ledger in `public` belong to the application. A
/// ledger in its own schema is located, inspected for its checksum column, read
/// and widened there alone: a public `pgorm_migrations` that has the column
/// cannot stand in for a ledger in the schema that lacks it, and a public
/// `seaql_migrations` is neither read nor adopted.
// [spec:pgorm:def:migration.runner+5/test]    every statement names the qualified table
// [spec:pgorm:req:migration.read-only+1/test]    located and inspected in its own schema
// [spec:pgorm:req:migration.ledger-upgrade+2/test]    a schema opts out of adoption
#[tokio::test]
async fn a_public_table_of_that_name_is_ignored() -> Result<(), Error> {
    use common::migrator::ledger_schema::{LEDGER_SCHEMA, Separate};

    let ctx = TestContext::new("pgorm_migration_schema_shadowed").await;
    let db = &ctx.db;

    let conn = db.get().await?;
    conn.batch_execute(&format!(
        "CREATE TABLE pgorm_migrations (\
         version TEXT NOT NULL PRIMARY KEY, applied_at BIGINT NOT NULL, checksum TEXT); \
         INSERT INTO pgorm_migrations VALUES ('m20220118_000001_create_cake_table', 0, 'app'); \
         CREATE TABLE seaql_migrations (version TEXT NOT NULL PRIMARY KEY, applied_at BIGINT NOT NULL); \
         INSERT INTO seaql_migrations VALUES ('m20220118_000002_create_fruit_table', 0); \
         CREATE SCHEMA \"{LEDGER_SCHEMA}\"; \
         CREATE TABLE \"{LEDGER_SCHEMA}\".pgorm_migrations (\
         version TEXT NOT NULL PRIMARY KEY, applied_at BIGINT NOT NULL)"
    ))
    .await?;

    // Neither public table's rows answer for the ledger, and the read names no
    // checksum column the ledger in the schema does not have.
    Separate.status(&conn).await?;
    assert!(Separate.get_applied_migrations(&conn).await?.is_empty());
    assert_eq!(Separate.get_pending_migrations(&conn).await?.len(), 5);
    assert!(!has_column_in(db, LEDGER_SCHEMA, "pgorm_migrations", "checksum").await?);
    drop(conn);

    Separate.up(db.clone(), None).await?;

    assert!(has_column_in(db, LEDGER_SCHEMA, "pgorm_migrations", "checksum").await?);
    assert_eq!(
        count_rows_in(db, LEDGER_SCHEMA, "pgorm_migrations").await?,
        5
    );
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 1);
    assert_eq!(count_rows(db, "seaql_migrations").await?, 1);
    assert!(!has_column(db, "seaql_migrations", "checksum").await?);

    ctx.delete().await;
    Ok(())
}

/// A `.sql` file is named after its file stem and reports the SHA-256 digest
/// of its text, with no database involved.
// [spec:pgorm:def:migration.sql-file+1/test]    the name is the file stem
// [spec:pgorm:req:migration.sql-file.checksum/test]    a known digest
#[test]
fn a_sql_file_is_named_and_digested() {
    use common::migrator::sql_files::{CREATE_NOTE_DIGEST, EDITED_NOTE_DIGEST};

    let migration = sql_migration!("tests/sql/m20260101_000001_create_note.sql");
    assert_eq!(migration.name(), "m20260101_000001_create_note");
    assert_eq!(migration.checksum().as_deref(), Some(CREATE_NOTE_DIGEST));
    assert!(migration.sql().contains("CREATE TABLE note ("));

    let edited = sql_migration!("tests/sql/edited/m20260101_000001_create_note.sql");
    assert_eq!(edited.name(), migration.name());
    assert_eq!(edited.checksum().as_deref(), Some(EDITED_NOTE_DIGEST));
}

/// SQL-file migrations run through the migrator beside one written in Rust,
/// each recorded under its file stem with its digest. Editing an applied file
/// is then reported by `status` and by `up`, which applies nothing.
// [spec:pgorm:def:migration.sql-file+1/test]    run by the migrator, in order, beside a Rust migration
// [spec:pgorm:req:migration.sql-file.checksum/test]    an edited file is caught by the checksum check
#[tokio::test]
async fn sql_file_migrations_run_and_detect_edits() -> Result<(), Error> {
    use common::migrator::sql_files::{CREATE_NOTE_DIGEST, EDITED_NOTE_DIGEST, Edited, Written};

    let ctx = TestContext::new("pgorm_migration_sql_files").await;
    let db = &ctx.db;

    Written.up(db.clone(), None).await?;
    assert!(has_table(db, "cake").await?);
    assert_eq!(count_rows(db, "note").await?, 2);

    let conn = db.get().await?;
    let recorded: Vec<(String, Option<String>)> = conn
        .query_all(
            "SELECT version, checksum FROM pgorm_migrations ORDER BY version",
            &[],
        )
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        recorded,
        vec![
            ("m20220118_000001_create_cake_table".to_owned(), None),
            (
                "m20260101_000001_create_note".to_owned(),
                Some(CREATE_NOTE_DIGEST.to_owned())
            ),
            (
                "m20260101_000002_seed_note".to_owned(),
                Some("8cf25c961bdc64bf30696cdc1ae9bc2618744207d4427d0ef7bab318e80aef80".to_owned())
            ),
        ]
    );

    let err = Edited
        .status(&conn)
        .await
        .expect_err("an edited SQL file must be reported");
    let Error::Custom(msg) = &err else {
        panic!("expected Error::Custom, got {err}");
    };
    assert!(msg.contains("m20260101_000001_create_note"), "{msg}");
    assert!(msg.contains(CREATE_NOTE_DIGEST), "{msg}");
    assert!(msg.contains(EDITED_NOTE_DIGEST), "{msg}");
    drop(conn);

    let err = Edited
        .up(db.clone(), None)
        .await
        .expect_err("an edited SQL file must be reported");
    assert!(
        matches!(&err, Error::Custom(msg) if msg.contains(EDITED_NOTE_DIGEST)),
        "{err}"
    );
    assert!(!has_column(db, "note", "pinned").await?);

    Written.up(db.clone(), None).await?;

    ctx.delete().await;
    Ok(())
}

fn sqlstate(err: &Error) -> Option<String> {
    match err {
        Error::Postgres(error) => error.as_db_error().map(|db| db.code().code().to_owned()),
        _ => None,
    }
}

async fn moods(db: &pgorm_migration::pgorm::DatabasePool) -> Result<Vec<String>, Error> {
    let conn = db.get().await?;
    Ok(conn
        .query_one("SELECT enum_range(NULL::mood)::text[]", &[])
        .await?
        .get(0))
}

/// A value added to an enum created earlier can be used once the transaction
/// adding it commits. Asked to by the migration adding it, the batch commits
/// there and the next migration uses the value; asked by none, the batch is
/// one transaction, in which the use is refused (`55P04`) and nothing is
/// applied.
// [spec:pgorm:req:migration.commit-after/test]    the batch commits after the migration asking, and a
// batch with none is one transaction
#[tokio::test]
async fn a_batch_commits_after_a_migration_that_asks() -> Result<(), Error> {
    use common::migrator::commit_after::{MoodCreated, Moods, Unsplit};

    let ctx = TestContext::new("pgorm_migration_commit_after").await;
    let db = &ctx.db;

    MoodCreated.up(db.clone(), None).await?;
    let err = Unsplit
        .up(db.clone(), None)
        .await
        .expect_err("a value used in the transaction adding it is refused");
    assert_eq!(sqlstate(&err).as_deref(), Some("55P04"), "{err}");
    assert_eq!(moods(db).await?, ["happy", "sad"]);
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 1);

    Moods.up(db.clone(), None).await?;
    assert_eq!(moods(db).await?, ["happy", "meh", "sad"]);
    assert_eq!(count_rows(db, "feeling").await?, 1);
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 3);
    assert!(
        Moods
            .get_pending_migrations(&db.get().await?)
            .await?
            .is_empty()
    );

    ctx.delete().await;
    Ok(())
}

/// A failure after the commit rolls back only the transaction it happens in:
/// the migrations committed before it stay applied with their ledger rows, and
/// the next run resumes after them.
// [spec:pgorm:req:migration.commit-after/test]    a failure after a commit, then a run that resumes
#[tokio::test]
async fn a_failure_after_a_commit_keeps_what_committed() -> Result<(), Error> {
    use common::migrator::commit_after::{FailingAfterCommit, Moods};

    let ctx = TestContext::new("pgorm_migration_commit_after_failure").await;
    let db = &ctx.db;

    let err = FailingAfterCommit
        .up(db.clone(), None)
        .await
        .expect_err("the last migration writes a value the enum lacks");
    assert_eq!(sqlstate(&err).as_deref(), Some("22P02"), "{err}");
    assert_eq!(moods(db).await?, ["happy", "meh", "sad"]);
    assert_eq!(count_rows(db, "feeling").await?, 0);
    let applied: Vec<String> = FailingAfterCommit
        .get_applied_migrations(&db.get().await?)
        .await?
        .iter()
        .map(|migration| migration.name().to_owned())
        .collect();
    assert_eq!(
        applied,
        ["m20240301_000001_create_mood", "m20240301_000002_add_meh"]
    );

    Moods.up(db.clone(), None).await?;
    assert_eq!(moods(db).await?, ["happy", "meh", "sad"]);
    assert_eq!(count_rows(db, "feeling").await?, 1);
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 3);

    ctx.delete().await;
    Ok(())
}

/// Two runners queued behind a held lock both finish, and each migration is
/// applied once: whichever transaction holds the lock next recomputes the
/// pending set against everything committed before it, so neither the value
/// nor the row is added twice, which would be refused (`42710`, `23505`).
// [spec:pgorm:req:migration.commit-after/test]    a runner queued behind the first applies nothing twice
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runners_queued_behind_a_commit_apply_nothing_twice() -> Result<(), Error> {
    use common::migrator::commit_after::Moods;
    use pgorm_migration::pgorm::TransactionTrait;
    use std::time::{Duration, Instant};

    const WAITING: &str = "SELECT count(*) FROM pg_locks l JOIN pg_database d ON d.oid = l.database \
                           WHERE l.locktype = 'advisory' AND NOT l.granted \
                           AND d.datname = current_database()";

    let ctx = TestContext::new("pgorm_migration_commit_after_queued").await;
    let held = ctx.second_pool();
    let watcher = ctx.second_pool();

    let mut holder = held.get().await?;
    let (first, second) = holder
        .transaction(async |tx| {
            tx.execute("SELECT pg_advisory_xact_lock($1)", &[&Moods.lock_key()])
                .await?;
            let first = ctx.db.clone();
            let second = ctx.second_pool();
            let runners = (
                tokio::spawn(async move { Moods.up(first, None).await }),
                tokio::spawn(async move { Moods.up(second, None).await }),
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let waiting: i64 = watcher.get().await?.query_one(WAITING, &[]).await?.get(0);
                if waiting == 2 {
                    break;
                }
                assert!(Instant::now() < deadline, "both runners queue on the lock");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok::<_, Error>(runners)
        })
        .await
        .map_err(|err| Error::Custom(err.to_string()))?;
    drop(holder);
    first.await.expect("the first runner panicked")?;
    second.await.expect("the second runner panicked")?;

    assert_eq!(moods(&ctx.db).await?, ["happy", "meh", "sad"]);
    assert_eq!(count_rows(&ctx.db, "feeling").await?, 1);
    assert_eq!(count_rows(&ctx.db, "pgorm_migrations").await?, 3);

    ctx.delete().await;
    Ok(())
}

/// Each commit of a batch clears the pool's statement caches and notifies the
/// channel, inside the transaction it ends. Through a pool of one connection,
/// the insert prepared before the batch is closed by the clear after the
/// migration changing the domain's default commits, so the next migration,
/// which writes through it on the same session, writes the new default; and
/// the channel hears once per commit, where a batch of two migrations with no
/// commit inside it is heard once.
// [spec:pgorm:req:migration.commit-after/test]    every commit clears and notifies
// [spec:pgorm:sem:migration.up+6/test]    each transaction of a split batch takes its connection from the
// pool, after the clear
#[tokio::test]
async fn every_commit_clears_and_notifies() -> Result<(), Error> {
    use common::migrator::commit_after::Redefaulted;
    use common::migrator::domain_default::{Defaulted, INSERT, prepared_insert};
    use futures::StreamExt;
    use pgorm_migration::pgorm::{Listener, Notifications, STATEMENT_CACHE_CHANNEL, notify};
    use std::time::Duration;

    async fn payload(notifications: &mut Notifications) -> Result<String, Error> {
        let notification = tokio::time::timeout(Duration::from_secs(10), notifications.next())
            .await
            .expect("a notification arrives in time")
            .expect("the stream has not ended")?;
        Ok(notification.payload().to_owned())
    }

    let ctx = TestContext::new("pgorm_migration_commit_after_clears").await;
    let db = ctx.pool_of_one();
    let (listener, mut notifications) = db.listener(Listener::DEFAULT_CAPACITY).await?;
    listener.listen(STATEMENT_CACHE_CHANNEL).await?;

    Defaulted.up(db.clone(), None).await?;
    assert_eq!(payload(&mut notifications).await?, "");

    let conn = db.get().await?;
    assert_eq!(conn.query_one(INSERT, &[&1]).await?.get::<_, i32>(0), 2);
    assert!(prepared_insert(&conn).await?.is_some());
    drop(conn);

    Redefaulted.up(db.clone(), None).await?;
    assert_eq!(payload(&mut notifications).await?, "");
    assert_eq!(payload(&mut notifications).await?, "");
    let conn = db.get().await?;
    let written: i32 = conn
        .query_one("SELECT qty FROM line WHERE id = 10", &[])
        .await?
        .get(0);
    assert_eq!(written, 3);
    notify(&conn, STATEMENT_CACHE_CHANNEL, "fence").await?;
    assert_eq!(payload(&mut notifications).await?, "fence");
    drop(conn);

    drop((listener, notifications));
    ctx.delete().await;
    Ok(())
}

/// A kind with a table added to a disjoint entity through the runner: the two
/// steps `add_disjoint_kind` writes run as two migrations of one batch, the
/// first answering `commits_after()`, so the second uses the value the first
/// added. The entity then writes and reads the new kind beside the old.
// [spec:pgorm:sem:disjoint.evolve+2/test]    each step one migration, every step but the last
// answering `commits_after()`
// [spec:pgorm:req:migration.commit-after/test]    a batch running the steps of a change
#[tokio::test]
async fn a_disjoint_kind_is_added_through_the_runner() -> Result<(), Error> {
    use common::migrator::disjoint::{
        Created, Fleet, FleetActiveModel, Vanned, VannedFleet, VannedFleetActiveModel, car, fleet,
        van, vanned_fleet,
    };
    use pgorm::{Disjoint, QueryOrder, set};

    let ctx = TestContext::new("pgorm_migration_disjoint_kind").await;
    let db = &ctx.db;

    Created.up(db.clone(), None).await?;
    let conn = db.get().await?;
    let (sedan, _) = Fleet::insert(
        fleet::ActiveModel {
            name: set("sedan"),
            ..Default::default()
        },
        FleetActiveModel::Car(car::ActiveModel {
            seats: set(5_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&conn)
    .await?;
    drop(conn);

    Vanned.up(db.clone(), None).await?;
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 3);
    let conn = db.get().await?;
    let (camper, _) = VannedFleet::insert(
        vanned_fleet::ActiveModel {
            name: set("camper"),
            ..Default::default()
        },
        VannedFleetActiveModel::Van(van::ActiveModel {
            doors: set(4_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&conn)
    .await?;
    let read: Vec<VannedFleet> = VannedFleet::find()
        .order_by_asc(vanned_fleet::Column::Id)
        .all(&conn)
        .await?
        .into_iter()
        .map(|(_, value)| value)
        .collect();
    assert_eq!(
        read,
        [
            VannedFleet::Car(car::Model {
                id: sedan.id,
                seats: 5
            }),
            VannedFleet::Van(van::Model {
                id: camper.id,
                doors: 4
            }),
        ]
    );
    drop(conn);

    ctx.delete().await;
    Ok(())
}

/// A change frozen when it is made: the tables a declaration creates in one
/// file, a kind added to it in a file per step, the first committing after
/// it runs, each the text the committed fixtures hold. A file already there
/// is never written again, so freezing the tables from the declaration as it
/// stands later leaves the history as it was.
// [spec:pgorm:def:migration.frozen/test]
// [spec:pgorm:req:migration.sql-file.commits-after/test]
// [spec:pgorm:sem:disjoint.evolve.frozen/test]
#[test]
fn a_change_freezes_into_its_migration_files() -> Result<(), Box<dyn std::error::Error>> {
    use common::migrator::disjoint::{Fleet, VannedFleet, VannedFleetKind};
    use pgorm::{MigrationFile, Schema};

    let created = MigrationFile::new(
        "m20240401_000001_create_fleet",
        Schema::new().create_disjoint::<Fleet>()?,
    );
    let added = Schema::new()
        .add_disjoint_kind::<VannedFleet>(VannedFleetKind::Van)?
        .migration_files("m20240401_000002_add_van");
    let names: Vec<&str> = added.iter().map(MigrationFile::name).collect();
    assert_eq!(
        names,
        ["m20240401_000002_add_van_1", "m20240401_000002_add_van_2"]
    );
    assert_eq!(
        created.text(),
        include_str!("sql/frozen/m20240401_000001_create_fleet.sql")
    );
    assert_eq!(
        added[0].text(),
        include_str!("sql/frozen/m20240401_000002_add_van_1.sql")
    );
    assert_eq!(
        added[1].text(),
        include_str!("sql/frozen/m20240401_000002_add_van_2.sql")
    );
    assert!(added[0].commits_after() && !added[1].commits_after());
    assert!(sql_migration!("tests/sql/frozen/m20240401_000002_add_van_1.sql").commits_after());
    assert!(!sql_migration!("tests/sql/frozen/m20240401_000002_add_van_2.sql").commits_after());

    let dir = std::env::temp_dir().join(pgorm_test_support::scoped_name("pgorm_migration_frozen"));
    std::fs::create_dir_all(&dir)?;
    assert!(created.freeze(&dir)?);
    assert!(!created.freeze(&dir)?);
    let later = MigrationFile::new(
        "m20240401_000001_create_fleet",
        Schema::new().create_disjoint::<VannedFleet>()?,
    );
    assert_ne!(later.text(), created.text());
    assert!(!later.freeze(&dir)?);
    let path = dir.join("m20240401_000001_create_fleet.sql");
    assert_eq!(std::fs::read_to_string(&path)?, created.text());

    for name in ["", "..", "nested/file", "back\\slash"] {
        let refused = MigrationFile::new(name, ["SELECT 1"]).freeze(&dir);
        assert_eq!(
            refused.map_err(|error| error.kind()),
            Err(std::io::ErrorKind::InvalidInput),
            "{name}"
        );
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

/// The frozen history replays on a database built from nothing: the tables
/// as they were first declared, then the van kind, its value committed before
/// its table uses it, after which the entity as it is declared now writes a
/// van. The same history generated as each migration runs cannot: its first
/// migration creates the kind its second adds.
// [spec:pgorm:sem:disjoint.evolve.frozen/test]    a fresh database replays the frozen history
// [spec:pgorm:req:migration.sql-file.commits-after/test]    the directive commits the batch
#[tokio::test]
async fn a_frozen_history_replays_on_a_fresh_database() -> Result<(), Error> {
    use common::migrator::disjoint::{
        Frozen, Recomputed, VannedFleet, VannedFleetActiveModel, van, vanned_fleet,
    };
    use pgorm::{Disjoint, set};

    let ctx = TestContext::new("pgorm_migration_frozen_replay").await;
    let db = &ctx.db;
    Frozen.up(db.clone(), None).await?;
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 3);
    let conn = db.get().await?;
    let (camper, _) = VannedFleet::insert(
        vanned_fleet::ActiveModel {
            name: set("camper"),
            ..Default::default()
        },
        VannedFleetActiveModel::Van(van::ActiveModel {
            doors: set(4_i16),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&conn)
    .await?;
    assert_eq!(camper.name, "camper");
    drop(conn);
    Frozen.up(db.clone(), None).await?;
    assert_eq!(count_rows(db, "pgorm_migrations").await?, 3);
    ctx.delete().await;

    let ctx = TestContext::new("pgorm_migration_recomputed_replay").await;
    let replayed = Recomputed.up(ctx.db.clone(), None).await;
    assert!(
        replayed
            .as_ref()
            .is_err_and(|error| error.to_string().contains("already exists")),
        "{replayed:?}"
    );
    ctx.delete().await;
    Ok(())
}

/// A `.sql` file read as the program runs is the migration `sql_migration!`
/// makes of it: its name, its text with CRLF read as LF, its checksum and its
/// directive. A directory reads its `.sql` files in file-name order, passing
/// over everything else.
// [spec:pgorm:req:migration.sql-file.read/test]
#[test]
fn runtime_sql_file_matches_the_macro() -> std::io::Result<()> {
    use common::migrator::sql_files::CREATE_NOTE_DIGEST;

    let manifest = env!("CARGO_MANIFEST_DIR");
    let compiled = sql_migration!("tests/sql/frozen/m20240401_000002_add_van_1.sql");
    let read = SqlMigration::read(format!(
        "{manifest}/tests/sql/frozen/m20240401_000002_add_van_1.sql"
    ))?;
    assert_eq!(read, compiled);
    assert_eq!(read.checksum(), compiled.checksum());
    assert!(read.commits_after());

    let names: Vec<String> = SqlMigration::read_dir(format!("{manifest}/tests/sql/frozen"))?
        .iter()
        .map(|migration| migration.name().to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "m20240401_000001_create_fleet",
            "m20240401_000002_add_van_1",
            "m20240401_000002_add_van_2"
        ]
    );

    let dir = std::env::temp_dir().join(format!("pgorm_migration_read_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("nested.sql"))?;
    let lf = sql_migration!("tests/sql/m20260101_000001_create_note.sql");
    std::fs::write(
        dir.join("m20260101_000001_create_note.sql"),
        lf.sql().replace('\n', "\r\n"),
    )?;
    std::fs::write(dir.join("notes.txt"), "not a migration")?;
    let crlf = SqlMigration::read_dir(&dir)?;
    assert_eq!(crlf, [lf]);
    assert_eq!(crlf[0].checksum().as_deref(), Some(CREATE_NOTE_DIGEST));

    std::fs::write(dir.join("m20260101_000002_binary.sql"), [0xff, 0xfe])?;
    let refused = SqlMigration::read_dir(&dir).map(|_| ());
    assert_eq!(
        refused.map_err(|error| error.kind()),
        Err(std::io::ErrorKind::InvalidData)
    );
    std::fs::remove_dir_all(&dir)?;

    let refused = SqlMigration::read("/").map(|_| ());
    assert_eq!(
        refused.map_err(|error| error.kind()),
        Err(std::io::ErrorKind::InvalidInput)
    );

    // The file the compile_fail doctest refuses is refused as it is read.
    let broken = format!("{manifest}/tests/sql/broken/m20260101_000001_create_note.sql");
    let refused = SqlMigration::read(&broken).map(|_| ());
    let Err(refused) = refused else {
        panic!("a file the grammar refuses is refused");
    };
    assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);
    let message = refused.to_string();
    assert!(
        message.contains("m20260101_000001_create_note.sql"),
        "{message}"
    );
    assert!(
        message.contains("PostgreSQL rejected this SQL: syntax error"),
        "{message}"
    );
    Ok(())
}

/// A migration that tries something inside a savepoint and goes on when it
/// fails, which only an exclusively borrowed transaction allows.
struct Tolerant;

impl MigrationName for Tolerant {
    fn name(&self) -> &str {
        "m20260201_000001_tolerant"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Tolerant {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        let refused = tx
            .transaction(async |savepoint| {
                savepoint
                    .batch_execute("CREATE EXTENSION pgorm_no_such_extension")
                    .await
            })
            .await;
        assert!(refused.is_err());
        tx.batch_execute("CREATE TABLE tolerant (id integer)").await
    }
}

/// A migrator built at run time from files read now and a migration written
/// in Rust runs through the one runner: its ledger placed where it says, the
/// files' checksums recorded and checked, a savepoint inside a migration, and
/// a second run applying nothing.
// [spec:pgorm:def:migration.runtime/test]
// [spec:pgorm:def:migration.runner+5/test]    a migration's transaction is borrowed exclusively
// [spec:pgorm:req:migration.sql-file.read/test]    files read at run time run and are checksummed
#[tokio::test]
async fn runtime_migrations_use_the_runner() -> Result<(), Error> {
    use common::migrator::sql_files::{CREATE_NOTE_DIGEST, EDITED_NOTE_DIGEST};

    let ctx = TestContext::new("pgorm_migration_runtime").await;
    let db = &ctx.db;
    let manifest = env!("CARGO_MANIFEST_DIR");
    let built = |directory: &str| -> Result<Migrations, Error> {
        let mut migrations: Vec<Box<dyn MigrationTrait>> = vec![Box::new(Tolerant)];
        for migration in SqlMigration::read_dir(format!("{manifest}/{directory}"))
            .map_err(|error| Error::Custom(error.to_string()))?
        {
            migrations.push(Box::new(migration));
        }
        Ok(Migrations::new(migrations)
            .with_migration_table_schema(Name::runtime("ops"))
            .with_migration_table_name(Name::runtime("runtime_ledger"))
            .with_statement_cache_channel(None))
    };
    let migrations = built("tests/sql")?;

    let conn = db.get().await?;
    assert_eq!(migrations.get_pending_migrations(&conn).await?.len(), 3);
    drop(conn);
    migrations.up(db.clone(), None).await?;
    migrations.up(db.clone(), None).await?;
    assert!(has_table(db, "tolerant").await?);
    assert_eq!(count_rows(db, "note").await?, 2);
    assert_eq!(count_rows_in(db, "ops", "runtime_ledger").await?, 3);
    assert!(!has_table(db, "pgorm_migrations").await?);

    let conn = db.get().await?;
    let recorded: Option<String> = conn
        .query_one(
            "SELECT checksum FROM ops.runtime_ledger WHERE version = 'm20260101_000001_create_note'",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(recorded.as_deref(), Some(CREATE_NOTE_DIGEST));
    assert!(
        migrations
            .get_applied_migrations(&conn)
            .await?
            .iter()
            .all(|migration| migration.status() == MigrationStatus::Applied)
    );

    let mut edited = built("tests/sql/edited")?.migrations();
    edited.push(Box::new(sql_migration!(
        "tests/sql/m20260101_000002_seed_note.sql"
    )));
    let edited = Migrations::new(edited)
        .with_migration_table_schema(Name::runtime("ops"))
        .with_migration_table_name(Name::runtime("runtime_ledger"));
    let err = edited
        .status(&conn)
        .await
        .expect_err("an edited file read at run time must be reported");
    assert!(
        matches!(&err, Error::Custom(msg) if msg.contains(EDITED_NOTE_DIGEST)),
        "{err}"
    );
    drop(conn);
    ctx.delete().await;
    Ok(())
}
