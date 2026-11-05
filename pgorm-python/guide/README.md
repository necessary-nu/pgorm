# The pgorm Python guide

pgorm is an async ORM for PostgreSQL. The `pgorm` Python package is pgorm's
Rust library loaded into a Python process, on asyncio. This guide teaches it
from installing the package and running a first query to every feature it
has. Every example in it is a whole program that the package's tests run
against a real PostgreSQL server, so what the guide shows is what pgorm does.

This guide is the Python dialect of [the Rust guide](../../docs/guide/README.md),
which is the main one. It has the Rust guide's chapters, in the same order and
under the same names, each teaching the same features in Python. Where Python
differs from Rust in shape, spelling or reach, the chapter says so at that
point, and a feature Python does not have yet is named as missing where the
Rust guide teaches it. A few chapters, marked *Python only*, cover what only
Python needs: asyncio, registering Rust entities, typing and packaging.

The examples share one schema, a small chain of bakeries, which [Getting
started](01-getting-started.md) introduces and the module
[`bakery.py`](bakery.py) beside this guide declares.

| Chapter | |
| --- | --- |
| 1. [Getting started](01-getting-started.md) | Install the package, connect, and run a first query. |
| 2. [Python and asyncio](02-asyncio.md) | *Python only.* Event loops, statements side by side, cancellation and closing. |
| 3. [Models and entities](03-entities.md) | Declare tables as models, and what each declaration says. |
| 4. [Writing rows](04-writing.md) | Insert, update, delete and upsert rows, and what each write returns. |
| 5. [Reading rows](05-reading.md) | Find rows: filters, projections, joins, grouping and ordering. |
| 6. [Pages, cursors and streams](06-pagination.md) | Read in pages, by keyset cursor, or as a stream. |
| 7. [Relations](07-relations.md) | Follow relations: related rows, batched loading and the graph. |
| 8. [Transactions](08-transactions.md) | Transactions and savepoints, isolation levels and retries. |
| 9. [Errors](09-errors.md) | What can fail, and how to tell one failure from another. |
| 10. [Raw SQL](10-raw-sql.md) | Run SQL you wrote, with its values bound. |
| 11. [The query builder](11-query-builder.md) | Build any statement with pgorm-query's builders. |
| 12. [The pipeline](12-pipeline.md) | Compose queries as a pipeline of relation transforms. |
| 13. [Values and types](13-values.md) | How Python values map to PostgreSQL types, from dates to ranges. |
| 14. [Schema generation](14-schema.md) | Generate DDL, and the DDL builders. |
| 15. [Disjoint enums](15-disjoint.md) | Store an enum as a base table and a table per kind. |
| 16. [One-of references](16-one-of.md) | Reference one of several tables. |
| 17. [COPY](17-copy.md) | Move rows in and out in bulk with `COPY`. |
| 18. [LISTEN and NOTIFY](18-listen-notify.md) | Receive notifications with `LISTEN`, and send them. |
| 19. [Statement caches and DDL](19-statement-caches.md) | Prepared statements, and clearing them after DDL. |
| 20. [Connection configuration and TLS](20-configuration.md) | Size the pool, set its timeouts, and connect over TLS. |
| 21. [Metrics](21-metrics.md) | Observe the queries a program runs. |
| 22. [Migrations](22-migrations.md) | Evolve the schema. |
| 23. [Generating entities](23-codegen.md) | Generate declarations from a schema. |
| 24. [Registering Rust entities](24-registrations.md) | *Python only.* Use a Rust crate's entities, graphs and hooks from Python. |
| 25. [Types and mypy](25-typing.md) | *Python only.* The package's types, checked with mypy. |
| 26. [PostgreSQL 18 and 19](26-postgres-releases.md) | Target PostgreSQL 18, or 19 with the `pg-19` feature. |
| 27. [Python and JavaScript](27-bindings.md) | How the Python package relates to Rust and to JavaScript. |
| 28. [Packaging and distribution](28-distribution.md) | *Python only.* Wheels, source distributions, platforms and notices. |

The reference beside the guide is the package's type stubs, the `.pyi` files
under [`python/pgorm`](../python/pgorm): they give each function's and
method's signature. The guide says what the items are for and how they fit
together. The rules the package keeps are specified in
[`docs/spec/python.md`](../../docs/spec/python.md) and, for what both
bindings share, [`docs/spec/bindings.md`](../../docs/spec/bindings.md).
