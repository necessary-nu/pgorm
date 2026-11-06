//! A declared model's table as pgorm's `TableShape`, which its schema and a
//! disjoint model's tables are written from.

use std::sync::Arc;

use neon::{prelude::*, types::Finalize};
use pgorm::pgorm_query::{ColumnType, Expr, GeneratedKind, Name, TableName, TypeName};
use pgorm::{
    CaseStyle, ColumnDef, ColumnPairs, ColumnTypeTrait, RelationDef, Schema, TableColumn,
    TableShape,
};

use crate::{
    codec::Codec,
    entities::{EntitySchema, schema_parts},
    errors::{Failure, throw},
    statements::{FOREIGN_KEY_ACTIONS, Node, data_type, expression, operand, pick},
    values::{Tag, read},
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("modelSchema", model_schema)?;
    cx.export_function("modelTable", model_table)?;
    cx.export_function("modelRenamed", model_renamed)?;
    cx.export_function("modelCasts", model_casts)?;
    cx.export_function("modelCastSelect", cast_select)?;
    cx.export_function("modelCastSave", cast_save)?;
    cx.export_function("modelCastComparedColumn", cast_compared_column)?;
    cx.export_function("modelCastComparedValue", cast_compared_value)?;
    cx.export_function("modelCastComparesAsEnum", cast_compares_as_enum)?;
    Ok(())
}

/// The column type a model column of `tag` is written with: its
/// `columnType` where it declares one, made an array where the column is;
/// otherwise a scalar kind's own (`ArrayType::column_type`), an enum of the
/// labels `values` lists, a created range over its subtype's, and an array of
/// its element's.
// [spec:pgorm:req:bindings.schema-from-model]
pub(crate) fn column_type(
    tag: &Tag,
    values: &[String],
    declared: Option<&ColumnType>,
) -> Result<ColumnType, Failure> {
    if let Some(declared) = declared {
        return Ok(match tag {
            Tag::Array(_) => ColumnType::Array(Arc::new(declared.clone())),
            _ => declared.clone(),
        });
    }
    Ok(match tag {
        Tag::Scalar(kind) => kind.column_type().ok_or_else(|| {
            Failure::Construction(format!(
                "no column decodes as `{}`, so a model's column cannot be one",
                kind.kind_name()
            ))
        })?,
        Tag::Enum(name) => ColumnType::Enum {
            name: Name::runtime(&name.name),
            schema: name.schema.as_ref().map(Name::runtime),
            variants: values.iter().map(Name::runtime).collect(),
        },
        Tag::Created(kind) => {
            let subtype = Arc::new(column_type(&Tag::Scalar(kind.subtype.clone()), &[], None)?);
            let (name, schema) = (
                Name::runtime(&kind.name.name),
                kind.name.schema.as_ref().map(Name::runtime),
            );
            match kind.multirange {
                true => ColumnType::CreatedMultirange {
                    name,
                    schema,
                    subtype,
                },
                false => ColumnType::CreatedRange {
                    name,
                    schema,
                    subtype,
                },
            }
        }
        Tag::Array(element) => ColumnType::Array(Arc::new(column_type(element, values, None)?)),
    })
}

pub(crate) fn property<'cx>(
    cx: &mut Cx<'cx>,
    object: Handle<'cx, JsObject>,
    key: &str,
) -> NeonResult<Handle<'cx, JsValue>> {
    object.get_value(cx, key)
}

fn missing(cx: &mut Cx, value: Handle<JsValue>) -> bool {
    value.is_a::<JsNull, _>(cx) || value.is_a::<JsUndefined, _>(cx)
}

pub(crate) fn text(cx: &mut Cx, value: Handle<JsValue>, what: &str) -> NeonResult<String> {
    match value.downcast::<JsString, _>(cx) {
        Ok(text) => Ok(text.value(cx)),
        Err(_) => cx.throw_type_error(format!("a described table's {what} is a string")),
    }
}

pub(crate) fn optional_text(
    cx: &mut Cx,
    value: Handle<JsValue>,
    what: &str,
) -> NeonResult<Option<String>> {
    if missing(cx, value) {
        return Ok(None);
    }
    text(cx, value, what).map(Some)
}

pub(crate) fn flag(cx: &mut Cx, value: Handle<JsValue>) -> NeonResult<bool> {
    match value.downcast::<JsBoolean, _>(cx) {
        Ok(flag) => Ok(flag.value(cx)),
        Err(_) => cx.throw_type_error("a described column's flags are booleans"),
    }
}

/// A list of names a described table gives, each a string.
pub(crate) fn names<'cx>(
    cx: &mut Cx<'cx>,
    list: Handle<'cx, JsValue>,
    what: &str,
) -> NeonResult<Vec<String>> {
    let Ok(list) = list.downcast::<JsArray, _>(cx) else {
        return cx.throw_type_error(format!("a described table's {what} is an array"));
    };
    list.to_vec(cx)?
        .into_iter()
        .map(|name| text(cx, name, what))
        .collect()
}

/// The type a column declares with `columnType`: any type a `ColumnDef`
/// takes but an array, since an array column's is its elements'.
// [spec:pgorm:req:bindings.schema-from-model]
fn declared_type<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<Option<ColumnType>> {
    if missing(cx, value) {
        return Ok(None);
    }
    let declared = data_type(cx, value)?;
    if matches!(declared, ColumnType::Array(_)) {
        return read::refuse(
            cx,
            "a column's columnType is the type of its values, an array column's its elements': \
             declare the array with array: true",
        );
    }
    Ok(Some(declared))
}

/// What fills a column an insert leaves out, as `def` writes it: a stated
/// default, an identity or a generation expression.
// [spec:pgorm:req:bindings.schema-from-model]
fn filled<'cx>(
    cx: &mut Cx<'cx>,
    column: Handle<'cx, JsObject>,
    mut def: ColumnDef,
) -> NeonResult<ColumnDef> {
    let default_value = property(cx, column, "defaultValue")?;
    if !missing(cx, default_value) {
        def = def.default(operand(cx, default_value)?);
    }
    let default_expr = property(cx, column, "defaultExpr")?;
    if !missing(cx, default_expr) {
        def = def.default(expression(cx, default_expr)?);
    }
    for (key, kind) in [
        ("generatedStored", GeneratedKind::Stored),
        ("generatedVirtual", GeneratedKind::Virtual),
    ] {
        let generation = property(cx, column, key)?;
        if !missing(cx, generation) {
            return Ok(def.generated(expression(cx, generation)?, kind));
        }
    }
    let generated = property(cx, column, "generated")?;
    Ok(
        match optional_text(cx, generated, "generated")?.as_deref() {
            Some("always") => def.identity(),
            Some("byDefault") => def.identity_by_default(),
            Some(_) => {
                return cx.throw_type_error(r#"a column is generated "always" or "byDefault""#);
            }
            None => def,
        },
    )
}

/// One model column, as JavaScript describes it: its name, its definition,
/// and whether the database fills it by a default the declaration does not
/// state.
// [spec:pgorm:req:bindings.schema-from-model]
pub(crate) fn column_of<'cx>(
    cx: &mut Cx<'cx>,
    column: Handle<'cx, JsValue>,
) -> NeonResult<(String, ColumnDef, bool)> {
    let Ok(column) = column.downcast::<JsObject, _>(cx) else {
        return cx.throw_type_error("a described column is an object");
    };
    let name = property(cx, column, "name")?;
    let name = text(cx, name, "column name")?;
    let codec = Codec::get(cx)?;
    let kind = property(cx, column, "kind")?;
    let kind = read::kind(cx, codec, kind)?;
    let array = property(cx, column, "array")?;
    let kind = match flag(cx, array)? {
        true => Tag::Array(Box::new(kind)),
        false => kind,
    };
    let values = property(cx, column, "values")?;
    let values: Vec<String> = match values.downcast::<JsArray, _>(cx) {
        Ok(values) => values
            .to_vec(cx)?
            .into_iter()
            .map(|value| text(cx, value, "enum value"))
            .collect::<NeonResult<_>>()?,
        Err(_) => Vec::new(),
    };
    let declared = property(cx, column, "columnType")?;
    let declared = declared_type(cx, declared)?;
    if let Some(declared) = &declared
        && let Err(failure) = refuse_declared(&kind, declared)
    {
        return throw(cx, failure);
    }
    let mut def = match column_type(&kind, &values, declared.as_ref()) {
        Ok(column_type) => column_type.def(),
        Err(failure) => return throw(cx, failure),
    };
    let nullable = property(cx, column, "nullable")?;
    if flag(cx, nullable)? {
        def = def.nullable();
    }
    let indexed = property(cx, column, "indexed")?;
    if flag(cx, indexed)? {
        def = def.indexed();
    }
    let comment = property(cx, column, "comment")?;
    if let Some(comment) = optional_text(cx, comment, "column's comment")? {
        def = def.comment(&comment);
    }
    let def = filled(cx, column, def)?;
    let default = property(cx, column, "default")?;
    let unstated = flag(cx, default)?;
    Ok((name, def, unstated))
}

/// Refuse a `columnType` the kind cannot be written as: a created range's
/// is its own, and an enum's is a domain over that enum alone, which the
/// column is read and compared as.
// [spec:pgorm:req:bindings.model-casts]
fn refuse_declared(tag: &Tag, declared: &ColumnType) -> Result<(), Failure> {
    let element = match tag {
        Tag::Array(element) => element,
        other => other,
    };
    match element {
        Tag::Created(_) => Err(Failure::Construction(
            "a created range column's type is its kind's own: it takes no columnType".to_owned(),
        )),
        Tag::Enum(name) => {
            let mut enum_type = TypeName::new(Name::runtime(&name.name));
            if let Some(schema) = &name.schema {
                enum_type = enum_type.schema(Name::runtime(schema));
            }
            let column = TableColumn::new("column", declared.clone().def());
            match column.compares_as() == Some(enum_type) {
                true => Ok(()),
                false => Err(Failure::Construction(
                    "an enum column's columnType is a domain over its enum".to_owned(),
                )),
            }
        }
        _ => Ok(()),
    }
}

/// A unique key a described table gives: `{ name, columns,
/// nullsNotDistinct }`, its name null for PostgreSQL to choose one.
pub(crate) type DescribedKey = (Option<String>, Vec<String>, bool);

// [spec:pgorm:req:bindings.model-unique-keys+2]
pub(crate) fn unique_keys<'cx>(
    cx: &mut Cx<'cx>,
    table: Handle<'cx, JsObject>,
) -> NeonResult<Vec<DescribedKey>> {
    let keys = property(cx, table, "uniqueKeys")?;
    let Ok(keys) = keys.downcast::<JsArray, _>(cx) else {
        return cx.throw_type_error("a described table's unique keys are an array");
    };
    keys.to_vec(cx)?
        .into_iter()
        .map(|key| {
            let Ok(key) = key.downcast::<JsObject, _>(cx) else {
                return cx.throw_type_error("a described unique key is an object");
            };
            let name = property(cx, key, "name")?;
            let name = optional_text(cx, name, "unique key's name")?;
            let columns = property(cx, key, "columns")?;
            let columns = names(cx, columns, "unique key's columns")?;
            let nulls_not_distinct = property(cx, key, "nullsNotDistinct")?;
            Ok((name, columns, flag(cx, nulls_not_distinct)?))
        })
        .collect()
}

/// The table a described model or relation names, schema-qualified where it
/// is.
pub(crate) fn table_name<'cx>(
    cx: &mut Cx<'cx>,
    table: Handle<'cx, JsObject>,
) -> NeonResult<TableName> {
    let schema = property(cx, table, "schema")?;
    let schema = optional_text(cx, schema, "schema")?;
    let name = property(cx, table, "table")?;
    let name = text(cx, name, "table")?;
    Ok(match schema {
        Some(schema) => TableName::SchemaTable(Name::runtime(schema), Name::runtime(name)),
        None => TableName::Table(Name::runtime(name)),
    })
}

/// The columns of a described table, each in field order.
pub(crate) fn columns<'cx>(
    cx: &mut Cx<'cx>,
    table: Handle<'cx, JsObject>,
) -> NeonResult<Vec<(String, ColumnDef, bool)>> {
    let columns = property(cx, table, "columns")?;
    let Ok(columns) = columns.downcast::<JsArray, _>(cx) else {
        return cx.throw_type_error("a described table's columns are an array");
    };
    columns
        .to_vec(cx)?
        .into_iter()
        .map(|column| column_of(cx, column))
        .collect()
}

fn refused<'cx, T>(cx: &mut Cx<'cx>, result: Result<T, pgorm::Error>) -> NeonResult<T> {
    result.or_else(|error| throw(cx, Failure::Construction(error.to_string())))
}

/// One model's table, as JavaScript describes it: its schema, its name, its
/// columns in field order, its key's columns and how the key is generated,
/// its unique keys and its comment.
// [spec:pgorm:req:bindings.schema-from-model]
// [spec:pgorm:req:bindings.model-unique-keys+2]
fn shape_of<'cx>(cx: &mut Cx<'cx>, table: Handle<'cx, JsValue>) -> NeonResult<TableShape> {
    let Ok(table) = table.downcast::<JsObject, _>(cx) else {
        return cx.throw_type_error("a described table is an object");
    };
    let columns = columns(cx, table)?
        .into_iter()
        .map(|(name, def, unstated)| {
            let column = TableColumn::new(name, def);
            match unstated {
                true => column.with_unstated_default(),
                false => column,
            }
        })
        .collect();
    let name = table_name(cx, table)?;
    let mut shape = refused(cx, TableShape::new(name, columns))?;
    let key = property(cx, table, "key")?;
    let key = names(cx, key, "key")?;
    if !key.is_empty() {
        let key: Vec<&str> = key.iter().map(String::as_str).collect();
        shape = refused(cx, shape.with_primary_key(&key))?;
    }
    let auto_increment = property(cx, table, "autoIncrement")?;
    if flag(cx, auto_increment)? {
        shape = shape.auto_increment();
    }
    let without_overlaps = property(cx, table, "withoutOverlaps")?;
    if flag(cx, without_overlaps)? {
        shape = shape.without_overlaps();
    }
    for (name, columns, nulls_not_distinct) in unique_keys(cx, table)? {
        let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
        let keyed = shape.with_unique_key(name.as_deref(), &columns, nulls_not_distinct);
        shape = refused(cx, keyed)?;
    }
    let comment = property(cx, table, "comment")?;
    if let Some(comment) = optional_text(cx, comment, "comment")? {
        shape = shape.with_comment(comment);
    }
    Ok(shape)
}

/// `shape` with the foreign key of one `belongsTo` relation, as JavaScript
/// describes it: the table it reaches, the columns it pairs at each end, its
/// actions and its constraint's name.
// [spec:pgorm:req:bindings.schema-from-model]
fn with_relation<'cx>(
    cx: &mut Cx<'cx>,
    shape: TableShape,
    relation: Handle<'cx, JsValue>,
) -> NeonResult<TableShape> {
    let Ok(relation) = relation.downcast::<JsObject, _>(cx) else {
        return cx.throw_type_error("a described relation is an object");
    };
    let from = property(cx, relation, "from")?;
    let from = names(cx, from, "relation's columns")?;
    let to = property(cx, relation, "to")?;
    let to = names(cx, to, "relation's columns")?;
    let target = table_name(cx, relation)?;
    let (Some(first_from), Some(first_to)) = (from.first(), to.first()) else {
        return read::refuse(cx, "a relation pairs at least one column");
    };
    let mut pairs = ColumnPairs::new(Name::runtime(first_from), Name::runtime(first_to));
    for (from, to) in from.iter().zip(&to).skip(1) {
        pairs.push(Name::runtime(from), Name::runtime(to));
    }
    let label = format!("{}({})", target.table().to_string(), from.join(", "));
    let mut def = RelationDef::belongs_to(shape.table().clone(), target, pairs);
    for (key, what) in [
        ("onDelete", "an ON DELETE action"),
        ("onUpdate", "an ON UPDATE action"),
    ] {
        let action = property(cx, relation, key)?;
        let action = match missing(cx, action) {
            true => None,
            false => Some(pick(cx, action, what, FOREIGN_KEY_ACTIONS)?),
        };
        match key {
            "onDelete" => def.on_delete = action,
            _ => def.on_update = action,
        }
    }
    let fk_name = property(cx, relation, "fkName")?;
    def.fk_name = optional_text(cx, fk_name, "foreign key's name")?;
    refused(cx, shape.with_relation(label, def))
}

/// `modelTable(table)`: the table a model declares, checked as pgorm's
/// `TableShape` checks it, so what it cannot describe is refused as the
/// model is declared.
// [spec:pgorm:req:bindings.schema-from-model]
fn model_table(mut cx: FunctionContext) -> JsResult<JsValue> {
    let table = cx.argument::<JsValue>(0)?;
    shape_of(&mut cx, table)?;
    Ok(cx.undefined().upcast())
}

/// `modelSchema(table, relations)`: `[table, enums, indexes, comments]`, the
/// schema of the table a model declares with a foreign key for each
/// `belongsTo` relation in `relations`, as pgorm's `Schema` writes a
/// `TableShape`'s.
// [spec:pgorm:req:bindings.schema-from-model]
fn model_schema(mut cx: FunctionContext) -> JsResult<JsValue> {
    let table = cx.argument::<JsValue>(0)?;
    let mut shape = shape_of(&mut cx, table)?;
    for relation in cx.argument::<JsArray>(1)?.to_vec(&mut cx)? {
        shape = with_relation(&mut cx, shape, relation)?;
    }
    let schema = Schema::new();
    let table = refused(&mut cx, schema.create_table_from_shape(&shape))?;
    let parts = EntitySchema {
        table,
        enums: schema.create_enum_from_shape(&shape),
        indexes: schema.create_index_from_shape(&shape),
        comments: schema.create_comments_from_shape(&shape),
    };
    Ok(schema_parts(&mut cx, parts)?.upcast())
}

/// `modelRenamed(field, style)`: the column a model's `renameAll` names the
/// field `field`, as the entity derive's `rename_all` names a field's.
// [spec:pgorm:req:bindings.schema-from-model]
fn model_renamed(mut cx: FunctionContext) -> JsResult<JsValue> {
    let field = cx.argument::<JsString>(0)?.value(&mut cx);
    let style = cx.argument::<JsString>(1)?.value(&mut cx);
    match style.parse::<CaseStyle>() {
        Ok(style) => Ok(cx.string(style.convert(&field)).upcast()),
        Err(error) => cx.throw_type_error(error.to_string()),
    }
}

/// A model column's casts: pgorm's `TableColumn` for the column's type, its
/// `selectAs` and `saveAs`, which a model reads, writes and compares a column
/// declaring one of them through.
// [spec:pgorm:req:bindings.model-casts]
pub(crate) struct CastsHandle(TableColumn);

impl Finalize for CastsHandle {}

/// The type a `selectAs` or `saveAs` names, as a cast spells it.
// [spec:pgorm:req:bindings.model-casts]
fn cast_type<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<Option<TypeName>> {
    if missing(cx, value) {
        return Ok(None);
    }
    let declared = data_type(cx, value)?;
    match declared.cast_type() {
        Some(type_name) => Ok(Some(type_name)),
        None => read::refuse(
            cx,
            "no cast names this type: an interval restricted to fields, or bit with no length",
        ),
    }
}

/// `modelCasts(column)`: the casts of the column JavaScript describes.
// [spec:pgorm:req:bindings.model-casts]
fn model_casts(mut cx: FunctionContext) -> JsResult<JsBox<CastsHandle>> {
    let described = cx.argument::<JsValue>(0)?;
    let (name, def, _) = column_of(&mut cx, described)?;
    let Ok(object) = described.downcast::<JsObject, _>(&mut cx) else {
        return cx.throw_type_error("a described column is an object");
    };
    let mut column = TableColumn::new(name, def);
    let select_as = property(&mut cx, object, "selectAs")?;
    if let Some(type_name) = cast_type(&mut cx, select_as)? {
        column = column.with_select_as(type_name);
    }
    let save_as = property(&mut cx, object, "saveAs")?;
    if let Some(type_name) = cast_type(&mut cx, save_as)? {
        column = column.with_save_as(type_name);
    }
    Ok(cx.boxed(CastsHandle(column)))
}

/// A value the casts take: an expression as it is, a value as its bare
/// datum, the cast its kind would bind it under left to the column.
fn cast_operand<'cx>(cx: &mut Cx<'cx>, value: Handle<'cx, JsValue>) -> NeonResult<Expr> {
    if let Some(Node::Expr(expr)) = crate::statements::node(cx, value) {
        return Ok(Expr::expr(expr));
    }
    let codec = Codec::get(cx)?;
    match read::infer(cx, codec, value)? {
        Some(tagged) => Ok(Expr::val(tagged.datum)),
        None => Ok(Expr::expr(operand(cx, value)?)),
    }
}

fn casts<'cx>(cx: &mut FunctionContext<'cx>) -> NeonResult<Handle<'cx, JsBox<CastsHandle>>> {
    cx.argument::<JsBox<CastsHandle>>(0)
}

fn expr_node<'cx>(
    cx: &mut FunctionContext<'cx>,
    expr: pgorm::pgorm_query::SimpleExpr,
) -> JsResult<'cx, JsValue> {
    Ok(cx.boxed(Node::Expr(expr)).upcast())
}

/// `modelCastSelect(casts, expr)`: a read of the column as a projection
/// reads one of its type, `null` where it is read as it stands.
fn cast_select(mut cx: FunctionContext) -> JsResult<JsValue> {
    let handle = casts(&mut cx)?;
    let expr = cx.argument::<JsValue>(1)?;
    let expr = Expr::expr(expression(&mut cx, expr)?);
    match handle.0.select(expr) {
        Some(selected) => expr_node(&mut cx, selected),
        None => Ok(cx.null().upcast()),
    }
}

/// `modelCastSave(casts, value)`: a value as it is written to the column.
fn cast_save(mut cx: FunctionContext) -> JsResult<JsValue> {
    let handle = casts(&mut cx)?;
    let value = cx.argument::<JsValue>(1)?;
    let value = cast_operand(&mut cx, value)?;
    let saved = handle.0.save(value);
    expr_node(&mut cx, saved)
}

/// `modelCastComparedColumn(casts, expr)`: a read of the column as a
/// comparison takes it.
fn cast_compared_column(mut cx: FunctionContext) -> JsResult<JsValue> {
    let handle = casts(&mut cx)?;
    let expr = cx.argument::<JsValue>(1)?;
    let expr = Expr::expr(expression(&mut cx, expr)?);
    let compared = handle.0.compared_column(expr);
    expr_node(&mut cx, compared.into())
}

/// `modelCastComparedValue(casts, value)`: a value as a comparison with the
/// column takes it.
fn cast_compared_value(mut cx: FunctionContext) -> JsResult<JsValue> {
    let handle = casts(&mut cx)?;
    let value = cx.argument::<JsValue>(1)?;
    let value = cast_operand(&mut cx, value)?;
    let compared = handle.0.compared_value(value);
    expr_node(&mut cx, compared)
}

/// `modelCastComparesAsEnum(casts)`: whether the column compares as an enum,
/// its type a domain over one.
fn cast_compares_as_enum(mut cx: FunctionContext) -> JsResult<JsBoolean> {
    let handle = casts(&mut cx)?;
    let compares = handle.0.compares_as().is_some();
    Ok(cx.boolean(compares))
}
