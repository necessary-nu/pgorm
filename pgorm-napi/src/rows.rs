//! A statement's rows handed to JavaScript a column at a time: each column an
//! array of its values in row order, which `lib/value-columns.js` finishes and
//! `lib/operations.js` makes rows of, each an object keyed by column name.
//!
//! A value made and placed in an array alone costs two Node-API calls, and a
//! `Decimal`, a `Uuid`, an instant or a JSON document a call into JavaScript
//! besides. So each column is decoded on the runtime thread into the form
//! JavaScript builds its values from; its values are handed to `gather` as
//! the arguments of one call per block, and built by `finish` in one loop per
//! column. A column of JSON crosses as the text of one array, which a single
//! `JSON.parse` reads.

use std::fmt::Write as _;

use jiff::Timestamp;
use neon::{prelude::*, types::JsBigInt};
use pgorm::{
    ExactDecimal,
    pgorm_query::{ArrayType, base_type},
};
use pgorm_bindings::ExactJsonText;
use pgorm_driver::{
    Row,
    types::{FromSql, Type},
};
use uuid::Uuid;

use crate::{
    codec::{Codec, Columns},
    decode::{self, CodecError},
    errors::Failure,
    values::{Tag, Tagged, write},
};

/// How many values one call to `gather` takes, well inside the arguments a
/// call may carry.
const BLOCK: usize = 8192;

/// How long a column's JSON text grows before the next document starts
/// another, so that no one string nears the longest JavaScript holds.
const SEGMENT: usize = 1 << 24;

/// A result decoded on the runtime thread: its column names, each column's
/// kind and values, its row count, and whether its values read as tagged
/// `Value`s.
#[derive(Debug, Default)]
pub(crate) struct Decoded {
    names: Vec<String>,
    kinds: Vec<Tag>,
    columns: Vec<Column>,
    count: usize,
    tagged: bool,
}

/// One column's values, in the form they cross in.
#[derive(Debug)]
enum Column {
    /// Each value as the binding converts any value, or as a tagged `Value`.
    Values(Vec<Tagged>),
    Bools(Vec<Option<bool>>),
    Numbers(ArrayType, Vec<Option<f64>>),
    BigInts(Vec<Option<i64>>),
    /// The values' texts end to end, and where each ends, `None` for NULL.
    Texts {
        built: Built,
        text: String,
        ends: Vec<Option<usize>>,
    },
    /// Each instant's nanoseconds since the epoch.
    Instants(Vec<Option<i128>>),
    /// The documents as the text of one JSON array per segment, NULL as
    /// `null`, and whether any holds an integer past 2^53 - 1.
    Json {
        segments: Vec<String>,
        wide: bool,
    },
}

/// What JavaScript makes of a text column's values.
#[derive(Debug, Clone, Copy)]
enum Built {
    String,
    Label,
    Decimal,
    Uuid,
}

impl Column {
    /// The column a result column of `kind` decodes into, its values plain
    /// unless `tagged`.
    fn of(kind: &Tag, tagged: bool, rows: usize) -> Self {
        let texts = |built| Self::Texts {
            built,
            text: String::new(),
            ends: Vec::with_capacity(rows),
        };
        match kind {
            _ if tagged => Self::Values(Vec::with_capacity(rows)),
            Tag::Enum(_) => texts(Built::Label),
            Tag::Scalar(ArrayType::String) => texts(Built::String),
            Tag::Scalar(ArrayType::Decimal) => texts(Built::Decimal),
            Tag::Scalar(ArrayType::Uuid) => texts(Built::Uuid),
            Tag::Scalar(ArrayType::Bool) => Self::Bools(Vec::with_capacity(rows)),
            Tag::Scalar(
                kind @ (ArrayType::TinyInt
                | ArrayType::SmallInt
                | ArrayType::Int
                | ArrayType::Unsigned
                | ArrayType::Float
                | ArrayType::Double),
            ) => Self::Numbers(kind.clone(), Vec::with_capacity(rows)),
            Tag::Scalar(ArrayType::BigInt) => Self::BigInts(Vec::with_capacity(rows)),
            Tag::Scalar(ArrayType::DateTimeWithTimeZone) => {
                Self::Instants(Vec::with_capacity(rows))
            }
            Tag::Scalar(ArrayType::Json) => Self::Json {
                segments: vec![String::from("[")],
                wide: false,
            },
            _ => Self::Values(Vec::with_capacity(rows)),
        }
    }

    /// Decode one value of a column of type `ty` into the column, by the
    /// codec `decode::decode` reads a value of its kind with.
    fn read(&mut self, ty: &Type, raw: Option<&[u8]>) -> Result<(), CodecError> {
        let base = base_type(ty);
        match self {
            Self::Values(values) => values.push(decode::decode(ty, raw)?),
            Self::Bools(values) => {
                values.push(raw.map(|raw| bool::from_sql(base, raw)).transpose()?)
            }
            Self::Numbers(kind, values) => {
                let kind = &*kind;
                values.push(raw.map(|raw| number(base, kind, raw)).transpose()?);
            }
            Self::BigInts(values) => {
                values.push(raw.map(|raw| i64::from_sql(base, raw)).transpose()?)
            }
            Self::Instants(values) => values.push(
                raw.map(|raw| {
                    Timestamp::from_sql(base, raw).map(|instant| instant.as_nanosecond())
                })
                .transpose()?,
            ),
            Self::Texts { built, text, ends } => {
                let Some(raw) = raw else {
                    ends.push(None);
                    return Ok(());
                };
                match built {
                    Built::String => text.push_str(<&str>::from_sql(base, raw)?),
                    Built::Label => text.push_str(decode::label(raw)?),
                    Built::Decimal => write!(text, "{}", ExactDecimal::from_sql(base, raw)?.0)?,
                    Built::Uuid => text.push_str(
                        Uuid::from_sql(base, raw)?
                            .hyphenated()
                            .encode_lower(&mut Uuid::encode_buffer()),
                    ),
                }
                ends.push(Some(text.len()));
            }
            Self::Json { segments, wide } => {
                let json = raw
                    .map(|raw| ExactJsonText::from_sql(base, raw))
                    .transpose()?;
                if segments
                    .last()
                    .is_none_or(|segment| segment.len() > SEGMENT)
                {
                    if let Some(full) = segments.last_mut() {
                        full.push(']');
                    }
                    segments.push(String::from("["));
                }
                let Some(segment) = segments.last_mut() else {
                    return Ok(());
                };
                if segment.len() > 1 {
                    segment.push(',');
                }
                match json {
                    Some(json) => {
                        json.push_to(segment);
                        *wide |= json.wide_integers();
                    }
                    None => segment.push_str("null"),
                }
            }
        }
        Ok(())
    }
}

/// A number of `kind` as JavaScript's number, which holds each exactly.
fn number(ty: &Type, kind: &ArrayType, raw: &[u8]) -> Result<f64, CodecError> {
    Ok(match kind {
        ArrayType::TinyInt => f64::from(i8::from_sql(ty, raw)?),
        ArrayType::SmallInt => f64::from(i16::from_sql(ty, raw)?),
        ArrayType::Int => f64::from(i32::from_sql(ty, raw)?),
        ArrayType::Unsigned => f64::from(u32::from_sql(ty, raw)?),
        ArrayType::Float => f64::from(f32::from_sql(ty, raw)?),
        _ => f64::from_sql(ty, raw)?,
    })
}

/// A statement's rows decoded, each value plain or, where `tagged`, a value
/// with its kind.
// [spec:pgorm:req:bindings.rows]
pub(crate) fn decode(rows: &[Row], tagged: bool) -> Result<Decoded, Failure> {
    let Some(first) = rows.first() else {
        return Ok(Decoded::default());
    };
    let names = decode::names(first)?;
    let mut kinds = Vec::with_capacity(names.len());
    let mut columns: Vec<Column> = Vec::with_capacity(names.len());
    for (at, row) in rows.iter().enumerate() {
        for (index, description) in row.columns().iter().enumerate() {
            let ty = description.type_();
            let undecodable = |error| decode::undecodable(row, index, &error);
            if at == 0 {
                let kind = decode::kind(ty).map_err(undecodable)?;
                columns.push(Column::of(&kind, tagged, rows.len()));
                kinds.push(kind);
            }
            let raw = decode::raw(row, index)?;
            if let Some(column) = columns.get_mut(index) {
                column.read(ty, raw).map_err(undecodable)?;
            }
        }
    }
    for column in &mut columns {
        if let Column::Json { segments, .. } = column
            && let Some(last) = segments.last_mut()
        {
            last.push(']');
        }
    }
    Ok(Decoded {
        names,
        kinds,
        columns,
        count: rows.len(),
        tagged,
    })
}

fn index(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// One value as JavaScript: plain, or the module's `Value` when the caller
/// asked for tagged rows.
pub(crate) fn value<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    value: Tagged,
    tagged: bool,
) -> JsResult<'cx, JsValue> {
    if tagged {
        let boxed = cx.boxed(value);
        codec.make(cx, ("value", boxed))
    } else {
        write::plain(cx, codec, &value)
    }
}

/// Each column's kind as `Tag::key` spells it, read off the first
/// row: every row of a column holds its type's kind, NULLs included.
pub(crate) fn kinds<'cx>(cx: &mut Cx<'cx>, row: Option<&[Tagged]>) -> JsResult<'cx, JsArray> {
    let row = row.unwrap_or_default();
    let kinds = JsArray::new(cx, row.len());
    for (at, value) in row.iter().enumerate() {
        let kind = value.tag.key();
        let kind = cx.string(kind);
        kinds.set(cx, index(at), kind)?;
    }
    Ok(kinds)
}

/// An array built a block of values at a time by `gather`.
struct Gathered<'cx> {
    gather: Handle<'cx, JsFunction>,
    /// The array so far, `null` before the first block, then the block.
    args: Vec<Handle<'cx, JsValue>>,
}

impl<'cx> Gathered<'cx> {
    fn new(cx: &mut Cx<'cx>, gather: Handle<'cx, JsFunction>, values: usize) -> Self {
        let mut args = Vec::with_capacity(values.min(BLOCK) + 1);
        args.push(cx.null().upcast());
        Self { gather, args }
    }

    fn push(&mut self, cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<()> {
        self.args.push(value);
        if self.args.len() > BLOCK {
            self.flush(cx)?;
        }
        Ok(())
    }

    fn flush(&mut self, cx: &mut Cx<'cx>) -> NeonResult<()> {
        let this = cx.undefined();
        let array = self.gather.call(cx, this, &self.args)?;
        self.args.clear();
        self.args.push(array);
        Ok(())
    }

    fn finish(mut self, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        self.flush(cx)?;
        Ok(self.args[0])
    }
}

/// Each of `values` made by `make`, or `null`, gathered into one array.
fn each<'cx, T>(
    cx: &mut Cx<'cx>,
    gather: Handle<'cx, JsFunction>,
    values: Vec<Option<T>>,
    make: impl Fn(&mut Cx<'cx>, T) -> Handle<'cx, JsValue>,
) -> NeonResult<Gathered<'cx>> {
    let mut array = Gathered::new(cx, gather, values.len());
    let null = cx.null().upcast();
    for value in values {
        let value = match value {
            Some(value) => make(cx, value),
            None => null,
        };
        array.push(cx, value)?;
    }
    Ok(array)
}

/// A column as the array `finish` builds its values in, and what it builds
/// them as: nothing, `decimal`, `uuid`, `instant`, or `json`, `json-wide`
/// where a document holds an integer past 2^53 - 1.
fn column<'cx>(
    cx: &mut Cx<'cx>,
    codec: Codec<'cx>,
    gather: Handle<'cx, JsFunction>,
    column: Column,
    tagged: bool,
) -> NeonResult<(Handle<'cx, JsValue>, &'static str)> {
    let (array, built) = match column {
        Column::Values(values) => {
            let mut array = Gathered::new(cx, gather, values.len());
            for item in values {
                let item = value(cx, codec, item, tagged)?;
                array.push(cx, item)?;
            }
            (array, "")
        }
        Column::Bools(values) => {
            let (yes, no) = (cx.boolean(true).upcast(), cx.boolean(false).upcast());
            (
                each(
                    cx,
                    gather,
                    values,
                    move |_, flag| if flag { yes } else { no },
                )?,
                "",
            )
        }
        Column::Numbers(_, values) => (
            each(cx, gather, values, |cx, number| cx.number(number).upcast())?,
            "",
        ),
        Column::BigInts(values) => (
            each(cx, gather, values, |cx, number| {
                JsBigInt::from_i64(cx, number).upcast()
            })?,
            "",
        ),
        Column::Instants(values) => (
            each(cx, gather, values, |cx, nanoseconds| {
                JsBigInt::from_i128(cx, nanoseconds).upcast()
            })?,
            "instant",
        ),
        Column::Texts { built, text, ends } => {
            let mut array = Gathered::new(cx, gather, ends.len());
            let null = cx.null().upcast();
            let mut start = 0;
            for end in ends {
                let item = match end {
                    Some(end) => {
                        let item = cx.string(text.get(start..end).unwrap_or_default());
                        start = end;
                        item.upcast()
                    }
                    None => null,
                };
                array.push(cx, item)?;
            }
            let built = match built {
                Built::String | Built::Label => "",
                Built::Decimal => "decimal",
                Built::Uuid => "uuid",
            };
            (array, built)
        }
        Column::Json { segments, wide } => {
            let array = JsArray::new(cx, segments.len());
            for (at, segment) in segments.iter().enumerate() {
                let segment = cx.string(segment);
                array.set(cx, index(at), segment)?;
            }
            return Ok((array.upcast(), if wide { "json-wide" } else { "json" }));
        }
    };
    Ok((array.finish(cx)?, built))
}

/// `[names, columns, kinds, count]`: each column an array of its values in
/// row order, and each column's kind for a model to hold to its declaration.
// [spec:pgorm:req:bindings.rows]
// [spec:pgorm:req:napi.records]
// [spec:pgorm:req:bindings.model-records+2]
pub(crate) fn to_js<'cx>(cx: &mut Cx<'cx>, decoded: Decoded) -> JsResult<'cx, JsValue> {
    let codec = Codec::get(cx)?;
    let Columns { gather, finish } = Columns::get(cx)?;
    let names = JsArray::new(cx, decoded.names.len());
    for (at, name) in decoded.names.iter().enumerate() {
        let name = cx.string(name);
        names.set(cx, index(at), name)?;
    }
    let kinds = JsArray::new(cx, decoded.kinds.len());
    for (at, kind) in decoded.kinds.iter().enumerate() {
        let kind = cx.string(kind.key());
        kinds.set(cx, index(at), kind)?;
    }
    let columns = JsArray::new(cx, decoded.columns.len());
    let mut builds = Vec::with_capacity(decoded.columns.len());
    for (at, item) in decoded.columns.into_iter().enumerate() {
        let (item, built) = column(cx, codec, gather, item, decoded.tagged)?;
        columns.set(cx, index(at), item)?;
        builds.push(built);
    }
    if builds.iter().any(|built| !built.is_empty()) {
        let built = JsArray::new(cx, builds.len());
        for (at, item) in builds.into_iter().enumerate() {
            let item = cx.string(item);
            built.set(cx, index(at), item)?;
        }
        let this = cx.undefined();
        let args: [Handle<JsValue>; 2] = [columns.upcast(), built.upcast()];
        finish.call(cx, this, args)?;
    }
    #[allow(clippy::cast_precision_loss)]
    let count = cx.number(decoded.count as f64);
    let result = JsArray::new(cx, 4);
    result.set(cx, 0, names)?;
    result.set(cx, 1, columns)?;
    result.set(cx, 2, kinds)?;
    result.set(cx, 3, count)?;
    Ok(result.upcast())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(column: &mut Column, ty: &Type, values: &[Option<&[u8]>]) -> Result<(), CodecError> {
        for raw in values {
            column.read(ty, *raw)?;
        }
        Ok(())
    }

    // [spec:pgorm:req:bindings.rows/test]
    #[test]
    fn a_json_column_crosses_as_one_array() -> Result<(), CodecError> {
        let mut column = Column::of(&Tag::Scalar(ArrayType::Json), false, 3);
        read(
            &mut column,
            &Type::JSONB,
            &[
                Some(b"\x01{\"b\": 1, \"a\": [2]}"),
                None,
                Some(b"\x0118446744073709551615"),
            ],
        )?;
        let Column::Json { segments, wide } = column else {
            return Err("not a JSON column".into());
        };
        assert_eq!(segments, ["[{\"a\":[2],\"b\":1},null,18446744073709551615"]);
        assert!(wide);
        Ok(())
    }

    // [spec:pgorm:req:bindings.rows/test]
    #[test]
    fn a_text_column_holds_values_end_to_end() -> Result<(), CodecError> {
        let mut column = Column::of(&Tag::Scalar(ArrayType::Uuid), false, 2);
        let uuid = Uuid::from_u128(0xa0ee_bc99_9c0b_4ef8_bb6d_6bb9_bd38_0a11);
        read(&mut column, &Type::UUID, &[Some(uuid.as_bytes()), None])?;
        let Column::Texts { text, ends, .. } = column else {
            return Err("not a text column".into());
        };
        assert_eq!(text, "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11");
        assert_eq!(ends, [Some(36), None]);
        let mut tagged = Column::of(&Tag::Scalar(ArrayType::Uuid), true, 1);
        read(&mut tagged, &Type::UUID, &[Some(uuid.as_bytes())])?;
        assert!(matches!(tagged, Column::Values(values) if values.len() == 1));
        Ok(())
    }
}
