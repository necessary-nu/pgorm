//! The start of a `COPY … FROM STDIN`, which a refusal leaves the connection
//! unable to account for.

use bytes::Buf;
use pgorm_driver::{CopyInSink, Error};
use tokio::task::AbortHandle;

/// Start `COPY … FROM STDIN` through `start`, closing the connection behind
/// `connection` if it does not start.
///
/// tokio-postgres sends `Bind`, `Execute` and `Sync` together, and a `Sync`
/// again when the sink it returns is finished or dropped. Once the server is
/// in copy-in mode it ignores the first `Sync`, so a COPY that starts is
/// answered by one `ReadyForQuery`. A COPY the server refuses before entering
/// copy-in mode — an undefined table or column, an option it rejects, a
/// transaction already aborted — answers the first `Sync` with a
/// `ReadyForQuery` that ends the request, and then answers the second with
/// another that no request is waiting for. tokio-postgres hands that one to
/// the next request, which reads it as its own answer and leaves its own
/// answer to the request after it: every later response on the connection is
/// read by the wrong request. Nothing in its public API sends a message the
/// server does not answer, so the extra one cannot be absorbed; the connection
/// is closed instead, every later call on it fails as closed, and the pool's
/// recycling discards it. A cancelled start closes it too, since whether the
/// server refused is then unknown.
// [spec:pgorm:req:conn.copy+6]    a refused start closes the connection
pub(crate) async fn guarded<F, U>(
    connection: &AbortHandle,
    start: F,
) -> Result<CopyInSink<U>, Error>
where
    F: Future<Output = Result<CopyInSink<U>, Error>>,
    U: Buf + 'static + Send,
{
    let mut close = CloseUnlessStarted(Some(connection));
    let sink = start.await?;
    close.0 = None;
    Ok(sink)
}

/// Aborts the connection task on drop unless disarmed.
struct CloseUnlessStarted<'a>(Option<&'a AbortHandle>);

impl Drop for CloseUnlessStarted<'_> {
    fn drop(&mut self) {
        if let Some(connection) = self.0 {
            tracing::warn!("COPY FROM STDIN did not start; closing its connection");
            connection.abort();
        }
    }
}
