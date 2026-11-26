#![allow(unused_imports, dead_code)]

//! `CREATE` / `ALTER` / `DROP SEQUENCE` against a live PostgreSQL server.
//!
//! The render tests in pgorm-query hold each statement to the parser's
//! `CreateSeqStmt`, `AlterSeqStmt`, `DropStmt` and `RenameStmt`, option by
//! option. What only a server settles is what the options mean: that the
//! values a sequence hands out follow its step, bounds, start and wrap; that
//! the catalogue records exactly the definition the builder wrote; that
//! `OWNED BY` ties the sequence's life to a column's; and that the numbers the
//! type leaves to the server — a zero step, crossed bounds, a bound outside the
//! counting type — are refused there, each with the same `22023`.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{Name, Sequence, SequenceOption, SequenceType};
use pgorm::{Connection, entity::prelude::*};
use pgorm_driver::error::SqlState;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("sequence_tests").await;
    let db = ctx.db.get().await?;

    a_sequence_hands_out_what_its_options_say(&db).await?;
    the_counting_type_sets_the_default_bounds(&db).await?;
    an_alter_changes_a_live_sequence(&db).await?;
    owned_by_ties_a_sequence_to_its_column(&db).await?;
    a_rename_and_a_drop_reach_the_sequence(&db).await?;
    the_server_judges_the_numbers(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// A sequence's definition as the catalogue holds it: type, start, minimum,
/// maximum, step, cycle, cache.
type Definition = (String, i64, i64, i64, i64, bool, i64);

async fn definition(db: &DatabaseConnection, name: &str) -> Result<Definition, Error> {
    let row = db
        .query_one(
            "SELECT data_type::text, start_value, min_value, max_value, increment_by, cycle, \
             cache_size FROM pg_sequences WHERE sequencename = $1",
            &[&name],
        )
        .await?;
    Ok((
        row.get(0),
        row.get(1),
        row.get(2),
        row.get(3),
        row.get(4),
        row.get(5),
        row.get(6),
    ))
}

async fn next(db: &DatabaseConnection, name: &str, times: usize) -> Result<Vec<i64>, Error> {
    let mut values = Vec::with_capacity(times);
    for _ in 0..times {
        let row = db
            .query_one("SELECT nextval($1::text::regclass)", &[&name])
            .await?;
        values.push(row.get(0));
    }
    Ok(values)
}

async fn exists(db: &DatabaseConnection, name: &str) -> Result<bool, Error> {
    let row = db
        .query_one(
            "SELECT count(*) FROM pg_class WHERE relkind = 'S' AND relname = $1",
            &[&name],
        )
        .await?;
    Ok(row.get::<_, i64>(0) == 1)
}

/// Every option reaches the catalogue, and the values follow them: a step of
/// two from one wraps back to one at five under `CYCLE`, and stops at the
/// bound under `NO CYCLE` (`2200H`).
// [spec:pgorm:req:sql.ddl.sequence/test]
async fn a_sequence_hands_out_what_its_options_say(db: &DatabaseConnection) -> Result<(), Error> {
    let create = Sequence::create(n("wraps"))
        .as_type(SequenceType::Integer)
        .options(
            SequenceOption::IncrementBy(2)
                .and(SequenceOption::MinValue(1))
                .and(SequenceOption::MaxValue(5))
                .and(SequenceOption::StartWith(1))
                .and(SequenceOption::Cache(3))
                .and(SequenceOption::Cycle),
        )
        .to_string();
    db.batch_execute(&create).await?;
    assert_eq!(
        definition(db, "wraps").await?,
        ("integer".to_owned(), 1, 1, 5, 2, true, 3)
    );
    assert_eq!(next(db, "wraps", 4).await?, [1, 3, 5, 1]);

    let create = Sequence::create(n("stops"))
        .options(SequenceOption::MaxValue(2).and(SequenceOption::NoCycle))
        .to_string();
    db.batch_execute(&create).await?;
    assert_eq!(next(db, "stops", 2).await?, [1, 2]);
    let exhausted = next(db, "stops", 1)
        .await
        .expect_err("a NO CYCLE sequence went past its bound");
    refused_with(&exhausted, &SqlState::SEQUENCE_GENERATOR_LIMIT_EXCEEDED);

    // `IF NOT EXISTS` leaves the existing definition alone.
    let again = Sequence::create(n("wraps"))
        .if_not_exists()
        .options(SequenceOption::StartWith(100))
        .to_string();
    db.batch_execute(&again).await?;
    assert_eq!(definition(db, "wraps").await?.1, 1);

    Ok(())
}

/// `NO MINVALUE` / `NO MAXVALUE` mean the counting type's own bounds, which
/// is what `AS` chooses — and a descending `smallint` sequence counts down
/// from -1 to the type's minimum.
// [spec:pgorm:req:sql.ddl.sequence/test]
async fn the_counting_type_sets_the_default_bounds(db: &DatabaseConnection) -> Result<(), Error> {
    let create = Sequence::create(n("down"))
        .as_type(SequenceType::SmallInteger)
        .options(
            SequenceOption::IncrementBy(-1)
                .and(SequenceOption::NoMinValue)
                .and(SequenceOption::NoMaxValue),
        )
        .to_string();
    db.batch_execute(&create).await?;
    assert_eq!(
        definition(db, "down").await?,
        (
            "smallint".to_owned(),
            -1,
            i64::from(i16::MIN),
            -1,
            -1,
            false,
            1
        )
    );
    assert_eq!(next(db, "down", 2).await?, [-1, -2]);

    let create = Sequence::create(n("wide")).to_string();
    db.batch_execute(&create).await?;
    assert_eq!(
        definition(db, "wide").await?,
        ("bigint".to_owned(), 1, 1, i64::MAX, 1, false, 1)
    );

    Ok(())
}

/// Each clause an alter begins with changes the live sequence: `RESTART WITH`
/// moves the next value, `INCREMENT BY` the step, and `AS` the type — taking
/// along a bound that sat at the old type's limit.
// [spec:pgorm:req:sql.ddl.sequence/test]
async fn an_alter_changes_a_live_sequence(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(
        &Sequence::create(n("counter"))
            .as_type(SequenceType::SmallInteger)
            .to_string(),
    )
    .await?;
    assert_eq!(next(db, "counter", 2).await?, [1, 2]);

    let alter = Sequence::alter(n("counter"))
        .restart_with(50)
        .options(SequenceOption::IncrementBy(5))
        .to_string();
    db.batch_execute(&alter).await?;
    assert_eq!(next(db, "counter", 2).await?, [50, 55]);

    db.batch_execute(
        &Sequence::alter(n("counter"))
            .as_type(SequenceType::Integer)
            .to_string(),
    )
    .await?;
    assert_eq!(
        definition(db, "counter").await?,
        ("integer".to_owned(), 1, 1, i64::from(i32::MAX), 5, false, 1)
    );

    db.batch_execute(&Sequence::alter(n("counter")).restart().to_string())
        .await?;
    assert_eq!(next(db, "counter", 1).await?, [1]);

    // `IF EXISTS` turns a missing sequence into a notice.
    db.batch_execute(
        &Sequence::alter(n("missing"))
            .restart()
            .if_exists()
            .to_string(),
    )
    .await?;

    Ok(())
}

/// `OWNED BY` makes the sequence the column's: dropping the table drops it,
/// and `OWNED BY NONE` takes that back. The owner has to share the sequence's
/// schema (`55000`).
// [spec:pgorm:req:sql.ddl.sequence/test]
async fn owned_by_ties_a_sequence_to_its_column(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(
        "CREATE TABLE ledger (no bigint); CREATE TABLE spare (no bigint); \
         CREATE SCHEMA elsewhere; CREATE TABLE elsewhere.ledger (no bigint)",
    )
    .await?;

    db.batch_execute(
        &Sequence::create(n("ledger_no"))
            .owned_by(n("ledger"), n("no"))
            .to_string(),
    )
    .await?;
    db.batch_execute("DROP TABLE ledger").await?;
    assert!(
        !exists(db, "ledger_no").await?,
        "dropping the owning table left the sequence behind"
    );

    db.batch_execute(
        &Sequence::create(n("spare_no"))
            .owned_by(n("spare"), n("no"))
            .to_string(),
    )
    .await?;
    db.batch_execute(&Sequence::alter(n("spare_no")).owned_by_none().to_string())
        .await?;
    db.batch_execute("DROP TABLE spare").await?;
    assert!(
        exists(db, "spare_no").await?,
        "OWNED BY NONE did not release the sequence"
    );

    let across = db
        .batch_execute(
            &Sequence::create(n("across"))
                .owned_by((n("elsewhere"), n("ledger")), n("no"))
                .to_string(),
        )
        .await
        .expect_err("a sequence was owned by a table in another schema");
    refused_with(&across, &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE);

    Ok(())
}

/// A rename moves the name and keeps the sequence; a drop refuses while a
/// column default still calls it (`2BP01`), and `CASCADE` takes the default
/// with it.
// [spec:pgorm:req:sql.ddl.sequence/test]
async fn a_rename_and_a_drop_reach_the_sequence(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(&Sequence::create(n("old_name")).to_string())
        .await?;
    db.batch_execute(&Sequence::rename(n("old_name"), n("new_name")).to_string())
        .await?;
    assert!(!exists(db, "old_name").await? && exists(db, "new_name").await?);

    db.batch_execute("CREATE TABLE fed (id bigint DEFAULT nextval('new_name'))")
        .await?;
    let refused = db
        .batch_execute(&Sequence::drop(n("new_name")).to_string())
        .await
        .expect_err("a sequence a default calls was dropped");
    refused_with(&refused, &SqlState::DEPENDENT_OBJECTS_STILL_EXIST);

    db.batch_execute(
        &Sequence::drop(n("new_name"))
            .name(n("wide"))
            .cascade()
            .to_string(),
    )
    .await?;
    assert!(!exists(db, "new_name").await? && !exists(db, "wide").await?);
    let row = db
        .query_one(
            "SELECT count(*) FROM pg_attrdef d JOIN pg_class c ON c.oid = d.adrelid \
             WHERE c.relname = 'fed'",
            &[],
        )
        .await?;
    assert_eq!(row.get::<_, i64>(0), 0, "CASCADE left the default behind");

    db.batch_execute(&Sequence::drop(n("new_name")).if_exists().to_string())
        .await?;

    Ok(())
}

/// The numbers are the server's to judge: each is checked against the whole
/// definition, so a value no single option could refuse alone is refused
/// there, every one with `22023`.
// [spec:pgorm:req:sql.ddl.sequence/test]
async fn the_server_judges_the_numbers(db: &DatabaseConnection) -> Result<(), Error> {
    let refused = [
        Sequence::create(n("bad"))
            .options(SequenceOption::IncrementBy(0))
            .to_owned(),
        Sequence::create(n("bad"))
            .options(SequenceOption::Cache(0))
            .to_owned(),
        Sequence::create(n("bad"))
            .options(SequenceOption::MinValue(10).and(SequenceOption::MaxValue(5)))
            .to_owned(),
        Sequence::create(n("bad"))
            .options(SequenceOption::MaxValue(50).and(SequenceOption::StartWith(100)))
            .to_owned(),
        Sequence::create(n("bad"))
            .as_type(SequenceType::SmallInteger)
            .options(SequenceOption::MaxValue(99_999))
            .to_owned(),
    ];
    for create in refused {
        let sql = create.to_string();
        let error = db
            .batch_execute(&sql)
            .await
            .expect_err(&format!("the server accepted {sql}"));
        refused_with(&error, &SqlState::INVALID_PARAMETER_VALUE);
    }
    assert!(!exists(db, "bad").await?);

    Ok(())
}
