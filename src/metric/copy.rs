//! COPY on the metrics wrappers: the start of each COPY reported as a
//! statement is, and the sink or stream handed back unwrapped.

use std::time::Instant;

use async_trait::async_trait;
use pgorm_query::{CopyFormat, CopyFromStatement, CopyToStatement};

use super::{InstrumentedConnection, InstrumentedTransaction, MetricsCollector, QueryContext};
use crate::{CopyColumn, CopyConnection, CopyInSink, CopyOutStream, Error};

// [spec:pgorm:req:metric.layer.copy+1]    COPY on an instrumented connection
#[async_trait]
impl<M: MetricsCollector> CopyConnection for InstrumentedConnection<M> {
    async fn copy_in<F>(
        &mut self,
        statement: &CopyFromStatement<F>,
    ) -> Result<CopyInSink<'_>, Error>
    where
        F: CopyFormat,
    {
        let sql = statement.to_string();
        report!(self, "copy_in", Some(&sql),
            self.connection.copy_in(statement).await, _ => None)
    }

    async fn copy_out<F>(
        &mut self,
        statement: &CopyToStatement<F>,
    ) -> Result<CopyOutStream<'_>, Error>
    where
        F: CopyFormat,
    {
        let sql = statement.to_string();
        report!(self, "copy_out", Some(&sql),
            self.connection.copy_out(statement).await, _ => None)
    }

    async fn copy_in_columns<F>(
        &mut self,
        statement: &CopyFromStatement<F>,
    ) -> Result<Vec<CopyColumn>, Error>
    where
        F: CopyFormat,
    {
        self.connection.copy_in_columns(statement).await
    }

    async fn copy_out_columns<F>(
        &mut self,
        statement: &CopyToStatement<F>,
    ) -> Result<Vec<CopyColumn>, Error>
    where
        F: CopyFormat,
    {
        self.connection.copy_out_columns(statement).await
    }
}

// [spec:pgorm:req:metric.layer.copy+1]    COPY inside an instrumented transaction
#[async_trait]
impl<M: MetricsCollector> CopyConnection for InstrumentedTransaction<'_, M> {
    async fn copy_in<F>(
        &mut self,
        statement: &CopyFromStatement<F>,
    ) -> Result<CopyInSink<'_>, Error>
    where
        F: CopyFormat,
    {
        let sql = statement.to_string();
        report!(self, "copy_in", Some(&sql),
            self.transaction.copy_in(statement).await, _ => None)
    }

    async fn copy_out<F>(
        &mut self,
        statement: &CopyToStatement<F>,
    ) -> Result<CopyOutStream<'_>, Error>
    where
        F: CopyFormat,
    {
        let sql = statement.to_string();
        report!(self, "copy_out", Some(&sql),
            self.transaction.copy_out(statement).await, _ => None)
    }

    async fn copy_in_columns<F>(
        &mut self,
        statement: &CopyFromStatement<F>,
    ) -> Result<Vec<CopyColumn>, Error>
    where
        F: CopyFormat,
    {
        self.transaction.copy_in_columns(statement).await
    }

    async fn copy_out_columns<F>(
        &mut self,
        statement: &CopyToStatement<F>,
    ) -> Result<Vec<CopyColumn>, Error>
    where
        F: CopyFormat,
    {
        self.transaction.copy_out_columns(statement).await
    }
}
