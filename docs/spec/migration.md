# Migrations

`pgorm-migration` runs ordered, forward-only schema changes against a PostgreSQL
database and records what it has applied in a ledger table. The crate is small
by design: `src/lib.rs` declares the two author-facing traits, `src/migrator.rs`
holds the whole runner, `src/ledger.rs` is the ledger entity, and `src/util.rs`
derives a migration's identity from its filename.

Unlike SeaORM, there is no `SchemaManager` indirection — a migration receives
the live `DatabaseTransaction` and drives it through `ConnectionTrait` with
`pgorm_query` builders.

## The runner and its ledger

> [spec:pgorm:def:migration.runner+5]
> A migration is a `MigrationTrait` implementor: `MigrationName + Send + Sync`
> plus a single `async fn up(&self, tx: &mut DatabaseTransaction<'_>)`, and an
> optional `fn checksum(&self) -> Option<String>` defaulting to `None`. The
> transaction is borrowed exclusively, as a transaction closure's is
> (`conn.tx.closure`), so a migration can open a savepoint inside the batch or
> run a COPY through it as well as run statements. A
> migrator is a `MigratorTrait` implementor (`Send + Sync`) whose only required
> method is `migrations(&self) -> Vec<Box<dyn MigrationTrait>>`, the ordered
> list of every migration the project owns. Every method of the trait takes
> `&self`, so a migrator is a value: a unit struct declared for a project's
> fixed list, called as `Migrator.up(pool, None)`, or a `Migrations` built at
> run time (`migration.runtime`).
>
> `MigratorTrait` provides, on top of that list: `migration_table_name() ->
> Name` (defaulting to the `ledger` entity's name) and `migration_table_schema()
> -> Option<Name>` (defaulting to `None`) to place the ledger, `install(db)` to
> create it, `adopt_legacy_ledger(db)` to take over one left under the
> inherited name, `up(db, steps)` to apply pending migrations, `status(db)` to
> log each migration's state, `lock_key()` and `lock(tx)` for the run's advisory
> lock, and the `get_migration_files` / `get_migration_models` /
> `get_migration_with_status` / `get_pending_migrations` /
> `get_applied_migrations` accessors. Every method except `up`, `lock_key` and
> `lock` takes `&impl ConnectionTrait`, so they can be called on a pooled
> connection or inside a caller's transaction; `up` takes an owned
> `DatabasePool` because it acquires and manages its own connection, and `lock`
> takes a `&DatabaseTransaction<'_>` because the lock it takes is
> transaction-scoped. `get_migration_files` is the one accessor that touches no
> database yet is fallible: it is where the name-uniqueness check lives.
>
> The ledger is a three-column table — `version TEXT NOT NULL PRIMARY KEY`,
> `applied_at BIGINT NOT NULL`, and a nullable `checksum TEXT` — mirrored by the
> `ledger::Model` entity, whose `checksum` field is `Option<String>`. `install`
> creates it with `IF NOT EXISTS` under `migration_table_name()`, in
> `migration_table_schema()` when that is set, so an overridden placement is
> honoured on creation as well as on read and write. It is the only method that
> provisions anything, and `up` is the only method that calls it — the
> accessors read instead, per `migration.read-only`. Results are reported as
> `Migration` values exposing `name()` and `status()`, where `MigrationStatus`
> is `Pending` or `Applied` and `Display`s as those words.
>
> `migration_table_schema()` puts the ledger in a schema of its own. `None`
> leaves the table's name unqualified, so it resolves through `search_path` as
> any unqualified name does, which is where every earlier release put it. A
> schema keeps the ledger apart from the application's tables and from the
> triggers, policies and grants they carry: `install` creates the schema before
> the table when `pg_namespace` does not list it, with `CREATE SCHEMA IF NOT
> EXISTS`, and every statement the runner issues against the ledger names the
> table qualified by it — the creation and the widening, the catalogue reads
> that locate the table and inspect its columns, the read of its rows, and each
> row's insert — so a table of the same name elsewhere on `search_path` is never
> taken for it. The catalogue is read before the `CREATE SCHEMA` because that
> statement checks the database's `CREATE` privilege before it checks whether
> the schema exists, and a role that owns a schema made for it need not hold
> that privilege. The schema does not enter `lock_key()`: ledgers of one name in
> two schemas share a key, which orders their runs and does nothing else, and
> the default ledger keeps the key every earlier release took. Moving a ledger
> that already exists is the operator's step, not the runner's: a ledger left
> where it was reads as absent from the new place, so every migration would run
> again. It is moved with `ALTER TABLE ... SET SCHEMA` before a release that
> names the schema runs.
>
> The ledger's default physical name is `pgorm_migrations`, exposed as
> `DEFAULT_LEDGER_TABLE`, and the Rust module holding its entity is `ledger`.
> Neither carries the `seaql_migrations` name inherited from SeaORM: the module
> named a vendor rather than a role, and the table put another project's name in
> the schema of every database pgorm touches. `ledger` is the word the rest of
> this document already uses for the thing, and `pgorm_migrations` reads in
> `\dt` the way the old name did, so an operator recognises it without being
> told. The old table name survives in exactly one place — as
> `LEGACY_LEDGER_TABLE`, the name adoption looks for.

> [spec:pgorm:sem:migration.up+6]
> `up(db, steps)` acquires one pooled connection, opens one transaction, and
> runs the entire batch inside it before committing, unless a migration of
> the batch asks to be committed before the next one runs
> (`[spec:pgorm:req:migration.commit-after]`): then each transaction the batch
> is split into acquires a pooled connection of its own and runs as this one
> does, so the clear after one commit reaches the connection the next
> transaction runs on. The transaction's first
> statement is `SELECT pg_advisory_xact_lock($key)`, where `$key` is
> `lock_key()`: a stable FNV-1a hash of `migration_table_name()`, spelled out in
> the crate rather than delegated to `DefaultHasher` so that two processes built
> by different compilers still agree on it. Only then does it install the
> ledger — which is also where a legacy-named ledger is adopted, so the rename
> happens under the lock and no batch can begin against a ledger another runner
> is midway through moving — compute the pending set, and apply migrations in
> `migrations()` declaration order — not sorted by name — passing each the
> batch's `&mut DatabaseTransaction`. After each successful `up` it inserts a ledger row
> whose `version` is the migration's name, whose `applied_at` is the current
> Unix time in seconds, and whose `checksum` is that migration's `checksum()`.
>
> Once the batch commits, and only if it applied at least one migration, `up`
> clears the statement caches of the pool it was given
> (`conn.pool.statement-cache.clear`). The batch may have changed what a
> statement the pool's connections had already prepared was planned with, and
> PostgreSQL does not re-plan every such change: an insert prepared before a
> migration changed a domain's default keeps writing the old one
> (`sql.ddl.domain`). A run that applied nothing changed no schema but perhaps
> the ledger's own, and a batch that rolled back changed nothing, so neither
> clears, and re-running `up` stays a no-op.
>
> The same batch, after its last ledger row and before it commits, also
> notifies `statement_cache_channel()` with an empty payload, through
> `pgorm::notify` on the batch's own transaction. The hook answers
> `Some(STATEMENT_CACHE_CHANNEL)` unless a migrator overrides it, and `None`
> sends nothing. The local clear reaches only the pool `up` was given, while
> every pool listening on the channel clears on the notification
> (`conn.pool.statement-cache.listen`): another replica's, a worker's,
> whichever process holds it. Sending it inside the transaction makes
> PostgreSQL deliver it exactly when the batch commits and never when it rolls
> back, the condition the local clear waits for, with no statement after the
> commit that could fail once the batch is already applied. It is sent whether
> or not any session listens, which costs nothing past the commit, and only
> when the batch applied at least one migration. A channel the server refuses
> — empty, or longer than 63 bytes — fails the notification, and with it the
> batch.
>
> The lock makes concurrent runners queue instead of collide. Without it the
> loser of a race fails on whatever it happened to hit first — a duplicate
> `version` key, or the catalog race inside two simultaneous `CREATE TABLE IF
> NOT EXISTS` on a fresh database — so parallel deploys of the same release
> fail intermittently on an error a retry would cure. Because the lock is
> transaction-scoped it is released by the winner's commit, and because the
> pending set is computed after the lock is held the waiter recomputes it
> against the winner's committed ledger and finds nothing to do. The lock
> orders runs; it is not what makes them safe — that is still the single
> transaction below.
>
> Pending detection is a set difference on names: every name in `migrations()`
> that has no ledger row is pending, and everything else is `Applied`. This is
> order-insensitive, so a migration inserted earlier in the list than
> already-applied ones is still detected and applied. `steps` bounds the batch
> and is checked before each migration, so `Some(0)` applies nothing and
> `None` applies everything pending. Re-running `up` with no new migrations is
> therefore a no-op.
>
> Because the whole batch shares one transaction, any error — from a
> migration's own `up` or from the ledger insert — aborts and rolls back every
> migration in that run together with its ledger rows and, on a fresh database,
> the ledger table itself. The only commit inside a batch is the one a
> migration asks for (`[spec:pgorm:req:migration.commit-after]`); a failure
> after it rolls back the transaction it happens in, and the migrations
> committed before it stay applied.
>
> The reverse direction is also checked: if the ledger holds a version with no
> corresponding entry in `migrations()`, `get_migration_with_status` fails with
> `Error::Custom` naming each missing migration, so a deleted or renamed
> migration file surfaces as an error rather than silently re-running.

> [spec:pgorm:req:migration.commit-after]
> A migration whose `commits_after()` answers `true` (the trait's default is
> `false`) MUST be committed, with its ledger row, before the next migration
> of the batch runs: `up` commits the batch so far once that migration's row
> is written, and runs the rest in a new transaction whose first statement
> takes the advisory lock again, under which it recomputes the pending set, so
> a runner that queued behind the first transaction starts nothing the first
> one already applied. This is for a change PostgreSQL will not let one
> transaction finish — a value `ALTER TYPE ... ADD VALUE` adds cannot be used
> before the transaction that added it commits — and it makes the batch no
> longer all-or-nothing: a failure after a commit rolls back only the
> transaction it happens in, the migrations committed before it stay applied
> with their ledger rows, and running `up` again resumes after them. Each
> commit clears the pool's statement caches and notifies
> `statement_cache_channel()` as the end of a batch does, inside the
> transaction it ends. A batch with no such migration is one transaction, as
> before.

## Forward-only by construction

> [spec:pgorm:req:migration.up-only]
> Migrations MUST be forward-only. `MigrationTrait` declares `up` and nothing
> else, and `MigratorTrait` offers no `down`, `fresh`, `refresh`, or `reset`.
> The ledger is append-only in practice: nothing in the crate deletes a
> migration row. A mistake is corrected by writing a new migration, never by
> rolling one back.
>
> This is a deliberate divergence from SeaORM, whose `down` half was removed
> along with the `SchemaManager` wrapper that existed largely to make reversible
> DDL expressible. A `down` written months earlier is rarely exercised, cannot
> restore data the corresponding `up` dropped, and gives false confidence in
> production recovery; forward-only migrations keep the applied history a
> monotonic, auditable log. Callers needing a reversal write it as the next
> migration.

> [spec:pgorm:sem:migration.name+4]
> A migration's identity — the value written to and matched against the
> ledger's `version` column — is its **source filename** without extension, not
> its type name. The `MigrationName` derive implements `MigrationName::name` as
> `util::get_file_stem(file!())`, which takes the `file!()` path's file stem, so
> every migration in a project must live in its own uniquely-named file and the
> conventional `m<timestamp>_<description>.rs` naming is what makes
> declaration order legible. `get_file_stem` is public API — the derive expands
> in the caller's crate — so it is reachable with any string and MUST be total:
> a path carrying no file stem, or one that is not valid UTF-8, yields that path
> unchanged rather than panicking.
>
> Uniqueness is enforced, not assumed. `get_migration_files` collects the names
> and MUST fail with an `Error::Custom` naming every repeated one — sorted, so
> the message does not depend on hash order — before any other step: before the
> ledger is installed, before a status is computed, and before a migration is
> applied. That placement is the requirement. Duplicates are reachable through
> the derive, which sees only the file stem, so `a/m1.rs` and `b/m1.rs` collide;
> left unchecked they read as one ledger row, which makes `up` on a fresh
> database die on the `version` primary key with the whole batch rolled back,
> and makes a `steps`-bounded run worse still — both migrations report
> `Applied` while only one ever ran, so the ledger says a schema exists that
> does not.
>
> The consequence of the filename rule is that renaming a migration file renames
> the migration: the old name remains in the ledger with no matching entry in
> `migrations()`, which `get_migration_with_status` reports as a
> missing-migration `Error::Custom`, and the new name is treated as pending and
> re-applied. Implementors may bypass the derive and write `MigrationName` by
> hand to pin a name independently of the filename — which is also how two
> migrations that must share a filename are told apart.

## Detecting an edited migration

> [spec:pgorm:req:migration.checksum+1]
> The ledger matches on name alone, so editing a migration that has already run
> is invisible: the name still matches, the row still says `Applied`, and a
> database built from scratch afterwards diverges from the one built before
> while both ledgers read identically. `MigrationTrait::checksum(&self) ->
> Option<String>` is the opt-in against that. `exec_up` stores the value in the
> new row's `checksum` column, and `get_migration_with_status` — reached by
> `status`, by `up`, and by the pending/applied accessors — compares the stored
> value against what the migration reports now, failing with an `Error::Custom`
> that names the migration and both digests when they differ.
>
> The check is deliberately three-valued. A stored `NULL` is unverifiable and
> MUST pass: rows written before the column existed, and rows written by a
> migration that reports no checksum, are grandfathered rather than treated as
> drift. A migration that has stopped reporting a checksum likewise MUST pass,
> because absence is not evidence. Only a stored digest that disagrees with a
> reported one is an error.
>
> `checksum()` defaults to `None` and there is no derived answer. Nothing the
> crate can see is a faithful digest of "what this migration does": the name is
> already the ledger key, and hashing the source file would report a
> reformatted comment as a schema change while missing a changed constant in an
> included file. Opting in is an override the author writes, and the value is
> opaque to the runner — a hash of the DDL text and a hand-bumped version string
> are equally valid.
>
> Existing deployments carry a two-column ledger, and `CREATE TABLE IF NOT
> EXISTS` will not widen it, so `install` MUST also add the column to a table
> that predates it. It consults `information_schema.columns` first and issues
> the `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` only when the column is
> missing, because `ADD COLUMN IF NOT EXISTS` takes an `ACCESS EXCLUSIVE` lock
> even in the case where it does nothing, and inside `up` that lock would then
> be held for the length of the migration batch.
>
> The widening step sits after legacy-name adoption in `install`, and is the
> only widening step there is. A ledger adopted from the old name is by
> definition older than the checksum column, so it arrives needing exactly what
> the widening already does; making adoption a separate provisioning path would
> mean two ways to reach a three-column ledger and two chances to disagree about
> its shape.
>
> Widening is therefore not a precondition of reading. A read reaching a ledger
> that predates the column MUST answer from it rather than widen it
> (`migration.read-only`), so it MUST NOT name a column that is not there: it
> projects a bound text `NULL` under the `checksum` alias and leaves the column
> list otherwise unchanged, which decodes into the same `Option<String>` the
> widened shape would have yielded. That substitution is exactly faithful
> because widening adds the column `NULL`-filled — a pre-checksum ledger's rows
> are unverifiable both before and after — so the three-valued check
> grandfathers them either way and the reported status does not depend on
> whether the widening has happened yet.

## A migration written as a SQL file

> [spec:pgorm:def:migration.sql-file+1]
> `sql_migration!("path/m20260101_000001_name.sql")` is a migration written as a
> `.sql` file. It expands to a `SqlMigration`, a `MigrationTrait` implementor
> holding the migration's name and the file's text, each borrowed for the
> program's life or owned, which boxes into `migrations()` beside migrations
> written in Rust. Its `name()` is the file stem of the path the macro was
> given: the identity `migration.name` gives a Rust migration through `file!()`,
> read from the SQL file instead, so `m20260101_000001_name.sql` and
> `m20260101_000001_name.rs` name the same migration and `get_migration_files`
> refuses the pair. Its `up` hands the text to `batch_execute` on the batch's
> transaction, which runs a `;`-separated script whole through the simple-query
> protocol, with no parameters, so a failing statement rolls back the run as
> any migration's error does.
>
> `SqlMigration::new(name, sql)` builds the same value from a name and text the
> caller supplies, either of them a `&'static str` or an owned `String`, without
> the macro's file or its compile-time check, and `sql()` returns the text a
> value runs. `SqlMigration::read` and `SqlMigration::read_dir` read files as
> the program runs (`migration.sql-file.read`).
>
> The macro is `pgorm-migration`'s, and its prelude exports it. It forwards
> `$crate` and the caller's tokens to a hidden proc macro in `pgorm-sql-macro`,
> the crate that holds the grammar check `sql!` makes (`macros.sql`), so the
> expansion names `SqlMigration` by the path the caller reaches
> `pgorm-migration` by, renamed or not.

> [spec:pgorm:req:migration.sql-file.checksum]
> A `SqlMigration` MUST report a checksum, and it MUST be the lowercase
> hexadecimal SHA-256 digest of the text `up` runs. This is the opt-in
> `migration.checksum` leaves to the author, taken on the author's behalf,
> because the objection to hashing a source file does not hold here: the file
> is the whole migration, so no change to what it does can hide outside it. A
> comment edited in an applied file therefore reads as an edit, which is the
> forward-only posture of `migration.up-only`: an applied file is left as it
> is, and the next file says what changed.
>
> The digest MUST NOT depend on where or how the file was built. The text is
> the file's bytes, as UTF-8, with every CRLF line ending written as LF, so a
> checkout that converts line endings — git's `core.autocrlf` on Windows — runs
> the same script and records the same digest as one that does not, and a
> ledger written from one machine verifies from another. The algorithm and its
> encoding are fixed, because changing either would report every applied file
> as edited.

> [spec:pgorm:req:migration.sql-file.compile+1]
> The file MUST be read while the calling crate compiles, and a file the macro
> cannot use MUST be a compile error spanned on the path literal rather than a
> failure at run time: a path that names no file stem, a file that cannot be
> read, text that is not UTF-8, and text PostgreSQL's grammar refuses. The
> grammar check is `sql!`'s — `pg_query::parse` over the whole script, held to
> PostgreSQL 18's grammar, syntax only (`macros.sql.script`,
> `macros.sql.ceiling`) — and the error names the file and carries the
> parser's diagnostic. When the diagnostic quotes a token the file holds, the
> error also names that token's line and reprints the line with a caret under
> the token. A diagnostic quoting no token, such as `syntax error at end of
> input`, gets neither, as in `sql!`. A psql meta-command or a
> `COPY ... FROM stdin` data block is refused like any other text the server
> would not run.
>
> The path is resolved against `CARGO_MANIFEST_DIR`, the directory holding the
> calling crate's `Cargo.toml`, as sqlx's `migrate!` resolves its own, and not
> against the calling file as `include_str!` is. Cargo sets that variable for
> every compilation and every tool that expands proc macros sets it too, so the
> same string names the same file from every module, from inside another
> macro, and in an editor. `proc_macro::Span::local_file`, which would give the
> calling file, is stable, but whether it answers depends on the host
> expanding the macro.
>
> The expansion MUST make the file an input of the calling crate's build. A
> proc macro's output is cached with the crate, and stable Rust gives a proc
> macro no way to declare a file it read, so an edited file would otherwise go
> on running, and digesting, its old text until something else forced a
> rebuild. The expansion therefore includes the file with `include_bytes!` in
> an unused constant: the compiler records it among the crate's inputs, an
> edit rebuilds the crate, and the macro reads, checks and digests the new
> text. The text that runs is the macro's own literal rather than the included
> bytes, so what is checked, what is digested and what runs come from one read
> of the file.

> [spec:pgorm:req:migration.sql-file.commits-after]
> A `SqlMigration` whose text opens with the line `-- pgorm: commits-after`
> (`MigrationFile::COMMITS_AFTER`), ended by a line break or by the text's
> end, MUST answer `true` from `commits_after()`, and any other text `false`
> (`migration.commit-after`). `SqlMigration::new` reads it from the text, so
> the macro's value and one built by hand from the same text agree. The
> directive is a comment, which PostgreSQL ignores, and part of the text the
> checksum digests (`migration.sql-file.checksum`), so adding or removing it
> in an applied file reads as the edit it is. It is how a change PostgreSQL
> will not let one transaction finish runs from the files it was frozen into
> (`migration.frozen`).

> [spec:pgorm:req:migration.sql-file.read]
> `SqlMigration::read(path)` MUST read the `.sql` file at `path` as the
> program runs into the migration `sql_migration!` would make of it: named for
> the path's file stem, its text the file's bytes as UTF-8 with every CRLF line
> ending written as LF, so it reports the checksum
> (`migration.sql-file.checksum`) and the `commits_after()`
> (`migration.sql-file.commits-after`) the macro's value of the same file
> reports, and a ledger written by either verifies against the other. A path
> naming no file stem MUST be `io::ErrorKind::InvalidInput` and text that is
> not UTF-8 `io::ErrorKind::InvalidData`, and a file that cannot be read is
> the I/O error it is. The text MUST be held to PostgreSQL's grammar through
> `pgorm-sql-check`'s `check_sql`, the check `sql_migration!` makes
> (`migration.sql-file.compile`), syntax only, so a file the server would
> refuse to parse is refused as it is read, before any batch runs, as
> `io::ErrorKind::InvalidData` naming the file and carrying the parser's
> diagnostic. `SqlMigration::read_dir(directory)` MUST read every
> regular file directly inside `directory` whose extension is `sql`, in the
> order of their file names compared as text, which is the order the
> `m<timestamp>_<description>` names `MigrationFile` freezes are written in
> (`migration.frozen`); other entries are passed over, and a file name that
> is not UTF-8 is `io::ErrorKind::InvalidData`.

## A migrator built at run time

> [spec:pgorm:def:migration.runtime]
> `Migrations` is a migrator built as the program runs: `Migrations::new`
> takes the migrations, `Box<dyn MigrationTrait>`s in the order they apply,
> and `with_migration_table_name(name)`, `with_migration_table_schema(schema)`
> and `with_statement_cache_channel(channel)` set what
> `migration_table_name()`, `migration_table_schema()` and
> `statement_cache_channel()` answer, each answering as a migrator
> overriding nothing does unless set. It implements `MigratorTrait`, so `up`,
> `status`, `install` and every accessor are the one runner's
> (`migration.runner`, `migration.up`), with every rule of this document
> holding for it as for a migrator declared as a type: the name check, the
> lock, the ledger and its placement, checksums, `commits_after`, legacy
> adoption and the statement-cache clear and notification. It is what a
> program reading its migrations at run time, `SqlMigration::read_dir`'s among
> them, runs them with, and what pgorm's language bindings run theirs with
> (`bindings.migrations`).

## Generated DDL frozen into a migration file

> [spec:pgorm:def:migration.frozen]
> DDL generated from a declaration describes the declaration as it stands
> when it is generated, so a migration that generated it each time it ran
> would follow the declaration as it changed: a database built from the
> history would get today's tables from the first migration and the later
> migrations' changes to them again, and fail. pgorm's `MigrationFile` is
> generated DDL frozen as the text of a migration written as a SQL file,
> generated once, when the change is made. `MigrationFile::new(name,
> statements)` takes the file's stem, which is the migration's name, and any
> statements that display as SQL, and its text is each statement followed by
> `;` and a line break, in order; `committing_after()` opens the text with
> the `migration.sql-file.commits-after` directive, and `commits_after()`
> answers whether it does; `name()` and `text()` read it. `freeze(dir)`
> writes the text to `<dir>/<name>.sql`, creating the file only where none
> is there, and answers whether it wrote it: a file already there is left as
> it is, never compared and never rewritten, because it is the migration
> databases have run and recorded the checksum of, so the code that froze a
> change can stay and run again after the declaration has moved on. A name
> that is not a plain file stem — empty, `.` or `..`, or holding `/`, `\` or
> a NUL — MUST be refused as `io::ErrorKind::InvalidInput` before anything is
> written, and a directory that does not exist is the I/O error it is.
> pgorm-migration's prelude exports `MigrationFile`, and `sql_migration!`
> runs the files it writes (`migration.sql-file`).

## Adopting a ledger left under the inherited name

> [spec:pgorm:req:migration.ledger-upgrade+2]
> Renaming the default ledger table cannot be done by changing the default. A
> deployment upgraded to the renamed crate would look at `pgorm_migrations`,
> find nothing, and read its entire applied history as pending — then re-run
> every migration against a schema that already has them, failing on the first
> `CREATE TABLE` if the operator is lucky and corrupting data if they are not.
> The rename is therefore paired with detection: `install` MUST adopt a ledger
> found under `LEGACY_LEDGER_TABLE` before it creates one under
> `migration_table_name()`.
>
> Adoption is `ALTER TABLE ... RENAME TO`, not a copy. Every row is preserved
> because none is moved, and at no instant do two ledgers exist that could
> disagree about what has been applied; a copy would have to decide what to do
> with the original, and every answer to that is worse. The primary key and
> indexes travel with the table under their original names, so an adopted ledger
> keeps a constraint called `seaql_migrations_pkey`. That is left alone
> deliberately: it is harmless, and it is the only remaining evidence of where
> the table came from.
>
> Three conditions gate it, all required. The ledger MUST be the default one:
> `migration_table_name()` the default name and `migration_table_schema()`
> `None`. A custom name or a schema is the caller's own placement, is not
> adopted from, and is never renamed, which also makes overriding the name back
> to `seaql_migrations` the supported way to decline the rename entirely. The
> legacy table is looked for, and renamed, where the unqualified name resolves,
> because that is where an earlier release left it. The default name MUST be
> absent — a database holding both keeps both, the ledger in use wins, and the
> legacy table is left untouched rather than read or destroyed. The legacy name
> MUST be present, or there is nothing to adopt. All three are re-tested on
> every `install`, which makes adoption idempotent: it happens once, and every
> later call is a single catalog read that answers no.
>
> That catalog read is the fast path and MUST come first, because the guarded
> rename below serialises and the steady state — fresh databases and adopted
> ones alike — has nothing to serialise. When it does report an adoptable
> ledger, the rename is issued as one `DO` block that re-tests the condition
> server-side under `pg_advisory_xact_lock(lock_key())`. A `DO` block is a
> single statement and so a single implicit transaction, which is exactly the
> extent a transaction-scoped lock needs; `install` is reachable in autocommit,
> as a caller invoking it directly does, as readily as from inside `up`'s
> transaction, and a caller already holding the lock takes it re-entrantly.
>
> The lock orders adopters but does not settle the guard, and the block MUST
> also catch `undefined_table` and `duplicate_table` from the rename itself.
> `to_regclass` resolves a name without taking a relation lock, so it never
> processes the invalidation messages a concurrent adopter's commit sent and can
> answer from a catalog snapshot older than that commit. The `ALTER` then
> re-resolves the name while acquiring its own lock and finds the source gone.
> Both that and its mirror — the target already present — are the race resolving
> in our favour, so they are caught and the block does nothing; the handler's
> subtransaction is what keeps them off the caller's transaction, which inside
> `up` is carrying the whole migration batch. Anything else propagates.
>
> Adoption is `install`'s alone, so a read never performs it — but a read MUST
> still answer honestly on a database that has not yet been adopted. A ledger
> found only under `LEGACY_LEDGER_TABLE` MUST therefore be *read where it
> stands*: reporting it as absent would reproduce, in the read path, the exact
> failure this requirement exists to prevent — an upgraded deployment's entire
> applied history read as pending — and would do so on the very call an operator
> makes to decide whether the upgrade is safe. The read is gated on the same
> first condition as the rename: the ledger MUST be the default one, so a
> caller who named or placed their own ledger is never answered from a table
> that is not theirs. Nor does the read need the second condition, because it resolves
> the two names in order — the ledger in use still wins, and a legacy table
> beside it is still neither read nor touched. The consequence is that `status`
> gives the same answer before and after adoption, and only `up` moves the
> table.

## Asking is not changing

> [spec:pgorm:req:migration.read-only+1]
> Checking migration status MUST write nothing. `status`,
> `get_migration_models`, `get_migration_with_status`, `get_pending_migrations`
> and `get_applied_migrations` are named for reads and are exactly what a
> readiness probe, a deploy gate, or an operator at a prompt reaches for; if
> they provision, then asking the question requires the privileges to change
> the schema, and every such call takes an `ACCESS EXCLUSIVE` lock on the
> ledger and can rename a table. `install` is the one method that provisions,
> and `up` — through `exec_up`, which calls it explicitly under the advisory
> lock — is the one method that calls it. A caller who wants the ledger created
> without applying anything still has `install` itself.
>
> A read therefore *locates* the ledger instead of creating it, from catalogue
> reads alone: the configured table — `migration_table_name()`, qualified by
> `migration_table_schema()` when that is set — if it exists, else
> `LEGACY_LEDGER_TABLE` under the gate `migration.ledger-upgrade` sets, else
> nothing. Resolution is one `to_regclass` over both candidates, which takes no
> relation lock, and each identifier is passed through `quote_ident` so the
> lookup agrees with the quoted form the crate renders everywhere else rather
> than case-folding a mixed-case ledger away. A configured schema qualifies the
> lookup, so the ledger is found in that schema or not at all, never in another
> schema on `search_path`; the checksum column is looked for in the same
> schema, or in the current schema for an unqualified ledger. Where the located
> table lacks the column, the projection substitutes a bound `NULL` per
> `migration.checksum`.
>
> No ledger is not an error and not an empty answer about a missing table: it
> means nothing has been applied, so `get_migration_models` yields no rows and
> every migration reads as `Pending`. That is the same answer the old
> install-then-read path produced on a fresh database, arrived at without the
> `CREATE TABLE` — which is the point: the observable status is unchanged, only
> the side effect is gone.
>
> The split is not merely in `install`'s call sites. `ReadableLedger` is the
> read path's own type, carrying the located name and whether it has the
> checksum column, and it is private: the read path is a smaller surface than
> the write path rather than a second public way to provision one. A migrator
> overriding `adopt_legacy_ledger` changes what `install` adopts, not what a
> read resolves; the two answer the same question by the same three gates, and
> a migrator that needs them to diverge overrides `get_migration_models`.
