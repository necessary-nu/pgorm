# The query builder

pgorm-query builds SQL statements as Rust values. A statement is a tree of
tables, columns and expressions, which renders to PostgreSQL text with every
value bound as a parameter. pgorm's entity builders are written on top of it,
and pgorm re-exports it as [`pgorm::pgorm_query`](crate::pgorm_query), so it
needs no dependency of its own.

The entity builders of [Writing rows](crate::guide::writing) and
[Reading rows](crate::guide::reading) cover reading and writing one entity's rows.
Reach for the query builder when a statement is about something else: a
report joining several tables into columns of your choosing, an `UPDATE`
that reads another table, a `MERGE`, a `WITH` query, a table function, or a
table no entity describes. It builds `SELECT`, `INSERT`, `UPDATE`, `DELETE`
and `MERGE`, and the DDL statements of [Schema generation](crate::guide::schema).

The builder refuses what PostgreSQL would refuse wherever the types can say
so: a `MERGE` with no `WHEN` clause has no `build`, an `UPDATE` has no
`LIMIT`, and a string cannot stand where a name belongs.

## A first statement

[`Query::select`](crate::pgorm_query::Query::select) starts a `SELECT`. The
builder methods take `&mut self` and return it, so a statement is written as
one chain and finished with `to_owned()`:

```rust
use pgorm::pgorm_query::{Expr, ExprTrait, Order, Query};
use pgorm_guide_support::{bakery, cake};

let query = Query::select()
    .column((cake::Entity, cake::Column::Name))
    .column((bakery::Entity, bakery::Column::City))
    .from(cake::Entity)
    .inner_join(
        bakery::Entity,
        Expr::col((cake::Entity, cake::Column::BakeryId)).equals((bakery::Entity, bakery::Column::Id)),
    )
    .and_where(Expr::col((cake::Entity, cake::Column::GlutenFree)).eq(true))
    .order_by((cake::Entity, cake::Column::Name), Order::Asc)
    .to_owned();

let (sql, values) = query.build();
assert_eq!(
    sql,
    r#"SELECT "cake"."name", "bakery"."city" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "cake"."gluten_free" = $1 ORDER BY "cake"."name" ASC"#
);
assert_eq!(values.0, [true.into()]);

assert_eq!(
    query.to_string(),
    r#"SELECT "cake"."name", "bakery"."city" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "cake"."gluten_free" = TRUE ORDER BY "cake"."name" ASC"#
);
```

A statement renders two ways.
[`build`](crate::pgorm_query::SelectStatement::build) writes `$1`, `$2`, …
and returns the values beside the SQL; that is what you execute.
`to_string` writes each value into the SQL as an escaped literal, which is
safe to read but is a different statement text for each value, so keep it
for tests and logs.

An entity's `Entity` and `Column` values are names here: `cake::Entity` is
the table `"cake"`, and `(cake::Entity, cake::Column::Name)` is the qualified
column `"cake"."name"`. The statements in this chapter name the bakery's
tables and columns this way.

## Running a statement

A `SELECT` runs through the same decode terminals as the entity builders.
[`DecodeSelect`](crate::DecodeSelect), in the prelude, gives a
`SelectStatement` [`into_tuple`](crate::DecodeSelect::into_tuple),
[`into_model`](crate::DecodeSelect::into_model) and
[`into_values`](crate::DecodeSelect::into_values):

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("query_builder_run").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{Order, Query};
use pgorm_guide_support::{bakery, cake};

let query = Query::select()
    .column((cake::Entity, cake::Column::Name))
    .column((bakery::Entity, bakery::Column::City))
    .from(cake::Entity)
    .inner_join(
        bakery::Entity,
        Expr::col((cake::Entity, cake::Column::BakeryId)).equals((bakery::Entity, bakery::Column::Id)),
    )
    .and_where(Expr::col((cake::Entity, cake::Column::GlutenFree)).eq(true))
    .order_by((cake::Entity, cake::Column::Name), Order::Asc)
    .to_owned();

let rows: Vec<(String, String)> = query.into_tuple().all(&db).await?;
assert_eq!(
    rows,
    [
        ("Chocolate Torte".to_owned(), "Oslo".to_owned()),
        ("Lemon Drizzle".to_owned(), "Oslo".to_owned()),
        ("Vanilla Sponge".to_owned(), "Bergen".to_owned()),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

An `INSERT`, `UPDATE`, `DELETE` or `MERGE` is built and executed: wrap each
value in [`ValueHolder`](crate::ValueHolder), which binds it under the type
the server asks for, and pass them to
[`execute_raw`](crate::ConnectionTrait::execute_raw), which returns the number
of rows written. A statement with a `RETURNING` clause yields rows, which the
`(sql, values)` pair decodes through [`DecodeRaw`](crate::DecodeRaw):

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("query_builder_writes").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::Query;
use pgorm::ValueHolder;
use pgorm_guide_support::cake;

let (sql, values) = Query::update()
    .table(cake::Entity)
    .value(cake::Column::Price, Expr::col(cake::Column::Price).add(Decimal::new(50, 2)))
    .and_where(Expr::col(cake::Column::BakeryId).eq(2))
    .build();
assert_eq!(sql, r#"UPDATE "cake" SET "price" = "price" + $1 WHERE "bakery_id" = $2"#);
let raised = db.execute_raw(&sql, values.into_iter().map(ValueHolder)).await?;
assert_eq!(raised, 2);

let deleted: Vec<(String,)> = Query::delete()
    .from_table(cake::Entity)
    .and_where(Expr::col(cake::Column::Id).eq(5))
    .returning_col(cake::Column::Name)
    .build()
    .into_tuple()
    .all(&db)
    .await?;
assert_eq!(deleted, [("Chocolate Torte".to_owned(),)]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Where a bound value's type comes from

Each value a statement binds is a parameter, and PostgreSQL decides its type
from where it stands. Compared with a column or assigned to one, it takes the
column's type, which is the common case: `"price" = $1` reads `$1` as a
`numeric`. Where an operator or a function chooses its overload by the
operand's own type, the builder writes that type beside the placeholder:
`unnest` and `generate_series` over a bound list or bound bounds
(`UNNEST($1::int4[])`), a number added to a date (`"day" + $1::int4`), an
interval taken from a timestamp, a bound range that contains a timestamp
(`$1::tstzrange @> "placed_at"`). Text and floats stay bare there too, the
operand beside them typing them. Where nothing around a value says, the
server still cannot choose: a value in a `SELECT` list or a `VALUES` row, or
an argument of a function called by name. There, cast the value with
[`cast_as`](crate::pgorm_query::ExprTrait::cast_as), which also tells the
server the type the value is sent as, `CAST($1::date AS date)`, as the
example under [WITH](#with) does.

## Names

A table, a column, an alias and a type are *names*, and the builder takes
them only as names: never as a `&str`. A string where a column belongs does
not compile, so no text from a user can become an identifier by accident:

```rust,compile_fail,E0277
use pgorm::pgorm_query::Query;

let query = Query::select().column("name").from(pgorm_guide_support::cake::Entity).to_owned();
```

A name comes from one of four places:

- An entity's `Entity` or `Column`, for its own table and columns.
- A type deriving [`SqlName`](derive@crate::SqlName), for a table no entity
  describes. Each variant is a column, named in snake_case unless
  `#[iden = ".."]` says otherwise, and a variant named `Table` is the table.
  [`StaticName`](derive@crate::StaticName) also gives each name as a
  `&'static str`.
- [`alias`](crate::pgorm_query::alias), for a name written in the program,
  such as an alias the statement introduces: `alias("total")`.
- [`Name::runtime`](crate::pgorm_query::Name::runtime), for a name computed
  while the program runs, such as a table chosen by configuration. It is the
  one door from text into a name, so every place one is made is easy to find.

Table, column and alias names render quoted, so `alias("order")` is
`"order"` and a reserved word needs no care.

```rust
use pgorm::SqlName;
use pgorm::pgorm_query::{Expr, Func, Name, Query, alias};

/// A table loaded by a nightly job, which no entity describes.
#[derive(SqlName)]
enum PriceList {
    Table,
    Name,
    #[iden = "new_price"]
    Price,
}

let total = alias("total");
let query = Query::select()
    .expr_as(Func::sum(Expr::col(PriceList::Price)), total)
    .from(PriceList::Table)
    .to_owned();
assert_eq!(query.to_string(), r#"SELECT SUM("new_price") AS "total" FROM "price_list""#);

let archive = Name::runtime(format!("price_list_{}", 2025));
let query = Query::select().column(PriceList::Name).from(archive).to_owned();
assert_eq!(query.to_string(), r#"SELECT "name" FROM "price_list_2025""#);
```

## Expressions

[`Expr`](crate::pgorm_query::Expr) starts the expressions a statement holds:
[`Expr::col`](crate::pgorm_query::Expr::col) for a column and
[`Expr::val`](crate::pgorm_query::Expr::val) for a value.
[`Func`](crate::pgorm_query::Func) calls functions: the aggregates,
`COALESCE`, `LOWER`, the full-text search functions, and any function by
name through [`Func::named`](crate::pgorm_query::Func::named). The operators
are the methods of [`ExprTrait`](crate::pgorm_query::ExprTrait): `eq`, `gt`,
`between`, `is_in`, `is_null`, `add`, `mul`, `and`, `or`, `like`, `cast_as`,
`count`, and many more. Every expression has them, a function call and what
an operator returns included, so `Func::lower(..).eq(..)` and
`Expr::col(..).count().gt(2)` read as written. The prelude brings the trait;
a module that names its imports one by one names `ExprTrait` beside `Expr`.
[`Condition::all`](crate::pgorm_query::Condition::all)
and [`Condition::any`](crate::pgorm_query::Condition::any) group conditions
under `AND` and `OR`.

```rust
use pgorm::pgorm_query::{Condition, Expr, ExprTrait, Func, LikeExpr, Query, alias};
use pgorm_guide_support::cake;

let query = Query::select()
    .column(cake::Column::Name)
    .expr_as(Func::lower(Expr::col(cake::Column::Name)), alias("key"))
    .expr_as(
        Expr::case(Expr::col(cake::Column::Price).gte(5), "premium")
            .case(Expr::col(cake::Column::Price).gte(3), "standard")
            .finally("budget"),
        alias("tier"),
    )
    .from(cake::Entity)
    .cond_where(
        Condition::any()
            .add(Expr::col(cake::Column::Name).like(LikeExpr::containing("Choc")))
            .add(
                Condition::all()
                    .add(Expr::col(cake::Column::GlutenFree).eq(true))
                    .add(Expr::col(cake::Column::Price).between(2, 4)),
            ),
    )
    .to_owned();

assert_eq!(
    query.to_string(),
    [
        r#"SELECT "name", LOWER("name") AS "key","#,
        r#"(CASE WHEN ("price" >= 5) THEN 'premium' WHEN ("price" >= 3) THEN 'standard' ELSE 'budget' END) AS "tier""#,
        r#"FROM "cake""#,
        r#"WHERE "name" LIKE '%Choc%' OR ("gluten_free" = TRUE AND "price" BETWEEN 2 AND 4)"#,
    ]
    .join(" ")
);
```

[`Expr::case_of`](crate::pgorm_query::Expr::case_of) writes the simple form
of `CASE`, comparing one operand with each arm's value.

A few expressions take a little more care:

- A `LIKE` pattern is a [`LikeExpr`](crate::pgorm_query::LikeExpr). Its `%`
  and `_` are wildcards and its backslash escapes them, and
  [`escape`](crate::pgorm_query::LikeExpr::escape) picks another escape
  character. Text from a user goes through
  [`LikeExpr::containing`](crate::pgorm_query::LikeExpr::containing),
  [`starting_with`](crate::pgorm_query::LikeExpr::starting_with),
  [`ending_with`](crate::pgorm_query::LikeExpr::ending_with) or
  [`literal`](crate::pgorm_query::LikeExpr::literal), which escape its `%`,
  `_` and escape character so it matches itself, as the entity columns'
  `contains`, `starts_with` and `ends_with` do ([Reading rows](crate::guide::reading)).
- [`cast_as`](crate::pgorm_query::ExprTrait::cast_as) writes `CAST(.. AS type)`,
  its type given as a name. A cast, like a function call or a column's
  aggregate, is an expression in its own right, so it is compared and built
  on as a column is, as the example below does.
- [`collate`](crate::pgorm_query::ExprTrait::collate) puts text under a
  collation, which decides how it compares and sorts.
- [`index`](crate::pgorm_query::ExprTrait::index) and
  [`slice`](crate::pgorm_query::ExprTrait::slice) subscript an array, counting
  from 1.
- [`in_subquery`](crate::pgorm_query::ExprTrait::in_subquery) and
  [`Expr::exists`](crate::pgorm_query::Expr::exists) test against a
  subquery, and [`from_subquery`](crate::pgorm_query::SelectStatement::from_subquery)
  reads one as a table.
- [`Expr::template`](crate::pgorm_query::Expr::template) writes a piece of
  SQL the builder has no method for, with `$1`, `$2`, … for its values, which
  are bound, and `$$` for a literal `$`. The placeholders must name exactly
  the values given; a template naming more or fewer is refused when it is
  made.
  The text is written into the statement as it stands, so it is a
  `&'static str`, SQL your program writes: text built while the program
  runs, a user's included, does not compile there.

```rust
use pgorm::pgorm_query::{Expr, ExprTrait, Name, Order, Query};
use pgorm_guide_support::{cake, order_line};

let query = Query::select()
    .column(cake::Column::Name)
    .from(cake::Entity)
    .and_where(
        Expr::col(cake::Column::Id).in_subquery(
            Query::select()
                .column(order_line::Column::CakeId)
                .from(order_line::Entity)
                .and_where(Expr::col(order_line::Column::Quantity).gt(1))
                .to_owned(),
        ),
    )
    .and_where(Expr::col(cake::Column::Price).cast_as(Name::runtime("integer")).lt(10))
    .and_where(Expr::template("char_length($1) < 40", ["Chocolate Fudge"])?)
    .order_by_expr(Expr::col(cake::Column::Name).collate(Name::runtime("C")), Order::Asc)
    .to_owned();

assert_eq!(
    query.to_string(),
    [
        r#"SELECT "name" FROM "cake""#,
        r#"WHERE "id" IN (SELECT "cake_id" FROM "order_line" WHERE "quantity" > 1)"#,
        r#"AND CAST("price" AS integer) < 10"#,
        r#"AND (char_length('Chocolate Fudge') < 40)"#,
        r#"ORDER BY ("name" COLLATE "C") ASC"#,
    ]
    .join(" ")
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## INSERT, ON CONFLICT and RETURNING

[`Query::insert`](crate::pgorm_query::Query::insert) names its table and
columns, then takes rows: [`values`](crate::pgorm_query::InsertStatement::values)
returns an error for a row whose length does not match the columns, and
[`values_panic`](crate::pgorm_query::InsertStatement::values_panic) panics
instead. [`select_from`](crate::pgorm_query::InsertStatement::select_from)
inserts a query's rows.

[`on_conflict`](crate::pgorm_query::InsertStatement::on_conflict) takes an
[`OnConflict`](crate::pgorm_query::OnConflict): the columns or constraint
that conflict, and whether to do nothing or update. An update names the
columns to take from the row that was refused, `excluded`, or sets values of
its own. A sign-up form that may name an existing customer:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("query_builder_upsert").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{OnConflict, Query};
use pgorm_guide_support::customer;

let sign_up = |name: &str, email: &str| -> Result<_, Error> {
    Ok(Query::insert()
        .into_table(customer::Entity)
        .columns([customer::Column::Name, customer::Column::Email])
        .values([name.into(), email.into()])?
        .on_conflict(OnConflict::column(customer::Column::Email).update_column(customer::Column::Name))
        .returning_col(customer::Column::Id)
        .build())
};

let (sql, _) = sign_up("Kimberly", "kim@example.com")?;
assert_eq!(
    sql,
    r#"INSERT INTO "customer" ("name", "email") VALUES ($1, $2) ON CONFLICT ("email") DO UPDATE SET "name" = "excluded"."name" RETURNING "id""#
);

// Kim's address is taken, so her row is updated and keeps its id.
let (id,): (i32,) = sign_up("Kimberly", "kim@example.com")?.into_tuple().one(&db).await?;
assert_eq!(id, 1);
// A new address is a new customer. Its id is 4, not 3: the insert that
// conflicted drew 3 from the id's sequence before it conflicted.
let (id,): (i32,) = sign_up("Mo", "mo@example.com")?.into_tuple().one(&db).await?;
assert_eq!(id, 4);

let kim = customer::Entity::find_by_id(1).one(&db).await?;
assert_eq!(kim.name, "Kimberly");
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

`OnConflict::do_nothing()` skips a conflicting row whatever it conflicts on,
and [`OnConflict::constraint`](crate::pgorm_query::OnConflict::constraint)
names the arbiter by its constraint rather than its columns. A target named
by columns takes a `WHERE` with `and_where` before its action, the predicate
of a partial unique index, and an update takes one after it, choosing which
conflicting rows to update.

`RETURNING` reads the rows a write touched:
[`returning_col`](crate::pgorm_query::InsertStatement::returning_col) one
column, [`returning_all`](crate::pgorm_query::InsertStatement::returning_all)
every one, and [`Query::returning`](crate::pgorm_query::Query::returning) a
list. PostgreSQL 18 also lets a `RETURNING` list read the row as it was before
the write and as it is after it, which
[`ReturningRow`](crate::pgorm_query::ReturningRow) names:

```rust
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{Query, ReturningRow};
use pgorm_guide_support::cake;

let query = Query::update()
    .table(cake::Entity)
    .value(cake::Column::Price, Expr::col(cake::Column::Price).mul(2))
    .and_where(Expr::col(cake::Column::Id).eq(1))
    .returning(Query::returning().columns([
        (ReturningRow::Old, cake::Column::Price),
        (ReturningRow::New, cake::Column::Price),
    ]))
    .to_owned();

assert_eq!(
    query.to_string(),
    r#"UPDATE "cake" SET "price" = "price" * 2 WHERE "id" = 1 RETURNING old."price", new."price""#
);
```

## UPDATE … FROM and DELETE … USING

An `UPDATE` can read other tables with
[`from`](crate::pgorm_query::UpdateStatement::from), and a `DELETE` with
[`using`](crate::pgorm_query::DeleteStatement::using), each joined in the
`WHERE` clause. Neither takes `ORDER BY` or `LIMIT`, which PostgreSQL refuses
on both; to write some rows and not others, filter on a subquery.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("query_builder_from_using").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::Query;
use pgorm::ValueHolder;
use pgorm_guide_support::{bakery, cake, order, order_line};

// Every cake sold in Bergen goes up by fifty cents.
let (sql, values) = Query::update()
    .table(cake::Entity)
    .value(cake::Column::Price, Expr::col((cake::Entity, cake::Column::Price)).add(Decimal::new(50, 2)))
    .from(bakery::Entity)
    .and_where(Expr::col((cake::Entity, cake::Column::BakeryId)).equals((bakery::Entity, bakery::Column::Id)))
    .and_where(Expr::col((bakery::Entity, bakery::Column::City)).eq("Bergen"))
    .build();
assert_eq!(
    sql,
    r#"UPDATE "cake" SET "price" = "cake"."price" + $1 FROM "bakery" WHERE "cake"."bakery_id" = "bakery"."id" AND "bakery"."city" = $2"#
);
assert_eq!(db.execute_raw(&sql, values.into_iter().map(ValueHolder)).await?, 2);

// Lee's order lines go.
let (sql, values) = Query::delete()
    .from_table(order_line::Entity)
    .using(order::Entity)
    .and_where(Expr::col((order_line::Entity, order_line::Column::OrderId)).equals((order::Entity, order::Column::Id)))
    .and_where(Expr::col((order::Entity, order::Column::CustomerId)).eq(2))
    .build();
assert_eq!(
    sql,
    r#"DELETE FROM "order_line" USING "order" WHERE "order_line"."order_id" = "order"."id" AND "order"."customer_id" = $1"#
);
assert_eq!(db.execute_raw(&sql, values.into_iter().map(ValueHolder)).await?, 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## MERGE

`MERGE` writes a table from a second relation, row by row: each row of the
target the condition pairs with a source row is *matched*, each source row
with no target row is *not matched*, and each target row with no source row
is *not matched by source*. Each kind takes its own arms, each arm an action
and an optional condition of its own, and a row takes the first arm whose
condition holds. A matched row can be updated or deleted, an unmatched
source row inserted.

[`Query::merge`](crate::pgorm_query::Query::merge) takes the target, the
source and the condition, and the statement exists from its first arm on, so
a `MERGE` with no arm cannot be built. An arm that cannot apply to its kind
of row, such as an insert for a matched row, does not compile.

Here the nightly price list for Sunrise holds two rows, Chocolate Fudge at
4.80 and Lemon Drizzle at 3.20. A cake on the list whose price differs takes
the new price, and a Sunrise cake missing from the list is withdrawn:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("query_builder_merge").await;
# let db = scratch.pool.get().await?;
# db.batch_execute("CREATE TABLE price_list (name text PRIMARY KEY, new_price numeric(10, 2) NOT NULL); \
#     INSERT INTO price_list VALUES ('Chocolate Fudge', 4.80), ('Lemon Drizzle', 3.20)").await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{MatchedAction, MergeUpdate, Query};
use pgorm_guide_support::cake;

#[derive(SqlName)]
enum PriceList {
    Table,
    Name,
    NewPrice,
}

let (sql, values) = Query::merge(
    cake::Entity,
    PriceList::Table,
    Expr::col((cake::Entity, cake::Column::Name)).equals((PriceList::Table, PriceList::Name)),
)
.when_matched_and(
    Expr::col((cake::Entity, cake::Column::Price)).ne(Expr::col((PriceList::Table, PriceList::NewPrice))),
    MergeUpdate::value(cake::Column::Price, Expr::col((PriceList::Table, PriceList::NewPrice))),
)
.when_not_matched_by_source_and(
    Expr::col((cake::Entity, cake::Column::BakeryId)).eq(1),
    MatchedAction::Delete,
)
.returning_action()
.returning(Query::returning().column((cake::Entity, cake::Column::Name)))
.build();

assert_eq!(
    sql,
    [
        r#"MERGE INTO "cake" USING "price_list" ON "cake"."name" = "price_list"."name""#,
        r#"WHEN MATCHED AND "cake"."price" <> "price_list"."new_price" THEN UPDATE SET "price" = "price_list"."new_price""#,
        r#"WHEN NOT MATCHED BY SOURCE AND "cake"."bakery_id" = $1 THEN DELETE"#,
        r#"RETURNING merge_action(), "cake"."name""#,
    ]
    .join(" ")
);

let mut changes: Vec<(String, String)> = (sql, values).into_tuple().all(&db).await?;
changes.sort();
assert_eq!(
    changes,
    [
        ("DELETE".to_owned(), "Chocolate Torte".to_owned()),
        ("UPDATE".to_owned(), "Chocolate Fudge".to_owned()),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

Lemon Drizzle matched but its price had not changed, and no arm applied, so
it was left as it was. [`returning_action`](crate::pgorm_query::MergeStatement::returning_action)
adds `merge_action()`, the action each row took, as the first column. The
source can be any relation a `FROM` takes, a subquery or a `VALUES` list
among them; a value in the source is bound like any other, and since the
source assigns it to no column, it may need a cast to the type it stands for.

## WITH

A [`WithClause`](crate::pgorm_query::WithClause) names queries a statement
reads like tables, each a
[`CommonTableExpression`](crate::pgorm_query::CommonTableExpression). Its
query can be a `SELECT` or a write with a `RETURNING` clause, which then runs
once and hands its rows to the rest of the statement.

```rust
use pgorm::pgorm_query::{CommonTableExpression, Expr, ExprTrait, Func, Query, WithClause, alias};
use pgorm_guide_support::{cake, order_line};

let (sold, units) = (alias("sold"), alias("units"));
let query = Query::select()
    .column((cake::Entity, cake::Column::Name))
    .column((sold, units))
    .from(cake::Entity)
    .inner_join(sold, Expr::col((cake::Entity, cake::Column::Id)).equals((sold, order_line::Column::CakeId)))
    .with(WithClause::new(CommonTableExpression::new(
        sold,
        Query::select()
            .column(order_line::Column::CakeId)
            .expr_as(Func::sum(Expr::col(order_line::Column::Quantity)), units)
            .from(order_line::Entity)
            .group_by_col(order_line::Column::CakeId)
            .to_owned(),
    )))
    .to_owned();

assert_eq!(
    query.to_string(),
    [
        r#"WITH "sold" AS (SELECT "cake_id", SUM("quantity") AS "units" FROM "order_line" GROUP BY "cake_id")"#,
        r#"SELECT "cake"."name", "sold"."units" FROM "cake""#,
        r#"INNER JOIN "sold" ON "cake"."id" = "sold"."cake_id""#,
    ]
    .join(" ")
);
```

A [`RecursiveWithClause`](crate::pgorm_query::RecursiveWithClause) holds one
query that reads its own rows: a starting query, `UNION ALL`, and a query
over the rows so far, repeated until it adds none. It also takes
`SEARCH` and `CYCLE` clauses for walking graphs. The days from the first of
November to the fourth, each with the orders placed on it, the days without
an order included:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("query_builder_recursive").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{
    CommonTableExpression, Name, Order, Query, RecursiveWithClause, UnionType, alias,
};
use pgorm_guide_support::order;

let (days, day) = (alias("days"), alias("day"));
let date = || Name::runtime("date");
let calendar = Query::select()
    .expr_as(Expr::val(Date::constant(2026, 11, 1)).cast_as(date()), day)
    .union(
        UnionType::All,
        Query::select()
            .expr(Expr::col(day).add(1))
            .from(days)
            .and_where(Expr::col(day).lt(Expr::val(Date::constant(2026, 11, 4)).cast_as(date())))
            .to_owned(),
    )
    .to_owned();

let query = Query::select()
    .column((days, day))
    .expr(Expr::col((order::Entity, order::Column::Id)).count())
    .from(days)
    .left_join(
        order::Entity,
        Expr::col((order::Entity, order::Column::PlacedAt)).cast_as(date()).equals((days, day)),
    )
    .group_by_col((days, day))
    .order_by((days, day), Order::Asc)
    .with(RecursiveWithClause::new(CommonTableExpression::new(days, calendar).column(day).to_owned()))
    .to_owned();

let rows: Vec<(Date, i64)> = query.into_tuple().all(&db).await?;
assert_eq!(
    rows,
    [
        (Date::constant(2026, 11, 1), 0),
        (Date::constant(2026, 11, 2), 1),
        (Date::constant(2026, 11, 3), 1),
        (Date::constant(2026, 11, 4), 0),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The starting row's date and the `1` added to each day are cast because
nothing around them gives them a type: the first stands in a `SELECT` list,
and the second beside a date, which PostgreSQL can add an integer or an
interval to.

## Window functions

A window function computes a value for each row from the rows around it.
[`expr_window_as`](crate::pgorm_query::SelectStatement::expr_window_as) takes
the function, a [`WindowStatement`](crate::pgorm_query::WindowStatement) and a
name. The window partitions the rows, orders them within each partition, and
can narrow them to a frame: [`FrameType`](crate::pgorm_query::FrameType)
starts one, `ROWS`, `RANGE` or `GROUPS`, and its bounds follow. Each cake
beside the running total of its bakery's prices, cheapest first:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("query_builder_window").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{FrameType, Order, OrderedStatement, Query, WindowStatement, alias};
use pgorm_guide_support::cake;

let query = Query::select()
    .column(cake::Column::Name)
    .expr_window_as(
        Func::sum(Expr::col(cake::Column::Price)),
        WindowStatement::partition_by(cake::Column::BakeryId)
            .order_by(cake::Column::Price, Order::Asc)
            .frame(FrameType::Rows.unbounded_preceding().and_current_row())
            .to_owned(),
        alias("running"),
    )
    .from(cake::Entity)
    .and_where(Expr::col(cake::Column::BakeryId).eq(1))
    .order_by(cake::Column::Price, Order::Asc)
    .to_owned();

assert_eq!(
    query.to_string(),
    [
        r#"SELECT "name", SUM("price") OVER ( PARTITION BY "bakery_id" ORDER BY "price" ASC"#,
        r#"ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW ) AS "running""#,
        r#"FROM "cake" WHERE "bakery_id" = 1 ORDER BY "price" ASC"#,
    ]
    .join(" ")
);

let rows: Vec<(String, Decimal)> = query.into_tuple().all(&db).await?;
assert_eq!(
    rows,
    [
        ("Lemon Drizzle".to_owned(), Decimal::new(320, 2)),
        ("Chocolate Fudge".to_owned(), Decimal::new(770, 2)),
        ("Chocolate Torte".to_owned(), Decimal::new(1370, 2)),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A window used by several columns can be named once in the statement's
`WINDOW` clause, with [`window`](crate::pgorm_query::SelectStatement::window),
and each column refers to it by name with
[`expr_window_name_as`](crate::pgorm_query::SelectStatement::expr_window_name_as). The [pipeline](crate::guide::pipeline) writes windows in fewer
lines, when its shape fits.

## Grouping sets

A `GROUP BY` list can group by several sets of columns in one query.
[`GroupingElement`](crate::pgorm_query::GroupingElement) builds them:
[`rollup`](crate::pgorm_query::GroupingElement::rollup) for subtotals down a
hierarchy, [`cube`](crate::pgorm_query::GroupingElement::cube) for every
combination, and [`sets`](crate::pgorm_query::GroupingElement::sets) for sets
of your choosing. A row grouped by a set that leaves a column out holds
`NULL` there, and [`Func::grouping`](crate::pgorm_query::Func::grouping) tells
that `NULL` from one in the data. The cakes counted by bakery and by whether
they are gluten-free, with a subtotal per bakery and a grand total:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("query_builder_grouping").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{GroupingElement, Order, Query};
use pgorm_guide_support::cake;

let query = Query::select()
    .column(cake::Column::BakeryId)
    .column(cake::Column::GlutenFree)
    .expr(Expr::col(cake::Column::Id).count())
    .from(cake::Entity)
    .group_by_element(GroupingElement::rollup([
        Expr::col(cake::Column::BakeryId),
        Expr::col(cake::Column::GlutenFree),
    ]))
    .order_by(cake::Column::BakeryId, Order::Asc)
    .order_by(cake::Column::GlutenFree, Order::Asc)
    .to_owned();

assert_eq!(
    query.to_string(),
    [
        r#"SELECT "bakery_id", "gluten_free", COUNT("id") FROM "cake""#,
        r#"GROUP BY ROLLUP ("bakery_id", "gluten_free")"#,
        r#"ORDER BY "bakery_id" ASC, "gluten_free" ASC"#,
    ]
    .join(" ")
);

let rows: Vec<(Option<i32>, Option<bool>, i64)> = query.into_tuple().all(&db).await?;
assert_eq!(
    rows,
    [
        (Some(1), Some(false), 1),
        (Some(1), Some(true), 2),
        (Some(1), None, 3),
        (Some(2), Some(false), 1),
        (Some(2), Some(true), 1),
        (Some(2), None, 2),
        (None, None, 5),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Table functions

A function that returns rows, such as `generate_series` or `unnest`, can be
read as a table. [`TableFunction`](crate::pgorm_query::TableFunction) wraps
the call, names its columns, and can add `WITH ORDINALITY`, a last column
numbering the rows from 1; [`alias`](crate::pgorm_query::TableFunction::alias)
finishes it as something `from` and the joins take.
[`Func::generate_series`](crate::pgorm_query::Func::generate_series),
[`Func::unnest`](crate::pgorm_query::Func::unnest) and the JSON record
functions are built in, and any other through `Func::named`.

Unnesting a list of ids with its ordinal reads rows back in the order the
program asked for them. `unnest` takes an array of any type, so the bound
array carries its own, `int4[]`:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("query_builder_table_function").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{Order, Query, TableFunction, alias};
use pgorm_guide_support::cake;

let (wanted, id, position) = (alias("wanted"), alias("id"), alias("position"));
let query = Query::select()
    .column((cake::Entity, cake::Column::Name))
    .from(
        TableFunction::new(Func::unnest(Expr::val(vec![4, 1, 3])))
            .with_ordinality()
            .columns([id, position])
            .alias(wanted),
    )
    .inner_join(cake::Entity, Expr::col((cake::Entity, cake::Column::Id)).equals((wanted, id)))
    .order_by((wanted, position), Order::Asc)
    .to_owned();

assert_eq!(
    query.build().0,
    [
        r#"SELECT "cake"."name" FROM UNNEST($1::int4[]) WITH ORDINALITY AS "wanted"("id", "position")"#,
        r#"INNER JOIN "cake" ON "cake"."id" = "wanted"."id" ORDER BY "wanted"."position" ASC"#,
    ]
    .join(" ")
);

let names: Vec<(String,)> = query.into_tuple().all(&db).await?;
assert_eq!(
    names,
    [
        ("Vanilla Sponge".to_owned(),),
        ("Chocolate Fudge".to_owned(),),
        ("Carrot Cake".to_owned(),),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## SQL/JSON

PostgreSQL's SQL/JSON functions query a JSON document with a path:
[`Func::json_exists`](crate::pgorm_query::Func::json_exists) tests that the
path finds something, [`Func::json_value`](crate::pgorm_query::Func::json_value)
reads one scalar, [`Func::json_query`](crate::pgorm_query::Func::json_query)
reads JSON, and [`Func::json_table`](crate::pgorm_query::Func::json_table)
turns what a path finds into rows. The constructors `json_object`,
`json_array`, `json_objectagg` and `json_arrayagg` build JSON, and
[`is_json`](crate::pgorm_query::ExprTrait::is_json) tests text. A path is bound as
a value, except in `JSON_TABLE`, whose path the server takes only as a
literal; a value the path needs is passed with `passing`, and bound.

An order arriving as JSON, read as rows of cake and quantity:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("query_builder_json").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{Asterisk, JsonTableColumn, Query, alias};

let incoming = serde_json::json!({
    "customer": "kim@example.com",
    "lines": [{ "cake": 2, "qty": 3 }, { "cake": 4, "qty": 1 }],
});

let query = Query::select()
    .column(Asterisk)
    .from(
        Func::json_table(
            Expr::val(incoming),
            "$.lines[*]",
            JsonTableColumn::value(alias("cake_id"), ColumnType::Integer).path("$.cake"),
        )
        .column(JsonTableColumn::value(alias("quantity"), ColumnType::Integer).path("$.qty"))
        .alias(alias("line")),
    )
    .to_owned();

assert_eq!(
    query.build().0,
    [
        r#"SELECT * FROM JSON_TABLE($1::jsonb, '$.lines[*]' COLUMNS"#,
        r#"("cake_id" integer PATH '$.cake', "quantity" integer PATH '$.qty')) AS "line""#,
    ]
    .join(" ")
);

let lines: Vec<(i32, i32)> = query.into_tuple().all(&db).await?;
assert_eq!(lines, [(2, 3), (4, 1)]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The operators PostgreSQL has for `jsonb`, such as `->`, `->>` and `?`, are
methods on `Expr` too:
[`get_json_field`](crate::pgorm_query::ExprTrait::get_json_field),
[`cast_json_field`](crate::pgorm_query::ExprTrait::cast_json_field) and
[`has_json_key`](crate::pgorm_query::ExprTrait::has_json_key) among them, and
[`contains`](crate::pgorm_query::ExprTrait::contains) is `@>`.

## Ranges

A [`Range`](crate::pgorm_query::Range) value binds as one of PostgreSQL's
range types, and the range operators are methods of every expression:
[`contains`](crate::pgorm_query::ExprTrait::contains) (`@>`),
[`contained`](crate::pgorm_query::ExprTrait::contained) (`<@`) and
[`overlaps`](crate::pgorm_query::ExprTrait::overlaps) (`&&`). Beside a range
column a bound range takes the column's type. Here the range contains a
timestamp, which tells the server nothing about the range, so the bound
range carries its own type, `tstzrange`. The orders placed on the second of
November:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("query_builder_ranges").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{Query, Range};
use pgorm_guide_support::order;

let start: DateTimeWithTimeZone = "2026-11-02T00:00:00Z".parse()?;
let end: DateTimeWithTimeZone = "2026-11-03T00:00:00Z".parse()?;
let query = Query::select()
    .column(order::Column::Id)
    .from(order::Entity)
    .and_where(
        Expr::val(Range::from(start..end)).contains(Expr::col(order::Column::PlacedAt)),
    )
    .to_owned();

assert_eq!(
    query.build().0,
    r#"SELECT "id" FROM "order" WHERE $1::tstzrange @> "placed_at""#
);

let ids: Vec<(i32,)> = query.into_tuple().all(&db).await?;
assert_eq!(ids, [(1,)]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

[Values and types](crate::guide::values) covers ranges and multiranges as column
types, including range types a schema creates.

## Locking

A `SELECT` can lock the rows it reads:
[`lock_exclusive`](crate::pgorm_query::SelectStatement::lock_exclusive) for
`FOR UPDATE`, [`lock_shared`](crate::pgorm_query::SelectStatement::lock_shared)
for `FOR SHARE`, and [`lock_with_behavior`](crate::pgorm_query::SelectStatement::lock_with_behavior)
for `NOWAIT` or `SKIP LOCKED`, the usual way for several workers to take
jobs from one queue table. A lock lasts until the transaction ends, so it
belongs inside one ([Transactions](crate::guide::transactions)).

```rust
use pgorm::pgorm_query::{Expr, ExprTrait, LockBehavior, LockType, Query};
use pgorm_guide_support::order;

let query = Query::select()
    .column(order::Column::Id)
    .from(order::Entity)
    .and_where(Expr::col(order::Column::BakeryId).eq(1))
    .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
    .to_owned();

assert_eq!(
    query.to_string(),
    r#"SELECT "id" FROM "order" WHERE "bakery_id" = 1 FOR UPDATE SKIP LOCKED"#
);
```
