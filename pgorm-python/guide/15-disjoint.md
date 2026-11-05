# Disjoint enums

Some things come in kinds, each kind with data of its own. The bakery takes
payments by card, in cash or with a gift voucher: every payment has an
amount, a card payment also has the card's last four digits, a voucher
payment the voucher's code, and a cash payment nothing more. In Rust that is
an enum. In PostgreSQL it is a *base* table holding what every payment has,
with a column saying which kind each row is, and a table for each kind that
carries data, keyed by the base row's key.

`pgorm.Disjoint` declares such a value over models, and pgorm writes the
tables and the constraints that hold them together, reads a row with its
kind's row in one statement, and writes both in one statement. The
constraints are PostgreSQL's own, with no triggers, so a state the
declaration cannot describe cannot be written, by pgorm or by anything else
connected to the database: a card payment without its card row, a card row
whose payment is cash, a payment with rows in two kinds' tables.

Here Python differs from Rust. Rust declares the enum with the `Disjoint`
derive over entities, and its variants are Rust types. Python declares it
over models, and a kind is its name, a string. The statements are the same:
both are pgorm's one description of the shape, so a Python declaration sends
what the Rust declaration of the same tables sends.

## Declaring one

The base is an ordinary model whose `kind` field holds the kind, an enum
column. Each kind with data is an ordinary model too, keyed by the same types
as the base, whose key the database does not fill, since its key is its base
row's. The mapping lists the kinds in order: a kind mapped to a model is a
kind with a table, and one mapped to `None` is a kind without one.

```python
from pgorm import Column, ConstructionError, Disjoint, Model, TypeName

KIND = TypeName("payment_kind", schema="public")

Payment = Model("payment", {
    "id": Column("i32", primary_key=True, generated="by_default"),
    "kind": Column(KIND, values=["card", "cash", "voucher"]),
    "order_id": Column("i32"),
    "amount": Column("decimal"),
})
CardPayment = Model("card_payment", {
    "payment_id": Column("i32", primary_key=True),
    "last_four": Column("text"),
})
VoucherPayment = Model("voucher_payment", {
    "payment_id": Column("i32", primary_key=True),
    "code": Column("text"),
})

Payments = Disjoint(
    "payment",
    Payment,
    {"card": CardPayment, "cash": None, "voucher": VoucherPayment},
    schema_name="public",
)

assert Payments.describe()["kind_type"] == {"schema": "public", "name": "payment_kind"}
assert list(Payments.kinds) == ["card", "cash", "voucher"]
assert Payments.kind_field == "kind"

# A kind's key is its base row's, so the database may not fill it.
NumberedCard = Model("card_payment", {
    "payment_id": Column("i32", primary_key=True, generated="by_default"),
    "last_four": Column("text"),
})
try:
    Disjoint(
        "payment",
        Payment,
        {"card": NumberedCard, "cash": None, "voucher": VoucherPayment},
        schema_name="public",
    )
except ConstructionError as error:
    assert "is an identity" in str(error)
else:
    raise AssertionError("a kind keyed by an identity was declared")
```

`Payments` is the disjoint model, which reads and writes a payment with its
kind's row. `Payment` is still an ordinary model of the `payment` table.

`Disjoint(name, base, kinds, *, kind="kind", enum_name=None,
schema_name=None)` takes the base's kind field, `kind` unless it says
otherwise, and names the kind type: `{name}_kind` unless `enum_name` names
another, in the schema `schema_name`. A model's enum column names its type's
schema, and the base's kind field must be of the kind type, so the two name
the same schema, here `public`. The kind field's `values` are the kinds, in
order.

Here Python differs from Rust. The derive writes two types beside the enum,
the kind type `PaymentKind` and `PaymentActiveModel`, which a write takes.
Python has neither: a kind is its value, `"card"`, and a write takes
dictionaries of fields. Rust's attributes have their Python spellings: `kind
= ..` is `kind=`, `enum_name` and `schema_name` are the same, and a variant's
`value = ".."` is its key in the mapping. `kind_attrs(..)`, which gives
Rust's generated kind type further attributes, has nothing to apply to.

Declaring one sends nothing. What Rust's `check_disjoint` checks before every
read and write, Python checks as the model is declared, and refuses with a
`ConstructionError`: no kinds, one model for two kinds, a kind model keyed by
other types than the base, a kind field missing, nullable, generated or not
of the kind type, and a kind model whose key the database fills, by an
identity or a default. A model whose table carries an alias is refused too,
since a disjoint read names each table bare.

## Creating the tables

`pgorm.schema.create_disjoint` returns the statements that create the kind
type and the tables, in the order they run: the kind type, the base table,
each kind's table, and one `ALTER TABLE` on the base adding what points back
at the kinds' tables. Every example is a whole program, so each below
declares the payments again:

```python
import asyncio
import os

from pgorm import Column, Disjoint, ForeignKeyViolationError, Model, Pool, RawSQL, TypeName
from pgorm import schema as s

KIND = TypeName("payment_kind", schema="public")
Payment = Model("payment", {
    "id": Column("i32", primary_key=True, generated="by_default"),
    "kind": Column(KIND, values=["card", "cash", "voucher"]),
    "order_id": Column("i32"),
    "amount": Column("decimal"),
})
CardPayment = Model("card_payment", {
    "payment_id": Column("i32", primary_key=True), "last_four": Column("text"),
})
VoucherPayment = Model("voucher_payment", {
    "payment_id": Column("i32", primary_key=True), "code": Column("text"),
})
Payments = Disjoint(
    "payment", Payment, {"card": CardPayment, "cash": None, "voucher": VoucherPayment},
    schema_name="public",
)

statements = s.create_disjoint(Payments)
assert [statement.inspect().sql for statement in statements] == [
    """CREATE TYPE "public"."payment_kind" AS ENUM ('card', 'cash', 'voucher')""",
    " ".join([
        'CREATE TABLE "payment" ( "id" integer NOT NULL GENERATED BY DEFAULT AS IDENTITY,',
        '"kind" public.payment_kind NOT NULL, "order_id" integer NOT NULL,',
        '"amount" decimal NOT NULL, PRIMARY KEY ("id"),',
        'CONSTRAINT "uq-payment-id-kind" UNIQUE ("id", "kind") )',
    ]),
    " ".join([
        'CREATE TABLE "card_payment" ( "payment_id" integer NOT NULL, "last_four" text NOT NULL,',
        """"kind" public.payment_kind GENERATED ALWAYS AS ('card') STORED, PRIMARY KEY ("payment_id"),""",
        'CONSTRAINT "fk-card_payment-payment_id-kind" FOREIGN KEY ("payment_id", "kind")',
        'REFERENCES "payment" ("id", "kind") ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE )',
    ]),
    " ".join([
        'CREATE TABLE "voucher_payment" ( "payment_id" integer NOT NULL, "code" text NOT NULL,',
        """"kind" public.payment_kind GENERATED ALWAYS AS ('voucher') STORED, PRIMARY KEY ("payment_id"),""",
        'CONSTRAINT "fk-voucher_payment-payment_id-kind" FOREIGN KEY ("payment_id", "kind")',
        'REFERENCES "payment" ("id", "kind") ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE )',
    ]),
    " ".join([
        'ALTER TABLE "payment"',
        """ADD COLUMN "card_id" integer GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'card') THEN "id" END)) STORED,""",
        """ADD COLUMN "voucher_id" integer GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'voucher') THEN "id" END)) STORED,""",
        'ADD CONSTRAINT "fk-payment-card_id" FOREIGN KEY ("card_id") REFERENCES "card_payment" ("payment_id")',
        'DEFERRABLE INITIALLY IMMEDIATE,',
        'ADD CONSTRAINT "fk-payment-voucher_id" FOREIGN KEY ("voucher_id")',
        'REFERENCES "voucher_payment" ("payment_id") DEFERRABLE INITIALLY IMMEDIATE',
    ]),
]


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for statement in statements:
            await pool.execute(statement)

        # A card payment written by hand without its card row is refused.
        try:
            await pool.execute(RawSQL(
                "INSERT INTO payment (kind, order_id, amount) VALUES ('card', 1, 12.20)"
            ))
        except ForeignKeyViolationError:
            pass
        else:
            raise AssertionError("a card payment was written without its card")


asyncio.run(main())
```

The constraints work like this. The base has a unique key over its key and
its kind. Each kind's table has a kind column of its own, generated as its
one kind, and a foreign key on its key and that column to the base's unique
key, so its row can only belong to a base row of its kind. The base has, for
each kind with a table, generated columns holding its key when the row is of
that kind and `NULL` otherwise, and a foreign key on them to that kind's
table, so a base row of that kind must have its row. Each foreign key is
`DEFERRABLE INITIALLY IMMEDIATE` and checked at the end of the statement,
which is how pgorm writes both rows in one statement. Deleting a base row
deletes its kind's row with it.

The generated columns and the kind tables' kind columns are bookkeeping: no
model has a field for them, and no read or write names them.

Here Python differs from Rust in the columns. A model's column has a kind
and no length or precision, so its table's columns are written as the kind
names them: `"decimal"` is `decimal` where Rust's entity says
`decimal(10, 2)`, and `"text"` is `text` where Rust's `String` is
`varchar`. Giving a model's column a length or a precision is not available
in Python yet; to store the amount as `numeric(10, 2)`, change the column
once the tables exist, with `alter_table("payment").modify_column(..)`
([Schema generation](14-schema.md)). A key the database fills is declared
`generated="by_default"`,
an identity, where Rust's entity has a `serial`: a column declared
`default=True` does not say what its default is, so `create_disjoint`
cannot write it and refuses the table. A unique key a model declares with
`unique_keys` is written into its table. The types the tables' other columns
use, such as another enum, are not among the statements; create them first.

## Writing

Every write is one statement, its parts joined by data-modifying `WITH`
clauses, which applies whole or not at all, with no transaction needed. Each
builds a write whose `returning_model(db)` sends it and returns the row as a
read gives it, the base's record, the kind and the kind's record or `None`,
and whose `inspect()` shows the statement.

- `insert(base, kind, values)` takes the base row's fields, the kind, and
  the kind row's fields, left out for a kind without a table. The base's kind
  field and the kind row's key are left out: the kind comes from `kind`, and
  the key from the inserted base row. A value given for either is refused
  with a `ConstructionError` before anything is sent.
- `update(base, kind, values)` changes the fields `base` sets of the row
  whose key `base` holds, and the fields `values` sets of its kind's row.
  Updating a row as another kind changes nothing and is a `DecodeError`, and
  so is a key no row has. Rust's refusal is `Error::RecordNotFound`.
- `change_kind(key, kind, values)` moves a row to another kind: it deletes
  the old kind's row, inserts the new one's from `values`, and sets the base
  row's kind. A change to the row's own kind changes nothing and is a
  `DecodeError`.

Deleting needs nothing of its own: delete the base row as any row is deleted,
and its kind's row goes with it.

```python
import asyncio
import os
from decimal import Decimal

from pgorm import (
    Column, DecodeError, Disjoint, ForeignKeyViolationError, Model, Pool, RawSQL, TypeName,
)
from pgorm import schema as s

KIND = TypeName("payment_kind", schema="public")
Payment = Model("payment", {
    "id": Column("i32", primary_key=True, generated="by_default"),
    "kind": Column(KIND, values=["card", "cash", "voucher"]),
    "order_id": Column("i32"),
    "amount": Column("decimal"),
})
CardPayment = Model("card_payment", {
    "payment_id": Column("i32", primary_key=True), "last_four": Column("text"),
})
VoucherPayment = Model("voucher_payment", {
    "payment_id": Column("i32", primary_key=True), "code": Column("text"),
})
Payments = Disjoint(
    "payment", Payment, {"card": CardPayment, "cash": None, "voucher": VoucherPayment},
    schema_name="public",
)


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for statement in s.create_disjoint(Payments):
            await pool.execute(statement)

        base, kind, card = await Payments.insert(
            {"order_id": 1, "amount": Decimal("12.20")}, "card", {"last_four": "4242"}
        ).returning_model(pool)
        assert kind == "card" and base["kind"] == "card"
        assert card is not None and dict(card) == {"payment_id": base["id"], "last_four": "4242"}

        # A new card number: the kind's row changes, found by the base's key.
        _, _, updated = await Payments.update(
            {"id": base["id"]}, "card", {"last_four": "4343"}
        ).returning_model(pool)
        assert updated is not None and updated["last_four"] == "4343"

        # An update as another kind finds no row of that kind.
        try:
            await Payments.update(
                {"id": base["id"], "amount": Decimal("1.00")}, "cash"
            ).returning_model(pool)
        except DecodeError:
            pass
        else:
            raise AssertionError("a card payment was updated as cash")

        cash, _, _ = await Payments.insert(
            {"order_id": 2, "amount": Decimal("3.80")}, "cash"
        ).returning_model(pool)

        # The customer paid with a voucher after all.
        moved, kind, voucher = await Payments.change_kind(
            {"id": cash["id"]}, "voucher", {"code": "XMAS-2026"}
        ).returning_model(pool)
        assert kind == "voucher" and moved["kind"] == "voucher"
        assert voucher is not None and voucher["code"] == "XMAS-2026"

        # A card payment cannot lose its card row behind pgorm's back.
        try:
            await pool.execute(RawSQL(
                'DELETE FROM "card_payment" WHERE "payment_id" = $1', [base["id"]]
            ))
        except ForeignKeyViolationError:
            pass
        else:
            raise AssertionError("a card row was deleted from under its payment")

        # Deleting the payment deletes its card row with it.
        await Payment.delete().where(Payment.key({"id": base["id"]})).execute(pool)
        assert await CardPayment.find().count(pool) == 0


asyncio.run(main())
```

## Reading

`find()` reads each base row with its kind's row in one statement, the base
`LEFT JOIN`ed to each kind's table, and gives each row as the triple
`(base record, kind, kind record)`, the last `None` for a kind without a
table. Here Python differs from Rust, whose rows are the pair
`(payment::Model, Payment)`: the kind and its row are Rust's enum value.

A read takes what a model's query takes: `where` and `order_by` on the
base's fields, `limit` and `offset`, and the terminals `all`, `one`,
`stream` and `count`, with `paginate(size)` and `cursor(*fields)` over base
fields, which [Pages, cursors and streams](06-pagination.md) covers for a
model. `find_by_key(key)` reads the row of one key, every key field named,
as Rust's `find_by_id` does. `one` is a `DecodeError` when there is no row.
Three methods are its own:

- `kind(kind)` and `kinds([..])` read only rows of the given kinds;
- `filter_kind(kind, predicate)` reads only rows of that kind whose kind row
  passes a condition over the kind model's fields.

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Column, Disjoint, Model, Pool, TypeName
from pgorm import schema as s

KIND = TypeName("payment_kind", schema="public")
Payment = Model("payment", {
    "id": Column("i32", primary_key=True, generated="by_default"),
    "kind": Column(KIND, values=["card", "cash", "voucher"]),
    "order_id": Column("i32"),
    "amount": Column("decimal"),
})
CardPayment = Model("card_payment", {
    "payment_id": Column("i32", primary_key=True), "last_four": Column("text"),
})
VoucherPayment = Model("voucher_payment", {
    "payment_id": Column("i32", primary_key=True), "code": Column("text"),
})
Payments = Disjoint(
    "payment", Payment, {"card": CardPayment, "cash": None, "voucher": VoucherPayment},
    schema_name="public",
)


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for statement in s.create_disjoint(Payments):
            await pool.execute(statement)
        for order_id, amount, kind, values in [
            (1, "12.20", "card", {"last_four": "4242"}),
            (2, "3.80", "cash", None),
            (3, "6.00", "card", {"last_four": "1881"}),
        ]:
            await Payments.insert(
                {"order_id": order_id, "amount": Decimal(amount)}, kind, values
            ).returning_model(pool)

        every = await Payments.find().order_by(Payment.col("id").asc()).all(pool)
        assert [kind for _, kind, _ in every] == ["card", "cash", "card"]

        assert await Payments.find().kind("card").count(pool) == 2

        base, kind, card = await (
            Payments.find()
            .filter_kind("card", CardPayment.col("last_four").ends_with("42"))
            .one(pool)
        )
        assert base["order_id"] == 1
        assert card is not None and card["last_four"] == "4242"


asyncio.run(main())
```

A row the constraints rule out can still be met in a database pgorm did not
create, such as a card payment with no card row. A read reports it as a
`DecodeError`, naming the table, the key and the kinds, and never decodes it
as some kind or drops it. Rust's report is `Error::Query`. A stream ends at
such a row, with the error.

## Changing the kinds

A program grows new kinds, and the database has to follow. `pgorm.schema`
writes each change from the declaration as it will be, as a
`DisjointChange`: its `steps`, each a `DisjointStep` whose `statements` run
in one transaction, committed before the next step starts. Some changes need
two steps because PostgreSQL will not use an enum value in the transaction
that added it. Each change takes the declaration and the kind's value.

| Change | What it does |
| --- | --- |
| `add_disjoint_kind` | adds a kind: `ALTER TYPE .. ADD VALUE`, then the kind's table, in two steps |
| `add_disjoint_kind_recreating` | the same change in one step, by recreating the kind type and rewriting the tables once |
| `remove_disjoint_kind` | removes a kind no row has any more, recreating the type; it fails while a row still has the kind |
| `add_disjoint_kind_table` | gives a kind without a table a table, leaving a gap in which the program writes each existing row's kind row |
| `remove_disjoint_kind_table` | drops a kind's table, named as its third argument, keeping the kind without one |

A change the declaration does not describe, such as adding a kind it does
not hold, is a `ConstructionError` as the function is called. A step's
`gap` is `None`, or a `DisjointGap` when the program has rows to write once
the step commits: a row in its `table` for every base row of its `kind`,
which the next step refuses the change without. Run by a migration tool,
each step is a migration of its own, committed before the next starts;
pgorm's Rust migration runner marks every step but the last with
`commits_after()` ([Migrations](22-migrations.md)).

A change is written from the declaration, and the declaration keeps changing
after the migration ships. Keep the statements a change returns, as their
text (`inspect().sql`), in the migration that applies them, rather than
computing them again each time the migration runs.

Here the bakery began taking payments by card and in cash, and adds
vouchers. The program declares the payments as they were, to create the
tables, and as they are now:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Column, Disjoint, Model, Pool, TypeName
from pgorm import schema as s

KIND = TypeName("payment_kind", schema="public")


def payment(kinds: list[str]) -> Model:
    return Model("payment", {
        "id": Column("i32", primary_key=True, generated="by_default"),
        "kind": Column(KIND, values=kinds),
        "order_id": Column("i32"),
        "amount": Column("decimal"),
    })


CardPayment = Model("card_payment", {
    "payment_id": Column("i32", primary_key=True), "last_four": Column("text"),
})
VoucherPayment = Model("voucher_payment", {
    "payment_id": Column("i32", primary_key=True), "code": Column("text"),
})

Before = Disjoint("payment", payment(["card", "cash"]), {"card": CardPayment, "cash": None},
                  schema_name="public")
Payments = Disjoint(
    "payment", payment(["card", "cash", "voucher"]),
    {"card": CardPayment, "cash": None, "voucher": VoucherPayment},
    schema_name="public",
)

change = s.add_disjoint_kind(Payments, "voucher")
assert len(change.steps) == 2
assert change.steps[0].statements[0].inspect().sql == (
    """ALTER TYPE "public"."payment_kind" ADD VALUE 'voucher' AFTER 'cash'"""
)
assert all(step.gap is None for step in change.steps)


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for statement in s.create_disjoint(Before):
            await pool.execute(statement)

        for step in change.steps:
            async with pool.transaction() as tx:
                for statement in step.statements:
                    await tx.execute(statement)
            pool.clear_statement_caches()

        _, kind, voucher = await Payments.insert(
            {"order_id": 2, "amount": Decimal("3.80")}, "voucher", {"code": "XMAS-2026"}
        ).returning_model(pool)
        assert kind == "voucher"
        assert voucher is not None and voucher["code"] == "XMAS-2026"


asyncio.run(main())
```

A step that recreates the kind type changes the type prepared statements
were planned with, so a program running steps itself clears its pool's
statement caches after each, as pgorm's migration runner does ([Statement
caches and DDL](19-statement-caches.md)).

Later the bakery records which till took each cash payment, so cash gets a
table. The rows already paid in cash have no row there yet, and the change
leaves a gap for the program to write them:

```python
import asyncio
import os

from pgorm import Column, Disjoint, Model, Pool, RawSQL, TypeName
from pgorm import schema as s

KIND = TypeName("payment_kind", schema="public")
Payment = Model("payment", {
    "id": Column("i32", primary_key=True, generated="by_default"),
    "kind": Column(KIND, values=["card", "cash"]),
    "order_id": Column("i32"),
    "amount": Column("decimal"),
})
CardPayment = Model("card_payment", {
    "payment_id": Column("i32", primary_key=True), "last_four": Column("text"),
})
CashPayment = Model("cash_payment", {
    "payment_id": Column("i32", primary_key=True), "till": Column("i16"),
})

Before = Disjoint("payment", Payment, {"card": CardPayment, "cash": None}, schema_name="public")
Payments = Disjoint("payment", Payment, {"card": CardPayment, "cash": CashPayment}, schema_name="public")

change = s.add_disjoint_kind_table(Payments, "cash")
first, second = change.steps
assert first.gap is not None
assert (first.gap.kind, first.gap.table.name) == ("cash", "cash_payment")
assert second.gap is None


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for statement in s.create_disjoint(Before):
            await pool.execute(statement)
        await pool.execute(RawSQL(
            "INSERT INTO payment (kind, order_id, amount) VALUES ('cash', 2, 3.80)"
        ))

        for step in change.steps:
            async with pool.transaction() as tx:
                for statement in step.statements:
                    await tx.execute(statement)
            pool.clear_statement_caches()
            if step.gap is not None:
                # Every cash payment so far was taken at till 1.
                await pool.execute(RawSQL(
                    "INSERT INTO cash_payment (payment_id, till) "
                    "SELECT id, 1 FROM payment WHERE kind::text = $1",
                    [step.gap.kind],
                ))

        _, kind, cash = await Payments.find().one(pool)
        assert kind == "cash"
        assert cash is not None and cash["till"] == 1


asyncio.run(main())
```

## Tables pgorm did not write

A schema written by hand may hold the same shape without a kind column: a
base table, and tables keyed by a foreign key onto it. Declare it with
`unkinded=True`. A row's kind is then the kind whose table has a row for it,
and a kind mapped to `None`, of which there is at most one, is a row with a
row in no kind's table.

Nothing in such a schema holds a row to one kind, so pgorm reads it and never
writes it. An unkinded model is a `Disjoint[Literal["unkinded"]]`: a type
checker refuses its `insert`, `update` and `change_kind`, and
`create_disjoint` and the kind changes of it, and each raises a
`ConstructionError` naming it read-only when it runs anyway. Rust's
unkinded enum has none of them, and a call to one does not compile. `kind`,
`enum_name` and `schema_name` are refused beside `unkinded`, since there is
no kind field or kind type, and so are a second kind without a model and no
kind with one.

```python
import asyncio
import os

from pgorm import Column, Disjoint, Model, Pool, RawSQL

Staff = Model("staff", {"id": Column("i32", primary_key=True), "name": Column("text")})
Driver = Model("driver", {"staff_id": Column("i32", primary_key=True), "licence": Column("text")})

Roles = Disjoint("staff", Staff, {"driver": Driver, "counter": None}, unkinded=True)
assert Roles.kind_field is None


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for sql in [
            "CREATE TABLE staff (id serial PRIMARY KEY, name text NOT NULL)",
            "CREATE TABLE driver ("
            "staff_id integer PRIMARY KEY REFERENCES staff, licence text NOT NULL)",
            "INSERT INTO staff (name) VALUES ('Eli'), ('Fay')",
            "INSERT INTO driver VALUES (1, 'B-1234')",
        ]:
            await pool.execute(RawSQL(sql))

        staff = await Roles.find().order_by(Staff.col("id").asc()).all(pool)
        assert [(member["name"], role) for member, role, _ in staff] == [
            ("Eli", "driver"), ("Fay", "counter"),
        ]
        eli = staff[0][2]
        assert eli is not None and dict(eli) == {"staff_id": 1, "licence": "B-1234"}
        assert await Roles.find().kind("counter").count(pool) == 1


asyncio.run(main())
```

A row with rows in two kinds' tables, or in none when no kind is mapped to
`None`, is a `DecodeError` when it is read.

## Disjoint enums a Rust crate declares

A Rust crate's `#[derive(Disjoint)]` enum reaches Python by registering it
into an application wheel ([Registering Rust entities](24-registrations.md)),
beside its base and each kind's entity, which register first:

```rust
registry.entity::<shipment::Entity>("app.Shipment")?;
registry.entity::<parcel::Entity>("app.Parcel")?;
registry.disjoint::<shipment::Shipment>("app.Shipments")?;
```

`pgorm.disjoint_entity("app.Shipments")` gives the registration, and
`pgorm.disjoint_entities()` lists the names the wheel registers. It reads as
a disjoint model does, each row the base entity's model, the kind, and the
kind entity's model or `None`, every one the Rust model it was read as, and
`create_disjoint` and the kind changes take it. Its writes take the
entities' `ActiveModel`s where a model's take dictionaries: `insert(base,
kind, active)`, with the base entity's `ActiveModel` and the kind entity's,
and `update` and `change_kind` likewise. The kind column set on the base's
`ActiveModel`, or a kind row's key set on its own, is refused with a
`ConstructionError` before anything is sent, and an `ActiveModel` of
another entity is an argument of the wrong type, a `TypeError`. The
statements are the ones Rust's `Shipment::find()`, `Shipment::insert` and
`Schema::create_disjoint` send. A [generated module](23-codegen.md) types
each registered enum's rows and writes by its kinds.

An enum declared `#[pgorm(base = .., unkinded)]` registers the same way and
is read-only, as its Rust declaration is: its kinds are its variants' names
in snake_case, its `describe()` has no kind field or kind type, and each
write and `create_disjoint` is refused with a `ConstructionError`.
