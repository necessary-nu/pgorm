use super::Transaction;
use crate::{Client, Error};
use std::{error::Error as StdError, fmt};

/// Why a native transaction could not confirm a successful commit.
///
/// An aborted transaction is distinct from a failure of COMMIT itself. Pool
/// wrappers retain that distinction and any additional rollback failure.
// [spec:pgorm:sem:turso.transactions]
#[derive(Debug)]
pub enum TursoCommitError {
    /// The native commit or release failed, or its outcome is unknown.
    Failed(Error),
    /// A previous statement aborted the transaction; rollback was attempted.
    Aborted {
        /// The native connection's refusal to execute in its aborted state.
        refusal: Error,
        /// An additional failure while rolling back, if cleanup failed.
        rollback: Option<Error>,
    },
}

impl fmt::Display for TursoCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(error) => write!(formatter, "native commit failed: {error}"),
            Self::Aborted { refusal, rollback } => {
                write!(formatter, "native transaction was aborted: {refusal}")?;
                if let Some(rollback) = rollback {
                    write!(formatter, "; rollback failed: {rollback}")?;
                }
                Ok(())
            }
        }
    }
}

impl StdError for TursoCommitError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(match self {
            Self::Failed(error) => error,
            Self::Aborted { refusal, .. } => refusal,
        })
    }
}

impl TursoCommitError {
    pub(crate) fn into_error(self) -> Error {
        match self {
            TursoCommitError::Failed(error) => error,
            TursoCommitError::Aborted { refusal, rollback } => match rollback {
                Some(rollback) => Error::turso_cleanup(refusal, rollback),
                None => refusal,
            },
        }
    }
}

impl Transaction<'_> {
    /// Commits a native transaction while preserving an aborted outcome.
    ///
    /// Intended for pool wrappers that distinguish an earlier statement's
    /// abort from COMMIT failing. PostgreSQL transactions reject this method;
    /// their ordinary commit path retains its wire-protocol behavior.
    // [spec:pgorm:sem:turso.transactions]
    pub async fn commit_checked_turso(mut self) -> Result<(), TursoCommitError> {
        let Some(client) = self.client.inner().native() else {
            return Err(TursoCommitError::Failed(Error::turso_unsupported(
                "native checked commit on a PostgreSQL connection",
            )));
        };
        self.done = true;
        client
            .commit(
                self.savepoint
                    .as_ref()
                    .map(|savepoint| savepoint.name.as_str()),
            )
            .await
    }
}

// [spec:pgorm:sem:turso.cleanup]
pub(super) struct SavepointGuard<'a> {
    pub(super) client: &'a Client,
    pub(super) name: &'a str,
    pub(super) complete: bool,
}

impl Drop for SavepointGuard<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.client.__private_api_rollback(Some(self.name));
        }
    }
}
