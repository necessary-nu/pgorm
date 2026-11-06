//! What pgorm's two language bindings, pgorm-python and pgorm-napi, share
//! beyond pgorm itself: everything both do alike, so that each binding is
//! only its language's glue.
//!
//! Each binding reads its language's arguments into the types here, calls
//! them, and raises what they refuse as its own errors, so a pool and its
//! TLS, a lent connection, a transaction's frames and retries, a stream, a
//! listener, a metrics hook and a failure's class behave the same from Python
//! and from JavaScript because one piece of Rust decides them. Nothing here
//! names either language: where a refusal names an option or quotes a
//! choice, [`Dialect`] spells it as the binding does.
// [spec:pgorm:req:bindings.shared-runtime]

mod connection;
mod copy;
mod dialect;
mod failure;
mod hook;
mod listen;
mod metrics;
mod pool;
mod stream;
mod tls;
pub mod transaction;
mod verify;

pub use connection::{Closed, ConnectionState, Held, Operation, PoolState, cancellable};
pub use copy::{builtin_type, copy_column};
pub use dialect::{Dialect, Refusal};
pub use failure::{Class, Diagnostics, Failure, Redactions, classify};
pub use hook::{HookCall, HookEvent, HookJob, HookJobs, hook};
pub use listen::{ListenRequest, ListenerState, Received};
pub use metrics::{Database, Hook, Metrics, Report, Reporter};
pub use pool::{PoolSettings, PoolShape, connect_listener, listener_config, multi_pool};
pub use stream::StreamState;
pub use tls::{Connector, TlsMode, TlsSettings};
pub use verify::describe;
