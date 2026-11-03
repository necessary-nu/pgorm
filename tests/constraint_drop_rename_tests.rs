#![allow(unused_imports, dead_code)]

//! A constraint dropped and renamed by name, against a live server.
//!
//! The render tests in pgorm-query settle that `DROP CONSTRAINT` and `RENAME
//! CONSTRAINT` parse. What only a server settles is that the one drop serves a
//! constraint of every kind — a key, a foreign key, a `CHECK`, PostgreSQL
//! 18's `NOT NULL` — so the row the constraint refused is admitted once it is
//! gone, and what the server refuses around a drop or a rename: the names it
//! does not have, the keys others depend on, and the constraints a table
//! inherited.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{ConstraintDrop, Name, Table};
use pgorm::{ConnectionTrait, entity::prelude::*};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

/// The names of `table`'s constraints of the kinds `kinds` (`pg_constraint`'s
/// letters), sorted.
async fn constraints_of(
    db: &DatabaseConnection,
    table: &str,
    kinds: &str,
) -> Result<Vec<String>, Error> {
    let rows = db
        .query_all(
            "SELECT conname::text FROM pg_constraint \
             WHERE conrelid = $1::text::regclass AND strpos($2, contype::text) > 0 \
             ORDER BY conname",
            &[&table, &kinds],
        )
        .await?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// `parent (id, code, note, v)` keyed on `id`, with a named unique key, a
/// named `NOT NULL` and a named `CHECK`, and `child (parent_id)` referencing
/// it by a named foreign key.
const SCHEMA: &str = "CREATE TABLE parent (\
       id integer PRIMARY KEY, \
       code integer CONSTRAINT parent_code UNIQUE, \
       note text CONSTRAINT parent_note_present NOT NULL, \
       v integer CONSTRAINT parent_v_positive CHECK (v > 0)); \
     CREATE TABLE child (\
       parent_id integer CONSTRAINT child_parent REFERENCES parent (id)); \
     INSERT INTO parent VALUES (1, 1, 'a', 1)";

/// Each kind of constraint is dropped by its name through the one drop: the
/// row each refuses — a second code, a null note, a negative `v`, an orphan
/// child — is admitted once it is dropped, and the catalogue no longer holds
/// it.
// [spec:pgorm:req:sql.ddl.alter-table+14/test]    against a live server: a key, a NOT NULL,
// a CHECK and a foreign key are each dropped by name, and admit what they refused
#[pgorm_macros::test]
async fn one_drop_serves_every_constraint_kind() -> Result<(), Error> {
    let ctx = TestContext::new("constraint_drop_every_kind").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SCHEMA).await?;

    for (constraint, table, breaking, state) in [
        (
            "parent_code",
            "parent",
            "INSERT INTO parent VALUES (2, 1, 'b', 1)",
            SqlState::UNIQUE_VIOLATION,
        ),
        (
            "parent_note_present",
            "parent",
            "INSERT INTO parent VALUES (3, 3, NULL, 1)",
            SqlState::NOT_NULL_VIOLATION,
        ),
        (
            "parent_v_positive",
            "parent",
            "INSERT INTO parent VALUES (4, 4, 'd', -1)",
            SqlState::CHECK_VIOLATION,
        ),
        (
            "child_parent",
            "child",
            "INSERT INTO child VALUES (99)",
            SqlState::FOREIGN_KEY_VIOLATION,
        ),
    ] {
        let refused = db.execute(breaking, &[]).await.expect_err(breaking);
        refused_with(&refused, &state);

        let drop = Table::alter(n(table))
            .drop_constraint(n(constraint))
            .to_string();
        db.batch_execute(&drop).await?;
        db.execute(breaking, &[]).await?;
        assert!(
            !constraints_of(&db, table, "pnucf")
                .await?
                .contains(&constraint.to_owned()),
            "{drop}"
        );
    }
    assert_eq!(
        constraints_of(&db, "parent", "pnucf").await?,
        ["parent_id_not_null", "parent_pkey"]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// What the server refuses around a drop: a name the table has no constraint
/// under (`42704`), which `IF EXISTS` passes over; a key a foreign key
/// depends on (`2BP01`), said `RESTRICT` or not, which `CASCADE` drops with
/// the foreign key; the `NOT NULL` of a primary-key column (`42P16`); and a
/// constraint a table inherited, which only its parent drops (`42P16`).
// [spec:pgorm:req:sql.ddl.alter-table+14/test]    against a live server: a missing name,
// a depended-on key, a key column's NOT NULL and an inherited constraint are refused, and
// IF EXISTS and CASCADE do what they say
#[pgorm_macros::test]
async fn a_drop_is_refused_where_it_cannot_be() -> Result<(), Error> {
    let ctx = TestContext::new("constraint_drop_refusals").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SCHEMA).await?;
    let dropping = |table: &str, drop: ConstraintDrop| {
        Table::alter(n(table)).drop_constraint(drop).to_string()
    };

    let missing = dropping("parent", ConstraintDrop::new(n("missing")));
    let refused = db.batch_execute(&missing).await.expect_err(&missing);
    refused_with(&refused, &SqlState::UNDEFINED_OBJECT);
    db.batch_execute(&dropping(
        "parent",
        ConstraintDrop::new(n("missing")).if_exists(),
    ))
    .await?;

    for depended_on in [
        ConstraintDrop::new(n("parent_pkey")),
        ConstraintDrop::new(n("parent_pkey")).restrict(),
    ] {
        let depended_on = dropping("parent", depended_on);
        let refused = db
            .batch_execute(&depended_on)
            .await
            .expect_err(&depended_on);
        refused_with(&refused, &SqlState::DEPENDENT_OBJECTS_STILL_EXIST);
    }
    assert_eq!(constraints_of(&db, "child", "f").await?, ["child_parent"]);
    db.batch_execute(&dropping(
        "parent",
        ConstraintDrop::new(n("parent_pkey")).cascade(),
    ))
    .await?;
    assert_eq!(
        constraints_of(&db, "child", "f").await?,
        Vec::<String>::new()
    );
    assert_eq!(
        constraints_of(&db, "parent", "p").await?,
        Vec::<String>::new()
    );

    db.batch_execute("CREATE TABLE keyed (id integer PRIMARY KEY)")
        .await?;
    let key_not_null = dropping("keyed", ConstraintDrop::new(n("keyed_id_not_null")));
    let refused = db
        .batch_execute(&key_not_null)
        .await
        .expect_err(&key_not_null);
    refused_with(&refused, &SqlState::INVALID_TABLE_DEFINITION);

    db.batch_execute(
        "CREATE TABLE base (a integer CONSTRAINT base_a_positive CHECK (a > 0)); \
         CREATE TABLE heir () INHERITS (base)",
    )
    .await?;
    let inherited = dropping("heir", ConstraintDrop::new(n("base_a_positive")));
    let refused = db.batch_execute(&inherited).await.expect_err(&inherited);
    refused_with(&refused, &SqlState::INVALID_TABLE_DEFINITION);
    db.batch_execute(&dropping("base", ConstraintDrop::new(n("base_a_positive"))))
        .await?;
    assert_eq!(
        constraints_of(&db, "heir", "c").await?,
        Vec::<String>::new()
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A rename gives a constraint of any kind a new name: a primary key's index
/// takes the name with it, and a parent's rename reaches its child's copy.
/// The server refuses a name the table has no constraint under (`42704`), a
/// new name another constraint holds (`42710`) — or, renaming a key, a name
/// another key's index holds (`42P07`), since the index takes the name too —
/// and a rename of an inherited copy on the child (`42P16`).
// [spec:pgorm:req:sql.ddl.alter-table+14/test]    against a live server: a rename reaches
// a key's index and a child's inherited copy, and is refused for a missing or taken name
// and on the child
#[pgorm_macros::test]
async fn a_rename_reaches_index_and_heirs() -> Result<(), Error> {
    let ctx = TestContext::new("constraint_rename").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SCHEMA).await?;
    let rename = |table: &str, from: &str, to: &str| {
        Table::rename_constraint(n(table), n(from), n(to)).to_string()
    };

    db.batch_execute(&rename("parent", "parent_pkey", "parent_key"))
        .await?;
    db.batch_execute(&rename("parent", "parent_note_present", "parent_note_set"))
        .await?;
    assert_eq!(
        constraints_of(&db, "parent", "pnuc").await?,
        [
            "parent_code",
            "parent_id_not_null",
            "parent_key",
            "parent_note_set",
            "parent_v_positive"
        ]
    );
    let index = db
        .query_one(
            "SELECT count(*) FROM pg_class WHERE relkind = 'i' AND relname = 'parent_key'",
            &[],
        )
        .await?;
    assert_eq!(index.get::<_, i64>(0), 1);

    let missing = rename("parent", "missing", "anything");
    let refused = db.batch_execute(&missing).await.expect_err(&missing);
    refused_with(&refused, &SqlState::UNDEFINED_OBJECT);
    let taken = rename("parent", "parent_v_positive", "parent_note_set");
    let refused = db.batch_execute(&taken).await.expect_err(&taken);
    refused_with(&refused, &SqlState::DUPLICATE_OBJECT);
    let index_taken = rename("parent", "parent_code", "parent_key");
    let refused = db
        .batch_execute(&index_taken)
        .await
        .expect_err(&index_taken);
    refused_with(&refused, &SqlState::DUPLICATE_TABLE);

    db.batch_execute(
        "CREATE TABLE base (a integer CONSTRAINT base_a_positive CHECK (a > 0)); \
         CREATE TABLE heir () INHERITS (base)",
    )
    .await?;
    let on_heir = rename("heir", "base_a_positive", "heir_a_positive");
    let refused = db.batch_execute(&on_heir).await.expect_err(&on_heir);
    refused_with(&refused, &SqlState::INVALID_TABLE_DEFINITION);
    db.batch_execute(&rename("base", "base_a_positive", "base_a_above_zero"))
        .await?;
    assert_eq!(
        constraints_of(&db, "heir", "c").await?,
        ["base_a_above_zero"]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
