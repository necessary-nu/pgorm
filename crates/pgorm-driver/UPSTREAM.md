# Upstream provenance

- Repository: https://github.com/rust-postgres/rust-postgres
- Revision: `6c161e3226c0ca0f18f665610cbeb09862fe2f4b`
- Source directory: `tokio-postgres`
- Upstream package version: `0.7.18`
- Imported: 2026-11-24

This is maintained source in the pgorm workspace. The package and Rust crate
names were changed to `pgorm-driver` and `pgorm_driver`.
The initial import changes dependency wiring and documentation, not PostgreSQL
execution behaviour. Upstream licenses and attribution are retained. The repository-root `THIRD_PARTY`
notice is preserved as `LICENSE-POSTGRESQL`, and its `docker/` authentication
fixtures are preserved under `tests/upstream/`.

For an update, compare the upstream source directory at this revision with the
chosen new revision, apply the relevant changes and tests here, and update this
record after validation. Keep upstream changes separate from native Turso work.

The upstream integration suite is retained as the `upstream` test target,
behind `upstream-tests`, because it needs a dedicated PostgreSQL authentication
fixture on port 5433. Its setup files are in `tests/upstream/`. Run it with
`cargo test -p pgorm-driver --features upstream-tests --test upstream`
after provisioning that fixture. pgorm's existing integration suite exercises
the driver with its normal test database.
