//! LISTEN on a connection of its own: pgorm's `Listener`, which changes the
//! connection's `LISTEN` set, and its `Notifications` stream, both held by
//! the binding's handle. The connection is the listener's alone, opened with
//! a pool's configuration or a connection string's, and closed with the
//! listener or its pool; it is never one a pool lends.

use std::{fmt, num::NonZeroUsize, sync::Arc};

use futures_util::StreamExt;
use pgorm::{Config, Listener, Notifications};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    Connector, Dialect, Failure, Redactions, classify,
    connection::{POOL_CLOSED, PoolState},
};

const LISTENER_CLOSED: &str = "the listener is closed";

/// What a request on a listener asks of its connection: `LISTEN`,
/// `UNLISTEN` or `UNLISTEN *`, each channel sent as a quoted identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ListenRequest {
    Listen(String),
    Unlisten(String),
    UnlistenAll,
}

/// One notification, owned until the language's thread makes it a value:
/// its channel, its payload and the process id of the session that sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Received {
    pub channel: String,
    pub payload: String,
    pub process_id: i32,
}

/// A listener: its `LISTEN` set, and its notifications until it closes.
pub struct ListenerState {
    /// Its pool's closing, which closes it; one that never fires for a
    /// listener of no pool.
    pool_closed: CancellationToken,
    secrets: Redactions,
    dialect: Dialect,
    listener: Listener,
    /// `None` once closed: dropping the stream closes the connection.
    notifications: Arc<Mutex<Option<Notifications>>>,
    closed: CancellationToken,
}

impl fmt::Debug for ListenerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListenerState")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

/// Closes the listener if the request it guards is abandoned before its
/// reply: the server may have acted on a `LISTEN` or `UNLISTEN` whose answer
/// never arrived, so which channels the connection listens on is unknown.
struct CloseUnlessAnswered<'a>(Option<&'a ListenerState>);

impl Drop for CloseUnlessAnswered<'_> {
    fn drop(&mut self) {
        if let Some(state) = self.0.take() {
            state.close_now();
        }
    }
}

impl ListenerState {
    fn new(
        (listener, notifications): (Listener, Notifications),
        pool_closed: CancellationToken,
        secrets: Redactions,
        dialect: Dialect,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool_closed,
            secrets,
            dialect,
            listener,
            notifications: Arc::new(Mutex::new(Some(notifications))),
            closed: CancellationToken::new(),
        })
    }

    /// A listener on `pool`'s configuration, registered so that closing the
    /// pool closes it, holding at most `capacity` notifications unread.
    // [spec:pgorm:req:bindings.listen+3]
    pub async fn on_pool<O: Send + Sync + 'static>(
        pool: &PoolState<O>,
        capacity: NonZeroUsize,
        cancel: &CancellationToken,
    ) -> Result<Arc<Self>, Failure> {
        pool.ensure_open()?;
        let opened = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled),
            () = pool.closed.cancelled() => return Err(Failure::Lifecycle(POOL_CLOSED.to_owned())),
            opened = pool.pool.listener(capacity) => opened.map_err(|error| pool.failure(&error))?,
        };
        let state = Self::new(
            opened,
            pool.closed.clone(),
            pool.secrets.clone(),
            pool.dialect(),
        );
        pool.register_listener(&state)?;
        Ok(state)
    }

    /// A listener on a connection of its own, belonging to no pool, as
    /// pgorm's `connect_listener` opens one.
    // [spec:pgorm:req:bindings.listen+3]
    pub async fn connect(
        config: Config,
        connector: Connector,
        capacity: NonZeroUsize,
        dialect: Dialect,
        cancel: &CancellationToken,
    ) -> Result<Arc<Self>, Failure> {
        let secrets = Redactions::from_config(&config);
        let opened = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled),
            opened = crate::connect_listener(config, connector, capacity) => opened
                .map_err(|error| classify(&error, &secrets, dialect))?,
        };
        Ok(Self::new(
            opened,
            CancellationToken::new(),
            secrets,
            dialect,
        ))
    }

    /// Whether the listener or its pool has closed.
    pub fn is_closed(&self) -> bool {
        self.closed.is_cancelled() || self.pool_closed.is_cancelled()
    }

    /// Refuse once the listener or its pool has closed.
    pub fn ensure_open(&self) -> Result<(), Failure> {
        if self.pool_closed.is_cancelled() {
            return Err(Failure::Lifecycle(POOL_CLOSED.to_owned()));
        }
        if self.closed.is_cancelled() {
            Err(Failure::Lifecycle(LISTENER_CLOSED.to_owned()))
        } else {
            Ok(())
        }
    }

    /// Close at once, dropping the stream unless a pull holds it; that pull
    /// sees the close and drops it.
    pub fn close_now(&self) {
        self.closed.cancel();
        if let Ok(mut slot) = self.notifications.try_lock() {
            drop(slot.take());
        }
    }

    /// Close, and finish once the stream, and with it the connection, is
    /// released.
    pub async fn release(&self) {
        self.closed.cancel();
        drop(self.notifications.lock().await.take());
    }

    /// `LISTEN`, `UNLISTEN` or `UNLISTEN *`. A request cut off, by `cancel`
    /// or by its future being dropped, closes the listener: the statement may
    /// have reached the server, leaving the channels it listens on unknown.
    // [spec:pgorm:req:bindings.listen+3]
    // [spec:pgorm:req:bindings.cancellation]
    pub async fn send(
        &self,
        request: ListenRequest,
        cancel: &CancellationToken,
    ) -> Result<(), Failure> {
        self.ensure_open()?;
        let mut guard = CloseUnlessAnswered(Some(self));
        let sent = async {
            match &request {
                ListenRequest::Listen(channel) => self.listener.listen(channel).await,
                ListenRequest::Unlisten(channel) => self.listener.unlisten(channel).await,
                ListenRequest::UnlistenAll => self.listener.unlisten_all().await,
            }
        };
        let sent = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled),
            () = self.pool_closed.cancelled() => return Err(Failure::Lifecycle(POOL_CLOSED.to_owned())),
            () = self.closed.cancelled() => return Err(Failure::Lifecycle(LISTENER_CLOSED.to_owned())),
            sent = sent => sent,
        };
        guard.0 = None;
        sent.map_err(|error| classify(&error, &self.secrets, self.dialect))
    }

    /// The next notification, or `None` once the listener has closed, before
    /// the pull was made or while it waits. A pool that closes under a
    /// waiting pull fails it. A failure the stream ends with, the connection
    /// lost or its capacity overrun, closes the listener; a pull cut off by
    /// `cancel` does not, since the notification it waited for stays queued
    /// for the next pull.
    // [spec:pgorm:req:bindings.listen+3]
    pub async fn next(&self, cancel: &CancellationToken) -> Result<Option<Received>, Failure> {
        let ended = || {
            if self.pool_closed.is_cancelled() {
                Err(Failure::Lifecycle(POOL_CLOSED.to_owned()))
            } else {
                Ok(None)
            }
        };
        if self.is_closed() {
            return ended();
        }
        let mut slot = self.notifications.clone().try_lock_owned().map_err(|_| {
            Failure::Lifecycle(
                "the listener is busy: one notification is pulled at a time".to_owned(),
            )
        })?;
        let Some(notifications) = slot.as_mut() else {
            return ended();
        };
        let item = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled),
            () = self.pool_closed.cancelled() => {
                drop(slot.take());
                return ended();
            }
            () = self.closed.cancelled() => {
                drop(slot.take());
                return ended();
            }
            item = notifications.next() => item,
        };
        match item {
            Some(Ok(notification)) => Ok(Some(Received {
                channel: notification.channel().to_owned(),
                payload: notification.payload().to_owned(),
                process_id: notification.process_id(),
            })),
            Some(Err(error)) => {
                drop(slot.take());
                self.closed.cancel();
                Err(classify(&error, &self.secrets, self.dialect))
            }
            None => {
                drop(slot.take());
                self.closed.cancel();
                Ok(None)
            }
        }
    }
}
