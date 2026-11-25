# pgorm

pgorm is an async ORM for PostgreSQL, with experimental support for Turso's
embedded engine. You declare each table as a Rust type, and pgorm
reads and writes it with SQL it builds
for you: every value bound as a parameter, never pasted into the SQL, and
each statement prepared and cached per connection. Where the types can say
so, a statement PostgreSQL would refuse does not compile. It runs on [pgorm-driver](crates/pgorm-driver), a vendored tokio-postgres fork, and a
[deadpool](https://docs.rs/deadpool) pool, and targets PostgreSQL 19.

On PostgreSQL, pgorm offers `RETURNING`
on every write, `ON CONFLICT` and `MERGE`, `COPY` in and out, `LISTEN` and
`NOTIFY`, enums, domains, arrays, JSON, ranges and multiranges, and
PostgreSQL 18's virtual generated columns and temporal keys. Beside the ORM
it has a typed SQL builder, a PRQL-shaped pipeline for composing queries,
compile-time checking of raw SQL against PostgreSQL's own grammar, entities
stored as a base table plus a table per kind, an opt-in metrics layer, a
forward-only migration runner and an entity generator. Python and JavaScript
bindings wrap the same crate.

Native Turso supports a smaller, evolving subset through the same driver and
pool APIs. The [compatibility matrix](docs/turso-compatibility.md) records the
pinned engine revision, supported operations and known differences.

## Documentation

- **[The guide](docs/guide/README.md)** teaches pgorm from a first query to
  every feature, and every example in it runs as a test. It is also the
  `pgorm::guide` module of the API documentation: run `cargo doc --open` and
  follow `guide`, where the examples read without their test scaffolding and
  every link resolves.
- [The Python guide](pgorm-python/guide/README.md) is its dialect for
  Python, chapter for chapter, every example run against a server.
- [The JavaScript guide](pgorm-napi/guide/README.md) is its dialect for
  Node.js and Deno, chapter for chapter, every example run in both runtimes.
- [Experimental native Turso](docs/turso.md) covers file and memory databases,
  package choices, compatibility, performance and engine upgrades.
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

To use an embedded database, enable the experimental feature:

```toml
[dependencies]
pgorm = { git = "https://github.com/necessary-nu/pgorm", features = ["turso"] }
```

Pass a file path or `:memory:` through the existing connection API:

```rust,no_run
use pgorm::ConnectionTrait;

async fn example() -> Result<(), Box<dyn std::error::Error>> {
    let pool = pgorm::connect("./app.db".parse()?);
    let connection = pool.get().await?;
    let row = connection.query_one("SELECT 1", &[]).await?;
    assert_eq!(row.get::<_, i32>(0), 1);
    Ok(())
}
```

Pool construction is lazy; the first acquisition opens the file. Connections
from one `:memory:` pool share its database, while separate memory pools are
isolated. PostgreSQL URLs and libpq connection strings keep their existing
meaning.

## The repository

| Crate | What it is |
| --- | --- |
| `pgorm` | The ORM |
| `pgorm-query` | The SQL builder, re-exported as `pgorm::pgorm_query` |
| `pgorm-macros`, `pgorm-sql-macro` | The derives, and the `sql!` and `prql!` macros |
| `pgorm-pool` | The connection pool |
| `pgorm-migration` | The migration runner ([guide](docs/guide/21-migrations.md)) |
| `pgorm-codegen` | Entity generation from DDL ([guide](docs/guide/22-codegen.md)) |
| `pgorm-python` | The Python package `pgorm` ([guide](pgorm-python/guide/README.md)) |
| `pgorm-napi` | The npm package `@necessary/pgorm`, for Node.js and Deno ([guide](pgorm-napi/guide/README.md)) |

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
