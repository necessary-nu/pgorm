#![allow(unused_imports, dead_code)]

//! Values of multiranges a schema created, against a live PostgreSQL server.
//!
//! tokio-postgres reports such a multirange as a simple type with no subtype,
//! so its binary form cannot be read or written safely. A value travels as its
//! text both ways: written as the text cast to the multirange by name, and
//! read through the column's `select_as`, which casts it to `text` as an
//! enum's does. What only a server settles is that the text written is the
//! value read, that a `NULL` stays `NULL` on the way out, that the server's
//! merging and ordering read back, that a bound holding the literal's syntax
//! stays data, that every reader honours the read cast, and what the server
//! refuses.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    ColumnType, Expr, Name, Query, SimpleExpr, TypeName, Value, Values,
    extension::{RangeDefinition, Type},
};
use pgorm::{
    ConnectionTrait, CreatedRange, DecodeRaw, QueryTrait, Schema, TryGetable, entity::prelude::*,
};
use pretty_assertions::assert_eq;
use std::ops::Bound::{Excluded, Included, Unbounded};
use tokio_postgres::error::SqlState;

mod spans {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, CreatedRange)]
    #[pgorm(multirange_name = "floatmultirange")]
    pub struct FloatSpans(pub Multirange<f64>);

    /// A multirange named apart from its range, with a name and a schema
    /// that need quoting.
    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(multirange_name = "Slot Set", schema_name = "We\"ird")]
    pub struct Slots(pub Multirange<i32>);

    #[derive(Clone, Debug, PartialEq, Eq, CreatedRange)]
    #[pgorm(multirange_name = "textmultirange")]
    pub struct Labels(pub Multirange<String>);

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "spans")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub spans: FloatSpans,
        pub slots: Option<Slots>,
        pub labels: Labels,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

use spans::{FloatSpans, Labels, Slots};

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("created_multirange_tests").await;
    let db = ctx.db.get().await?;

    create_the_types(&db).await?;
    the_table_names_each_multirange(&db).await?;
    a_multirange_round_trips_through_an_entity(&db).await?;
    every_reader_takes_the_text_cast(&db).await?;
    the_server_merges_what_is_written(&db).await?;
    the_bound_and_inline_renderings_agree(&db).await?;
    a_bound_holding_range_syntax_stays_data(&db).await?;
    a_builtin_multirange_reaches_a_created_one(&db).await?;
    the_server_refuses_what_it_cannot_store(&db).await?;

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

fn multirange<T>(ranges: impl IntoIterator<Item = Range<T>>) -> Multirange<T> {
    ranges.into_iter().collect()
}

/// A multirange expression read back as its text, inline and bound, decoded
/// the way a column of the type is.
async fn both<T>(db: &DatabaseConnection, expr: SimpleExpr) -> Result<(T, T), Error>
where
    T: TryGetable,
{
    let query = Query::select()
        .expr(expr.cast_as_type(TypeName::new(n("text"))))
        .to_owned();
    let inline: T = (query.to_string(), Values(Vec::new()))
        .into_tuple()
        .one(db)
        .await?;
    let (sql, values) = query.build();
    let bound: T = (sql, values).into_tuple().one(db).await?;
    Ok((inline, bound))
}

async fn create_the_types(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute("CREATE SCHEMA \"We\"\"ird\"").await?;
    for statement in [
        Type::create(n("floatrange"))
            .as_range(RangeDefinition::new(ColumnType::Double))
            .to_string(),
        Type::create(n("slot"))
            .as_range(
                RangeDefinition::new(ColumnType::Integer)
                    .multirange_type_name((n("We\"ird"), n("Slot Set"))),
            )
            .to_string(),
        Type::create(n("textrange"))
            .as_range(RangeDefinition::new(ColumnType::Text).collation(n("C")))
            .to_string(),
    ] {
        db.batch_execute(&statement).await?;
    }
    Ok(())
}

/// A table built from the entity names each multirange in full, quoted where
/// the name needs it, and the server takes it.
// [spec:pgorm:def:sql.value.created-range+5/test]
// [spec:pgorm:sem:macros.derive.created-range+2/test]
async fn the_table_names_each_multirange(db: &DatabaseConnection) -> Result<(), Error> {
    let table = Schema::new().create_table_from_entity(spans::Entity)?;
    let sql = table.to_string();
    assert!(
        sql.contains(r#""spans" floatmultirange NOT NULL"#)
            && sql.contains(r#""slots" "We""ird"."Slot Set""#),
        "{sql}"
    );
    create_table_without_asserts(db, &table).await?;
    assert_eq!(
        <Slots as pgorm::pgorm_query::ValueType>::column_type(),
        ColumnType::CreatedMultirange {
            name: n("Slot Set"),
            schema: Some(n("We\"ird")),
            subtype: std::sync::Arc::new(ColumnType::Integer),
        }
    );
    Ok(())
}

fn written(id: i32) -> spans::Model {
    spans::Model {
        id,
        spans: FloatSpans(multirange([
            Range::new(Unbounded, Excluded(-1.5)),
            Range::from(0.25..1e300),
        ])),
        slots: Some(Slots(multirange([
            Range::from(1..=5),
            Range::new(Excluded(7), Included(9)),
        ]))),
        labels: Labels(multirange([
            Range::from("a,b".to_owned().."c\"d".to_owned()),
            Range::new(Included("x y".to_owned()), Unbounded),
        ])),
    }
}

/// A model is written by an entity insert, its text bound and cast by name,
/// and read back through the column's text cast; a `NULL` multirange stays
/// `NULL` and the empty one stays empty.
// [spec:pgorm:def:sql.value.created-range+5/test]
// [spec:pgorm:sem:entity.traits.column.enum-cast+13/test]
async fn a_multirange_round_trips_through_an_entity(db: &DatabaseConnection) -> Result<(), Error> {
    let first = written(1);
    assert_eq!(first.clone().into_active_model().insert(db).await?, first);
    let second = spans::Model {
        id: 2,
        spans: FloatSpans(Multirange::default()),
        slots: None,
        labels: Labels(Multirange::default()),
    };
    assert_eq!(second.clone().into_active_model().insert(db).await?, second);
    assert_eq!(
        spans::Entity::find()
            .order_by_asc(spans::Column::Id)
            .all(db)
            .await?,
        vec![first, second]
    );
    let found = spans::Entity::find()
        .filter(spans::Column::Spans.eq(FloatSpans(Multirange::default())))
        .all(db)
        .await?;
    assert_eq!(
        found.into_iter().map(|model| model.id).collect::<Vec<_>>(),
        vec![2]
    );
    Ok(())
}

/// `find`, the graph and the pipeline all read through the column's cast to
/// `text`; without it the driver could not read the column at all.
// [spec:pgorm:sem:entity.traits.column.enum-cast+13/test]
async fn every_reader_takes_the_text_cast(db: &DatabaseConnection) -> Result<(), Error> {
    let first = written(1);
    let found = spans::Entity::find_by_id(1).one(db).await?;
    assert_eq!(found, first);
    let graphed = spans::Entity::graph()
        .filter(spans::Column::Id.eq(1))
        .all(db)
        .await?;
    assert_eq!(graphed, vec![first.clone()]);
    let piped = pgorm::pipeline::Pipeline::from(spans::Entity)
        .select_sources(spans::Entity)
        .all(db)
        .await?;
    assert!(piped.contains(&Some(first)), "{piped:?}");
    let raw = ("SELECT spans FROM spans WHERE id = 1", Values(Vec::new()))
        .into_tuple::<FloatSpans>()
        .one(db)
        .await;
    assert!(raw.is_err(), "the bare column decoded as {raw:?}");
    Ok(())
}

/// The server stores a multirange sorted and merged, with the empty ranges
/// dropped, and the merged value is what reads back.
// [spec:pgorm:def:sql.value.created-range+5/test]
async fn the_server_merges_what_is_written(db: &DatabaseConnection) -> Result<(), Error> {
    let written = FloatSpans(multirange([
        Range::from(5.0..8.0),
        Range::Empty,
        Range::from(1.0..3.0),
        Range::from(2.0..4.0),
    ]));
    let (inline, bound) = both::<FloatSpans>(db, written.into_expr()).await?;
    let merged = FloatSpans(multirange([Range::from(1.0..4.0), Range::from(5.0..8.0)]));
    assert_eq!((inline, bound), (merged.clone(), merged));
    let (inline, bound) = both::<Slots>(
        db,
        Slots(multirange([Range::from(1..=5), Range::from(6..7)])).into_expr(),
    )
    .await?;
    assert_eq!(
        (inline.0, bound.0),
        (
            multirange([Range::from(1..=5), Range::from(6..7)]),
            multirange([Range::from(1..=5), Range::from(6..7)])
        ),
        "a created range has no canonical function, so [1,5] and [6,7) do not merge"
    );
    Ok(())
}

/// A value cast to its type renders as an escaped literal inline and as a
/// `text` parameter bound, and the two are the value written.
// [spec:pgorm:def:sql.value.created-range+5/test]
async fn the_bound_and_inline_renderings_agree(db: &DatabaseConnection) -> Result<(), Error> {
    let slots = Slots(multirange([Range::from(1..=5)]));
    let query = Query::select().expr(slots.clone().into_expr()).to_owned();
    assert_eq!(
        query.to_string(),
        r#"SELECT CAST('{[1,5]}' AS "We""ird"."Slot Set")"#
    );
    assert_eq!(
        query.build().0,
        r#"SELECT CAST($1::text AS "We""ird"."Slot Set")"#
    );
    let (inline, bound) = both::<Slots>(db, slots.clone().into_expr()).await?;
    assert_eq!((inline, bound), (slots.clone(), slots));
    let null = Expr::val(Value::String(None)).as_range(Slots::name());
    let (inline, bound) = both::<Option<Slots>>(db, null).await?;
    assert_eq!((inline, bound), (None, None));
    Ok(())
}

/// A bound holding the literal's syntax, the SQL string's, or a
/// placeholder's is written and read back as the text it is.
// [spec:pgorm:def:sql.value.created-range+5/test]
async fn a_bound_holding_range_syntax_stays_data(db: &DatabaseConnection) -> Result<(), Error> {
    let hostile = [
        "'); DROP TABLE spans; --",
        "\"",
        "\\",
        "{",
        "}",
        "},{",
        ",",
        "]",
        "$1",
        "empty",
        "\t ",
    ];
    for (id, text) in (10..).zip(hostile) {
        let labels = Labels(multirange([Range::new(
            Included(text.to_owned()),
            Included(text.to_owned()),
        )]));
        let model = spans::Model {
            id,
            spans: FloatSpans(Multirange::default()),
            slots: None,
            labels: labels.clone(),
        };
        assert_eq!(model.clone().into_active_model().insert(db).await?, model);
        let (inline, bound) = both::<Labels>(db, labels.clone().into_expr()).await?;
        assert_eq!((&inline, &bound), (&labels, &labels), "{text:?}");
    }
    Ok(())
}

/// A built-in `int4multirange` value written to a column of a multirange
/// created over `int4` is turned into its text and cast, on both renderings;
/// cast as the built-in it is, it is refused.
// [spec:pgorm:def:sql.value.created-range+5/test]
async fn a_builtin_multirange_reaches_a_created_one(db: &DatabaseConnection) -> Result<(), Error> {
    let builtin = multirange([Range::from(2..7)]);
    let insert = Query::insert()
        .into_table(spans::Entity)
        .columns([
            spans::Column::Id,
            spans::Column::Spans,
            spans::Column::Slots,
            spans::Column::Labels,
        ])
        .values_panic([
            Expr::val(3).into(),
            spans::Column::Spans.save_as(Expr::val(FloatSpans(Multirange::default()))),
            spans::Column::Slots.save_as(Expr::val(builtin.clone())),
            spans::Column::Labels.save_as(Expr::val(Labels(Multirange::default()))),
        ])
        .to_owned();
    db.batch_execute(&insert.to_string()).await?;
    let filter = spans::Entity::find()
        .filter(spans::Column::Slots.eq(builtin.clone()))
        .into_query();
    assert!(
        filter
            .to_string()
            .contains(r#""spans"."slots" = CAST('{[2,7)}' AS "We""ird"."Slot Set")"#),
        "{filter}"
    );
    let (sql, values) = filter.build();
    let inline = (filter.to_string(), Values(Vec::new()))
        .into_model::<spans::Model>()
        .all(db)
        .await?;
    let bound = (sql, values).into_model::<spans::Model>().all(db).await?;
    assert_eq!(inline, bound);
    assert_eq!(
        inline.into_iter().map(|model| model.id).collect::<Vec<_>>(),
        vec![3]
    );

    let cast = Query::select()
        .expr(Expr::val(builtin).cast_as_type(Slots::name()))
        .to_owned();
    let refused = (cast.to_string(), Values(Vec::new()))
        .into_tuple::<String>()
        .one(db)
        .await
        .unwrap_err();
    refused_with(&refused, &SqlState::CANNOT_COERCE);
    Ok(())
}

/// What the server refuses, it refuses with its own code on both renderings:
/// a range with bounds out of order, malformed text, and text that is a range
/// rather than a multirange. Text that reads as no multirange of the subtype
/// is refused on the way out.
// [spec:pgorm:def:sql.value.created-range+5/test]
async fn the_server_refuses_what_it_cannot_store(db: &DatabaseConnection) -> Result<(), Error> {
    let cases = [
        (
            FloatSpans(multirange([Range::from(2.0..1.0)])).into_expr(),
            SqlState::DATA_EXCEPTION,
        ),
        (
            Expr::val("{[1,2)").as_range(FloatSpans::name()),
            SqlState::INVALID_TEXT_REPRESENTATION,
        ),
        (
            Expr::val("[1,2)").as_range(FloatSpans::name()),
            SqlState::INVALID_TEXT_REPRESENTATION,
        ),
    ];
    for (expr, state) in cases {
        let query = Query::select()
            .expr(expr.cast_as_type(TypeName::new(n("text"))))
            .to_owned();
        let inline = (query.to_string(), Values(Vec::new()))
            .into_tuple::<String>()
            .one(db)
            .await
            .unwrap_err();
        refused_with(&inline, &state);
        let (sql, values) = query.build();
        let bound = (sql, values)
            .into_tuple::<String>()
            .one(db)
            .await
            .unwrap_err();
        refused_with(&bound, &state);
    }
    let unread = ("SELECT '{[a,b)}'::text", Values(Vec::new()))
        .into_tuple::<FloatSpans>()
        .one(db)
        .await
        .unwrap_err();
    assert!(
        unread.to_string().contains("is not a value of FloatSpans"),
        "{unread}"
    );
    Ok(())
}
