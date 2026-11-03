//! Each row of a disjoint read, judged by pgorm's shape and read back by the
//! codec, and handed to JavaScript.

use std::collections::HashMap;

use neon::prelude::*;
use pgorm::{QueryResult, pgorm_query::ColumnType};
use tokio_postgres::Row;

use super::{Core, RowCodec};
use crate::{
    codec::Codec,
    connect::job::Settled,
    decode,
    entities::{Model, record},
    errors::Failure,
    rows,
    values::{Tag, Tagged, TypeName},
};

/// One decoded row: a model's values beside the index of its kind and the
/// decoded source its kind's row is under, which `lib/disjoint.js` views by
/// field, or a registration's base model, kind and kind model.
enum Item {
    Record {
        values: Vec<Tagged>,
        kind: usize,
        source: Option<usize>,
    },
    Entity {
        base: Model,
        kind: String,
        row: Option<Model>,
    },
}

/// A read's rows, with the result's column names.
pub(crate) struct Rows {
    names: Vec<String>,
    items: Vec<Item>,
    entities: bool,
}

/// A count a read of `count_statement` gave.
pub(crate) struct Count(pub(crate) i64);

impl Settled for Count {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        if self.0 > (1 << 53) - 1 {
            let error = Failure::Decode(format!(
                "{} rows is past the integers a number holds exactly",
                self.0
            ))
            .into_js(cx)?;
            return cx.throw(error);
        }
        Ok(cx.number(self.0 as f64).upcast())
    }
}

impl Rows {
    /// Judge each of `rows` by its kind and read it back.
    // [spec:pgorm:req:bindings.disjoint+1]
    pub(crate) fn of(core: &Core, rows: Vec<Row>) -> Result<Self, Failure> {
        let names = match rows.first() {
            Some(row) => decode::names(row)?,
            None => Vec::new(),
        };
        let at: HashMap<&str, usize> = names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.as_str(), index))
            .collect();
        let enums = enum_columns(core, &at);
        let items = rows
            .into_iter()
            .map(|row| item(core, row, &at, &enums))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            names,
            items,
            entities: matches!(core.codec, RowCodec::Entities { .. }),
        })
    }
}

/// Where each enum column of every source is in the result, and its type:
/// the read projects one as its text, as pgorm's `ColumnTrait::select_as`
/// reads an entity's, and a model's field is the enum its column names.
fn enum_columns(core: &Core, at: &HashMap<&str, usize>) -> Vec<(usize, Tag)> {
    let shape = &core.shape;
    let tables = std::iter::once((0, shape.base()))
        .chain(shape.sources().map(|(source, _, table)| (source, table)));
    let mut found = Vec::new();
    for (source, table) in tables {
        let names = shape.result_names(source).unwrap_or_default();
        for (column, result_name) in table.columns().iter().zip(&names) {
            let (column_type, array) = match column.def().get_column_type() {
                ColumnType::Array(element) => (element.as_ref(), true),
                column_type => (column_type, false),
            };
            let ColumnType::Enum { name, schema, .. } = column_type else {
                continue;
            };
            let Some(index) = at.get(result_name.as_str()) else {
                continue;
            };
            let tag = Tag::Enum(TypeName {
                name: name.to_string(),
                schema: schema.as_ref().map(|schema| schema.to_string()),
            });
            found.push((
                *index,
                if array {
                    Tag::Array(Box::new(tag))
                } else {
                    tag
                },
            ));
        }
    }
    found
}

fn item(
    core: &Core,
    row: Row,
    at: &HashMap<&str, usize>,
    enums: &[(usize, Tag)],
) -> Result<Item, Failure> {
    let shape = &core.shape;
    match &core.codec {
        RowCodec::Models => {
            let mut values = decode::row(&row)?;
            for (index, tag) in enums {
                if let Some(value) = values.get_mut(*index) {
                    value.tag = tag.clone();
                }
            }
            let key = || {
                let prefix = pgorm::source_prefix(0);
                shape
                    .base()
                    .key()
                    .map(|column| {
                        at.get(
                            pgorm::result_column_name(&prefix, &column.name().to_string()).as_str(),
                        )
                        .and_then(|index| values.get(*index))
                        .map(|value| value.datum.to_string())
                        .unwrap_or_default()
                    })
                    .collect()
            };
            let judged = shape
                .judge(&QueryResult::from(row), key)
                .map_err(|error| Failure::Decode(error.to_string()))?;
            Ok(Item::Record {
                values,
                kind: judged.kind,
                source: judged.source,
            })
        }
        RowCodec::Entities { base, kinds } => {
            let result = QueryResult::from(row);
            let model = base
                .decode(&result, &pgorm::source_prefix(0))
                .map_err(|error| Failure::Decode(error.to_string()))?;
            let key = || {
                shape
                    .base()
                    .key()
                    .map(|column| {
                        model
                            .get(&column.name().to_string())
                            .map(|value| value.to_string())
                            .unwrap_or_default()
                    })
                    .collect()
            };
            let judged = shape
                .judge(&result, key)
                .map_err(|error| Failure::Decode(error.to_string()))?;
            let row = match (judged.source, &kinds[judged.kind]) {
                (Some(source), Some(entity)) => Some(
                    entity
                        .decode(&result, &pgorm::source_prefix(source))
                        .map_err(|error| Failure::Decode(error.to_string()))?,
                ),
                _ => None,
            };
            Ok(Item::Entity {
                base: model,
                kind: shape.kinds()[judged.kind].value().to_owned(),
                row,
            })
        }
    }
}

fn index(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// A model's rows are `[names, rows, kinds, judged]`, each row its values in
/// column order and each judgment `[kind index, source or null]`; a
/// registration's are `[[values, model], kind, [values, model] or null]`
/// each.
// [spec:pgorm:req:bindings.disjoint+1]
impl Settled for Rows {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        let Self {
            names,
            items,
            entities,
        } = *self;
        let codec = Codec::get(cx)?;
        let first = items.first().and_then(|item| match item {
            Item::Record { values, .. } => Some(values.clone()),
            Item::Entity { .. } => None,
        });
        let rows = JsArray::new(cx, items.len());
        let judged = JsArray::new(cx, items.len());
        for (at, item) in items.into_iter().enumerate() {
            match item {
                Item::Record {
                    values,
                    kind,
                    source,
                } => {
                    let array = JsArray::new(cx, values.len());
                    for (column, value) in values.into_iter().enumerate() {
                        let value = rows::value(cx, codec, value, false)?;
                        array.set(cx, index(column), value)?;
                    }
                    rows.set(cx, index(at), array)?;
                    let judgment = JsArray::new(cx, 2);
                    let kind = cx.number(kind as f64);
                    judgment.set(cx, 0, kind)?;
                    let source: Handle<JsValue> = match source {
                        Some(source) => cx.number(source as f64).upcast(),
                        None => cx.null().upcast(),
                    };
                    judgment.set(cx, 1, source)?;
                    judged.set(cx, index(at), judgment)?;
                }
                Item::Entity { base, kind, row } => {
                    let triple = JsArray::new(cx, 3);
                    let base = record(cx, base)?;
                    triple.set(cx, 0, base)?;
                    let kind = cx.string(kind);
                    triple.set(cx, 1, kind)?;
                    let row: Handle<JsValue> = match row {
                        Some(row) => record(cx, row)?,
                        None => cx.null().upcast(),
                    };
                    triple.set(cx, 2, row)?;
                    rows.set(cx, index(at), triple)?;
                }
            }
        }
        if entities {
            return Ok(rows.upcast());
        }
        let list = JsArray::new(cx, names.len());
        for (at, name) in names.iter().enumerate() {
            let name = cx.string(name);
            list.set(cx, index(at), name)?;
        }
        let kinds = rows::kinds(cx, first.as_deref())?;
        let result = JsArray::new(cx, 4);
        result.set(cx, 0, list)?;
        result.set(cx, 1, rows)?;
        result.set(cx, 2, kinds)?;
        result.set(cx, 3, judged)?;
        Ok(result.upcast())
    }
}
