# Migrations

A database's schema changes as the program does. A *migration* is one such
change, written once and applied to every database the program runs against:
a developer's, a test server's, production's. `pgorm-migration` runs them. It
records each migration it applies in a table of its own, the *ledger*, so that
each database receives each migration exactly once, in the order the program
lists them.

Migrations run forward only. A migration has an `up` and nothing else: there
is no `down`, and no command that rolls back, refreshes or resets a database.
A mistake is corrected by the next migration.

## Adding the crate

`pgorm-migration` lives in pgorm's repository beside pgorm, and is depended
on the same way:

```toml
[dependencies]
pgorm-migration = { git = "https://github.com/necessary-nu/pgorm" }
```

`pgorm_migration::prelude` brings in what a migration needs: the two traits,
the `MigrationName` derive, the `sql_migration!` macro, pgorm's connection
types and [`Error`](crate::Error), and the whole of the [query
builder](crate::guide::query_builder).

## Writing a migration

A migration is a type implementing `MigrationTrait`. Its `up` receives the
transaction the migration runs in, a
[`DatabaseTransaction`](crate::DatabaseTransaction) borrowed for the migration
alone, and changes the schema through it. The statements can come from the
query builder's DDL, from [schema generation](crate::guide::schema), or from
SQL text. Since the borrow is exclusive, a migration can also open a
[savepoint](crate::guide::transactions) or run a [COPY](crate::guide::copy)
inside it.

A *migrator* implements `MigratorTrait`, whose `migrations` lists every
migration the program owns, in the order they apply. It is usually a unit
struct, so the migrator is its one value, `Migrator`. Its `up` applies the
migrations the database has not had yet:

```rust
use pgorm_migration::prelude::*;

/// Creates the bakery's first table.
pub struct CreateBakery;

impl MigrationName for CreateBakery {
    fn name(&self) -> &str {
        "m20260101_000001_create_bakery"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateBakery {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        let table = Table::create(Bakery::Table)
            .col(ColumnDef::new(Bakery::Id).integer().not_null().auto_increment())
            .primary_key(Bakery::Id)
            .col(ColumnDef::new(Bakery::Name).string().not_null())
            .col(ColumnDef::new(Bakery::City).string().not_null())
            .to_owned();
        tx.batch_execute(&table.to_string()).await
    }
}

#[derive(SqlName)]
enum Bakery {
    Table,
    Id,
    Name,
    City,
}

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateBakery)]
    }
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("migrations_first").await;
# let pool = scratch.pool.clone();
Migrator.up(pool.clone(), None).await?;

let db = pool.get().await?;
db.execute("INSERT INTO bakery (name, city) VALUES ($1, $2)", &[&"Sunrise", &"Oslo"])
    .await?;

// Running it again applies nothing: the ledger says the table exists.
Migrator.up(pool.clone(), None).await?;
let applied = Migrator.get_applied_migrations(&db).await?;
let names: Vec<&str> = applied.iter().map(|migration| migration.name()).collect();
assert_eq!(names, ["m20260101_000001_create_bakery"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

`up` takes the [`DatabasePool`](crate::DatabasePool) by value; it is cheap to
clone. Its second argument bounds the run: `None` applies everything pending,
and `Some(n)` at most `n` migrations.

## Naming migrations

A migration's name is its identity in the ledger, so it must be unique and
must never change. In a migration crate each migration is a file of its own,
named for when it was written and what it does, such as
`m20260101_000001_create_bakery.rs`, and derives its name from the file:
`#[derive(MigrationName)]` names a migration for the stem of the file it is
written in.

```rust
use pgorm_migration::prelude::*;

#[derive(MigrationName)]
pub struct Migration;

# fn main() {
// In `src/m20260101_000001_create_bakery.rs` this is
// "m20260101_000001_create_bakery".
assert_eq!(Migration.name(), pgorm_migration::util::get_file_stem(file!()));
# }
```

Two files with the same stem in different directories would give two
migrations one name. The runner refuses a list that names one migration
twice, before it touches the database. Renaming a file renames its migration:
the old name is still in the ledger with no migration to match, and the
runner reports it as missing rather than applying the new name a second time.
A migration that must keep a name of its own implements `MigrationName` by
hand, as the examples in this chapter do.

## Migrations written as SQL

A migration can be a `.sql` file instead. `sql_migration!` takes the file's
path, relative to the crate's `Cargo.toml`, and makes a migration named for
the file's stem that runs the file's text. The macro reads the file while the
crate compiles and checks it against PostgreSQL's grammar, as
[`sql!`](crate::sql) checks a literal, so a syntax error in the file is a
compile error naming its line. The check is syntax only: a table the file
names but the database lacks is the server's to report when the migration
runs.

```rust
# mod create_bakery {
#     use pgorm_migration::prelude::*;
#     pub struct Migration;
#     impl MigrationName for Migration {
#         fn name(&self) -> &str {
#             "m20260101_000001_create_cake"
#         }
#     }
#     #[async_trait::async_trait]
#     impl MigrationTrait for Migration {
#         async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
#             tx.batch_execute("CREATE TABLE cake (id serial PRIMARY KEY, name varchar NOT NULL)").await
#         }
#     }
# }
use pgorm_migration::prelude::*;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(create_bakery::Migration),
            Box::new(sql_migration!("docs/guide/migrations/m20260102_000001_index_cake_name.sql")),
        ]
    }
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("migrations_sql_file").await;
# let pool = scratch.pool.clone();
let index = sql_migration!("docs/guide/migrations/m20260102_000001_index_cake_name.sql");
assert_eq!(index.name(), "m20260102_000001_index_cake_name");
assert!(index.sql().contains("CREATE INDEX cake_name_index ON cake (name);"));

Migrator.up(pool.clone(), None).await?;
let db = pool.get().await?;
let found = db
    .query_one("SELECT count(*) FROM pg_indexes WHERE indexname = 'cake_name_index'", &[])
    .await?;
assert_eq!(found.get::<_, i64>(0), 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A file migration reports the SHA-256 digest of its text as its checksum, so an
edit made after it ran is caught, as the next sections show.

## One transaction, one runner at a time

`up` runs every pending migration in one transaction, with each migration's
ledger row, and commits once at the end. If any migration fails, the whole
run rolls back: the schema and the ledger are as they were before it started.

Before it reads the ledger, the run takes a PostgreSQL advisory lock keyed by
the ledger's name. Two deploys that start together therefore queue rather than
collide: the second waits until the first commits, then finds nothing left to
apply.

```rust
use pgorm_migration::prelude::*;

pub struct CreateCustomer;

impl MigrationName for CreateCustomer {
    fn name(&self) -> &str {
        "m20260101_000001_create_customer"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateCustomer {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.batch_execute("CREATE TABLE customer (id serial PRIMARY KEY, email text UNIQUE NOT NULL)")
            .await
    }
}

/// A migration that cannot succeed: the table already exists.
pub struct CreateCustomerAgain;

impl MigrationName for CreateCustomerAgain {
    fn name(&self) -> &str {
        "m20260101_000002_create_customer_again"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateCustomerAgain {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.batch_execute("CREATE TABLE customer (id serial PRIMARY KEY)").await
    }
}

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateCustomer)]
    }
}

pub struct BrokenMigrator;

impl MigratorTrait for BrokenMigrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateCustomer), Box::new(CreateCustomerAgain)]
    }
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("migrations_transaction").await;
# let pool = scratch.pool.clone();
// The second migration fails, so the first is rolled back with it.
assert!(BrokenMigrator.up(pool.clone(), None).await.is_err());
let db = pool.get().await?;
let tables = db
    .query_one("SELECT count(*) FROM pg_tables WHERE tablename = 'customer'", &[])
    .await?;
assert_eq!(tables.get::<_, i64>(0), 0);

// Two runners at once: one applies the migration, the other waits and finds
// nothing to do.
let (first, second) = tokio::join!(
    Migrator.up(pool.clone(), None),
    Migrator.up(pool.clone(), None),
);
first?;
second?;
assert_eq!(Migrator.get_applied_migrations(&db).await?.len(), 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

### Committing partway

Some changes PostgreSQL will not let one transaction finish. A value that
`ALTER TYPE ... ADD VALUE` adds to an enum cannot be used until the
transaction that added it has committed. A migration making such a change
answers `true` from `commits_after`. `up` then commits the run so far once
that migration and its ledger row are written, and runs the rest in a new
transaction, which takes the lock again.

Such a run is no longer all or nothing. A failure after the commit rolls back
only the transaction it happens in; the migrations committed before it stay
applied, and the next `up` resumes after them.

```rust
use pgorm_migration::prelude::*;

macro_rules! migration {
    ($type:ident, $name:literal, $sql:literal, $commits_after:literal) => {
        pub struct $type;

        impl MigrationName for $type {
            fn name(&self) -> &str {
                $name
            }
        }

        #[async_trait::async_trait]
        impl MigrationTrait for $type {
            async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
                tx.batch_execute($sql).await
            }

            fn commits_after(&self) -> bool {
                $commits_after
            }
        }
    };
}

migration!(
    CreateCake,
    "m20260101_000001_create_cake",
    "CREATE TYPE flavor AS ENUM ('chocolate', 'vanilla');
     CREATE TABLE cake (id serial PRIMARY KEY, name text NOT NULL, flavor flavor NOT NULL)",
    false
);
migration!(
    AddCarrot,
    "m20260101_000002_add_carrot",
    "ALTER TYPE flavor ADD VALUE 'carrot'",
    true
);
migration!(
    AddCarrotCake,
    "m20260101_000003_add_carrot_cake",
    "INSERT INTO cake (name, flavor) VALUES ('Carrot Cake', 'carrot')",
    false
);

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateCake), Box::new(AddCarrot), Box::new(AddCarrotCake)]
    }
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("migrations_commits_after").await;
# let pool = scratch.pool.clone();
Migrator.up(pool.clone(), None).await?;

let db = pool.get().await?;
let row = db.query_one("SELECT flavor::text FROM cake", &[]).await?;
assert_eq!(row.get::<_, String>(0), "carrot");
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

[Disjoint enums](crate::guide::disjoint) produces changes of this kind: a change to
the kinds is a list of steps, one migration each, every step but the last
answering `true`. Frozen into migration files, each such step opens with the
line `-- pgorm: commits-after`, which answers `true` for a file
([Generated DDL in a migration](#generated-ddl-in-a-migration)).

## The ledger

The ledger is a table named `pgorm_migrations`. It has one row per applied
migration: its `version`, the migration's name; `applied_at`, the Unix time in
seconds it was applied; and `checksum`, what the migration reported as its
checksum, or `NULL`. `up` creates the table the first time it runs. Nothing in
pgorm-migration deletes a row.

By default the ledger sits wherever `search_path` puts an unqualified name,
usually beside the application's tables in `public`. A migrator can place it
in a schema of its own with `migration_table_schema`, which keeps it apart
from triggers, policies and grants that apply to every application table, and
rename it with `migration_table_name`:

```rust
use pgorm_migration::prelude::*;

pub struct CreateBakery;

impl MigrationName for CreateBakery {
    fn name(&self) -> &str {
        "m20260101_000001_create_bakery"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateBakery {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.batch_execute("CREATE TABLE bakery (id serial PRIMARY KEY, name text NOT NULL)")
            .await
    }
}

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateBakery)]
    }

    fn migration_table_schema(&self) -> Option<Name> {
        Some(Name::runtime("ops"))
    }

    fn migration_table_name(&self) -> Name {
        Name::runtime("bakery_migrations")
    }
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("migrations_ledger").await;
# let pool = scratch.pool.clone();
Migrator.up(pool.clone(), None).await?;

let db = pool.get().await?;
let row = db
    .query_one("SELECT version, checksum FROM ops.bakery_migrations", &[])
    .await?;
assert_eq!(row.get::<_, String>(0), "m20260101_000001_create_bakery");
assert_eq!(row.get::<_, Option<String>>(1), None);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

`up` creates the schema if it is missing, and names the table in it
everywhere, so a table of the same name elsewhere on `search_path` is never
taken for it. The runner does not move a ledger that already exists: a ledger
left in its old place reads as absent from the new one, and every migration
would run again. Move it with `ALTER TABLE .. SET SCHEMA` before deploying a
release that names a new place.

### A ledger from SeaORM

A database last migrated by SeaORM keeps its ledger under the name
`seaql_migrations`, with the same `version` and `applied_at` columns. The first
`up` renames that table to `pgorm_migrations` in place, so nothing runs
twice. This applies to the default name with no schema; a migrator that names
or places its own ledger takes the old table over by hand.

```rust
use pgorm_migration::prelude::*;

pub struct CreateCake;

impl MigrationName for CreateCake {
    fn name(&self) -> &str {
        "m20220101_000001_create_cake"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateCake {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.batch_execute("CREATE TABLE cake (id serial PRIMARY KEY)").await
    }
}

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateCake)]
    }
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("migrations_seaorm").await;
# let pool = scratch.pool.clone();
let db = pool.get().await?;
// What SeaORM left behind: the table, and the ledger row saying it exists.
db.batch_execute(
    "CREATE TABLE cake (id serial PRIMARY KEY);
     CREATE TABLE seaql_migrations (version varchar PRIMARY KEY, applied_at bigint NOT NULL);
     INSERT INTO seaql_migrations VALUES ('m20220101_000001_create_cake', 1640995200);",
)
.await?;

// The migration is not run again, which would fail: the table exists.
Migrator.up(pool.clone(), None).await?;
let row = db.query_one("SELECT applied_at FROM pgorm_migrations", &[]).await?;
assert_eq!(row.get::<_, i64>(0), 1640995200);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Asking what has run

`status` logs, at `info` level through `tracing`, each migration's name and
whether it is `Pending` or `Applied`. `get_migration_with_status`,
`get_pending_migrations` and `get_applied_migrations` answer the same question
as values. Each takes any [`ConnectionTrait`](crate::ConnectionTrait) and only
reads: against a database never migrated, every migration is pending and no
ledger is created to say so. A readiness probe or a CI step can ask with a role
that has no right to change the schema.

```rust
use pgorm_migration::prelude::*;

pub struct CreateBakery;

impl MigrationName for CreateBakery {
    fn name(&self) -> &str {
        "m20260101_000001_create_bakery"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateBakery {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.batch_execute("CREATE TABLE bakery (id serial PRIMARY KEY)").await
    }
}

pub struct CreateCake;

impl MigrationName for CreateCake {
    fn name(&self) -> &str {
        "m20260101_000002_create_cake"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateCake {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.batch_execute("CREATE TABLE cake (id serial PRIMARY KEY)").await
    }
}

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateBakery), Box::new(CreateCake)]
    }
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("migrations_status").await;
# let pool = scratch.pool.clone();
let db = pool.get().await?;
assert_eq!(Migrator.get_pending_migrations(&db).await?.len(), 2);

// Apply one migration of the two.
Migrator.up(pool.clone(), Some(1)).await?;

let status: Vec<(String, MigrationStatus)> = Migrator.get_migration_with_status(&db)
    .await?
    .iter()
    .map(|migration| (migration.name().to_owned(), migration.status()))
    .collect();
assert_eq!(
    status,
    [
        ("m20260101_000001_create_bakery".to_owned(), MigrationStatus::Applied),
        ("m20260101_000002_create_cake".to_owned(), MigrationStatus::Pending),
    ]
);
Migrator.status(&db).await?;
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Catching an edited migration

The ledger matches migrations by name, so a migration edited after it ran
would go unnoticed: its name still matches, and a database built afresh would
quietly differ from the ones built before. A migration that reports a
`checksum` closes that gap. `up` stores the checksum in the migration's ledger
row, and every later `status` and `up` compares it with what the migration
reports now, failing with an error that names the migration when the two
differ.

The checksum is any text the author controls, such as a hash of the
migration's SQL or a version bumped by hand. It is off by default: a
migration that reports none is never checked, nor is a row written before it
reported one. A file migration from `sql_migration!` always reports one.

```rust
use pgorm_migration::prelude::*;

pub struct CreateBakery {
    revision: &'static str,
}

impl MigrationName for CreateBakery {
    fn name(&self) -> &str {
        "m20260101_000001_create_bakery"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateBakery {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.batch_execute("CREATE TABLE bakery (id serial PRIMARY KEY)").await
    }

    fn checksum(&self) -> Option<String> {
        Some(self.revision.to_owned())
    }
}

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateBakery { revision: "1" })]
    }
}

/// The same migration, edited after it ran.
pub struct EditedMigrator;

impl MigratorTrait for EditedMigrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateBakery { revision: "2" })]
    }
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("migrations_checksum").await;
# let pool = scratch.pool.clone();
Migrator.up(pool.clone(), None).await?;

let refused = EditedMigrator.up(pool.clone(), None).await;
match refused {
    Err(Error::Custom(message)) => {
        assert!(message.contains("'m20260101_000001_create_bakery' was applied with checksum '1'"));
    }
    other => panic!("expected the edit to be caught, got {other:?}"),
}
# scratch.finish().await;
# Ok(())
# }
```

## Generated DDL in a migration

[Schema generation](crate::guide::schema) writes a table's DDL from its entity,
so the entity is the one place the table is described. But an entity
describes the table as it is now, while a migration describes a change made
once. A migration that generated its DDL as it ran would follow the entity as
it changed: once the entity gained a column, a database built from nothing
would get the column from the first migration and then again from the
migration that added it, and fail.

So generate the DDL once, when the change is made, and freeze it into a
migration written as SQL. [`MigrationFile`](crate::MigrationFile), which the
prelude exports, holds generated statements as a file's text, and
[`freeze`](crate::MigrationFile::freeze) writes `<name>.sql` into the
migrations directory if it is not there, answering whether it wrote it. A
file already there is never rewritten: it is the migration databases have
run, its checksum in their ledgers. So the code that freezes each change can
live in a test or a small binary of the migration crate and run again
whenever a change is added, writing the new file and keeping the old ones,
however far the entities have moved on. Run it, then list the new file with
`sql_migration!`, which reads the file as the crate compiles. The prelude's
`Schema` is the query builder's `CREATE SCHEMA` statement, so code that
generates DDL names [`pgorm::Schema`](crate::Schema) by its path:

```rust
use pgorm_guide_support::bakery;
use pgorm_migration::prelude::*;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(sql_migration!(
            "docs/guide/migrations/m20260101_000001_create_bakery.sql"
        ))]
    }
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
// Frozen when the bakery table was added; freezing again writes nothing.
let create = pgorm::Schema::new().create_table_from_entity(bakery::Entity)?;
let file = MigrationFile::new("m20260101_000001_create_bakery", [create]);
assert_eq!(
    file.text(),
    "CREATE TABLE \"bakery\" ( \"id\" serial NOT NULL, \"name\" varchar NOT NULL, \"city\" varchar NOT NULL, PRIMARY KEY (\"id\") );\n"
);
assert!(!file.freeze(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/guide/migrations"))?);

# let scratch = pgorm_guide_support::Scratch::empty("migrations_generated").await;
# let pool = scratch.pool.clone();
use pgorm::entity::prelude::*;

Migrator.up(pool.clone(), None).await?;
let db = pool.get().await?;
let sunrise = bakery::ActiveModel {
    name: set("Sunrise"),
    city: set("Oslo"),
    ..Default::default()
}
.insert(&db)
.await?;
assert_eq!(sunrise.id, 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A change PostgreSQL will not let one transaction finish is frozen into
several files, a [disjoint enum](crate::guide::disjoint)'s new kind into one
per step. A file whose first line is `-- pgorm: commits-after`, as
[`committing_after`](crate::MigrationFile::committing_after) writes it,
answers `true` from `commits_after`, so its batch commits once it has run, as
[Committing partway](#committing-partway) describes.

## A migrator built at run time

A migrator need not be a type of the program's own. `Migrations` is one built
as the program runs, from a list of migrations it is given, and it runs them
with the same runner: the ledger, the lock, the checksums and everything else
this chapter describes. `SqlMigration::read_dir` reads every `.sql` file in a
directory, in the order of their names, each as `sql_migration!` would have
read it: named for its file, its checksum the digest of its text, and a file
opening with `-- pgorm: commits-after` committing its batch. So a program can
replay the files it has frozen without listing each in its source. The files
are read when the program runs rather than when it compiles, so their grammar
is checked by the server as each migration runs, not by the compiler.

Here the frozen history of this guide's migrations directory is replayed on
an empty database, after a migration written in Rust that creates the cakes
the second file indexes. `with_migration_table_schema` and
`with_migration_table_name` place the ledger, as `migration_table_schema` and
`migration_table_name` do for a migrator declared as a type:

```rust
use pgorm_migration::prelude::*;

pub struct CreateCake;

impl MigrationName for CreateCake {
    fn name(&self) -> &str {
        "m20251231_000001_create_cake"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateCake {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.batch_execute("CREATE TABLE cake (id serial PRIMARY KEY, name text NOT NULL)")
            .await
    }
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("migrations_runtime").await;
# let pool = scratch.pool.clone();
let mut migrations: Vec<Box<dyn MigrationTrait>> = vec![Box::new(CreateCake)];
for file in SqlMigration::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/guide/migrations"))? {
    migrations.push(Box::new(file));
}
let migrator = Migrations::new(migrations)
    .with_migration_table_schema(Name::runtime("ops"))
    .with_migration_table_name(Name::runtime("bakery_migrations"));

migrator.up(pool.clone(), None).await?;

let db = pool.get().await?;
let applied: Vec<String> = migrator
    .get_applied_migrations(&db)
    .await?
    .iter()
    .map(|migration| migration.name().to_owned())
    .collect();
assert_eq!(
    applied,
    [
        "m20251231_000001_create_cake",
        "m20260101_000001_create_bakery",
        "m20260102_000001_index_cake_name",
        "m20260201_000001_create_payment",
        "m20260301_000001_add_voucher_1",
        "m20260301_000001_add_voucher_2",
    ]
);
// The file read now reports the checksum the compiled one does.
let compiled = sql_migration!("docs/guide/migrations/m20260102_000001_index_cake_name.sql");
let recorded: Option<String> = db
    .query_one(
        "SELECT checksum FROM ops.bakery_migrations WHERE version = $1",
        &[&compiled.name()],
    )
    .await?
    .get(0);
assert_eq!(recorded, compiled.checksum());
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

pgorm's [language bindings](crate::guide::bindings) run their migrations this
way: a list of the language's own migrations and of files read from a
directory, run by this runner.

## Statement caches

pgorm prepares each statement once per pooled connection and reuses it. Most
schema changes make PostgreSQL plan the statement again, but some do not: an
insert prepared before a domain's default changed keeps writing the old
default. So once a run that applied a migration commits, `up` clears the
statement caches of the pool it was given. It also sends a notification on
`statement_cache_channel`, `pgorm_statement_caches` by default, from inside
the run's transaction, so it is delivered exactly when the run commits. Every
other process's pool that listens there clears too. A migrator that answers
`None` sends nothing. [Statement caches and DDL](crate::guide::statement_caches)
covers both.

## A migration crate

There is no command-line tool: a program's migrations are a crate of their
own with a small binary that runs them. A starter crate lives in pgorm's
repository at `pgorm-migration/template/migration/`. Its `src/lib.rs` holds
the migrator, one file per migration beside it, and its `src/main.rs` reads
`DATABASE_URL` and runs `up`, `up <n>` or `status`:

```sh
DATABASE_URL=postgres://localhost/bakery cargo run -p migration -- up
DATABASE_URL=postgres://localhost/bakery cargo run -p migration -- status
```

Copy it into the project, add each new migration's module to `migrations`, and
run the binary as part of each deploy. A program can equally call
`Migrator.up(pool, None)` itself at startup; the advisory lock makes that safe
when several replicas start at once.
