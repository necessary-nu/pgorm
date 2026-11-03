# pgorm for Python

Compose native pgorm SQL builders in Python and execute them directly against
PostgreSQL through an optional PyO3 extension. Query execution runs in-process
and uses PostgreSQL's native protocol, without HTTP.

The package includes runtime schema and CRUD builders, async pools, verified
TLS, transactions and savepoints, streaming, pipelines, and typing information.
Applications can optionally compile Rust entity and graph registrations into
their own wheel with generated concrete Python types.

See the [Python documentation](https://github.com/necessary-nu/pgorm/tree/main/pgorm-python)
for installation, the support matrix and runnable application examples. Wheels
use the CPython 3.14 ABI with the GIL. Free-threaded Python and subinterpreters
are not supported.
