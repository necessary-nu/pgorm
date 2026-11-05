//! pgorm from Node.js and Deno, through Node-API.
//!
//! The addon is a Neon cdylib that both runtimes load as a `.node` file;
//! `lib/index.js` loads it, defines the error and value classes, and is what
//! JavaScript imports. pgorm's asynchronous work runs on one tokio runtime per
//! process ([`runtime`]), and each result crosses back to JavaScript by
//! settling a promise on the instance's JavaScript thread.

mod codec;
mod connect;
mod decode;
mod disjoint;
pub mod entities;
mod errors;
mod models;
mod one_of;
mod params;
#[cfg(debug_assertions)]
mod probes;
mod rows;
mod runtime;
mod settle;
mod statements;
mod values;

use neon::prelude::*;

pub use entities::{GraphSlots, RegistrationError, Registry};

/// The addon's own module, with no registered entity: what `lib/index.js`
/// loads unless an application builds a module of its own around
/// [`install`].
// [spec:pgorm:def:napi.api+1]
// [spec:pgorm:req:bindings.optional]
// [spec:pgorm:req:napi.loading+1]
#[cfg(feature = "standalone-module")]
#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    install(&mut cx, Registry::default())
}

/// Install the binding's whole API, and `registry`'s entities and graphs,
/// into one module: what an application's own `#[neon::main]` calls, having
/// registered its entities, in a crate that depends on this one without its
/// `standalone-module` feature.
// [spec:pgorm:req:bindings.entities+5]
// [spec:pgorm:req:napi.application+2]
pub fn install(cx: &mut ModuleContext, registry: Registry) -> NeonResult<()> {
    runtime::runtime(cx)?;
    cx.export_function("setErrorFactory", errors::set_error_factory)?;
    cx.export_function("setCodec", codec::set_codec)?;
    connect::export(cx)?;
    values::export(cx)?;
    statements::export(cx)?;
    models::export(cx)?;
    entities::install(cx, registry)?;
    disjoint::export(cx)?;
    one_of::export(cx)?;
    #[cfg(debug_assertions)]
    probes::export(cx)?;
    let version = cx.string(env!("CARGO_PKG_VERSION"));
    cx.export_value("version", version)?;
    // [spec:pgorm:req:bindings.connections+2]    the channel pgorm-migration's runner notifies
    let channel = cx.string(pgorm::STATEMENT_CACHE_CHANNEL);
    cx.export_value("statementCacheChannel", channel)?;
    Ok(())
}
