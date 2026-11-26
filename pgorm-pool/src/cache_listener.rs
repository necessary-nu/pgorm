//! The connection a pool keeps listening on its
//! [`statement_cache_channel`](crate::ManagerConfig::statement_cache_channel),
//! so that a clear asked for by another process reaches this one.

use std::{
    future::{Future, poll_fn},
    mem,
    pin::pin,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    task::Poll,
    time::Duration,
};

use pgorm_driver::{AsyncMessage, Client, Config as PgConfig, Error};
use tokio::task::JoinHandle;

use crate::{Connect, RawConnection};

/// The channel pgorm-migration's runner notifies once a batch that applied a
/// migration commits, and so the one a pool listens on to hear of it.
// [spec:pgorm:req:conn.pool.statement-cache.listen+1]    the channel both sides default to
pub const STATEMENT_CACHE_CHANNEL: &str = "pgorm_statement_caches";

/// The most of an identifier PostgreSQL keeps: `NAMEDATALEN - 1` bytes. A
/// longer name given to `LISTEN` is truncated with only a notice, and
/// `pg_notify` refuses it, so nothing could notify the listener by name.
const CHANNEL_BYTES: usize = 63;

/// The wait before reconnecting after the listening connection is lost or
/// cannot be opened. It doubles with each attempt that fails to `LISTEN`, up
/// to [`LONGEST_RETRY`], and starts again here once one succeeds.
const FIRST_RETRY: Duration = Duration::from_millis(100);

const LONGEST_RETRY: Duration = Duration::from_secs(10);

/// Sent on the listening session before its `LISTEN`, so that a server,
/// database or role whose `idle_session_timeout` ends idle sessions does not
/// end this one each period, every reconnection clearing every cache. The
/// setting's context is `user`, so any role may change it for its own
/// session, and the change reaches no other.
// [spec:pgorm:req:conn.pool.statement-cache.listen+1]    the listening session does not idle out
const NEVER_IDLE_OUT: &str = "SET idle_session_timeout = 0";

/// Whether `channel` is a name `LISTEN` takes without truncating it, and
/// `pg_notify` accepts: from 1 to [`CHANNEL_BYTES`] bytes.
pub(crate) fn is_channel_name(channel: &str) -> bool {
    !channel.is_empty() && channel.len() <= CHANNEL_BYTES
}

/// What advancing a pool's [`StatementCaches`](crate::StatementCaches)
/// generation means: every connection's cache is cleared before it is next
/// lent.
pub(crate) fn advance(generation: &AtomicU64) {
    let _ = generation.fetch_add(1, Ordering::AcqRel);
}

enum State {
    Idle,
    Running(JoinHandle<()>),
    Stopped,
}

/// The listening task's slot in its [`Manager`](crate::Manager): started once,
/// by the pool's first connection, and stopped once, after which it never
/// starts again.
pub(crate) struct CacheListener {
    state: Mutex<State>,
}

impl Default for CacheListener {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::Idle),
        }
    }
}

impl CacheListener {
    /// Spawn the task `listen` returns, unless one has been started or the
    /// listener stopped.
    pub(crate) fn start<F>(&self, listen: impl FnOnce() -> F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*state, State::Idle) {
            *state = State::Running(tokio::spawn(listen()));
        }
    }

    /// End the task, closing its connection, and keep it from starting.
    pub(crate) fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let State::Running(task) = mem::replace(&mut *state, State::Stopped) {
            task.abort();
        }
    }
}

/// Listen on `channel` through connections opened by `connect`, advancing
/// `generation` once each time a `LISTEN` takes effect and once per
/// notification, until aborted.
///
/// It holds nothing of the pool but these, so the pool can be dropped while
/// it runs: the [`Manager`](crate::Manager)'s drop aborts it.
// [spec:pgorm:req:conn.pool.statement-cache.listen+1]    reconnect with a bounded backoff, clear on each LISTEN
pub(crate) async fn listen_for_clears(
    connect: Arc<dyn Connect>,
    pg_config: PgConfig,
    channel: String,
    generation: Arc<AtomicU64>,
) {
    if !is_channel_name(&channel) {
        tracing::error!(
            channel,
            "statement_cache_channel is not a channel name of 1 to {CHANNEL_BYTES} bytes; \
             no statement-cache clear will be heard"
        );
        return;
    }
    let listen = format!("LISTEN \"{}\"", channel.replace('"', "\"\""));
    let mut retry = FIRST_RETRY;
    loop {
        match connect.connect_raw(&pg_config).await {
            Ok((client, connection)) => {
                let (listened, error) =
                    listening(&client, connection, &listen, &channel, &generation).await;
                if listened {
                    retry = FIRST_RETRY;
                }
                match error {
                    Some(error) => tracing::warn!(
                        channel,
                        "statement-cache listener lost its connection: {error}"
                    ),
                    None => tracing::warn!(channel, "statement-cache listener's connection closed"),
                }
            }
            Err(error) => {
                tracing::warn!(channel, "statement-cache listener cannot connect: {error}");
            }
        }
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(LONGEST_RETRY);
    }
}

/// Drive `connection` until it ends, sending [`NEVER_IDLE_OUT`] and then
/// `listen` on it first. Answers whether the `LISTEN` took effect, and the
/// error the connection ended with.
async fn listening(
    client: &Client,
    mut connection: Box<dyn RawConnection>,
    listen: &str,
    channel: &str,
    generation: &AtomicU64,
) -> (bool, Option<Error>) {
    // The `LISTEN` goes last and alone: a session's last statement is what
    // `pg_stat_activity` shows, and how an operator tells this session apart.
    // A `SET` the server refuses costs only the timeout's protection, so the
    // `LISTEN` goes ahead without it.
    let set_up = async {
        match client.batch_execute(NEVER_IDLE_OUT).await {
            Err(error) if error.as_db_error().is_some() => tracing::warn!(
                channel,
                "statement-cache listener cannot turn off idle_session_timeout: {error}"
            ),
            Err(error) => return Err(error),
            Ok(()) => {}
        }
        client.batch_execute(listen).await
    };
    // The requests complete only while the connection is polled, so the two
    // are driven together, the requests until the `LISTEN` answers.
    let mut request = pin!(set_up);
    let mut listened = false;
    let error = poll_fn(|cx| {
        if !listened && let Poll::Ready(answer) = request.as_mut().poll(cx) {
            match answer {
                Ok(()) => {
                    listened = true;
                    advance(generation);
                    tracing::debug!(channel, "listening; statement caches cleared");
                }
                Err(error) => return Poll::Ready(Some(error)),
            }
        }
        loop {
            match connection.poll_message(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(AsyncMessage::Notification(_)))) => {
                    advance(generation);
                    tracing::debug!(channel, "notified; statement caches cleared");
                }
                Poll::Ready(Some(Ok(_))) => {}
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Some(error)),
                Poll::Ready(None) => return Poll::Ready(None),
            }
        }
    })
    .await;
    (listened, error)
}
