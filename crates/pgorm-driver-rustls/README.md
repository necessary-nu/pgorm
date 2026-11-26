# pgorm-driver-rustls

Rustls integration for `pgorm-driver`, maintained from tokio-postgres-rustls.
The upstream revision and import details are recorded in [UPSTREAM.md](UPSTREAM.md).

Use `pgorm_driver_rustls::MakeRustlsConnect` with `pgorm_driver::Config` or
pgorm's connection constructors. The connector uses the same driver crate as
pgorm so its TLS traits and connection types agree.

The original MIT license is included in this directory.
