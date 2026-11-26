//! Native transaction endings and synchronous FIFO rollback on drop.

use super::{
    commands::Command,
    statement_store::Store,
    worker::{
        Client, Control, Failure, Operation, finish_statement, install_progress, step_statement,
    },
};
use crate::{Error, TursoCommitError};
use std::sync::Arc;
use tokio::sync::oneshot;
use turso_core::Insn;
use turso_pg::{PgConnection, TransactionState};

pub(super) struct End {
    commit: bool,
    savepoint: Option<String>,
    reply: Option<oneshot::Sender<Result<(), TursoCommitError>>>,
}

impl End {
    pub(super) fn close(self) {
        if let Some(reply) = self.reply {
            let _ = reply.send(Err(TursoCommitError::Failed(Error::closed())));
        }
    }
}

impl Client {
    pub(crate) async fn commit(&self, savepoint: Option<&str>) -> Result<(), TursoCommitError> {
        self.end(true, savepoint).await
    }

    pub(crate) async fn rollback(&self, savepoint: Option<&str>) -> Result<(), Error> {
        self.end(false, savepoint)
            .await
            .map_err(TursoCommitError::into_error)
    }

    async fn end(&self, commit: bool, savepoint: Option<&str>) -> Result<(), TursoCommitError> {
        let mut guard = self.request_guard().map_err(TursoCommitError::Failed)?;
        guard.operation.transaction_boundary(&self.control);
        if !commit {
            guard.operation.begin_restore();
        }
        let (reply, response) = oneshot::channel();
        let mut shutdown = self.control.shutdown.subscribe();
        self.submit(
            Command::End {
                request: End {
                    commit,
                    savepoint: savepoint.map(str::to_owned),
                    reply: Some(reply),
                },
                operation: Arc::clone(&guard.operation),
            },
            &mut shutdown,
        )
        .await
        .map_err(TursoCommitError::Failed)?;
        let result = self.response_raw(response, &mut shutdown).await;
        guard.complete();
        result.map_err(TursoCommitError::Failed)?
    }

    // [spec:pgorm:sem:turso.cleanup] rollback shares the request FIFO; saturation disposes the connection
    pub(crate) fn rollback_on_drop(&self, savepoint: Option<&str>) {
        if self.is_closed() {
            return;
        }
        let operation = self.control.new_operation();
        operation.begin_restore();
        operation.acknowledge();
        let command = Command::End {
            request: End {
                commit: false,
                savepoint: savepoint.map(str::to_owned),
                reply: None,
            },
            operation,
        };
        if self.sender.try_send(command).is_err() {
            self.control.shutdown();
        }
    }
}

/// Only explicit transaction boundaries have these native instructions;
/// ordinary DML uses Transaction. Inspect once at prepare, never parse SQL.
pub(super) fn is_boundary(statement: &turso_pg::Statement) -> bool {
    statement
        .get_program()
        .insns
        .iter()
        .any(|(instruction, _)| {
            matches!(
                instruction,
                Insn::AutoCommit { .. } | Insn::Savepoint { .. }
            )
        })
}

// [spec:pgorm:sem:turso.transactions] check native failure state and end it in one serialized command
pub(super) fn dispatch(
    request: End,
    connection: &PgConnection,
    operation: &Arc<Operation>,
    control: &Control,
    store: &mut Store,
) -> bool {
    let result = if request.commit && connection.transaction_state() == TransactionState::Aborted {
        aborted(connection, request.savepoint.as_deref(), operation, control)
    } else {
        finish(
            connection,
            request.commit,
            request.savepoint.as_deref(),
            operation,
            control,
        )
        .map_err(|failure| {
            if failure.fatal {
                control.quarantine();
            }
            TursoCommitError::Failed(failure.error)
        })
    };
    // Rollback failure is never safe for another borrower, even when no
    // caller is waiting to receive the drop cleanup's result.
    if !request.commit && result.is_err() {
        control.quarantine();
    }
    let fatal = control.closed.load(std::sync::atomic::Ordering::Acquire);
    control.complete(connection, operation, store);
    if let Some(reply) = request.reply {
        let _ = reply.send(result);
    }
    fatal
}

fn aborted(
    connection: &PgConnection,
    savepoint: Option<&str>,
    operation: &Arc<Operation>,
    control: &Control,
) -> Result<(), TursoCommitError> {
    // The native frontend refuses preparation in an aborted block, supplying
    // its actual 25P02. Healthy commits do not execute a probing query.
    let refusal = match connection.prepare("SELECT 1") {
        Err(error) => Error::turso(error),
        Ok(_) => {
            control.quarantine();
            return Err(TursoCommitError::Failed(Error::turso_worker(
                std::io::Error::other("native aborted state accepted an ordinary statement"),
            )));
        }
    };
    let rollback = finish(connection, false, savepoint, operation, control)
        .err()
        .map(|failure| {
            control.quarantine();
            failure.error
        });
    Err(TursoCommitError::Aborted { refusal, rollback })
}

fn finish(
    connection: &PgConnection,
    commit: bool,
    savepoint: Option<&str>,
    operation: &Arc<Operation>,
    control: &Control,
) -> Result<(), Failure> {
    operation.transaction_boundary(control);
    let sql = match (commit, savepoint) {
        (true, None) => "COMMIT".to_owned(),
        (false, None) => "ROLLBACK".to_owned(),
        (true, Some(name)) => format!("RELEASE {name}"),
        (false, Some(name)) => format!("ROLLBACK TO {name}"),
    };
    let mut statement = match connection.prepare(&sql) {
        Ok(statement) => statement,
        Err(error) => {
            return validate_outcome(
                connection,
                commit,
                savepoint.is_some(),
                false,
                Err(Failure::recoverable(Error::turso(error))),
            );
        }
    };
    install_progress(connection, operation, control);
    let outcome = step_statement(&mut statement, operation, control);
    // reset clears this flag, so capture the actual outcome before cleanup.
    let rolled_back = statement.command_tag_override() == Some("ROLLBACK");
    let result = finish_statement(connection, &mut statement, operation, control, outcome);
    validate_outcome(connection, commit, savepoint.is_some(), rolled_back, result)
}

fn validate_outcome(
    connection: &PgConnection,
    commit: bool,
    savepoint: bool,
    rolled_back: bool,
    result: Result<(), Failure>,
) -> Result<(), Failure> {
    let state = connection.transaction_state();
    let expected = if savepoint {
        TransactionState::Active
    } else {
        TransactionState::Idle
    };
    match result {
        Ok(()) if state == expected && !(commit && rolled_back) => Ok(()),
        Ok(()) => Err(Failure {
            error: Error::turso_worker(std::io::Error::other(
                "native transaction end did not establish the requested outcome",
            )),
            fatal: true,
        }),
        Err(failure) if failure.error.is_turso_cleanup_failure() => Err(failure),
        Err(failure) if !failure.fatal && !savepoint && state == TransactionState::Idle => {
            Err(failure)
        }
        Err(failure) => Err(Failure {
            // Preserve diagnostics while marking an unestablished outcome
            // in the same structured cleanup envelope the retry gate checks.
            error: Error::turso_cleanup(
                failure.error,
                Error::turso_worker(std::io::Error::other(
                    "native transaction outcome is unknown after failed end",
                )),
            ),
            fatal: true,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:pgorm:sem:turso.transactions/test] a retryable native cause requires a known ended transaction
    #[tokio::test]
    async fn failed_end_requires_known_outcome() {
        let database = crate::native::Database::open(std::path::Path::new(":memory:"))
            .await
            .unwrap();
        let connection = PgConnection::connect(&database.inner).unwrap();
        connection.execute("BEGIN").unwrap();
        let failure = validate_outcome(
            &connection,
            true,
            false,
            false,
            Err(Failure::recoverable(Error::turso(
                turso_core::LimboError::WriteWriteConflict.into(),
            ))),
        )
        .err()
        .unwrap();
        assert!(failure.fatal);
        assert!(failure.error.is_turso_cleanup_failure());
        assert_eq!(failure.error.code().unwrap().code(), "40001");
        connection.execute("ROLLBACK").unwrap();
        let failure = validate_outcome(
            &connection,
            true,
            false,
            false,
            Err(Failure::recoverable(Error::turso(
                turso_core::LimboError::WriteWriteConflict.into(),
            ))),
        )
        .err()
        .unwrap();
        assert!(!failure.fatal);
        assert!(!failure.error.is_turso_cleanup_failure());
        assert_eq!(failure.error.code().unwrap().code(), "40001");
        connection.close().unwrap();
    }
}
