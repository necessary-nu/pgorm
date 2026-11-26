#![allow(unused_imports, dead_code)]

//! `CREATE TYPE ... AS RANGE` against a live PostgreSQL server.
//!
//! The render tests in pgorm-query hold the statement to the parser's
//! `CreateRangeStmt`. What only a server settles is what it made: a range type
//! over the subtype written, ordered by the operator class and collation
//! given, measured by the difference function given, with a multirange of the
//! name given or the one PostgreSQL derives; which options it refuses and with
//! what code; and that the existing `DROP TYPE` and `RENAME TO` reach a range
//! as they reach the other kinds.

use crate::common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    ColumnDef, ColumnType, Name, RangeType, Table,
    extension::{RangeDefinition, Type},
};
use pgorm::{Connection, ConnectionMut, entity::prelude::*};
use pgorm_driver::error::SqlState;
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("range_type_tests").await;
    let db = ctx.db.get().await?;

    the_options_are_the_ones_written(&db).await?;
    the_server_judges_the_options(&db).await?;
    drop_and_rename_reach_a_range(&db).await?;
    a_column_takes_a_builtin_range_type(&db).await?;

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

/// A range type as `pg_range` holds it: subtype, operator class, collation
/// (`None` for the subtype's own), difference function (`None` for none),
/// and the qualified name of its multirange.
#[derive(Debug, PartialEq)]
struct Catalogued {
    subtype: String,
    opclass: String,
    collation: Option<String>,
    subtype_diff: Option<String>,
    multirange: String,
}

async fn catalogued(
    db: &DatabaseConnection,
    schema: &str,
    name: &str,
) -> Result<Catalogued, Error> {
    let row = db
        .query_one(
            "SELECT format_type(r.rngsubtype, NULL), o.opcname::text, \
             CASE WHEN r.rngcollation <> 0 THEN k.collname::text END, \
             CASE WHEN r.rngsubdiff <> 0 THEN r.rngsubdiff::regproc::text END, \
             mn.nspname || '.' || m.typname \
             FROM pg_range r JOIN pg_type t ON t.oid = r.rngtypid \
             JOIN pg_namespace tn ON tn.oid = t.typnamespace \
             JOIN pg_opclass o ON o.oid = r.rngsubopc \
             LEFT JOIN pg_collation k ON k.oid = r.rngcollation \
             JOIN pg_type m ON m.oid = r.rngmultitypid \
             JOIN pg_namespace mn ON mn.oid = m.typnamespace \
             WHERE tn.nspname = $1 AND t.typname = $2",
            &[&schema, &name],
        )
        .await?;
    Ok(Catalogued {
        subtype: row.get(0),
        opclass: row.get(1),
        collation: row.get(2),
        subtype_diff: row.get(3),
        multirange: row.get(4),
    })
}

/// Each option reaches the catalogue: the subtype as a column's type would
/// have it, a non-default operator class and collation, the difference
/// function, and the multirange's name — given, schema and all, or derived
/// by PostgreSQL from the range's when it is not.
// [spec:pgorm:req:sql.ddl.type-range+3/test]
async fn the_options_are_the_ones_written(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute("CREATE SCHEMA app; CREATE SCHEMA spans")
        .await?;
    db.batch_execute(
        &Type::create(n("floatrange"))
            .as_range(RangeDefinition::new(ColumnType::Double).subtype_diff(n("float8mi")))
            .to_string(),
    )
    .await?;
    assert_eq!(
        catalogued(db, "public", "floatrange").await?,
        Catalogued {
            subtype: "double precision".to_owned(),
            opclass: "float8_ops".to_owned(),
            collation: None,
            subtype_diff: Some("float8mi".to_owned()),
            multirange: "public.floatmultirange".to_owned(),
        }
    );

    db.batch_execute(
        &Type::create((n("app"), n("Text Span")))
            .as_range(
                RangeDefinition::new(ColumnType::Text)
                    .subtype_opclass(n("text_pattern_ops"))
                    .collation(n("C"))
                    .multirange_type_name((n("spans"), n("Text Spans"))),
            )
            .to_string(),
    )
    .await?;
    assert_eq!(
        catalogued(db, "app", "Text Span").await?,
        Catalogued {
            subtype: "text".to_owned(),
            opclass: "text_pattern_ops".to_owned(),
            collation: Some("C".to_owned()),
            subtype_diff: None,
            multirange: "spans.Text Spans".to_owned(),
        }
    );

    // A type modifier on the subtype is dropped: the range is over `varchar`.
    db.batch_execute(
        &Type::create(n("labels"))
            .as_range(RangeDefinition::new(ColumnType::String(
                pgorm::pgorm_query::StringLen::N(10),
            )))
            .to_string(),
    )
    .await?;
    assert_eq!(
        catalogued(db, "public", "labels").await?.subtype,
        "character varying"
    );
    Ok(())
}

/// What PostgreSQL refuses in a range type's options, it refuses with its own
/// code: a collation on a subtype that has none, a difference function or an
/// operator class that does not fit the subtype, and one that does not exist.
// [spec:pgorm:req:sql.ddl.type-range+3/test]
async fn the_server_judges_the_options(db: &DatabaseConnection) -> Result<(), Error> {
    let refused =
        |definition: RangeDefinition| Type::create(n("refused")).as_range(definition).to_string();
    let cases = [
        (
            refused(RangeDefinition::new(ColumnType::Integer).collation(n("C"))),
            SqlState::WRONG_OBJECT_TYPE,
        ),
        (
            refused(RangeDefinition::new(ColumnType::Double).subtype_diff(n("int4mi"))),
            SqlState::UNDEFINED_FUNCTION,
        ),
        (
            refused(RangeDefinition::new(ColumnType::Double).subtype_opclass(n("int4_ops"))),
            SqlState::DATATYPE_MISMATCH,
        ),
        (
            refused(RangeDefinition::new(ColumnType::Double).subtype_opclass(n("no_such_ops"))),
            SqlState::UNDEFINED_OBJECT,
        ),
        (
            refused(RangeDefinition::new(ColumnType::named("no_such_type"))),
            SqlState::UNDEFINED_OBJECT,
        ),
    ];
    for (sql, state) in cases {
        let error = db.batch_execute(&sql).await.unwrap_err();
        refused_with(&error, &state);
    }
    Ok(())
}

/// `DROP TYPE` and `ALTER TYPE ... RENAME TO` reach a range type: a column of
/// the type holds a plain drop back, `CASCADE` takes the column's dependency
/// with it, and the multirange goes with its range.
// [spec:pgorm:req:sql.ddl.type-range+3/test]
// [spec:pgorm:req:sql.ddl.type-alter-drop+7/test]
async fn drop_and_rename_reach_a_range(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(
        &Type::create(n("slot"))
            .as_range(RangeDefinition::new(ColumnType::Integer))
            .to_string(),
    )
    .await?;
    db.batch_execute(&Type::alter(n("slot")).rename_to(n("shift")).to_string())
        .await?;
    db.batch_execute(
        &Table::create(n("rota"))
            .col(ColumnDef::new(n("id")).integer())
            .col(ColumnDef::new_with_type(
                n("span"),
                ColumnType::named("shift"),
            ))
            .to_string(),
    )
    .await?;
    let error = db
        .batch_execute(&Type::drop(n("shift")).restrict().to_string())
        .await
        .unwrap_err();
    refused_with(&error, &SqlState::DEPENDENT_OBJECTS_STILL_EXIST);
    db.batch_execute(&Type::drop(n("shift")).cascade().to_string())
        .await?;
    let left = db
        .query_one(
            "SELECT count(*) FROM pg_type WHERE typname IN ('shift', 'slot_multirange')",
            &[],
        )
        .await?;
    assert_eq!(left.get::<_, i64>(0), 0);
    Ok(())
}

/// A table column takes each built-in range and multirange type by its
/// catalogue name.
// [spec:pgorm:def:sql.value.range+5/test]
async fn a_column_takes_a_builtin_range_type(db: &DatabaseConnection) -> Result<(), Error> {
    let kinds = [
        RangeType::Int4,
        RangeType::Int8,
        RangeType::Numeric,
        RangeType::Date,
        RangeType::Timestamp,
        RangeType::TimestampTz,
    ];
    let mut table = Table::create(n("kinds"));
    for (i, kind) in kinds.into_iter().enumerate() {
        table
            .col(ColumnDef::new_with_type(
                n(&format!("r{i}")),
                ColumnType::Range(kind),
            ))
            .col(ColumnDef::new_with_type(
                n(&format!("m{i}")),
                ColumnType::Multirange(kind),
            ));
    }
    db.batch_execute(&table.to_string()).await?;
    let declared: Vec<String> = db
        .query_all(
            "SELECT format_type(atttypid, atttypmod) FROM pg_attribute \
             WHERE attrelid = 'kinds'::regclass AND attnum > 0 ORDER BY attnum",
            &[],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(
        declared,
        [
            "int4range",
            "int4multirange",
            "int8range",
            "int8multirange",
            "numrange",
            "nummultirange",
            "daterange",
            "datemultirange",
            "tsrange",
            "tsmultirange",
            "tstzrange",
            "tstzmultirange",
        ]
    );
    Ok(())
}
