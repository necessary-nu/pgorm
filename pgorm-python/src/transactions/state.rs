use super::{
    scope::Command,
    work::{Output, Work},
};
use crate::{
    errors::{InternalError, LifecycleError},
    runtime::ConnectionState,
};
use pyo3::prelude::*;
use std::sync::{Arc, Weak};
use tokio::sync::{Mutex, OwnedMutexGuard, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub(super) struct Root {
    pub(super) aborted: CancellationToken,
    pub(super) finished: CancellationToken,
}

impl Root {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            aborted: CancellationToken::new(),
            finished: CancellationToken::new(),
        })
    }
}

pub(crate) struct State {
    pub(crate) connection: Arc<ConnectionState>,
    pub(super) commands: mpsc::UnboundedSender<Command>,
    pub(super) busy: Arc<Mutex<()>>,
    pub(super) abandoned: CancellationToken,
    pub(super) finished: CancellationToken,
    pub(super) root: Arc<Root>,
    pub(super) released: CancellationToken,
    pub(super) parent: Option<Weak<State>>,
}

impl State {
    pub(super) fn channel(
        connection: Arc<ConnectionState>,
        parent: Option<&Arc<Self>>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Command>) {
        let root = parent.map_or_else(Root::new, |parent| parent.root.clone());
        Self::rooted(connection, parent, root)
    }

    /// One attempt of a retrying transaction, sharing the root every attempt
    /// on its connection shares.
    pub(super) fn attempt(
        connection: Arc<ConnectionState>,
        root: Arc<Root>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Command>) {
        Self::rooted(connection, None, root)
    }

    fn rooted(
        connection: Arc<ConnectionState>,
        parent: Option<&Arc<Self>>,
        root: Arc<Root>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Command>) {
        let (commands, receiver) = mpsc::unbounded_channel();
        let abandoned = parent.map_or_else(CancellationToken::new, |parent| {
            parent.abandoned.child_token()
        });
        (
            Arc::new(Self {
                connection,
                commands,
                busy: Arc::new(Mutex::new(())),
                abandoned,
                finished: CancellationToken::new(),
                root,
                released: CancellationToken::new(),
                parent: parent.map(Arc::downgrade),
            }),
            receiver,
        )
    }

    pub(crate) fn check(&self, py: Python<'_>) -> PyResult<()> {
        self.connection.pool.check_owner(py)?;
        self.ensure_open()
    }

    pub(crate) fn closed(&self) -> bool {
        self.finished.is_cancelled()
            || self.abandoned.is_cancelled()
            || self.root.aborted.is_cancelled()
            || self.root.finished.is_cancelled()
            || self.connection.cancelled.is_cancelled()
            || self.connection.pool.cancelled.is_cancelled()
    }

    pub(super) fn ensure_open(&self) -> PyResult<()> {
        if self.closed() {
            Err(LifecycleError::new_err("transaction is closed"))
        } else {
            Ok(())
        }
    }

    pub(crate) async fn run(self: &Arc<Self>, work: Work) -> PyResult<Output> {
        self.request(Some(work), false, false).await
    }

    /// Run `work`, or with none end the frame: commit it, or roll it back,
    /// telling a retrying transaction whether the failure that ended it is
    /// one to retry.
    pub(super) async fn request(
        self: &Arc<Self>,
        work: Option<Work>,
        commit: bool,
        retryable: bool,
    ) -> PyResult<Output> {
        let (result, _busy, mut pending) = self
            .exchange(|reply| match work {
                Some(work) => Command::Run { work, reply },
                None => Command::Finish {
                    commit,
                    retryable,
                    reply,
                },
            })
            .await?;
        pending.complete();
        result.map_err(|failure| failure.raised(&self.connection.pool.secrets))
    }

    /// Reserve the transaction, send its owner the command `command` makes
    /// of the reply it is given, and wait for that reply, which comes with
    /// the reservation held and the request still pending: until the caller
    /// completes it, the transaction is discarded if the caller is dropped.
    pub(super) async fn exchange<T>(
        self: &Arc<Self>,
        command: impl FnOnce(oneshot::Sender<T>) -> Command,
    ) -> PyResult<(T, OwnedMutexGuard<()>, Pending)> {
        self.ensure_open()?;
        let busy = self.busy.clone().try_lock_owned().map_err(|_| {
            LifecycleError::new_err("transaction has an active operation or nested savepoint")
        })?;
        let (reply, receive) = oneshot::channel();
        let pending = Pending::new(self.root.aborted.clone());
        self.commands
            .send(command(reply))
            .map_err(|_| LifecycleError::new_err("transaction owner has closed"))?;
        let result = receive
            .await
            .map_err(|_| LifecycleError::new_err("transaction owner has closed"))?;
        Ok((result, busy, pending))
    }

    pub(super) async fn begin(self: &Arc<Self>) -> PyResult<Arc<Self>> {
        self.ensure_open()?;
        let lock = self.busy.clone().try_lock_owned().map_err(|_| {
            LifecycleError::new_err("transaction has an active operation or nested savepoint")
        })?;
        let (child, receiver) = Self::channel(self.connection.clone(), Some(self));
        let (opened, receive) = oneshot::channel();
        let mut guard = Pending::new(self.root.aborted.clone());
        self.commands
            .send(Command::Begin {
                child: child.clone(),
                receiver,
                opened,
                parent_lock: lock,
            })
            .map_err(|_| LifecycleError::new_err("transaction owner has closed"))?;
        let result = receive.await.map_err(|_| {
            InternalError::new_err("transaction owner ended during savepoint creation")
        })?;
        guard.complete();
        result?;
        Ok(child)
    }
}

/// A cancelled request has an unknown outcome and invalidates the entire lease.
pub(super) struct Pending {
    aborted: CancellationToken,
    complete: bool,
}
impl Pending {
    pub(super) fn new(aborted: CancellationToken) -> Self {
        Self {
            aborted,
            complete: false,
        }
    }
    pub(super) fn complete(&mut self) {
        self.complete = true;
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        if !self.complete {
            self.aborted.cancel();
        }
    }
}

pub(super) struct Finished(pub(super) CancellationToken);
impl Drop for Finished {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
