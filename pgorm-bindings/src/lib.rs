//! What pgorm's two language bindings, pgorm-python and pgorm-napi, share
//! beyond pgorm itself.
//!
//! Each binding reads its language's arguments into the types here, calls
//! them, and raises what they refuse as its own errors, so a pool, its TLS
//! and its hooks behave the same from Python and from JavaScript because one
//! piece of Rust decides them. Nothing here names either language: where a
//! refusal names an option or quotes a choice, [`Dialect`] spells it as the
//! binding does.

mod copy;
mod dialect;
mod failure;
mod hook;
mod metrics;
mod pool;
mod tls;
mod verify;

pub use copy::{builtin_type, copy_column};
pub use dialect::{Dialect, Refusal};
pub use failure::{Class, Diagnostics, Failure, Redactions, classify};
pub use hook::{HookCall, HookEvent, HookJob, HookJobs, hook};
pub use metrics::{Database, Hook, Metrics, Report, Reporter};
pub use pool::{PoolSettings, PoolShape, connect_listener, listener_config, multi_pool};
pub use tls::{Connector, TlsMode, TlsSettings};
pub use verify::describe;
