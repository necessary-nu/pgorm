#![allow(unused_imports, dead_code)]

//! `CREATE DOMAIN`, `ALTER DOMAIN` and `DROP DOMAIN` against a live
//! PostgreSQL server, and an entity whose columns are domains.
//!
//! The render tests in pgorm-query hold each statement to libpg_query's
//! `CreateDomainStmt`, `AlterDomainStmt`, `RenameStmt` and `DropStmt`. What
//! only a server settles is what they do: that a domain's default, `NOT NULL`
//! and checks hold every column typed with it, that each alter action changes
//! the domain as it says, that a `NOT VALID` check leaves the values already
//! there until it is validated, that a drop is refused while a column has the
//! type, and what the server refuses that no builder value records. An entity
//! whose columns are domains — over an integer, over an enum, and an array of
//! one — is created from its own definition, written, read and filtered, and a
//! check's refusal reaches the caller as the error PostgreSQL raised.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    ColumnType, ConstraintDrop, Domain, DomainCheck, Expr, Name, Query, Schema, alias,
    extension::Type,
};
use pgorm::{ConnectionTrait, QueryTrait, TransactionTrait, entity::prelude::*};
use pgorm_test_support::PREPARED_STATEMENT;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tokio_postgres::error::SqlState;

/// A mood: the enum type `cheerful` is a domain over.
mod mood {
    use pgorm::entity::prelude::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, ActiveEnum)]
    #[pgorm(rs_type = String, enum_name = "mood")]
    pub enum Mood {
        #[pgorm(string_value = "sad")]
        Sad,
        #[pgorm(string_value = "calm")]
        Calm,
        #[pgorm(string_value = "glad")]
        Glad,
    }
}

use mood::Mood;

/// `line`, each of whose columns but its key is typed with a domain.
mod line {
    use super::Mood;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "line")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(
            column_type = ColumnType::Domain { name: Name::runtime("quantity"), schema: Some(Name::runtime("billing")), base: Arc::new(ColumnType::Integer) }
        )]
        pub qty: i32,
        #[pgorm(
            column_type = ColumnType::Domain { name: Name::runtime("cheerful"), schema: None, base: Arc::new(Mood::db_type().get_column_type().clone()) }
        )]
        pub mood: Mood,
        #[pgorm(
            column_type = ColumnType::Array(Arc::new(ColumnType::Domain { name: Name::runtime("code"), schema: None, base: Arc::new(ColumnType::Text) }))
        )]
        pub codes: Vec<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// `temper`, keyed by a column of the domain `line.mood` is typed with: the
/// relation between the two joins a domain over an enum on either side.
mod temper {
    use super::Mood;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "temper")]
    pub struct Model {
        #[pgorm(
            primary_key,
            auto_increment = false,
            column_type = ColumnType::Domain { name: Name::runtime("cheerful"), schema: None, base: Arc::new(Mood::db_type().get_column_type().clone()) }
        )]
        pub mood: Mood,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// `tariff`, whose fees are `money` under the domain `fee`: one, and an
/// array of them.
mod tariff {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "tariff")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(
            column_type = ColumnType::Domain { name: Name::runtime("fee"), schema: None, base: Arc::new(ColumnType::Money) }
        )]
        pub fee: Decimal,
        #[pgorm(
            column_type = ColumnType::Array(Arc::new(ColumnType::Domain { name: Name::runtime("fee"), schema: None, base: Arc::new(ColumnType::Money) }))
        )]
        pub fees: Vec<Decimal>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Neither entity declares the relation in its `Relation` enum: PostgreSQL
/// cannot implement a foreign key between two columns of a domain over an enum
/// (`42804`), so `create_table_from_entity` would refuse the relation, and it
/// is a join the two entities describe instead.
impl Related<temper::Entity> for line::Entity {
    fn to() -> RelationDef {
        line::Entity::belongs_to(temper::Entity)
            .columns(line::Column::Mood, temper::Column::Mood)
            .into()
    }
}

impl Related<line::Entity> for temper::Entity {
    fn to() -> RelationDef {
        temper::Entity::has_many(line::Entity).into()
    }
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

#[track_caller]
fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// A check's refusal: `23514`, naming the constraint and the domain.
#[track_caller]
fn checked_by(error: &Error, constraint: &str, domain: &str) {
    refused_with(error, &SqlState::CHECK_VIOLATION);
    let Error::Postgres(e) = error else {
        unreachable!("refused_with matched Error::Postgres");
    };
    let db = e.as_db_error().expect("a server error");
    assert_eq!(db.constraint(), Some(constraint), "{e}");
    assert_eq!(db.datatype(), Some(domain), "{e}");
}

/// A statement the server refuses, and the error it refused it with.
async fn refused(db: &impl ConnectionTrait, sql: &str) -> Error {
    db.batch_execute(sql)
        .await
        .expect_err(&format!("the server refuses {sql}"))
}

/// `billing.quantity`, `code` and `cheerful`, each created by the builder
/// over the type the entity's field holds.
async fn domains(db: &impl ConnectionTrait) -> Result<(), Error> {
    db.batch_execute(&Schema::create(n("billing")).to_string())
        .await?;
    let generator = pgorm::Schema::new();
    db.batch_execute(
        &generator
            .create_enum_from_active_enum::<Mood>()?
            .to_string(),
    )
    .await?;
    let quantity = Domain::create((n("billing"), n("quantity")), ColumnType::Integer)
        .default(1)
        .not_null()
        .check(DomainCheck::new(Expr::domain_value().gt(0)).name(n("positive")))
        .to_string();
    assert_eq!(
        quantity,
        [
            r#"CREATE DOMAIN "billing"."quantity" AS integer DEFAULT 1 NOT NULL"#,
            r#"CONSTRAINT "positive" CHECK (VALUE > 0)"#,
        ]
        .join(" ")
    );
    db.batch_execute(&quantity).await?;
    db.batch_execute(
        &Domain::create(n("code"), ColumnType::Text)
            .check(Expr::domain_value().ne(""))
            .to_string(),
    )
    .await?;
    let cheerful = Domain::create(n("cheerful"), Mood::db_type().get_column_type().clone())
        .check(Expr::domain_value().ne("sad"))
        .to_string();
    assert_eq!(
        cheerful,
        r#"CREATE DOMAIN "cheerful" AS mood CHECK (VALUE <> 'sad')"#
    );
    db.batch_execute(&cheerful).await
}

/// Each column of `table` with its type as the server prints it.
async fn columns_of(
    db: &impl ConnectionTrait,
    table: &str,
) -> Result<Vec<(String, String)>, Error> {
    let rows = db
        .query_all(
            "SELECT attname::text, format_type(atttypid, atttypmod) FROM pg_attribute \
             WHERE attrelid = $1::text::regclass AND attnum > 0 AND NOT attisdropped ORDER BY attnum",
            &[&table],
        )
        .await?;
    Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
}

/// A domain's constraints: each one's name, kind and whether it is validated.
async fn constraints_of(
    db: &impl ConnectionTrait,
    domain: &str,
) -> Result<Vec<(String, String, bool)>, Error> {
    let rows = db
        .query_all(
            "SELECT conname::text, contype::text, convalidated FROM pg_constraint \
             WHERE contypid = $1::text::regtype ORDER BY conname COLLATE \"C\"",
            &[&domain],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

/// One `t` row's `v`, by id.
async fn value_of(db: &impl ConnectionTrait, id: i32) -> Result<Option<i32>, Error> {
    let row = db
        .query_one("SELECT v FROM t WHERE id = $1", &[&id])
        .await?;
    Ok(row.get(0))
}

fn written(id: i32, qty: i32, mood: Mood, codes: &[&str]) -> line::Model {
    line::Model {
        id,
        qty,
        mood,
        codes: codes.iter().map(|code| (*code).to_owned()).collect(),
    }
}

/// The table an entity of domain columns creates names each domain, and the
/// entity writes, reads and filters through them — the domain over an enum
/// compared as its enum, which is the only way PostgreSQL compares one —
/// while each domain's check refuses what it does not hold.
// [spec:pgorm:req:sql.ddl.domain+3/test]    an entity whose columns are domains over an
// integer, over an enum and an array of one creates its table with them, reads, writes and
// filters through them, a predicate on the enum's domain comparing as the enum, and a check's
// refusal arrives as 23514 naming the constraint
// [spec:pgorm:def:sql.types.column-type+11/test]
// [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]
// [spec:pgorm:def:exec.decode.array+2/test]
#[pgorm_macros::test]
async fn an_entity_reads_and_writes_through_domains() -> Result<(), Error> {
    let ctx = TestContext::new("domain_entity_round_trip").await;
    let db = ctx.db.get().await?;
    domains(&db).await?;
    let create = pgorm::Schema::new()
        .create_table_from_entity(line::Entity)?
        .to_string();
    assert_eq!(
        create,
        [
            r#"CREATE TABLE "line" ( "id" integer NOT NULL, "qty" billing.quantity NOT NULL,"#,
            r#""mood" cheerful NOT NULL, "codes" code[] NOT NULL, PRIMARY KEY ("id") )"#,
        ]
        .join(" ")
    );
    db.batch_execute(&create).await?;
    assert_eq!(
        columns_of(&db, "line").await?,
        [
            ("id".to_owned(), "integer".to_owned()),
            ("qty".to_owned(), "billing.quantity".to_owned()),
            ("mood".to_owned(), "cheerful".to_owned()),
            ("codes".to_owned(), "code[]".to_owned()),
        ]
    );

    let first = written(1, 5, Mood::Glad, &["A", "B"]);
    first.clone().into_active().insert(&db).await?;
    written(2, 9, Mood::Calm, &[])
        .into_active()
        .insert(&db)
        .await?;
    assert_eq!(line::Entity::find_by_id(1).one(&db).await?, first.clone());

    let large = line::Entity::find()
        .filter(line::Column::Qty.gt(6))
        .all(&db)
        .await?;
    assert_eq!(large.iter().map(|line| line.id).collect::<Vec<_>>(), [2]);

    // PostgreSQL resolves no enum operator for a domain over an enum, even
    // between two values of the domain, so the predicate compares the column
    // and the value as the enum.
    let glad = line::Entity::find()
        .filter(line::Column::Mood.eq(Mood::Glad))
        .all(&db)
        .await?;
    assert_eq!(glad, std::slice::from_ref(&first));

    let zero = written(3, 0, Mood::Glad, &[])
        .into_active()
        .insert(&db)
        .await
        .expect_err("a quantity of zero");
    checked_by(&zero, "positive", "quantity");
    let sad = written(3, 1, Mood::Sad, &[])
        .into_active()
        .insert(&db)
        .await
        .expect_err("a sad mood");
    checked_by(&sad, "cheerful_check", "cheerful");
    let empty = written(3, 1, Mood::Calm, &["A", ""])
        .into_active()
        .insert(&db)
        .await
        .expect_err("an empty code");
    checked_by(&empty, "code_check", "code");
    let mut lowered = first.clone().into_active();
    lowered.qty = set(-1);
    let lowered = lowered.update(&db).await.expect_err("a negative quantity");
    checked_by(&lowered, "positive", "quantity");
    assert_eq!(line::Entity::find_by_id(1).one(&db).await?, first);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A domain over `money` reads and writes as `numeric`, a `Decimal` field's
/// value cast to the domain so that its check holds, and compares through the
/// same cast; an array of the domain does the same element by element.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]    a domain over `money`, and an
// array of it, is read as `numeric` and written through `numeric` to the domain, by name
#[pgorm_macros::test]
async fn a_money_domain_reads_and_writes_as_decimal() -> Result<(), Error> {
    let ctx = TestContext::new("domain_money_round_trip").await;
    let db = ctx.db.get().await?;
    db.batch_execute("CREATE DOMAIN fee AS money CHECK (VALUE >= '0'::money)")
        .await?;
    db.batch_execute(
        &pgorm::Schema::new()
            .create_table_from_entity(tariff::Entity)?
            .to_string(),
    )
    .await?;

    let model = tariff::Model {
        id: 1,
        fee: Decimal::new(1234, 2),
        fees: vec![Decimal::new(15, 1), Decimal::TWO],
    };
    model.clone().into_active().insert(&db).await?;
    assert_eq!(tariff::Entity::find_by_id(1).one(&db).await?, model);

    let (sql, _) = tariff::Entity::find()
        .filter(tariff::Column::Fee.eq(Decimal::ONE))
        .filter(tariff::Column::Fee.eq_any([Decimal::ONE]))
        .filter(tariff::Column::Fees.eq(vec![Decimal::ONE]))
        .build();
    assert_eq!(
        sql,
        [
            r#"SELECT "tariff"."id", CAST("tariff"."fee" AS numeric), CAST("tariff"."fees" AS numeric[])"#,
            r#"FROM "tariff" WHERE "tariff"."fee" = CAST($1::numeric AS fee)"#,
            r#"AND "tariff"."fee" = ANY(CAST($2::numeric[] AS fee[]))"#,
            r#"AND "tariff"."fees" = CAST($3::numeric[] AS fee[])"#,
        ]
        .join(" ")
    );
    for filter in [
        tariff::Column::Fee.eq(Decimal::new(1234, 2)),
        tariff::Column::Fee.eq_any([Decimal::ONE, Decimal::new(1234, 2)]),
        tariff::Column::Fees.eq(vec![Decimal::new(15, 1), Decimal::TWO]),
    ] {
        let found = tariff::Entity::find().filter(filter).all(&db).await?;
        assert_eq!(found, std::slice::from_ref(&model));
    }

    let negative = tariff::Model {
        id: 2,
        fee: Decimal::NEGATIVE_ONE,
        fees: Vec::new(),
    }
    .into_active()
    .insert(&db)
    .await
    .expect_err("a negative fee");
    checked_by(&negative, "fee_check", "fee");

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The ids of the lines `filter` selects, in id order.
async fn ids_where(
    db: &impl ConnectionTrait,
    filter: impl pgorm::pgorm_query::IntoCondition,
) -> Result<Vec<i32>, Error> {
    Ok(line::Entity::find()
        .filter(filter)
        .order_by_asc(line::Column::Id)
        .all(db)
        .await?
        .iter()
        .map(|line| line.id)
        .collect())
}

/// Every value predicate on a column whose domain is over an enum compares
/// it as the enum, the column and the value each cast to it: PostgreSQL has
/// no operator for the domain itself. The value is compared, not written, so
/// the domain's check does not refuse it, and a keyset cursor over the column
/// pages by the enum's order in either direction.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]    every value predicate, the
// column-taking and expression-taking forms and if_null cast a column whose domain is over an
// enum, and its values, to the enum, live
// [spec:pgorm:sem:exec.cursor.keyset+8/test]    a keyset cursor over a column whose domain is
// over an enum pages forwards and backwards by the enum's order
#[pgorm_macros::test]
async fn domain_enum_predicates_compare_as_the_enum() -> Result<(), Error> {
    use line::Column::{Id, Mood as Moods};

    let ctx = TestContext::new("domain_enum_predicates").await;
    let db = ctx.db.get().await?;
    domains(&db).await?;
    db.batch_execute(
        &pgorm::Schema::new()
            .create_table_from_entity(line::Entity)?
            .to_string(),
    )
    .await?;
    for (id, mood) in [
        (1, Mood::Calm),
        (2, Mood::Glad),
        (3, Mood::Calm),
        (4, Mood::Glad),
        (5, Mood::Calm),
    ] {
        written(id, 1, mood, &[]).into_active().insert(&db).await?;
    }

    assert_eq!(
        line::Entity::find()
            .select_only()
            .column(Id)
            .filter(Moods.eq(Mood::Glad))
            .as_query()
            .to_string(),
        r#"SELECT "line"."id" FROM "line" WHERE CAST("line"."mood" AS mood) = CAST('glad' AS mood)"#
    );
    let (calm, glad) = (vec![1, 3, 5], vec![2, 4]);
    assert_eq!(ids_where(&db, Moods.eq(Mood::Glad)).await?, glad);
    assert_eq!(ids_where(&db, Moods.ne(Mood::Glad)).await?, calm);
    assert_eq!(ids_where(&db, Moods.gt(Mood::Calm)).await?, glad);
    assert_eq!(
        ids_where(&db, Moods.gte(Mood::Calm)).await?,
        [1, 2, 3, 4, 5]
    );
    assert_eq!(ids_where(&db, Moods.lt(Mood::Glad)).await?, calm);
    assert_eq!(ids_where(&db, Moods.lte(Mood::Calm)).await?, calm);
    assert_eq!(ids_where(&db, Moods.is_in([Mood::Glad])).await?, glad);
    assert_eq!(ids_where(&db, Moods.is_not_in([Mood::Glad])).await?, calm);
    assert_eq!(ids_where(&db, Moods.eq_any([Mood::Calm])).await?, calm);
    assert_eq!(ids_where(&db, Moods.ne_all([Mood::Calm])).await?, glad);
    assert_eq!(
        ids_where(&db, Moods.between(Mood::Sad, Mood::Calm)).await?,
        calm
    );
    assert_eq!(
        ids_where(&db, Moods.not_between(Mood::Sad, Mood::Calm)).await?,
        glad
    );
    assert_eq!(ids_where(&db, Moods.eq_col(Moods)).await?, [1, 2, 3, 4, 5]);
    let glad_enum = Expr::val("glad").cast_as(n("mood"));
    assert_eq!(
        ids_where(&db, Moods.eq_expr(glad_enum.clone())).await?,
        glad
    );
    assert_eq!(
        ids_where(&db, Expr::expr(Moods.if_null(Mood::Calm)).eq(glad_enum)).await?,
        glad
    );
    // The domain's check refuses 'sad' as a value written to it; compared
    // with the column, 'sad' is an enum value that no row holds.
    assert_eq!(
        ids_where(&db, Moods.eq(Mood::Sad)).await?,
        Vec::<i32>::new()
    );

    let mut cursor = line::Entity::find().cursor_by((Moods, Id));
    let page = |models: Vec<line::Model>| models.iter().map(|line| line.id).collect::<Vec<_>>();
    assert_eq!(page(cursor.first(2).all(&db).await?), [1, 3]);
    assert_eq!(
        page(cursor.after((Mood::Calm, 3)).first(2).all(&db).await?),
        [5, 2]
    );
    assert_eq!(
        page(cursor.after((Mood::Glad, 2)).first(2).all(&db).await?),
        [4]
    );
    let mut backwards = line::Entity::find().cursor_by((Moods, Id));
    assert_eq!(
        page(backwards.before((Mood::Glad, 2)).last(2).all(&db).await?),
        [3, 5]
    );
    let mut descending = line::Entity::find().cursor_by(Moods);
    assert_eq!(
        page(descending.desc().after(Mood::Glad).all(&db).await?).len(),
        3
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Lines 1 to 5, calm, glad, calm, glad, calm, beside a calm and a glad
/// temper, each table created from its entity over the same domain.
async fn lines_and_tempers(db: &impl ConnectionTrait) -> Result<(), Error> {
    domains(db).await?;
    let schema = pgorm::Schema::new();
    db.batch_execute(&schema.create_table_from_entity(line::Entity)?.to_string())
        .await?;
    db.batch_execute(&schema.create_table_from_entity(temper::Entity)?.to_string())
        .await?;
    for (id, mood) in [
        (1, Mood::Calm),
        (2, Mood::Glad),
        (3, Mood::Calm),
        (4, Mood::Glad),
        (5, Mood::Calm),
    ] {
        written(id, 1, mood, &[]).into_active().insert(db).await?;
    }
    for (mood, label) in [(Mood::Calm, "steady"), (Mood::Glad, "bright")] {
        temper::Model {
            mood,
            label: label.to_owned(),
        }
        .into_active()
        .insert(db)
        .await?;
    }
    Ok(())
}

/// Subquery membership compares a column whose domain is over an enum as the
/// enum, with the subquery's projection cast beside it — even a projection of
/// the same domain, which the enum does not compare with either — and max and
/// min aggregate the column as the enum, the domain having neither.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]    in_subquery and
// not_in_subquery against a subquery projecting the same domain, and max and min of the
// column, live
#[pgorm_macros::test]
async fn domain_enum_subqueries_and_extrema_compare() -> Result<(), Error> {
    use line::Column::{Id, Mood as Moods};

    let ctx = TestContext::new("domain_enum_subqueries").await;
    let db = ctx.db.get().await?;
    lines_and_tempers(&db).await?;

    let bright = || {
        Query::select()
            .column(temper::Column::Mood)
            .from(temper::Entity)
            .and_where(temper::Column::Label.eq("bright"))
            .to_owned()
    };
    assert_eq!(
        line::Entity::find()
            .select_only()
            .column(Id)
            .filter(Moods.in_subquery(bright()))
            .as_query()
            .to_string(),
        [
            r#"SELECT "line"."id" FROM "line" WHERE CAST("line"."mood" AS mood)"#,
            r#"IN (SELECT CAST("mood" AS mood) FROM "temper" WHERE "temper"."label" = 'bright')"#,
        ]
        .join(" ")
    );
    assert_eq!(ids_where(&db, Moods.in_subquery(bright())).await?, [2, 4]);
    assert_eq!(
        ids_where(&db, Moods.not_in_subquery(bright())).await?,
        [1, 3, 5]
    );
    let own = Query::select()
        .column(Moods)
        .from(line::Entity)
        .and_where(Id.eq(3))
        .to_owned();
    assert_eq!(ids_where(&db, Moods.in_subquery(own)).await?, [1, 3, 5]);

    let extreme = |aggregate: pgorm::pgorm_query::SimpleExpr| {
        line::Entity::find()
            .select_only()
            .expr_as(Expr::expr(aggregate).cast_as(n("text")), alias("extreme"))
    };
    assert_eq!(
        extreme(Moods.max()).as_query().to_string(),
        r#"SELECT CAST(MAX(CAST("line"."mood" AS mood)) AS text) AS "extreme" FROM "line""#
    );
    assert_eq!(
        extreme(Moods.max()).into_tuple::<String>().one(&db).await?,
        "glad"
    );
    assert_eq!(
        extreme(Moods.min()).into_tuple::<String>().one(&db).await?,
        "calm"
    );
    assert_eq!(
        line::Entity::find()
            .select_only()
            .expr(Moods.count())
            .into_tuple::<i64>()
            .one(&db)
            .await?,
        5
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Subquery membership on a column whose domain is over an enum compares as
/// the enum whatever the subquery projects: a `*`, a table's `*` and a window
/// function's projection, which no cast reaches in place, are compared
/// through the subquery's array, with `NOT IN`'s answer to a null member. A
/// `*` wider than one column, and several projected expressions, are refused
/// by the server as they are under any `IN`, never narrowed to one column.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]    in_subquery and not_in_subquery
// against a `*`, a table's `*` and a windowed projection of the domain compare as the enum,
// live, while a wider `*` and several projections are refused with 42601
#[pgorm_macros::test]
async fn domain_enum_subqueries_of_every_shape_compare() -> Result<(), Error> {
    use line::Column::{Id, Mood as Moods};
    use pgorm::pgorm_query::{Asterisk, Func, Order, OrderedStatement, WindowStatement};

    let ctx = TestContext::new("domain_enum_subquery_shapes").await;
    let db = ctx.db.get().await?;
    lines_and_tempers(&db).await?;

    let bright = || {
        Query::select()
            .column(temper::Column::Mood)
            .from(temper::Entity)
            .and_where(temper::Column::Label.eq("bright"))
            .to_owned()
    };
    let starred = || {
        Query::select()
            .column(Asterisk)
            .from_subquery(bright(), alias("b"))
            .to_owned()
    };
    assert_eq!(
        line::Entity::find()
            .select_only()
            .column(Id)
            .filter(Moods.in_subquery(starred()))
            .as_query()
            .to_string(),
        [
            r#"SELECT "line"."id" FROM "line" WHERE CAST("line"."mood" AS mood) = ANY(CAST(ARRAY("#,
            r#"SELECT * FROM (SELECT "mood" FROM "temper" WHERE "temper"."label" = 'bright') AS "b""#,
            r#") AS mood[]))"#,
        ]
        .concat()
    );
    assert_eq!(ids_where(&db, Moods.in_subquery(starred())).await?, [2, 4]);
    assert_eq!(
        ids_where(&db, Moods.not_in_subquery(starred())).await?,
        [1, 3, 5]
    );
    let table_starred = Query::select()
        .column((alias("b"), Asterisk))
        .from_subquery(bright(), alias("b"))
        .to_owned();
    assert_eq!(
        ids_where(&db, Moods.in_subquery(table_starred)).await?,
        [2, 4]
    );

    // Ordered by label, the previous temper's mood is null for "bright" and
    // glad for "steady": a member is glad, and a null member leaves no row
    // NOT IN the set, as it leaves none under any NOT IN.
    let previous = || {
        Query::select()
            .expr_window(
                Func::named(n("lag")).arg(Expr::col(temper::Column::Mood)),
                WindowStatement::new()
                    .order_by(temper::Column::Label, Order::Asc)
                    .take(),
            )
            .from(temper::Entity)
            .to_owned()
    };
    assert_eq!(ids_where(&db, Moods.in_subquery(previous())).await?, [2, 4]);
    assert_eq!(
        ids_where(&db, Moods.not_in_subquery(previous())).await?,
        Vec::<i32>::new()
    );

    let wide = Query::select()
        .column(Asterisk)
        .from(temper::Entity)
        .to_owned();
    let error = ids_where(&db, Moods.in_subquery(wide))
        .await
        .expect_err("a two-column * is refused");
    refused_with(&error, &SqlState::SYNTAX_ERROR);
    let several = Query::select()
        .columns([temper::Column::Mood, temper::Column::Label])
        .from(temper::Entity)
        .to_owned();
    let error = ids_where(&db, Moods.in_subquery(several))
        .await
        .expect_err("two projected expressions are refused");
    refused_with(&error, &SqlState::SYNTAX_ERROR);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A relation joined on columns whose domain is over an enum compares them
/// as the enum wherever it is joined: a related read, the graph's edge, both
/// loaders — whose batch key is the column — and a linked read scoped by a
/// key of the domain.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]    a relation over columns whose
// domain is over an enum joins them as the enum in a related graph read, inner_join, a graph
// edge, load_one and load_many, and find_related and find_linked scope by such a key, live
// [spec:pgorm:sem:query.build.join+5/test]    a pair over such columns joins them cast
// [spec:pgorm:sem:query.loader.batching+8/test]    a batch keyed by such a column casts the
// key column and its values, from either side of the relation
#[pgorm_macros::test]
async fn domain_enum_relations_join_as_the_enum() -> Result<(), Error> {
    use pgorm::{LoaderTrait, RelatedLink};

    let ctx = TestContext::new("domain_enum_relations").await;
    let db = ctx.db.get().await?;
    lines_and_tempers(&db).await?;

    let labelled = |rows: Vec<(line::Model, Option<temper::Model>)>| {
        rows.into_iter()
            .map(|(line, temper)| (line.id, temper.map(|temper| temper.label)))
            .collect::<Vec<_>>()
    };
    let related = line::Entity::graph()
        .related_maybe::<temper::Entity>()
        .order_by_asc(line::Column::Id);
    assert!(
        related.as_query().to_string().contains(
            r#"LEFT JOIN "temper" ON CAST("line"."mood" AS mood) = CAST("temper"."mood" AS mood)"#
        ),
        "{}",
        related.as_query().to_string()
    );
    let steady = || Some("steady".to_owned());
    let bright = || Some("bright".to_owned());
    assert_eq!(
        labelled(related.all(&db).await?),
        [
            (1, steady()),
            (2, bright()),
            (3, steady()),
            (4, bright()),
            (5, steady())
        ]
    );
    let joined = line::Entity::find()
        .inner_join(temper::Entity)
        .filter(temper::Column::Label.eq("bright"))
        .order_by_asc(line::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(
        joined.iter().map(|line| line.id).collect::<Vec<_>>(),
        [2, 4]
    );

    let graph = line::Entity::graph()
        .join_one::<temper::Entity>(<line::Entity as Related<temper::Entity>>::to())
        .order_by_asc(line::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(
        graph
            .iter()
            .map(|(line, temper)| (line.id, temper.label.as_str()))
            .collect::<Vec<_>>(),
        [
            (1, "steady"),
            (2, "bright"),
            (3, "steady"),
            (4, "bright"),
            (5, "steady")
        ]
    );

    let lines = line::Entity::find()
        .order_by_asc(line::Column::Id)
        .all(&db)
        .await?;
    let tempers = lines.load_one(temper::Entity, &db).await?;
    assert_eq!(
        tempers
            .iter()
            .map(|temper| temper.as_ref().map(|temper| temper.mood))
            .collect::<Vec<_>>(),
        lines.iter().map(|line| Some(line.mood)).collect::<Vec<_>>()
    );
    let tempers = temper::Entity::find()
        .order_by_asc(temper::Column::Label)
        .all(&db)
        .await?;
    let loaded = tempers.load_many(line::Entity, &db).await?;
    assert_eq!(
        loaded
            .iter()
            .map(|lines| {
                let mut ids = lines.iter().map(|line| line.id).collect::<Vec<_>>();
                ids.sort_unstable();
                ids
            })
            .collect::<Vec<_>>(),
        [vec![2, 4], vec![1, 3, 5]]
    );

    let glad = temper::Entity::find_by_id(Mood::Glad).one(&db).await?;
    let ids = |lines: Vec<line::Model>| {
        let mut ids = lines.iter().map(|line| line.id).collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    };
    assert_eq!(ids(glad.find_related(line::Entity).all(&db).await?), [2, 4]);
    let linked = glad.find_linked(RelatedLink::to(line::Entity));
    assert!(
        linked
            .as_query()
            .to_string()
            .contains(r#"WHERE CAST("r0"."mood" AS mood) = CAST('glad' AS mood)"#),
        "{}",
        linked.as_query().to_string()
    );
    assert_eq!(ids(linked.all(&db).await?), [2, 4]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A graph cursor's tiebreak on a key whose domain is over an enum compares
/// it as the enum, as an order column on it does: the root's key under
/// `cursor_by`, a joined slot's under `cursor_by`, and the ordered slot's own
/// key under `cursor_by_on`, each resumed from inside a run of rows sharing
/// the order-column value.
// [spec:pgorm:sem:exec.cursor.keyset+8/test]    a whole-keyset boundary casts a tiebreak column
// whose domain is over an enum, and its value, to the enum, live
// [spec:pgorm:sem:query.graph.cursor+4/test]    the root's key, a joined slot's key and the
// ordered slot's own key, each a domain over an enum, tiebreak a graph cursor live
#[pgorm_macros::test]
async fn domain_enum_graph_cursors_tiebreak_as_the_enum() -> Result<(), Error> {
    let ctx = TestContext::new("domain_enum_graph_cursors").await;
    let db = ctx.db.get().await?;
    lines_and_tempers(&db).await?;

    let labelled = |rows: Vec<(temper::Model, Option<line::Model>)>| {
        rows.into_iter()
            .map(|(temper, line)| (temper.label, line.map(|line| line.id)))
            .collect::<Vec<_>>()
    };
    let by_label = || {
        temper::Entity::graph()
            .join_maybe::<line::Entity>(<temper::Entity as Related<line::Entity>>::to())
            .cursor_by(temper::Column::Label)
    };
    let bright = |id| ("bright".to_owned(), Some(id));
    let steady = |id| ("steady".to_owned(), Some(id));
    assert_eq!(
        labelled(
            by_label()
                .after_with(("bright", Mood::Glad, 2))
                .first(2)
                .all(&db)
                .await?
        ),
        [bright(4), steady(1)]
    );
    assert_eq!(
        labelled(
            by_label()
                .before_with(("steady", Mood::Calm, 3))
                .last(2)
                .all(&db)
                .await?
        ),
        [bright(4), steady(1)]
    );

    let ids = |rows: Vec<(line::Model, temper::Model)>| {
        rows.into_iter()
            .map(|(line, temper)| (line.id, temper.label))
            .collect::<Vec<_>>()
    };
    let graph = || {
        line::Entity::graph()
            .join_one::<temper::Entity>(<line::Entity as Related<temper::Entity>>::to())
    };
    assert_eq!(
        ids(graph()
            .cursor_by(line::Column::Qty)
            .after_with((1, 2, Mood::Glad))
            .first(2)
            .all(&db)
            .await?),
        [(3, "steady".to_owned()), (4, "bright".to_owned())]
    );
    assert_eq!(
        ids(graph()
            .cursor_by_on::<1, _>(temper::Column::Label)
            .after_with(("bright", Mood::Glad, 2))
            .first(2)
            .all(&db)
            .await?),
        [(4, "bright".to_owned()), (1, "steady".to_owned())]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A tiebreak a caller appends on a column whose domain is over an enum
/// compares as the enum, its casts taken from the column; the same entry
/// installed as bare identifiers has no operator to compare with.
// [spec:pgorm:sem:exec.cursor.keyset+8/test]    an entry appended through
// add_secondary_order_by on a column whose domain is over an enum resumes from inside a run,
// in either direction, where set_secondary_order_by's bare entry is refused with 42883, live
#[pgorm_macros::test]
async fn domain_enum_appended_tiebreak_compares_as_enum() -> Result<(), Error> {
    use line::Column::{Id, Mood as Moods, Qty};
    use pgorm::IntoKey;

    let ctx = TestContext::new("domain_enum_appended_tiebreak").await;
    let db = ctx.db.get().await?;
    lines_and_tempers(&db).await?;

    let page = |models: Vec<line::Model>| models.iter().map(|line| line.id).collect::<Vec<_>>();
    // Every line's quantity is 1, so the tiebreaks order the rows: the calm
    // lines 1, 3 and 5, then the glad lines 2 and 4.
    let typed = || {
        let mut cursor = line::Entity::find().cursor_by(Qty);
        cursor
            .add_secondary_order_by(line::Entity, Moods)
            .add_secondary_order_by(line::Entity, Id);
        cursor
    };
    assert_eq!(page(typed().first(2).all(&db).await?), [1, 3]);
    assert_eq!(
        page(
            typed()
                .after_with((1, Mood::Calm, 3))
                .first(2)
                .all(&db)
                .await?
        ),
        [5, 2]
    );
    assert_eq!(
        page(
            typed()
                .before_with((1, Mood::Glad, 4))
                .last(2)
                .all(&db)
                .await?
        ),
        [5, 2]
    );
    assert_eq!(
        page(
            typed()
                .desc()
                .after_with((1, Mood::Glad, 2))
                .first(2)
                .all(&db)
                .await?
        ),
        [5, 3]
    );

    let mut untyped = line::Entity::find().cursor_by(Qty);
    untyped.set_secondary_order_by(vec![
        (Name::new(line::Entity), Moods.into_key()),
        (Name::new(line::Entity), Id.into_key()),
    ]);
    let refused = untyped
        .after_with((1, Mood::Calm, 3))
        .first(2)
        .all(&db)
        .await
        .expect_err("a bare value has no operator against the domain");
    refused_with(&refused, &SqlState::UNDEFINED_FUNCTION);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A pipeline compares an entity column whose domain is over an enum as the
/// enum — against a literal, a bound value, a list, or the same domain across
/// a join — and min and max aggregate it as the enum.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]    pipeline comparisons, in_array,
// coalesce, a join condition and min / max over a column whose domain is over an enum, live
#[pgorm_macros::test]
async fn domain_enum_pipeline_compares_as_the_enum() -> Result<(), Error> {
    use pgorm::pipeline::{CastType, ExprOps, JoinSide, Pipeline, max, min};

    let ctx = TestContext::new("domain_enum_pipeline").await;
    let db = ctx.db.get().await?;
    lines_and_tempers(&db).await?;

    let conn = &db;
    let ids = |pipeline: Pipeline| async move {
        pipeline
            .sort(line::Column::Id)
            .select(line::Column::Id)
            .into_tuple::<i32>()
            .expect("the pipeline compiles")
            .all(conn)
            .await
    };
    let glad = Pipeline::from(line::Entity).filter(line::Column::Mood.eq("glad"));
    assert_eq!(ids(glad).await?, [2, 4]);
    let bound = Pipeline::from(line::Entity)
        .filter_with(|binder| line::Column::Mood.lt(binder.bind(Mood::Glad)));
    assert_eq!(ids(bound).await?, [1, 3, 5]);
    let listed = Pipeline::from(line::Entity).filter(line::Column::Mood.in_array(["glad"]));
    assert_eq!(ids(listed).await?, [2, 4]);
    let coalesced =
        Pipeline::from(line::Entity).filter(line::Column::Mood.coalesce("glad").eq("calm"));
    assert_eq!(ids(coalesced).await?, [1, 3, 5]);
    let joined = Pipeline::from(line::Entity)
        .join(
            JoinSide::Inner,
            temper::Entity,
            line::Column::Mood.eq(temper::Column::Mood),
        )
        .filter(temper::Column::Label.eq("bright"));
    assert_eq!(ids(joined).await?, [2, 4]);

    let extremes = Pipeline::from(line::Entity)
        .group(line::Column::Qty)
        .aggregate((
            max(line::Column::Mood).cast(CastType::Text).as_("highest"),
            min(line::Column::Mood).cast(CastType::Text).as_("lowest"),
        ))
        .into_tuple::<(i32, String, String)>()
        .expect("the pipeline compiles")
        .all(&db)
        .await?;
    assert_eq!(extremes, [(1, "glad".to_owned(), "calm".to_owned())]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A pipeline compares a column whose domain is over an enum as the enum
/// where it reads the column without its entity's own reference: a named
/// relation's column through `Named::col`, which the entity types, and a
/// `col` reference or an alias token through `compares_as`, which names the
/// enum. Each left bare is compared as the domain, which the server refuses.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14/test]    a named relation's column,
// a col reference and an alias token naming a projection compare as the enum, bound or
// literal, live, where each left bare is refused with 42883
#[pgorm_macros::test]
async fn untyped_pipeline_columns_compare_as_the_enum() -> Result<(), Error> {
    use pgorm::pipeline::{Expr, ExprOps, IntoSource, JoinSide, Pipeline, alias, col};

    let ctx = TestContext::new("domain_enum_untyped_pipeline").await;
    let db = ctx.db.get().await?;
    lines_and_tempers(&db).await?;

    let conn = &db;
    let ids = |pipeline: Pipeline| async move {
        pipeline
            .sort(line::Column::Id)
            .select(line::Column::Id)
            .into_tuple::<i32>()
            .expect("the pipeline compiles")
            .all(conn)
            .await
    };
    let undefined = |result: Result<Vec<i32>, Error>| {
        refused_with(
            &result.expect_err("the domain compared bare has no operator"),
            &SqlState::UNDEFINED_FUNCTION,
        );
    };

    let peer = temper::Entity.named(alias("peer"));
    let joined = |mood: Expr<'static>| {
        Pipeline::from(line::Entity)
            .join(JoinSide::Inner, peer.clone(), line::Column::Mood.eq(mood))
            .filter(peer.col(temper::Column::Label).eq("bright"))
    };
    assert_eq!(ids(joined(peer.col(temper::Column::Mood))).await?, [2, 4]);
    undefined(ids(joined(col(alias("peer"), alias("mood")))).await);

    let bound = |typed: bool| {
        Pipeline::from(line::Entity).filter_with(move |binder| {
            let mood = col(alias("line"), alias("mood"));
            let mood = if typed {
                mood.compares_as(alias("mood"))
            } else {
                mood
            };
            mood.eq(binder.bind("glad"))
        })
    };
    assert_eq!(ids(bound(true)).await?, [2, 4]);
    undefined(ids(bound(false)).await);

    let m = alias("m");
    let derived = |compared: Expr<'static>| {
        Pipeline::from(line::Entity)
            .derive(line::Column::Mood.as_(m))
            .filter(compared.lt("glad"))
    };
    assert_eq!(ids(derived(m.compares_as(alias("mood")))).await?, [1, 3, 5]);
    undefined(ids(derived(m.into())).await);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A function the planner folds into a plan as a constant, being declared
/// immutable, though it reads a table: a session answers `SELECT plan_mark()`
/// with what `plan_mark` held when the session last planned that statement.
const PLAN_MARK: &str = "CREATE TABLE plan_mark (n integer); INSERT INTO plan_mark VALUES (1); \
                         CREATE FUNCTION plan_mark() RETURNS integer IMMUTABLE LANGUAGE plpgsql \
                         AS $$ BEGIN RETURN (SELECT n FROM plan_mark); END $$";

async fn plan_mark(db: &impl ConnectionTrait) -> Result<i32, Error> {
    Ok(db.query_one("SELECT plan_mark()", &[]).await?.get(0))
}

/// Plan `SELECT plan_mark()` on `db`'s session and then change what it reads,
/// so that the session answers 1 until it re-plans its statements and 2 after.
/// Run before the statement whose plan is in question is prepared, so that a
/// re-plan of that statement is a re-plan of this one too.
async fn mark_plans(db: &impl ConnectionTrait) -> Result<(), Error> {
    db.batch_execute(PLAN_MARK).await?;
    assert_eq!(plan_mark(db).await?, 1);
    db.batch_execute("UPDATE plan_mark SET n = 2").await
}

/// Whether `db`'s session has re-planned its statements since [`mark_plans`].
/// A reset of the session's caches re-plans every statement it keeps prepared,
/// under the same name, and one comes whenever the session falls behind the
/// server's shared invalidation queue, which other databases' catalogue churn
/// can cause at any moment; asked after a statement ran, a no says that the
/// statement ran with the plan it was prepared with.
async fn replanned(db: &impl ConnectionTrait) -> Result<bool, Error> {
    Ok(plan_mark(db).await? != 1)
}

/// A `line` written with every column but `qty`, which the domain's default
/// fills.
fn defaulted(id: i32) -> line::ActiveModel {
    line::ActiveModel {
        id: set(id),
        mood: set(Mood::Calm),
        codes: set(Vec::new()),
        ..Default::default()
    }
}

/// PostgreSQL's plan cache does not see a domain's default change: an insert a
/// connection prepared before `SET DEFAULT` keeps writing the default it was
/// planned with, where the same insert prepared afterwards — on another
/// connection, whose statement cache is its own — writes the new one. A
/// check added at the same point holds at once. The old default is held only
/// while the session is shown not to have re-planned ([`replanned`]): a re-plan
/// keeps the statement and writes the new default.
// [spec:pgorm:req:sql.ddl.domain+3/test]    an insert prepared before SET DEFAULT keeps the old
// default on its connection, as one statement, while the session has not re-planned, a fresh
// connection's writes the new one, and a check added then holds at once
#[pgorm_macros::test]
async fn a_prepared_insert_keeps_its_planned_default() -> Result<(), Error> {
    let ctx = TestContext::new("domain_default_plan_cache").await;
    let first = ctx.db.get().await?;
    domains(&first).await?;
    first
        .batch_execute(
            &pgorm::Schema::new()
                .create_table_from_entity(line::Entity)?
                .to_string(),
        )
        .await?;
    mark_plans(&first).await?;
    assert_eq!(defaulted(1).insert(&first).await?.qty, 1);
    let prepared = prepared_insert(&first).await?;
    assert!(prepared.is_some());

    let set_default = Domain::alter((n("billing"), n("quantity")))
        .set_default(2)
        .to_string();
    assert_eq!(
        set_default,
        r#"ALTER DOMAIN "billing"."quantity" SET DEFAULT 2"#
    );
    first.batch_execute(&set_default).await?;
    let written = defaulted(2).insert(&first).await?.qty;
    assert_eq!(prepared_insert(&first).await?, prepared);
    if !replanned(&first).await? {
        assert_eq!(
            written, 1,
            "the insert keeps the default it was planned with"
        );
    }
    let second = ctx.db.get().await?;
    assert_eq!(defaulted(3).insert(&second).await?.qty, 2);

    first
        .batch_execute(
            &Domain::alter((n("billing"), n("quantity")))
                .add_check(
                    DomainCheck::new(Expr::domain_value().gt(1))
                        .name(n("several"))
                        .not_valid(),
                )
                .to_string(),
        )
        .await?;
    let stale = defaulted(4).insert(&first).await;
    if !replanned(&first).await? {
        let refused = stale.expect_err("the planned default under the new check");
        checked_by(&refused, "several", "quantity");
    }

    drop(first);
    drop(second);
    ctx.delete().await;
    Ok(())
}

/// A reset of the session's caches re-plans every statement it keeps
/// prepared, under the name it was prepared with, and [`replanned`] sees it:
/// after `DISCARD PLANS`, and after another session creates a schema in the
/// database, the insert prepared before `SET DEFAULT` writes the new default
/// as the same statement. A session falling behind the server's shared
/// invalidation queue is reset the same way, but only catalogue churn heavy
/// enough to reset every other session on the server causes that.
// [spec:pgorm:req:sql.ddl.domain+3/test]    a reset of the session's caches, by DISCARD PLANS or a
// schema created in its database, re-plans the kept insert under its name with the new default
#[pgorm_macros::test]
async fn a_reset_replans_the_kept_insert() -> Result<(), Error> {
    let ctx = TestContext::new("domain_default_replanned").await;
    let first = ctx.db.get().await?;
    domains(&first).await?;
    first
        .batch_execute(
            &pgorm::Schema::new()
                .create_table_from_entity(line::Entity)?
                .to_string(),
        )
        .await?;
    mark_plans(&first).await?;
    assert_eq!(defaulted(1).insert(&first).await?.qty, 1);
    let prepared = prepared_insert(&first).await?;
    assert!(prepared.is_some());

    let second = ctx.db.get().await?;
    second
        .batch_execute(r#"ALTER DOMAIN "billing"."quantity" SET DEFAULT 2"#)
        .await?;
    first.batch_execute("DISCARD PLANS").await?;
    assert_eq!(defaulted(2).insert(&first).await?.qty, 2);
    assert_eq!(prepared_insert(&first).await?, prepared);
    assert_eq!(plan_mark(&first).await?, 2);

    second
        .batch_execute(
            r#"UPDATE plan_mark SET n = 3; ALTER DOMAIN "billing"."quantity" SET DEFAULT 3;
               CREATE SCHEMA elsewhere"#,
        )
        .await?;
    assert_eq!(defaulted(3).insert(&first).await?.qty, 3);
    assert_eq!(prepared_insert(&first).await?, prepared);
    assert_eq!(plan_mark(&first).await?, 3);

    drop(first);
    drop(second);
    ctx.delete().await;
    Ok(())
}

/// The server process behind a connection, which names the physical
/// connection the pool lent.
async fn backend(db: &impl ConnectionTrait) -> Result<i32, Error> {
    Ok(db.query_one("SELECT pg_backend_pid()", &[]).await?.get(0))
}

/// The insert into `line` the connection's session holds prepared, or `None`.
async fn prepared_insert(db: &impl ConnectionTrait) -> Result<Option<String>, Error> {
    Ok(db
        .query_opt(PREPARED_STATEMENT, &[&r#"INSERT INTO "line""#])
        .await?
        .map(|row| row.get(0)))
}

/// After `clear_statement_caches`, a connection that already ran an insert
/// prepares it again and writes the domain's new default: one idle at the time
/// as soon as it is lent, and one checked out at the time once it is returned
/// and lent again, keeping its insert until then. The insert it had prepared
/// is closed on the server before it runs anything.
// [spec:pgorm:req:conn.pool.statement-cache.clear+1/test]    idle and checked-out connections that
// prepared an insert before SET DEFAULT write the new default once next lent, their old statement
// deallocated on the server, while a connection still checked out keeps it
// [spec:pgorm:req:sql.ddl.domain+3/test]    the clear is the remedy for a planned default
#[pgorm_macros::test]
async fn a_cleared_pool_writes_the_new_default() -> Result<(), Error> {
    let ctx = TestContext::new("domain_default_cleared").await;
    let held = ctx.db.get().await?;
    domains(&held).await?;
    held.batch_execute(
        &pgorm::Schema::new()
            .create_table_from_entity(line::Entity)?
            .to_string(),
    )
    .await?;

    let idle = ctx.db.get().await?;
    assert_eq!(defaulted(1).insert(&held).await?.qty, 1);
    assert_eq!(defaulted(2).insert(&idle).await?.qty, 1);
    let (held_backend, idle_backend) = (backend(&held).await?, backend(&idle).await?);
    let (held_insert, idle_insert) = (prepared_insert(&held).await?, prepared_insert(&idle).await?);
    assert!(held_insert.is_some() && idle_insert.is_some());
    drop(idle);

    held.batch_execute(
        &Domain::alter((n("billing"), n("quantity")))
            .set_default(2)
            .to_string(),
    )
    .await?;
    ctx.db.clear_statement_caches();

    defaulted(3).insert(&held).await?;
    assert_eq!(
        prepared_insert(&held).await?,
        held_insert,
        "a connection checked out keeps its insert"
    );

    let idle = ctx.db.get().await?;
    assert_eq!(backend(&idle).await?, idle_backend);
    assert_eq!(prepared_insert(&idle).await?, None);
    assert_eq!(defaulted(4).insert(&idle).await?.qty, 2);
    let prepared_again = prepared_insert(&idle).await?;
    assert!(prepared_again.is_some() && prepared_again != idle_insert);

    drop(held);
    let held = ctx.db.get().await?;
    assert_eq!(backend(&held).await?, held_backend);
    assert_eq!(prepared_insert(&held).await?, None);
    assert_eq!(defaulted(5).insert(&held).await?.qty, 2);
    let prepared_again = prepared_insert(&held).await?;
    assert!(prepared_again.is_some() && prepared_again != held_insert);

    drop(held);
    drop(idle);
    ctx.delete().await;
    Ok(())
}

/// Each alter action does what it says to a domain a table's column is typed
/// with: a default fills an insert that leaves the column out, `NOT NULL` is
/// refused over a null and then holds, and a named check is added, renamed and
/// dropped; a domain is renamed with the column still typed by it.
// [spec:pgorm:req:sql.ddl.domain+3/test]    each alter action changes the domain: a default and
// its drop, NOT NULL refused over a null (23502) and then held, a check added, renamed and
// dropped, a rename of the domain; and a missing constraint (42704) or a taken name (42710)
// refused
#[pgorm_macros::test]
async fn each_alter_action_changes_the_domain() -> Result<(), Error> {
    let ctx = TestContext::new("domain_alter_actions").await;
    let db = ctx.db.get().await?;
    db.batch_execute(&Domain::create(n("d"), ColumnType::Integer).to_string())
        .await?;
    db.batch_execute("CREATE TABLE t (id integer PRIMARY KEY, v d)")
        .await?;

    let set_default = Domain::alter(n("d")).set_default(7).to_string();
    assert_eq!(set_default, r#"ALTER DOMAIN "d" SET DEFAULT 7"#);
    db.batch_execute(&set_default).await?;
    db.batch_execute("INSERT INTO t (id) VALUES (1)").await?;
    assert_eq!(value_of(&db, 1).await?, Some(7));
    db.batch_execute(&Domain::alter(n("d")).drop_default().to_string())
        .await?;
    db.batch_execute("INSERT INTO t (id) VALUES (2)").await?;
    assert_eq!(value_of(&db, 2).await?, None);

    let not_null = Domain::alter(n("d")).set_not_null().to_string();
    refused_with(
        &refused(&db, &not_null).await,
        &SqlState::NOT_NULL_VIOLATION,
    );
    db.batch_execute("DELETE FROM t WHERE id = 2").await?;
    db.batch_execute(&not_null).await?;
    let null = refused(&db, "INSERT INTO t (id, v) VALUES (3, NULL)").await;
    refused_with(&null, &SqlState::NOT_NULL_VIOLATION);
    db.batch_execute(&Domain::alter(n("d")).drop_not_null().to_string())
        .await?;
    db.batch_execute("INSERT INTO t (id, v) VALUES (3, NULL)")
        .await?;

    let small = Domain::alter(n("d"))
        .add_check(DomainCheck::new(Expr::domain_value().lt(100)).name(n("small")))
        .to_string();
    db.batch_execute(&small).await?;
    checked_by(
        &refused(&db, "INSERT INTO t VALUES (4, 500)").await,
        "small",
        "d",
    );
    db.batch_execute(
        &Domain::alter(n("d"))
            .add_check(Expr::domain_value().gt(-100))
            .to_string(),
    )
    .await?;
    db.batch_execute(
        &Domain::alter(n("d"))
            .rename_constraint(n("small"), n("tiny"))
            .to_string(),
    )
    .await?;
    assert_eq!(
        constraints_of(&db, "d").await?,
        [
            ("d_check".to_owned(), "c".to_owned(), true),
            ("tiny".to_owned(), "c".to_owned(), true),
        ]
    );
    let taken = Domain::alter(n("d"))
        .rename_constraint(n("tiny"), n("d_check"))
        .to_string();
    refused_with(&refused(&db, &taken).await, &SqlState::DUPLICATE_OBJECT);

    db.batch_execute(&Domain::alter(n("d")).drop_constraint(n("tiny")).to_string())
        .await?;
    let again = Domain::alter(n("d"))
        .drop_constraint(ConstraintDrop::new(n("tiny")).if_exists().restrict())
        .to_string();
    assert_eq!(
        again,
        r#"ALTER DOMAIN "d" DROP CONSTRAINT IF EXISTS "tiny" RESTRICT"#
    );
    db.batch_execute(&again).await?;
    let missing = Domain::alter(n("d")).drop_constraint(n("tiny")).to_string();
    refused_with(&refused(&db, &missing).await, &SqlState::UNDEFINED_OBJECT);
    let missing = Domain::alter(n("d"))
        .validate_constraint(n("tiny"))
        .to_string();
    refused_with(&refused(&db, &missing).await, &SqlState::UNDEFINED_OBJECT);
    db.batch_execute("INSERT INTO t VALUES (4, 500)").await?;

    db.batch_execute(&Domain::alter(n("d")).rename_to(n("e")).to_string())
        .await?;
    assert_eq!(
        columns_of(&db, "t").await?,
        [
            ("id".to_owned(), "integer".to_owned()),
            ("v".to_owned(), "e".to_owned()),
        ]
    );
    db.batch_execute(&Domain::create(n("f"), ColumnType::Text).to_string())
        .await?;
    let taken = Domain::alter(n("e")).rename_to(n("f")).to_string();
    refused_with(&refused(&db, &taken).await, &SqlState::DUPLICATE_OBJECT);
    let missing = Domain::alter(n("d")).drop_default().to_string();
    refused_with(&refused(&db, &missing).await, &SqlState::UNDEFINED_OBJECT);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A check added over values that fail it is refused; added `NOT VALID`, it
/// leaves them and holds every value written after it, and validating it is
/// refused until the failing value is gone.
// [spec:pgorm:req:sql.ddl.domain+3/test]    a check over failing values is refused (23514);
// NOT VALID leaves them, holds new values, and VALIDATE CONSTRAINT is refused (23514) until
// they are gone
#[pgorm_macros::test]
async fn a_not_valid_check_leaves_old_values() -> Result<(), Error> {
    let ctx = TestContext::new("domain_not_valid_check").await;
    let db = ctx.db.get().await?;
    db.batch_execute(&Domain::create(n("d"), ColumnType::Integer).to_string())
        .await?;
    db.batch_execute("CREATE TABLE t (id integer PRIMARY KEY, v d)")
        .await?;
    db.batch_execute("INSERT INTO t VALUES (1, -5), (2, 3)")
        .await?;

    let valid = Domain::alter(n("d"))
        .add_check(DomainCheck::new(Expr::domain_value().gt(0)).name(n("positive")))
        .to_string();
    refused_with(&refused(&db, &valid).await, &SqlState::CHECK_VIOLATION);

    let not_valid = Domain::alter(n("d"))
        .add_check(
            DomainCheck::new(Expr::domain_value().gt(0))
                .name(n("positive"))
                .not_valid(),
        )
        .to_string();
    assert_eq!(
        not_valid,
        r#"ALTER DOMAIN "d" ADD CONSTRAINT "positive" CHECK (VALUE > 0) NOT VALID"#
    );
    db.batch_execute(&not_valid).await?;
    assert_eq!(
        constraints_of(&db, "d").await?,
        [("positive".to_owned(), "c".to_owned(), false)]
    );
    checked_by(
        &refused(&db, "INSERT INTO t VALUES (3, -1)").await,
        "positive",
        "d",
    );

    let validate = Domain::alter(n("d"))
        .validate_constraint(n("positive"))
        .to_string();
    assert_eq!(
        validate,
        r#"ALTER DOMAIN "d" VALIDATE CONSTRAINT "positive""#
    );
    refused_with(&refused(&db, &validate).await, &SqlState::CHECK_VIOLATION);
    db.batch_execute("DELETE FROM t WHERE id = 1").await?;
    db.batch_execute(&validate).await?;
    assert_eq!(
        constraints_of(&db, "d").await?,
        [("positive".to_owned(), "c".to_owned(), true)]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A drop of a domain a column has is refused under `RESTRICT`, said or
/// unsaid, until it cascades and takes the column; several domains drop in
/// one statement, and a missing one is refused unless the drop says `IF
/// EXISTS`.
// [spec:pgorm:req:sql.ddl.domain+3/test]    a drop of a domain a column is typed with is refused
// under RESTRICT, said or unsaid (2BP01), until CASCADE drops the column; several drop at once,
// and a missing domain is 42704 unless IF EXISTS
#[pgorm_macros::test]
async fn a_drop_is_refused_while_typed_with() -> Result<(), Error> {
    let ctx = TestContext::new("domain_drop_restrict").await;
    let db = ctx.db.get().await?;
    db.batch_execute(&Domain::create(n("d"), ColumnType::Integer).to_string())
        .await?;
    db.batch_execute(&Domain::create(n("e"), ColumnType::Text).to_string())
        .await?;
    db.batch_execute("CREATE TABLE t (id integer PRIMARY KEY, v d)")
        .await?;

    let unsaid = Domain::drop(n("d")).to_string();
    refused_with(
        &refused(&db, &unsaid).await,
        &SqlState::DEPENDENT_OBJECTS_STILL_EXIST,
    );
    let restrict = Domain::drop(n("d")).restrict().to_string();
    assert_eq!(restrict, r#"DROP DOMAIN "d" RESTRICT"#);
    refused_with(
        &refused(&db, &restrict).await,
        &SqlState::DEPENDENT_OBJECTS_STILL_EXIST,
    );

    let cascade = Domain::drop(n("d")).cascade().to_string();
    db.batch_execute(&cascade).await?;
    assert_eq!(
        columns_of(&db, "t").await?,
        [("id".to_owned(), "integer".to_owned())]
    );

    refused_with(
        &refused(&db, &Domain::drop(n("d")).to_string()).await,
        &SqlState::UNDEFINED_OBJECT,
    );
    let both = Domain::drop(n("d")).name(n("e")).if_exists().to_string();
    assert_eq!(both, r#"DROP DOMAIN IF EXISTS "d", "e""#);
    db.batch_execute(&both).await?;
    let left = db
        .query_one(
            "SELECT count(*) FROM pg_type WHERE typname IN ('d', 'e')",
            &[],
        )
        .await?;
    assert_eq!(left.get::<_, i64>(0), 0);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// What the server refuses, which no builder value can see: a name a type
/// already has, two constraints of one name, a subquery in a check or a
/// default, a collation on a type without one, and a default its base cannot
/// hold.
// [spec:pgorm:req:sql.ddl.domain+3/test]    a taken name (42710), a constraint named twice (42710),
// a subquery in a check or a default (0A000), a COLLATE on integer (42804) and a default its base
// cannot read (22P02) are the server's refusals
#[pgorm_macros::test]
async fn the_server_refuses_what_no_builder_sees() -> Result<(), Error> {
    let ctx = TestContext::new("domain_server_refusals").await;
    let db = ctx.db.get().await?;
    db.batch_execute(&Domain::create(n("d"), ColumnType::Integer).to_string())
        .await?;

    let cases = [
        (
            Domain::create(n("d"), ColumnType::Text).to_string(),
            SqlState::DUPLICATE_OBJECT,
        ),
        (
            Domain::create(n("twice"), ColumnType::Integer)
                .check(DomainCheck::new(Expr::domain_value().gt(0)).name(n("c")))
                .check(DomainCheck::new(Expr::domain_value().lt(9)).name(n("c")))
                .to_string(),
            SqlState::DUPLICATE_OBJECT,
        ),
        (
            Domain::create(n("sub"), ColumnType::Integer)
                .check(Expr::domain_value().in_subquery(Query::select().expr(1).take()))
                .to_string(),
            SqlState::FEATURE_NOT_SUPPORTED,
        ),
        (
            Domain::create(n("sub"), ColumnType::Integer)
                .default(pgorm::pgorm_query::SimpleExpr::SubQuery(
                    None,
                    Box::new(Query::select().expr(1).take().into_sub_query_statement()),
                ))
                .to_string(),
            SqlState::FEATURE_NOT_SUPPORTED,
        ),
        (
            Domain::create(n("collated"), ColumnType::Integer)
                .collate(n("C"))
                .to_string(),
            SqlState::DATATYPE_MISMATCH,
        ),
        (
            Domain::create(n("wrong"), ColumnType::Integer)
                .default("abc")
                .to_string(),
            SqlState::INVALID_TEXT_REPRESENTATION,
        ),
    ];
    for (sql, state) in cases {
        refused_with(&refused(&db, &sql).await, &state);
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A domain is created inside a transaction, and a rollback takes it away.
// [spec:pgorm:req:sql.ddl.domain+3/test]    a domain created in a transaction is gone when the
// transaction rolls back
#[pgorm_macros::test]
async fn a_domain_rolls_back_with_its_transaction() -> Result<(), Error> {
    let ctx = TestContext::new("domain_rolls_back").await;
    let mut db = ctx.db.get().await?;
    let outcome = db
        .transaction(async |txn| {
            txn.batch_execute(&Domain::create(n("d"), ColumnType::Integer).to_string())
                .await?;
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);
    let left = db
        .query_one("SELECT count(*) FROM pg_type WHERE typname = 'd'", &[])
        .await?;
    assert_eq!(left.get::<_, i64>(0), 0);

    drop(db);
    ctx.delete().await;
    Ok(())
}
