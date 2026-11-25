mod postgres;

use crate::codec::{BackendMessage, PostgresCodec};
use crate::maybe_tls_stream::MaybeTlsStream;
use crate::{AsyncMessage, Error};
use futures_channel::mpsc;
use postgres::PostgresConnection;
pub use postgres::{Request, RequestMessages};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

#[cfg(not(feature = "turso"))]
type Backend<S, T> = PostgresConnection<S, T>;

// Keep the established PostgreSQL connection inline: native support must not
// introduce another allocation into every PostgreSQL connection.
#[cfg(feature = "turso")]
#[allow(clippy::large_enum_variant)]
enum Backend<S, T> {
    Postgres(PostgresConnection<S, T>),
    Turso(crate::TursoConnection),
}

/// Drives the lifetime of a PostgreSQL or embedded database connection.
///
/// Spawn this future after connecting. It resolves when the connection closes
/// or fails. Dropping a native connection requests worker shutdown even before
/// its first poll. PostgreSQL continues to use its ordinary wire IO state.
#[must_use = "futures do nothing unless polled"]
// [spec:pgorm:req:turso.pool+1] both configured backends return this same lifecycle type
pub struct Connection<S, T> {
    inner: Backend<S, T>,
}

impl<S, T> Connection<S, T>
where
    S: AsyncRead + AsyncWrite + Unpin,
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub(crate) fn new(
        stream: Framed<MaybeTlsStream<S, T>, PostgresCodec>,
        pending_responses: VecDeque<BackendMessage>,
        parameters: HashMap<String, String>,
        receiver: mpsc::UnboundedReceiver<Request>,
    ) -> Self {
        let inner = PostgresConnection::new(stream, pending_responses, parameters, receiver);
        #[cfg(feature = "turso")]
        let inner = Backend::Postgres(inner);
        Self { inner }
    }

    #[cfg(feature = "turso")]
    pub(crate) fn from_turso(connection: crate::TursoConnection) -> Self {
        Self {
            inner: Backend::Turso(connection),
        }
    }

    /// A PostgreSQL runtime parameter, or `None` for native connections.
    pub fn parameter(&self, name: &str) -> Option<&str> {
        #[cfg(feature = "turso")]
        match &self.inner {
            Backend::Postgres(connection) => connection.parameter(name),
            Backend::Turso(_) => None,
        }
        #[cfg(not(feature = "turso"))]
        self.inner.parameter(name)
    }

    /// Polls PostgreSQL notices and notifications instead of driving its future.
    ///
    /// `None` or an error is terminal; do not call this method again afterward.
    /// Native connections refuse this wire-only operation without closing the
    /// client. Their lifecycle must be driven by the `Future` implementation;
    /// dropping this connection still requests native shutdown.
    pub fn poll_message(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<AsyncMessage, Error>>> {
        #[cfg(feature = "turso")]
        match &mut self.inner {
            Backend::Postgres(connection) => connection.poll_message(cx),
            Backend::Turso(_) => Poll::Ready(Some(Err(Error::turso_unsupported(
                "native asynchronous messages",
            )))),
        }
        #[cfg(not(feature = "turso"))]
        self.inner.poll_message(cx)
    }
}

impl<S, T> Future for Connection<S, T>
where
    S: AsyncRead + AsyncWrite + Unpin,
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<(), Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(feature = "turso")]
        match &mut self.get_mut().inner {
            Backend::Postgres(connection) => Pin::new(connection).poll(cx),
            Backend::Turso(connection) => Pin::new(connection).poll(cx),
        }
        #[cfg(not(feature = "turso"))]
        Pin::new(&mut self.get_mut().inner).poll(cx)
    }
}
