//! pgorm-migration's runner, run from JavaScript.
//!
//! A JavaScript program's migrations are pgorm-bindings' list of them, whose
//! written migrations' `up` sends the object's `up` to the JavaScript thread
//! with a transaction handle on the batch's transaction, and awaits it.

use std::sync::{Arc, Mutex, PoisonError};

use futures_util::FutureExt;
use neon::{event::Channel, prelude::*, types::Finalize};
use pgorm_bindings::migrations::{Entry, Failed, Host, Listed, statuses};
use pgorm_migration::{MigrationName, MigrationTrait, MigratorTrait, SqlMigration};
use tokio::sync::oneshot;

use super::{
    PoolHandle, PoolState,
    job::{JobHandle, Output, Settled},
    transaction::TransactionHandle,
};
use crate::{
    errors::{Failure, failure, throw},
    settle,
};

pub(super) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("sqlMigration", sql_migration)?;
    cx.export_function("sqlMigrationRead", sql_migration_read)?;
    cx.export_function("sqlMigrationReadDir", sql_migration_read_dir)?;
    cx.export_function("sqlMigrationParts", sql_migration_parts)?;
    cx.export_function("migrationsNew", migrations_new)?;
    cx.export_function("migrationsUp", migrations_up)?;
    cx.export_function("migrationsJob", migrations_job)?;
    Ok(())
}

/// A migration whose whole body is one SQL script, pgorm-migration's
/// `SqlMigration`, as `lib/migrations.js` holds it.
pub(crate) struct SqlMigrationHandle(SqlMigration);

impl Finalize for SqlMigrationHandle {}

/// `sqlMigration(name, sql)`.
// [spec:pgorm:req:bindings.migrations]
fn sql_migration(mut cx: FunctionContext) -> JsResult<JsBox<SqlMigrationHandle>> {
    let name = cx.argument::<JsString>(0)?.value(&mut cx);
    let sql = cx.argument::<JsString>(1)?.value(&mut cx);
    Ok(cx.boxed(SqlMigrationHandle(SqlMigration::new(name, sql))))
}

/// A failure to read as JavaScript throws it: a path naming no file, text
/// that is not UTF-8 and text the grammar refuses a `ConstructionError`, as a
/// migration file's name that is no file stem is, and anything else an
/// `Error` with its message.
fn read_failure<'cx, T>(cx: &mut FunctionContext<'cx>, error: std::io::Error) -> NeonResult<T> {
    match error.kind() {
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData => {
            throw(cx, Failure::Construction(error.to_string()))
        }
        _ => cx.throw_error(error.to_string()),
    }
}

/// `sqlMigrationRead(path)`: the migration the `.sql` file at `path` holds.
// [spec:pgorm:req:bindings.migrations]
fn sql_migration_read(mut cx: FunctionContext) -> JsResult<JsBox<SqlMigrationHandle>> {
    let path = cx.argument::<JsString>(0)?.value(&mut cx);
    match SqlMigration::read(&path) {
        Ok(migration) => Ok(cx.boxed(SqlMigrationHandle(migration))),
        Err(error) => read_failure(&mut cx, error),
    }
}

/// `sqlMigrationReadDir(directory)`: every `.sql` file directly inside it,
/// in file-name order.
// [spec:pgorm:req:bindings.migrations]
fn sql_migration_read_dir(mut cx: FunctionContext) -> JsResult<JsArray> {
    let directory = cx.argument::<JsString>(0)?.value(&mut cx);
    let migrations = match SqlMigration::read_dir(&directory) {
        Ok(migrations) => migrations,
        Err(error) => return read_failure(&mut cx, error),
    };
    let array = JsArray::new(&mut cx, migrations.len());
    for (at, migration) in (0_u32..).zip(migrations) {
        let handle = cx.boxed(SqlMigrationHandle(migration));
        array.set(&mut cx, at, handle)?;
    }
    Ok(array)
}

/// `sqlMigrationParts(migration)`: `[name, sql, checksum, commitsAfter]`.
fn sql_migration_parts(mut cx: FunctionContext) -> JsResult<JsArray> {
    let handle = cx.argument::<JsBox<SqlMigrationHandle>>(0)?;
    let migration = &handle.0;
    let array = JsArray::new(&mut cx, 4);
    let name = cx.string(migration.name());
    array.set(&mut cx, 0, name)?;
    let sql = cx.string(migration.sql());
    array.set(&mut cx, 1, sql)?;
    let checksum = cx.string(migration.checksum().unwrap_or_default());
    array.set(&mut cx, 2, checksum)?;
    let commits_after = cx.boolean(migration.commits_after());
    array.set(&mut cx, 3, commits_after)?;
    Ok(array)
}

/// A migration written in JavaScript: the object whose `up` is called.
type Written = pgorm_bindings::migrations::Written<Root<JsObject>>;

/// What one `up` call's JavaScript migrations run with: the pool their
/// transaction handles report through, the channel to the JavaScript thread
/// that called `up`, `dispatch` in `lib/migrations.js`, which calls a
/// migration's `up` and reports how it settled.
struct Call {
    pool: Arc<PoolState>,
    channel: Channel,
    dispatch: Root<JsFunction>,
}

/// How a JavaScript migration's `up` settled: resolved, or rejected with the
/// `error` of the object held, which `dispatch` wraps it in because a value
/// that is no object cannot be rooted.
type Settlement = Result<(), Root<JsObject>>;

impl Host for Call {
    type Object = Root<JsObject>;
    type Owner = ();
    type Raised = Root<JsObject>;

    fn pool(&self) -> &Arc<PoolState> {
        &self.pool
    }

    async fn up(
        self: &Arc<Self>,
        written: &Arc<Written>,
        state: Arc<pgorm_bindings::transaction::State<()>>,
    ) -> Result<(), Failed<Root<JsObject>>> {
        let handle = TransactionHandle(state);
        let (settled, settlement) = oneshot::channel::<Settlement>();
        let call = self.clone();
        let written = written.clone();
        let _ = self.channel.try_send(move |mut cx| {
            let dispatch = call.dispatch.to_inner(&mut cx);
            let object = written.object.to_inner(&mut cx);
            let handle = cx.boxed(handle);
            let settled = Mutex::new(Some(settled));
            let done = JsFunction::new(&mut cx, move |mut cx| {
                let rejected = cx.argument::<JsBoolean>(0)?.value(&mut cx);
                let outcome = match rejected {
                    true => Err(cx.argument::<JsObject>(1)?.root(&mut cx)),
                    false => Ok(()),
                };
                if let Some(settled) = settled
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                {
                    let _ = settled.send(outcome);
                }
                Ok(cx.undefined())
            })?;
            dispatch
                .bind(&mut cx)
                .arg(object)?
                .arg(handle)?
                .arg(done)?
                .exec()
        });
        match settlement.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(rejected)) => Err(Failed::With(rejected, "rejected")),
            Err(_) => Err(Failed::Unrun("could not be run on the JavaScript thread")),
        }
    }
}

/// A migrator built from JavaScript's list: pgorm-migration's `Migrations`,
/// its ledger placed and its channel named as given.
// [spec:pgorm:req:bindings.migrations]
pub(crate) struct MigrationsHandle(Listed<Root<JsObject>>);

impl Finalize for MigrationsHandle {}

/// `value` as a string, `None` when it is `null` or undefined.
fn optional_string<'cx>(
    cx: &mut FunctionContext<'cx>,
    value: Handle<'cx, JsValue>,
    what: &str,
) -> NeonResult<Option<String>> {
    if value.is_a::<JsNull, _>(cx) || value.is_a::<JsUndefined, _>(cx) {
        return Ok(None);
    }
    match value.downcast::<JsString, _>(cx) {
        Ok(text) => Ok(Some(text.value(cx))),
        Err(_) => cx.throw_type_error(format!("{what} is a string")),
    }
}

/// A migration written in JavaScript, read once as the migrator is built.
// [spec:pgorm:req:bindings.migrations]    name, checksum and commitsAfter read once
fn written<'cx>(
    cx: &mut FunctionContext<'cx>,
    object: Handle<'cx, JsObject>,
) -> NeonResult<Written> {
    let name = object.get_value(cx, "name")?;
    let Ok(name) = name.downcast::<JsString, _>(cx) else {
        return cx.throw_type_error("a migration has a string name");
    };
    let name = name.value(cx);
    let up = object.get_value(cx, "up")?;
    if !up.is_a::<JsFunction, _>(cx) {
        return cx.throw_type_error(format!("migration \"{name}\" has no up method"));
    }
    let checksum = object.get_value(cx, "checksum")?;
    let checksum = optional_string(cx, checksum, &format!("migration \"{name}\"'s checksum"))?;
    let commits_after = object.get_value(cx, "commitsAfter")?;
    let commits_after = if commits_after.is_a::<JsUndefined, _>(cx) {
        false
    } else {
        match commits_after.downcast::<JsBoolean, _>(cx) {
            Ok(flag) => flag.value(cx),
            Err(_) => {
                return cx
                    .throw_type_error(format!("migration \"{name}\"'s commitsAfter is a boolean"));
            }
        }
    };
    Ok(Written {
        name,
        checksum,
        commits_after,
        object: object.root(cx),
    })
}

/// `migrationsNew(migrations, tableName, tableSchema, channel)`.
// [spec:pgorm:req:bindings.migrations]
fn migrations_new(mut cx: FunctionContext) -> JsResult<JsBox<MigrationsHandle>> {
    let list = cx.argument::<JsArray>(0)?.to_vec(&mut cx)?;
    let mut entries = Vec::with_capacity(list.len());
    for item in list {
        if let Ok(sql) = item.downcast::<JsBox<SqlMigrationHandle>, _>(&mut cx) {
            entries.push(Entry::Sql(sql.0.clone()));
            continue;
        }
        let Ok(object) = item.downcast::<JsObject, _>(&mut cx) else {
            return cx.throw_type_error(
                "a migration is a SqlMigration or an object with a name and an up",
            );
        };
        entries.push(Entry::Written(Arc::new(written(&mut cx, object)?)));
    }
    let table_name = cx.argument::<JsValue>(1)?;
    let table_name = optional_string(&mut cx, table_name, "migrationTableName")?;
    let table_schema = cx.argument::<JsValue>(2)?;
    let table_schema = optional_string(&mut cx, table_schema, "migrationTableSchema")?;
    let channel = cx.argument::<JsValue>(3)?;
    let channel = optional_string(&mut cx, channel, "statementCacheChannel")?;
    Ok(cx.boxed(MigrationsHandle(Listed::new(
        entries,
        table_name,
        table_schema,
        channel,
    ))))
}

/// `migrationsUp(migrations, pool, steps, dispatch)`: pgorm-migration's `up`
/// on connections of the pool's own, rejecting with what a migration's `up`
/// rejected with when one did.
// [spec:pgorm:req:bindings.migrations]
fn migrations_up(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let handle = cx.argument::<JsBox<MigrationsHandle>>(0)?;
    let pool = cx.argument::<JsBox<PoolHandle>>(1)?.0.state.clone();
    let steps = cx.argument::<JsValue>(2)?;
    let steps = if steps.is_a::<JsNull, _>(&mut cx) || steps.is_a::<JsUndefined, _>(&mut cx) {
        None
    } else {
        let steps = steps
            .downcast_or_throw::<JsNumber, _>(&mut cx)?
            .value(&mut cx);
        if steps.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&steps) {
            return cx.throw_range_error("steps is a whole number of migrations");
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some(steps as u32)
    };
    let dispatch = cx.argument::<JsFunction>(3)?.root(&mut cx);
    if let Err(refused) = pool.ensure_open() {
        return settle::rejected(&mut cx, refused);
    }
    let call = Call {
        pool,
        channel: cx.channel(),
        dispatch,
    };
    let work = handle.0.up(call, steps);
    settle::promise(&mut cx, work, |cx, raised| match raised {
        None => Ok(cx.undefined().upcast()),
        Some(raised) => {
            let raised = raised.into_inner(cx).get_value(cx, "error")?;
            cx.throw(raised)
        }
    })
}

/// Each migration's name and whether the ledger records it applied.
struct Statuses(Vec<(String, bool)>);

impl Settled for Statuses {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        let array = JsArray::new(cx, self.0.len());
        for (at, (name, applied)) in (0_u32..).zip(self.0) {
            let pair = JsArray::new(cx, 2);
            let name = cx.string(name);
            pair.set(cx, 0, name)?;
            let applied = cx.boolean(applied);
            pair.set(cx, 1, applied)?;
            array.set(cx, at, pair)?;
        }
        Ok(array.upcast())
    }
}

struct Installed;

impl Settled for Installed {
    fn into_js<'cx>(self: Box<Self>, cx: &mut Cx<'cx>) -> JsResult<'cx, JsValue> {
        Ok(cx.undefined().upcast())
    }
}

/// `migrationsJob(migrations, install)`: the runner's `install`, or its
/// reading of every migration's status, as a job any executor runs.
// [spec:pgorm:req:bindings.migrations]    the runner's own install and reads
fn migrations_job(mut cx: FunctionContext) -> JsResult<JsBox<JobHandle>> {
    let migrator = cx
        .argument::<JsBox<MigrationsHandle>>(0)?
        .0
        .ledger::<Call>();
    let install = cx.argument::<JsBoolean>(1)?.value(&mut cx);
    Ok(cx.boxed(JobHandle::new(Box::new(move |db, secrets| {
        async move {
            if install {
                migrator
                    .install(&db)
                    .await
                    .map(|()| Box::new(Installed) as Output)
                    .map_err(|error| failure(&error, secrets))
            } else {
                statuses(&migrator, &db)
                    .await
                    .map(|statuses| Box::new(Statuses(statuses)) as Output)
                    .map_err(|error| failure(&error, secrets))
            }
        }
        .boxed()
    }))))
}
