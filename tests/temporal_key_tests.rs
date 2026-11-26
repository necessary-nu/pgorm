#![allow(unused_imports, dead_code)]

//! PostgreSQL 18's temporal keys against a live server: a primary or unique
//! key ending `WITHOUT OVERLAPS`, and a foreign key matching on a `PERIOD`.
//!
//! The render tests in pgorm-query hold each spelling to the parser's
//! `Constraint` node. What only a server settles is what the clauses mean —
//! that two rows may share a key's other columns only while their periods do
//! not overlap, and that a referencing row's period must be covered by the
//! referenced rows' — and what it refuses around them. Each case sets the
//! refusal beside the row the same key admits, so a clause that rendered and
//! was then ignored still fails.

use crate::common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    ColumnDef, ColumnType, Deferrability, Enforcement, Expr, ForeignKey, ForeignKeyAction,
    ForeignKeyCreateStatement, Name, OnConflict, Primary, Query, RangeType, Table,
    TableCreateStatement, TableKey, Unique, extension::Extension,
};
use pgorm::{Connection, ConnectionMut, Transact, entity::prelude::*};
use pgorm_driver::error::SqlState;
use pretty_assertions::assert_eq;

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn date(y: i16, m: i8, d: i8) -> Date {
    jiff::civil::date(y, m, d)
}

/// `room (id integer, valid_at daterange, rate integer)`, keyed by `key`.
fn room(key: TableKey<Primary>) -> TableCreateStatement {
    Table::create(n("room"))
        .col(ColumnDef::new(n("id")).integer())
        .col(ColumnDef::new_with_type(
            n("valid_at"),
            ColumnType::Range(RangeType::Date),
        ))
        .col(ColumnDef::new(n("rate")).integer())
        .primary_key(key)
        .to_owned()
}

/// The room's temporal key: one rate per room at a time.
fn room_key() -> TableKey<Primary> {
    TableKey::new(n("id"))
        .without_overlaps(n("valid_at"))
        .name(n("room_pkey"))
}

/// `booking`'s temporal foreign key onto `room`: a booking's period inside
/// the room's.
fn booking_key() -> ForeignKeyCreateStatement {
    ForeignKey::create(n("booking"), n("room_id"), n("room"), n("id"))
        .name(n("booking_room"))
        .period(n("during"), n("valid_at"))
        .to_owned()
}

/// `booking (room_id integer, during daterange)`.
fn booking() -> TableCreateStatement {
    Table::create(n("booking"))
        .col(ColumnDef::new(n("room_id")).integer())
        .col(ColumnDef::new_with_type(
            n("during"),
            ColumnType::Range(RangeType::Date),
        ))
        .to_owned()
}

/// `btree_gist`, which gives the GiST index behind a temporal key an
/// operator class for its scalar columns.
async fn btree_gist(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(&Extension::create(n("btree_gist")).to_string())
        .await
}

/// The rooms the foreign-key cases reference: room 1 over two adjacent
/// months, room 2 over the first.
async fn rooms(db: &DatabaseConnection) -> Result<(), Error> {
    btree_gist(db).await?;
    db.batch_execute(&room(room_key()).to_string()).await?;
    db.batch_execute(
        "INSERT INTO room VALUES \
         (1, '[2020-01-01,2020-02-01)', 10), \
         (1, '[2020-02-01,2020-03-01)', 12), \
         (2, '[2020-01-01,2020-02-01)', 20)",
    )
    .await
}

/// `pg_get_constraintdef` and `conperiod` of the constraint `name` on `table`.
async fn definition(
    db: &DatabaseConnection,
    table: &str,
    name: &str,
) -> Result<(String, bool), Error> {
    let row = db
        .query_one(
            "SELECT pg_get_constraintdef(oid), conperiod FROM pg_constraint \
             WHERE conrelid = $1::text::regclass AND conname = $2",
            &[&table, &name],
        )
        .await?;
    Ok((row.get(0), row.get(1)))
}

/// A key ending `WITHOUT OVERLAPS` admits two rows for one room only while
/// their periods do not overlap: adjacent months are both kept, an overlap is
/// refused as the exclusion it is (`23P01`), and so is an empty period
/// (`23514`). The plain key over the same columns, the control, admits the
/// overlap. The GiST index behind the key needs `btree_gist` for the integer
/// column (`42704` without it).
// [spec:pgorm:req:sql.ddl.create-table+16/test]    against a live server: a WITHOUT OVERLAPS
// key refuses overlapping periods for one key and admits adjacent ones
#[pgorm_macros::test]
async fn a_temporal_key_refuses_overlapping_periods() -> Result<(), Error> {
    let ctx = TestContext::new("temporal_key_refuses_overlaps").await;
    let db = ctx.db.get().await?;

    let create = room(room_key()).to_string();
    let refused = db
        .batch_execute(&create)
        .await
        .expect_err("no GiST operator class for integer without btree_gist");
    refused_with(&refused, &SqlState::UNDEFINED_OBJECT);

    btree_gist(&db).await?;
    db.batch_execute(&create).await?;
    assert_eq!(
        definition(&db, "room", "room_pkey").await?,
        (
            "PRIMARY KEY (id, valid_at WITHOUT OVERLAPS)".to_owned(),
            true
        )
    );

    db.batch_execute(
        "INSERT INTO room VALUES \
         (1, '[2020-01-01,2020-02-01)', 10), \
         (1, '[2020-02-01,2020-03-01)', 12), \
         (2, '[2020-01-15,2020-03-01)', 20)",
    )
    .await?;
    let overlap = db
        .batch_execute("INSERT INTO room VALUES (1, '[2020-01-20,2020-02-10)', 11)")
        .await
        .expect_err("an overlapping period for room 1");
    refused_with(&overlap, &SqlState::EXCLUSION_VIOLATION);
    let empty = db
        .batch_execute("INSERT INTO room VALUES (3, 'empty', 30)")
        .await
        .expect_err("an empty period");
    refused_with(&empty, &SqlState::CHECK_VIOLATION);

    db.batch_execute("DROP TABLE room").await?;
    db.batch_execute(&room(TableKey::new(n("id")).col(n("valid_at"))).to_string())
        .await?;
    db.batch_execute(
        "INSERT INTO room VALUES \
         (1, '[2020-01-01,2020-02-01)', 10), (1, '[2020-01-20,2020-02-10)', 11)",
    )
    .await?;

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A unique key may end `WITHOUT OVERLAPS` too, with the unique kind's
/// `NULLS NOT DISTINCT`, under which two rows with no room and one period
/// conflict where the plain temporal key admits them; and it may be deferred,
/// holding an overlap between statements until `COMMIT` refuses it.
// [spec:pgorm:req:sql.ddl.create-table+16/test]    against a live server: a temporal unique key
// takes NULLS NOT DISTINCT and deferrability, and both mean what they mean on a plain key
#[pgorm_macros::test]
async fn a_temporal_unique_key_takes_its_options() -> Result<(), Error> {
    let ctx = TestContext::new("temporal_unique_takes_options").await;
    let mut db = ctx.db.get().await?;
    btree_gist(&db).await?;

    let slot = |table: &str, key: TableKey<Unique>| {
        Table::create(n(table))
            .col(ColumnDef::new(n("room_id")).integer())
            .col(ColumnDef::new_with_type(
                n("during"),
                ColumnType::Range(RangeType::Date),
            ))
            .unique(key)
            .to_string()
    };
    let key = || TableKey::new(n("room_id")).without_overlaps(n("during"));
    db.batch_execute(&slot("plain_slot", key())).await?;
    db.batch_execute(&slot("strict_slot", key().nulls_not_distinct()))
        .await?;
    db.batch_execute(&slot(
        "deferred_slot",
        key().deferrability(Deferrability::DeferrableInitiallyDeferred),
    ))
    .await?;

    let twice = |table: &str| {
        format!(
            "INSERT INTO {table} VALUES \
             (NULL, '[2020-01-01,2020-02-01)'), (NULL, '[2020-01-01,2020-02-01)')"
        )
    };
    db.batch_execute(&twice("plain_slot")).await?;
    let refused = db
        .batch_execute(&twice("strict_slot"))
        .await
        .expect_err("NULLS NOT DISTINCT makes the two rows one key");
    refused_with(&refused, &SqlState::EXCLUSION_VIOLATION);

    let refused = commit_refused(
        db.transaction(async |txn| {
            txn.batch_execute(
                "INSERT INTO deferred_slot VALUES (1, '[2020-01-01,2020-03-01)'), \
                 (1, '[2020-02-01,2020-04-01)')",
            )
            .await
        })
        .await,
    );
    refused_with(&refused, &SqlState::EXCLUSION_VIOLATION);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A `PERIOD` foreign key holds a booking to the room's periods: a booking
/// spanning the room's two adjacent months is covered by their union, one
/// running past them or naming no room is refused (`23503`), and so is
/// deleting a month a booking needs. A plain foreign key onto the temporal
/// key, the same pairs without `PERIOD`, is refused outright (`42830`).
// [spec:pgorm:req:sql.ddl.foreign-key+11/test]    against a live server: a PERIOD foreign key
// needs the referencing period covered by the referenced rows' periods
#[pgorm_macros::test]
async fn a_period_foreign_key_needs_a_covering_row() -> Result<(), Error> {
    let ctx = TestContext::new("period_foreign_key_covering").await;
    let db = ctx.db.get().await?;
    rooms(&db).await?;

    let plain = booking()
        .foreign_key(
            ForeignKey::create(n("booking"), n("room_id"), n("room"), n("id"))
                .col(n("during"), n("valid_at"))
                .to_owned(),
        )
        .to_string();
    let refused = db
        .batch_execute(&plain)
        .await
        .expect_err("a plain foreign key onto a temporal key");
    refused_with(&refused, &SqlState::INVALID_FOREIGN_KEY);

    db.batch_execute(&booking().foreign_key(booking_key()).to_string())
        .await?;
    assert_eq!(
        definition(&db, "booking", "booking_room").await?,
        (
            "FOREIGN KEY (room_id, PERIOD during) REFERENCES room(id, PERIOD valid_at)".to_owned(),
            true
        )
    );

    db.batch_execute("INSERT INTO booking VALUES (1, '[2020-01-20,2020-02-10)')")
        .await?;
    for (row, why) in [
        (
            "(1, '[2020-02-20,2020-03-10)')",
            "past the room's last month",
        ),
        ("(2, '[2020-01-20,2020-02-10)')", "past room 2's one month"),
        ("(3, '[2020-01-01,2020-01-02)')", "no room 3"),
    ] {
        let refused = db
            .batch_execute(&format!("INSERT INTO booking VALUES {row}"))
            .await
            .expect_err(why);
        refused_with(&refused, &SqlState::FOREIGN_KEY_VIOLATION);
    }
    let refused = db
        .batch_execute("DELETE FROM room WHERE id = 1 AND valid_at = '[2020-02-01,2020-03-01)'")
        .await
        .expect_err("the booking needs February");
    refused_with(&refused, &SqlState::FOREIGN_KEY_VIOLATION);
    db.batch_execute("DELETE FROM room WHERE id = 2").await?;

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// PostgreSQL 19 runs a `PERIOD` foreign key's referential actions as `NO
/// ACTION` only: every other action, on delete or on update, is refused
/// (`0A000`), and `NO ACTION` said outright is taken. Deferrability and
/// enforcement it takes as a plain key does.
// [spec:pgorm:req:sql.ddl.foreign-key+11/test]    against a live server: a PERIOD foreign key
// takes NO ACTION and refuses every other referential action
#[pgorm_macros::test]
async fn a_period_foreign_key_takes_only_no_action() -> Result<(), Error> {
    let ctx = TestContext::new("period_foreign_key_no_action").await;
    let db = ctx.db.get().await?;
    rooms(&db).await?;
    db.batch_execute(&booking().to_string()).await?;

    let refused = [
        ForeignKeyAction::Cascade,
        ForeignKeyAction::SetNull,
        ForeignKeyAction::SetDefault,
        ForeignKeyAction::Restrict,
    ];
    for action in refused {
        let on_delete = booking_key().on_delete(action).to_string();
        let on_update = booking_key().on_update(action).to_string();
        for alter in [on_delete, on_update] {
            let error = db.batch_execute(&alter).await.expect_err(&alter);
            refused_with(&error, &SqlState::FEATURE_NOT_SUPPORTED);
        }
    }

    let no_action = booking_key()
        .on_delete(ForeignKeyAction::NoAction)
        .on_update(ForeignKeyAction::NoAction)
        .to_string();
    db.batch_execute(&no_action).await?;

    let later = booking_key()
        .name(n("booking_room_later"))
        .deferrability(Deferrability::DeferrableInitiallyDeferred)
        .enforcement(Enforcement::NotEnforced)
        .to_string();
    db.batch_execute(&later).await?;
    let row = db
        .query_one(
            "SELECT conperiod, condeferred, conenforced FROM pg_constraint \
             WHERE conname = 'booking_room_later'",
            &[],
        )
        .await?;
    assert_eq!(
        (row.get(0), row.get(1), row.get(2)),
        (true, true, false),
        "a temporal key deferred and not enforced, as a plain one can be"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A temporal key is enforced as an exclusion constraint, not a unique index,
/// so an `ON CONFLICT` cannot infer it from its columns (`42P10`); named, it
/// arbitrates `DO NOTHING`, an overlapping row being the conflict, and refuses
/// `DO UPDATE` (`42809`), as any exclusion constraint does.
// [spec:pgorm:req:sql.ast.on-conflict+4/test]    against a live server: a temporal key
// arbitrates only by name and only DO NOTHING
#[pgorm_macros::test]
async fn a_temporal_key_arbitrates_only_by_name() -> Result<(), Error> {
    let ctx = TestContext::new("temporal_key_arbitrates_by_name").await;
    let db = ctx.db.get().await?;
    rooms(&db).await?;

    let insert = |conflict: OnConflict| {
        Query::insert()
            .into_table(n("room"))
            .columns([n("id"), n("valid_at"), n("rate")])
            .values_panic([
                1.into(),
                Expr::value(Range::from(date(2020, 1, 20)..date(2020, 2, 10))),
                11.into(),
            ])
            .on_conflict(conflict)
            .to_string()
    };

    let inferred = insert(OnConflict::columns((n("id"), n("valid_at"))).do_nothing());
    let refused = db.execute(&inferred, &[]).await.expect_err(&inferred);
    refused_with(&refused, &SqlState::INVALID_COLUMN_REFERENCE);

    let named = insert(OnConflict::constraint(n("room_pkey")).do_nothing());
    assert_eq!(db.execute(&named, &[]).await?, 0, "{named}");
    let rates = db
        .query_all("SELECT rate FROM room WHERE id = 1 ORDER BY valid_at", &[])
        .await?;
    assert_eq!(
        rates.iter().map(|row| row.get(0)).collect::<Vec<i32>>(),
        [10, 12],
        "the overlapping row was the conflict"
    );

    let updated = insert(
        OnConflict::constraint(n("room_pkey"))
            .update_column(n("rate"))
            .into(),
    );
    let refused = db.execute(&updated, &[]).await.expect_err(&updated);
    refused_with(&refused, &SqlState::WRONG_OBJECT_TYPE);

    drop(db);
    ctx.delete().await;
    Ok(())
}
