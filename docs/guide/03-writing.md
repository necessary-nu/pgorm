# Writing rows

pgorm writes rows through *active models* and three statement builders,
[`Insert`](crate::Insert), [`Update`](crate::Update) and
[`Delete`](crate::Delete). This chapter writes to the bakery: it adds cakes,
changes their prices, removes them, and signs customers up without creating
duplicates.

## Active models

A `Model` is a row as it was read. An `ActiveModel` is a row being written:
it has the same fields, but each holds an [`ActiveValue`](crate::ActiveValue)
that says what to do with its column.

| Value | Meaning |
| --- | --- |
| `Set(v)` | write `v` to the column |
| `Unchanged(v)` | the column holds `v` already; do not write it |
| `NotSet` | leave the column out of the statement |

The free function [`set`](crate::set) makes a `Set`, converting its argument
into the column's type, so `set("Lemon Drizzle")` fills a `String` column.
`.into()` makes a `Set` too, from a value of the column's own type or from a
`&str` for a text column. `Default::default()` leaves every column `NotSet`,
unless the entity's [`new` hook](crate::guide::entities#hooks) fills some, and a
column left out of an insert takes its default in the database, which
for the generated `id` is the next number:

```rust
# use pgorm_guide_support::cake::{self, Flavor};
use pgorm::entity::prelude::*;

let muffin = cake::ActiveModel {
    name: set("Blueberry Muffin"),
    flavor: Flavor::Vanilla.into(),
    price: set(Decimal::new(275, 2)),
    gluten_free: set(false),
    bakery_id: set(2),
    ..Default::default()
};
assert_eq!(muffin.name, ActiveValue::Set("Blueberry Muffin".to_owned()));
assert!(muffin.id.is_not_set());
assert!(muffin.is_changed());
```

A model read from the database becomes an active model with
[`into_active`](crate::ModelTrait::into_active). Each of its columns starts
`Unchanged`, so writing it back changes only the columns you then `set`.
[`set_if_not_equals`](crate::ActiveValue::set_if_not_equals) sets a column
only when the value differs from the one read, so
[`is_changed`](crate::ActiveModelTrait::is_changed) then says whether there
is anything to write. [`reset`](crate::ActiveValue::reset) turns an
`Unchanged` column into a `Set` one, to write it although its value is the
same.

## Inserting a row

[`insert`](crate::ActiveModelTrait::insert) on an active model writes it and
returns the row as the database stored it, generated `id` and defaults
included. It also runs the entity's [hooks](crate::guide::entities#hooks).

[`Insert::one`](crate::Insert::one) builds the same statement without running
hooks, and its terminals say what comes back:

| Terminal | Returns |
| --- | --- |
| [`exec`](crate::Insert::exec) | the number of rows written, with no `RETURNING` |
| [`exec_returning_pk`](crate::Insert::exec_returning_pk) | the row's primary key |
| [`exec_returning_model`](crate::Insert::exec_returning_model) | the row as a `Model` |
| [`exec_returning_upsert`](crate::Insert::exec_returning_upsert) | whether it inserted or updated, [below](#upserts) |

```rust
# use pgorm_guide_support::cake::{self, Flavor};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("writing_insert_one").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let cupcake = |name: &str, cents: i64| cake::ActiveModel {
    name: set(name),
    flavor: set(Flavor::Chocolate),
    price: set(Decimal::new(cents, 2)),
    gluten_free: set(false),
    bakery_id: set(1),
    ..Default::default()
};

let id: i32 = Insert::one(cupcake("Mocha Cupcake", 310)).exec_returning_pk(&db).await?;
assert_eq!(id, 6);

let model: cake::Model = Insert::one(cupcake("Cherry Cupcake", 330))
    .exec_returning_model(&db)
    .await?;
assert_eq!((model.id, model.name.as_str()), (7, "Cherry Cupcake"));

let written: u64 = Insert::one(cupcake("Plain Cupcake", 250)).exec(&db).await?;
assert_eq!(written, 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The statement binds every value as a parameter. An enum value is cast to its
type, since it travels as text:

```rust
# use pgorm_guide_support::cake::{self, Flavor};
use pgorm::entity::prelude::*;

let (sql, values) = Insert::one(cake::ActiveModel {
    name: set("Mocha Cupcake"),
    flavor: set(Flavor::Chocolate),
    price: set(Decimal::new(310, 2)),
    gluten_free: set(false),
    bakery_id: set(1),
    ..Default::default()
})
.build();
assert_eq!(
    sql,
    r#"INSERT INTO "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") VALUES ($1, CAST($2::text AS flavor), $3, $4, $5)"#
);
assert_eq!(values.0.len(), 5);
```

## Inserting many rows

[`Insert::many`](crate::Insert::many) writes a batch in one statement, and its
terminals return one key or one model per row, in the order the rows were
given: [`exec_returning_pks`](crate::Insert::exec_returning_pks) and
[`exec_returning_models`](crate::Insert::exec_returning_models). `exec`
returns the count. [`add`](crate::Insert::add) and
[`add_many`](crate::Insert::add_many) grow a batch, and turn a one-row insert
into a batch.

Every row of a batch must set the same columns, since one `INSERT` has one
column list; a batch that does not is refused before anything is sent. A
batch of no rows sends nothing: `exec` returns `Ok(0)` and the returning
terminals an empty `Vec`.

```rust
# use pgorm_guide_support::cake::{self, Flavor};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("writing_insert_many").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let tart = |name: &str, flavor: Flavor, cents: i64| cake::ActiveModel {
    name: set(name),
    flavor: set(flavor),
    price: set(Decimal::new(cents, 2)),
    gluten_free: set(true),
    bakery_id: set(2),
    ..Default::default()
};

let ids: Vec<i32> = Insert::many([
    tart("Lemon Tart", Flavor::Lemon, 420),
    tart("Chocolate Tart", Flavor::Chocolate, 450),
])
.exec_returning_pks(&db)
.await?;
assert_eq!(ids, [6, 7]);

let tarts: Vec<cake::Model> = Insert::many([tart("Vanilla Tart", Flavor::Vanilla, 400)])
    .add(tart("Carrot Tart", Flavor::Carrot, 410))
    .exec_returning_models(&db)
    .await?;
let names: Vec<&str> = tarts.iter().map(|t| t.name.as_str()).collect();
assert_eq!(names, ["Vanilla Tart", "Carrot Tart"]);

let nothing: Vec<cake::ActiveModel> = Vec::new();
assert_eq!(Insert::many(nothing).exec(&db).await?, 0);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Updating rows

To change a row you have read, turn it into an active model, set the columns
that change, and [`update`](crate::ActiveModelTrait::update) it. The statement
writes only those columns, finds the row by its primary key, and returns it
as it now is:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("writing_update_one").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let fudge: cake::Model = cake::Entity::find_by_id(1).one(&db).await?;
let mut cheaper = fudge.into_active();

// The same name is no change.
cheaper.name.set_if_not_equals("Chocolate Fudge".to_owned());
assert!(!cheaper.is_changed());

cheaper.price = set(Decimal::new(399, 2));
assert!(cheaper.is_changed());

let (sql, _) = Update::one(cheaper.clone())?.build();
assert_eq!(sql, r#"UPDATE "cake" SET "price" = $1 WHERE "cake"."id" = $2"#);

let fudge = cheaper.update(&db).await?;
assert_eq!(fudge.price.to_string(), "3.99");
assert_eq!(fudge.name, "Chocolate Fudge");
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

[`Update::one`](crate::Update::one) builds that statement without hooks. It
returns a `Result`, refusing a model whose primary key is not set with
[`Error::PrimaryKeyNotSet`](crate::Error::PrimaryKeyNotSet) before anything
is sent, and its terminal
[`exec_returning_model`](crate::UpdateOne::exec_returning_model) reads the row
back. A key that matches no row is
[`Error::RecordNotFound`](crate::Error::RecordNotFound).

[`Update::many`](crate::Update::many) updates every row its filter matches.
[`col_expr`](crate::UpdateMany::col_expr) sets a column to an expression,
which can read the row's own columns, and [`set`](crate::UpdateMany::set)
takes the `Set` columns of an active model. [`exec`](crate::UpdateMany::exec)
returns the number of rows changed, and
[`exec_returning_models`](crate::UpdateMany::exec_returning_models) the rows
themselves:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("writing_update_many").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

// Sunrise raises every price by fifty cents.
let raise = Update::many(cake::Entity)
    .col_expr(
        cake::Column::Price,
        Expr::col(cake::Column::Price).add(Decimal::new(50, 2)),
    )
    .filter(cake::Column::BakeryId.eq(1));
assert_eq!(
    raise.build().0,
    r#"UPDATE "cake" SET "price" = "price" + $1::numeric WHERE "cake"."bakery_id" = $2"#
);
assert_eq!(raise.exec(&db).await?, 3);

// Moonlight's cakes all become gluten-free.
let changed: Vec<cake::Model> = Update::many(cake::Entity)
    .set(cake::ActiveModel {
        gluten_free: set(true),
        ..Default::default()
    })
    .filter(cake::Column::BakeryId.eq(2))
    .exec_returning_models(&db)
    .await?;
assert!(changed.iter().all(|cake| cake.gluten_free));
assert_eq!(changed.len(), 2);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

An update with nothing to set is
[`Error::NothingToSet`](crate::Error::NothingToSet) rather than `Ok(0)`, since
no statement was sent. That is easy to reach by handing `set` a model read
from the database, whose columns are all `Unchanged`. `UPDATE` takes no
`ORDER BY` or `LIMIT` in PostgreSQL, so neither builder has them; filter on a
subquery instead. To read another table's columns in the update, name it with
[`from`](crate::UpdateMany::from) and join it in the filter.

### Before and after

PostgreSQL 18's `RETURNING` can read a row both as it was and as the update
left it. [`exec_returning_change`](crate::UpdateOne::exec_returning_change)
and [`exec_returning_changes`](crate::UpdateMany::exec_returning_changes)
return each row as a [`Change`](crate::Change), its `old` and `new` models:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("writing_change").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let changes = Update::many(cake::Entity)
    .col_expr(cake::Column::Price, Expr::col(cake::Column::Price).mul(2))
    .filter(cake::Column::Name.eq("Vanilla Sponge"))
    .exec_returning_changes(&db)
    .await?;
assert_eq!(changes.len(), 1);
assert_eq!(changes[0].old.price.to_string(), "2.90");
assert_eq!(changes[0].new.price.to_string(), "5.80");
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Deleting rows

A model, or an active model with its key set, deletes its own row with
`delete`, which returns the number of rows deleted and runs the entity's
`before_delete` and `after_delete` hooks.
[`Delete::one`](crate::Delete::one) builds the same statement and, like
`Update::one`, refuses a model without its key.
[`Delete::many`](crate::Delete::many) deletes what its filter matches, and
[`delete_by_id`](crate::EntityTrait::delete_by_id) deletes by key.

```rust
# use pgorm_guide_support::{cake, cake_baker};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("writing_delete").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

// Cy no longer bakes the Vanilla Sponge.
let row = cake_baker::Entity::find_by_id((4, 3)).one(&db).await?;
assert_eq!(row.delete(&db).await?, 1);

// Bo leaves the Lemon Drizzle to Ada.
assert_eq!(cake_baker::Entity::delete_by_id((2, 2)).exec(&db).await?, 1);

// Moonlight stops selling the cakes nobody bakes.
let unbaked = Delete::many(cake::Entity)
    .filter(cake::Column::BakeryId.eq(2))
    .filter(
        cake::Column::Id.not_in_subquery(
            pgorm::pgorm_query::Query::select()
                .column(cake_baker::Column::CakeId)
                .from(cake_baker::Entity)
                .to_owned(),
        ),
    );
assert_eq!(unbaked.exec(&db).await?, 1);
assert_eq!(cake::Entity::find().count(&db).await?, 4);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

[`using`](crate::DeleteMany::using) names another table the filter reads,
PostgreSQL's `DELETE .. USING`. A delete that matches nothing returns
`Ok(0)`.

## Upserts

An upsert inserts a row unless it would conflict with one already there, and
then does something else: updates the row that is there, or skips the new
one. [`on_conflict`](crate::Insert::on_conflict) attaches PostgreSQL's
`ON CONFLICT` clause, built with
[`OnConflict`](crate::pgorm_query::OnConflict): the columns of the unique key
that may conflict, and what to do. `update_column` takes a column's new value
from the row that failed to insert.

The bakery's customers are unique by email. A sign-up from an address already
known updates the name instead of failing.
[`exec_returning_upsert`](crate::Insert::exec_returning_upsert) says which
happened, as an [`Upserted`](crate::Upserted): `Inserted` with the new row,
or `Updated` with the row before and after:

```rust
# use pgorm_guide_support::customer;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("writing_upsert").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::OnConflict;
use pgorm::Upserted;

let sign_up = |name: &str, email: &str| {
    Insert::one(customer::ActiveModel {
        name: set(name),
        email: set(email),
        notes: set(None),
        ..Default::default()
    })
    .on_conflict(
        OnConflict::column(customer::Column::Email).update_column(customer::Column::Name),
    )
};

match sign_up("Kimberly", "kim@example.com").exec_returning_upsert(&db).await? {
    Some(Upserted::Updated(change)) => {
        assert_eq!((change.old.name.as_str(), change.new.name.as_str()), ("Kim", "Kimberly"));
    }
    other => panic!("Kim signed up before: {other:?}"),
}

match sign_up("Mo", "mo@example.com").exec_returning_upsert(&db).await? {
    // Kim's upsert drew id 3 before it conflicted, so Mo gets 4.
    Some(Upserted::Inserted(mo)) => assert_eq!(mo.id, 4),
    other => panic!("Mo is new: {other:?}"),
}
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

An insert draws its generated key before it finds the conflict, so a
conflicting row uses up a number of the sequence, as Kim's did.
[`exec_returning_upserts`](crate::Insert::exec_returning_upserts) does the
same as `exec_returning_upsert` for a batch, row by row. The other terminals work under `ON CONFLICT`
too: `exec_returning_pk` returns the key of the row written, whether it was
inserted or updated.

To skip conflicting rows instead,
[`on_conflict_do_nothing`](crate::Insert::on_conflict_do_nothing) adds
`ON CONFLICT (primary key) DO NOTHING`, and `OnConflict::column(..).do_nothing()`
names another key. Both turn the insert into a
[`TryInsert`](crate::TryInsert), whose terminals answer with a
[`TryInsertResult`](crate::TryInsertResult): `Inserted` with what was written,
or `Conflicted` when every row was skipped.
[`on_empty_do_nothing`](crate::Insert::on_empty_do_nothing) makes a
`TryInsert` too, answering an empty batch with `Empty`.

```rust
# use pgorm_guide_support::cake_baker;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("writing_do_nothing").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::TryInsertResult;

let bakes = |cake_id: i32, baker_id: i32| cake_baker::ActiveModel {
    cake_id: set(cake_id),
    baker_id: set(baker_id),
};

// Ada already bakes the Chocolate Fudge.
let again = Insert::one(bakes(1, 1)).on_conflict_do_nothing();
assert_eq!(
    again.build().0,
    r#"INSERT INTO "cake_baker" ("cake_id", "baker_id") VALUES ($1, $2) ON CONFLICT ("cake_id", "baker_id") DO NOTHING"#
);
assert!(matches!(again.exec(&db).await?, TryInsertResult::Conflicted));

// Bo starts baking the Chocolate Fudge too.
let new = Insert::one(bakes(1, 2)).on_conflict_do_nothing().exec(&db).await?;
assert!(matches!(new, TryInsertResult::Inserted(1)));

let none: Vec<cake_baker::ActiveModel> = Vec::new();
let empty = Insert::many(none).on_empty_do_nothing().exec(&db).await?;
assert!(matches!(empty, TryInsertResult::Empty));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## MERGE

PostgreSQL's `MERGE` reconciles a table with a source of rows in one
statement: it updates the rows that match, inserts the ones that do not, and
can delete. pgorm builds it with the query builder's
[`Query::merge`](crate::pgorm_query::Query::merge), not with an entity
builder; [The query builder](crate::guide::query_builder) covers it.
