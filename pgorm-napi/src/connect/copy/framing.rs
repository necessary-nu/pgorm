//! The binary COPY format's two sides for a program that moves the bytes
//! itself: pgorm's `CopyRowEncoder` and `CopyRowDecoder` over a list of
//! columns, each row written and read as a COPY of rows writes and reads it.

use std::cell::RefCell;

use bytes::BytesMut;
use neon::{
    prelude::*,
    types::{Finalize, JsUint8Array, buffer::TypedArray},
};
use pgorm::{CopyRowDecoder, CopyRowEncoder};

use super::{
    columns::columns_from,
    rows::{encode_row, next_decoded, refusal, row_to_js},
};
use crate::{
    codec::Codec,
    errors::{Failure, throw},
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("copyEncoderNew", copy_encoder_new)?;
    cx.export_function("copyEncoderEncode", copy_encoder_encode)?;
    cx.export_function("copyEncoderHeader", copy_encoder_header)?;
    cx.export_function("copyEncoderTrailer", copy_encoder_trailer)?;
    cx.export_function("copyDecoderNew", copy_decoder_new)?;
    cx.export_function("copyDecoderPush", copy_decoder_push)?;
    cx.export_function("copyDecoderNext", copy_decoder_next)?;
    cx.export_function("copyDecoderFinish", copy_decoder_finish)?;
    Ok(())
}

#[derive(Debug)]
pub(crate) struct EncoderHandle(CopyRowEncoder);

impl Finalize for EncoderHandle {}

/// The decoder JavaScript feeds and reads on its own thread, one call at a
/// time.
#[derive(Debug)]
pub(crate) struct DecoderHandle(RefCell<CopyRowDecoder>);

impl Finalize for DecoderHandle {}

// [spec:pgorm:req:bindings.copy-rows+2]
fn copy_encoder_new(mut cx: FunctionContext) -> JsResult<JsBox<EncoderHandle>> {
    let columns = columns_from(&mut cx, 0)?;
    Ok(cx.boxed(EncoderHandle(CopyRowEncoder::new(columns))))
}

/// `copyEncoderEncode(encoder, row)`: one row in the binary format, read and
/// checked as a COPY of rows' sink reads and checks one.
// [spec:pgorm:req:bindings.copy-rows+2]
fn copy_encoder_encode(mut cx: FunctionContext) -> JsResult<JsUint8Array> {
    let encoder = cx.argument::<JsBox<EncoderHandle>>(0)?;
    let row = cx.argument::<JsValue>(1)?;
    let codec = Codec::get(&mut cx)?;
    let mut out = BytesMut::new();
    encode_row(&mut cx, codec, &encoder.0, row, &mut out)?;
    JsUint8Array::from_slice(&mut cx, &out)
}

/// `copyEncoderHeader()`: the signature, flags and extension area that come
/// once before the first row.
fn copy_encoder_header(mut cx: FunctionContext) -> JsResult<JsUint8Array> {
    let mut out = BytesMut::new();
    CopyRowEncoder::header(&mut out);
    JsUint8Array::from_slice(&mut cx, &out)
}

/// `copyEncoderTrailer()`: the field count of -1 that comes once after the
/// last row.
fn copy_encoder_trailer(mut cx: FunctionContext) -> JsResult<JsUint8Array> {
    let mut out = BytesMut::new();
    CopyRowEncoder::trailer(&mut out);
    JsUint8Array::from_slice(&mut cx, &out)
}

// [spec:pgorm:req:bindings.copy-rows+2]
fn copy_decoder_new(mut cx: FunctionContext) -> JsResult<JsBox<DecoderHandle>> {
    let columns = columns_from(&mut cx, 0)?;
    Ok(cx.boxed(DecoderHandle(RefCell::new(CopyRowDecoder::new(columns)))))
}

/// `copyDecoderPush(decoder, chunk)`: a copy of the next chunk of the data,
/// split anywhere.
fn copy_decoder_push(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let decoder = cx.argument::<JsBox<DecoderHandle>>(0)?;
    let chunk = cx.argument::<JsUint8Array>(1)?;
    decoder.0.borrow_mut().push(chunk.as_slice(&cx));
    Ok(cx.undefined())
}

/// `copyDecoderNext(decoder)`: the next whole row, each field decoded as a
/// COPY of rows' stream decodes it, or `null` until more data arrives and
/// after the trailer. Data that is not the binary format, and a field that
/// does not decode, are a `DecodeError`.
// [spec:pgorm:req:bindings.copy-rows+2]
fn copy_decoder_next(mut cx: FunctionContext) -> JsResult<JsValue> {
    let decoder = cx.argument::<JsBox<DecoderHandle>>(0)?;
    let next = next_decoded(&mut decoder.0.borrow_mut());
    match next {
        Ok(Some(row)) => {
            let codec = Codec::get(&mut cx)?;
            Ok(row_to_js(&mut cx, codec, &row)?.upcast())
        }
        Ok(None) => Ok(cx.null().upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// `copyDecoderFinish(decoder)`: once the data has all been pushed, refuse
/// it with a `DecodeError` if it stopped part way through a row or before
/// the trailer.
// [spec:pgorm:req:bindings.copy-rows+2]
fn copy_decoder_finish(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let decoder = cx.argument::<JsBox<DecoderHandle>>(0)?;
    let finished = decoder.0.borrow().finish();
    match finished {
        Ok(()) => Ok(cx.undefined()),
        Err(error) => throw(&mut cx, Failure::Decode(refusal(error))),
    }
}
