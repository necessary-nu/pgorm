//! A statement's rows streamed inside a transaction: the frame's task opens
//! them, and the stream holds the frame's lock until its last row.

use neon::prelude::*;
use pgorm_bindings::StreamState;

use super::{abort_argument, settle, state};

/// `transactionStream(transaction, sql, params, abort, decoder?)`: a stream
/// of the statement's rows in the transaction, which holds its lock until the
/// last row; closed before it, the transaction is discarded.
// [spec:pgorm:req:bindings.streams+1]
pub(super) fn transaction_stream(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let state = state(&mut cx)?;
    let (sql, params) = crate::connect::sql_and_params(&mut cx, 1)?;
    let abort = abort_argument(&mut cx, 3)?;
    let decode = crate::connect::stream::decode_argument(&mut cx, 4);
    let lock = match state.lock() {
        Ok(lock) => lock,
        Err(failure) => return settle::rejected(&mut cx, failure),
    };
    let work = async move {
        let stream = StreamState::in_transaction(&state, lock, sql, params, abort.token()).await?;
        Ok(crate::connect::stream::Stream::new(stream, decode))
    };
    settle::promise(&mut cx, work, |cx, stream| {
        Ok(cx
            .boxed(crate::connect::stream::StreamHandle(stream))
            .upcast())
    })
}
