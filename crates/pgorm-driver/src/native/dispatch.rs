//! Dispatch one serialized native command and publish completion last.
use super::{
    commands::Command,
    query,
    statement_store::Store,
    transaction,
    worker::{Control, Failure, Operation, cancelled, execute_script, synchronize_preparation},
};
use std::sync::Arc;
use tokio::{runtime::Handle, sync::watch};
use turso_pg::PgConnection;

pub(super) struct Context<'a> {
    pub(super) connection: &'a PgConnection,
    pub(super) database: &'a turso_core::Database,
    pub(super) store: &'a mut Store,
    pub(super) operation: &'a Arc<Operation>,
    pub(super) control: &'a Control,
    pub(super) shutdown: &'a mut watch::Receiver<bool>,
    pub(super) runtime: &'a Handle,
}

pub(super) fn run(command: Command, context: Context<'_>) -> bool {
    let Context {
        connection,
        database,
        store,
        operation,
        control,
        shutdown,
        runtime,
    } = context;
    match command {
        Command::Batch { sql, reply, .. } => {
            let result = execute_script(connection, &sql, operation, control);
            let fatal = result.as_ref().err().is_some_and(|error| error.fatal);
            if fatal {
                control.quarantine();
            }
            control.complete(connection, operation, store);
            let _ = reply.send(result.map_err(|failure| failure.error));
            fatal
        }
        Command::Prepare {
            owner,
            sql,
            types,
            reply,
            ..
        } => {
            let result = if control.cancelled(operation) {
                Err(Failure::recoverable(cancelled()))
            } else {
                synchronize_preparation(connection, database, operation, control).and_then(|()| {
                    store
                        .prepare(connection, owner, &sql, types)
                        .map_err(Failure::recoverable)
                })
            };
            let fatal = result.as_ref().err().is_some_and(|error| error.fatal);
            if fatal {
                control.quarantine();
            }
            control.complete(connection, operation, store);
            let _ = reply.send(result.map_err(|failure| failure.error));
            fatal
        }
        Command::Execute {
            id,
            params,
            reply,
            _statement: statement,
            ..
        } => {
            let result = match store.get(id, connection) {
                Ok(entry) => {
                    let result = query::execute(connection, entry, params, operation, control);
                    // A native failure remains the primary error even if
                    // step reprepared first; the public handle is still
                    // stale and must leave the cache before another use.
                    if !entry.metadata_is_current() {
                        statement.mark_invalidated();
                    }
                    result
                }
                Err(error) => Err(Failure::recoverable(error)),
            };
            let fatal = result.as_ref().err().is_some_and(|error| error.fatal);
            if fatal {
                control.quarantine();
            }
            if result
                .as_ref()
                .err()
                .is_some_and(|failure| failure.error.is_statement_invalidated())
            {
                statement.mark_invalidated();
            }
            control.complete(connection, operation, store);
            let _ = reply.send(result.map_err(|failure| failure.error));
            fatal
        }
        Command::Query {
            id,
            statement,
            params,
            ready,
            output,
            ..
        } => query::query(
            id,
            query::Request {
                statement,
                params,
                ready,
                output,
            },
            query::Context {
                connection,
                store,
                operation,
                control,
                shutdown,
                runtime,
            },
        ),
        Command::End { request, .. } => {
            transaction::dispatch(request, connection, operation, control, store)
        }
    }
}
