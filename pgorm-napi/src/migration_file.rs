//! Generated DDL frozen as a migration written as a `.sql` file: pgorm's
//! [`MigrationFile`], whose text and whose freezing are pgorm's alone, behind
//! the handles `lib/migration-file.js` wraps.

use neon::prelude::*;
use pgorm::{DisjointChange, MigrationFile};

use crate::errors::{Failure, throw};

/// A frozen migration file, as `lib/migration-file.js` holds it.
pub(crate) struct FileHandle(MigrationFile);

impl Finalize for FileHandle {}

/// A disjoint kind change as pgorm wrote it, which the change object
/// `lib/disjoint-schema.js` makes keeps so that `migrationFiles` freezes it
/// as pgorm does.
pub(crate) struct ChangeHandle(pub(crate) DisjointChange);

impl Finalize for ChangeHandle {}

type Export = fn(FunctionContext) -> JsResult<JsValue>;

const EXPORTS: &[(&str, Export)] = &[
    ("migrationFile", migration_file),
    ("migrationFileParts", parts),
    ("migrationFileFreeze", freeze),
    ("disjointMigrationFiles", disjoint_files),
];

pub(crate) fn export(cx: &mut ModuleContext) -> NeonResult<()> {
    for (name, export) in EXPORTS {
        cx.export_function(name, *export)?;
    }
    Ok(())
}

/// `migrationFile(name, sql, commitsAfter)`: the statements' SQL frozen as
/// the migration `name`.
// [spec:pgorm:req:bindings.migration-file]
fn migration_file(mut cx: FunctionContext) -> JsResult<JsValue> {
    let name = cx.argument::<JsString>(0)?.value(&mut cx);
    let statements = cx.argument::<JsArray>(1)?.to_vec(&mut cx)?;
    let commits_after = cx.argument::<JsBoolean>(2)?.value(&mut cx);
    let mut sql = Vec::with_capacity(statements.len());
    for statement in statements {
        sql.push(
            statement
                .downcast_or_throw::<JsString, _>(&mut cx)?
                .value(&mut cx),
        );
    }
    let file = MigrationFile::new(name, sql);
    let file = match commits_after {
        true => file.committing_after(),
        false => file,
    };
    Ok(cx.boxed(FileHandle(file)).upcast())
}

/// `migrationFileParts(file)`: `[name, text, commitsAfter]`.
// [spec:pgorm:req:bindings.migration-file]
fn parts(mut cx: FunctionContext) -> JsResult<JsValue> {
    let handle = cx.argument::<JsBox<FileHandle>>(0)?;
    let file = &handle.0;
    let array = JsArray::new(&mut cx, 3);
    let name = cx.string(file.name());
    array.set(&mut cx, 0, name)?;
    let text = cx.string(file.text());
    array.set(&mut cx, 1, text)?;
    let commits_after = cx.boolean(file.commits_after());
    array.set(&mut cx, 2, commits_after)?;
    Ok(array.upcast())
}

/// `migrationFileFreeze(file, directory)`: whether it wrote `<name>.sql`
/// there, never rewriting one; a name that is no file stem is a
/// `ConstructionError`, any other failure an `Error` with its message.
// [spec:pgorm:req:bindings.migration-file]
fn freeze(mut cx: FunctionContext) -> JsResult<JsValue> {
    let handle = cx.argument::<JsBox<FileHandle>>(0)?;
    let directory = cx.argument::<JsString>(1)?.value(&mut cx);
    match handle.0.freeze(&directory) {
        Ok(written) => Ok(cx.boolean(written).upcast()),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
            throw(&mut cx, Failure::Construction(error.to_string()))
        }
        Err(error) => cx.throw_error(format!(
            "cannot write {}.sql in {directory}: {error}",
            handle.0.name()
        )),
    }
}

/// `disjointMigrationFiles(change, name)`: pgorm's
/// `DisjointChange::migration_files`, a file per step.
// [spec:pgorm:req:bindings.migration-file]
fn disjoint_files(mut cx: FunctionContext) -> JsResult<JsValue> {
    let files = cx
        .argument::<JsBox<ChangeHandle>>(0)?
        .0
        .migration_files(&cx.argument::<JsString>(1)?.value(&mut cx));
    let array = JsArray::new(&mut cx, files.len());
    for (index, file) in files.into_iter().enumerate() {
        let handle = cx.boxed(FileHandle(file));
        array.set(&mut cx, u32::try_from(index).unwrap_or(u32::MAX), handle)?;
    }
    Ok(array.upcast())
}
