# Schema generation

A program's tables have to exist before it can use them. pgorm writes the
DDL that creates them in two ways: from the entities you already declared, and
from DDL builders for everything an entity does not describe, such as views,
sequences or a change to an existing table. Both only build statements. You
run each one yourself, with
[`batch_execute`](crate::ConnectionTrait::batch_execute) and the statement's
text, usually frozen into a [migration](crate::guide::migrations) when the
change is made, since a statement generated from an entity follows the entity
as it changes ([Generated DDL in a
migration](crate::guide::migrations#generated-ddl-in-a-migration)).

Schema generation writes; it does not read. pgorm never inspects a database
to compare it with the entities.

## Tables from entities

[`Schema`](crate::Schema) turns an entity into the statements that create its
table: [`create_table_from_entity`](crate::Schema::create_table_from_entity)
for the table, with its primary key, its unique keys and a foreign key for
each `belongs_to` relation; [`create_enum_from_entity`](crate::Schema::create_enum_from_entity)
for the PostgreSQL enums its columns use;
[`create_index_from_entity`](crate::Schema::create_index_from_entity) for its
`indexed` columns; and [`create_comments_from_entity`](crate::Schema::create_comments_from_entity)
for its comments.

Types come before the tables that use them, and a table before the tables
whose foreign keys point at it. This creates the bakery's first three tables
on an empty database:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("schema_from_entities").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::Schema;
use pgorm_guide_support::{bakery, cake};

let schema = Schema::new();

let types = schema.create_enum_from_entity(cake::Entity);
assert_eq!(
    types[0].to_string(),
    r#"CREATE TYPE "flavor" AS ENUM ('chocolate', 'vanilla', 'lemon', 'carrot')"#
);

let bakeries = schema.create_table_from_entity(bakery::Entity)?;
let cakes = schema.create_table_from_entity(cake::Entity)?;
assert_eq!(
    cakes.to_string(),
    r#"CREATE TABLE "cake" ( "id" serial NOT NULL, "name" varchar NOT NULL, "flavor" flavor NOT NULL, "price" decimal(10, 2) NOT NULL, "gluten_free" bool NOT NULL, "bakery_id" integer NOT NULL, PRIMARY KEY ("id"), CONSTRAINT "fk-cake-bakery_id" FOREIGN KEY ("bakery_id") REFERENCES "bakery" ("id") )"#
);

for create in &types {
    db.batch_execute(&create.to_string()).await?;
}
db.batch_execute(&bakeries.to_string()).await?;
db.batch_execute(&cakes.to_string()).await?;

// The foreign key holds: a cake needs its bakery.
let orphan = cake::ActiveModel {
    name: set("Nowhere Cake"),
    flavor: set(cake::Flavor::Vanilla),
    price: set(Decimal::ONE),
    gluten_free: set(false),
    bakery_id: set(99),
    ..Default::default()
}
.insert(&db)
.await;
assert!(matches!(orphan.unwrap_err().sql_error(), Some(SqlError::ForeignKeyConstraintViolation(_))));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The names generation gives are fixed: a composite primary key is
`pk-{table}`, a foreign key `fk-{table}-{columns}`, an index
`idx-{table}-{column}`. A relation's `fk_name` names its foreign key yourself.
A derived name longer than PostgreSQL's 63 bytes is shortened to 47 bytes and
a hash, so two long names never collide.

`create_table_from_entity` returns a `Result`. It refuses an entity it cannot
write a table for: a unique key with no columns, which only a hand-written
`EntityName` can describe, and a `belongs_to` relation with a column typed as
a domain over an enum on either side, a foreign key PostgreSQL cannot build.
The refusal is [`Error::Type`](crate::Error::Type), naming the entity and the
relation.

## Enum types

An entity's enums come from its columns: one `CREATE TYPE .. AS ENUM` for
each enum a column holds, an array of an enum included, each written once
however many columns use it. [`create_enum_from_active_enum`](crate::Schema::create_enum_from_active_enum)
writes one enum's type directly, and refuses an `ActiveEnum` that is not a
PostgreSQL enum:

```rust
use pgorm::entity::prelude::*;
use pgorm::Schema;
use pgorm_guide_support::cake::Flavor;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ActiveEnum)]
#[pgorm(rs_type = i16, db_type = ColumnType::SmallInteger)]
pub enum Shelf {
    #[pgorm(num_value = 1)]
    Top,
    #[pgorm(num_value = 2)]
    Bottom,
}

let schema = Schema::new();
assert_eq!(
    schema.create_enum_from_active_enum::<Flavor>()?.to_string(),
    r#"CREATE TYPE "flavor" AS ENUM ('chocolate', 'vanilla', 'lemon', 'carrot')"#
);
assert!(matches!(schema.create_enum_from_active_enum::<Shelf>(), Err(Error::Type(_))));
# Ok::<(), pgorm::Error>(())
```

PostgreSQL has no `CREATE TYPE IF NOT EXISTS`, so creating a type twice is
an error; run each enum's statement once. Adding a value later is
`ALTER TYPE .. ADD VALUE`, which the [types](#types) builder writes.

## Indexes and comments

A column marked `indexed` gets an index of its own, and `comment` on the
`Model` or on a field becomes a `COMMENT ON`. Both are statements apart from
the `CREATE TABLE`, and run after it:

```rust
mod recipe {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "recipe", comment = "How each cake is made")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(indexed)]
        pub cake_id: i32,
        #[pgorm(comment = "Minutes in the oven")]
        pub bake_minutes: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("schema_indexes").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::Schema;

let schema = Schema::new();
let indexes: Vec<String> = schema
    .create_index_from_entity(recipe::Entity)
    .iter()
    .map(ToString::to_string)
    .collect();
assert_eq!(indexes, [r#"CREATE INDEX "idx-recipe-cake_id" ON "recipe" ("cake_id")"#]);

let comments: Vec<String> = schema
    .create_comments_from_entity(recipe::Entity)
    .iter()
    .map(ToString::to_string)
    .collect();
assert_eq!(
    comments,
    [
        r#"COMMENT ON TABLE "recipe" IS 'How each cake is made'"#,
        r#"COMMENT ON COLUMN "recipe"."bake_minutes" IS 'Minutes in the oven'"#,
    ]
);

db.batch_execute(&schema.create_table_from_entity(recipe::Entity)?.to_string()).await?;
for statement in indexes.iter().chain(&comments) {
    db.batch_execute(statement).await?;
}
let comment: Option<String> = db
    .query_one("SELECT obj_description('recipe'::regclass, 'pg_class')", &[])
    .await?
    .get(0);
assert_eq!(comment.as_deref(), Some("How each cake is made"));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

An index over several columns, a unique index, a partial index or one of
another method is the [index builder](#indexes)'s. A unique key over several
columns is the `Model`'s `unique(columns = (..))`, which [Entities](crate::guide::entities)
shows; it is part of the `CREATE TABLE`.

## Tables described as data

An entity is described at compile time. A table known only at run time, such
as one a program reads from configuration or a language binding's model
declares, is described as a [`TableShape`](crate::TableShape) instead: its
columns, each a name and the [`ColumnDef`](crate::ColumnDef) an entity's
column returns from `def()`, its primary key, unique keys, belongs-to
relations and comment. [`create_table_from_shape`](crate::Schema::create_table_from_shape),
`create_enum_from_shape`, `create_index_from_shape` and
`create_comments_from_shape` write the same statements the entity methods
write, and [`TableShape::of`](crate::TableShape::of) describes an entity, so
both come from one writer. pgorm's Python and JavaScript bindings write a
declared model's schema this way.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("schema_shapes").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{ForeignKeyAction, GeneratedKind, Name, TableName};
use pgorm::{ColumnPairs, ColumnTypeTrait, RelationDef, Schema, TableColumn, TableShape};

let table = |name: &str| TableName::Table(Name::runtime(name));
let price = || ColumnType::Decimal(Some((10, 2)));

let mut to_order = RelationDef::belongs_to(
    table("order_line"),
    table("order"),
    ColumnPairs::new(Name::runtime("order_id"), Name::runtime("id")),
);
to_order.on_delete = Some(ForeignKeyAction::Cascade);

let order_line = TableShape::new(
    table("order_line"),
    vec![
        TableColumn::new("id", ColumnType::Integer.def()),
        TableColumn::new("order_id", ColumnType::Integer.def().indexed()),
        TableColumn::new("quantity", ColumnType::Integer.def().default_value(1)),
        TableColumn::new("price", price().def()),
        TableColumn::new(
            "total",
            price()
                .def()
                .generated(Expr::col(Name::runtime("price")).mul(Expr::col(Name::runtime("quantity"))), GeneratedKind::Stored)
                .comment("price times quantity"),
        ),
    ],
)?
.with_primary_key(&["id"])?
.auto_increment()
.with_relation("Order", to_order)?;

let schema = Schema::new();
let create = schema.create_table_from_shape(&order_line)?;
assert_eq!(
    create.to_string(),
    r#"CREATE TABLE "order_line" ( "id" serial NOT NULL, "order_id" integer NOT NULL, "quantity" integer NOT NULL DEFAULT 1, "price" decimal(10, 2) NOT NULL, "total" decimal(10, 2) NOT NULL GENERATED ALWAYS AS ("price" * "quantity") STORED, PRIMARY KEY ("id"), CONSTRAINT "fk-order_line-order_id" FOREIGN KEY ("order_id") REFERENCES "order" ("id") ON DELETE CASCADE )"#
);

db.batch_execute(r#"CREATE TABLE "order" ( "id" serial PRIMARY KEY )"#).await?;
db.batch_execute(&create.to_string()).await?;
for index in schema.create_index_from_shape(&order_line) {
    db.batch_execute(&index.to_string()).await?;
}
for comment in schema.create_comments_from_shape(&order_line) {
    db.batch_execute(&comment.to_string()).await?;
}

// An entity's shape writes the entity's own statements.
let customer = TableShape::of::<pgorm_guide_support::customer::Entity>();
assert_eq!(
    schema.create_table_from_shape(&customer)?.to_string(),
    schema.create_table_from_entity(pgorm_guide_support::customer::Entity)?.to_string()
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A shape refuses what it cannot describe as it is built, with
[`Error::Type`](crate::Error::Type): two columns of one name, a key or unique
key naming a column it lacks, and a relation that holds no key of the table,
a `has_many` among them. A column built
[`with_unstated_default`](crate::TableColumn::with_unstated_default), which
the database fills by a default the description does not state, has no
`DEFAULT` to write, so `create_table_from_shape` refuses its table.

A `TableColumn` also reads, writes and compares as an entity's column of its
type does: [`select`](crate::TableColumn::select) reads a `money` column as
`numeric` and an enum as its text, `save` casts a value to the column's type,
and `compared_column` and `compared_value` give the two sides of a
comparison, both cast to the enum where the column is a domain over one.
`with_select_as` and `with_save_as` replace the read's and the write's cast,
as an entity column's `select_as` and `save_as` attributes do.
[`compared_array`](crate::TableColumn::compared_array) is the array `= ANY`
and `<> ALL` compare the column with, as an entity column's `eq_any` casts
it, and [`read_cast`](crate::TableColumn::read_cast) and
[`compared_value_cast`](crate::TableColumn::compared_value_cast) name the
types the read and a compared value are cast to. A
[pipeline](crate::guide::pipeline) reads such a table through them:
[`col_shaped`](crate::pipeline::col_shaped) is a column compared as its
`TableColumn` compares, and
[`read_columns`](crate::pipeline::Pipeline::read_columns) reads the rows as
a reader of those columns does, each cast column projected cast, as
`into_model` reads an entity's:

```rust
use pgorm::pgorm_query::{ColumnType, Expr, Query};
use pgorm::pipeline::{ExprOps, Pipeline, alias, col_shaped};
use pgorm::{ColumnTypeTrait, TableColumn};

let takings = TableColumn::new("takings", ColumnType::Money.def());
assert_eq!(
    Query::select().expr(takings.compared_array(Expr::val(vec![1.5]))).to_string(),
    "SELECT CAST(ARRAY [1.5] AS money[])"
);

let (sql, _) = Pipeline::from(alias("till"))
    .filter(col_shaped(alias("till"), alias("takings"), &takings).gt(10))
    .read_columns([("id".to_owned(), None), ("takings".to_owned(), takings.read_cast())])
    .into_sql()?;
assert_eq!(
    sql,
    "SELECT id, CAST(takings AS numeric) AS takings FROM till WHERE takings > CAST(10 AS money)"
);
# Ok::<_, pgorm::pipeline::PipelineError>(())
```

## The DDL builders

The [query builder](crate::guide::query_builder), pgorm-query, has a builder for each
DDL statement PostgreSQL has a use for here: tables, indexes, foreign keys,
types, domains, sequences, views, materialized views, schemas, extensions
and comments. Each is started by a unit struct (`Table::create`,
`Index::create`, `Type::create`, ...), takes its names as typed identifiers,
and quotes every one it writes.

Names come from anything that implements
[`SqlName`](trait@crate::SqlName). An entity and its `Column` do, and so does an
enum deriving `SqlName`, whose `Table` variant is the type's own name. That is
how a migration names a table no entity describes:

```rust
use pgorm::SqlName;

#[derive(SqlName)]
enum Delivery {
    Table,
    Id,
    OrderId,
    Address,
    DueAt,
    Fragile,
    Notes,
}

assert_eq!(Delivery::Table.to_string(), "delivery");
assert_eq!(Delivery::DueAt.to_string(), "due_at");
```

The examples below build the bakery's deliveries with these names. The
statements take their values inline, as literals, because PostgreSQL takes no
parameters in DDL, so `to_string()` is the statement to run.

Two of the names differ from pgorm's: `pgorm::Schema` generates DDL from
entities, while `pgorm::pgorm_query::Schema` is the builder for
`CREATE SCHEMA`.

### Tables

[`Table::create`](crate::pgorm_query::Table::create) takes the table's name
and then its columns, keys, foreign keys and checks.
[`ColumnDef`](crate::pgorm_query::ColumnDef) describes a column: its type, then
`not_null`, `default`, `identity`, `generated` and the rest.

```rust
# use pgorm::SqlName;
# #[derive(SqlName)]
# enum Delivery { Table, Id, OrderId, Address, DueAt, Fragile, Notes }
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("schema_tables").await;
# let db = scratch.pool.get().await?;
use pgorm::ConnectionTrait;
use pgorm::pgorm_query::{ColumnDef, Expr, ExprTrait, ForeignKey, ForeignKeyAction, Table};
use pgorm_guide_support::order;

let create = Table::create(Delivery::Table)
    .col(ColumnDef::new(Delivery::Id).integer().not_null().identity())
    .col(ColumnDef::new(Delivery::OrderId).integer().not_null())
    .col(ColumnDef::new(Delivery::Address).text().not_null())
    .col(ColumnDef::new(Delivery::DueAt).timestamp_with_time_zone().not_null())
    .col(ColumnDef::new(Delivery::Fragile).boolean().not_null().default(false))
    .primary_key(Delivery::Id)
    .foreign_key(
        ForeignKey::create(Delivery::Table, Delivery::OrderId, order::Entity, order::Column::Id)
            .on_delete(ForeignKeyAction::Cascade)
            .to_owned(),
    )
    .check(Expr::col(Delivery::Address).ne(""))
    .to_owned();
assert_eq!(
    create.to_string(),
    [
        r#"CREATE TABLE "delivery" ("#,
        r#""id" integer NOT NULL GENERATED ALWAYS AS IDENTITY,"#,
        r#""order_id" integer NOT NULL,"#,
        r#""address" text NOT NULL,"#,
        r#""due_at" timestamp with time zone NOT NULL,"#,
        r#""fragile" bool NOT NULL DEFAULT FALSE,"#,
        r#"PRIMARY KEY ("id"),"#,
        r#"FOREIGN KEY ("order_id") REFERENCES "order" ("id") ON DELETE CASCADE,"#,
        r#"CHECK ("address" <> '')"#,
        r#")"#,
    ]
    .join(" ")
);
db.batch_execute(&create.to_string()).await?;

// A column added later, and the table renamed.
let alter = Table::alter(Delivery::Table)
    .add_column(ColumnDef::new(Delivery::Notes).text())
    .to_owned();
assert_eq!(alter.to_string(), r#"ALTER TABLE "delivery" ADD COLUMN "notes" text"#);
db.batch_execute(&alter.to_string()).await?;

let rename = Table::rename(Delivery::Table, pgorm::alias("courier_run"));
assert_eq!(rename.to_string(), r#"ALTER TABLE "delivery" RENAME TO "courier_run""#);
db.batch_execute(&rename.to_string()).await?;

let drop_table = Table::drop(pgorm::alias("courier_run")).if_exists().to_owned();
assert_eq!(drop_table.to_string(), r#"DROP TABLE IF EXISTS "courier_run""#);
db.batch_execute(&drop_table.to_string()).await?;
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

`Table::alter` also changes a column's type (`alter_column_type_using`),
drops a column or a default, adds and drops constraints, and validates one
added `NOT VALID`. `Table::rename_column` and `Table::truncate` do what they
say. A [generated column](crate::guide::entities) is `ColumnDef::generated(expr,
GeneratedKind::Stored)`, or `Virtual`.

### Foreign keys and their timing

A foreign key is checked at the end of each statement unless it says
otherwise. [`Deferrability`](crate::pgorm_query::Deferrability) moves the
check: `DeferrableInitiallyDeferred` checks it once, at commit, which lets a
transaction write rows in an order no single statement could, such as two
rows that point at each other. PostgreSQL 18 also takes
[`Enforcement::NotEnforced`](crate::pgorm_query::Enforcement): a key that is
recorded, for the people and tools reading the schema, but never checked.

The same two settings are on an entity's relation, as `deferrability = ..`
and `enforcement = ..`. A key added to an existing table is
`Table::alter(..).add_foreign_key(..)`:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("schema_foreign_keys").await;
# let mut db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{Deferrability, Table, TableForeignKey};
use pgorm::TransactionTrait;
use pgorm_guide_support::{baker, bakery};

// Each bakery names its head baker, and each baker their bakery.
db.batch_execute(r#"ALTER TABLE "bakery" ADD COLUMN "head_baker_id" integer"#).await?;
let head_baker = Table::alter(bakery::Entity)
    .add_foreign_key(
        TableForeignKey::new(bakery::Entity, alias("head_baker_id"), baker::Entity, baker::Column::Id)
            .deferrability(Deferrability::DeferrableInitiallyDeferred)
            .to_owned(),
    )
    .to_owned();
assert_eq!(
    head_baker.to_string(),
    r#"ALTER TABLE "bakery" ADD FOREIGN KEY ("head_baker_id") REFERENCES "baker" ("id") DEFERRABLE INITIALLY DEFERRED"#
);
db.batch_execute(&head_baker.to_string()).await?;

// Point the bakery at a baker who does not exist yet: the key is checked at
// commit, by which time the baker does.
db.transaction(async |txn| {
    txn.execute(r#"UPDATE "bakery" SET "head_baker_id" = 4 WHERE "id" = 1"#, &[]).await?;
    baker::ActiveModel {
        name: set("Di"),
        hired_on: set(jiff::civil::date(2026, 11, 1)),
        bakery_id: set(1),
        ..Default::default()
    }
    .insert(&txn)
    .await?;
    Ok::<_, Error>(())
})
.await?;
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

### Indexes

[`Index::create`](crate::pgorm_query::Index::create) takes the table and its
first column; `col` adds more. `unique`, `nulls_not_distinct`, `include` (a
covering index's extra columns), `index_type` (`Gin`, `Hash` or a named
method) and `and_where` (a partial index) say what kind of index it is:

```rust
# use pgorm::SqlName;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("schema_index_builder").await;
# let db = scratch.pool.get().await?;
use pgorm::ConnectionTrait;
use pgorm::pgorm_query::{Expr, ExprTrait, Index, Name};
use pgorm_guide_support::cake;

// One cake of each name per bakery.
let unique = Index::create(cake::Entity, cake::Column::BakeryId)
    .col(cake::Column::Name)
    .name(Name::runtime("uq-cake-bakery-name"))
    .unique()
    .to_owned();
assert_eq!(
    unique.to_string(),
    r#"CREATE UNIQUE INDEX "uq-cake-bakery-name" ON "cake" ("bakery_id", "name")"#
);
db.batch_execute(&unique.to_string()).await?;

// Only the gluten-free cakes, which the shop's filter asks for most.
let partial = Index::create(cake::Entity, cake::Column::Price)
    .name(Name::runtime("idx-cake-gluten-free-price"))
    .and_where(Expr::col(cake::Column::GlutenFree).eq(true))
    .to_owned();
assert_eq!(
    partial.to_string(),
    r#"CREATE INDEX "idx-cake-gluten-free-price" ON "cake" ("price") WHERE "gluten_free" = TRUE"#
);
db.batch_execute(&partial.to_string()).await?;

let drop_index = Index::drop(Name::runtime("idx-cake-gluten-free-price"));
assert_eq!(drop_index.to_string(), r#"DROP INDEX "idx-cake-gluten-free-price""#);
db.batch_execute(&drop_index.to_string()).await?;
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

### Types

[`Type::create`](crate::pgorm_query::extension::Type::create) creates an
enum (`as_enum` with `values`), a composite (`as_composite` with
`attribute`), or a range (`as_range`, which [Values and
types](crate::guide::values) uses). [`Type::alter`](crate::pgorm_query::extension::Type::alter)
adds an enum value, before or after another, renames one, or renames the
type:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("schema_types").await;
# let db = scratch.pool.get().await?;
use pgorm::ConnectionTrait;
use pgorm::pgorm_query::{ColumnType, Name, extension::Type};

let box_size = Type::create(Name::runtime("box_size"))
    .as_enum()
    .values(["small", "large"])
    .to_owned();
assert_eq!(box_size.to_string(), r#"CREATE TYPE "box_size" AS ENUM ('small', 'large')"#);
db.batch_execute(&box_size.to_string()).await?;

let medium = Type::alter(Name::runtime("box_size")).add_value("medium").before("large");
assert_eq!(medium.to_string(), r#"ALTER TYPE "box_size" ADD VALUE 'medium' BEFORE 'large'"#);
db.batch_execute(&medium.to_string()).await?;

let address = Type::create(Name::runtime("address"))
    .attribute(Name::runtime("street"), ColumnType::Text)
    .attribute(Name::runtime("city"), ColumnType::Text)
    .to_owned();
assert_eq!(address.to_string(), r#"CREATE TYPE "address" AS ("street" text, "city" text)"#);
db.batch_execute(&address.to_string()).await?;

let labels: String = db
    .query_one("SELECT enum_range(NULL::box_size)::text", &[])
    .await?
    .get(0);
assert_eq!(labels, "{small,medium,large}");
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A value added with `ADD VALUE` cannot be used in the transaction that adds
it. [Migrations](crate::guide::migrations) has a way to commit after such a
migration.

### Domains, sequences and schemas

[`Domain::create`](crate::pgorm_query::Domain::create) makes a domain, a type
with a default and checks ([Values and types](crate::guide::values) uses one as a
column). [`Sequence::create`](crate::pgorm_query::Sequence::create) makes a
counter of its own, beyond the ones an identity column keeps. The
`CREATE SCHEMA` builder is [`pgorm_query::Schema`](crate::pgorm_query::Schema):

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("schema_sequences").await;
# let db = scratch.pool.get().await?;
use pgorm::ConnectionTrait;
use pgorm::pgorm_query::{self, ColumnType, Domain, Expr, ExprTrait, Name, Sequence, SequenceOption, SequenceType};

let archive = pgorm_query::Schema::create(Name::runtime("archive")).if_not_exists().to_owned();
assert_eq!(archive.to_string(), r#"CREATE SCHEMA IF NOT EXISTS "archive""#);
db.batch_execute(&archive.to_string()).await?;

let receipt_no = Sequence::create((Name::runtime("archive"), Name::runtime("receipt_no")))
    .as_type(SequenceType::BigInteger)
    .options(SequenceOption::StartWith(1000))
    .to_owned();
assert_eq!(
    receipt_no.to_string(),
    r#"CREATE SEQUENCE "archive"."receipt_no" AS bigint START WITH 1000"#
);
db.batch_execute(&receipt_no.to_string()).await?;
let next: i64 = db.query_one("SELECT nextval('archive.receipt_no')", &[]).await?.get(0);
assert_eq!(next, 1000);

let percent = Domain::create(Name::runtime("percent"), ColumnType::SmallInteger)
    .check(Expr::domain_value().between(0, 100))
    .to_string();
assert_eq!(percent, r#"CREATE DOMAIN "percent" AS smallint CHECK (VALUE BETWEEN 0 AND 100)"#);
db.batch_execute(&percent).await?;
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

### Views and tables from a query

A view is a named query. [`View::create`](crate::pgorm_query::View::create)
takes its name and the query, built with the [query builder](crate::guide::query_builder);
a [`MaterializedView`](crate::pgorm_query::MaterializedView) also stores the
query's rows until it is refreshed, and
[`Table::create_as`](crate::pgorm_query::Table::create_as) makes an ordinary
table from them once. Each refuses a query PostgreSQL would not take in that
place, so construction returns a `Result`:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("schema_views").await;
# let db = scratch.pool.get().await?;
use pgorm::ConnectionTrait;
use pgorm::pgorm_query::{Expr, ExprTrait, Func, MaterializedView, Name, Query, View};
use pgorm_guide_support::{cake, order_line};

let gluten_free = View::create(
    Name::runtime("gluten_free_cake"),
    Query::select()
        .columns([cake::Column::Id, cake::Column::Name])
        .from(cake::Entity)
        .and_where(Expr::col(cake::Column::GlutenFree).eq(true))
        .take(),
)?
.or_replace()
.to_owned();
assert_eq!(
    gluten_free.to_string(),
    r#"CREATE OR REPLACE VIEW "gluten_free_cake" AS SELECT "id", "name" FROM "cake" WHERE "gluten_free" = TRUE"#
);
db.batch_execute(&gluten_free.to_string()).await?;

let sold = MaterializedView::create(
    Name::runtime("cakes_sold"),
    Query::select()
        .column(order_line::Column::CakeId)
        .expr(Func::sum(Expr::col(order_line::Column::Quantity)))
        .from(order_line::Entity)
        .group_by_col(order_line::Column::CakeId)
        .take(),
)?
.columns([Name::runtime("cake_id"), Name::runtime("sold")])
.to_owned();
db.batch_execute(&sold.to_string()).await?;
db.batch_execute(&MaterializedView::refresh(Name::runtime("cakes_sold")).to_string()).await?;

let fudge_sold: i64 = db
    .query_one("SELECT sold FROM cakes_sold WHERE cake_id = 1", &[])
    .await?
    .get(0);
assert_eq!(fudge_sold, 2);
let gluten_free_count: i64 = db.query_one("SELECT count(*) FROM gluten_free_cake", &[]).await?.get(0);
assert_eq!(gluten_free_count, 3);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

### Extensions and comments

[`Extension::create`](crate::pgorm_query::extension::Extension::create)
installs an extension the server has available, such as `btree_gist`, which a
temporal key over a scalar column needs.
[`Comment::on_table`](crate::pgorm_query::Comment::on_table) and
`Comment::on_column` write `COMMENT ON` for tables an entity does not
describe:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("schema_extensions").await;
# let db = scratch.pool.get().await?;
use pgorm::ConnectionTrait;
use pgorm::pgorm_query::{Comment, Name, extension::Extension};
use pgorm_guide_support::cake;

let btree_gist = Extension::create(Name::runtime("btree_gist")).if_not_exists().to_owned();
assert_eq!(btree_gist.to_string(), r#"CREATE EXTENSION IF NOT EXISTS "btree_gist""#);
db.batch_execute(&btree_gist.to_string()).await?;

let note = Comment::on_column(cake::Entity, cake::Column::Price, "In kroner, VAT included");
assert_eq!(
    note.to_string(),
    r#"COMMENT ON COLUMN "cake"."price" IS 'In kroner, VAT included'"#
);
db.batch_execute(&note.to_string()).await?;
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```
