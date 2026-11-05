# The pgorm guide for JavaScript

pgorm is an async ORM for PostgreSQL. The npm package `@necessary/pgorm` runs
it from Node.js and Deno, through a native addon that holds pgorm itself. This
guide teaches it from installing the package and running a first query to
every feature the package has.

It is the JavaScript dialect of [the Rust guide](../../docs/guide/README.md),
which is pgorm's main guide: the same chapters, in the same order, teaching
the same features in TypeScript. The examples share the Rust guide's schema, a
small chain of bakeries, which [Getting started](01-getting-started.md)
introduces and [Entities](03-entities.md) declares. The TypeScript
declarations under [`lib/`](../lib/index.d.ts) say what each export does; the
guide says what the exports are for and how they fit together.

## Reading this guide

- **Where JavaScript differs from Rust,** in shape, spelling or reach, a
  chapter says so at the point the difference arises, naming what Rust does.
- **A feature JavaScript cannot express yet** is named in a paragraph that
  begins *Not available in JavaScript yet*, saying what Rust has. The chapter
  shows the nearest thing JavaScript does have, if there is one.
- **Chapters of JavaScript's own,** which the Rust guide has no counterpart
  of, are marked *JavaScript only* below and say so in their first lines.
- **Every `ts` example runs,** in Node.js and in Deno, against a scratch
  database holding the bakery, and asserts what it shows. An example names
  the bakery's models and `pool`, a pool over its copy of the bakery, without
  declaring them; [Getting started](01-getting-started.md#the-bakery) says
  how. A TypeScript block fenced `typescript` rather than `ts` is a sketch
  that needs what the guide's tests do not have, such as a generated module,
  and says what checks the same code instead.

## Chapters

| Chapter | |
| --- | --- |
| 1. [Getting started](01-getting-started.md) | Install the package, connect, and run a first query. |
| 2. [Promises, cancellation and cleanup](02-promises.md) | *JavaScript only.* How every operation settles, stops early, and releases what it holds. |
| 3. [Entities](03-entities.md) | Declare tables as models, and what a declaration can say. |
| 4. [Writing rows](04-writing.md) | Insert, update, delete and upsert rows, and what each write returns. |
| 5. [Reading rows](05-reading.md) | Find rows: filters, projections, joins, grouping and ordering. |
| 6. [Pages, cursors and streams](06-pagination.md) | Read in pages, by keyset cursor, or as a stream. |
| 7. [Relations](07-relations.md) | Follow relations: related rows, batched loading and the graph. |
| 8. [Transactions](08-transactions.md) | Transactions and savepoints, isolation levels and access modes. |
| 9. [Errors](09-errors.md) | What can fail, and how to tell one failure from another. |
| 10. [Raw SQL](10-raw-sql.md) | Run SQL you wrote, its values bound as parameters. |
| 11. [The query builder](11-query-builder.md) | Build any statement with pgorm-query's builders. |
| 12. [The pipeline](12-pipeline.md) | Compose queries as a pipeline of relation transforms. |
| 13. [Values and types](13-values.md) | How JavaScript values map to PostgreSQL types, from dates to ranges. |
| 14. [Schema generation](14-schema.md) | Build DDL with the DDL builders. |
| 15. [Disjoint enums](15-disjoint.md) | Store a value of one of several kinds as a base table and a table per kind. |
| 16. [One-of references](16-one-of.md) | Reference one of several tables. |
| 17. [COPY](17-copy.md) | Move rows in and out in bulk with `COPY`. |
| 18. [LISTEN and NOTIFY](18-listen-notify.md) | Receive notifications with `LISTEN`, and send them. |
| 19. [Statement caches and DDL](19-statement-caches.md) | Prepared statements, and clearing them after DDL. |
| 20. [Connection configuration and TLS](20-configuration.md) | Size the pool, set its timeouts, and connect over TLS. |
| 21. [Metrics](21-metrics.md) | Observe what the program asks of PostgreSQL. |
| 22. [Migrations](22-migrations.md) | Evolve the schema. |
| 23. [Generating entities](23-codegen.md) | Generate declarations from a schema. |
| 24. [Registered Rust entities](24-registered-entities.md) | *JavaScript only.* Use a Rust application's entities, hooks included, from JavaScript. |
| 25. [Generating registrations](25-generating-registrations.md) | *JavaScript only.* Build an application's module and its TypeScript declarations. |
| 26. [TypeScript](26-typescript.md) | *JavaScript only.* How the declarations type records, writes and keys, and what they refuse. |
| 27. [PostgreSQL 18 and 19](27-postgres-releases.md) | The PostgreSQL release the package targets. |
| 28. [Python and JavaScript](28-bindings.md) | How this package relates to pgorm and to its Python binding. |

[DISTRIBUTION.md](../DISTRIBUTION.md) covers the npm packages themselves:
their platforms, what the addon contains, and how they are built and checked.
