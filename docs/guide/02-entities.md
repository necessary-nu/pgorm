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
- [`Model`](crate::Model) and
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

[`find_by_id`](crate::Entity::find_by_id) takes a composite key as a
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

## Immutable columns

Some columns are written once, when their row is inserted, and are wrong to
change after: when a row was created and by whom, an id another system gave
it, the amount of a payment. `immutable` on a field says so:
[`ColumnDef::immutable`](crate::ColumnDef::immutable) marks the column's
definition, and every statement pgorm builds that would change the column
is refused with [`Error::ImmutableColumn`](crate::Error::ImmutableColumn),
naming the table and the column, before anything is sent:

- [`Update::one`](crate::Update::one) and its forms, and so an active model's
  `update`, given a model that sets the column. A value its `before_save`
  hook sets is refused as the caller's is. A primary key the model holds
  names the row and is never set, so an immutable key is not refused there;
- a bulk update whose `SET` names the column, by `col_expr` or by `set`;
- an upsert whose `ON CONFLICT .. DO UPDATE SET` names it. pgorm writes no
  `DO UPDATE SET` of its own: an upsert updates the columns it names, and an
  immutable one is left as its insert wrote it unless it is named;
- a [disjoint](crate::guide::disjoint) update setting one in the base's row
  or the kind's, and a change of kind whose kind column is immutable.

The bakery's till keeps each payment's reference and amount as it took them,
and lets its note be corrected:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("entities_immutable").await;
# let db = scratch.pool.get().await?;
mod payment {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "payment")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(unique, immutable)]
        pub reference: String,
        #[pgorm(immutable)]
        pub amount_cents: i64,
        pub note: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

use pgorm::entity::prelude::*;
use pgorm::pgorm_query::OnConflict;
# db.batch_execute(&pgorm::Schema::new().create_table_from_entity(payment::Entity)?.to_string()).await?;

let paid = payment::ActiveModel {
    reference: set("till-0042"),
    amount_cents: set(1250),
    note: set("two loaves"),
    ..Default::default()
}
.insert(&db)
.await?;

let noted = payment::ActiveModel {
    id: set(paid.id),
    note: set("two sourdough loaves"),
    ..Default::default()
}
.update(&db)
.await?;
assert_eq!((noted.amount_cents, noted.note.as_str()), (1250, "two sourdough loaves"));

let discounted = Update::one(payment::ActiveModel {
    id: set(paid.id),
    amount_cents: set(950),
    ..Default::default()
});
assert!(matches!(
    discounted,
    Err(Error::ImmutableColumn { table, column }) if table == "payment" && column == "amount_cents"
));

let doubled = Update::many(payment::Entity)
    .col_expr(
        payment::Column::AmountCents,
        Expr::col(payment::Column::AmountCents).mul(2),
    )
    .exec(&db)
    .await;
assert!(matches!(doubled, Err(Error::ImmutableColumn { .. })));

let resent = || payment::ActiveModel {
    reference: set("till-0042"),
    amount_cents: set(1300),
    note: set("sent twice"),
    ..Default::default()
};
let repriced = Insert::one(resent())
    .on_conflict(
        OnConflict::column(payment::Column::Reference)
            .update_column(payment::Column::AmountCents),
    )
    .exec(&db)
    .await;
assert!(matches!(repriced, Err(Error::ImmutableColumn { .. })));

let renoted = Insert::one(resent())
    .on_conflict(
        OnConflict::column(payment::Column::Reference).update_column(payment::Column::Note),
    )
    .exec_returning_model(&db)
    .await?;
assert_eq!((renoted.amount_cents, renoted.note.as_str()), (1250, "sent twice"));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The refusal is made as the statement is built or run, not as the program
compiles: one active model serves an insert, which writes the column, and an
update, which may not, and `col_expr` takes a column named at run time. The
derive refuses `immutable` where no statement writes the column at all: on
an `ignore`d field, and on a generated column.

Immutability is pgorm's own. The table pgorm creates is the same with it as
without it, and no trigger is written, so raw SQL, pgorm-query's builders
used directly and other programs writing the table are held to it only by
what the application puts in the database: a trigger refusing the change, or
an `UPDATE` privilege granted on the other columns alone. Every
[scope](#scopes) column is immutable, since a row cannot leave its scope.

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
[`insert`](crate::ActiveModel::insert),
[`update`](crate::ActiveModel::update) and
[`delete`](crate::ActiveModel::delete). The statement builders of
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
                ..pgorm::ActiveModel::default()
            }
        }

        async fn before_save<C>(self, _db: &C, _insert: bool) -> Result<Self, Error>
        where
            C: Connection,
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
| [`Model`](derive@crate::Model) | a further model of an entity, some of its columns, read through `Model` |
| [`IntoActiveModel`](derive@crate::IntoActiveModel) | a struct, such as a form, that becomes an `ActiveModel` with its fields set and the rest left out |
| [`ValueType`](derive@crate::ValueType) | a newtype over a column value, usable as a field type |
| [`JsonValueType`](derive@crate::JsonValueType) | a `Serialize + Deserialize` type stored in a `json` or `jsonb` column |
| [`CreatedRange`](derive@crate::CreatedRange) | a range type the schema creates with `CREATE TYPE .. AS RANGE` |
| [`Disjoint`](derive@crate::Disjoint) | an enum stored as a base table and a table per kind ([Disjoint enums](crate::guide::disjoint)) |
| [`OneOf`](derive@crate::OneOf) | a reference to one of several entities ([One-of references](crate::guide::one_of)) |
| [`Column`](derive@crate::Column) | a column enum for a hand-written projection |
| [`Entity`](derive@crate::Entity), on a hand-written `Entity` | an entity whose table or schema name is computed at run time |
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
[`Entity`](derive@crate::Entity) on it, and implement
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

#[derive(Copy, Clone, Debug, Entity)]
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
[`find_in`](crate::Entity::find_in),
[`find_by_id_in`](crate::Entity::find_by_id_in),
[`graph_in`](crate::Entity::graph_in),
[`delete_by_id_in`](crate::Entity::delete_by_id_in),
[`Insert::one_in`](crate::Insert::one_in) and
[`many_in`](crate::Insert::many_in),
[`Update::one_in`](crate::Update::one_in),
[`Delete::one_in`](crate::Delete::one_in), and an active model's
[`insert_in`](crate::ActiveModel::insert_in),
[`update_in`](crate::ActiveModel::update_in) and
[`delete_in`](crate::ActiveModel::delete_in), which run its hooks as
`insert` does. [`Update::many`](crate::Update::many),
[`Delete::many`](crate::Delete::many) and
[`Pipeline::from`](crate::pipeline::Pipeline::from) take the entity value
already:

```rust
use pgorm::entity::prelude::*;

#[derive(Copy, Clone, Debug, Entity)]
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

## Scopes

Some tables are read only under a condition the application supplies: a
company's notes by its members, a user's messages by that user. Forgetting
the condition once lets a statement read another company's rows. A *scope*
holds the statements of such a table to it: every statement on a scoped
entity is built from a *context* holding its scopes, and every way to build
one without such a context is a compile error that names the entity and the
scopes it needs. pgorm writes the condition; holding the rows to it in the
database, with row-level security or otherwise, stays the application's.

This section's examples use a small domain beside the bakery: companies, the
notes their users write, and the messages users send each other.

### Declaring scopes

A scope is a type of the application's own, usually the newtype its column
holds. [`#[derive(Scope)]`](derive@crate::Scope) makes it one, beside
`ValueType`. An entity marks each column that holds a scope with
`#[pgorm(scope)]`, and the column's type is the scope; a column's type alone
never makes it a scope column:

```rust
pub mod note {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ValueType, Scope)]
    pub struct CompanyId(pub i64);

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ValueType, Scope)]
    pub struct UserId(pub i64);

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "note")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        #[pgorm(scope)]
        pub company_id: CompanyId,
        #[pgorm(scope)]
        pub author_id: UserId,
        pub body: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# fn main() {
use pgorm::entity::prelude::*;
use pgorm::EntityScopes;

let declared = note::Entity::scopes().map(|scopes| scopes.names());
assert_eq!(declared, Some(&["CompanyId", "UserId"][..]));
assert!(note::Column::AuthorId.def().is_immutable());
# }
```

The entity's [`EntityScopes`](crate::EntityScopes) lists them, and each
scope column's definition is [`immutable`](crate::ColumnDef::immutable): a
row cannot leave its scope, so an update never changes one
([Immutable columns](#immutable-columns)). A scope column is
never `NULL`, so a field `Option` or `nullable` is refused
where `scope` is written. A primary key can be a scope column too: the
`company` entity's `id` is a `CompanyId`, so a member reads their own
company's row and no other.

### Contexts

A *context* is what a statement on a scoped entity is built under. One scope
value is a context holding itself, and
[`#[derive(ScopeContext)]`](derive@crate::ScopeContext) makes a struct of
scopes one, holding each of its fields. A field of
[`Every<S>`](crate::Every) holds every value of `S`: a context lifts that
scope on purpose, and the statement gets no condition for it. A member is
signed in to one company as one user, and a company's administrator sees
every one of its users:

```rust
use pgorm::entity::prelude::*;
use pgorm_guide_support::notes::{CompanyId, UserId, company, note};

#[derive(Clone, Copy, Debug, ScopeContext)]
pub struct Member {
    pub company: CompanyId,
    pub user: UserId,
}

#[derive(Clone, Copy, Debug, ScopeContext)]
pub struct CompanyAdmin {
    pub company: CompanyId,
    pub users: Every<UserId>,
}

# fn main() {
let ada = Member { company: CompanyId(1), user: UserId(10) };
let admin = CompanyAdmin { company: CompanyId(1), users: Every::new() };
assert_eq!(
    note::Entity::find_for(&ada).build().0,
    [
        r#"SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body""#,
        r#"FROM "note" WHERE "note"."company_id" = $1 AND "note"."author_id" = $2"#,
    ]
    .join(" ")
);
assert_eq!(
    note::Entity::find_for(&admin).build().0,
    [
        r#"SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body""#,
        r#"FROM "note" WHERE "note"."company_id" = $1"#,
    ]
    .join(" ")
);
// A scope value alone is a context, and the company's key is its scope.
assert_eq!(
    company::Entity::find_for(&CompanyId(1)).build().0,
    r#"SELECT "company"."id", "company"."name" FROM "company" WHERE "company"."id" = $1"#
);
# }
```

A context holds each scope once, as one value or as every value; a struct
holding one twice is refused by the derive.

### Every statement takes its context first

Each statement constructor has a form ending in `_for` that takes the
context first, as the `_in` forms take an entity value first:
[`find_for`](crate::Entity::find_for),
[`find_by_id_for`](crate::Entity::find_by_id_for),
[`graph_for`](crate::Entity::graph_for),
[`delete_by_id_for`](crate::Entity::delete_by_id_for),
[`Insert::one_for`](crate::Insert::one_for) and
[`many_for`](crate::Insert::many_for) (and `add_for`, `add_many_for`, and
`TryInsert`'s), [`Update::one_for`](crate::Update::one_for) and
[`many_for`](crate::Update::many_for),
[`Delete::one_for`](crate::Delete::one_for) and
[`many_for`](crate::Delete::many_for), an active model's
[`insert_for`](crate::ActiveModel::insert_for),
[`update_for`](crate::ActiveModel::update_for) and
[`delete_for`](crate::ActiveModel::delete_for), a model's
[`find_related_for`](crate::Model::find_related_for),
[`find_linked_for`](crate::Model::find_linked_for) and
[`delete_for`](crate::Model::delete_for), a disjoint enum's
[`find_for`](crate::Disjoint::find_for) and its writes, a one-of
reference's [`load_for`](crate::OneOf::load_for), and
[`Pipeline::from_for`](crate::pipeline::Pipeline::from_for). An unscoped
entity takes any context, and its forms taking none are the ones it always
had.

The forms taking no context are those of the context
[`NoContext`](crate::NoContext), which holds nothing, so for a scoped entity
they do not compile, and neither does a `_for` form given a context missing
one of its scopes:

```rust,compile_fail
use pgorm::entity::prelude::*;
use pgorm_guide_support::notes::note;

let _ = note::Entity::find();
```

The error is the entity's own, the derive having written it: "`note::Entity`
is scoped by `CompanyId` and `UserId`, which `NoContext` does not hold". The
same message names any other context that falls short, such as
`find_for(&CompanyId(1))`. A loader is given a statement rather than an
entity: hand it a `find_for` read, since the entity alone is every row and
does not compile for a scoped one.

### Reading and joining

A read ANDs each scope its context holds the value of into the `WHERE`, so
Ada reads her own notes and the administrator reads the company's:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::notes("entities_scoped_reads").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm_guide_support::notes::{CompanyAdmin, CompanyId, Member, UserId, company, message, note};

let ada = Member { company: CompanyId(1), user: UserId(10) };
let admin = CompanyAdmin { company: CompanyId(1), users: Every::new() };

let hers = note::Entity::find_for(&ada).all(&db).await?;
assert_eq!(hers.iter().map(|note| note.body.as_str()).collect::<Vec<_>>(), ["Ada's plan"]);

let theirs = note::Entity::find_for(&admin)
    .order_by_asc(note::Column::Id)
    .all(&db)
    .await?;
assert_eq!(theirs.len(), 2, "Ada's and Bo's notes, and not Globex's");

// Cy's note belongs to Globex, so Ada finds no note of its key.
assert!(matches!(
    note::Entity::find_by_id_for(&ada, 3).one(&db).await,
    Err(Error::RecordNotFound)
));

// A graph keeps its context, and each slot's scopes go in its join's `ON`.
let acme: Vec<(company::Model, Option<note::Model>)> = company::Entity::graph_for(&ada)
    .join_maybe::<note::Entity>(company::Relation::Note.def())
    .all(&db)
    .await?;
assert_eq!(acme.len(), 1);
assert_eq!(acme[0].1.as_ref().map(|note| note.author_id), Some(UserId(10)));
# let _ = message::Entity;
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A graph's slot is typed, so a slot of an entity the graph's context does not
hold the scopes of is the same compile error. A join taking a
[`RelationDef`](crate::RelationDef) is not typed by its target, so the
`Relation` derive records the target's scopes in the relation. Joined with
its context, through [`join_for`](crate::QuerySelect::join_for) (or
`join_rev_for`, `join_as_for` and `join_as_rev_for`), a scoped table's
scopes are ANDed into the join's `ON`; joined by a form taking no context,
the statement is refused with
[`ScopeError::Unscoped`](crate::ScopeError::Unscoped) when it would run, and
nothing is sent:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::notes("entities_scoped_joins").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm_guide_support::notes::{CompanyId, Member, UserId, company};

let ada = Member { company: CompanyId(1), user: UserId(10) };

let joined = company::Entity::find_for(&ada)
    .join_for(&ada, JoinType::InnerJoin, company::Relation::Note.def())
    .all(&db)
    .await?;
assert_eq!(joined.len(), 1);

let unscoped = company::Entity::find_for(&ada)
    .join(JoinType::InnerJoin, company::Relation::Note.def())
    .all(&db)
    .await;
assert!(matches!(
    unscoped,
    Err(Error::Scope(ScopeError::Unscoped { table, .. })) if table == "note"
));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

### Writing

An insert under a context fills each scope column the row leaves unset with
the value the context holds, and refuses a row setting one to another value
with [`ScopeError::Conflict`](crate::ScopeError::Conflict). Under `Every` of
a scope the insert fills nothing: the row sets the column itself, and a row
that does not is refused with [`ScopeError::Unset`](crate::ScopeError::Unset).

An update or a delete by key carries the scopes beside the key, so a key of
a row outside them matches nothing: `Update::one_for` reports
[`Error::RecordNotFound`](crate::Error::RecordNotFound) and a delete counts
`0`. A bulk update or delete is filtered by the scopes. A scope column is
[immutable](#immutable-columns), since a row cannot leave its scope: an
update setting one, `Update::one`, `Update::many`'s `set` and `col_expr`,
and an upsert's `ON CONFLICT .. DO UPDATE SET` naming one, is refused with
[`Error::ImmutableColumn`](crate::Error::ImmutableColumn) before anything is
sent, as it is for any immutable column.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::notes("entities_scoped_writes").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm_guide_support::notes::{CompanyAdmin, CompanyId, Member, UserId, note};

let ada = Member { company: CompanyId(1), user: UserId(10) };
let admin = CompanyAdmin { company: CompanyId(1), users: Every::new() };

let written = Insert::one_for(
    &ada,
    note::ActiveModel {
        body: set("Ada's second plan"),
        ..Default::default()
    },
)
.exec_returning_model(&db)
.await?;
assert_eq!((written.company_id, written.author_id), (CompanyId(1), UserId(10)));

let elsewhere = Insert::one_for(
    &ada,
    note::ActiveModel {
        company_id: set(CompanyId(2)),
        body: set("for Globex"),
        ..Default::default()
    },
)
.exec(&db)
.await;
assert!(matches!(elsewhere, Err(Error::Scope(ScopeError::Conflict { .. }))));

// Bo's note is the company's, so the administrator edits it and Ada cannot.
let edit = note::ActiveModel {
    id: set(2),
    body: set("Bo's draft, reviewed"),
    ..Default::default()
};
assert!(matches!(
    Update::one_for(&ada, edit.clone())?.exec_returning_model(&db).await,
    Err(Error::RecordNotFound)
));
assert_eq!(
    Update::one_for(&admin, edit)?.exec_returning_model(&db).await?.body,
    "Bo's draft, reviewed"
);

// Handing Bo's note to Ada would move it out of Bo's scope.
let handed = Update::one_for(
    &admin,
    note::ActiveModel {
        id: set(2),
        author_id: set(UserId(10)),
        ..Default::default()
    },
);
assert!(matches!(handed, Err(Error::ImmutableColumn { column, .. }) if column == "author_id"));

assert_eq!(note::Entity::delete_by_id_for(&ada, 2).exec(&db).await?, 0);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A bulk update's `FROM` and a bulk delete's `USING` add a table whose rows
the statement reads, as a join does, so a scoped entity added there is held
to its scopes too. [`from_for`](crate::UpdateMany::from_for) and
[`using_for`](crate::DeleteMany::using_for) take the context the entity is
read under, which, as a join's, need not be the statement's own, and AND its
scopes into the `WHERE`. The support desk, over every company, deletes
Globex's notes, reading `company` under Globex's scope so that no other
company's row joins:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::notes("entities_scoped_using").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm_guide_support::notes::{CompanyAdmin, CompanyId, UserId, company, note};

#[derive(Clone, Copy, Debug, ScopeContext)]
pub struct Support {
    pub companies: Every<CompanyId>,
    pub users: Every<UserId>,
}

let support = Support { companies: Every::new(), users: Every::new() };
let globex = Delete::many_for(&support, note::Entity)
    .using_for(&CompanyId(2), company::Entity)
    .filter(note::Column::CompanyId.eq_col(company::Column::Id));
assert_eq!(
    globex.as_query().to_string(),
    [
        r#"DELETE FROM "note" USING "company""#,
        r#"WHERE "company"."id" = 2 AND "note"."company_id" = "company"."id""#,
    ]
    .join(" ")
);
assert_eq!(globex.exec(&db).await?, 1, "Cy's note, and neither of Acme's");

let acme = CompanyAdmin { company: CompanyId(1), users: Every::new() };
assert_eq!(note::Entity::find_for(&acme).count(&db).await?, 2);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

[`from`](crate::UpdateMany::from) and [`using`](crate::DeleteMany::using),
which take no context, take an unscoped entity or one of pgorm-query's own
items (a [`FromItem`](crate::pgorm_query::FromItem), `NamedTable`,
`TableName`, `Name`, `TableSample` or `AliasName`), which name no entity to
pgorm and are added as they are. A scoped entity there does not compile, the
error the entity's own, as a plain statement form's is:

```rust,compile_fail,E0277
use pgorm::entity::prelude::*;
use pgorm_guide_support::notes::{CompanyId, Member, UserId, company, note};

let ada = Member { company: CompanyId(1), user: UserId(10) };
let _ = Delete::many_for(&ada, company::Entity).using(note::Entity);
```

### Scopes written by hand

A scope whose condition is not one column equal to its value is declared on
the entity, `#[pgorm(scope(UserId))]`, and written as the scope type's
[`ScopeOn`](crate::ScopeOn) the entity: the condition its value puts on a
row read under a given name, the table's own or a join's alias, and what an
insert under it does to a row. A message is its sender's and its
recipient's to read, and is sent by the user writing it:

```rust
pub mod message {
    use pgorm::entity::prelude::*;
    use pgorm_guide_support::notes::UserId;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "message", scope(Recipient))]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        pub sender_id: UserId,
        pub recipient_id: UserId,
        pub body: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    /// The user reading their mail.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Scope)]
    pub struct Recipient(pub UserId);

    impl ScopeOn<Entity> for Recipient {
        fn condition(&self, table: Name) -> Condition {
            Condition::any()
                .add(Expr::col((table.clone(), Column::SenderId)).eq(self.0))
                .add(Expr::col((table, Column::RecipientId)).eq(self.0))
        }

        fn insert(&self, row: &mut ActiveModel) -> Result<(), ScopeError> {
            match &row.sender_id {
                ActiveValue::NotSet => {
                    row.sender_id = set(self.0);
                    Ok(())
                }
                ActiveValue::Set(sender) | ActiveValue::Unchanged(sender) if *sender == self.0 => {
                    Ok(())
                }
                _ => Err(ScopeError::Refused("a message is sent by the user writing it".to_owned())),
            }
        }
    }
}

# fn main() {
use pgorm::entity::prelude::*;
use message::Recipient;
use pgorm_guide_support::notes::UserId;

assert_eq!(
    message::Entity::find_for(&Recipient(UserId(10))).build().0,
    [
        r#"SELECT "message"."id", "message"."sender_id", "message"."recipient_id", "message"."body""#,
        r#"FROM "message" WHERE "message"."sender_id" = $1 OR "message"."recipient_id" = $2"#,
    ]
    .join(" ")
);
assert_eq!(
    Insert::one_for(
        &Recipient(UserId(10)),
        message::ActiveModel {
            recipient_id: set(UserId(11)),
            body: set("lunch?"),
            ..Default::default()
        },
    )
    .as_query()
    .to_string(),
    r#"INSERT INTO "message" ("sender_id", "recipient_id", "body") VALUES (10, 11, 'lunch?')"#
);
# }
```

Its condition is pgorm-query's, which a pipeline cannot read, so
`Pipeline::from_for` over such an entity is refused with
[`ScopeError::Unpiped`](crate::ScopeError::Unpiped) under a context holding
a value of the scope. A hand-written scope's columns are the condition's
business; pgorm marks immutable only the columns `#[pgorm(scope)]` marks, so
a column the condition reads that a row must not change, such as a
message's sender, is keyed [`immutable`](#immutable-columns) by the program.

An entity whose names are computed at run time is read under its `Default`
names by the `_for` forms. Its `_in` forms take no context, so for a scoped
entity they are compile errors as its plain forms are, and there is no form
taking both an entity value and a context.

### Raw SQL

[`sql!`](crate::sql), [`prql!`](crate::prql), the methods taking SQL text,
and the pgorm-query builders used directly build statements pgorm does not
read, so they are outside scopes. A project that wants every statement on a
scoped table to go through a context can say so to clippy, banning the
macros and the text-taking methods in its `clippy.toml` and allowing a case
where it is meant:

```toml
disallowed-macros = [
    { path = "pgorm::sql", reason = "raw SQL is outside scopes; build the statement with a `_for` form" },
    { path = "pgorm::prql", reason = "raw PRQL is outside scopes; build the statement with a `_for` form" },
]
disallowed-methods = [
    { path = "pgorm::Connection::execute", reason = "raw SQL is outside scopes" },
    { path = "pgorm::Connection::execute_raw", reason = "raw SQL is outside scopes" },
    { path = "pgorm::Connection::query_one", reason = "raw SQL is outside scopes" },
    { path = "pgorm::Connection::query_opt", reason = "raw SQL is outside scopes" },
    { path = "pgorm::Connection::query_all", reason = "raw SQL is outside scopes" },
    { path = "pgorm::Connection::query_raw", reason = "raw SQL is outside scopes" },
    { path = "pgorm::Connection::batch_execute", reason = "raw SQL is outside scopes" },
    { path = "pgorm::SelectorRaw::from_statement", reason = "raw SQL is outside scopes" },
    { path = "pgorm::SelectorRaw::into_tuple", reason = "raw SQL is outside scopes" },
    { path = "pgorm::SelectorRaw::with_columns", reason = "raw SQL is outside scopes" },
    { path = "pgorm::Select::from_raw_sql", reason = "raw SQL is outside scopes" },
    { path = "pgorm::FromQueryResult::find_by_statement", reason = "raw SQL is outside scopes" },
]
```

The paths are the ones a crate depending on pgorm writes; a macro imported
by `use pgorm::sql;` is caught under its own name too. A case meant to read
across scopes, such as a report over every company, says so where it is
written, with `#[expect]`, which also warns once the case no longer needs
it, or `#[allow]`:

```rust
# fn main() {
#[expect(clippy::disallowed_macros, reason = "the monthly report reads every company")]
let report = pgorm::sql!("SELECT company_id, count(*) FROM note GROUP BY company_id");
assert!(report.starts_with("SELECT company_id"));
# }
```

[jiff]: https://docs.rs/jiff
[rust_decimal]: https://docs.rs/rust_decimal
