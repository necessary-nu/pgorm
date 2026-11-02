# Running Migrator CLI

Migrations are up-only: there is no `down`, `fresh`, `refresh` or `reset`.
Roll a mistake forward with a new migration.

Set `DATABASE_URL` to your PostgreSQL connection string before running.

- Apply all pending migrations
    ```sh
    cargo run
    ```
    ```sh
    cargo run -- up
    ```
- Apply the first 10 pending migrations
    ```sh
    cargo run -- up 10
    ```
- Check the status of all migrations
    ```sh
    cargo run -- status
    ```

# Writing a migration

A migration is either a Rust module or a `.sql` file, and `src/lib.rs` lists
both kinds in the order they apply. Name each one `m<YYYYMMDD>_<NNNNNN>_<what>`:
the name is the migration's identity in the ledger, so a file is never renamed
once it has been applied.

- In Rust, as `src/m20220101_000001_create_table.rs` is: a module with a
  `#[derive(MigrationName)]` struct whose `up` drives the transaction
  with the query builders, listed as `Box::new(m20220101_000001_create_table::Migration)`.
- In SQL, as `src/m20220101_000002_index_post_title.sql` is: a file of
  statements, listed as `Box::new(sql_migration!("src/m20220101_000002_index_post_title.sql"))`
  with its path relative to this crate's `Cargo.toml`. The file is checked
  against PostgreSQL's grammar when the crate compiles, and the checksum of its
  text is recorded when it is applied, so editing it afterwards is reported
  rather than ignored.
