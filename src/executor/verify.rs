use pgorm_pool::GenericClient;
use pgorm_query::TypeName;
use tokio_postgres::{Statement, types::Type};

use crate::query::graph::source_read_cast;
use crate::{
    ColumnTrait, DatabaseConnection, DatabaseTransaction, Error, FromQueryResult, VerifyError,
    error::query_err,
};

/// One column a [`FromQueryResult`] target reads out of a row, as the target
/// itself reports it.
///
/// The `accepts` function is the field's own decode test — `TryGetable::accepts`
/// for the field's Rust type — so a column's PostgreSQL type is judged by the
/// same mapping that would decode it.
///
/// A column the type reads through a cast also carries that cast, the type
/// [`selected_as`](ExpectedColumn::selected_as) took off the entity column's
/// own [`select_as`](crate::ColumnTrait::select_as): an enum column read as
/// its `text`, a `money` column as `numeric`. A reader that writes the
/// projection itself, as a pipeline's model terminals do, casts the column
/// to it, so the decode receives what it accepts.
// [spec:pgorm:def:exec.verify+1]    the reflected column shape
#[derive(Debug, Clone)]
pub struct ExpectedColumn {
    name: &'static str,
    rust_type: &'static str,
    accepts: fn(&Type) -> bool,
    read_cast: Option<TypeName>,
}

// [spec:pgorm:def:exec.verify+1]
impl ExpectedColumn {
    /// Declare a column read under `name` and decoded into `rust_type`, whose
    /// PostgreSQL types are the ones `accepts` answers `true` for.
    pub const fn new(
        name: &'static str,
        rust_type: &'static str,
        accepts: fn(&Type) -> bool,
    ) -> Self {
        Self {
            name,
            rust_type,
            accepts,
            read_cast: None,
        }
    }

    /// The same column, read under the cast `column` is selected through:
    /// the type its `select_as` casts it to, or no cast for a column that
    /// `select_as` leaves as it is. The `Model` and `PartialModel` derives
    /// report each column an entity's field reads this way:
    ///
    /// ```
    /// use pgorm::{FromQueryResult, tests_cfg::lunch_set};
    ///
    /// let casts: Vec<_> = lunch_set::Model::expected_columns()
    ///     .unwrap()
    ///     .iter()
    ///     .map(|column| (column.name(), column.read_cast().map(|cast| cast.to_sql_string())))
    ///     .collect();
    /// assert_eq!(
    ///     casts,
    ///     [("id", None), ("name", None), ("tea", Some("text".to_owned()))]
    /// );
    /// ```
    pub fn selected_as<C: ColumnTrait>(mut self, column: &C) -> Self {
        self.read_cast = source_read_cast(column);
        self
    }

    /// The column name looked up in the row.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The Rust type the column is decoded into, as the field spells it.
    pub fn rust_type(&self) -> &'static str {
        self.rust_type
    }

    /// The type the column is cast to when it is selected, or `None` for a
    /// column read as the server sends it.
    pub fn read_cast(&self) -> Option<&TypeName> {
        self.read_cast.as_ref()
    }
}

/// Check a decode target against a statement before any row exists to catch a
/// mismatch.
///
/// A [`FromQueryResult`] target that names a column the statement does not
/// return, or reads one into a Rust type that cannot decode its PostgreSQL
/// type, fails only when a row arrives: against an empty result set the decode
/// loop never runs and the query yields `Ok(vec![])`. Preparing the statement
/// makes PostgreSQL describe the result columns whether rows exist or not, and
/// this trait compares that description with what the target reports through
/// [`FromQueryResult::expected_columns`].
///
/// ```no_run
/// # #[cfg(feature = "macros")]
/// # {
/// # use pgorm::{error::*, DatabasePool, FromQueryResult, Values, VerifyStatement};
/// #
/// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
/// #[derive(Debug, FromQueryResult)]
/// struct SelectResult {
///     name: String,
///     num_of_cakes: i64,
/// }
///
/// let db = pool.get().await?;
/// let sql = r#"SELECT "name", COUNT(*) AS "num_of_cakes" FROM "cake" GROUP BY("name")"#;
///
/// db.verify::<SelectResult>(sql).await?;
/// let res: Vec<SelectResult> = SelectResult::find_by_statement(sql, Values(vec![])).all(&db).await?;
/// # Ok(())
/// # }
/// # }
/// ```
// [spec:pgorm:def:exec.verify+1]
pub trait VerifyStatement {
    /// Prepare `sql` and check `M`'s reported columns against the result
    /// columns PostgreSQL says the statement returns.
    ///
    /// Every column `M` names MUST be returned by the statement, and each one's
    /// PostgreSQL type MUST be one the field's Rust type accepts. The first
    /// column that fails either test is reported as [`Error::Verify`]; a target
    /// that reports no columns at all is [`VerifyError::Unreflected`] rather
    /// than a pass.
    ///
    /// Column names are matched bare, as
    /// [`find_by_statement`](FromQueryResult::find_by_statement) decodes them —
    /// with the empty prefix. Nothing is executed, so the statement's
    /// parameters are neither bound nor checked.
    fn verify<M>(&self, sql: &str) -> impl Future<Output = Result<(), Error>> + Send
    where
        M: FromQueryResult + 'static;
}

/// Compare `M`'s reported columns against a prepared statement's result columns.
// [spec:pgorm:def:exec.verify+1]    name presence then type acceptance
// [spec:pgorm:req:exec.verify.limits+1]    and nothing else: the statement's
// parameters, its columns' nullability, and the columns `M` does not read are
// all left alone
fn check<M>(statement: &Statement) -> Result<(), Error>
where
    M: FromQueryResult + 'static,
{
    let target = std::any::type_name::<M>();

    // [spec:pgorm:req:exec.verify.manual+1]    an unreported shape is unverifiable, not verified
    let Some(expected) = M::expected_columns() else {
        return Err(VerifyError::Unreflected { target }.into());
    };

    for column in expected {
        // [spec:pgorm:req:exec.verify.errors]    the columns the statement does return
        let Some(returned) = statement
            .columns()
            .iter()
            .find(|candidate| candidate.name() == column.name())
        else {
            return Err(VerifyError::ColumnMissing {
                target,
                column: column.name(),
                returned: statement
                    .columns()
                    .iter()
                    .map(|candidate| candidate.name())
                    .collect::<Vec<_>>()
                    .join(", "),
            }
            .into());
        };

        // [spec:pgorm:req:exec.verify.errors]    the Rust type and the type it cannot read
        if !(column.accepts)(returned.type_()) {
            return Err(VerifyError::ColumnType {
                target,
                column: column.name(),
                rust_type: column.rust_type(),
                pg_type: returned.type_().to_string(),
            }
            .into());
        }
    }

    Ok(())
}

// [spec:pgorm:def:exec.verify+1]    pooled connection
impl VerifyStatement for DatabaseConnection {
    async fn verify<M>(&self, sql: &str) -> Result<(), Error>
    where
        M: FromQueryResult + 'static,
    {
        let statement = GenericClient::prepare(&self.0, sql).await?;
        check::<M>(&statement)
    }
}

// [spec:pgorm:def:exec.verify+1]    open transaction
impl VerifyStatement for DatabaseTransaction<'_> {
    async fn verify<M>(&self, sql: &str) -> Result<(), Error>
    where
        M: FromQueryResult + 'static,
    {
        let Some(transaction) = self.0.as_ref() else {
            return Err(query_err("transaction already consumed"));
        };

        let statement = GenericClient::prepare(transaction, sql).await?;
        check::<M>(&statement)
    }
}

// [spec:pgorm:def:exec.verify+1]    borrowed connections and wrappers
impl<T> VerifyStatement for &T
where
    T: VerifyStatement + Sync + ?Sized,
{
    fn verify<M>(&self, sql: &str) -> impl Future<Output = Result<(), Error>> + Send
    where
        M: FromQueryResult + 'static,
    {
        T::verify::<M>(self, sql)
    }
}
