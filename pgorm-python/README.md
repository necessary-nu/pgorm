# pgorm for Python

The `pgorm` package is pgorm, the async PostgreSQL ORM, for Python. It loads
pgorm's Rust library into your Python process as a native extension and runs
on asyncio: queries, models, transactions, the pipeline, `COPY` and
`LISTEN`/`NOTIFY` are pgorm's own, rendered and executed by the Rust library
over PostgreSQL's protocol, with no HTTP service or query server in between.

**[The Python guide](guide/README.md)** teaches the package from a first
query to every feature, and every example in it runs as a test. It is the
Python dialect of [the Rust guide](../docs/guide/README.md).

## Installing

The package is not published to a registry yet. Build its wheel from a
checkout of this repository with maturin and install it, using CPython 3.14:

```sh
uv venv --python 3.14 .venv
uv pip install --python .venv/bin/python 'maturin>=1.15,<2'
.venv/bin/maturin build --manifest-path pgorm-python/Cargo.toml \
  --interpreter .venv/bin/python --out dist --locked
uv pip install --python .venv/bin/python dist/pgorm-*.whl
```

Free-threaded Python and subinterpreters are not supported. [Packaging and
distribution](guide/28-distribution.md) covers source distributions,
platforms and the notices the wheel ships.

## A first query

This program reads the gluten-free cakes of the guide's bakery, whose tables
[`guide/bakery.py`](guide/bakery.py) creates:

```python
import asyncio
import os

from pgorm import Column, Model, Pool

Cake = Model("cake", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "gluten_free": Column("bool"),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cakes = await (
            Cake.find()
            .where(Cake.col("gluten_free").eq(True))
            .order_by(Cake.col("name").asc())
            .all(pool)
        )
        names = [cake["name"] for cake in cakes]
        assert names == ["Chocolate Torte", "Lemon Drizzle", "Vanilla Sponge"]


asyncio.run(main())
```

[Getting started](guide/01-getting-started.md) goes on from here.

## Maintainers

pgorm-python is a Cargo workspace of its own, so building the repository's
root workspace never loads PyO3 or Python. The checks that build and verify
the package, and how to run them, are described in
[checks/README.md](checks/README.md).
