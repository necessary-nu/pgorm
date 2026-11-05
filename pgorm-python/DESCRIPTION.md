# pgorm for Python

pgorm is an async ORM for PostgreSQL. This package is pgorm's Rust library
loaded into your Python process as a native extension, on asyncio. Queries run
in-process over PostgreSQL's own protocol, without HTTP or a query server.

The package has models declared in Python, the query builder and the
pipeline, async pools with verified TLS, transactions and savepoints,
streaming, `COPY`, `LISTEN` and `NOTIFY`, schema statements, and type stubs
checked with strict mypy. Applications can also compile their Rust entities
and graphs into their own wheel, with generated, typed Python models.

The [Python guide](https://github.com/necessary-nu/pgorm/tree/main/pgorm-python/guide)
covers installation, the supported platforms and every feature, with
examples that run as tests. Wheels use the CPython 3.14 ABI with the GIL.
Free-threaded Python and subinterpreters are not supported.
