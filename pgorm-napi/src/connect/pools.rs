//! The natives that make a pool and read it: from a connection string and
//! its options, several by tag over shared options, or an instrumented pool
//! around one; and its tag, status and statement caches.

use crate::errors::FailureJs as _;
use neon::prelude::*;
use pgorm::metric::InstrumentedPool;

use super::{
    Lender, PoolHandle,
    config::{self, Shared},
    metrics, pool,
};
use crate::values::read;

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("poolNew", pool_new)?;
    cx.export_function("poolsNew", pools_new)?;
    cx.export_function("poolTag", pool_tag)?;
    cx.export_function("poolStatus", pool_status)?;
    cx.export_function("poolClearStatementCaches", pool_clear_statement_caches)?;
    cx.export_function("poolInstrument", pool_instrument)?;
    Ok(())
}

/// `poolNew(dsn, options)`: a pool, which opens connections as they are
/// first needed. Nothing is sent before then.
// [spec:pgorm:req:bindings.connections+3]
fn pool_new(mut cx: FunctionContext) -> JsResult<JsBox<PoolHandle>> {
    let dsn = cx.argument::<JsValue>(0)?;
    let options = cx.argument::<JsObject>(1)?;
    let shared = Shared::read(&mut cx, dsn, options)?;
    let shape = config::shape(&mut cx, options)?;
    let tag = match config::option(&mut cx, options, "tag")? {
        Some(tag) => Some(read::text(&mut cx, tag, "tag")?),
        None => None,
    };
    let built = match shared.build(tag, shape) {
        Ok(built) => built,
        Err(failure) => {
            let error = failure.into_js(&mut cx)?;
            return cx.throw(error);
        }
    };
    Ok(cx.boxed(PoolHandle(Lender::new(built))))
}

/// `poolInstrument(pool, hooks, dispatch)`: the pool, sharing its
/// connections and its closing, reporting to the collector whose seven hooks
/// `hooks` holds in `MetricsCollector`'s order — `null` for one never to
/// run — each run on the JavaScript thread by `dispatch`.
// [spec:pgorm:req:bindings.metrics]
fn pool_instrument(mut cx: FunctionContext) -> JsResult<JsBox<PoolHandle>> {
    let lender = pool(&mut cx)?;
    if lender.metered.is_some() {
        return cx.throw_type_error("an InstrumentedPool wraps a pool that is not instrumented");
    }
    let listed = cx.argument::<JsArray>(1)?;
    let dispatch = cx.argument::<JsFunction>(2)?;
    let mut hooks = [None; pgorm_bindings::Hook::ALL.len()];
    if listed.len(&mut cx) as usize != hooks.len() {
        return cx.throw_type_error("a collector has seven hooks");
    }
    for (at, hook) in (0_u32..).zip(hooks.iter_mut()) {
        let value: Handle<JsValue> = listed.get(&mut cx, at)?;
        if !value.is_a::<JsNull, _>(&mut cx) {
            *hook = Some(value.downcast_or_throw::<JsFunction, _>(&mut cx)?);
        }
    }
    let metrics = metrics::metrics(&mut cx, dispatch, hooks, lender.secrets.clone());
    Ok(cx.boxed(PoolHandle(Lender {
        metered: Some(InstrumentedPool::new(lender.pool.clone(), metrics)),
        state: lender.state,
    })))
}

/// `poolsNew(dsn, shapes, options)`: one pool per own property of `shapes`,
/// each tagged with its key and shaped by its value, sharing `options`, as
/// `[[tag, pool], ...]` in the tags' order. Nothing is sent.
// [spec:pgorm:req:bindings.multi-pool]
fn pools_new(mut cx: FunctionContext) -> JsResult<JsArray> {
    let dsn = cx.argument::<JsValue>(0)?;
    let shapes = cx.argument::<JsObject>(1)?;
    let options = cx.argument::<JsObject>(2)?;
    let shared = Shared::read(&mut cx, dsn, options)?;
    if config::option(&mut cx, options, "tag")?.is_some() {
        return read::refuse(
            &mut cx,
            "each pool's tag is its key, so the shared options name none",
        );
    }
    let keys = shapes.get_own_property_names(&mut cx)?.to_vec(&mut cx)?;
    let mut read_shapes = std::collections::BTreeMap::new();
    for key in keys {
        let tag = read::string(&mut cx, key)?;
        let value: Handle<JsValue> = shapes.get(&mut cx, tag.as_str())?;
        let Ok(object) = value.downcast::<JsObject, _>(&mut cx) else {
            return cx.throw_type_error(format!("pool {tag:?}'s options are an object"));
        };
        let shape = config::shape(&mut cx, object)?;
        read_shapes.insert(tag, shape);
    }
    let built = match shared.build_multi(read_shapes) {
        Ok(built) => built,
        Err(failure) => {
            let error = failure.into_js(&mut cx)?;
            return cx.throw(error);
        }
    };
    let pools = cx.empty_array();
    for (index, (tag, built)) in (0u32..).zip(built) {
        let entry = cx.empty_array();
        let tag = cx.string(tag);
        entry.set(&mut cx, 0, tag)?;
        let handle = cx.boxed(PoolHandle(Lender::new(built)));
        entry.set(&mut cx, 1, handle)?;
        pools.set(&mut cx, index, entry)?;
    }
    Ok(pools)
}

/// `poolTag(pool)`: the pool's tag, or the `default-N` pgorm named it.
// [spec:pgorm:req:bindings.connections+3]
fn pool_tag(mut cx: FunctionContext) -> JsResult<JsString> {
    let tag = pool(&mut cx)?.pool.tag();
    Ok(cx.string(tag.as_str()))
}

/// `poolStatus(pool)`: `{ maxSize, size, available, waiting }`.
fn pool_status(mut cx: FunctionContext) -> JsResult<JsObject> {
    let status = pool(&mut cx)?.pool.status();
    let object = cx.empty_object();
    for (name, value) in [
        ("maxSize", status.max_size),
        ("size", status.size),
        ("available", status.available),
        ("waiting", status.waiting),
    ] {
        #[allow(clippy::cast_precision_loss)]
        let value = cx.number(value as f64);
        object.set(&mut cx, name, value)?;
    }
    Ok(object)
}

/// `poolClearStatementCaches(pool)`: mark every connection's cached prepared
/// statements stale, as `DatabasePool::clear_statement_caches` does. Nothing
/// is sent, so a closed pool is no reason to refuse it.
// [spec:pgorm:req:bindings.connections+3]
// [spec:pgorm:req:conn.pool.statement-cache.clear+1]
fn pool_clear_statement_caches(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    pool(&mut cx)?.pool.clear_statement_caches();
    Ok(cx.undefined())
}
