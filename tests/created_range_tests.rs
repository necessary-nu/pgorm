#![allow(unused_imports, dead_code)]

//! Values of range types a schema created, against a live PostgreSQL server.
//!
//! The unit tests hold the text form to the quoting PostgreSQL's range parser
//! reads. What only a server settles is what that text means: that a value
//! written as its text and cast to the type by name — bound as a `text`
//! parameter, or escaped inline — is the value read back from the range's
//! binary form; that the server's canonical text reads back as the value
//! written; that a bound holding the range syntax's own characters stays
//! data; that a built-in range value reaches a range type created over its
//! subtype, which it cannot by any cast; and what the server refuses.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    BinOper, ColumnType, Expr, Name, Query, SimpleExpr, TypeName, Value, Values,
    extension::{RangeDefinition, Type},
};
use pgorm::{
    ConnectionTrait, CreatedRange, DecodeRaw, QueryTrait, Schema, TryGetable, entity::prelude::*,
};
use pretty_assertions::assert_eq;
use std::ops::Bound::{Excluded, Included, Unbounded};
use tokio_postgres::error::SqlState;

mod measures {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, CreatedRange)]
    #[pgorm(range_name = "floatrange")]
    pub struct FloatRange(pub Range<f64>);

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(range_name = "slot")]
    pub struct Slot(pub Range<i32>);

    /// A name and a schema that need quoting, over `text`.
    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(range_name = "Label Span", schema_name = "We\"ird")]
    pub struct LabelSpan(pub Range<String>);

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "measures")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub span: FloatRange,
        pub slot: Slot,
        pub label: Option<LabelSpan>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// One range type created over each remaining subtype.
mod subtypes {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(range_name = "smallrange")]
    pub struct Small(pub Range<i16>);

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(range_name = "bigrange")]
    pub struct Big(pub Range<i64>);

    #[derive(Clone, Debug, PartialEq, CreatedRange)]
    #[pgorm(range_name = "realrange")]
    pub struct Real(pub Range<f32>);

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(range_name = "amountrange")]
    pub struct Amount(pub Range<Decimal>);

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(range_name = "dayrange")]
    pub struct Day(pub Range<Date>);

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(range_name = "shiftrange")]
    pub struct Shift(pub Range<Time>);

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(range_name = "stamprange")]
    pub struct Stamp(pub Range<DateTime>);

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(range_name = "instantrange")]
    pub struct Instant(pub Range<DateTimeWithTimeZone>);

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(range_name = "idrange")]
    pub struct Id(pub Range<Uuid>);

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "subtypes")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub small: Small,
        pub big: Big,
        pub real: Real,
        pub amount: Amount,
        pub day: Day,
        pub shift: Shift,
        pub stamp: Stamp,
        pub instant: Instant,
        pub uuid: Id,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

use measures::{FloatRange, LabelSpan, Slot};

/// A table whose columns are arrays of a range type the schema created.
mod shelves {
    use super::measures::Slot;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "shelves")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub slots: Vec<Slot>,
        pub spare: Option<Vec<Slot>>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[path = "../pgorm-codegen/tests/sql/created_range/measurement.rs"]
mod measurement;
/// The entity pgorm-codegen generates from its created-range schema: the
/// files its own tests hold the writer's output to, so what is run here is
/// what the generator writes.
#[path = "../pgorm-codegen/tests/sql/created_range/pgorm_range_types.rs"]
mod pgorm_range_types;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("created_range_tests").await;
    let db = ctx.db.get().await?;

    create_the_range_types(&db).await?;
    the_table_names_each_created_type(&db).await?;
    every_subtype_round_trips_through_an_entity(&db).await?;
    the_bound_and_inline_renderings_agree(&db).await?;
    the_canonical_text_reads_back_as_written(&db).await?;
    a_bound_holding_range_syntax_stays_data(&db).await?;
    a_builtin_range_value_reaches_a_created_type(&db).await?;
    a_created_range_over_int4_is_continuous(&db).await?;
    a_column_reads_only_its_own_subtype(&db).await?;
    the_server_refuses_what_it_cannot_store(&db).await?;
    a_subtype_modifier_is_dropped(&db).await?;
    created_range_arrays_bind_as_texts(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

/// A schema with range types it creates generates an entity that writes and
/// reads its rows, a range and a multirange each and an array of each: the
/// DDL the generator read, run as written, then the generated model inserted
/// and found.
// [spec:pgorm:sem:codegen.entity.types+13/test]
#[pgorm_macros::test]
async fn a_generated_entity_round_trips() -> Result<(), Error> {
    let ctx = TestContext::new("created_range_generated_tests").await;
    let db = ctx.db.get().await?;
    db.batch_execute("CREATE SCHEMA booking").await?;
    db.batch_execute(include_str!(
        "../pgorm-codegen/tests/sql/created_range/schema.sql"
    ))
    .await?;

    let written = measurement::Model {
        id: 1,
        span: pgorm_range_types::Floatrange(Range::new(Included(0.5), Unbounded)),
        slot: Some(Range::from(1..=5).into()),
        label: pgorm_range_types::Textrange(Range::from("a".to_owned().."b, \"c\"".to_owned())),
        spans: pgorm_range_types::Floatmultirange(
            [Range::from(0.5..1.5), Range::new(Excluded(2.0), Unbounded)]
                .into_iter()
                .collect(),
        ),
        slots: Some(Multirange::from(vec![Range::from(1..=5)]).into()),
        spares: Some(vec![
            pgorm_range_types::Floatrange(Range::from(1.5..2.5)),
            pgorm_range_types::Floatrange(Range::Empty),
        ]),
        ledgers: Some(vec![pgorm_range_types::Floatmultirange(
            [Range::from(0.5..1.5)].into_iter().collect(),
        )]),
    };
    let returned = written.clone().into_active_model().insert(&db).await?;
    assert_eq!(returned, written);
    let unslotted = measurement::Model {
        id: 2,
        slot: None,
        slots: None,
        spares: Some(Vec::new()),
        ledgers: None,
        ..written.clone()
    };
    unslotted.clone().into_active_model().insert(&db).await?;
    assert_eq!(
        measurement::Entity::find()
            .order_by_asc(measurement::Column::Id)
            .all(&db)
            .await?,
        vec![written, unslotted]
    );

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

/// Run one statement both ways — inline, and with its values bound — and
/// decode the one column it returns each time.
async fn both<T>(
    db: &DatabaseConnection,
    query: &pgorm::pgorm_query::SelectStatement,
) -> Result<(T, T), Error>
where
    T: TryGetable,
{
    let inline: T = (query.to_string(), Values(Vec::new()))
        .into_tuple()
        .one(db)
        .await?;
    let (sql, values) = query.build();
    let bound: T = (sql, values).into_tuple().one(db).await?;
    Ok((inline, bound))
}

fn select(expr: SimpleExpr) -> pgorm::pgorm_query::SelectStatement {
    Query::select().expr(expr).to_owned()
}

async fn create_the_range_types(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute("CREATE SCHEMA \"We\"\"ird\"").await?;
    let created = [
        (None, "floatrange", ColumnType::Double),
        (None, "slot", ColumnType::Integer),
        (Some("We\"ird"), "Label Span", ColumnType::Text),
        (None, "smallrange", ColumnType::SmallInteger),
        (None, "bigrange", ColumnType::BigInteger),
        (None, "realrange", ColumnType::Float),
        (None, "amountrange", ColumnType::Decimal(None)),
        (None, "dayrange", ColumnType::Date),
        (None, "shiftrange", ColumnType::Time),
        (None, "stamprange", ColumnType::Timestamp),
        (None, "instantrange", ColumnType::TimestampWithTimeZone),
        (None, "idrange", ColumnType::Uuid),
    ];
    for (schema, name, subtype) in created {
        let statement = match schema {
            Some(schema) => Type::create((n(schema), n(name)))
                .as_range(RangeDefinition::new(subtype))
                .to_string(),
            None => Type::create(n(name))
                .as_range(RangeDefinition::new(subtype))
                .to_string(),
        };
        db.batch_execute(&statement).await?;
    }
    Ok(())
}

/// A table built from the entity names each created type in full, quoted
/// where the name needs it, and the server takes it.
// [spec:pgorm:def:sql.value.created-range+8/test]
// [spec:pgorm:sem:macros.derive.created-range+3/test]
async fn the_table_names_each_created_type(db: &DatabaseConnection) -> Result<(), Error> {
    let schema = Schema::new();
    let measures = schema.create_table_from_entity(measures::Entity)?;
    let sql = measures.to_string();
    assert!(
        sql.contains(r#""span" floatrange NOT NULL"#)
            && sql.contains(r#""label" "We""ird"."Label Span""#),
        "{sql}"
    );
    create_table_without_asserts(db, &measures).await?;
    create_table_without_asserts(db, &schema.create_table_from_entity(subtypes::Entity)?).await?;
    assert_eq!(
        <Slot as pgorm::pgorm_query::ValueType>::column_type(),
        ColumnType::CreatedRange {
            name: n("slot"),
            schema: None,
            subtype: std::sync::Arc::new(ColumnType::Integer),
        }
    );
    Ok(())
}

fn measure(id: i32) -> measures::Model {
    measures::Model {
        id,
        span: FloatRange(Range::new(Excluded(-0.5), Included(1e300))),
        slot: Slot(Range::from(1..=5)),
        label: Some(LabelSpan(Range::from("a".to_owned().."b".to_owned()))),
    }
}

/// Every subtype is written by an entity insert, its text bound and cast by
/// name, and read back from the range's binary form by its select; a NULL of
/// a created range type round-trips too.
// [spec:pgorm:def:sql.value.created-range+8/test]
// [spec:pgorm:def:exec.decode.range+3/test]
async fn every_subtype_round_trips_through_an_entity(db: &DatabaseConnection) -> Result<(), Error> {
    let written = measure(1);
    written.clone().into_active_model().insert(db).await?;
    let empty = measures::Model {
        id: 2,
        span: FloatRange(Range::Empty),
        slot: Slot(Range::from(..)),
        label: None,
    };
    empty.clone().into_active_model().insert(db).await?;
    assert_eq!(
        measures::Entity::find()
            .order_by_asc(measures::Column::Id)
            .all(db)
            .await?,
        vec![written, empty]
    );

    let instant: DateTimeWithTimeZone = "2024-02-29T23:59:59.123456Z".parse().expect("an instant");
    let every = subtypes::Model {
        id: 1,
        small: subtypes::Small(Range::from(-3..i16::MAX)),
        big: subtypes::Big(Range::from(-5_000_000_000..)),
        real: subtypes::Real(Range::from(0.1..0.3)),
        amount: subtypes::Amount(Range::new(
            Excluded(Decimal::new(0, 2)),
            Included(Decimal::new(150, 2)),
        )),
        day: subtypes::Day(Range::from(..jiff::civil::date(2024, 2, 29))),
        shift: subtypes::Shift(Range::from(
            jiff::civil::time(9, 0, 0, 0)..jiff::civil::time(17, 30, 0, 250_000_000),
        )),
        stamp: subtypes::Stamp(Range::from(jiff::civil::date(2024, 1, 1).at(10, 0, 0, 0)..)),
        instant: subtypes::Instant(Range::from(instant..=instant)),
        uuid: subtypes::Id(Range::from(
            ..Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
        )),
    };
    let returned = every.clone().into_active_model().insert(db).await?;
    assert_eq!(returned, every);
    assert_eq!(subtypes::Entity::find().one(db).await?, every);

    let found = measures::Entity::find()
        .filter(measures::Column::Span.eq(FloatRange(Range::Empty)))
        .all(db)
        .await?;
    assert_eq!(
        found.into_iter().map(|model| model.id).collect::<Vec<_>>(),
        vec![2]
    );
    Ok(())
}

/// A value cast to its type renders as an escaped literal inline and as a
/// `text` parameter bound, and the two are one value — the value written.
// [spec:pgorm:def:sql.value.created-range+8/test]
async fn the_bound_and_inline_renderings_agree(db: &DatabaseConnection) -> Result<(), Error> {
    let floats = [
        Range::from(1.5..2.5),
        Range::new(Included(f64::NEG_INFINITY), Excluded(f64::INFINITY)),
        Range::new(Excluded(-0.0), Unbounded),
        Range::from(1e-300..1e300),
        Range::Empty,
    ];
    for range in floats {
        let query = select(FloatRange(range.clone()).into_expr());
        assert_eq!(
            query.build().0,
            "SELECT CAST($1::text AS floatrange)",
            "the text is a parameter"
        );
        let (inline, bound) = both::<FloatRange>(db, &query).await?;
        assert_eq!(inline, bound, "{query}");
        assert_eq!(bound.0, range, "{query}");
    }

    let label = LabelSpan(Range::from("x".to_owned()..));
    let query = select(label.clone().into_expr());
    assert_eq!(
        query.to_string(),
        r#"SELECT CAST('[x,)' AS "We""ird"."Label Span")"#
    );
    let (inline, bound) = both::<LabelSpan>(db, &query).await?;
    assert_eq!((inline, bound), (label.clone(), label));

    let null = select(Expr::val(Value::String(None)).as_range(FloatRange::name()));
    let (inline, bound) = both::<Option<FloatRange>>(db, &null).await?;
    assert_eq!((inline, bound), (None, None));
    Ok(())
}

/// The server's own text for a value reads back as the value written, and
/// for a continuous range whose bounds print alike it is the text written.
// [spec:pgorm:def:sql.value.created-range+8/test]
async fn the_canonical_text_reads_back_as_written(db: &DatabaseConnection) -> Result<(), Error> {
    async fn canonical(db: &DatabaseConnection, expr: SimpleExpr) -> Result<String, Error> {
        let query = select(expr.cast_as_type(TypeName::new(n("text"))));
        let (inline, bound) = both::<String>(db, &query).await?;
        assert_eq!(inline, bound, "{query}");
        Ok(bound)
    }

    let floats = [
        Range::from(1.5..2.5),
        Range::new(Included(f64::NEG_INFINITY), Included(f64::INFINITY)),
        Range::from(1e300..),
        Range::new(Excluded(-0.0), Included(0.1 + 0.2)),
    ];
    for range in floats {
        let text = canonical(db, FloatRange(range.clone()).into_expr()).await?;
        assert_eq!(text.parse::<Range<f64>>().ok(), Some(range), "{text}");
    }
    assert_eq!(
        canonical(db, FloatRange(Range::from(1.5..2.5)).into_expr()).await?,
        "[1.5,2.5)"
    );
    assert_eq!(
        canonical(
            db,
            FloatRange(Range::new(Included(f64::NEG_INFINITY), Unbounded)).into_expr()
        )
        .await?,
        "[-Infinity,)"
    );

    let labels = [
        Range::from("a,b".to_owned().."say \"hi\"".to_owned()),
        Range::new(Included(String::new()), Excluded(" padded ".to_owned())),
        Range::from("(a]".to_owned()..="z\\b".to_owned()),
    ];
    for range in labels {
        let text = canonical(db, LabelSpan(range.clone()).into_expr()).await?;
        assert_eq!(
            text,
            range.to_string(),
            "the server writes the text as pgorm does"
        );
        assert_eq!(text.parse::<Range<String>>().ok(), Some(range), "{text}");
    }
    Ok(())
}

/// A bound holding the range literal's own syntax, the SQL string's, or a
/// placeholder's is written and read back as the text it is, by entity insert
/// and by both renderings of a cast.
// [spec:pgorm:def:sql.value.created-range+8/test]
async fn a_bound_holding_range_syntax_stays_data(db: &DatabaseConnection) -> Result<(), Error> {
    let hostile = [
        "'); DROP TABLE measures; --",
        "\"",
        "\\",
        "\\\"",
        ",",
        ")",
        "]",
        "[",
        "\" , \"",
        "$1",
        "' || current_user || '",
        "empty",
        "NULL",
        "\t\n ",
        "雪",
    ];
    for (id, text) in (10..).zip(hostile) {
        let label = LabelSpan(Range::new(
            Included(text.to_owned()),
            Included(text.to_owned()),
        ));
        let model = measures::Model {
            id,
            span: FloatRange(Range::Empty),
            slot: Slot(Range::Empty),
            label: Some(label.clone()),
        };
        let inserted = model.clone().into_active_model().insert(db).await?;
        assert_eq!(inserted, model, "{text:?}");

        let query = select(label.clone().into_expr());
        let (inline, bound) = both::<LabelSpan>(db, &query).await?;
        assert_eq!((&inline, &bound), (&label, &label), "{query}");
    }
    assert_eq!(
        measures::Entity::find()
            .filter(measures::Column::Id.gte(10))
            .count(db)
            .await?,
        hostile.len() as u64
    );
    Ok(())
}

/// A built-in `int4range` value written to a column of a range type created
/// over `int4` is turned into its text and cast, on both renderings, a
/// pipeline binds one compared with the column through its text too, and so
/// do the column's range operators and its quantified comparisons, an array
/// of ranges written as the array of their texts. Written
/// as the built-in it is, it is refused: there is no cast between two range
/// types.
// [spec:pgorm:def:sql.value.created-range+8/test]
async fn a_builtin_range_value_reaches_a_created_type(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let insert = Query::insert()
        .into_table(measures::Entity)
        .columns([
            measures::Column::Id,
            measures::Column::Span,
            measures::Column::Slot,
        ])
        .values_panic([
            Expr::val(3).into(),
            measures::Column::Span.save_as(Expr::val(FloatRange(Range::Empty))),
            measures::Column::Slot.save_as(Expr::val(Range::from(2..7))),
        ])
        .to_owned();
    db.batch_execute(&insert.to_string()).await?;
    assert_eq!(
        insert.to_string(),
        r#"INSERT INTO "measures" ("id", "span", "slot") VALUES (3, CAST('empty' AS floatrange), CAST('[2,7)' AS slot))"#
    );

    let filter = measures::Entity::find()
        .filter(measures::Column::Slot.eq(Range::from(2..7)))
        .into_query();
    assert!(
        filter
            .to_string()
            .contains(r#""measures"."slot" = CAST('[2,7)' AS slot)"#),
        "{filter}"
    );
    let (sql, values) = filter.build();
    assert_eq!(
        values.0,
        vec![Value::String(Some(Box::new("[2,7)".to_owned())))]
    );
    let inline = (filter.to_string(), Values(Vec::new()))
        .into_model::<measures::Model>()
        .all(db)
        .await?;
    let bound = (sql, values)
        .into_model::<measures::Model>()
        .all(db)
        .await?;
    assert_eq!(inline, bound);
    assert_eq!(
        inline.into_iter().map(|model| model.id).collect::<Vec<_>>(),
        vec![3]
    );

    let piped = {
        use pgorm::pipeline::{ExprOps, Pipeline};
        Pipeline::from(measures::Entity)
            .filter_with(|binder| {
                measures::Column::Slot
                    .eq(binder.bind(Range::from(2..7)))
                    .or(binder.bind(Range::from(1..2)).eq(measures::Column::Slot))
            })
            .select(measures::Column::Id)
    };
    let (sql, _) = piped.clone().into_sql().expect("the pipeline compiles");
    assert_eq!(
        sql,
        "SELECT id FROM measures WHERE slot = CAST($1::int4range::text AS slot) \
         OR CAST($2::int4range::text AS slot) = slot"
    );
    let ids = piped
        .into_tuple::<i32>()
        .expect("the pipeline compiles")
        .all(db)
        .await?;
    assert_eq!(ids, vec![3]);
    let untyped = {
        use pgorm::pipeline::{ExprOps, Pipeline, col};
        Pipeline::from(Name::runtime("measures"))
            .filter_with(|binder| {
                col(Name::runtime("measures"), Name::runtime("slot"))
                    .compares_as(Slot::name())
                    .eq(binder.bind(Range::from(2..7)))
            })
            .select(col(Name::runtime("measures"), Name::runtime("id")))
    };
    let ids = untyped
        .into_tuple::<i32>()
        .expect("the pipeline compiles")
        .all(db)
        .await?;
    assert_eq!(ids, vec![3]);

    let slot = measures::Column::Slot;
    let quantified = measures::Entity::find()
        .filter(slot.eq_any([Range::from(2..7)]))
        .into_query();
    assert!(
        quantified
            .to_string()
            .contains(r#""measures"."slot" = ANY(CAST(ARRAY ['[2,7)'] AS slot[]))"#),
        "{quantified}"
    );
    assert!(
        quantified
            .build()
            .0
            .contains("= ANY(CAST($1::text[] AS slot[]))"),
        "{quantified}"
    );
    for (operator, holds) in [
        (slot.overlaps(Range::from(5..9)), true),
        (slot.includes(Range::from(3..4)), true),
        (slot.contained_by(Range::from(0..10)), true),
        (slot.strictly_left_of(Range::from(8..9)), true),
        (slot.strictly_right_of(Range::from(8..9)), false),
        (slot.adjacent_to(Range::from(7..8)), true),
        (slot.eq_any([Range::from(1..2), Range::from(2..7)]), true),
        (slot.ne_all([Range::from(2..7)]), false),
        (slot.eq_any([Slot(Range::from(2..7))]), true),
        (slot.eq_any(Vec::<Range<i32>>::new()), false),
        (
            slot.compare_any(BinOper::Overlap, vec![Range::from(6..9)]),
            true,
        ),
        (
            slot.compare_all(BinOper::Overlap, vec![Range::from(6..9), Range::from(0..1)]),
            false,
        ),
    ] {
        let found = measures::Entity::find()
            .filter(measures::Column::Id.eq(3))
            .filter(operator)
            .all(db)
            .await?;
        assert_eq!(found.len(), usize::from(holds));
    }

    let builtin = select(Expr::val(Range::from(2..7)).cast_as_type(Slot::name()));
    let inline = (builtin.to_string(), Values(Vec::new()))
        .into_tuple::<Slot>()
        .one(db)
        .await
        .unwrap_err();
    refused_with(&inline, &SqlState::CANNOT_COERCE);
    let bound = builtin
        .build()
        .into_tuple::<Slot>()
        .one(db)
        .await
        .unwrap_err();
    refused_with(&bound, &SqlState::CANNOT_COERCE);
    Ok(())
}

/// A column of an array of a range type the schema created takes its values
/// as the array of their texts cast to the type's array — written, compared
/// with, and bound by a pipeline — an empty array and a NULL element
/// included, where a built-in range array cast as it is is refused.
// [spec:pgorm:def:sql.value.created-range+8/test]
// [spec:pgorm:sem:entity.traits.column.enum-cast+17/test]
// [spec:pgorm:sem:macros.derive.created-range+3/test]    `Vec` of the newtype is a field, read
// element by element through the subtype's codec
async fn created_range_arrays_bind_as_texts(db: &DatabaseConnection) -> Result<(), Error> {
    use pgorm::pgorm_query::ArrayType;

    db.batch_execute(
        &Schema::new()
            .create_table_from_entity(shelves::Entity)?
            .to_string(),
    )
    .await?;
    let shelf = |id, slots: Vec<Slot>, spare| shelves::Model { id, slots, spare };
    for model in [
        shelf(
            1,
            vec![Slot(Range::from(1..5)), Slot(Range::from(7..9))],
            None,
        ),
        shelf(2, Vec::new(), Some(vec![Slot(Range::from(2..3))])),
    ] {
        model.into_active_model().insert(db).await?;
    }
    let mut found = shelves::Entity::find().all(db).await?;
    found.sort_by_key(|model| model.id);
    assert_eq!(
        found,
        vec![
            shelf(
                1,
                vec![Slot(Range::from(1..5)), Slot(Range::from(7..9))],
                None
            ),
            shelf(2, Vec::new(), Some(vec![Slot(Range::from(2..3))])),
        ]
    );

    let filter = shelves::Entity::find()
        .filter(shelves::Column::Slots.eq(vec![Range::from(1..5), Range::from(7..9)]))
        .into_query();
    assert!(
        filter
            .to_string()
            .contains(r#""shelves"."slots" = CAST(ARRAY ['[1,5)','[7,9)'] AS slot[])"#),
        "{filter}"
    );
    assert!(
        filter.build().0.contains("= CAST($1::text[] AS slot[])"),
        "{filter}"
    );
    let ids =
        |models: Vec<shelves::Model>| models.into_iter().map(|model| model.id).collect::<Vec<_>>();
    let null_element = Value::Array(
        ArrayType::String,
        Some(Box::new(vec![Value::String(None), Value::from("[2,3)")])),
    );
    for (predicate, expected) in [
        (
            shelves::Column::Slots.eq(vec![Range::from(1..5), Range::from(7..9)]),
            vec![1],
        ),
        (shelves::Column::Slots.eq(Vec::<Range<i32>>::new()), vec![2]),
        (
            shelves::Column::Slots.includes(vec![Slot(Range::from(7..9))]),
            vec![1],
        ),
        (shelves::Column::Spare.ne(null_element), vec![2]),
    ] {
        let found = shelves::Entity::find().filter(predicate).all(db).await?;
        assert_eq!(ids(found), expected);
    }

    let piped = {
        use pgorm::pipeline::{ExprOps, Pipeline};
        Pipeline::from(shelves::Entity)
            .filter_with(|binder| {
                shelves::Column::Slots.eq(binder.bind(vec![Range::from(1..5), Range::from(7..9)]))
            })
            .select(shelves::Column::Id)
    };
    let (sql, _) = piped.clone().into_sql().expect("the pipeline compiles");
    assert!(
        sql.contains("slots = CAST($1::int4range[]::text AS slot[])"),
        "{sql}"
    );
    let ids = piped
        .into_tuple::<i32>()
        .expect("the pipeline compiles")
        .all(db)
        .await?;
    assert_eq!(ids, vec![1]);

    let builtin = select(Expr::val(vec![Range::from(1..5)]).cast_as_type(Slot::name().array()));
    let refused = (builtin.to_string(), Values(Vec::new()))
        .into_tuple::<String>()
        .one(db)
        .await
        .unwrap_err();
    refused_with(&refused, &SqlState::CANNOT_COERCE);
    Ok(())
}

/// A range type a schema creates has no canonical function — pgorm cannot
/// author the C function one needs — so it is continuous whatever its
/// subtype: a range over `int4` keeps the bounds written, where `int4range`
/// moves them.
// [spec:pgorm:def:sql.value.created-range+8/test]
async fn a_created_range_over_int4_is_continuous(db: &DatabaseConnection) -> Result<(), Error> {
    for range in [Range::from(1..=5), Range::new(Excluded(1), Included(5))] {
        let (inline, bound) = both::<Slot>(db, &select(Slot(range.clone()).into_expr())).await?;
        assert_eq!((inline.0, bound.0), (range.clone(), range));
    }
    let (inline, bound) = both::<Slot>(db, &select(Slot(Range::from(5..5)).into_expr())).await?;
    assert_eq!((inline.0, bound.0), (Range::Empty, Range::Empty));
    Ok(())
}

/// A row decodes by the subtype the server reports: a `floatrange` column
/// read as a range over `int4` is refused, and so is a built-in `int4range`
/// read as `floatrange`'s newtype.
// [spec:pgorm:def:exec.decode.range+3/test]
async fn a_column_reads_only_its_own_subtype(db: &DatabaseConnection) -> Result<(), Error> {
    let query = select(FloatRange(Range::from(1.0..2.0)).into_expr());
    let (sql, values) = query.build();
    assert!((sql, values).into_tuple::<Slot>().one(db).await.is_err());
    assert!(
        ("SELECT int4range(1, 2)", Values(Vec::new()))
            .into_tuple::<FloatRange>()
            .one(db)
            .await
            .is_err()
    );
    assert!(!<FloatRange as TryGetable>::accepts(
        &tokio_postgres::types::Type::INT4_RANGE
    ));
    assert!(<Slot as TryGetable>::accepts(
        &tokio_postgres::types::Type::INT4_RANGE
    ));
    Ok(())
}

/// What the server refuses, it refuses with its own code on both renderings:
/// bounds out of order, a bound the subtype does not read, and one outside
/// its range.
// [spec:pgorm:def:sql.value.created-range+8/test]
async fn the_server_refuses_what_it_cannot_store(db: &DatabaseConnection) -> Result<(), Error> {
    let cases = [
        (
            FloatRange(Range::from(2.0..1.0)).into_expr(),
            SqlState::DATA_EXCEPTION,
        ),
        (
            Expr::val("[1.5,2)").as_range(Slot::name()),
            SqlState::INVALID_TEXT_REPRESENTATION,
        ),
        (
            Expr::val("[1,2").as_range(Slot::name()),
            SqlState::INVALID_TEXT_REPRESENTATION,
        ),
        (
            Expr::val(Range::from(5_000_000_000i64..)).as_range(Slot::name()),
            SqlState::NUMERIC_VALUE_OUT_OF_RANGE,
        ),
    ];
    for (expr, state) in cases {
        let query = select(expr);
        let inline = (query.to_string(), Values(Vec::new()))
            .into_tuple::<Slot>()
            .one(db)
            .await
            .unwrap_err();
        refused_with(&inline, &state);
        let (sql, values) = query.build();
        let bound = (sql, values)
            .into_tuple::<Slot>()
            .one(db)
            .await
            .unwrap_err();
        refused_with(&bound, &state);
    }
    Ok(())
}

/// PostgreSQL keeps no type modifier on a range's subtype: `pg_range`
/// records the type alone, so the range pgorm-query's builder declares over
/// `numeric(10, 2)` ranges over `numeric`, its bounds held to no scale. The
/// DDL bridge refuses the modifier for that reason.
// [spec:pgorm:req:codegen.ddl.unsupported+21/test]
// [spec:pgorm:req:sql.ddl.type-range+2/test]
async fn a_subtype_modifier_is_dropped(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(
        &Type::create(Name::runtime("scaled"))
            .as_range(RangeDefinition::new(ColumnType::Decimal(Some((10, 2)))))
            .to_string(),
    )
    .await?;
    let row = db
        .query_one(
            "SELECT format_type(rngsubtype, NULL), '[1.234,5.678]'::scaled::text \
             FROM pg_range WHERE rngtypid = 'scaled'::regtype",
            &[],
        )
        .await?;
    assert_eq!(
        (row.get::<_, String>(0), row.get::<_, String>(1)),
        ("numeric".to_owned(), "[1.234,5.678]".to_owned())
    );
    Ok(())
}
