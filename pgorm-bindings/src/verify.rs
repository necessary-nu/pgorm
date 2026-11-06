//! A statement's result described for a binding's declared model, on
//! whichever executor the binding holds.

use pgorm::{
    DatabaseRef, DescribedColumn, Error, RuntimeError, VerifyStatement, metric::MetricsCollector,
};

/// The result columns `sql` describes on `db`, prepared without running it,
/// as [`VerifyStatement::describe`] gives them. An instrumented connection or
/// transaction describes through the one it wraps, which is where pgorm's
/// metrics layer has a caller verify; a pool hook's connection, which runs
/// only its hook's statements, describes none.
// [spec:pgorm:req:bindings.verify]
pub async fn describe<M: MetricsCollector>(
    db: DatabaseRef<'_, M>,
    sql: &str,
) -> Result<Vec<DescribedColumn>, Error> {
    match db {
        DatabaseRef::Connection(connection) => connection.describe(sql).await,
        DatabaseRef::Transaction(transaction) => transaction.describe(sql).await,
        DatabaseRef::InstrumentedConnection(connection) => connection.inner().describe(sql).await,
        DatabaseRef::InstrumentedTransaction(transaction) => {
            transaction.inner().describe(sql).await
        }
        DatabaseRef::Hook(_) => Err(Error::Query(RuntimeError::Internal(
            "a pool hook's connection runs its hook's statements and describes none".to_owned(),
        ))),
    }
}
