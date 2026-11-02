#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, setup::*};
use pgorm::{
    Error, ExpectedColumn, RowIndex, TryGetable, VerifyError, VerifyStatement, entity::prelude::*,
};
use pretty_assertions::assert_eq;

/// A statement whose result columns PostgreSQL describes at prepare time and
/// which returns no rows at all, so a mismatched target decodes into an empty
/// `Vec` without ever touching a value.
const NO_ROWS: &str = r#"SELECT 1 AS "id", 'a'::text AS "name" WHERE false"#;

mod widget {
    use pgorm::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "verify_widget")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Debug, FromQueryResult)]
struct Matching {
    id: i32,
    name: String,
}

#[derive(Debug, FromQueryResult)]
struct WrongType {
    id: String,
    name: String,
}

#[derive(Debug, FromQueryResult)]
struct MissingColumn {
    id: i32,
    identifier: String,
}

#[derive(Debug, FromQueryResult)]
struct Skipping {
    id: i32,
    #[pgorm(skip)]
    absent: i64,
}

#[derive(Debug, FromQueryResult)]
struct Nullable {
    id: Option<i32>,
    name: String,
}

#[derive(Debug, FromQueryResult)]
struct Oids {
    oid: u32,
}

/// A field whose `TryGetable` does not override `accepts`, so verification has
/// nothing to say about its column type.
#[derive(Debug)]
struct Anything(String);

impl pgorm::TryGetable for Anything {
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        index: I,
    ) -> Result<Self, pgorm::TryGetError> {
        Ok(Self(String::try_get_by(res, index)?))
    }
}

#[derive(Debug, FromQueryResult)]
struct Opaque {
    opaque: Anything,
}

#[derive(Debug, PartialModel)]
#[pgorm(entity = widget::Entity)]
struct PartialWidget {
    id: i32,
    #[pgorm(from_col = widget::Column::Name)]
    name: String,
}

/// A target whose decode pgorm cannot see into: it reports no columns, so it
/// cannot be verified.
struct Handwritten {
    id: i32,
}

impl FromQueryResult for Handwritten {
    fn from_query_result(res: &QueryResult, pre: &str) -> Result<Self, Error> {
        Ok(Self {
            id: res.try_get(pre, "id")?,
        })
    }
}

/// A hand-written target that opts back in by reporting its columns.
struct Reflected {
    id: i32,
}

impl FromQueryResult for Reflected {
    fn from_query_result(res: &QueryResult, pre: &str) -> Result<Self, Error> {
        Ok(Self {
            id: res.try_get(pre, "id")?,
        })
    }

    fn expected_columns() -> Option<Vec<ExpectedColumn>> {
        Some(vec![ExpectedColumn::new(
            "id",
            "i32",
            <i32 as pgorm::TryGetable>::accepts,
        )])
    }
}

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("statement_verify_tests").await;
    let mut db = ctx.db.get().await?;

    matching_target_verifies(&db).await?;
    wrong_type_caught_at_zero_rows(&db).await?;
    missing_column_caught_at_zero_rows(&db).await?;
    manual_impl_reports_unreflected(&db).await?;
    manual_impl_can_opt_back_in(&db).await?;
    skipped_field_reads_no_column(&db).await?;
    nullability_is_not_verified(&db).await?;
    accepts_follows_the_decode_path(&db).await?;
    partial_model_target_verifies(&db).await?;
    entity_model_target_verifies(&db).await?;
    parameters_are_left_to_bind(&db).await?;
    invalid_sql_reports_the_server_error(&db).await?;
    verify_inside_transaction(&mut db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:exec.verify/test]    a target whose columns the statement returns
async fn matching_target_verifies(db: &DatabaseConnection) -> Result<(), Error> {
    db.verify::<Matching>(NO_ROWS).await?;

    Ok(())
}

// [spec:pgorm:def:exec.verify/test]    the type mismatch a zero-row query hides
// [spec:pgorm:req:exec.verify.errors/test]
async fn wrong_type_caught_at_zero_rows(db: &DatabaseConnection) -> Result<(), Error> {
    let decoded: Vec<WrongType> = WrongType::find_by_statement(NO_ROWS, vec![])
        .all(db)
        .await?;
    assert!(decoded.is_empty());

    let error = db
        .verify::<WrongType>(NO_ROWS)
        .await
        .expect_err("`id` is INT4, which `String` cannot read");

    let Error::Verify(VerifyError::ColumnType {
        column,
        rust_type,
        pg_type,
        ..
    }) = error
    else {
        panic!("expected a column-type mismatch, got {error}");
    };
    assert_eq!(column, "id");
    assert_eq!(rust_type, "String");
    assert_eq!(pg_type, "int4");

    Ok(())
}

// [spec:pgorm:def:exec.verify/test]    the missing column a zero-row query hides
// [spec:pgorm:req:exec.verify.errors/test]
async fn missing_column_caught_at_zero_rows(db: &DatabaseConnection) -> Result<(), Error> {
    let decoded: Vec<MissingColumn> = MissingColumn::find_by_statement(NO_ROWS, vec![])
        .all(db)
        .await?;
    assert!(decoded.is_empty());

    let error = db
        .verify::<MissingColumn>(NO_ROWS)
        .await
        .expect_err("the statement returns no `identifier` column");

    let Error::Verify(VerifyError::ColumnMissing {
        column, returned, ..
    }) = error
    else {
        panic!("expected a missing column, got {error}");
    };
    assert_eq!(column, "identifier");
    assert_eq!(returned, "id, name");

    Ok(())
}

// [spec:pgorm:req:exec.verify.manual+1/test]
async fn manual_impl_reports_unreflected(db: &DatabaseConnection) -> Result<(), Error> {
    let error = db
        .verify::<Handwritten>(NO_ROWS)
        .await
        .expect_err("a hand-written impl reports no columns");

    let Error::Verify(VerifyError::Unreflected { target }) = error else {
        panic!("expected an unreflected target, got {error}");
    };
    assert!(target.ends_with("Handwritten"));

    Ok(())
}

// [spec:pgorm:req:exec.verify.manual+1/test]    overriding the hook opts back in
async fn manual_impl_can_opt_back_in(db: &DatabaseConnection) -> Result<(), Error> {
    db.verify::<Reflected>(NO_ROWS).await?;

    Ok(())
}

// [spec:pgorm:sem:macros.derive.from-query-result+4/test]    `skip` reads no column
async fn skipped_field_reads_no_column(db: &DatabaseConnection) -> Result<(), Error> {
    let columns = Skipping::expected_columns().expect("the derive reports columns");
    assert_eq!(columns.len(), 1);
    assert_eq!(columns[0].name(), "id");

    db.verify::<Skipping>(NO_ROWS).await?;

    Ok(())
}

// [spec:pgorm:req:exec.verify.limits+1/test]    nullability is outside what is checked
// [spec:pgorm:sem:exec.verify.accepts+1/test]    `Option<T>` accepts what `T` accepts
async fn nullability_is_not_verified(db: &DatabaseConnection) -> Result<(), Error> {
    db.verify::<Nullable>(NO_ROWS).await?;
    db.verify::<Matching>(r#"SELECT NULL::int4 AS "id", NULL::text AS "name""#)
        .await?;

    Ok(())
}

// [spec:pgorm:sem:exec.verify.accepts+1/test]    `u32` reads `OID`, and a custom
// `TryGetable` that does not override `accepts` takes any column type
async fn accepts_follows_the_decode_path(db: &DatabaseConnection) -> Result<(), Error> {
    db.verify::<Oids>(r#"SELECT 'pg_class'::regclass::oid AS "oid""#)
        .await?;

    let error = db
        .verify::<Oids>(r#"SELECT 1 AS "oid""#)
        .await
        .expect_err("`oid` is INT4, which `u32` reads as OID cannot take");
    assert!(matches!(
        error,
        Error::Verify(VerifyError::ColumnType { .. })
    ));

    db.verify::<Opaque>(r#"SELECT 1 AS "opaque""#).await?;
    db.verify::<Opaque>(r#"SELECT 'a'::text AS "opaque""#)
        .await?;

    Ok(())
}

// [spec:pgorm:def:exec.verify/test]    a partial model derives the same hook
async fn partial_model_target_verifies(db: &DatabaseConnection) -> Result<(), Error> {
    db.verify::<PartialWidget>(NO_ROWS).await?;

    Ok(())
}

// [spec:pgorm:sem:macros.derive.model+5/test]    an entity model reports its columns
async fn entity_model_target_verifies(db: &DatabaseConnection) -> Result<(), Error> {
    db.verify::<widget::Model>(NO_ROWS).await?;

    let error = db
        .verify::<widget::Model>(r#"SELECT 1 AS "id", 2 AS "name" WHERE false"#)
        .await
        .expect_err("`name` is INT4, which `String` cannot read");

    let Error::Verify(VerifyError::ColumnType {
        column, rust_type, ..
    }) = error
    else {
        panic!("expected a column-type mismatch, got {error}");
    };
    assert_eq!(column, "name");
    assert_eq!(rust_type, "String");

    Ok(())
}

// [spec:pgorm:req:exec.verify.limits+1/test]    parameters are left to Bind, which
// refuses a mismatch on every execution rather than only once rows exist
async fn parameters_are_left_to_bind(db: &DatabaseConnection) -> Result<(), Error> {
    const PARAMETERISED: &str = r#"SELECT $1::int4 AS "id", 'a'::text AS "name" WHERE false"#;

    db.verify::<Matching>(PARAMETERISED).await?;

    let error = db
        .query_all(PARAMETERISED, &[])
        .await
        .expect_err("the statement requires one parameter");
    assert!(matches!(error, Error::Postgres(_)), "got {error}");

    Ok(())
}

// [spec:pgorm:req:exec.verify.limits+1/test]    the server refuses the statement first
async fn invalid_sql_reports_the_server_error(db: &DatabaseConnection) -> Result<(), Error> {
    let error = db
        .verify::<Matching>(r#"SELECT "id" FROM "no_such_table""#)
        .await
        .expect_err("the table does not exist");

    assert!(matches!(error, Error::Postgres(_)), "got {error}");

    Ok(())
}

// [spec:pgorm:def:exec.verify/test]    an open transaction verifies on its own client
async fn verify_inside_transaction(db: &mut DatabaseConnection) -> Result<(), Error> {
    let outcome = db
        .transaction(async |tx| {
            tx.verify::<Matching>(NO_ROWS).await?;
            let error = tx
                .verify::<WrongType>(NO_ROWS)
                .await
                .expect_err("`id` is INT4, which `String` cannot read");
            assert!(matches!(
                error,
                Error::Verify(VerifyError::ColumnType { .. })
            ));

            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);

    Ok(())
}
