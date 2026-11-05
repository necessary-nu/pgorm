# pgorm

pgorm is an async ORM for PostgreSQL, and for PostgreSQL alone. You declare
each table as a Rust type, and pgorm reads and writes it with SQL it builds
for you: every value bound as a parameter, never pasted into the SQL, and
each statement prepared and cached per connection. Where the types can say
so, a statement PostgreSQL would refuse does not compile. It runs on [tokio-postgres](https://docs.rs/tokio-postgres) and a
[deadpool](https://docs.rs/deadpool) pool, and targets PostgreSQL 18 (or 19,
with the `pg-19` feature).

Because it serves one database, pgorm offers what PostgreSQL has: `RETURNING`
on every write, `ON CONFLICT` and `MERGE`, `COPY` in and out, `LISTEN` and
`NOTIFY`, enums, domains, arrays, JSON, ranges and multiranges, and
PostgreSQL 18's virtual generated columns and temporal keys. Beside the ORM
it has a typed SQL builder, a PRQL-shaped pipeline for composing queries,
compile-time checking of raw SQL against PostgreSQL's own grammar, entities
stored as a base table plus a table per kind, an opt-in metrics layer, a
forward-only migration runner and an entity generator. Python and JavaScript
bindings wrap the same crate.

## Documentation

- **[The guide](docs/guide/README.md)** teaches pgorm from a first query to
  every feature, and every example in it runs as a test. It is also the
  `pgorm::guide` module of the API documentation: run `cargo doc --open` and
  follow `guide`, where the examples read without their test scaffolding and
  every link resolves.
- [Coming from SeaORM](COMING_FROM_SEAORM.md) maps each SeaORM API to what
  replaces it, for porting an application. pgorm began as a fork of SeaORM
  and keeps its shape: entities, models, active models and relations.
- The API reference is `cargo doc`.

```toml
[dependencies]
pgorm = { git = "https://github.com/necessary-nu/pgorm" }
```

pgorm depends on forks of two crates, pinned to a git revision, so it is not
published on crates.io; depend on it by git.

## The repository

| Crate | What it is |
| --- | --- |
| `pgorm` | The ORM |
| `pgorm-query` | The SQL builder, re-exported as `pgorm::pgorm_query` |
| `pgorm-macros`, `pgorm-sql-macro` | The derives, and the `sql!` and `prql!` macros |
| `pgorm-pool` | The connection pool |
| `pgorm-migration` | The migration runner ([guide](docs/guide/21-migrations.md)) |
| `pgorm-codegen` | Entity generation from DDL ([guide](docs/guide/22-codegen.md)) |
| `pgorm-python` | The Python package `pgorm` |
| `pgorm-napi` | The npm package `@necessary/pgorm`, for Node.js and Deno |

## License

Licensed under either of

-   Apache License, Version 2.0
    ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
-   MIT license
    ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
