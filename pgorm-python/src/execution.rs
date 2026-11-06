//! Borrowed execution context: all calls preserve the selected native executor.

mod target;
pub(crate) use target::{Executor, Target};

use crate::metrics::Metrics;

/// What an operation runs on: a connection or a transaction, plain or, from
/// an instrumented pool, pgorm's metrics wrapper around one.
pub(crate) type Database<'a> = pgorm::DatabaseRef<'a, Metrics>;
