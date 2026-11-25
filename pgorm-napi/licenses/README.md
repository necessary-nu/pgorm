# Supplemental dependency license evidence

`supplemental.json` records exact source URLs and SHA-256 hashes for files
omitted from dependency archives. Upstream revisions come from each published
crate's `.cargo_vcs_info.json`; bundled libpg_query files come from the gitlink
in pg_query 18.1.0's published revision.

Two dependencies declare licenses without including license text, in either
the archive or that upstream revision:

- `assoc` 0.1.3 declares MIT. `assoc-Cargo.toml` preserves its exact declaration
  and author metadata. `MIT-SPDX.txt` is the unmodified standard MIT text from
  SPDX license-list-data v3.27.0; its copyright placeholders are part of that
  reference text, not an attribution supplied by assoc or pgorm. No copyright
  year or holder has been invented.
- `pack1` 1.1.0 offers `Zlib OR Apache-2.0 OR MIT`. `pack1-Cargo.toml` preserves
  that declaration; the complete Apache-2.0 text supplies the Apache option.

`cfg_block` 0.1.1 includes its copyright and Apache-2.0 grant in the archive's
`LICENSE`, but declares `license-file` instead of an SPDX expression. The
notice generators retain that notice, narrowly classify this exact package,
and supplement it with the full Apache-2.0 text.

`pg_query` 18.1.0's Rust wrapper is MIT. Its supplements also preserve the
separate licenses for its bundled libpg_query, PostgreSQL, upb, and utf8_range
sources; the wrapper's declared license does not replace those notices.
