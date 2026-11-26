use std::{
    collections::VecDeque,
    fmt, mem,
    num::NonZeroUsize,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    task::{Context, Poll, Waker},
};

use futures::Stream;
use pgorm_driver::{AsyncMessage, Client, Notification};
use pgorm_pool::{ConfigConnectImpl, Connect, RawConnection};
use tokio::task::JoinHandle;

use super::{Config, MakeTlsConnect, Socket, TlsConnect};
use crate::{ConnectionMut, DatabasePool, Error, ListenError};

/// The most of an identifier PostgreSQL keeps: `NAMEDATALEN - 1` bytes. A
/// longer channel name given to `LISTEN` is truncated with only a notice, and
/// `pg_notify` refuses one outright, so a listener on it could never be sent
/// to by name.
const CHANNEL_BYTES: usize = 63;

/// A dedicated connection's `LISTEN` set: the half of a listener that chooses
/// the channels whose notifications arrive on its [`Notifications`].
///
/// A notification is delivered to the session that listened, outside any
/// statement's result, so a pooled connection — driven by a task that
/// discards such messages, and lent to another caller once returned — cannot
/// receive one. A listener owns its connection outright instead: it is opened
/// by [`connect_listener`] or [`DatabasePool::listener`], never recycled, and
/// closed when its [`Notifications`] is dropped.
///
/// Cloning is cheap and every clone changes the same `LISTEN` set, so channels
/// can be added from another task while one reads the notifications. Once the
/// connection has closed, every method fails.
///
/// A listener runs nothing but `LISTEN` and `UNLISTEN`, and is not a
/// [`Connection`](crate::Connection): a statement on it would wait behind the
/// notifications it shares the connection with, and a transaction on it would
/// hold back their delivery until it ended. [`notify`] sends from any
/// connection or transaction that may write instead.
// [spec:pgorm:req:conn.listen+4]
#[derive(Clone)]
pub struct Listener {
    client: Arc<Client>,
}

/// The notifications a [`Listener`]'s connection receives, in the order the
/// server sent them: by commit order across transactions, and in sending
/// order within one, a payload sent twice on one channel by the same
/// transaction arriving once.
///
/// The stream never ends while its connection is open, and holds the
/// connection open while it lives; dropping it closes the connection, after
/// which the listener's methods fail. It ends after an error, with nothing
/// reconnected — notifications sent while no session was listening are not
/// delivered to the next one, so a silent reconnection would hide a gap a
/// caller has to know about:
///
/// - when the connection fails or is terminated, every notification already
///   received is yielded, then the [`Error::Postgres`] it failed with;
/// - when `capacity` notifications are waiting unread and another arrives,
///   the connection is closed at once, and the waiting notifications are
///   yielded, then [`ListenError::Overflow`].
///
/// Closing rather than waiting for the reader is deliberate. A listener that
/// stopped reading its socket would hold back its own `LISTEN` and `UNLISTEN`
/// replies behind the unread notifications, and its session would stop
/// consuming PostgreSQL's notification queue, which every database in the
/// cluster shares; once that fills, every `NOTIFY` fails at commit.
// [spec:pgorm:req:conn.listen+4]
pub struct Notifications {
    shared: Arc<Shared>,
    /// Keeps the connection open while the stream lives: the connection
    /// closes once every handle on its client is dropped.
    client: Arc<Client>,
    driver: JoinHandle<()>,
}

/// What the driver task and the stream share.
struct Shared {
    queue: Mutex<Queue>,
}

struct Queue {
    waiting: VecDeque<Notification>,
    end: End,
    reader: Option<Waker>,
}

enum End {
    Open,
    Failed(Error),
    Ended,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queue `notification`, or report that `capacity` are already waiting.
    fn push(&self, notification: Notification, capacity: NonZeroUsize) -> bool {
        let mut queue = self.lock();
        if queue.waiting.len() >= capacity.get() {
            return false;
        }
        queue.waiting.push_back(notification);
        let reader = queue.reader.take();
        drop(queue);
        if let Some(reader) = reader {
            reader.wake();
        }
        true
    }

    fn end(&self, end: End) {
        let mut queue = self.lock();
        queue.end = end;
        let reader = queue.reader.take();
        drop(queue);
        if let Some(reader) = reader {
            reader.wake();
        }
    }
}

/// Drive the connection until it fails or overflows, queueing every
/// notification it receives. A notice is logged, as tokio-postgres's own
/// connection task does.
async fn drive(
    mut connection: Box<dyn RawConnection>,
    shared: Arc<Shared>,
    capacity: NonZeroUsize,
) {
    let end = std::future::poll_fn(|cx| {
        loop {
            match connection.poll_message(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(AsyncMessage::Notification(notification)))) => {
                    if !shared.push(notification, capacity) {
                        let overflow = ListenError::Overflow {
                            capacity: capacity.get(),
                        };
                        return Poll::Ready(End::Failed(overflow.into()));
                    }
                }
                Poll::Ready(Some(Ok(AsyncMessage::Notice(notice)))) => {
                    tracing::info!(
                        target: "pgorm::listen",
                        "{}: {}",
                        notice.severity(),
                        notice.message()
                    );
                }
                Poll::Ready(Some(Ok(_))) => {}
                Poll::Ready(Some(Err(error))) => return Poll::Ready(End::Failed(error.into())),
                Poll::Ready(None) => return Poll::Ready(End::Ended),
            }
        }
    })
    .await;
    // The socket closes before the end is reported, so a caller who has seen
    // the error can rely on the server having released the session.
    drop(connection);
    shared.end(end);
}

fn listening(
    client: Client,
    connection: Box<dyn RawConnection>,
    capacity: NonZeroUsize,
) -> (Listener, Notifications) {
    let shared = Arc::new(Shared {
        queue: Mutex::new(Queue {
            waiting: VecDeque::new(),
            end: End::Open,
            reader: None,
        }),
    });
    let driver = tokio::spawn(drive(connection, shared.clone(), capacity));
    let client = Arc::new(client);
    (
        Listener {
            client: client.clone(),
        },
        Notifications {
            shared,
            client,
            driver,
        },
    )
}

/// Open a dedicated listening connection with `config` and the TLS connector
/// `tls`, which [`connect_with`](crate::connect_with) takes too.
///
/// `capacity` bounds the notifications held unread; see [`Notifications`] for
/// what happens past it. [`Listener::DEFAULT_CAPACITY`] is a reasonable
/// choice.
///
/// # Errors
///
/// Returns [`Error::Postgres`] if the connection cannot be opened.
// [spec:pgorm:req:conn.listen+4]
pub async fn connect_listener<T>(
    config: Config,
    tls: T,
    capacity: NonZeroUsize,
) -> Result<(Listener, Notifications), Error>
where
    T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
    T::Stream: Sync + Send,
    T::TlsConnect: Sync + Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    let (client, connection) = ConfigConnectImpl { tls }.connect_raw(&config).await?;
    Ok(listening(client, connection, capacity))
}

impl DatabasePool {
    /// Open a dedicated PostgreSQL listening connection with this pool's
    /// configuration and TLS connector.
    ///
    /// The connection is the listener's alone: it is not taken from the pool,
    /// does not count against its size, and is never returned to it.
    /// `capacity` bounds the notifications held unread; see [`Notifications`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Postgres`] if the connection cannot be opened or this
    /// is a native Turso pool, which does not support notification connections.
    /// Native refusal opens no additional client.
    // [spec:pgorm:req:conn.listen+4]
    pub async fn listener(
        &self,
        capacity: NonZeroUsize,
    ) -> Result<(Listener, Notifications), Error> {
        let (client, connection) = self.0.manager().connect_raw().await?;
        Ok(listening(client, connection, capacity))
    }
}

/// `channel` as the quoted identifier `LISTEN` and `UNLISTEN` take: its case
/// and punctuation kept, every `"` doubled.
fn identifier(channel: &str) -> Result<String, Error> {
    if channel.len() > CHANNEL_BYTES {
        return Err(ListenError::ChannelTooLong {
            channel: channel.to_owned(),
        }
        .into());
    }
    Ok(format!("\"{}\"", channel.replace('"', "\"\"")))
}

impl Listener {
    /// A capacity for [`connect_listener`] and [`DatabasePool::listener`]
    /// that a reader keeping up with its notifications never reaches.
    pub const DEFAULT_CAPACITY: NonZeroUsize = match NonZeroUsize::new(1024) {
        Some(capacity) => capacity,
        None => panic!("the default capacity is not zero"),
    };

    /// Start receiving the notifications sent on `channel`.
    ///
    /// The name is exact: it is sent as a quoted identifier, so `Jobs` and
    /// `jobs` are two channels, and it matches what [`notify`] sends to. Every
    /// notification on it committed after this returns is delivered.
    /// Listening on a channel already listened on does nothing.
    ///
    /// # Errors
    ///
    /// Returns [`ListenError::ChannelTooLong`] for a name longer than the 63
    /// bytes PostgreSQL keeps of an identifier, before anything is sent: the
    /// server would listen on the truncated name instead. Returns
    /// [`Error::Postgres`] if the server refuses the name, or the connection
    /// has closed.
    // [spec:pgorm:req:conn.listen+4]
    pub async fn listen(&self, channel: &str) -> Result<(), Error> {
        let sql = format!("LISTEN {}", identifier(channel)?);
        Ok(self.client.batch_execute(&sql).await?)
    }

    /// Stop receiving the notifications sent on `channel`.
    ///
    /// None arrives from a transaction that commits after this returns, but
    /// one already received is still yielded by the [`Notifications`].
    /// Unlistening a channel not listened on does nothing.
    ///
    /// # Errors
    ///
    /// As [`listen`](Listener::listen).
    // [spec:pgorm:req:conn.listen+4]
    pub async fn unlisten(&self, channel: &str) -> Result<(), Error> {
        let sql = format!("UNLISTEN {}", identifier(channel)?);
        Ok(self.client.batch_execute(&sql).await?)
    }

    /// Stop receiving notifications on every channel.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Postgres`] if the connection has closed.
    // [spec:pgorm:req:conn.listen+4]
    pub async fn unlisten_all(&self) -> Result<(), Error> {
        Ok(self.client.batch_execute("UNLISTEN *").await?)
    }
}

impl fmt::Debug for Listener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Listener")
            .field("closed", &self.client.is_closed())
            .finish()
    }
}

impl Stream for Notifications {
    type Item = Result<Notification, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut queue = self.shared.lock();
        if let Some(notification) = queue.waiting.pop_front() {
            return Poll::Ready(Some(Ok(notification)));
        }
        match mem::replace(&mut queue.end, End::Ended) {
            End::Open => {
                queue.end = End::Open;
                queue.reader = Some(cx.waker().clone());
                Poll::Pending
            }
            End::Failed(error) => Poll::Ready(Some(Err(error))),
            End::Ended => Poll::Ready(None),
        }
    }
}

impl Drop for Notifications {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

impl fmt::Debug for Notifications {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let waiting = self.shared.lock().waiting.len();
        f.debug_struct("Notifications")
            .field("waiting", &waiting)
            .field("closed", &self.client.is_closed())
            .finish()
    }
}

/// Send `payload` on `channel` from `db`, through `SELECT pg_notify($1, $2)`.
///
/// Both values are bound parameters, so neither is ever quoted into SQL, and
/// the statement is one text the connection's statement cache prepares once.
/// `NOTIFY` itself cannot take a parameter: its payload would have to be
/// escaped into the statement, making each payload a statement of its own.
/// The channel is exact, as [`Listener::listen`] names it.
///
/// A notification is delivered when the transaction that sent it commits —
/// at once outside a transaction, never if it rolls back — and goes to every
/// session listening on the channel, a listener on `db`'s own session
/// included. Sending one acts on other sessions, so `db` is a
/// [`ConnectionMut`]: a connection that may only read sends none.
///
/// # Errors
///
/// Returns [`Error::Postgres`] for what the server refuses: an empty channel
/// name or one longer than 63 bytes, and a payload of 8000 bytes or more
/// (SQLSTATE `22023`).
// [spec:pgorm:req:conn.listen+4]
// [spec:pgorm:req:read-only.writes]
pub async fn notify<C>(db: &C, channel: &str, payload: &str) -> Result<(), Error>
where
    C: ConnectionMut,
{
    db.execute("SELECT pg_notify($1, $2)", &[&channel, &payload])
        .await?;
    Ok(())
}
