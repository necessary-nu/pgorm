# pgorm-query

A dynamic query builder for PostgreSQL, and nothing else. This is pgorm's
in-tree fork of [sea-query](https://github.com/SeaQL/sea-query) with every
other backend removed and the PostgreSQL surface hardened.

## What it does

Construct expressions, queries, and schema DDL as abstract syntax trees using
an ergonomic builder API, then render them to SQL:

```rust
use pgorm_query::{Query, Expr, QueryStatementBuilder, tests_cfg::Char};

let query = Query::select()
    .column(Char::Character)
    .from(Char::Table)
    .and_where(Expr::col(Char::SizeW).eq(3))
    .take();

// Value-inlined SQL via Display:
let sql = query.to_string();

// Parameterized SQL plus its values, for execution:
let (sql, values) = query.build();
```

## How it differs from sea-query

- **PostgreSQL only.** One `QueryBuilder`, no backend abstraction: statements
  render through `Display`, and the `build_any`/`to_string_any` indirection is
  gone along with every MySQL/SQLite spelling.
- **Invalid SQL doesn't construct.** The AST leans on the type system so that
  statements PostgreSQL would reject fail to compile instead of rendering:
  `OVER` attaches only to function calls, DML targets are table names rather
  than subqueries, DDL statements name their targets and carry their required
  parts by construction, `ON CONFLICT` is a closed enum, and empty collections
  that would render as syntax errors are unrepresentable.
- **Grammar-checked by the real parser.** The test suite feeds every render
  through [libpg_query](https://github.com/pganalyze/libpg_query) — the
  PostgreSQL server's own parser — so the rendered SQL is held to the actual
  grammar, not to string expectations. Known-invalid renders are pinned and
  documented until their fix lands.
- **No render panics.** Anything the renderer would have to refuse at runtime
  is either unconstructible or valid.

## Use through pgorm

This crate is developed and versioned as part of
[pgorm](https://github.com/necessary-nu/pgorm), which re-exports what you need.
It works standalone, but its API tracks pgorm's needs rather than upstream
sea-query's.
