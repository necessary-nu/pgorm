//! A connection opened outside the pool, whose I/O half its owner drives.

use std::task::{Context, Poll};

use pgorm_driver::{AsyncMessage, Client, Connection, Error};
use tokio::io::{AsyncRead, AsyncWrite};

/// A client with the undriven I/O half of its connection, as
/// [`Connect::connect_raw`](crate::Connect::connect_raw) opens them.
pub type Unpooled = (Client, Box<dyn RawConnection>);

/// The I/O half of a connection, polled by whoever owns it rather than by a
/// task the pool spawned.
///
/// The pool drives every connection it creates on a task of its own, and that
/// task discards the asynchronous messages the server sends outside any
/// request — the notices, and the notifications a `LISTEN` asks for. A
/// connection opened through [`Connect::connect_raw`](crate::Connect::connect_raw)
/// is handed back undriven instead, so its owner sees them. Nothing is sent or
/// received, not even the replies to its client's requests, except while this
/// is polled.
///
/// It is a trait object because a connection's type names its TLS stream, which
/// the pool's connector erases.
// [spec:pgorm:req:conn.listen+4]    the connection the listener drives itself
pub trait RawConnection: Send {
    /// Drive the connection, returning the next asynchronous message.
    ///
    /// This is `pgorm_driver::Connection::poll_message`: `None` once the
    /// client has been dropped and the connection has closed, an `Err` when it
    /// failed, and either one is the last answer it gives.
    fn poll_message(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<AsyncMessage, Error>>>;
}

impl<S, T> RawConnection for Connection<S, T>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    fn poll_message(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<AsyncMessage, Error>>> {
        Connection::poll_message(self, cx)
    }
}
