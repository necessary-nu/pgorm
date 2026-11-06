//! A related row that is present but undecodable must be reported, not
//! silently turned into "no related row".
//!
//! Every joined decode funnels through
//! [`FromQueryResult::from_query_result_optional`], so the tables here are
//! built with raw DDL: the point is a column whose PostgreSQL type the Rust
//! model cannot decode, which no entity-derived schema would produce.

#![allow(unused_imports, dead_code)]

pub mod common;

use futures::StreamExt;
use pgorm::tests_cfg::{cake, cake_filling, filling, fruit, vendor};
use pgorm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Error, FromQueryResult,
    QueryFilter, QueryResult, QuerySelect, RelationTrait, Value, Values, alias,
};
use pretty_assertions::assert_eq;

pub use common::TestContext;

/// The cake that has a related row, whose related row does not decode.
const BROKEN: i32 = 1;
/// The cake the outer join matches nothing for.
const LONELY: i32 = 2;

fn cake_broken() -> cake::Model {
    cake::Model {
        id: BROKEN,
        name: "Cheesecake".to_owned(),
    }
}

fn cake_lonely() -> cake::Model {
    cake::Model {
        id: LONELY,
        name: "Lonely".to_owned(),
    }
}

/// A decode failure the caller can act on, rather than a missing row.
#[track_caller]
fn assert_wrong_type<T: std::fmt::Debug>(result: Result<T, Error>) {
    let err = match result {
        Ok(ok) => panic!("a present but undecodable related row was reported as {ok:?}"),
        Err(err) => err,
    };
    let rendered = err.to_string();
    assert!(
        rendered.contains("WrongType"),
        "expected a decode failure naming the type mismatch, got: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// A related source graph: paired, grouped and streamed
// ---------------------------------------------------------------------------

async fn related_schema(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(
        r#"
        CREATE TABLE "cake" ("id" int PRIMARY KEY, "name" text NOT NULL);
        CREATE TABLE "fruit" ("id" int PRIMARY KEY, "name" int NOT NULL, "cake_id" int);
        INSERT INTO "cake" VALUES (1, 'Cheesecake'), (2, 'Lonely');
        INSERT INTO "fruit" VALUES (10, 123, 1);
        "#,
    )
    .await
}

// [spec:pgorm:req:exec.decode.absent/test]    a present related row whose
// columns do not decode is an error on every joined path, while an outer join
// that matched nothing stays `None`
#[pgorm_macros::test]
async fn related_decode_errors() -> Result<(), Error> {
    let ctx = TestContext::new("joined_decode_related").await;
    let db = ctx.db.get().await?;
    related_schema(&db).await?;

    // The related side on its own already reports the mismatch, and the join
    // must not be the weaker path.
    assert_wrong_type(fruit::Entity::find().all(&db).await);

    let joined = |cake_id: i32| {
        cake::Entity::graph()
            .related_maybe::<fruit::Entity>()
            .filter(cake::Column::Id.eq(cake_id))
    };

    // paired: present-but-undecodable
    assert_wrong_type(joined(BROKEN).all(&db).await);
    assert_wrong_type(joined(BROKEN).one_opt(&db).await);

    // paired: genuinely absent
    assert_eq!(joined(LONELY).all(&db).await?, [(cake_lonely(), None)]);

    // the grouped read, through the regroup path
    assert_wrong_type(joined(BROKEN).all_grouped(&db).await);
    assert_eq!(
        joined(LONELY).all_grouped(&db).await?,
        [(cake_lonely(), vec![])]
    );

    // The stream decodes row by row through the same selector.
    let mut stream = joined(BROKEN).stream(&db).await?;
    assert_wrong_type(stream.next().await.expect("the join produced one row"));
    drop(stream);

    let mut stream = joined(LONELY).stream(&db).await?;
    assert_eq!(
        stream.next().await.expect("the join produced one row")?,
        (cake_lonely(), None)
    );
    drop(stream);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Custom projections over a joined pair of prefixes
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, FromQueryResult)]
struct CakeIdName {
    id: i32,
    name: String,
}

#[derive(Debug, PartialEq, FromQueryResult)]
struct FruitIdName {
    id: i32,
    name: String,
}

/// A caller-authored pair over the writer's prefix scheme: the graph generates
/// its own projection, so a decode that reads two prefixes out of a
/// caller-authored select list is written directly against the row.
#[derive(Debug, PartialEq)]
struct CakeAndFruit(CakeIdName, Option<FruitIdName>);

impl FromQueryResult for CakeAndFruit {
    fn from_query_result(res: &QueryResult, pre: &str) -> Result<Self, Error> {
        Ok(CakeAndFruit(
            CakeIdName::from_query_result(res, &format!("{pre}s0_"))?,
            FruitIdName::from_query_result_optional(res, &format!("{pre}s1_"))?,
        ))
    }
}

// [spec:pgorm:req:exec.decode.absent/test]    a custom projection over a joined
// pair follows the same rule, and a column the projection never produced is
// reported rather than read as an absent row
#[pgorm_macros::test]
async fn projected_decode_errors() -> Result<(), Error> {
    let ctx = TestContext::new("joined_decode_projected").await;
    let db = ctx.db.get().await?;
    related_schema(&db).await?;

    let projected = |cake_id: i32| {
        CakeAndFruit::find_by_statement(
            r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name",
                      "fruit"."id" AS "s1_id", "fruit"."name" AS "s1_name"
               FROM "cake" LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id"
               WHERE "cake"."id" = $1"#,
            Values(vec![Value::Int(Some(cake_id))]),
        )
    };

    assert_wrong_type(projected(BROKEN).all(&db).await);
    assert_eq!(
        projected(LONELY).all(&db).await?,
        [CakeAndFruit(
            CakeIdName {
                id: LONELY,
                name: "Lonely".to_owned(),
            },
            None
        )]
    );

    // `s1_name` is never projected, so `FruitIdName` names a column the result
    // set does not carry. That is a projection mistake, not an absent row, on
    // the matched row and the unmatched one alike.
    let under_projected = |cake_id: i32| {
        CakeAndFruit::find_by_statement(
            r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name",
                      "fruit"."id" AS "s1_id"
               FROM "cake" LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id"
               WHERE "cake"."id" = $1"#,
            Values(vec![Value::Int(Some(cake_id))]),
        )
    };

    let matched = under_projected(BROKEN)
        .all(&db)
        .await
        .expect_err("a column the projection omits must be reported");
    assert!(
        matched.to_string().contains("s1_name"),
        "expected the missing column to be named, got: {matched}"
    );

    // The unmatched row cannot be called absent either: `s1_id` is `NULL` but
    // `s1_name` is not a column of the result set at all, so the witness is
    // incomplete and the decode failure stands.
    under_projected(LONELY)
        .all(&db)
        .await
        .expect_err("an incomplete witness must not be read as an absent row");

    drop(db);
    ctx.delete().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// A multi-hop chain as `via` hops plus one decoded slot
// ---------------------------------------------------------------------------

// [spec:pgorm:req:exec.decode.absent/test]    the multi-hop chain reads its far
// side through the same optional decode
#[pgorm_macros::test]
async fn linked_decode_errors() -> Result<(), Error> {
    let ctx = TestContext::new("joined_decode_linked").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        r#"
        CREATE TABLE "cake" ("id" int PRIMARY KEY, "name" text NOT NULL);
        CREATE TABLE "filling" ("id" int PRIMARY KEY, "name" text NOT NULL, "vendor_id" int);
        CREATE TABLE "vendor" ("id" int PRIMARY KEY, "name" int NOT NULL);
        CREATE TABLE "cake_filling" (
            "cake_id" int, "filling_id" int, PRIMARY KEY ("cake_id", "filling_id")
        );
        INSERT INTO "cake" VALUES (1, 'Cheesecake'), (2, 'Lonely');
        INSERT INTO "vendor" VALUES (7, 123);
        INSERT INTO "filling" VALUES (5, 'Cream', 7);
        INSERT INTO "cake_filling" VALUES (1, 5);
        "#,
    )
    .await?;

    // The chain `CakeToFillingVendor` describes, as a graph: two hops joined
    // but never decoded, and the vendor as the one slot.
    let chained = |cake_id: i32| {
        cake::Entity::graph()
            .via(cake_filling::Relation::Cake.def().rev())
            .via(cake_filling::Relation::Filling.def())
            .join_maybe::<vendor::Entity>(filling::Relation::Vendor.def())
            .filter(cake::Column::Id.eq(cake_id))
    };

    assert_wrong_type(chained(BROKEN).all(&db).await);
    assert_eq!(chained(LONELY).all(&db).await?, [(cake_lonely(), None)]);

    assert_wrong_type(chained(BROKEN).all_grouped(&db).await);
    assert_eq!(
        chained(LONELY).all_grouped(&db).await?,
        [(cake_lonely(), vec![])]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Payload decodes: an enum label and a JSON body the model cannot read
// ---------------------------------------------------------------------------

mod tea {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
    #[pgorm(enum_name = "tea")]
    pub enum Tea {
        #[pgorm(string_value = "EverydayTea")]
        EverydayTea,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "brew")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub cake_id: Option<i32>,
        pub tea: Tea,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = pgorm::tests_cfg::cake::Entity,
            from = Column::CakeId,
            to = pgorm::tests_cfg::cake::Column::Id
        )]
        Cake,
    }
}

impl pgorm::Related<tea::Entity> for cake::Entity {
    fn to() -> pgorm::RelationDef {
        use pgorm::RelationTrait;

        tea::Relation::Cake.def().rev()
    }
}

// [spec:pgorm:req:exec.decode.absent/test]    an unrecognised enum label in a
// present related row is reported, not read as an absent row
#[pgorm_macros::test]
async fn enum_payload_decode_errors() -> Result<(), Error> {
    use pgorm::RelationTrait;

    let ctx = TestContext::new("joined_decode_enum").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        r#"
        CREATE TABLE "cake" ("id" int PRIMARY KEY, "name" text NOT NULL);
        CREATE TYPE "tea" AS ENUM ('EverydayTea', 'BreakfastTea');
        CREATE TABLE "brew" (
            "id" int PRIMARY KEY, "cake_id" int, "tea" tea NOT NULL
        );
        INSERT INTO "cake" VALUES (1, 'Cheesecake'), (2, 'Lonely');
        INSERT INTO "brew" VALUES (10, 1, 'BreakfastTea');
        "#,
    )
    .await?;

    // `BreakfastTea` is a label the Rust enum does not carry, so the related
    // row is present and undecodable.
    let err = cake::Entity::graph()
        .related_maybe::<tea::Entity>()
        .filter(cake::Column::Id.eq(BROKEN))
        .all(&db)
        .await
        .expect_err("an unknown enum label must be reported");
    assert!(
        err.to_string().contains("BreakfastTea"),
        "expected the unknown label to be named, got: {err}"
    );

    assert_eq!(
        cake::Entity::graph()
            .related_maybe::<tea::Entity>()
            .filter(cake::Column::Id.eq(LONELY))
            .all(&db)
            .await?,
        [(cake_lonely(), None)]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// The witness is judged before decoding succeeds
// ---------------------------------------------------------------------------

/// Nothing but nullable fields: an all-`NULL` row decodes successfully, so
/// only the witness can tell it apart from an absent one.
#[derive(Debug, PartialEq, FromQueryResult)]
struct NullableName {
    name: Option<String>,
}

/// A hand-written mirror that reports no columns, so it is judged against the
/// result set under its prefix instead.
#[derive(Debug, PartialEq)]
struct UnreflectedName {
    name: Option<String>,
}

impl FromQueryResult for UnreflectedName {
    fn from_query_result(res: &QueryResult, pre: &str) -> Result<Self, Error> {
        Ok(Self {
            name: res.try_get(pre, "name")?,
        })
    }
}

#[derive(Debug, PartialEq)]
struct MaybeNullable(Option<NullableName>, Option<UnreflectedName>);

impl FromQueryResult for MaybeNullable {
    fn from_query_result(res: &QueryResult, pre: &str) -> Result<Self, Error> {
        Ok(MaybeNullable(
            NullableName::from_query_result_optional(res, &format!("{pre}s0_"))?,
            UnreflectedName::from_query_result_optional(res, &format!("{pre}s1_"))?,
        ))
    }
}

// [spec:pgorm:req:exec.decode.absent/test]    an all-`NULL` row is absent even
// when every field is nullable and the decode would have succeeded, through
// both the reported-columns witness and the prefix fallback
#[pgorm_macros::test]
async fn all_null_nullable_projection_is_absent() -> Result<(), Error> {
    let ctx = TestContext::new("joined_decode_nullable_witness").await;
    let db = ctx.db.get().await?;

    let row = |name_a: &str, name_b: &str| {
        MaybeNullable::find_by_statement(
            &format!(r#"SELECT {name_a}::text AS "s0_name", {name_b}::text AS "s1_name""#),
            Values(vec![]),
        )
    };

    // All witness columns NULL: absent on both paths, not Some(None-fields).
    assert_eq!(
        row("NULL", "NULL").one(&db).await?,
        MaybeNullable(None, None)
    );

    // A present value decodes as usual on both paths.
    assert_eq!(
        row("'a'", "'b'").one(&db).await?,
        MaybeNullable(
            Some(NullableName {
                name: Some("a".to_owned())
            }),
            Some(UnreflectedName {
                name: Some("b".to_owned())
            })
        )
    );

    // Present on one side, absent on the other: the witnesses are independent.
    assert_eq!(
        row("'a'", "NULL").one(&db).await?,
        MaybeNullable(
            Some(NullableName {
                name: Some("a".to_owned())
            }),
            None
        )
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:exec.decode.absent/test]    a present-but-invalid column
// still errors: judging the witness first must not swallow decode failures
#[pgorm_macros::test]
async fn present_invalid_column_still_errors() -> Result<(), Error> {
    let ctx = TestContext::new("joined_decode_nullable_invalid").await;
    let db = ctx.db.get().await?;

    let result = MaybeNullable::find_by_statement(
        r#"SELECT 123::int AS "s0_name", NULL::text AS "s1_name""#,
        Values(vec![]),
    )
    .one(&db)
    .await;
    assert_wrong_type(result);

    drop(db);
    ctx.delete().await;
    Ok(())
}
