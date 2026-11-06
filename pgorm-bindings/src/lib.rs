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
mod hook;
mod pool;
mod tls;

pub use copy::{builtin_type, copy_column};
pub use dialect::{Dialect, Refusal};
pub use hook::{HookCall, HookEvent, HookJob, HookJobs, hook};
pub use pool::{
    PoolFailure, PoolSettings, PoolShape, connect_listener, listener_config, multi_pool,
    pool_failure,
};
pub use tls::{Connector, TlsMode, TlsSettings};
