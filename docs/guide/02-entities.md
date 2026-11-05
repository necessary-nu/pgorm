# Entities

An *entity* is a table, described as Rust types. Each one is a module holding
two items you write, `Model` and `Relation`, and several the derives write for
you. Everything that reads or writes the table starts from it.

This chapter declares the bakery's entities and goes through what each part
of a declaration can say. The other chapters use these entities.

## Declaring an entity

Here is the bakery's `cake` table:

```rust
# mod bakery { pub use pgorm_guide_support::bakery::*; }
# mod baker { pub use pgorm_guide_support::baker::*; }
# mod cake_baker { pub use pgorm_guide_support::cake_baker::*; }
# mod order_line {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "order_line")]
#     pub struct Model {
#         #[pgorm(primary_key)]
#         pub id: i32,
#         pub cake_id: i32,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {
#         #[pgorm(belongs_to = super::cake::Entity, from = Column::CakeId, to = super::cake::Column::Id)]
#         Cake,
#     }
# }
mod cake {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ActiveEnum)]
    pub enum Flavor {
        Chocolate,
        Vanilla,
        Lemon,
        Carrot,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "cake")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        pub flavor: Flavor,
        #[pgorm(column_type = ColumnType::Decimal(Some((10, 2))))]
        pub price: Decimal,
        pub gluten_free: bool,
        pub bakery_id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::bakery::Entity,
            from = Column::BakeryId,
            to = super::bakery::Column::Id
        )]
        Bakery,
        #[pgorm(has_many = super::order_line::Entity)]
        OrderLine,
    }

    impl Related<super::baker::Entity> for Entity {
        fn to() -> RelationDef {
            super::cake_baker::Relation::Baker.def()
        }

        fn via() -> Option<RelationDef> {
            Some(super::cake_baker::Relation::Cake.def().rev())
        }
    }
}

# fn main() {
use pgorm::entity::prelude::*;

assert_eq!(cake::Entity.table_name(), "cake");
let columns: Vec<String> = cake::Column::iter().map(|c| c.to_string()).collect();
assert_eq!(columns, ["id", "name", "flavor", "price", "gluten_free", "bakery_id"]);
# }
```

`Model` is a row: one field per column, each column named for its field.
`#[derive(Entity)]` on it writes the rest of the entity:

- `Entity`, a unit struct naming the table. Queries start from it:
  `cake::Entity::find()`.
- `Column`, an enum with one variant per field (`Column::GlutenFree` for
  `gluten_free`). Filters, orderings and projections name columns through it.
- `PrimaryKey`, an enum of the key's columns.
- `ActiveModel`, the `Model` with each field wrapped in an
  [`ActiveValue`](crate::ActiveValue): a row being written, in which each
  column is set or left out ([Writing rows](crate::guide::writing)).
- [`ModelTrait`](crate::ModelTrait) and
  [`FromQueryResult`](trait@crate::FromQueryResult) for the `Model`, so a row
  decodes into it.

`Relation` lists the other tables this one is related to, and
`#[derive(Relation)]` turns each variant into a relation the queries can join
along. The bakery has seven entities, `bakery`, `baker`, `cake`, `cake_baker`,
`customer`, `order` and `order_line`, declared the same way. Their full
source is in the `pgorm-guide-support` crate in pgorm's repository, and each
example in this guide runs against them.

Every key inside `#[pgorm(..)]` is checked. An unknown key is a compile error
at the key, so a typo cannot silently change a column. A key whose value is
Rust code, such as a column type, an entity or a default expression, takes
the code as written, without quotes; a key whose value is a name in the
database, such as `table_name`, takes a string.

## Column types

Each field's type decides its column's type:

| Field type | Column type |
| --- | --- |
| `String` | `varchar` |
| `i16`, `i32`, `i64` | `smallint`, `integer`, `bigint` |
| `f32`, `f64` | `real`, `double precision` |
| `bool` | `boolean` |
| `Decimal` | `decimal`, which PostgreSQL also spells `numeric` |
| `Date`, `Time`, `DateTime` | `date`, `time`, `timestamp` |
| `DateTimeWithTimeZone` | `timestamptz` |
| `Uuid` | `uuid` |
| `Json` | `json` |
| `Vec<u8>` | `bytea` |
| `Option<T>` | `T`'s type, nullable |
| an `ActiveEnum`, a `ValueType`, a `Vec<T>` | the type it declares |

The date and time types are [jiff]'s, and `Decimal` is [rust_decimal]'s; the
prelude exports each under the name in the table. PostgreSQL has no unsigned
integers, so `u8`, `u16` and `u64` fields do not compile, nor do `i8` and
`char`, whose values would not read back as written.

`column_type` gives the column a type of your choosing, written as a
[`ColumnType`](crate::ColumnType) expression. The cake's price above is
`decimal(10, 2)`. A field that should be `text` rather than `varchar` says
`column_type = ColumnType::Text`.

A column is named for its field. `column_name` names it otherwise, and
`rename_all` on the `Model` renames every column by a case style such as
`"camelCase"`. A column's `Column` variant is named for its field too, and
`variant` names it otherwise.

Each declaration also says how the table is created. [Schema
generation](crate::guide::schema) turns an entity into `CREATE TABLE`, and these
examples use it to show what a declaration means:

```rust
mod customer {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "customer")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        #[pgorm(unique)]
        pub email: String,
        #[pgorm(column_type = ColumnType::Text)]
        pub notes: Option<String>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# fn main() -> Result<(), pgorm::Error> {
use pgorm::Schema;

let create = Schema::new().create_table_from_entity(customer::Entity)?;
assert_eq!(
    create.to_string(),
    r#"CREATE TABLE "customer" ( "id" serial NOT NULL, "name" varchar NOT NULL, "email" varchar NOT NULL, "notes" text, PRIMARY KEY ("id"), UNIQUE ("email") )"#
);
# Ok(())
# }
```

## Primary keys

Every entity has a primary key; a `Model` without a `primary_key` field does
not compile. A key of one integer column is generated by the database, as
`serial` above, and an insert leaves it out. Write `auto_increment = false` on
the field when the program chooses the key itself.

A key of several columns is a field marked `primary_key` for each. The bakery's
`cake_baker` table records which baker bakes which cake, keyed by the pair:

```rust
# mod cake { pub use pgorm_guide_support::cake::*; }
# mod baker { pub use pgorm_guide_support::baker::*; }
mod cake_baker {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "cake_baker")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub cake_id: i32,
        #[pgorm(primary_key)]
        pub baker_id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::cake::Entity,
            from = Column::CakeId,
            to = super::cake::Column::Id,
            on_delete = ForeignKeyAction::Cascade
        )]
        Cake,
        #[pgorm(
            belongs_to = super::baker::Entity,
            from = Column::BakerId,
            to = super::baker::Column::Id,
            on_delete = ForeignKeyAction::Cascade
        )]
        Baker,
    }
}

# fn main() -> Result<(), pgorm::Error> {
use pgorm::Schema;

let create = Schema::new().create_table_from_entity(cake_baker::Entity)?;
assert_eq!(
    create.to_string(),
    r#"CREATE TABLE "cake_baker" ( "cake_id" integer NOT NULL, "baker_id" integer NOT NULL, CONSTRAINT "pk-cake_baker" PRIMARY KEY ("cake_id", "baker_id"), CONSTRAINT "fk-cake_baker-cake_id" FOREIGN KEY ("cake_id") REFERENCES "cake" ("id") ON DELETE CASCADE, CONSTRAINT "fk-cake_baker-baker_id" FOREIGN KEY ("baker_id") REFERENCES "baker" ("id") ON DELETE CASCADE )"#
);
# Ok(())
# }
```

No column of a composite key is generated unless it says so. `identity`
makes a column `GENERATED ALWAYS AS IDENTITY` and `identity_by_default` makes
it `GENERATED BY DEFAULT AS IDENTITY`, inside a composite key or out of one.
A multi-tenant table keyed by `(tenant_id, id)` marks `id` this way, and its
inserts give the tenant and get the id back:

```rust
mod note {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "note")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key, identity)]
        pub id: i64,
        pub body: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# fn main() -> Result<(), pgorm::Error> {
use pgorm::Schema;

let create = Schema::new().create_table_from_entity(note::Entity)?;
assert_eq!(
    create.to_string(),
    r#"CREATE TABLE "note" ( "tenant_id" integer NOT NULL, "id" bigint NOT NULL GENERATED ALWAYS AS IDENTITY, "body" varchar NOT NULL, CONSTRAINT "pk-note" PRIMARY KEY ("tenant_id", "id") )"#
);
# Ok(())
# }
```

[`find_by_id`](crate::EntityTrait::find_by_id) takes a composite key as a
tuple, borrowing its parts: `note::Entity::find_by_id((7, 1))`, or
`(7, "x")` against a key of `(i32, String)`.

A key whose last column is a range can be a PostgreSQL 18 temporal key: mark
that field `primary_key, without_overlaps`, and no two rows with equal other
key columns may hold overlapping ranges.

## Defaults, generated columns and keys

A column can take a default, be computed from the others, carry a comment,
be indexed, or be unique:

- `default_value = 0` gives a constant default, and `default_expr = ..` an
  expression, such as `Expr::current_timestamp()` or `Func::uuidv7()`.
- `generated_stored = ..` and `generated_virtual = ..` make a generated
  column, computed from an expression over the row's other columns, which the
  database stores or computes on read. An insert never writes one.
- `unique` makes a column unique, and `unique(columns = (..))` on the `Model`
  makes a unique key of several columns, with an optional `name` and
  `nulls_not_distinct`.
- `indexed` asks [schema generation](crate::guide::schema) for an index on the
  column.
- `comment = ".."` on the `Model` or on a field becomes a `COMMENT ON`.

The bakery's order lines could carry their total as a stored generated column,
and allow each cake once per order:

```rust
mod order_line {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(
        table_name = "order_line",
        unique(name = "one_line_per_cake", columns = (Column::OrderId, Column::CakeId))
    )]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub order_id: i32,
        pub cake_id: i32,
        #[pgorm(default_value = 1)]
        pub quantity: i32,
        #[pgorm(column_type = ColumnType::Decimal(Some((10, 2))))]
        pub price: Decimal,
        #[pgorm(
            column_type = ColumnType::Decimal(Some((10, 2))),
            generated_stored = Expr::col(Column::Price).mul(Expr::col(Column::Quantity))
        )]
        pub total: Decimal,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# fn main() -> Result<(), pgorm::Error> {
use pgorm::Schema;

let create = Schema::new().create_table_from_entity(order_line::Entity)?;
assert_eq!(
    create.to_string(),
    r#"CREATE TABLE "order_line" ( "id" serial NOT NULL, "order_id" integer NOT NULL, "cake_id" integer NOT NULL, "quantity" integer NOT NULL DEFAULT 1, "price" decimal(10, 2) NOT NULL, "total" decimal(10, 2) NOT NULL GENERATED ALWAYS AS ("price" * "quantity") STORED, PRIMARY KEY ("id"), CONSTRAINT "one_line_per_cake" UNIQUE ("order_id", "cake_id") )"#
);
# Ok(())
# }
```

## Relations

Each variant of `Relation` is one relation, and its attributes say which
table it reaches and along which columns:

- `belongs_to = E` is the side holding the foreign key. `from` names this
  table's columns and `to` the other table's, a tuple of columns for a
  composite key. `on_delete` and `on_update` take a
  [`ForeignKeyAction`](crate::pgorm_query::ForeignKeyAction), and `fk_name`
  names the constraint.
- `has_many = E` and `has_one = E` are the other side, reading the columns
  from the other entity's `belongs_to`.
- `on_condition` narrows a relation with an extra predicate, written as an
  expression or as a closure over the two tables, `|left, right| ..`.

The derive implements [`Related<E>`](crate::Related) for each entity exactly
one variant reaches. That is what lets a query name the related entity alone,
as `cake.find_related(bakery::Entity)` does ([Relations](crate::guide::relations)).
An entity reached by two variants gets no `Related`, since only you know
which one you mean.

A many-to-many relation goes through a junction table. The cake's
`Related<baker::Entity>` above is written by hand: `to` is the junction's
relation to the baker, and `via` the junction's relation to the cake, turned
around with [`rev`](crate::RelationDef::rev). Where a junction's `Related`
reaches an entity one variant also reaches, mark that variant
`related = false`.

## Enums

An enum of strings derives [`ActiveEnum`](trait@crate::ActiveEnum) and becomes a
PostgreSQL enum. The cake's `Flavor` is the type `flavor`, its values the
variants' names in snake_case:

```rust
use pgorm::entity::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ActiveEnum)]
pub enum Flavor {
    Chocolate,
    Vanilla,
    Lemon,
    Carrot,
}

assert_eq!(Flavor::name().to_string(), "flavor");
assert_eq!(Flavor::values(), ["chocolate", "vanilla", "lemon", "carrot"]);
assert_eq!(Flavor::Lemon.to_value(), "lemon");
assert_eq!(Flavor::try_from_value(&"carrot".to_owned())?, Flavor::Carrot);
# Ok::<(), pgorm::Error>(())
```

The enum's attributes say otherwise where needed:

- `enum_name = ".."` names the type, and `schema_name = ".."` puts it in a
  schema of its own.
- `rename_all = ".."` writes every value in another case style, and a
  variant's `string_value = ".."` writes its own.
- `rs_type` and `db_type` together store the enum some other way, as
  integers or as plain text, each variant's value given by `num_value` or
  `string_value`.
- `display` also implements `Display`, writing each variant's
  `display_value`, or else its name.

```rust
use pgorm::entity::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ActiveEnum)]
#[pgorm(rs_type = i16, db_type = ColumnType::SmallInteger, display)]
pub enum Shelf {
    #[pgorm(num_value = 1, display_value = "Top shelf")]
    Top,
    #[pgorm(num_value = 2)]
    Bottom,
}

assert_eq!(Shelf::Top.to_value(), 1);
assert_eq!(Shelf::Top.to_string(), "Top shelf");
assert_eq!(Shelf::Bottom.to_string(), "Bottom");
```

The derive also makes the enum iterable (`Flavor::iter()`). [Schema
generation](crate::guide::schema) creates a PostgreSQL enum's type, and [Values and
types](crate::guide::values) covers arrays of enums and enums inside domains.

## Hooks

The `Entity` derive gives the `ActiveModel` an
[`ActiveModelBehavior`](crate::ActiveModelBehavior) with no hooks. An entity
that wants hooks says `#[pgorm(hooks)]` on its `Model` and implements the
trait itself. `new` builds the active model `Default::default()` starts from;
`before_save` and `before_delete` run before the statement and can refuse it
by returning an error; `after_save` and `after_delete` run after it.

The hooks run when an `ActiveModel` writes itself through
[`insert`](crate::ActiveModelTrait::insert),
[`update`](crate::ActiveModelTrait::update) and
[`delete`](crate::ActiveModelTrait::delete). The statement builders of
[Writing rows](crate::guide::writing), such as `Insert::many`, run none.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("entities_hooks").await;
# let db = scratch.pool.get().await?;
mod special {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "special", hooks)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        pub note: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    #[async_trait::async_trait]
    impl ActiveModelBehavior for ActiveModel {
        fn new() -> Self {
            Self {
                note: set("Ask at the counter"),
                ..ActiveModelTrait::default()
            }
        }

        async fn before_save<C>(self, _db: &C, _insert: bool) -> Result<Self, Error>
        where
            C: ConnectionTrait,
        {
            if self.name.as_ref().is_empty() {
                return Err(Error::Custom("a special needs a name".to_owned()));
            }
            Ok(self)
        }
    }
}

use pgorm::entity::prelude::*;
# use pgorm::Schema;
# db.batch_execute(&Schema::new().create_table_from_entity(special::Entity)?.to_string()).await?;

let saved = special::ActiveModel {
    name: set("Saturday cinnamon buns"),
    ..Default::default()
}
.insert(&db)
.await?;
assert_eq!(saved.note, "Ask at the counter");

let refused = special::ActiveModel {
    name: set(""),
    ..Default::default()
}
.insert(&db)
.await;
assert!(matches!(refused, Err(Error::Custom(message)) if message == "a special needs a name"));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## The other derives

An entity is one `Model` and one `Relation`, but a program reads and writes
other shapes too. Each of these derives makes one:

| Derive | Makes |
| --- | --- |
| [`PartialModel`](derive@crate::PartialModel) | a struct of some of an entity's columns and expressions, which a query projects and decodes |
| [`FromQueryResult`](derive@crate::FromQueryResult) | a struct a row of any query decodes into, by column name |
| [`Model`](derive@crate::Model) | a further model of an entity, some of its columns, read through `ModelTrait` |
| [`IntoActiveModel`](derive@crate::IntoActiveModel) | a struct, such as a form, that becomes an `ActiveModel` with its fields set and the rest left out |
| [`ValueType`](derive@crate::ValueType) | a newtype over a column value, usable as a field type |
| [`JsonValueType`](derive@crate::JsonValueType) | a `Serialize + Deserialize` type stored in a `json` or `jsonb` column |
| [`CreatedRange`](derive@crate::CreatedRange) | a range type the schema creates with `CREATE TYPE .. AS RANGE` |
| [`Disjoint`](derive@crate::Disjoint) | an enum stored as a base table and a table per kind ([Disjoint enums](crate::guide::disjoint)) |
| [`OneOf`](derive@crate::OneOf) | a reference to one of several entities ([One-of references](crate::guide::one_of)) |
| [`Column`](derive@crate::Column) | a column enum for a hand-written projection |
| [`EntityTrait`](derive@crate::EntityTrait) | an entity whose table or schema name is computed at run time |
| [`SqlName`](derive@crate::SqlName), [`StaticName`](derive@crate::StaticName) | names for tables and columns the [query builder](crate::guide::query_builder) uses |

[Reading rows](crate::guide::reading) uses `PartialModel` and `FromQueryResult`, and
[Values and types](crate::guide::values) uses `ValueType`, `JsonValueType` and
`CreatedRange`. The two for writing look like this:

```rust
mod subscriber {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "subscriber")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        #[pgorm(unique)]
        pub email: String,
        #[pgorm(default_value = true)]
        pub weekly: bool,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    /// What the newsletter's sign-up form sends: two of the columns.
    #[derive(IntoActiveModel)]
    pub struct SignUp {
        pub name: String,
        pub email: String,
    }
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("entities_into_active_model").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
# db.batch_execute(&pgorm::Schema::new().create_table_from_entity(subscriber::Entity)?.to_string()).await?;

let form = subscriber::SignUp {
    name: "Mo".to_owned(),
    email: "mo@example.com".to_owned(),
};
let mo = form.into_active_model().insert(&db).await?;
assert_eq!(mo.email, "mo@example.com");
// The form left `weekly` out, so it took the column's default.
assert!(mo.weekly);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A form beside its entity converts into the `ActiveModel` there. Anywhere
else, in another module or another crate, such as beside the handler that
reads it, `#[pgorm(active_model = ..)]` names the active model by its path. A
field may be of any column type, an `ActiveEnum` among them:

```rust
use pgorm::entity::prelude::*;
use pgorm_guide_support::cake::{self, Flavor};

/// What the shop's "new cake" form sends.
#[derive(IntoActiveModel)]
#[pgorm(active_model = cake::ActiveModel)]
struct NewCake {
    name: String,
    flavor: Flavor,
    price: Decimal,
    gluten_free: bool,
    bakery_id: i32,
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("entities_form_elsewhere").await;
# let db = scratch.pool.get().await?;
let form = NewCake {
    name: "Lemon Tart".to_owned(),
    flavor: Flavor::Lemon,
    price: Decimal::new(410, 2),
    gluten_free: false,
    bakery_id: 1,
};
let tart = form.into_active_model().insert(&db).await?;
assert_eq!(tart.flavor, Flavor::Lemon);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A field of type `Option<T>` sets its column when it is `Some` and leaves it
out when it is `None`; a nullable column is set to `NULL` by a field of type
`Option<Option<T>>` holding `Some(None)`.

A `ValueType` newtype stands for the type it wraps, wherever a column value
goes: as a field, as a bound parameter, as a decoded value, and as an
`Option` of one, a nullable column. The recipe below weighs its flour, and
perhaps its sugar, in grams, a type the compiler keeps apart from other
integers, stored as an ordinary `integer`:

```rust
mod recipe {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ValueType)]
    pub struct Grams(pub i32);

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "recipe")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub flour: Grams,
        pub sugar: Option<Grams>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# fn main() -> Result<(), pgorm::Error> {
use pgorm::entity::prelude::*;

let heavy = recipe::Entity::find().filter(recipe::Column::Flour.gt(recipe::Grams(500)));
assert_eq!(
    heavy.build().0,
    r#"SELECT "recipe"."id", "recipe"."flour", "recipe"."sugar" FROM "recipe" WHERE "recipe"."flour" > $1"#
);
assert_eq!(
    pgorm::Schema::new().create_table_from_entity(recipe::Entity)?.to_string(),
    r#"CREATE TABLE "recipe" ( "id" serial NOT NULL, "flour" integer NOT NULL, "sugar" integer, PRIMARY KEY ("id") )"#
);
// A newtype's `None` is its integer's NULL.
let unsweetened: Value = Option::<recipe::Grams>::None.into();
assert_eq!(unsweetened, Value::Int(None));
# Ok(())
# }
```

## Names computed at run time

An entity's table and schema are usually fixed when it is written. When the
schema is only known once the program runs, such as a deployment that keeps
each region in a schema of its own, write the `Entity` struct yourself,
holding what the name is computed from, derive
[`EntityTrait`](derive@crate::EntityTrait) on it, and implement
[`EntityName`](crate::EntityName). The `Model` then derives `Entity` with no
`table_name`.

A statement built the usual way, by `find()`, `Insert::one` and the rest,
takes the entity's names from `Entity::default()`, so for a name the whole
program shares the `Default` implementation is where it is decided:

```rust
use std::sync::OnceLock;

use pgorm::entity::prelude::*;

/// The schema this deployment keeps its tables in, set once at startup.
static REGION: OnceLock<&'static str> = OnceLock::new();

#[derive(Copy, Clone, Debug, EntityTrait)]
pub struct Entity {
    pub schema: &'static str,
}

impl Default for Entity {
    fn default() -> Self {
        Self {
            schema: REGION.get().copied().unwrap_or("public"),
        }
    }
}

impl EntityName for Entity {
    fn schema_name(&self) -> Option<&str> {
        Some(self.schema)
    }

    fn table_name(&self) -> &str {
        "cake"
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}

# fn main() {
REGION.set("oslo").expect("the region is set once");
assert_eq!(
    Entity::find().build().0,
    r#"SELECT "cake"."id", "cake"."name" FROM "oslo"."cake""#
);
# }
```

When the name changes from one statement to the next, such as a schema per
tenant, hand the statement the entity itself. Each constructor has a form
ending in `_in` that takes an entity value first and names its table:
[`find_in`](crate::EntityTrait::find_in),
[`find_by_id_in`](crate::EntityTrait::find_by_id_in),
[`graph_in`](crate::EntityTrait::graph_in),
[`delete_by_id_in`](crate::EntityTrait::delete_by_id_in),
[`Insert::one_in`](crate::Insert::one_in) and
[`many_in`](crate::Insert::many_in),
[`Update::one_in`](crate::Update::one_in),
[`Delete::one_in`](crate::Delete::one_in), and an active model's
[`insert_in`](crate::ActiveModelTrait::insert_in),
[`update_in`](crate::ActiveModelTrait::update_in) and
[`delete_in`](crate::ActiveModelTrait::delete_in), which run its hooks as
`insert` does. [`Update::many`](crate::Update::many),
[`Delete::many`](crate::Delete::many) and
[`Pipeline::from`](crate::pipeline::Pipeline::from) take the entity value
already:

```rust
use pgorm::entity::prelude::*;

#[derive(Copy, Clone, Debug, EntityTrait)]
pub struct Entity {
    pub tenant: &'static str,
}

impl Default for Entity {
    fn default() -> Self {
        Self { tenant: "public" }
    }
}

impl EntityName for Entity {
    fn schema_name(&self) -> Option<&str> {
        Some(self.tenant)
    }

    fn table_name(&self) -> &str {
        "cake"
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}

# fn main() -> Result<(), pgorm::Error> {
let acme = Entity { tenant: "acme" };
assert_eq!(
    Entity::find_by_id_in(acme, 1).build().0,
    r#"SELECT "cake"."id", "cake"."name" FROM "acme"."cake" WHERE "cake"."id" = $1"#
);
let rename = Update::one_in(
    acme,
    ActiveModel {
        id: set(1),
        name: set("Acme Tart"),
    },
)?;
assert_eq!(
    rename.as_query().to_string(),
    r#"UPDATE "acme"."cake" SET "name" = 'Acme Tart' WHERE "cake"."id" = 1"#
);
// The default still decides what `find()` reads.
assert_eq!(
    Entity::find().build().0,
    r#"SELECT "cake"."id", "cake"."name" FROM "public"."cake""#
);
# Ok(())
# }
```

An entity's columns are qualified by the default's table name, so the default
has to name a table, and an instance naming another table is read under the
default's name: an entity whose table is `cake_2026` for one instance and
`cake` by default reads `FROM "cake_2026" AS "cake"`. A statement keeps the
entity value itself to name its table by, so the entity is `Copy` and
`'static`: a name read at run time, such as a tenant's, is kept once for the
program and handed out as a `&'static str`.

An entity in a schema fixed when it is written says `schema_name = ".."`
beside `table_name` instead.

[jiff]: https://docs.rs/jiff
[rust_decimal]: https://docs.rs/rust_decimal
