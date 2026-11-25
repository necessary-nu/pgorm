# pgorm-pool

pgorm's connection pool: a vendored fork of
[deadpool-postgres](https://crates.io/crates/deadpool-postgres), the
[`deadpool`](https://crates.io/crates/deadpool) manager for
[`pgorm-driver`](../crates/pgorm-driver). It pools
connections and provides a statement cache by wrapping
`pgorm_driver::Client` and `pgorm_driver::Transaction`, so repeated
SQL is prepared once per connection and reused.

Most applications don't touch this crate directly: `pgorm::connect` builds
the pool, and `pgorm::DatabasePool` wraps it. It is vendored rather than
depended on so pgorm can shape the pool surface to its own needs — the
fork tracks pgorm's requirements, not upstream deadpool's release cadence.

## Differences from upstream deadpool-postgres

- Tokio only: the `async-std` runtime support and its feature flags are gone.
- Native targets only: the wasm32 target support is gone.
- The pool does not implement pgorm's `Connection` — you must `get()` a
  connection first, so pool acquisition is always explicit.

## Example

```rust,ignore
use pgorm_pool::{Config, Runtime};
use pgorm_driver::NoTls;

let mut cfg = Config::new();
cfg.dbname = Some("app".to_string());
let pool = cfg.create_pool(Some(Runtime::Tokio1), NoTls)?;
let client = pool.get().await?;
let rows = client.query("SELECT 1", &[]).await?;
```
