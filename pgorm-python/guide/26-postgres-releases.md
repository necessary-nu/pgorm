# PostgreSQL 19

pgorm targets PostgreSQL 19, and nothing older. No feature names a release:
every wheel makes the statements 19 accepts, the documentation gives 19's
answers, and pgorm's own tests run against a 19 server. An application whose
server runs an older release moves it to 19 before it moves to this pgorm.

## What pgorm uses

pgorm uses what recent releases added wherever it fits. Among what this guide
covers, from PostgreSQL 18:

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

## Coming from PostgreSQL 18

Some statements 18 and 19 both accept, but answer differently. A builder
records only what the statement says, not what the server holds, so the
statement renders as it did, and the server's answer is 19's, which a Python
program sees as a result or a `DatabaseError`:

- `ALTER CONSTRAINT .. ENFORCED` and `NOT ENFORCED`
  (`alter_table(table).alter_constraint(name, "enforced")`) reach a `CHECK`
  constraint as they reach a foreign key, `ENFORCED` checking the rows already
  there, where 18 refused a `CHECK` with SQLSTATE `42809`;
- `SET EXPRESSION` (`alter_table(table).set_expression(column, expression)`)
  on a virtual generated column that a `CHECK` reads checks the rows against
  the `CHECK`, where 18 refused it with SQLSTATE `0A000`;
- `JSON_ARRAY` over a query of no rows (`json_array_query`) is the empty
  array, where 18 answered `NULL`.

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool, json_array_query, select

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # The names of the cakes over 10.00: there are none.
        dear = select(Cake.col("name")).from_(Cake.table).where(Cake.col("price") > Decimal("10.00"))
        row = await pool.one(select(json_array_query(dear).as_("names")))
        assert row["names"] == []


asyncio.run(main())
```

## The grammar is PostgreSQL 19's

Every SQL parse pgorm makes uses one parser, pgsql-syntax, a Rust port of
PostgreSQL 19's own scanner and parser. `sql()` and `prql()`
([Raw SQL](10-raw-sql.md)) check text with it, through the code Rust's `sql!`
and `prql!` run; `pgorm.codegen.models_from_sql`
([Generating entities](23-codegen.md)) reads DDL with it; the pipeline reads
the SQL it compiled with its lexer, to match the parameters it binds to the
placeholders that remain; and the paginator's reading of raw SQL and query
fingerprints in [Metrics](21-metrics.md) parse with it, as a registration
crate's `sql!` and `prql!` do. A check of text is a check against 19's
grammar, so what 19 added passes it:

```python
from pgorm import sql

# REPACK and ON CONFLICT .. DO SELECT are PostgreSQL 19's.
assert sql("REPACK cake") == "REPACK cake"
upsert = "INSERT INTO cake (id) VALUES (1) ON CONFLICT (id) DO SELECT RETURNING *"
assert sql(upsert) == upsert
```

19's grammar also refuses a little that 18's parsed. It makes `ignore` and
`respect` keywords that a column label needs `AS` before, so a bare
`SELECT 1 ignore`, which an 18 server ran, is refused; `SELECT 1 AS ignore`
parses. pgorm writes `AS` before every column label it renders, so no
statement a builder makes is affected. The five keywords 19 adds, `ignore`,
`lsn`, `repack`, `respect` and `wait`, are all unreserved, so pgorm quotes the
identifiers it quoted before.
