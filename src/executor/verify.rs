use pgorm_pool::GenericClient;
use tokio_postgres::{Statement, types::Type};

use crate::{
    DatabaseConnection, DatabaseTransaction, Error, FromQueryResult, VerifyError, error::query_err,
};

/// One column a [`FromQueryResult`] target reads out of a row, as the target
/// itself reports it.
///
/// The `accepts` function is the field's own decode test — `TryGetable::accepts`
/// for the field's Rust type — so a column's PostgreSQL type is judged by the
/// same mapping that would decode it.
// [spec:pgorm:def:exec.verify]    the reflected column shape
#[derive(Debug, Clone, Copy)]
pub struct ExpectedColumn {
    name: &'static str,
    rust_type: &'static str,
    accepts: fn(&Type) -> bool,
}

// [spec:pgorm:def:exec.verify]
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
        }
    }

    /// The column name looked up in the row.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The Rust type the column is decoded into, as the field spells it.
    pub fn rust_type(&self) -> &'static str {
        self.rust_type
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
/// # use pgorm::{error::*, DatabasePool, FromQueryResult, VerifyStatement};
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
/// let res: Vec<SelectResult> = SelectResult::find_by_statement(sql, vec![]).all(&db).await?;
/// # Ok(())
/// # }
/// # }
/// ```
// [spec:pgorm:def:exec.verify]
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

    /// Prepare `sql` and give the result columns PostgreSQL says it returns,
    /// in order, without running it.
    ///
    /// This is the description [`verify`](VerifyStatement::verify) checks a
    /// target against, for a decode target whose columns are known only at
    /// run time, such as a model a language binding declares.
    // [spec:pgorm:def:exec.verify.describe]
    fn describe(
        &self,
        sql: &str,
    ) -> impl Future<Output = Result<Vec<DescribedColumn>, Error>> + Send;
}

/// One result column of a statement, as the server describes it before the
/// statement runs.
// [spec:pgorm:def:exec.verify.describe]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribedColumn {
    name: String,
    type_: Type,
}

impl DescribedColumn {
    /// The column's name, as a row reads it by.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The column's PostgreSQL type.
    pub fn type_(&self) -> &Type {
        &self.type_
    }
}

/// The result columns `statement` describes.
fn described(statement: &Statement) -> Vec<DescribedColumn> {
    statement
        .columns()
        .iter()
        .map(|column| DescribedColumn {
            name: column.name().to_owned(),
            type_: column.type_().clone(),
        })
        .collect()
}

/// Compare `M`'s reported columns against a prepared statement's result columns.
// [spec:pgorm:def:exec.verify]    name presence then type acceptance
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

// [spec:pgorm:def:exec.verify]    pooled connection
impl VerifyStatement for DatabaseConnection {
    async fn verify<M>(&self, sql: &str) -> Result<(), Error>
    where
        M: FromQueryResult + 'static,
    {
        let statement = GenericClient::prepare(&self.0, sql).await?;
        check::<M>(&statement)
    }

    async fn describe(&self, sql: &str) -> Result<Vec<DescribedColumn>, Error> {
        let statement = GenericClient::prepare(&self.0, sql).await?;
        Ok(described(&statement))
    }
}

// [spec:pgorm:def:exec.verify]    open transaction
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

    async fn describe(&self, sql: &str) -> Result<Vec<DescribedColumn>, Error> {
        let Some(transaction) = self.0.as_ref() else {
            return Err(query_err("transaction already consumed"));
        };

        let statement = GenericClient::prepare(transaction, sql).await?;
        Ok(described(&statement))
    }
}

// [spec:pgorm:def:exec.verify]    borrowed connections and wrappers
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

    fn describe(
        &self,
        sql: &str,
    ) -> impl Future<Output = Result<Vec<DescribedColumn>, Error>> + Send {
        T::describe(self, sql)
    }
}
