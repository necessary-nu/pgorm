# pgorm-driver

pgorm's asynchronous PostgreSQL driver, maintained from tokio-postgres.
The upstream revision and import details are recorded in [UPSTREAM.md](UPSTREAM.md).

The crate retains tokio-postgres's PostgreSQL API and feature flags. Rust code
imports it as `pgorm_driver`. Its `types` module re-exports the shared
`postgres-types` codecs used by pgorm, rust_decimal and pgvector.

TLS through rustls is supplied by the sibling `pgorm-driver-rustls` crate.
Native Turso execution is not part of this import.

The original MIT and Apache-2.0 licenses and PostgreSQL documentation notice
are included in this directory.

The upstream integration suite is retained as the `upstream` module of the
crate's test binary, behind `upstream-tests`, because it needs a dedicated
PostgreSQL authentication fixture on port 5433. Its sources and setup files
are in `tests/upstream/`. Run it with
`cargo test -p pgorm-driver --features upstream-tests --test it -- upstream::`
after provisioning that fixture. pgorm's existing integration suite exercises
the driver with its normal test database.
