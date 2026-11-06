//! What SQL text does beside its terminals: a script run unprepared, text
//! held to the grammar, PRQL compiled, a statement described without running
//! it, and a `SELECT` wrapped for its pages and its count. Each is pgorm's
//! own: `batch_execute`, `pgorm::sql_check`, `VerifyStatement::describe`,
//! and `raw_select` with `page_statement` and `count_statement`.

use std::num::NonZeroU64;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use neon::prelude::*;
use pgorm::{
    CopyColumn, DescribedColumn, Error,
    pgorm_query::{Value, Values},
    sql_check,
};

use crate::{
    connect::job::{Database, Job, JobHandle, Output, Settled},
    decode,
    errors::{Failure, failure},
    models,
    values::read,
};

type Export = fn(FunctionContext) -> JsResult<JsValue>;

const EXPORTS: &[(&str, Export)] = &[
    ("sqlCheck", sql_check),
    ("prqlSql", prql_sql),
    ("rawPage", raw_page),
    ("rawCount", raw_count),
    ("batchExecuteJob", batch_execute_job),
    ("describeJob", describe_job),
];

pub(crate) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    for (name, export) in EXPORTS {
        cx.export_function(name, *export)?;
    }
    Ok(())
}

/// `sqlCheck(text)`: the text, held to PostgreSQL 18's grammar, a script
/// statement by statement, as Rust's `sql!` holds a literal.
// [spec:pgorm:req:bindings.sql-check]
fn sql_check(mut cx: FunctionContext) -> JsResult<JsValue> {
    let argument = cx.argument::<JsValue>(0)?;
    let text = read::text(&mut cx, argument, "SQL text")?;
    match sql_check::check_sql(&text) {
        Ok(()) => Ok(argument),
        Err(rejection) => read::refuse(&mut cx, rejection.to_string()),
    }
}

/// `prqlSql(text, values)`: the SQL `text`, PRQL, compiles to, held to the
/// grammar, its placeholders held to the number of values given.
// [spec:pgorm:req:bindings.prql-text]
fn prql_sql(mut cx: FunctionContext) -> JsResult<JsValue> {
    let argument = cx.argument::<JsValue>(0)?;
    let text = read::text(&mut cx, argument, "PRQL text")?;
    let given = cx.argument::<JsNumber>(1)?.value(&mut cx);
    let compiled = match sql_check::compile_prql(&text) {
        Ok(compiled) => compiled,
        Err(rejection) => return read::refuse(&mut cx, rejection.to_string()),
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    if let Err(rejection) = compiled.check_arity(given as usize) {
        return read::refuse(&mut cx, rejection.to_string());
    }
    Ok(cx.string(compiled.sql()).upcast())
}

/// Stand-ins for a raw statement's `count` parameters, each the position it
/// came from, so the statement wrapped around it says which of the caller's
/// values goes where without the values crossing: wrapping renumbers the
/// statement's markers in first-reference order, one parameter per
/// reference.
fn positions(count: usize) -> Vec<Value> {
    (0..count)
        .map(|position| Value::BigInt(Some(i64::try_from(position).unwrap_or(i64::MAX))))
        .collect()
}

/// `[sql, order, added]` for a statement built over the stand-ins: `order[k]`
/// is the position of the caller's value the `k`th parameter takes, or, past
/// the caller's values, of one of `added`, the values the wrapper itself
/// binds, in order.
fn ordered<'cx>(
    cx: &mut FunctionContext<'cx>,
    (sql, values): (String, Values),
    count: usize,
) -> JsResult<'cx, JsValue> {
    let order = JsArray::new(cx, values.0.len());
    let added = JsArray::new(cx, 0);
    let mut extra = 0_u32;
    for (index, value) in values.0.into_iter().enumerate() {
        let position = match value {
            Value::BigInt(Some(position)) => usize::try_from(position).unwrap_or(usize::MAX),
            Value::BigUnsigned(Some(bound)) => {
                #[allow(clippy::cast_precision_loss)]
                let bound = cx.number(bound as f64);
                added.set(cx, extra, bound)?;
                extra += 1;
                count + extra as usize - 1
            }
            other => return read::refuse(cx, format!("pgorm bug: a raw page binds {other:?}")),
        };
        #[allow(clippy::cast_precision_loss)]
        let position = cx.number(position as f64);
        order.set(cx, u32::try_from(index).unwrap_or(u32::MAX), position)?;
    }
    let triple = JsArray::new(cx, 3);
    let sql = cx.string(sql);
    triple.set(cx, 0, sql)?;
    triple.set(cx, 1, order)?;
    triple.set(cx, 2, added)?;
    Ok(triple.upcast())
}

fn wrapped<'cx>(
    cx: &mut FunctionContext<'cx>,
    sql: &str,
    count: usize,
) -> NeonResult<pgorm::pgorm_query::SelectStatement> {
    match pgorm::raw_select(sql, positions(count)) {
        Ok(select) => Ok(select),
        Err(Error::Query(refusal)) => read::refuse(cx, refusal.to_string()),
        Err(other) => read::refuse(cx, other.to_string()),
    }
}

/// `rawPage(sql, count, pageSize, page)`: page `page` of `sql`, a statement
/// of `count` parameters, read as a subquery as Rust's
/// `SelectorRaw::paginate` reads it; refused unless it is one row-returning
/// `SELECT`. The page's limit and offset are the two values the wrapper adds.
// [spec:pgorm:req:bindings.raw-pages]
fn raw_page(mut cx: FunctionContext) -> JsResult<JsValue> {
    let sql = cx.argument::<JsString>(0)?.value(&mut cx);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = cx.argument::<JsNumber>(1)?.value(&mut cx) as usize;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let size = cx.argument::<JsNumber>(2)?.value(&mut cx) as u64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let page = cx.argument::<JsNumber>(3)?.value(&mut cx) as u64;
    let select = wrapped(&mut cx, &sql, count)?;
    let Some(size) = NonZeroU64::new(size) else {
        return read::refuse(&mut cx, "a page size is at least 1");
    };
    match pgorm::page_statement(&select, size, page) {
        Ok(statement) => ordered(&mut cx, statement.build(), count),
        Err(error) => read::refuse(&mut cx, error.to_string()),
    }
}

/// `rawCount(sql, count)`: the statement counting `sql`'s rows, as Rust's
/// `SelectorRaw::count` counts them.
// [spec:pgorm:req:bindings.raw-pages]
fn raw_count(mut cx: FunctionContext) -> JsResult<JsValue> {
    let sql = cx.argument::<JsString>(0)?.value(&mut cx);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = cx.argument::<JsNumber>(1)?.value(&mut cx) as usize;
    let select = wrapped(&mut cx, &sql, count)?;
    match pgorm::count_statement(&select) {
        Ok(statement) => ordered(&mut cx, statement.build(), count),
        Err(error) => read::refuse(&mut cx, error.to_string()),
    }
}

/// A job's empty answer.
struct Finished;

impl Settled for Finished {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        Ok(cx.undefined().upcast())
    }
}

/// `batchExecuteJob(script)`: the script, `;`-separated statements with no
/// parameters, run through the simple-query protocol as a job.
// [spec:pgorm:req:bindings.batch-execute]
fn batch_execute_job(mut cx: FunctionContext) -> JsResult<JsValue> {
    let argument = cx.argument::<JsValue>(0)?;
    if !argument.is_a::<JsString, _>(&mut cx) {
        return cx.throw_type_error("a script is a string");
    }
    let script = read::sql(&mut cx, argument)?;
    let job: Job = Box::new(
        move |db: Database<'_>, secrets| -> BoxFuture<'_, Result<Output, Failure>> {
            async move {
                pgorm::ConnectionTrait::batch_execute(&db, &script)
                    .await
                    .map_err(|error| failure(&error, secrets))?;
                Ok(Box::new(Finished) as Output)
            }
            .boxed()
        },
    );
    Ok(cx.boxed(JobHandle::new(job)).upcast())
}

/// A statement's description: each column's name, its type's name, and the
/// kind its rows decode as, `null` for a type the binding reads no value of.
struct Described(Vec<(String, String, Option<String>)>);

impl Settled for Described {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        let columns = JsArray::new(cx, self.0.len());
        for (index, (name, type_name, kind)) in self.0.into_iter().enumerate() {
            let column = JsArray::new(cx, 3);
            let name = cx.string(name);
            column.set(cx, 0, name)?;
            let type_name = cx.string(type_name);
            column.set(cx, 1, type_name)?;
            let kind: Handle<JsValue> = match kind {
                Some(kind) => cx.string(kind).upcast(),
                None => cx.null().upcast(),
            };
            column.set(cx, 2, kind)?;
            columns.set(cx, u32::try_from(index).unwrap_or(u32::MAX), column)?;
        }
        Ok(columns.upcast())
    }
}

/// The kind a column of `column`'s type decodes as, read off a `NULL` of it
/// through the codec a row's value goes through.
fn kind(column: &DescribedColumn) -> Option<String> {
    let copy = CopyColumn::new(column.name(), column.type_().clone());
    let null = decode::field(&copy, None).ok()?;
    models::kind_key(&null.tag).ok()
}

/// `describeJob(sql)`: the result columns the server describes for `sql`,
/// without running it, as a job.
// [spec:pgorm:req:bindings.verify]
fn describe_job(mut cx: FunctionContext) -> JsResult<JsValue> {
    let argument = cx.argument::<JsValue>(0)?;
    let sql = read::sql(&mut cx, argument)?;
    let job: Job = Box::new(
        move |db: Database<'_>, secrets| -> BoxFuture<'_, Result<Output, Failure>> {
            async move {
                let columns = pgorm_bindings::describe(db, &sql)
                    .await
                    .map_err(|error| failure(&error, secrets))?;
                let described = columns
                    .iter()
                    .map(|column| {
                        (
                            column.name().to_owned(),
                            column.type_().name().to_owned(),
                            kind(column),
                        )
                    })
                    .collect();
                Ok(Box::new(Described(described)) as Output)
            }
            .boxed()
        },
    );
    Ok(cx.boxed(JobHandle::new(job)).upcast())
}
