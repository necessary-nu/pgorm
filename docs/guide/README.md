# The pgorm guide

pgorm is an async ORM for PostgreSQL, built on tokio-postgres and deadpool.
This guide teaches it from adding the dependency and running a first query to
every feature it has. Each chapter is a page of this module, and every example
in it runs as a test against a real PostgreSQL server, so what the guide shows
is what pgorm does.

The examples share one schema, a small chain of bakeries, which [Getting
started](self::getting_started) introduces and [Entities](self::entities)
declares. The API reference beside the guide says what each item does; the
guide says what the items are for and how they fit together.

| Chapter | |
| --- | --- |
| 1. [Getting started](self::getting_started) | Add pgorm to a project, connect, and run a first query. |
| 2. [Entities](self::entities) | Declare tables as entities, and what each derive writes. |
| 3. [Writing rows](self::writing) | Insert, update, delete and upsert rows, and what each write returns. |
| 4. [Reading rows](self::reading) | Find rows: filters, projections, joins, grouping and ordering. |
| 5. [Pages, cursors and streams](self::pagination) | Read in pages, by keyset cursor, or as a stream. |
| 6. [Relations](self::relations) | Follow relations: related rows, batched loading and the graph. |
| 7. [Transactions](self::transactions) | Transactions and savepoints, isolation levels and retries. |
| 8. [Errors](self::errors) | What can fail, and how to tell one failure from another. |
| 9. [Raw SQL](self::raw_sql) | Run SQL you wrote, checked at compile time with `sql!` and `prql!`. |
| 10. [The query builder](self::query_builder) | Build any statement with pgorm-query. |
| 11. [The pipeline](self::pipeline) | Compose queries as a pipeline of relation transforms. |
| 12. [Values and types](self::values) | How Rust values map to PostgreSQL types, from dates to ranges. |
| 13. [Schema generation](self::schema) | Generate DDL from entities, and the DDL builders. |
| 14. [Disjoint enums](self::disjoint) | Store an enum as a base table and a table per kind. |
| 15. [One-of references](self::one_of) | Reference one of several entities. |
| 16. [COPY](self::copy) | Move rows in and out in bulk with `COPY`. |
| 17. [LISTEN and NOTIFY](self::listen_notify) | Receive notifications with `LISTEN`, and send them. |
| 18. [Statement caches and DDL](self::statement_caches) | Prepared statements, and clearing them after DDL. |
| 19. [Connection configuration and TLS](self::configuration) | Size the pool, set its timeouts, and connect over TLS. |
| 20. [Metrics](self::metrics) | Instrument the pool, and observe PostgreSQL itself. |
| 21. [Migrations](self::migrations) | Evolve the schema with pgorm-migration. |
| 22. [Generating entities](self::codegen) | Generate entities from a schema with pgorm-codegen. |
| 23. [PostgreSQL 18 and 19](self::postgres_releases) | Target PostgreSQL 18, or 19 with the `pg-19` feature. |
| 24. [Python and JavaScript](self::bindings) | pgorm from Python and from JavaScript. |

Porting an application from SeaORM? [Coming from
SeaORM](crate::coming_from_seaorm) maps each SeaORM name to what replaces it.

The Python and JavaScript guides follow this one chapter for chapter, in
their own language.
