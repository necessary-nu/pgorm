use std::{sync::Arc, time::Instant};

use super::{InstrumentedConnection, MetricsCollector};
use crate::{ConnectionPool, DatabasePool, Error};

/// A connection pool wrapper that produces instrumented connections: of a
/// [`DatabasePool`] by default, or of a [`ReadOnlyPool`](crate::ReadOnlyPool),
/// whose instrumented connections and transactions may still only read.
// [spec:pgorm:def:metric.layer+4]    pool wrapper
// [spec:pgorm:req:read-only.metrics]
#[derive(Debug, Clone)]
#[doc(alias = "set_metric_callback")]
pub struct InstrumentedPool<M: MetricsCollector, P = DatabasePool> {
    pool: P,
    metrics: M,
}

impl<M: MetricsCollector, P: ConnectionPool> InstrumentedPool<M, P> {
    /// Create a new instrumented pool
    pub fn new(pool: P, metrics: M) -> Self {
        Self { pool, metrics }
    }

    /// Get an instrumented connection from the pool
    pub async fn get(&self) -> Result<InstrumentedConnection<M, P::Connection>, Error> {
        let start = Instant::now();
        let result = self.pool.get().await;
        let elapsed = start.elapsed();

        match result {
            Ok(conn) => {
                self.metrics.record_connection_acquired(elapsed).await;
                Ok(InstrumentedConnection::new(conn, self.metrics.clone()))
            }
            Err(e) => {
                self.metrics.record_connection_error(elapsed, &e).await;
                Err(e)
            }
        }
    }

    /// Get the pool tag
    pub fn tag(&self) -> Arc<String> {
        self.pool.tag()
    }

    /// Get the pool status
    pub fn status(&self) -> deadpool::Status {
        self.pool.status()
    }

    /// [`DatabasePool::clear_statement_caches`] on the wrapped pool.
    // [spec:pgorm:req:conn.pool.statement-cache.clear+2]
    pub fn clear_statement_caches(&self) {
        self.pool.clear_statement_caches();
    }

    /// Get a reference to the underlying pool
    pub fn inner(&self) -> &P {
        &self.pool
    }

    /// Get a reference to the metrics collector
    pub fn metrics(&self) -> &M {
        &self.metrics
    }
}
