//! A metrics collector recording the SQL of every statement sent, for a
//! test showing what a pool's connections sent, or that they sent nothing.

use async_trait::async_trait;
use pgorm::Error;
use pgorm::metric::{MetricsCollector, QueryContext};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// The SQL of every statement a connection sends.
#[derive(Clone, Debug, Default)]
pub struct Statements(Arc<Mutex<Vec<String>>>);

impl Statements {
    fn push(&self, query: QueryContext<'_>) {
        if let Ok(mut sent) = self.0.lock() {
            sent.push(query.sql().unwrap_or_default().to_owned());
        }
    }

    /// Every statement sent since the last `take`, in the order sent.
    pub fn take(&self) -> Vec<String> {
        self.0
            .lock()
            .map(|mut sent| std::mem::take(&mut *sent))
            .unwrap_or_default()
    }
}

#[async_trait]
impl MetricsCollector for Statements {
    async fn record_query_success(
        &self,
        query: QueryContext<'_>,
        _duration: Duration,
        _rows: Option<u64>,
    ) {
        self.push(query);
    }

    async fn record_query_error(&self, query: QueryContext<'_>, _duration: Duration, _: &Error) {
        self.push(query);
    }

    async fn record_connection_acquired(&self, _duration: Duration) {}

    async fn record_connection_error(&self, _duration: Duration, _error: &Error) {}

    async fn record_transaction_begin(&self, _duration: Duration) {}

    async fn record_transaction_commit(&self, _duration: Duration) {}

    async fn record_transaction_rollback(&self, _duration: Duration) {}
}
