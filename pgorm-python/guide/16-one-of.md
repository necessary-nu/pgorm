# One-of references

A review at the bakery is of a cake or of a bakery, never both and never
neither. Rust says that with an enum. PostgreSQL says it with one nullable
foreign-key column per target, `cake_id` and `bakery_id`, and a `CHECK` that
exactly one of them is set. Each column is an ordinary foreign key, so the
database still knows what every review points at, and the `CHECK` keeps a row
from pointing at two things.

`pgorm.OneOf` declares such a reference over the model holding it, and gives
you three things: a row's reference read as its target and key, the fields a
write sets for a reference with the other columns cleared, and the targets of
many rows loaded with one query per target table.
`pgorm.schema.create_one_of` writes the `CHECK`.

Here Python differs from Rust. Rust declares the reference with the `OneOf`
derive, an enum whose variants name its targets. Python declares it over
models, and a target is its name, a string. Both are pgorm's one description
of the shape, so the `CHECK`, the loads and the refusals are Rust's.

## Declaring one

The model holding the reference has a nullable field for each target.
`OneOf(name, model, targets, *, optional=False)` names the reference and
maps each target's value, in order, to the field of `model` holding its key
and the target's model, which is keyed by one field. `optional=True` lets a
row reference none of them, as a Rust enum's unit variant does:

```python
from pgorm import Column, ConstructionError, Model, OneOf

from bakery import Bakery, Cake

Review = Model("review", {
    "id": Column("i32", primary_key=True, generated="by_default"),
    "stars": Column("i16"),
    "body": Column("text"),
    "cake_id": Column("i32", nullable=True),
    "bakery_id": Column("i32", nullable=True),
})

Subject = OneOf("subject", Review, {"cake": ("cake_id", Cake), "bakery": ("bakery_id", Bakery)})

assert Subject.describe() == {
    "name": "subject",
    "model": "review",
    "targets": [
        {"value": "cake", "field": "cake_id", "target": "cake"},
        {"value": "bakery", "field": "bakery_id", "target": "bakery"},
    ],
    "optional": False,
}

# A reference to the cake of key 1, as the fields a write sets.
assert Subject.set("cake", 1) == {"cake_id": 1, "bakery_id": None}

# A row may lack any one target, so each field is nullable.
try:
    OneOf("subject", Review, {"cake": ("id", Cake)})
except ConstructionError as error:
    assert "is not nullable" in str(error)
else:
    raise AssertionError("a reference through a key field was declared")
```

Here Python differs from Rust. The derive writes two enums beside the
reference, `SubjectKey`, each variant holding its target's key, and
`SubjectModel`, each holding its target's `Model`, and `key_attrs(..)` and
`model_attrs(..)` give them further attributes. Python has neither: a key is
the pair `("cake", 1)`, and a loaded target the pair of its value and its
record.

Declaring one sends nothing. What does not fit the shape, which in Rust does
not compile, is a `ConstructionError` as the reference is declared: a field
the model lacks, one that is not nullable, one field for two targets, a
field of another kind than its target's key, a target keyed by more than one
field, one target model for two values, no target, and a name longer than 59
bytes, so that the `CHECK` named for it keeps the name whole within
PostgreSQL's 63 bytes. A Rust enum's targets
must be entities the holding entity has a `Related` relation to; a model's
need no relation.

## The constraint

The model's table, with a foreign key along each target's field, is written
with the [DDL builders](14-schema.md). `create_one_of` writes the `CHECK`
beside it, an `ALTER TABLE` adding `num_nonnulls(..) = 1`, or `<= 1` where
the reference is optional. It is named `ck-{table}-{name}`:

```python
import asyncio
import os

from pgorm import CheckViolationError, Column, Model, OneOf, Pool
from pgorm import schema as s

from bakery import Bakery, Cake

Review = Model("review", {
    "id": Column("i32", primary_key=True, generated="by_default"),
    "stars": Column("i16"),
    "body": Column("text"),
    "cake_id": Column("i32", nullable=True),
    "bakery_id": Column("i32", nullable=True),
})
Subject = OneOf("subject", Review, {"cake": ("cake_id", Cake), "bakery": ("bakery_id", Bakery)})

reviews = (
    s.create_table("review")
    .column(s.ColumnDef("id", "integer").not_null().identity("by_default"))
    .column(s.ColumnDef("stars", "smallint").not_null())
    .column(s.ColumnDef("body", "text").not_null())
    .column(s.ColumnDef("cake_id", "integer"))
    .column(s.ColumnDef("bakery_id", "integer"))
    .primary_key("id")
    .foreign_key("cake_id", "cake", "id")
    .foreign_key("bakery_id", "bakery", "id")
)
check = s.create_one_of(Subject)
assert check.inspect().sql == (
    'ALTER TABLE "review" ADD CONSTRAINT "ck-review-subject" '
    'CHECK (num_nonnulls("cake_id", "bakery_id") = 1)'
)


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(reviews)
        await pool.execute(check)

        # A review of a cake and a bakery at once is refused.
        try:
            await Review.insert(
                {"stars": 3, "body": "Both?", "cake_id": 1, "bakery_id": 1}
            ).execute(pool)
        except CheckViolationError as error:
            assert error.constraint == "ck-review-subject"
        else:
            raise AssertionError("a review of two things was written")


asyncio.run(main())
```

Here Python differs from Rust, where `create_table_from_entity` writes the
foreign keys from the entity's relations: a model's table is written by
hand, as [Schema generation](14-schema.md) says.

## Reading and writing

`key(record)` reads a row's reference as the pair `(value, key)`, or `None`
for a row that references nothing where the reference is optional. `set(value,
key)` gives the fields a write sets for a reference: the target's field to
its key and every other to `None`, so moving a review from a cake to a bakery
is one update. `set(None)` clears them all where the reference is optional.
A model's `insert` and `update` take them beside their other fields.
`load(db, records)` takes a list of records and returns each one's target, in
order, as the pair `(value, record)`, or `None` for a record that references
nothing, reading each target table once, with one query for the keys the
records hold for it:

```python
import asyncio
import os

from pgorm import Column, Model, OneOf, Pool
from pgorm import schema as s

from bakery import Bakery, Cake

Review = Model("review", {
    "id": Column("i32", primary_key=True, generated="by_default"),
    "stars": Column("i16"),
    "body": Column("text"),
    "cake_id": Column("i32", nullable=True),
    "bakery_id": Column("i32", nullable=True),
})
Subject = OneOf("subject", Review, {"cake": ("cake_id", Cake), "bakery": ("bakery_id", Bakery)})

reviews = (
    s.create_table("review")
    .column(s.ColumnDef("id", "integer").not_null().identity("by_default"))
    .column(s.ColumnDef("stars", "smallint").not_null())
    .column(s.ColumnDef("body", "text").not_null())
    .column(s.ColumnDef("cake_id", "integer"))
    .column(s.ColumnDef("bakery_id", "integer"))
    .primary_key("id")
    .foreign_key("cake_id", "cake", "id")
    .foreign_key("bakery_id", "bakery", "id")
)


def review(stars: int, body: str, target: str, key: int) -> dict[str, object]:
    return {"stars": stars, "body": body, **Subject.set(target, key)}


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(reviews)
        await pool.execute(s.create_one_of(Subject))

        await Review.insert_many([
            review(5, "Rich and dark", "cake", 1),
            review(4, "Friendly staff", "bakery", 2),
            review(3, "A little dry", "cake", 3),
        ]).execute(pool)

        written = await Review.find().order_by(Review.col("id").asc()).all(pool)
        assert [Subject.key(record) for record in written] == [
            ("cake", 1), ("bakery", 2), ("cake", 3),
        ]

        named = []
        for subject in await Subject.load(pool, written):
            assert subject is not None
            target, record = subject
            named.append(f"{target} {record['name']}")
        assert named == ["cake Chocolate Fudge", "bakery Moonlight", "cake Carrot Cake"]

        # The third review was really about the bakery that sells the cake.
        moved = await (
            Review.update(Subject.set("bakery", 2))
            .where(Review.key({"id": written[2]["id"]}))
            .returning()
            .one(pool)
        )
        assert (moved["cake_id"], moved["bakery_id"]) == (None, 2)
        assert Subject.key(moved) == ("bakery", 2)


asyncio.run(main())
```

A row the `CHECK` rules out can still be met in a database without it: a
review with both fields set, or with neither where the reference is not
optional. `key` and `load` report it as a `DecodeError`, naming the table,
the row's key and the columns, rather than read it as one of its targets. So
does `load` for a key its target's table does not hold. Rust's report is
`Error::Query`. `set` refuses a target the reference does not have, a target
without its key, and `set(None)` where the reference is not optional, each
with a `ConstructionError`.

The targets of one row are also reachable the ordinary way: each field is a
key of its target, so a relation declared along it, such as
`Review.belongs_to(Cake, from_="cake_id", to="id")`, reads them too, as
[Relations](07-relations.md) shows. `OneOf` is for when the program wants
the one target a row has, whichever it is.

## Generated entities

A Rust crate's `#[derive(OneOf)]` enum reaches Python by registering it into
an application wheel ([Registering Rust entities](24-registrations.md)),
after its entity and each target's entity:

```rust
registry.entity::<review::Entity>("app.Review")?;
registry.one_of::<review::Subject>("app.Subject")?;
```

`pgorm.one_of_entity("app.Subject")` gives the registration, and
`pgorm.one_of_entities()` lists the names the wheel registers. Its targets
are named as its variants are, in snake_case, and its records are the
entities' own models. `key(record)` and `load(db, records)` work as a
model's do, `set(active, value, key)` returns the entity's `ActiveModel`
with the target's column set to the key and every other to null, and
`create_one_of` writes its `CHECK`, the one Rust's `Schema::create_one_of`
writes.

pgorm-codegen, Rust's entity generator, recognises the shape in a schema:
nullable foreign-key columns covered by a `CHECK (num_nonnulls(..) = 1)` or
`<= 1` generate a `OneOf` enum, named for the constraint, beside the entity
([Generating entities](23-codegen.md)).
