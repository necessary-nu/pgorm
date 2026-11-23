#![allow(unused_imports, dead_code)]

//! `ON CONFLICT ON CONSTRAINT` against a live PostgreSQL server.
//!
//! The render tests in pgorm-query hold the spelling to the parser's
//! `InferClause`. What only a server settles is which constraint a name
//! reaches and what it does there: that the named constraint arbitrates and
//! no other does, so a conflict elsewhere in the row is still raised; that an
//! exclusion constraint, which index inference cannot reach at all, takes
//! `DO NOTHING` and refuses `DO UPDATE`; and that a name which is not an
//! arbiter — a standalone unique index, a check, a deferrable key — is
//! refused rather than quietly read as something else. Each case sets the
//! named form beside the answer another arbiter gives the same row, so a name
//! that rendered and was then ignored still fails.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{Expr, Name, OnConflict, Query};
use pgorm::{ConnectionTrait, entity::prelude::*};
use tokio_postgres::error::SqlState;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("conflict_constraint_tests").await;
    let db = ctx.db.get().await?;

    db.batch_execute(
        "CREATE TABLE slot (\
             id integer CONSTRAINT slot_pk PRIMARY KEY, \
             k integer NOT NULL CONSTRAINT \"SlotKey\" UNIQUE, \
             v text NOT NULL, \
             CONSTRAINT slot_positive CHECK (k > 0)); \
         CREATE UNIQUE INDEX slot_v ON slot (v); \
         INSERT INTO slot VALUES (1, 1, 'a'); \
         CREATE TABLE booking (\
             id integer, \
             during int4range NOT NULL, \
             CONSTRAINT booking_overlap EXCLUDE USING gist (during WITH &&)); \
         INSERT INTO booking VALUES (1, '[1,5)'); \
         CREATE TABLE deferred (k integer CONSTRAINT deferred_k UNIQUE DEFERRABLE); \
         INSERT INTO deferred VALUES (1)",
    )
    .await?;

    the_named_constraint_arbitrates(&db).await?;
    a_conflict_on_another_constraint_is_raised(&db).await?;
    an_exclusion_constraint_takes_only_do_nothing(&db).await?;
    what_is_not_an_arbiter_is_refused(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// `INSERT INTO slot (id, k, v) VALUES (..) <conflict>`.
fn slot(id: i32, k: i32, v: &str, conflict: impl Into<OnConflict>) -> String {
    Query::insert()
        .into_table(Name::runtime("slot"))
        .columns([Name::runtime("id"), Name::runtime("k"), Name::runtime("v")])
        .values_panic([id.into(), k.into(), v.into()])
        .on_conflict(conflict)
        .to_string()
}

async fn values(db: &DatabaseConnection) -> Result<Vec<(i32, String)>, Error> {
    let rows = db
        .query_all("SELECT id, v FROM slot ORDER BY id", &[])
        .await?;
    Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
}

fn slot_key() -> Name {
    Name::runtime("SlotKey")
}

/// A conflict on `"SlotKey"` is answered by the clause that names it: `DO
/// UPDATE` takes the proposed row's value, under its own filter, and `DO
/// NOTHING` writes nothing. The name is mixed-case, so it is found only
/// because it is quoted: bare, it folds to `slotkey`, which the table does
/// not have.
// [spec:pgorm:req:sql.ast.on-conflict+4/test]    against a live server: the named constraint
// arbitrates both actions, and the update's filter still applies
// [spec:pgorm:req:sql.render.on-conflict+2/test]
// [spec:pgorm:req:sql.scope+23/test]
async fn the_named_constraint_arbitrates(db: &DatabaseConnection) -> Result<(), Error> {
    let updated = slot(
        2,
        1,
        "b",
        OnConflict::constraint(slot_key()).update_column(Name::runtime("v")),
    );
    assert_eq!(db.execute(&updated, &[]).await?, 1, "{updated}");
    assert_eq!(values(db).await?, [(1, "b".to_owned())]);

    let filtered = slot(
        2,
        1,
        "c",
        OnConflict::constraint(slot_key())
            .update_column(Name::runtime("v"))
            .and_where(Expr::col((Name::runtime("slot"), Name::runtime("v"))).eq("a")),
    );
    assert_eq!(db.execute(&filtered, &[]).await?, 0, "{filtered}");
    assert_eq!(values(db).await?, [(1, "b".to_owned())]);

    let ignored = slot(2, 1, "c", OnConflict::constraint(slot_key()).do_nothing());
    assert_eq!(db.execute(&ignored, &[]).await?, 0, "{ignored}");
    assert_eq!(values(db).await?, [(1, "b".to_owned())]);

    let folded = ignored.replace("\"SlotKey\"", "SlotKey");
    let refused = db.execute(&folded, &[]).await.expect_err(&folded);
    refused_with(&refused, &SqlState::UNDEFINED_OBJECT);

    Ok(())
}

/// The row below collides with the primary key and not with `"SlotKey"`.
/// Naming `"SlotKey"` does not swallow it, where the arbiter-less clause —
/// any constraint — does.
// [spec:pgorm:req:sql.ast.on-conflict+4/test]    against a live server: only the named
// constraint arbitrates
// [spec:pgorm:req:sql.scope+23/test]
async fn a_conflict_on_another_constraint_is_raised(db: &DatabaseConnection) -> Result<(), Error> {
    let named = slot(1, 7, "z", OnConflict::constraint(slot_key()).do_nothing());
    let raised = db.execute(&named, &[]).await.expect_err(&named);
    refused_with(&raised, &SqlState::UNIQUE_VIOLATION);

    let any = slot(1, 7, "z", OnConflict::do_nothing());
    assert_eq!(db.execute(&any, &[]).await?, 0, "{any}");
    assert_eq!(values(db).await?, [(1, "b".to_owned())]);

    Ok(())
}

/// An exclusion constraint is an arbiter only by name: inference, which
/// looks for a unique index over the listed columns, finds none (`42P10`).
/// Named, it swallows an overlapping row under `DO NOTHING`, and refuses
/// `DO UPDATE` outright (`42809`), because an exclusion conflict has no one
/// row to update.
// [spec:pgorm:req:sql.ast.on-conflict+4/test]    against a live server: an exclusion
// constraint arbitrates DO NOTHING only, and only by name
// [spec:pgorm:req:sql.scope+23/test]
async fn an_exclusion_constraint_takes_only_do_nothing(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let booking = |conflict: OnConflict| {
        Query::insert()
            .into_table(Name::runtime("booking"))
            .columns([Name::runtime("id"), Name::runtime("during")])
            .values_panic([
                2.into(),
                Expr::val("[3,8)").cast_as(Name::runtime("int4range")),
            ])
            .on_conflict(conflict)
            .to_string()
    };
    let overlap = || OnConflict::constraint(Name::runtime("booking_overlap"));

    let ignored = booking(overlap().do_nothing());
    assert_eq!(db.execute(&ignored, &[]).await?, 0, "{ignored}");

    let updated = booking(overlap().value(Name::runtime("id"), 3).into());
    let refused = db.execute(&updated, &[]).await.expect_err(&updated);
    refused_with(&refused, &SqlState::WRONG_OBJECT_TYPE);

    let inferred = booking(OnConflict::column(Name::runtime("during")).do_nothing());
    let refused = db.execute(&inferred, &[]).await.expect_err(&inferred);
    refused_with(&refused, &SqlState::INVALID_COLUMN_REFERENCE);

    let rows = db.query_one("SELECT count(*) FROM booking", &[]).await?;
    assert_eq!(rows.get::<_, i64>(0), 1);

    Ok(())
}

/// A name the server finds but cannot arbitrate by is refused, each for its
/// own reason: a unique index made by `CREATE UNIQUE INDEX` is no constraint
/// at all (`42704`); a check constraint has no index (`42809`); and a
/// deferrable key cannot say whether a row conflicts while its check may be
/// pending (`55000`).
// [spec:pgorm:req:sql.ast.on-conflict+4/test]    against a live server: a name that is not
// an arbiter is refused, never reinterpreted
// [spec:pgorm:req:sql.scope+23/test]
async fn what_is_not_an_arbiter_is_refused(db: &DatabaseConnection) -> Result<(), Error> {
    for (name, state) in [
        ("slot_v", SqlState::UNDEFINED_OBJECT),
        ("slot_positive", SqlState::WRONG_OBJECT_TYPE),
        ("no_such_constraint", SqlState::UNDEFINED_OBJECT),
    ] {
        let insert = slot(
            2,
            2,
            "a",
            OnConflict::constraint(Name::runtime(name)).do_nothing(),
        );
        let refused = db.execute(&insert, &[]).await.expect_err(&insert);
        refused_with(&refused, &state);
    }

    let deferred = Query::insert()
        .into_table(Name::runtime("deferred"))
        .columns([Name::runtime("k")])
        .values_panic([1.into()])
        .on_conflict(OnConflict::constraint(Name::runtime("deferred_k")).do_nothing())
        .to_string();
    let refused = db.execute(&deferred, &[]).await.expect_err(&deferred);
    refused_with(&refused, &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE);

    Ok(())
}
