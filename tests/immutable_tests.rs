#![allow(unused_imports, dead_code)]

//! Immutable columns on a live server: every statement pgorm builds that
//! would change one is refused before anything is sent, through each write
//! path, its row left as its insert wrote it, while the same paths writing
//! the row's other columns run.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54319" cargo nextest run --test it immutable_tests::`

use crate::common;
pub use common::{TestContext, setup::*};

use common::statements::Statements;
use pgorm::metric::InstrumentedPool;
use pgorm::pgorm_query::{ColumnType, OnConflict, StringLen, TableName, TypeName};
use pgorm::{DisjointColumn, DisjointShape, DisjointTable, Schema, entity::prelude::*};
use pretty_assertions::assert_eq;

/// A ledger entry: its key, external id, amount and recorder are written
/// once, and its memo is corrected.
pub mod entry {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "entry")]
    pub struct Model {
        #[pgorm(primary_key, immutable)]
        pub id: i32,
        #[pgorm(unique, immutable)]
        pub external_id: String,
        #[pgorm(immutable)]
        pub amount: i64,
        #[pgorm(immutable, column_name = "recorded_by")]
        pub author: String,
        pub memo: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// [`entry`] with no column immutable, over the same table.
pub mod plain_entry {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "entry")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(unique)]
        pub external_id: String,
        pub amount: i64,
        #[pgorm(column_name = "recorded_by")]
        pub author: String,
        pub memo: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A receipt whose `before_save` hook stamps who issued it on every save, an
/// update's among them.
pub mod receipt {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "receipt", hooks)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(immutable)]
        pub issued_by: String,
        pub body: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    #[async_trait::async_trait]
    impl ActiveModelBehavior for ActiveModel {
        async fn before_save<C>(mut self, _db: &C, _insert: bool) -> Result<Self, Error>
        where
            C: ConnectionMut,
        {
            self.issued_by = set("the till");
            Ok(self)
        }
    }
}

/// The base of [`Payment`]: its key, its payer and its kind are written
/// once, a payment keeping the kind it was made as.
pub mod payment {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "payment")]
    pub struct Model {
        #[pgorm(primary_key, immutable)]
        pub id: i64,
        #[pgorm(immutable)]
        pub kind: super::PaymentKind,
        #[pgorm(immutable)]
        pub payer: String,
        pub note: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The kind `card` of [`Payment`]: the card's last digits are written once,
/// its nickname is not.
pub mod card {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "card")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub payment_id: i64,
        #[pgorm(immutable)]
        pub last_four: String,
        pub nickname: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A payment by card, in a table of its own, or in cash.
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = payment::Entity)]
pub enum Payment {
    Card(card::Model),
    Cash,
}

fn refused<T: std::fmt::Debug>(result: Result<T, Error>, table: &str, column: &str) {
    assert!(
        matches!(
            &result,
            Err(Error::ImmutableColumn { table: t, column: c }) if t == table && c == column
        ),
        "expected `{table}`.`{column}` refused, got {result:?}"
    );
}

fn entry(id: i32, external_id: &str, amount: i64, memo: &str) -> entry::ActiveModel {
    entry::ActiveModel {
        id: set(id),
        external_id: set(external_id),
        amount: set(amount),
        author: set("ada"),
        memo: set(memo),
    }
}

async fn ledger<C: ConnectionMut>(db: &C) -> Result<Vec<entry::Model>, Error> {
    db.execute(
        &Schema::new()
            .create_table_from_entity(entry::Entity)?
            .to_string(),
        &[],
    )
    .await?;
    Insert::many([
        entry(1, "inv-1", 100, "rent"),
        entry(2, "inv-2", 250, "stock"),
    ])
    .exec(db)
    .await?;
    entries(db).await
}

async fn entries<C: ConnectionMut>(db: &C) -> Result<Vec<entry::Model>, Error> {
    entry::Entity::find()
        .order_by_asc(entry::Column::Id)
        .all(db)
        .await
}

// [spec:pgorm:def:entity.immutable/test]    the key writes `.immutable()` into its column's definition, renamed or keyed alike, and leaves the schema as it is without it
#[test]
fn the_key_marks_the_definition_not_the_schema() -> Result<(), Error> {
    let immutable: Vec<String> = <entry::Entity as Entity>::Column::iter()
        .filter(|column| column.def().is_immutable())
        .map(|column| column.as_str().to_owned())
        .collect();
    assert_eq!(immutable, ["id", "external_id", "amount", "recorded_by"]);
    assert!(!plain_entry::Column::Amount.def().is_immutable());
    assert!(ColumnType::Text.def().immutable().is_immutable());
    let schema = Schema::new();
    assert_eq!(
        schema.create_table_from_entity(entry::Entity)?.to_string(),
        schema
            .create_table_from_entity(plain_entry::Entity)?
            .to_string(),
    );
    Ok(())
}

// [spec:pgorm:req:entity.immutable.writes/test]    `Update::one`, its `_in` form and `ActiveModel::update` and `update_in` setting an immutable column are refused, a value a `before_save` hook sets as well, and a `SET` added through `Statement::query` when the update would run; nothing is sent, and an update setting the memo by an immutable key, or from a model read back, runs
#[pgorm_macros::test]
async fn update_one_refuses_an_immutable_column_live() -> Result<(), Error> {
    let ctx = TestContext::new("immutable_update_one").await;
    let statements = Statements::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), statements.clone());
    let db = pool.get().await?;
    let written = ledger(&db).await?;
    db.execute(
        &Schema::new()
            .create_table_from_entity(receipt::Entity)?
            .to_string(),
        &[],
    )
    .await?;
    db.execute(
        "INSERT INTO receipt (id, issued_by, body) VALUES (1, 'the counter', 'bread')",
        &[],
    )
    .await?;
    statements.take();

    let amount = entry::ActiveModel {
        id: set(1),
        amount: set(-100),
        ..Default::default()
    };
    refused(Update::one(amount.clone()), "entry", "amount");
    refused(
        Update::one_in(entry::Entity, amount.clone()),
        "entry",
        "amount",
    );
    refused(amount.clone().update(&db).await, "entry", "amount");
    refused(
        amount.update_in(entry::Entity, &db).await,
        "entry",
        "amount",
    );
    let recorder = entry::ActiveModel {
        id: set(1),
        author: set("mallory"),
        ..Default::default()
    };
    refused(recorder.update(&db).await, "entry", "recorded_by");
    let rekeyed = entry::ActiveModel {
        id: set(2),
        external_id: set("inv-1"),
        memo: set("stock, rebooked"),
        ..Default::default()
    };
    refused(rekeyed.update(&db).await, "entry", "external_id");

    let stamped = receipt::ActiveModel {
        id: set(1),
        body: set("bread and milk"),
        ..Default::default()
    };
    refused(stamped.update(&db).await, "receipt", "issued_by");

    let mut memo = Update::one(entry::ActiveModel {
        id: set(1),
        memo: set("rent, March"),
        ..Default::default()
    })?;
    Statement::query(&mut memo).value(entry::Column::Amount, Expr::value(0));
    refused(memo.exec_returning_model(&db).await, "entry", "amount");
    assert_eq!(statements.take(), Vec::<String>::new(), "nothing was sent");
    assert_eq!(entries(&db).await?, written);

    let corrected = entry::ActiveModel {
        id: set(1),
        memo: set("rent, March"),
        ..Default::default()
    }
    .update(&db)
    .await?;
    assert_eq!(
        corrected,
        entry::Model {
            memo: "rent, March".to_owned(),
            ..written[0].clone()
        }
    );
    let mut read_back = written[1].clone().into_active();
    read_back.memo = set("stock, delivered");
    assert_eq!(
        Update::one(read_back)?
            .exec_returning_model(&db)
            .await?
            .memo,
        "stock, delivered"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:entity.immutable.writes/test]    a bulk update naming an immutable column by `col_expr`, by its SQL name at run time, by `set`, or through `Statement::query`, the key among them, is refused from each terminal; nothing is sent, and a bulk update of the memo runs
#[pgorm_macros::test]
async fn update_many_refuses_an_immutable_column_live() -> Result<(), Error> {
    let ctx = TestContext::new("immutable_update_many").await;
    let statements = Statements::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), statements.clone());
    let db = pool.get().await?;
    let written = ledger(&db).await?;
    statements.take();

    let doubled = || {
        Update::many(entry::Entity).col_expr(
            entry::Column::Amount,
            Expr::col(entry::Column::Amount).mul(2),
        )
    };
    refused(doubled().exec(&db).await, "entry", "amount");
    refused(
        doubled().exec_returning_models(&db).await,
        "entry",
        "amount",
    );
    refused(
        doubled().exec_returning_changes(&db).await,
        "entry",
        "amount",
    );
    refused(
        Update::many(entry::Entity)
            .col_expr(entry::Column::Author, Expr::value("mallory"))
            .exec(&db)
            .await,
        "entry",
        "recorded_by",
    );
    refused(
        Update::many(entry::Entity)
            .col_expr(Name::runtime("recorded_by"), Expr::value("mallory"))
            .exec(&db)
            .await,
        "entry",
        "recorded_by",
    );
    refused(
        Update::many(entry::Entity)
            .col_expr(entry::Column::Id, Expr::col(entry::Column::Id).add(10))
            .exec(&db)
            .await,
        "entry",
        "id",
    );
    refused(
        Update::many(entry::Entity)
            .set(entry::ActiveModel {
                external_id: set("inv-0"),
                ..Default::default()
            })
            .filter(entry::Column::Id.eq(1))
            .exec(&db)
            .await,
        "entry",
        "external_id",
    );
    let mut memo = Update::many(entry::Entity).col_expr(entry::Column::Memo, Expr::value("?"));
    Statement::query(&mut memo).value(entry::Column::Amount, Expr::value(0));
    refused(memo.exec(&db).await, "entry", "amount");
    assert_eq!(statements.take(), Vec::<String>::new(), "nothing was sent");
    assert_eq!(entries(&db).await?, written);

    assert_eq!(
        Update::many(entry::Entity)
            .col_expr(entry::Column::Memo, Expr::value("audited"))
            .exec(&db)
            .await?,
        2
    );
    assert!(
        entries(&db)
            .await?
            .iter()
            .zip(&written)
            .all(|(now, then)| now.memo == "audited" && now.amount == then.amount)
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:entity.immutable.writes/test]    an upsert whose `DO UPDATE SET` names an immutable column, by `update_column` or `value`, under an inferred or a named arbiter, through `Insert` and `TryInsert`, one row or many, is refused from each terminal; nothing is sent, and an upsert naming only the memo leaves the amount as its insert wrote it
#[pgorm_macros::test]
async fn upserts_refuse_an_immutable_column_live() -> Result<(), Error> {
    let ctx = TestContext::new("immutable_upsert").await;
    let statements = Statements::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), statements.clone());
    let db = pool.get().await?;
    let written = ledger(&db).await?;
    statements.take();

    let again = || entry(3, "inv-1", 999, "rent, again");
    let amount =
        || OnConflict::column(entry::Column::ExternalId).update_column(entry::Column::Amount);
    refused(
        Insert::one(again()).on_conflict(amount()).exec(&db).await,
        "entry",
        "amount",
    );
    refused(
        Insert::one(again())
            .on_conflict(amount())
            .exec_returning_model(&db)
            .await,
        "entry",
        "amount",
    );
    refused(
        Insert::one(again())
            .on_conflict(amount())
            .exec_returning_pk(&db)
            .await,
        "entry",
        "amount",
    );
    refused(
        Insert::one(again())
            .on_conflict(amount())
            .exec_returning_upsert(&db)
            .await,
        "entry",
        "amount",
    );
    refused(
        Insert::one(again())
            .on_conflict(
                OnConflict::column(entry::Column::ExternalId)
                    .update_column(entry::Column::Memo)
                    .value(entry::Column::Author, Expr::value("mallory")),
            )
            .exec(&db)
            .await,
        "entry",
        "recorded_by",
    );
    refused(
        Insert::one(again())
            .on_conflict(
                OnConflict::constraint(Name::runtime("entry_external_id_key"))
                    .update_column(entry::Column::Amount),
            )
            .exec(&db)
            .await,
        "entry",
        "amount",
    );
    refused(
        Insert::one(again())
            .on_empty_do_nothing()
            .on_conflict(amount())
            .exec(&db)
            .await,
        "entry",
        "amount",
    );
    refused(
        Insert::many([again(), entry(4, "inv-2", 1, "stock, again")])
            .on_conflict(amount())
            .exec_returning_upserts(&db)
            .await,
        "entry",
        "amount",
    );
    refused(
        Insert::many([again()])
            .on_conflict(amount())
            .exec_returning_models(&db)
            .await,
        "entry",
        "amount",
    );
    let mut later = Insert::one(again());
    Statement::query(&mut later).on_conflict(amount());
    refused(later.exec(&db).await, "entry", "amount");
    assert_eq!(statements.take(), Vec::<String>::new(), "nothing was sent");
    assert_eq!(entries(&db).await?, written);

    let upserted = Insert::one(again())
        .on_conflict(
            OnConflict::column(entry::Column::ExternalId).update_column(entry::Column::Memo),
        )
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        upserted,
        entry::Model {
            memo: "rent, again".to_owned(),
            ..written[0].clone()
        },
        "the amount the conflicting row proposed is left out"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:entity.immutable.writes/test]    a disjoint update setting an immutable column of the base's row or of the kind's, and a change of kind whose kind column is immutable, are refused at construction; nothing is sent, and an update of the note and the nickname by an immutable key runs
#[pgorm_macros::test]
async fn disjoint_writes_refuse_an_immutable_column_live() -> Result<(), Error> {
    let ctx = TestContext::new("immutable_disjoint").await;
    let statements = Statements::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), statements.clone());
    let db = pool.get().await?;
    create_disjoint::<Payment>(&db).await?;
    let (paid, _) = Payment::insert(
        payment::ActiveModel {
            payer: set("ada"),
            note: set("lunch"),
            ..Default::default()
        },
        PaymentActiveModel::Card(card::ActiveModel {
            last_four: set("4242"),
            nickname: set("work"),
            ..Default::default()
        }),
    )?
    .exec_returning_model(&db)
    .await?;
    let before = Payment::find_by_id(paid.id).one(&db).await?;
    statements.take();

    let keyed = || payment::ActiveModel {
        id: set(paid.id),
        ..Default::default()
    };
    let nickname = || {
        PaymentActiveModel::Card(card::ActiveModel {
            nickname: set("travel"),
            ..Default::default()
        })
    };
    refused(
        Payment::update(
            payment::ActiveModel {
                payer: set("mallory"),
                ..keyed()
            },
            nickname(),
        ),
        "payment",
        "payer",
    );
    refused(
        Payment::update(
            keyed(),
            PaymentActiveModel::Card(card::ActiveModel {
                last_four: set("0000"),
                ..Default::default()
            }),
        ),
        "card",
        "last_four",
    );
    refused(
        Payment::change_kind(paid.id, PaymentActiveModel::Cash),
        "payment",
        "kind",
    );
    assert_eq!(statements.take(), Vec::<String>::new(), "nothing was sent");
    assert_eq!(Payment::find_by_id(paid.id).one(&db).await?, before);

    let (base, kind) = Payment::update(
        payment::ActiveModel {
            note: set("lunch, split"),
            ..keyed()
        },
        nickname(),
    )?
    .exec_returning_model(&db)
    .await?;
    assert_eq!(
        (base.payer.as_str(), base.note.as_str()),
        ("ada", "lunch, split")
    );
    assert_eq!(
        kind,
        Payment::Card(card::Model {
            payment_id: paid.id,
            last_four: "4242".to_owned(),
            nickname: "travel".to_owned(),
        })
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:entity.immutable.writes/test]    a disjoint shape described at run time refuses an update setting a column its definition marks immutable, in the base's row or the kind's, and a change of kind whose kind column is immutable
#[test]
fn a_run_time_shape_holds_its_immutable_columns() -> Result<(), Error> {
    let table = |name: &str, columns, key: &[&str]| {
        DisjointTable::new(TableName::Table(Name::runtime(name)), columns, key)
    };
    let kind_type = ColumnType::Enum {
        name: Name::runtime("payment_kind"),
        schema: None,
        variants: ["card", "cash"].map(Name::runtime).to_vec(),
    };
    let text = || ColumnType::String(StringLen::None).def();
    let shape = DisjointShape::new(
        table(
            "payment",
            vec![
                DisjointColumn::new("id", ColumnType::BigInteger.def().identity_by_default()),
                DisjointColumn::new("kind", kind_type.def().immutable()),
                DisjointColumn::new("payer", text().immutable()),
                DisjointColumn::new("note", text()),
            ],
            &["id"],
        )?,
        "kind",
        TypeName::new(Name::runtime("payment_kind")),
        vec![
            (
                "card".to_owned(),
                Some(table(
                    "card",
                    vec![
                        DisjointColumn::new("payment_id", ColumnType::BigInteger.def()),
                        DisjointColumn::new("last_four", text().immutable()),
                        DisjointColumn::new("nickname", text()),
                    ],
                    &["payment_id"],
                )?),
            ),
            ("cash".to_owned(), None),
        ],
    )?;
    let card = shape.kinds()[0].table().expect("a card has a table");
    let key = ("id", ActiveValue::Unchanged(Value::from(1_i64)));
    let base = |column: &str| {
        shape.base().values([
            key.clone(),
            (column, ActiveValue::Set(Value::from("changed"))),
        ])
    };
    let row = |column: &str| card.values([(column, ActiveValue::Set(Value::from("changed")))]);
    refused(
        shape.update(&base("payer")?, "card", Some(&row("nickname")?)),
        "payment",
        "payer",
    );
    refused(
        shape.update(&base("note")?, "card", Some(&row("last_four")?)),
        "card",
        "last_four",
    );
    refused(
        shape.change_kind(vec![Value::from(1_i64)], "cash", None),
        "payment",
        "kind",
    );
    assert!(
        shape
            .update(&base("note")?, "card", Some(&row("nickname")?))
            .is_ok()
    );
    Ok(())
}
