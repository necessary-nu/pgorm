//! COPY on the metrics wrappers: the start of each COPY reported as a
//! statement is, and the sink or stream handed back unwrapped.

use std::time::Instant;

use async_trait::async_trait;
use pgorm_query::{CopyFormat, CopyFromStatement, CopyToStatement};

use super::{InstrumentedConnection, InstrumentedTransaction, MetricsCollector, QueryContext};
use crate::{CopyColumn, CopyConnection, CopyConnectionMut, CopyInSink, CopyOutStream, Error};

/// The COPY impls of an instrumented wrapper: each COPY's start reported,
/// its column descriptions handed to the wrapped connection unreported.
macro_rules! instrumented_copy {
    ([$($generics:tt)*] $ty:ty, $field:ident) => {
        // [spec:pgorm:req:metric.layer.copy+2]
        #[async_trait]
        impl<$($generics)*> CopyConnection for $ty {
            async fn copy_out<F>(
                &mut self,
                statement: &CopyToStatement<F>,
            ) -> Result<CopyOutStream<'_>, Error>
            where
                F: CopyFormat,
            {
                let sql = statement.to_string();
                report!(self, "copy_out", Some(&sql),
                    self.$field.copy_out(statement).await, _ => None)
            }

            async fn copy_out_columns<F>(
                &mut self,
                statement: &CopyToStatement<F>,
            ) -> Result<Vec<CopyColumn>, Error>
            where
                F: CopyFormat,
            {
                self.$field.copy_out_columns(statement).await
            }
        }
    };
}

/// The `CopyConnectionMut` impl of an instrumented wrapper over a connection
/// that may write.
macro_rules! instrumented_copy_mut {
    ([$($generics:tt)*] $ty:ty, $field:ident) => {
        // [spec:pgorm:req:metric.layer.copy+2]
        #[async_trait]
        impl<$($generics)*> CopyConnectionMut for $ty {
            async fn run_copy_in<F>(
                &mut self,
                statement: &CopyFromStatement<F>,
            ) -> Result<CopyInSink<'_>, Error>
            where
                F: CopyFormat,
            {
                let sql = statement.to_string();
                report!(self, "copy_in", Some(&sql),
                    self.$field.copy_in(statement).await, _ => None)
            }

            async fn run_copy_in_columns<F>(
                &mut self,
                statement: &CopyFromStatement<F>,
            ) -> Result<Vec<CopyColumn>, Error>
            where
                F: CopyFormat,
            {
                self.$field.copy_in_columns(statement).await
            }
        }
    };
}

instrumented_copy!([M: MetricsCollector, C: CopyConnection + Send] InstrumentedConnection<M, C>, connection);
instrumented_copy_mut!([M: MetricsCollector, C: CopyConnectionMut + Send] InstrumentedConnection<M, C>, connection);
instrumented_copy!(['a, M: MetricsCollector, Tx: CopyConnection + Send] InstrumentedTransaction<'a, M, Tx>, transaction);
instrumented_copy_mut!(['a, M: MetricsCollector, Tx: CopyConnectionMut + Send] InstrumentedTransaction<'a, M, Tx>, transaction);
