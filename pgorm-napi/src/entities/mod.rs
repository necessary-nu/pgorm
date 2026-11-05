//! Rust entities an application registers with the binding, which
//! JavaScript reaches by name. One native module carries both: the binding's
//! whole API, and the application's [`Registry`] of concrete entities and
//! graph shapes, installed together by [`crate::install`] from the
//! application's own `#[neon::main]`, as pgorm-python's application module
//! installs its registry. JavaScript cannot instantiate a Rust generic; it
//! uses the ones the application compiled in.

/// The native half of the handle of type `$handle` at argument `$index`.
macro_rules! held {
    ($cx:expr, $handle:ty, $index:expr) => {
        $cx.argument::<neon::types::JsBox<$handle>>($index)?
            .0
            .clone()
    };
}

/// The string at argument `$index`.
macro_rules! text {
    ($cx:expr, $index:expr) => {
        $cx.argument::<neon::types::JsString>($index)?
            .value(&mut $cx)
    };
}

mod adapter;
mod convert;
mod graph;
mod graph_reads;
mod graphs;
mod info;
mod inserts;
mod keysets;
pub(crate) mod reads;
mod registry;
mod select_reads;
mod selected;
mod sources;
mod versions;

use std::sync::Arc;

use neon::{prelude::*, types::Finalize};
use pgorm::{ActiveValue, pgorm_query::Condition};

use adapter::{Active, Between, Change, Comparison, Members, Read, Select, Write};
pub(crate) use adapter::{EntityBackend, Model, PipelineColumn};
use convert::column_array;
use convert::{ModelHandle, Records, Wrote, entity_failure, names};
pub(crate) use convert::{column_value, record};
pub use graph::{GraphBindings, GraphModel, GraphSlots, RegisteredSlot, Source};
pub(crate) use info::registration_name;
pub use registry::{RegistrationError, Registry};
pub use sources::{SourceBindings, SourceModel, SourceTypes};

use crate::{
    codec::Codec,
    connect::job::{JobHandle, Output},
    errors::{Failure, throw},
    rows,
    statements::{self, Ddl, Node},
    values::read,
};

/// The registry as JavaScript holds it: the module's own, exported as
/// `registry`.
#[derive(Debug)]
pub(crate) struct RegistryHandle(pub(crate) Arc<Registry>);

impl Finalize for RegistryHandle {}

#[derive(Debug)]
pub(crate) struct EntityHandle(pub(crate) Arc<dyn EntityBackend>);

impl Finalize for EntityHandle {}

#[derive(Debug)]
pub(crate) struct SelectHandle(Select);

impl Finalize for SelectHandle {}

#[derive(Debug)]
pub(crate) struct ActiveHandle(pub(crate) Active);

impl Finalize for ActiveHandle {}

pub(crate) fn install(cx: &mut ModuleContext, registry: Registry) -> NeonResult<()> {
    for (name, export) in EXPORTS {
        cx.export_function(name, *export)?;
    }
    versions::export(cx)?;
    inserts::export(cx)?;
    graphs::export(cx)?;
    selected::export(cx)?;
    reads::export(cx)?;
    select_reads::export(cx)?;
    graph_reads::export(cx)?;
    let registry = cx.boxed(RegistryHandle(Arc::new(registry)));
    cx.export_value("registry", registry)?;
    Ok(())
}

type Export = fn(FunctionContext) -> JsResult<JsValue>;

const EXPORTS: &[(&str, Export)] = &[
    ("entityNames", entity_names),
    ("entityGet", entity_get),
    ("entityDescribe", entity_describe),
    ("entitySchema", entity_schema),
    ("entityColumns", entity_columns),
    ("entityCol", entity_col),
    ("entityCompare", entity_compare),
    ("entityBetween", entity_between),
    ("entityQuantified", entity_quantified),
    ("entityIn", entity_in),
    ("entityFind", entity_find),
    ("entityActive", entity_active),
    ("entitySelectChange", select_change),
    ("entitySelectInspect", select_inspect),
    ("entitySelectJob", select_job),
    ("entityModelSet", model_set),
    ("entityModelActive", model_active),
    ("entityModelTagged", model_tagged),
    ("entityActiveName", active_name),
    ("entityActiveGet", active_get),
    ("entityActiveChange", active_change),
    ("entityActiveChanged", active_changed),
    ("entityActiveJob", active_job),
];

/// A list of names as a JavaScript array.
fn strings<'cx>(
    cx: &mut Cx<'cx>,
    items: impl ExactSizeIterator<Item = String>,
) -> JsResult<'cx, JsValue> {
    let array = JsArray::new(cx, items.len());
    for (index, item) in items.enumerate() {
        let item = cx.string(item);
        array.set(cx, u32::try_from(index).unwrap_or(u32::MAX), item)?;
    }
    Ok(array.upcast())
}

/// `[sql, values]`, each value a tagged `Value`, as `statementInspect`
/// gives them.
pub(crate) fn inspected<'cx>(
    cx: &mut Cx<'cx>,
    (sql, values): (String, pgorm::pgorm_query::Values),
) -> JsResult<'cx, JsValue> {
    let codec = Codec::get(cx)?;
    let list = JsArray::new(cx, values.0.len());
    for (at, value) in values.0.into_iter().enumerate() {
        let value = crate::values::Tagged::value(value);
        let value = rows::value(cx, codec, value, true)?;
        list.set(cx, u32::try_from(at).unwrap_or(u32::MAX), value)?;
    }
    let pair = JsArray::new(cx, 2);
    let sql = cx.string(sql);
    pair.set(cx, 0, sql)?;
    pair.set(cx, 1, list)?;
    Ok(pair.upcast())
}

/// The condition `value` is, an expression or a `Condition`.
pub(crate) fn condition<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> NeonResult<Condition> {
    match statements::node(cx, value) {
        Some(Node::Expr(expr)) => Ok(Condition::all().add(expr)),
        Some(Node::Condition(condition)) => Ok(condition),
        _ => cx.throw_type_error("a condition is an expression or a Condition"),
    }
}

/// A registered entity as a pipeline reads it: its own `IntoSource`, so its
/// table and schema are the ones its Rust declaration names.
#[derive(Debug, Clone)]
pub(crate) struct EntitySource(Arc<dyn EntityBackend>);

impl EntitySource {
    /// The entity's table as a pipeline source.
    // [spec:pgorm:req:bindings.pipeline-sources+3]
    pub(crate) fn source(&self) -> pgorm::pipeline::Source {
        self.0.source()
    }
}

/// The registered entity `value` is, as a pipeline source, or `None` when
/// `value` is no entity.
pub(crate) fn entity_source<'cx>(
    cx: &mut Cx<'cx>,
    value: Handle<'cx, JsValue>,
) -> Option<EntitySource> {
    let entity = value.downcast::<JsBox<EntityHandle>, _>(cx).ok()?;
    Some(EntitySource(entity.0.clone()))
}

/// `entityNames(registry)`: every registered entity's name.
fn entity_names(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = held!(cx, RegistryHandle, 0);
    strings(
        &mut cx,
        registry
            .entities
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter(),
    )
}

/// `entityGet(registry, name)`: the entity registered as `name`.
// [spec:pgorm:req:bindings.entities+5]
fn entity_get(mut cx: FunctionContext) -> JsResult<JsValue> {
    let registry = held!(cx, RegistryHandle, 0);
    let name = text!(cx, 1);
    match registry.entities.get(&name) {
        Some(entity) => Ok(cx.boxed(EntityHandle(entity.clone())).upcast()),
        None => throw(
            &mut cx,
            Failure::Construction(format!(
                "no entity is registered as {name:?} in this module"
            )),
        ),
    }
}

/// `entityDescribe(entity)`: the registration as JSON text.
fn entity_describe(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = held!(cx, EntityHandle, 0);
    let described = entity
        .info()
        .describe()
        .or_else(|failure| throw(&mut cx, failure))?;
    Ok(cx.string(described.to_string()).upcast())
}

/// `entitySchema(entity)`: `[table, enums, indexes, comments]`, each
/// statement one pgorm's own entity schema path wrote. A table that path
/// refuses is a `ConstructionError` carrying its message, and gives no
/// schema at all.
// [spec:pgorm:req:bindings.schema-from-entity]
fn entity_schema(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = held!(cx, EntityHandle, 0);
    let schema = match entity.schema() {
        Ok(schema) => schema,
        Err(error) => return throw(&mut cx, Failure::Construction(error.to_string())),
    };
    let table = cx.boxed(Node::from(Ddl::CreateTable(schema.table)));
    let enums = ddl_list(&mut cx, schema.enums.into_iter().map(Ddl::CreateType))?;
    let indexes = ddl_list(&mut cx, schema.indexes.into_iter().map(Ddl::CreateIndex))?;
    let comments = ddl_list(&mut cx, schema.comments.into_iter().map(Ddl::Comment))?;
    let parts = JsArray::new(&mut cx, 4);
    parts.set(&mut cx, 0, table)?;
    parts.set(&mut cx, 1, enums)?;
    parts.set(&mut cx, 2, indexes)?;
    parts.set(&mut cx, 3, comments)?;
    Ok(parts.upcast())
}

/// DDL statements as an array of the nodes JavaScript builders own.
fn ddl_list<'cx>(
    cx: &mut Cx<'cx>,
    statements: impl ExactSizeIterator<Item = Ddl>,
) -> JsResult<'cx, JsArray> {
    let array = JsArray::new(cx, statements.len());
    for (index, statement) in statements.enumerate() {
        let statement = cx.boxed(Node::from(statement));
        array.set(cx, u32::try_from(index).unwrap_or(u32::MAX), statement)?;
    }
    Ok(array)
}

fn entity_columns(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = held!(cx, EntityHandle, 0);
    Ok(names(&mut cx, entity.info())?.upcast())
}

/// `entityCol(entity, column)`: the column as the entity's own
/// `ColumnTrait` names it, an expression any builder takes.
fn entity_col(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = held!(cx, EntityHandle, 0);
    let column = text!(cx, 1);
    match entity.column(&column) {
        Ok(expr) => Ok(cx.boxed(Node::Expr(expr)).upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// The comparison named at argument `index`, by its JavaScript method name.
fn comparison_at(cx: &mut FunctionContext, index: usize) -> NeonResult<Comparison> {
    Ok(match cx.argument::<JsString>(index)?.value(cx).as_str() {
        "eq" => Comparison::Eq,
        "ne" => Comparison::Ne,
        "gt" => Comparison::Gt,
        "gte" => Comparison::Ge,
        "lt" => Comparison::Lt,
        "lte" => Comparison::Le,
        "isDistinctFrom" => Comparison::IsDistinctFrom,
        "isNotDistinctFrom" => Comparison::IsNotDistinctFrom,
        "contains" => Comparison::Contains,
        "containedBy" => Comparison::ContainedBy,
        "overlaps" => Comparison::Overlaps,
        "strictlyLeftOf" => Comparison::StrictlyLeftOf,
        "strictlyRightOf" => Comparison::StrictlyRightOf,
        "notExtendRightOf" => Comparison::NotExtendRightOf,
        "notExtendLeftOf" => Comparison::NotExtendLeftOf,
        "adjacentTo" => Comparison::AdjacentTo,
        _ => return cx.throw_type_error("unknown comparison"),
    })
}

/// `entityCompare(entity, column, operator, value)`: the column compared
/// through its `ColumnTrait` method — a comparison, `IS [NOT] DISTINCT
/// FROM` or a range operator — the value converted to the column's declared
/// kind and written through its `save_as`.
// [spec:pgorm:req:bindings.entity-reads+4]
// [spec:pgorm:req:bindings.column-operators+3]
fn entity_compare(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = held!(cx, EntityHandle, 0);
    let column = text!(cx, 1);
    let comparison = comparison_at(&mut cx, 2)?;
    let value = compared_value(&mut cx, &*entity, &column, 3)?;
    match entity.compare(&column, comparison, value) {
        Ok(expr) => Ok(cx.boxed(Node::Expr(expr)).upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// `entityQuantified(entity, column, operator, "any" | "all", values)`: the
/// column compared with `ANY` or `ALL` of `values` through its
/// `ColumnTrait::compare_any` / `compare_all`, the values read as one array
/// of the column's declared kind; `null` for a column of no scalar kind,
/// whose quantifier is compared as written.
// [spec:pgorm:req:bindings.entity-reads+4]
fn entity_quantified(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = held!(cx, EntityHandle, 0);
    let column = text!(cx, 1);
    let comparison = comparison_at(&mut cx, 2)?;
    let all = text!(cx, 3) == "all";
    let info = match entity.info().column(&column) {
        Ok(info) => info.clone(),
        Err(failure) => return throw(&mut cx, failure),
    };
    let values = cx.argument::<JsValue>(4)?;
    let Some(array) = column_array(&mut cx, &info, values)? else {
        return Ok(cx.null().upcast());
    };
    match entity.quantified(&column, comparison, all, array) {
        Ok(expr) => Ok(cx.boxed(Node::Expr(expr)).upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// `entityBetween(entity, column, negated, symmetric, lower, upper)`: the
/// column's `[NOT] BETWEEN [SYMMETRIC]` through its `ColumnTrait` method,
/// each bound converted as a compared value is.
// [spec:pgorm:req:bindings.column-operators+3]
fn entity_between(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = held!(cx, EntityHandle, 0);
    let column = text!(cx, 1);
    let negated = cx.argument::<JsBoolean>(2)?.value(&mut cx);
    let symmetric = cx.argument::<JsBoolean>(3)?.value(&mut cx);
    let lower = compared_value(&mut cx, &*entity, &column, 4)?;
    let upper = compared_value(&mut cx, &*entity, &column, 5)?;
    let between = Between { negated, symmetric };
    match entity.between(&column, between, lower, upper) {
        Ok(expr) => Ok(cx.boxed(Node::Expr(expr)).upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// The value at argument `index`, compared with `column`: converted to the
/// column's declared kind, `null` refused, as a comparison never holds for
/// it.
fn compared_value(
    cx: &mut FunctionContext,
    entity: &dyn EntityBackend,
    column: &str,
    index: usize,
) -> NeonResult<pgorm::pgorm_query::Value> {
    let info = match entity.info().column(column) {
        Ok(info) => info.clone(),
        Err(failure) => return throw(cx, failure),
    };
    let data = cx.argument::<JsValue>(index)?;
    if data.is_a::<JsNull, _>(cx) {
        return read::refuse(
            cx,
            format!("{column} compared with null is never true: test it with isNull()"),
        );
    }
    column_value(cx, &info, data)
}

/// `entityIn(entity, column, negated, values | select)`: the column's
/// membership through its `ColumnTrait` method — `is_in` / `is_not_in` over
/// values read as the column's declared kind, `in_subquery` /
/// `not_in_subquery` over a subquery's rows — so a column whose domain is over
/// an enum is a member as the enum, and so is what the subquery projects.
// [spec:pgorm:req:bindings.entity-reads+4]
fn entity_in(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = held!(cx, EntityHandle, 0);
    let column = text!(cx, 1);
    let negated = cx.argument::<JsBoolean>(2)?.value(&mut cx);
    let source = cx.argument::<JsValue>(3)?;
    let info = match entity.info().column(&column) {
        Ok(info) => info.clone(),
        Err(failure) => return throw(&mut cx, failure),
    };
    let members = match source.downcast::<JsArray, _>(&mut cx) {
        Ok(list) => {
            let items = list.to_vec(&mut cx)?;
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(column_value(&mut cx, &info, item)?);
            }
            Members::Values(values)
        }
        Err(_) => Members::Subquery(Box::new(statements::embedded_select(&mut cx, source)?)),
    };
    match entity.membership(&column, negated, members) {
        Ok(expr) => Ok(cx.boxed(Node::Expr(expr)).upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// The registered entity at argument `entity` and its column named at
/// argument `column`, as a pipeline reads it.
// [spec:pgorm:req:bindings.pipeline-sources+3]
pub(crate) fn pipeline_column(
    cx: &mut FunctionContext,
    entity: usize,
    column: usize,
) -> NeonResult<Arc<dyn PipelineColumn>> {
    let backend = cx.argument::<JsBox<EntityHandle>>(entity)?.0.clone();
    let name = cx.argument::<JsString>(column)?.value(cx);
    match backend.pipeline_column(&name) {
        Ok(column) => Ok(column),
        Err(failure) => throw(cx, failure),
    }
}

fn entity_find(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = held!(cx, EntityHandle, 0);
    Ok(cx.boxed(SelectHandle(entity.select())).upcast())
}

/// `entityActive(entity)`: an ActiveModel from `ActiveModelBehavior::new`,
/// its defaults included.
fn entity_active(mut cx: FunctionContext) -> JsResult<JsValue> {
    let entity = held!(cx, EntityHandle, 0);
    Ok(cx.boxed(ActiveHandle(entity.active())).upcast())
}

/// A count a query takes: a non-negative integer, or `null` to remove it.
fn count_at(cx: &mut FunctionContext, index: usize) -> NeonResult<Option<u64>> {
    let value = cx.argument::<JsValue>(index)?;
    if value.is_a::<JsNull, _>(cx) {
        return Ok(None);
    }
    let number = value.downcast_or_throw::<JsNumber, _>(cx)?.value(cx);
    if number.fract() != 0.0 || !(0.0..=9_007_199_254_740_991.0).contains(&number) {
        return read::refuse(cx, "a limit or offset is a non-negative integer, or null");
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(Some(number as u64))
}

/// `entitySelectChange(select, step, argument)`: the query with one more
/// step — `where`, `orderBy`, `limit` or `offset` — as `Select<E>` takes it.
// [spec:pgorm:req:bindings.entity-reads+4]
fn select_change(mut cx: FunctionContext) -> JsResult<JsValue> {
    let select = held!(cx, SelectHandle, 0);
    let change = step(&mut cx)?;
    Ok(cx.boxed(SelectHandle(select.change(change))).upcast())
}

/// The step at arguments 1 and 2 — `where` a condition, `orderBy` an
/// ordering, `limit` or `offset` a count — as the query method of its
/// meaning takes it.
pub(crate) fn step(cx: &mut FunctionContext) -> NeonResult<Change> {
    let step = cx.argument::<JsString>(1)?.value(cx);
    let argument = cx.argument::<JsValue>(2)?;
    Ok(match step.as_str() {
        "where" => Change::Filter(condition(cx, argument)?),
        "orderBy" => match statements::node(cx, argument) {
            Some(Node::Ordering(ordering)) => {
                Change::Order(ordering.expr, ordering.order, ordering.nulls)
            }
            _ => return read::refuse(cx, "an ordering is expr.asc() or expr.desc()"),
        },
        "limit" => Change::Limit(count_at(cx, 2)?),
        "offset" => Change::Offset(count_at(cx, 2)?),
        _ => return cx.throw_type_error("unknown query step"),
    })
}

fn read_terminal(cx: &mut FunctionContext, index: usize) -> NeonResult<Read> {
    match cx.argument::<JsString>(index)?.value(cx).as_str() {
        "all" => Ok(Read::All),
        "one" => Ok(Read::One),
        "optional" => Ok(Read::OneOpt),
        _ => cx.throw_type_error("a read terminal is all, one or optional"),
    }
}

/// `entitySelectInspect(select, terminal)`: the SQL and values the terminal
/// sends, `one` and `optional` with the `LIMIT 1` Rust's terminals add.
fn select_inspect(mut cx: FunctionContext) -> JsResult<JsValue> {
    let select = held!(cx, SelectHandle, 0);
    let read = read_terminal(&mut cx, 1)?;
    let compiled = select.compile(read);
    inspected(&mut cx, compiled)
}

/// `entitySelectJob(select, terminal)`: the read, as a job any executor runs.
fn select_job(mut cx: FunctionContext) -> JsResult<JsValue> {
    let select = held!(cx, SelectHandle, 0);
    let read = read_terminal(&mut cx, 1)?;
    let job: crate::connect::job::Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            let models = select
                .run(db, read)
                .await
                .map_err(|error| entity_failure(&error, secrets))?;
            Ok(Box::new(Records(select.info().clone(), models)) as Output)
        })
    });
    Ok(cx.boxed(JobHandle::new(job)).upcast())
}

/// `entityModelSet(model, column, value)`: `[values, model]` of a clone
/// with `ModelTrait::set` applied. Nothing is written.
// [spec:pgorm:req:bindings.entity-writes+1]
fn model_set(mut cx: FunctionContext) -> JsResult<JsValue> {
    let model = held!(cx, ModelHandle, 0);
    let column = text!(cx, 1);
    let info = match model.info().column(&column) {
        Ok(info) => info.clone(),
        Err(failure) => return throw(&mut cx, failure),
    };
    let data = cx.argument::<JsValue>(2)?;
    let value = column_value(&mut cx, &info, data)?;
    match model.set(&column, value) {
        Ok(model) => record(&mut cx, model),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// `entityModelActive(model)`: the model's ActiveModel, by its real
/// `IntoActiveModel`, every column `Unchanged`.
fn model_active(mut cx: FunctionContext) -> JsResult<JsValue> {
    let model = held!(cx, ModelHandle, 0);
    Ok(cx.boxed(ActiveHandle(model.active())).upcast())
}

/// `entityModelTagged(model, column)`: the column's value as a `Value`.
fn model_tagged(mut cx: FunctionContext) -> JsResult<JsValue> {
    let model = held!(cx, ModelHandle, 0);
    let column = text!(cx, 1);
    let info = match model.info().column(&column) {
        Ok(info) => info.clone(),
        Err(failure) => return throw(&mut cx, failure),
    };
    let value = match model.get(&column) {
        Ok(value) => value,
        Err(failure) => return throw(&mut cx, failure),
    };
    let codec = Codec::get(&mut cx)?;
    let value = convert::tagged(&info, value).or_else(|failure| throw(&mut cx, failure))?;
    rows::value(&mut cx, codec, value, true)
}

/// `entityActiveName(active)`: the registration that made the ActiveModel.
fn active_name(mut cx: FunctionContext) -> JsResult<JsValue> {
    let active = held!(cx, ActiveHandle, 0);
    Ok(cx.string(&active.info().name).upcast())
}

/// `entityActiveGet(active, column)`: `[state, value]`, the state one of
/// `notSet`, `set` and `unchanged`, the value absent for `notSet`.
fn active_get(mut cx: FunctionContext) -> JsResult<JsValue> {
    let active = held!(cx, ActiveHandle, 0);
    let column = text!(cx, 1);
    let info = match active.info().column(&column) {
        Ok(info) => info.clone(),
        Err(failure) => return throw(&mut cx, failure),
    };
    let (state, value) = match active.get(&column) {
        Ok(ActiveValue::NotSet) => ("notSet", None),
        Ok(ActiveValue::Set(value)) => ("set", Some(value)),
        Ok(ActiveValue::Unchanged(value)) => ("unchanged", Some(value)),
        Err(failure) => return throw(&mut cx, failure),
    };
    let codec = Codec::get(&mut cx)?;
    let pair = JsArray::new(&mut cx, 2);
    let state = cx.string(state);
    pair.set(&mut cx, 0, state)?;
    if let Some(value) = value {
        let value = convert::tagged(&info, value).or_else(|failure| throw(&mut cx, failure))?;
        let value = rows::value(&mut cx, codec, value, false)?;
        pair.set(&mut cx, 1, value)?;
    }
    Ok(pair.upcast())
}

/// `entityActiveChanged(active)`: `ActiveModelTrait::is_changed`.
// [spec:pgorm:req:bindings.entity-active-changes]
fn active_changed(mut cx: FunctionContext) -> JsResult<JsValue> {
    let active = held!(cx, ActiveHandle, 0);
    Ok(cx.boolean(active.is_changed()).upcast())
}

/// `entityActiveChange(active, step, column, value?)`: a new ActiveModel
/// with one column `set` to a value, `setIfNotEquals` it, `notSet` or
/// `reset`.
// [spec:pgorm:req:bindings.entity-writes+1]
// [spec:pgorm:req:bindings.entity-active-changes]
fn active_change(mut cx: FunctionContext) -> JsResult<JsValue> {
    let active = held!(cx, ActiveHandle, 0);
    let step = text!(cx, 1);
    let column = text!(cx, 2);
    let info = match active.info().column(&column) {
        Ok(info) => info.clone(),
        Err(failure) => return throw(&mut cx, failure),
    };
    let changed = match step.as_str() {
        "set" => {
            let data = cx.argument::<JsValue>(3)?;
            let value = column_value(&mut cx, &info, data)?;
            active.set(&column, value)
        }
        "setIfNotEquals" => {
            let data = cx.argument::<JsValue>(3)?;
            let value = column_value(&mut cx, &info, data)?;
            active.set_if_not_equals(&column, value)
        }
        "notSet" => active.not_set(&column),
        "reset" => active.reset(&column),
        _ => return cx.throw_type_error("unknown ActiveModel step"),
    };
    match changed {
        Ok(active) => Ok(cx.boxed(ActiveHandle(active)).upcast()),
        Err(failure) => throw(&mut cx, failure),
    }
}

/// `entityActiveJob(active, write)`: `insert`, `update` or `delete` through
/// `ActiveModelTrait`, its `ActiveModelBehavior` hooks around it.
// [spec:pgorm:req:bindings.entity-writes+1]
fn active_job(mut cx: FunctionContext) -> JsResult<JsValue> {
    let active = held!(cx, ActiveHandle, 0);
    let write = match text!(cx, 1).as_str() {
        "insert" => Write::Insert,
        "update" => Write::Update,
        "delete" => Write::Delete,
        _ => return cx.throw_type_error("a write is insert, update or delete"),
    };
    let job: crate::connect::job::Job = Box::new(move |db, secrets| {
        Box::pin(async move {
            let written = active
                .run(db, write)
                .await
                .map_err(|error| entity_failure(&error, secrets))?;
            Ok(Box::new(Wrote(written)) as Output)
        })
    });
    Ok(cx.boxed(JobHandle::new(job)).upcast())
}
