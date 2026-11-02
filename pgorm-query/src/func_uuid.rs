//! PostgreSQL 18's UUID generators and the two functions that read a UUID
//! back: its version, and the instant a time-ordered one was minted at.

use super::*;

impl Func {
    /// Call `UUIDV4`: a random, version 4 UUID — PostgreSQL 18's name for what
    /// [`gen_random_uuid`](Self::gen_random_uuid) has always returned.
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// assert_eq!(
    ///     Query::select().expr(Func::uuidv4()).to_string(),
    ///     r#"SELECT UUIDV4()"#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.func+12]
    pub fn uuidv4() -> FunctionCall {
        FunctionCall::new(Function::UuidV4)
    }

    /// Call `UUIDV7`: a time-ordered, version 7 UUID, whose leading bits are
    /// the Unix time in milliseconds it was minted at.
    ///
    /// Values minted later sort later, so a key filled this way indexes like
    /// a sequence — new rows land at the right edge of the B-tree — while
    /// still being unguessable and mintable without the database. Within one
    /// session PostgreSQL also orders values minted in the same millisecond.
    /// As a column default it fills the key whenever an insert leaves it out:
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(ColumnDef::new(Glyph::Id).uuid().default(Func::uuidv7()))
    ///         .primary_key(Glyph::Id)
    ///         .to_string(),
    ///     r#"CREATE TABLE "glyph" ( "id" uuid DEFAULT UUIDV7(), PRIMARY KEY ("id") )"#
    /// );
    /// ```
    ///
    /// An entity says the same with `#[pgorm(default_expr = Func::uuidv7())]`.
    // [spec:pgorm:def:sql.ast.func+12]
    pub fn uuidv7() -> FunctionCall {
        FunctionCall::new(Function::UuidV7)
    }

    /// Call `UUIDV7(shift)`: a version 7 UUID whose embedded time is moved by
    /// `shift`, an `interval` — what a back-filled row minted for an earlier
    /// instant carries.
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::uuidv7_shifted(Expr::val("-1 hour").cast_as(Name::runtime("interval"))))
    ///         .build(),
    ///     (
    ///         r#"SELECT UUIDV7(CAST($1::text AS interval))"#.to_owned(),
    ///         Values(vec!["-1 hour".into()])
    ///     )
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.func+12]
    pub fn uuidv7_shifted<T>(shift: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::UuidV7).arg(shift)
    }

    /// Call `UUID_EXTRACT_TIMESTAMP`: the instant a version 7 (or version 1)
    /// UUID was minted at, a `timestamptz` at millisecond precision, and
    /// `NULL` for any other version — a version 4 UUID carries no time.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .expr(Func::uuid_extract_timestamp(Expr::col(Glyph::Id)))
    ///         .from(Glyph::Table)
    ///         .to_string(),
    ///     r#"SELECT UUID_EXTRACT_TIMESTAMP("id") FROM "glyph""#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.func+12]
    pub fn uuid_extract_timestamp<T>(uuid: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::UuidExtractTimestamp).arg(uuid)
    }

    /// Call `UUID_EXTRACT_VERSION`: a UUID's version number, a `smallint`, and
    /// `NULL` for a UUID that is not of the RFC 9562 variant.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .column(Glyph::Id)
    ///         .from(Glyph::Table)
    ///         .and_where(Expr::expr(Func::uuid_extract_version(Expr::col(Glyph::Id))).eq(7))
    ///         .to_string(),
    ///     r#"SELECT "id" FROM "glyph" WHERE UUID_EXTRACT_VERSION("id") = 7"#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.func+12]
    pub fn uuid_extract_version<T>(uuid: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::UuidExtractVersion).arg(uuid)
    }
}
