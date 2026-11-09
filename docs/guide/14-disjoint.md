# Disjoint enums

Some things come in kinds, each kind with data of its own. The bakery takes
payments by card, in cash or with a gift voucher: every payment has an
amount, a card payment also has the card's last four digits, a voucher
payment the voucher's code, and a cash payment nothing more. In Rust that is
an enum. In PostgreSQL it is a *base* table holding what every payment has,
with a column saying which kind each row is, and a table for each kind that
carries data, keyed by the base row's key.

The [`Disjoint`](derive@crate::Disjoint) derive declares such an enum, and
pgorm writes the tables and the constraints that hold them together, reads a
row with its kind's row in one statement, and writes both in one statement.
The constraints are PostgreSQL's own, with no triggers, so a state the enum
cannot describe cannot be written, by pgorm or by anything else connected to
the database: a card payment without its card row, a card row whose payment
is cash, a payment with rows in two kinds' tables.

## Declaring one

The base is an ordinary entity whose `kind` column holds the kind. Each kind
with data is an ordinary entity too, keyed by the same types as the base and
`auto_increment = false`, since its key is its base row's. The enum lists the
kinds: a variant holding a kind entity's `Model` is a kind with a table, and a
unit variant is a kind without one.

```rust
pub mod payment {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "payment")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: super::PaymentKind,
        pub order_id: i32,
        #[pgorm(column_type = ColumnType::Decimal(Some((10, 2))))]
        pub amount: Decimal,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod card_payment {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "card_payment")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub payment_id: i32,
        pub last_four: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod voucher_payment {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "voucher_payment")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub payment_id: i32,
        pub code: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = payment::Entity)]
pub enum Payment {
    Card(card_payment::Model),
    Cash,
    Voucher(voucher_payment::Model),
}

# fn main() -> Result<(), pgorm::Error> {
assert_eq!(PaymentKind::name().to_string(), "payment_kind");
assert_eq!(PaymentKind::values(), ["card", "cash", "voucher"]);

let paid = Payment::Card(card_payment::Model {
    payment_id: 1,
    last_four: "4242".to_owned(),
});
assert_eq!(paid.kind(), PaymentKind::Card);

pgorm::check_disjoint::<Payment>()?;
# Ok(())
# }
```

The derive writes two more types beside the enum:

- `PaymentKind`, the kind type: an [`ActiveEnum`](trait@crate::ActiveEnum) stored as
  the PostgreSQL enum `payment_kind`, one value per variant in declaration
  order, each the variant's name in snake_case. The base's kind column holds
  it.
- `PaymentActiveModel`, the same variants each holding its kind entity's
  `ActiveModel`, which a write takes.

The enum's attributes rename what it makes: `kind = ..` names a kind column
other than `Column::Kind`, `enum_name = ".."` and `schema_name = ".."` name
the kind type, a variant's `value = ".."` names its kind, and
`kind_attrs(..)` gives the kind type further attributes, such as
`kind_attrs(derive(Serialize, Deserialize))`.

What the types cannot check, [`check_disjoint`](crate::check_disjoint) checks
when it runs: a kind column that is nullable, generated or not of the kind
type, and a kind entity whose key the database fills, by a sequence, an
identity or a default. Every read and write runs it before it sends anything.

## Creating the tables

[`Schema::create_disjoint`](crate::Schema::create_disjoint) returns the
statements that create the kind type and the tables, in the order they run:
the kind type, the base table, each kind's table, and one `ALTER TABLE` on the
base adding what points back at the kinds' tables.

```rust
# pub mod payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "payment")]
#     pub struct Model {
#         #[pgorm(primary_key)]
#         pub id: i32,
#         pub kind: super::PaymentKind,
#         pub order_id: i32,
#         #[pgorm(column_type = ColumnType::Decimal(Some((10, 2))))]
#         pub amount: Decimal,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# pub mod card_payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "card_payment")]
#     pub struct Model {
#         #[pgorm(primary_key, auto_increment = false)]
#         pub payment_id: i32,
#         pub last_four: String,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# pub mod voucher_payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "voucher_payment")]
#     pub struct Model {
#         #[pgorm(primary_key, auto_increment = false)]
#         pub payment_id: i32,
#         pub code: String,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# use pgorm::entity::prelude::*;
# #[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
# #[pgorm(base = payment::Entity)]
# pub enum Payment {
#     Card(card_payment::Model),
#     Cash,
#     Voucher(voucher_payment::Model),
# }
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("disjoint_create").await;
# let db = scratch.pool.get().await?;
use pgorm::Schema;

let statements: Vec<String> = Schema::new()
    .create_disjoint::<Payment>()?
    .iter()
    .map(ToString::to_string)
    .collect();
assert_eq!(
    statements,
    [
        r#"CREATE TYPE "payment_kind" AS ENUM ('card', 'cash', 'voucher')"#,
        [
            r#"CREATE TABLE "payment" ( "id" serial NOT NULL, "kind" payment_kind NOT NULL,"#,
            r#""order_id" integer NOT NULL, "amount" decimal(10, 2) NOT NULL, PRIMARY KEY ("id"),"#,
            r#"CONSTRAINT "uq-payment-id-kind" UNIQUE ("id", "kind") )"#,
        ]
        .join(" ")
        .as_str(),
        [
            r#"CREATE TABLE "card_payment" ( "payment_id" integer NOT NULL, "last_four" varchar NOT NULL,"#,
            r#""kind" payment_kind GENERATED ALWAYS AS ('card') STORED, PRIMARY KEY ("payment_id"),"#,
            r#"CONSTRAINT "fk-card_payment-payment_id-kind" FOREIGN KEY ("payment_id", "kind")"#,
            r#"REFERENCES "payment" ("id", "kind") ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE )"#,
        ]
        .join(" ")
        .as_str(),
        [
            r#"CREATE TABLE "voucher_payment" ( "payment_id" integer NOT NULL, "code" varchar NOT NULL,"#,
            r#""kind" payment_kind GENERATED ALWAYS AS ('voucher') STORED, PRIMARY KEY ("payment_id"),"#,
            r#"CONSTRAINT "fk-voucher_payment-payment_id-kind" FOREIGN KEY ("payment_id", "kind")"#,
            r#"REFERENCES "payment" ("id", "kind") ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE )"#,
        ]
        .join(" ")
        .as_str(),
        [
            r#"ALTER TABLE "payment""#,
            r#"ADD COLUMN "card_id" integer GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'card') THEN "id" END)) STORED,"#,
            r#"ADD COLUMN "voucher_id" integer GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'voucher') THEN "id" END)) STORED,"#,
            r#"ADD CONSTRAINT "fk-payment-card_id" FOREIGN KEY ("card_id") REFERENCES "card_payment" ("payment_id") DEFERRABLE INITIALLY IMMEDIATE,"#,
            r#"ADD CONSTRAINT "fk-payment-voucher_id" FOREIGN KEY ("voucher_id") REFERENCES "voucher_payment" ("payment_id") DEFERRABLE INITIALLY IMMEDIATE"#,
        ]
        .join(" ")
        .as_str(),
    ]
);

for statement in &statements {
    db.batch_execute(statement).await?;
}
# drop(db);
# scratch.finish().await;
# Ok(())
# }
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

## Writing

Every write is one statement, which applies whole or not at all, with no
transaction needed. Each returns the row as a read would, the base `Model`
with the enum.

- [`insert`](crate::Disjoint::insert) takes the base's `ActiveModel` and a
  `PaymentActiveModel`. The base's kind column and the kind row's key are left
  out: the kind comes from the variant, and the key from the inserted base
  row.
- [`update`](crate::Disjoint::update) changes the set columns of a row and of
  its kind's row, found by the base's key. Updating a row with another kind's
  `ActiveModel` changes nothing and is `Error::RecordNotFound`.
- [`change_kind`](crate::Disjoint::change_kind) moves a row to another kind:
  it deletes the old kind's row, inserts the new one's, and sets the base
  row's kind.

Deleting needs nothing of its own: delete the base row as any row is deleted,
and its kind's row goes with it.

```rust
# pub mod payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "payment")]
#     pub struct Model {
#         #[pgorm(primary_key)]
#         pub id: i32,
#         pub kind: super::PaymentKind,
#         pub order_id: i32,
#         #[pgorm(column_type = ColumnType::Decimal(Some((10, 2))))]
#         pub amount: Decimal,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# pub mod card_payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "card_payment")]
#     pub struct Model {
#         #[pgorm(primary_key, auto_increment = false)]
#         pub payment_id: i32,
#         pub last_four: String,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# pub mod voucher_payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "voucher_payment")]
#     pub struct Model {
#         #[pgorm(primary_key, auto_increment = false)]
#         pub payment_id: i32,
#         pub code: String,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# use pgorm::entity::prelude::*;
# #[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
# #[pgorm(base = payment::Entity)]
# pub enum Payment {
#     Card(card_payment::Model),
#     Cash,
#     Voucher(voucher_payment::Model),
# }
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("disjoint_write").await;
# let db = scratch.pool.get().await?;
# for statement in pgorm::Schema::new().create_disjoint::<Payment>()? {
#     db.batch_execute(&statement.to_string()).await?;
# }
let paid = |order_id: i32, cents: i64| payment::ActiveModel {
    order_id: set(order_id),
    amount: set(Decimal::new(cents, 2)),
    ..Default::default()
};

let (base, by_card) = Payment::insert(
    paid(1, 1220),
    PaymentActiveModel::Card(card_payment::ActiveModel {
        last_four: set("4242"),
        ..Default::default()
    }),
)?
.exec_returning_model(&db)
.await?;
assert_eq!(base.kind, PaymentKind::Card);
assert_eq!(
    by_card,
    Payment::Card(card_payment::Model { payment_id: base.id, last_four: "4242".to_owned() })
);

// A new card number: the kind's row changes, found by the base's key.
let (_, updated) = Payment::update(
    payment::ActiveModel {
        id: Unchanged(base.id),
        ..Default::default()
    },
    PaymentActiveModel::Card(card_payment::ActiveModel {
        last_four: set("4343"),
        ..Default::default()
    }),
)?
.exec_returning_model(&db)
.await?;
assert!(matches!(updated, Payment::Card(card) if card.last_four == "4343"));

// An update as another kind finds no row of that kind.
let as_cash = Payment::update(
    payment::ActiveModel {
        id: Unchanged(base.id),
        amount: set(Decimal::ONE),
        ..Default::default()
    },
    PaymentActiveModel::Cash,
)?
.exec_returning_model(&db)
.await;
assert!(matches!(as_cash, Err(Error::RecordNotFound)));

let (cash, _) = Payment::insert(paid(2, 380), PaymentActiveModel::Cash)?
    .exec_returning_model(&db)
    .await?;

// The customer paid with a voucher after all.
let (moved, voucher) = Payment::change_kind(
    cash.id,
    PaymentActiveModel::Voucher(voucher_payment::ActiveModel {
        code: set("XMAS-2026"),
        ..Default::default()
    }),
)?
.exec_returning_model(&db)
.await?;
assert_eq!(moved.kind, PaymentKind::Voucher);
assert!(matches!(voucher, Payment::Voucher(v) if v.code == "XMAS-2026"));

// A card payment cannot lose its card row behind pgorm's back.
let refused = db
    .execute(r#"DELETE FROM "card_payment" WHERE "payment_id" = $1"#, &[&base.id])
    .await;
assert!(matches!(refused.unwrap_err().sql_error(), Some(SqlError::ForeignKeyConstraintViolation(_))));

// Deleting the payment deletes its card row with it.
payment::Entity::delete_by_id(base.id).exec(&db).await?;
assert_eq!(card_payment::Entity::find().count(&db).await?, 0);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Reading

[`Payment::find()`](crate::Disjoint::find) reads each base row with its
kind's row in one statement, the base `LEFT JOIN`ed to each kind's table, and
decodes each row as the pair `(payment::Model, Payment)`. It takes what a
`find()` on the base takes: filters and ordering on the base's columns,
`limit` and `offset`, pagination and keyset cursors, and
[`find_by_id`](crate::Disjoint::find_by_id). Three methods are its own:

- [`kind`](crate::SelectDisjoint::kind) and
  [`kinds`](crate::SelectDisjoint::kinds) read only rows of the given kinds;
- [`filter_kind::<M>`](crate::SelectDisjoint::filter_kind) reads only rows of
  `M`'s kind whose kind row passes a condition.

```rust
# pub mod payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "payment")]
#     pub struct Model {
#         #[pgorm(primary_key)]
#         pub id: i32,
#         pub kind: super::PaymentKind,
#         pub order_id: i32,
#         #[pgorm(column_type = ColumnType::Decimal(Some((10, 2))))]
#         pub amount: Decimal,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# pub mod card_payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "card_payment")]
#     pub struct Model {
#         #[pgorm(primary_key, auto_increment = false)]
#         pub payment_id: i32,
#         pub last_four: String,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# pub mod voucher_payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "voucher_payment")]
#     pub struct Model {
#         #[pgorm(primary_key, auto_increment = false)]
#         pub payment_id: i32,
#         pub code: String,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# use pgorm::entity::prelude::*;
# #[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
# #[pgorm(base = payment::Entity)]
# pub enum Payment {
#     Card(card_payment::Model),
#     Cash,
#     Voucher(voucher_payment::Model),
# }
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("disjoint_read").await;
# let db = scratch.pool.get().await?;
# for statement in pgorm::Schema::new().create_disjoint::<Payment>()? {
#     db.batch_execute(&statement.to_string()).await?;
# }
# let paid = |order_id: i32, cents: i64| payment::ActiveModel {
#     order_id: set(order_id),
#     amount: set(Decimal::new(cents, 2)),
#     ..Default::default()
# };
# let card = |digits: &str| PaymentActiveModel::Card(card_payment::ActiveModel {
#     last_four: set(digits),
#     ..Default::default()
# });
# Payment::insert(paid(1, 1220), card("4242"))?.exec_returning_model(&db).await?;
# Payment::insert(paid(2, 380), PaymentActiveModel::Cash)?.exec_returning_model(&db).await?;
# Payment::insert(paid(3, 600), card("1881"))?.exec_returning_model(&db).await?;
let every: Vec<(payment::Model, Payment)> = Payment::find()
    .order_by_asc(payment::Column::Id)
    .all(&db)
    .await?;
let kinds: Vec<PaymentKind> = every.iter().map(|(_, payment)| payment.kind()).collect();
assert_eq!(kinds, [PaymentKind::Card, PaymentKind::Cash, PaymentKind::Card]);

let cards = Payment::find().kind(PaymentKind::Card).count(&db).await?;
assert_eq!(cards, 2);

let (base, ending_42) = Payment::find()
    .filter_kind::<card_payment::Model>(card_payment::Column::LastFour.ends_with("42"))
    .one(&db)
    .await?;
assert_eq!(base.order_id, 1);
assert!(matches!(ending_42, Payment::Card(card) if card.last_four == "4242"));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A row the constraints rule out can still be met in a database pgorm did not
create, such as a card payment with no card row. A read reports it as
[`Error::Query`](crate::Error::Query), naming the table, the key and the
kinds, and never decodes it as some kind or drops it.

## Changing the kinds

A program grows new kinds, and the database has to follow. [`Schema`](crate::Schema)
writes each change from the declaration as it will be, as a
[`DisjointChange`](crate::DisjointChange): a list of steps, each a list of
statements run in one transaction, committed before the next step starts.
Some changes need two steps because PostgreSQL will not use an enum value in
the transaction that added it.

| Change | What it does |
| --- | --- |
| [`add_disjoint_kind`](crate::Schema::add_disjoint_kind) | adds a kind: `ALTER TYPE .. ADD VALUE`, then the kind's table, in two steps |
| [`add_disjoint_kind_recreating`](crate::Schema::add_disjoint_kind_recreating) | the same change in one step, by recreating the kind type and rewriting the tables once |
| [`remove_disjoint_kind`](crate::Schema::remove_disjoint_kind) | removes a kind no row has any more, recreating the type; it fails while a row still has the kind |
| [`add_disjoint_kind_table`](crate::Schema::add_disjoint_kind_table) | gives a kind without a table a table, leaving a gap in which the program writes each existing row's kind row |
| [`remove_disjoint_kind_table`](crate::Schema::remove_disjoint_kind_table) | drops a kind's table, keeping the kind without one |

A step after which the program has rows to write carries a
[`DisjointGap`](crate::DisjointGap) saying which. Run through
[pgorm-migration](crate::guide::migrations), each step is one migration, and every
step but the last answers `commits_after()`.

A change is written from the declaration, and the declaration keeps changing
after the change ships, so a migration must not compute it again each time it
runs: freeze it when it is made, as the next section shows.

Here the bakery began taking payments by card and in cash, and adds vouchers.
The hidden lines create the tables as the earlier declaration did:

```rust
# pub mod payment_before {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "payment")]
#     pub struct Model {
#         #[pgorm(primary_key)]
#         pub id: i32,
#         pub kind: super::PaymentBeforeKind,
#         pub order_id: i32,
#         #[pgorm(column_type = ColumnType::Decimal(Some((10, 2))))]
#         pub amount: Decimal,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# #[derive(Clone, Debug, PartialEq, Eq, pgorm::Disjoint)]
# #[pgorm(base = payment_before::Entity, enum_name = "payment_kind")]
# pub enum PaymentBefore {
#     Card(card_payment::Model),
#     Cash,
# }
# pub mod payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "payment")]
#     pub struct Model {
#         #[pgorm(primary_key)]
#         pub id: i32,
#         pub kind: super::PaymentKind,
#         pub order_id: i32,
#         #[pgorm(column_type = ColumnType::Decimal(Some((10, 2))))]
#         pub amount: Decimal,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# pub mod card_payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "card_payment")]
#     pub struct Model {
#         #[pgorm(primary_key, auto_increment = false)]
#         pub payment_id: i32,
#         pub last_four: String,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# pub mod voucher_payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "voucher_payment")]
#     pub struct Model {
#         #[pgorm(primary_key, auto_increment = false)]
#         pub payment_id: i32,
#         pub code: String,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = payment::Entity)]
pub enum Payment {
    Card(card_payment::Model),
    Cash,
    Voucher(voucher_payment::Model),
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("disjoint_evolve").await;
# let mut db = scratch.pool.get().await?;
# for statement in pgorm::Schema::new().create_disjoint::<PaymentBefore>()? {
#     db.batch_execute(&statement.to_string()).await?;
# }
use pgorm::{Schema, TransactionTrait};

let change = Schema::new().add_disjoint_kind::<Payment>(PaymentKind::Voucher)?;
let steps = change.steps();
assert_eq!(steps.len(), 2);
assert_eq!(
    steps[0].statements()[0].to_string(),
    r#"ALTER TYPE "payment_kind" ADD VALUE 'voucher' AFTER 'cash'"#
);
assert!(steps.iter().all(|step| step.gap().is_none()));

for step in steps {
    db.transaction(async |txn| {
        for statement in step.statements() {
            txn.batch_execute(&statement.to_string()).await?;
        }
        Ok::<_, Error>(())
    })
    .await?;
}
# drop(db);
# scratch.pool.clear_statement_caches();
# let db = scratch.pool.get().await?;

let (_, voucher) = Payment::insert(
    payment::ActiveModel {
        order_id: set(2),
        amount: set(Decimal::new(380, 2)),
        ..Default::default()
    },
    PaymentActiveModel::Voucher(voucher_payment::ActiveModel {
        code: set("XMAS-2026"),
        ..Default::default()
    }),
)?
.exec_returning_model(&db)
.await?;
assert_eq!(voucher.kind(), PaymentKind::Voucher);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A step that recreates the kind type changes the type prepared statements
were planned with, so a program running steps itself clears its pool's
statement caches after each, as [pgorm-migration](crate::guide::migrations) does
([Statement caches and DDL](crate::guide::statement_caches)).

### Freezing a change into migrations

A migration that called `add_disjoint_kind` as it ran would compute the
change from the declaration as it stands on the day it runs. So would the
migration before it that called `create_disjoint`, which on a database built
from nothing would already create the voucher kind the next migration adds,
and the history would fail to replay. A change is therefore frozen into
migration files when it is made, and the migrations list the files, which
nothing computes again.

[`migration_files`](crate::DisjointChange::migration_files) freezes a change
as one [`MigrationFile`](crate::MigrationFile) per step, named after the
migration with the step's number, and
[`MigrationFile::new`](crate::MigrationFile::new) freezes any other
statements, such as the tables `create_disjoint` writes. Every step but the
last opens with the line `-- pgorm: commits-after`, which pgorm-migration
reads as `commits_after()`, and a step leaving rows to the program ends with a
comment naming them, for a migration of the program's own listed before the
next file. [`freeze`](crate::MigrationFile::freeze) writes a file into the
migrations directory only when it is not there, so the code that froze each
change can stay and run again after the declaration has moved on: it writes
what is new and keeps what has shipped. Run it when the change is made, from
a test or a small binary, and then list the new files with
[`sql_migration!`](crate::guide::migrations), which reads them as the crate
compiles.

The bakery froze its payment tables when it opened and the voucher kind when
it added it, and a database built from nothing replays both:

```rust
# pub mod payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "payment")]
#     pub struct Model {
#         #[pgorm(primary_key)]
#         pub id: i32,
#         pub kind: super::PaymentKind,
#         pub order_id: i32,
#         #[pgorm(column_type = ColumnType::Decimal(Some((10, 2))))]
#         pub amount: Decimal,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# pub mod card_payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "card_payment")]
#     pub struct Model {
#         #[pgorm(primary_key, auto_increment = false)]
#         pub payment_id: i32,
#         pub last_four: String,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
# pub mod voucher_payment {
#     use pgorm::entity::prelude::*;
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "voucher_payment")]
#     pub struct Model {
#         #[pgorm(primary_key, auto_increment = false)]
#         pub payment_id: i32,
#         pub code: String,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {}
# }
use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = payment::Entity)]
pub enum Payment {
    Card(card_payment::Model),
    Cash,
    Voucher(voucher_payment::Model),
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
use pgorm::{MigrationFile, Schema};
use pgorm_migration::{MigrationTrait, MigratorTrait, sql_migration};

let migrations = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/guide/migrations");
let change = Schema::new().add_disjoint_kind::<Payment>(PaymentKind::Voucher)?;
let files = change.migration_files("m20260301_000001_add_voucher");
let names: Vec<&str> = files.iter().map(MigrationFile::name).collect();
assert_eq!(names, ["m20260301_000001_add_voucher_1", "m20260301_000001_add_voucher_2"]);
assert_eq!(
    files[0].text(),
    "-- pgorm: commits-after\nALTER TYPE \"payment_kind\" ADD VALUE 'voucher' AFTER 'cash';\n"
);
for file in &files {
    // Frozen when vouchers were added, so there is nothing to write.
    assert!(!file.freeze(migrations)?);
}

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(sql_migration!("docs/guide/migrations/m20260201_000001_create_payment.sql")),
            Box::new(sql_migration!("docs/guide/migrations/m20260301_000001_add_voucher_1.sql")),
            Box::new(sql_migration!("docs/guide/migrations/m20260301_000001_add_voucher_2.sql")),
        ]
    }
}

# let scratch = pgorm_guide_support::Scratch::empty("disjoint_frozen").await;
Migrator.up(scratch.pool.clone(), None).await?;

let db = scratch.pool.get().await?;
let (_, voucher) = Payment::insert(
    payment::ActiveModel {
        order_id: set(7),
        amount: set(Decimal::new(500, 2)),
        ..Default::default()
    },
    PaymentActiveModel::Voucher(voucher_payment::ActiveModel {
        code: set("SPRING-2026"),
        ..Default::default()
    }),
)?
.exec_returning_model(&db)
.await?;
assert_eq!(voucher.kind(), PaymentKind::Voucher);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Tables pgorm did not write

A schema written by hand may hold the same shape without a kind column: a
base table, and tables keyed by a foreign key onto it. Declare it with
`#[pgorm(base = .., unkinded)]`. A row's kind is then the kind whose table
has a row for it, and a unit variant, of which there is at most one, is a row
with a row in no kind's table.

Nothing in such a schema holds a row to one kind, so pgorm reads it and never
writes it: an unkinded enum has no `insert`, `update` or `change_kind`, and
no `create_disjoint`, and a call to one does not compile. `<Enum>Kind` is
still generated, as a plain enum.

```rust
pub mod staff {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "staff")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod driver {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "driver")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub staff_id: i32,
        pub licence: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = staff::Entity, unkinded)]
pub enum Staff {
    Driver(driver::Model),
    Counter,
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("disjoint_unkinded").await;
# let db = scratch.pool.get().await?;
db.batch_execute(
    "CREATE TABLE staff (id serial PRIMARY KEY, name text NOT NULL);
     CREATE TABLE driver (staff_id integer PRIMARY KEY REFERENCES staff, licence text NOT NULL);
     INSERT INTO staff (name) VALUES ('Eli'), ('Fay');
     INSERT INTO driver VALUES (1, 'B-1234');",
)
.await?;

let staff: Vec<(staff::Model, Staff)> = Staff::find()
    .order_by_asc(staff::Column::Id)
    .all(&db)
    .await?;
assert_eq!(staff[0].1, Staff::Driver(driver::Model { staff_id: 1, licence: "B-1234".to_owned() }));
assert_eq!(staff[1].1, Staff::Counter);
assert_eq!(Staff::find().kind(StaffKind::Counter).count(&db).await?, 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A row with rows in two kinds' tables, or in none when there is no unit
variant, is reported as `Error::Query` when it is read.
