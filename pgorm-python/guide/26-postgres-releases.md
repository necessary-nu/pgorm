# PostgreSQL 18 and 19

pgorm targets PostgreSQL 18 or later, and nothing older. A wheel targets one
major release, 18 unless it was built for 19, and the release decides two
things: which statements the builders can make, and which answers the
documentation and the tests expect where the two releases' servers answer
one statement differently.

## The default: PostgreSQL 18

A wheel built with no feature named targets PostgreSQL 18. There is no
feature to say so, and no build reaches back to 17. pgorm uses what 18 added
wherever it fits. Among what this guide covers:

- *virtual generated columns*, computed when read rather than stored
  (`ColumnDef(...).generated(expression, "virtual")` in [Schema
  generation](14-schema.md));
- *temporal keys*, a primary or unique key `WITHOUT OVERLAPS` and a foreign
  key over a `PERIOD` (the `without_overlaps` and `period` options);
- `uuidv7()`, a time-ordered UUID a key can default to (`call("uuidv7")`);
- `RETURNING` the old and the new version of a written row
  ([Writing rows](04-writing.md));
- named `NOT NULL` constraints, and constraints declared `NOT ENFORCED`
  ([Schema generation](14-schema.md)).

```python
from pgorm import call, col
from pgorm.schema import ColumnDef, create_table

# When each of the bakery's ovens is booked, and for which batch.
oven_booking = (
    create_table("oven_booking")
    .column(ColumnDef("oven_id", "integer").not_null())
    .column(ColumnDef("during", "tstzrange").not_null())
    .column(ColumnDef("batch", "uuid").not_null().default(call("uuidv7")))
    .column(ColumnDef("trays", "integer").not_null())
    .column(ColumnDef("cakes", "integer").not_null().generated(col("trays") * 12, "virtual"))
    .primary_key("oven_id", name="pk-oven_booking", without_overlaps="during")
)

assert oven_booking.inspect().sql == (
    'CREATE TABLE "oven_booking" ( "oven_id" integer NOT NULL, "during" tstzrange NOT NULL, '
    '"batch" uuid NOT NULL DEFAULT UUIDV7(), "trays" integer NOT NULL, '
    '"cakes" integer NOT NULL GENERATED ALWAYS AS ("trays" * 12) VIRTUAL, '
    'CONSTRAINT "pk-oven_booking" PRIMARY KEY ("oven_id", "during" WITHOUT OVERLAPS) )'
)
```

Here Python differs from Rust. The Rust guide declares the table as an entity
and has pgorm write its `CREATE TABLE`. A model declared in Python holds no
DDL, so the table is written with `pgorm.schema`'s builders, and a model
over it declares `cakes` with `generated="always"`: an insert leaves it out,
and a write that sets it is refused. The statement is the one the Rust
entity gives. A temporal key over a
plain column beside the range, such as `oven_id` here, needs the
`btree_gist` extension in the database.

## Asking for PostgreSQL 19

The `pg-19` feature targets PostgreSQL 19. The package has one build option,
and this is it: build the wheel with the feature named, from the repository
root as [Getting started](01-getting-started.md) builds it:

```sh
.venv/bin/maturin build --manifest-path pgorm-python/Cargo.toml \
  --interpreter .venv/bin/python --out dist --locked --features pg-19
```

The feature passes through: pgorm-python forwards it to pgorm, which
forwards it to pgorm-query. An application wheel's registration crate
([Registering Rust entities](24-registrations.md)) depends on pgorm itself,
so it asks for 19 as a Rust crate does, with `features = ["pg-19"]` on that
dependency.

The feature only adds. What 19 alone accepts arrives behind it as new
functions, methods and options, and nothing an 18 build has changes shape or
meaning: a statement built the same way renders the same SQL under either
target. That matters for an application wheel, which is one Cargo build:
Cargo turns a feature on for every crate in a build once any one crate asks
for it, so a crate elsewhere in the registration crate's dependencies that
turns on `pg-19` cannot break the statements a Python program builds.

`pgorm.capabilities()` lists the installed build's features, and `pg-19` is
among them in a 19 build:

```python
import pgorm

features = pgorm.capabilities()["features"]
postgres_target = 19 if "pg-19" in features else 18
assert postgres_target in (18, 19)

# In a program that relies on 19:
# if postgres_target < 19:
#     raise RuntimeError("install a pgorm wheel built with its pg-19 feature")
```

Here Python differs from Rust, where the constant
`pgorm_query::POSTGRES_TARGET` says which release a build targets and a
program can refuse to compile without 19. A Python program has no compile
step, so it reads the features when it starts.

## Where the two releases answer differently

Some statements both releases accept, but answer differently. A builder
records only what the statement says, not what the server holds, so pgorm
renders the statement the same way for both. Three differences exist today,
and a Python program sees each as a different result or a different
`DatabaseError`:

- `ALTER CONSTRAINT .. ENFORCED` and `NOT ENFORCED`
  (`alter_table(table).alter_constraint(name, "enforced")`) reach a `CHECK`
  constraint from 19, and only a foreign key on 18, which refuses a `CHECK`
  with SQLSTATE `42809`;
- `SET EXPRESSION` (`alter_table(table).set_expression(column, expression)`)
  on a virtual generated column that a `CHECK` reads is refused on 18, with
  SQLSTATE `0A000`, and accepted on 19;
- `JSON_ARRAY` over a query of no rows (`json_array_query`) is `NULL` on 18
  and the empty array on 19.

Code that runs against both reads the last as `None` or a list:

```python
import asyncio
import os
from decimal import Decimal

import pgorm
from pgorm import Pool, json_array_query, select

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # The names of the cakes over 10.00: there are none.
        dear = select(Cake.col("name")).from_(Cake.table).where(Cake.col("price") > Decimal("10.00"))
        row = await pool.one(select(json_array_query(dear).as_("names")))

        if "pg-19" in pgorm.capabilities()["features"]:
            assert row["names"] == []
        else:
            assert row["names"] is None


asyncio.run(main())
```

pgorm's tests hold the answer of the release a build targets, so the
default wheel's tests run against a PostgreSQL 18 server, and a `pg-19`
wheel's against 19.

## The grammar stays at 18 for now

Every SQL parse pgorm makes uses PostgreSQL 18's grammar, whichever release
a build targets. The parser is libpg_query, which has no 19 grammar yet. In
a Python program, the pipeline reads the SQL it compiled with PostgreSQL's
lexer, to match the parameters it binds to the placeholders that remain. The
other parses the Rust guide names are Rust's: the `sql!` and `prql!` macros,
the DDL reader of Rust's code generation, the paginator's reading of raw
SQL, and query fingerprints in metrics. A registration crate that uses them
gets 18's grammar too, under `pg-19` as well. No builder makes syntax only 19
parses, since everything a builder renders has to parse.
