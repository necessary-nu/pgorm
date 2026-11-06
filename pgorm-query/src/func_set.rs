//! The set-returning functions a [`TableFunction`](crate::TableFunction)
//! reads as a relation: a series of numbers or instants, an array's
//! elements as rows, and a JSON object or array of objects as `record`s.
//!
//! The first two are overloaded on their argument types, which a bare
//! placeholder lacks: `GENERATE_SERIES($1, $2)` and `UNNEST($1)` are
//! ambiguous (`42725`). A bound argument therefore carries its own type,
//! `GENERATE_SERIES($1::int4, $2::int4)` and `UNNEST($1::int4[])`
//! (`[spec:pgorm:sem:sql.render.placeholder-typing+2]`), and a cast chooses
//! another overload. The JSON ones each take one type, `json` or `jsonb`,
//! which a placeholder there is typed as.

use super::*;

impl Func {
    /// Call `GENERATE_SERIES(start, stop)`: every value from `start` to
    /// `stop` inclusive, a step of one apart — integers or numerics. A
    /// series of instants needs its interval step, which
    /// [`generate_series_step`](Self::generate_series_step) takes.
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .column(Asterisk)
    ///         .from(TableFunction::new(Func::generate_series(1, 3)).alias(Name::runtime("n")))
    ///         .to_string(),
    ///     r#"SELECT * FROM GENERATE_SERIES(1, 3) AS "n""#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.select.table-function+3]
    pub fn generate_series<S, E>(start: S, stop: E) -> FunctionCall
    where
        S: Into<SimpleExpr>,
        E: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::GenerateSeries)
            .arg(start)
            .arg(stop)
    }

    /// Call `GENERATE_SERIES(start, stop, step)`: the values from `start`
    /// towards `stop`, `step` apart — a negative step counts down, and over
    /// timestamps the step is an interval. A zero step is PostgreSQL's
    /// refusal (`22023`).
    // [spec:pgorm:req:sql.ast.select.table-function+3]
    pub fn generate_series_step<S, E, P>(start: S, stop: E, step: P) -> FunctionCall
    where
        S: Into<SimpleExpr>,
        E: Into<SimpleExpr>,
        P: Into<SimpleExpr>,
    {
        Self::generate_series(start, stop).arg(step)
    }

    /// Call `UNNEST(array)`: the array's elements as rows, in order — which
    /// `WITH ORDINALITY` numbers, giving each its position.
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// assert_eq!(
    ///     Query::select()
    ///         .column(Asterisk)
    ///         .from(
    ///             TableFunction::new(Func::unnest(Expr::col(Name::runtime("tags"))))
    ///                 .with_ordinality()
    ///                 .alias(Name::runtime("t")),
    ///         )
    ///         .to_string(),
    ///     r#"SELECT * FROM UNNEST("tags") WITH ORDINALITY AS "t""#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.select.table-function+3]
    pub fn unnest<T>(array: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::Unnest).arg(array)
    }

    /// Call `JSON_TO_RECORD(json)`: a JSON object as one `record`, its
    /// columns those the [column definition
    /// list](crate::TableFunction::column_definition) names, each read from
    /// the key of its name, NULL where the object has none.
    // [spec:pgorm:req:sql.ast.select.column-definitions]
    pub fn json_to_record<T>(json: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::JsonToRecord).arg(json)
    }

    /// Call `JSONB_TO_RECORD(jsonb)`, [`json_to_record`](Self::json_to_record)
    /// over `jsonb`.
    // [spec:pgorm:req:sql.ast.select.column-definitions]
    pub fn jsonb_to_record<T>(jsonb: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::JsonbToRecord).arg(jsonb)
    }

    /// Call `JSON_TO_RECORDSET(json)`: a JSON array of objects as one
    /// `record` each, in order, read as
    /// [`json_to_record`](Self::json_to_record) reads one.
    // [spec:pgorm:req:sql.ast.select.column-definitions]
    pub fn json_to_recordset<T>(json: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::JsonToRecordset).arg(json)
    }

    /// Call `JSONB_TO_RECORDSET(jsonb)`,
    /// [`json_to_recordset`](Self::json_to_recordset) over `jsonb`.
    // [spec:pgorm:req:sql.ast.select.column-definitions]
    pub fn jsonb_to_recordset<T>(jsonb: T) -> FunctionCall
    where
        T: Into<SimpleExpr>,
    {
        FunctionCall::new(Function::JsonbToRecordset).arg(jsonb)
    }
}
