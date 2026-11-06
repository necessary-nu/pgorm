//! A stream of a statement's rows inside a transaction: the scope task owning
//! the borrowed transaction opens it, and the stream holds the transaction's
//! reservation until its last row.

use super::{State, copy::Lease, scope::Command};
use crate::{errors::database_error, expressions::Compiled};
use pgorm::{ConnectionTrait, Error, ValueHolder};
use pyo3::prelude::*;
use std::sync::Arc;
use tokio_postgres::{RowStream, types::ToSql};
use tokio_util::sync::CancellationToken;

impl State {
    /// Reserve the transaction and have its owner open `compiled`'s rows.
    /// A statement the server refuses leaves the transaction as any failed
    /// statement does.
    // [spec:pgorm:req:bindings.streams+1]
    pub(crate) async fn stream(
        self: &Arc<Self>,
        compiled: Compiled,
    ) -> PyResult<(Lease, RowStream)> {
        let (result, busy, mut pending) = self
            .exchange(|opened| Command::Stream { compiled, opened })
            .await?;
        match result {
            Ok(rows) => Ok((Lease::new(busy, pending), rows)),
            Err(error) => {
                pending.complete();
                Err(database_error(error, &self.connection.pool.secrets))
            }
        }
    }

    /// A token that fires once this transaction or savepoint has ended, by
    /// any route.
    pub(crate) fn ended(&self) -> CancellationToken {
        let ended = CancellationToken::new();
        let fire = ended.clone();
        let (aborted, finished, abandoned, own) = (
            self.root.aborted.clone(),
            self.root.finished.clone(),
            self.abandoned.clone(),
            self.finished.clone(),
        );
        tokio::spawn(async move {
            tokio::select! {
                _ = aborted.cancelled() => (),
                _ = finished.cancelled() => (),
                _ = abandoned.cancelled() => (),
                _ = own.cancelled() => (),
            }
            fire.cancel();
        });
        ended
    }
}

/// Open `compiled`'s rows on the owner's transaction.
pub(super) async fn open(
    tx: &impl ConnectionTrait,
    compiled: Compiled,
) -> Result<RowStream, Error> {
    let values: Vec<_> = compiled.values.0.into_iter().map(ValueHolder).collect();
    tx.query_raw(
        &compiled.sql,
        values.iter().map(|value| value as &(dyn ToSql + Sync)),
    )
    .await
}
