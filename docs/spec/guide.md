# User guide

pgorm's user documentation is one guide, written for Rust and kept in
`docs/guide/`. It takes a new user from adding the dependency through every
feature pgorm has. The Python and JavaScript guides are dialects of it: the
same chapters in their own language. The API reference (rustdoc, the Python
stubs, the TypeScript declarations) says what each item does; the guide says
what the items are for and how they fit together.

## The Rust guide

> [spec:pgorm:req:guide.rust]
> The Rust guide MUST teach pgorm's features chapter by chapter, from adding
> the dependency and running a first query to each of the following, with no
> feature left to the API reference alone: connecting and the pool, entities
> and every derive pgorm exports, writing rows (insert, update, delete,
> upsert, `MERGE`), reading rows (filters, projections, joins, ordering,
> pagination, cursors, streaming), relations and the relational-read graph,
> transactions, raw SQL with `sql!` and `prql!`, the pgorm-query builder, the
> pipeline, the value and type vocabulary, schema generation and the DDL
> builders, disjoint enums, one-of references, `COPY`, `LISTEN` and `NOTIFY`,
> statement caches, errors, connection configuration and TLS, metrics,
> migrations, code generation, the PostgreSQL release a build targets, and
> the language bindings. A feature pgorm adds MUST arrive in the guide with
> the change that adds it.
>
> The guide is user documentation. It explains what a thing is for before how
> to call it, and states only what the code does. Each chapter is a markdown
> file under `docs/guide/` that pgorm includes as a rustdoc module, the
> chapter's module under `pgorm::guide`, so the guide renders with the API it
> describes and its links to that API are checked when the documentation is
> built. The guide's index is `docs/guide/README.md`, the `pgorm::guide`
> module's own page. The root README and the crate root's documentation point
> to it, and so do the READMEs of the crates a chapter covers.

## Examples

> [spec:pgorm:req:guide.examples]
> Every code example in the Rust guide MUST compile and run as a doctest of
> pgorm, and MUST assert what it shows: an example that builds a statement
> asserts the SQL it renders, and one that reads or writes asserts what it
> read or wrote. An example that needs a database runs against the server
> `DATABASE_URL` names, in a scratch database of its own named as
> `testing.isolation` requires and dropped when it finishes. The shared
> schema, the bakery, and the scratch databases come from the unpublished
> crate `pgorm-guide-support`, a dev-dependency of pgorm, through hidden
> lines, so the reader sees only the code the chapter teaches. Each chapter's
> examples share one running domain rather than unrelated fragments.
>
> No example is marked `ignore`. An example is marked `no_run` only where
> running it is impossible in a doctest, such as one needing a TLS server or a
> second process, and the guide's tests hold the list of such examples, each
> with its reason.

## Features

> [spec:pgorm:req:guide.features]
> Every Cargo feature pgorm declares MUST change the build: it turns on a
> dependency's feature or an optional dependency, or a `cfg` in pgorm's
> sources reads it. A feature that does neither is a name a program can turn
> on and see nothing happen, and is deleted rather than kept for a meaning it
> might be given. The getting-started chapter's feature table MUST list every
> feature a program turns on, with its default, which is all of them but
> `tests-cfg`: that one is pgorm's own test scaffolding, hidden from its
> documentation.
>
> Three were deleted under this rule: `runtime-tokio`, on by default, and
> `pgorm-internal`, which nothing read, and `debug-print`, which gated only a
> `debug_print!` macro pgorm exported and never called.

## Chapters

> [spec:pgorm:req:guide.chapters]
> `docs/guide/chapters.json` lists the guide's chapters in reading order, each
> with an `id`, a `title` and a `file` under `docs/guide/`. It is the list the
> Rust guide and its dialects are held to. Each listed file MUST exist and
> open with a level-one heading equal to the chapter's title, the
> `pgorm::guide` module MUST include each file, in the listed order, as a
> submodule named for the id with `-` written `_`, and no other file under
> `docs/guide/` is a chapter.

## Dialects

> [spec:pgorm:req:guide.dialects]
> The Python and JavaScript guides MUST follow the Rust guide's chapters one
> for one, in the order `docs/guide/chapters.json` lists them, each chapter
> under the same id and teaching the same features in its own language, with
> its examples run as its language's tests run them. Where a binding differs
> from Rust in shape, spelling or reach, its chapter MUST say so at the point
> the difference arises. A dialect adds only what its language needs, such as
> installing the package, the async runtime, typing and packaging, and leaves
> nothing out but a feature its binding does not have, which it names.
