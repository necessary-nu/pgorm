use super::{
    codecs,
    rows::{BATCH_BYTES, BATCH_ROWS, Output},
    statement_store::Entry,
    worker::{
        Control, Failure, Operation, cancelled, finish_statement, install_progress, next_row,
    },
};
use crate::{Error, Statement};
use std::sync::Arc;
use tokio::{
    runtime::Handle,
    sync::{oneshot, watch},
};
use turso_core::Value;
use turso_pg::PgConnection;

fn bind(entry: &mut Entry, params: Vec<Value>) -> Result<(), Error> {
    let count = entry.statement.parameters_count();
    if params.len() != count {
        return Err(Error::parameters(params.len(), count));
    }
    // Move owned values into native slots; clearing first avoids a second
    // string/blob copy into the preceding execution's retained binding.
    entry.statement.clear_bindings();
    for (index, value) in params.into_iter().enumerate() {
        entry
            .statement
            .bind_at((index + 1).try_into().unwrap(), value)
            .map_err(Error::turso)?;
    }
    Ok(())
}

fn run(
    connection: &PgConnection,
    entry: &mut Entry,
    operation: &Arc<Operation>,
    control: &Control,
    mut row: impl FnMut(&turso_core::Row) -> Result<(), Error>,
) -> Result<u64, Failure> {
    install_progress(connection, operation, control);
    let mut returned = 0_u64;
    let outcome = loop {
        let stepped = next_row(&mut entry.statement, operation, control);
        // Native step can automatically reprepare after our preflight, both
        // at startup and on an internal SchemaUpdated retry. Never expose a
        // row/count under metadata from the previous compiled program. This
        // check is after execution: the refusal does not prove zero effects.
        if stepped.is_ok() && !entry.metadata_is_current() {
            break Err(Error::turso_statement_invalidated());
        }
        match stepped {
            Ok(true) => {
                returned = match returned.checked_add(1) {
                    Some(count) => count,
                    None => {
                        break Err(Error::turso_worker(std::io::Error::other(
                            "native row count overflow",
                        )));
                    }
                };
                if let Err(error) =
                    row(entry.statement.row().expect("Row step must expose its row"))
                {
                    break Err(error);
                }
            }
            Ok(false) => {
                let affected = if entry.statement.get_program().change_cnt_on {
                    match u64::try_from(entry.statement.n_change()) {
                        Ok(changes) => changes,
                        Err(_) => {
                            break Err(Error::turso_worker(std::io::Error::other(
                                "negative native affected-row count",
                            )));
                        }
                    }
                } else {
                    returned
                };
                break Ok(affected);
            }
            Err(error) => break Err(error),
        }
    };
    let affected = outcome.as_ref().ok().copied();
    finish_statement(
        connection,
        &mut entry.statement,
        operation,
        control,
        outcome.map(|_| ()),
    )?;
    entry.statement.clear_bindings();
    Ok(affected.expect("successful execution has an affected-row count"))
}

pub(super) fn execute(
    connection: &PgConnection,
    entry: &mut Entry,
    params: Vec<Value>,
    operation: &Arc<Operation>,
    control: &Control,
) -> Result<u64, Failure> {
    if control.cancelled(operation) {
        return Err(Failure::recoverable(cancelled()));
    }
    if let Err(error) = bind(entry, params) {
        return finish_statement(
            connection,
            &mut entry.statement,
            operation,
            control,
            Err(error),
        )
        .map(|_| 0);
    }
    run(connection, entry, operation, control, |_| Ok(()))
}

pub(super) struct Request {
    pub(super) statement: Statement,
    pub(super) params: Vec<Value>,
    pub(super) ready: oneshot::Sender<Result<(), Error>>,
    pub(super) output: Output,
}

pub(super) struct Context<'a> {
    pub(super) connection: &'a PgConnection,
    pub(super) operation: &'a Arc<Operation>,
    pub(super) control: &'a Control,
    pub(super) shutdown: &'a mut watch::Receiver<bool>,
    pub(super) runtime: &'a Handle,
}

pub(super) fn query(entry: &mut Entry, request: Request, context: Context<'_>) -> bool {
    let Request {
        statement,
        params,
        ready,
        output,
    } = request;
    let Context {
        connection,
        operation,
        control,
        shutdown,
        runtime,
    } = context;
    if control.cancelled(operation) {
        let _ = ready.send(Err(cancelled()));
        output.finish(Err(cancelled()));
        return false;
    }
    if let Err(error) = bind(entry, params) {
        let result = finish_statement(
            connection,
            &mut entry.statement,
            operation,
            control,
            Err(error),
        );
        let fatal = result.as_ref().err().is_some_and(|failure| failure.fatal);
        if fatal {
            control.quarantine();
        }
        let _ = ready.send(result.map_err(|failure| failure.error));
        output.finish(Err(Error::closed()));
        return fatal;
    }
    if ready.send(Ok(())).is_err() {
        let result = finish_statement(
            connection,
            &mut entry.statement,
            operation,
            control,
            Err(cancelled()),
        );
        let fatal = result.as_ref().err().is_some_and(|failure| failure.fatal);
        if fatal {
            control.quarantine();
        }
        return fatal;
    }
    let mut batch = Vec::with_capacity(BATCH_ROWS);
    let mut bytes = 0;
    // The plan is small and prepared once. Split the entry's fields so native
    // row values remain borrowed until each ordinary Row owns its bytes.
    let result_codecs = std::mem::take(&mut entry.result_codecs);
    let result = run(connection, entry, operation, control, |native| {
        let row = codecs::encode_row(statement.clone(), &result_codecs, native)?;
        bytes += row.raw_size_bytes();
        batch.push(row);
        if batch.len() >= BATCH_ROWS || bytes >= BATCH_BYTES {
            output.send(
                std::mem::replace(&mut batch, Vec::with_capacity(BATCH_ROWS)),
                operation,
                control,
                shutdown,
                runtime,
            )?;
            bytes = 0;
        }
        Ok(())
    });
    entry.result_codecs = result_codecs;
    let result = match result {
        Ok(affected) if !batch.is_empty() => output
            .send(batch, operation, control, shutdown, runtime)
            .map(|_| affected)
            .map_err(Failure::recoverable),
        result => result,
    };
    let fatal = result.as_ref().err().is_some_and(|failure| failure.fatal);
    if fatal {
        control.quarantine();
    }
    output.finish(result.map_err(|failure| failure.error));
    fatal
}
