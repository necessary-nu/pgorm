use super::worker::{Control, Operation, RequestGuard, cancelled};
use crate::{Error, Row};
use futures_util::Stream;
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch},
};

pub(super) const BATCH_ROWS: usize = 32;
pub(super) const BATCH_BYTES: usize = 64 * 1024;
const CHANNEL_BATCHES: usize = 1;

enum Terminal {
    Pending,
    Done(u64),
    Failed(Error),
    Consumed,
}

pub(crate) struct Rows {
    receiver: mpsc::Receiver<Vec<Row>>,
    pending: std::vec::IntoIter<Row>,
    terminal: Arc<Mutex<Terminal>>,
    guard: RequestGuard,
    affected: Option<u64>,
    exhausted: bool,
}

pub(super) struct Output {
    sender: mpsc::Sender<Vec<Row>>,
    terminal: Arc<Mutex<Terminal>>,
}

pub(super) fn channel(guard: RequestGuard) -> (Rows, Output) {
    let (sender, receiver) = mpsc::channel(CHANNEL_BATCHES);
    let terminal = Arc::new(Mutex::new(Terminal::Pending));
    (
        Rows {
            receiver,
            pending: Vec::new().into_iter(),
            terminal: Arc::clone(&terminal),
            guard,
            affected: None,
            exhausted: false,
        },
        Output { sender, terminal },
    )
}

// [spec:pgorm:req:turso.rows] rows own ordinary binary storage, independent of subsequent native steps
impl Stream for Rows {
    type Item = Result<Row, Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.exhausted {
            return Poll::Ready(None);
        }
        loop {
            if let Some(row) = this.pending.next() {
                return Poll::Ready(Some(Ok(row)));
            }
            match this.receiver.poll_recv(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(batch)) => this.pending = batch.into_iter(),
                Poll::Ready(None) => {
                    this.exhausted = true;
                    this.guard.complete();
                    return match std::mem::replace(
                        &mut *this.terminal.lock().unwrap_or_else(|e| e.into_inner()),
                        Terminal::Consumed,
                    ) {
                        Terminal::Done(affected) => {
                            this.affected = Some(affected);
                            Poll::Ready(None)
                        }
                        Terminal::Failed(error) => Poll::Ready(Some(Err(error))),
                        Terminal::Pending => {
                            this.guard.worker_lost();
                            Poll::Ready(Some(Err(Error::closed())))
                        }
                        Terminal::Consumed => Poll::Ready(None),
                    };
                }
            }
        }
    }
}
impl Rows {
    pub(crate) fn rows_affected(&self) -> Option<u64> {
        self.affected
    }
}
impl Drop for Rows {
    fn drop(&mut self) {
        self.receiver.close();
    }
}
impl Output {
    pub(super) fn send(
        &self,
        batch: Vec<Row>,
        operation: &Operation,
        control: &Control,
        shutdown: &mut watch::Receiver<bool>,
        runtime: &Handle,
    ) -> Result<(), Error> {
        if control.cancelled(operation) {
            return Err(cancelled());
        }
        runtime.block_on(async {
            tokio::select! {
                biased;
                _ = shutdown.changed() => Err(Error::closed()),
                _ = operation.cancelled_notify.notified() => Err(cancelled()),
                permit = self.sender.reserve() => {
                    let permit = permit.map_err(|_| cancelled())?;
                    permit.send(batch);
                    Ok(())
                }
            }
        })
    }

    pub(super) fn finish(self, outcome: Result<u64, Error>) {
        *self.terminal.lock().unwrap_or_else(|e| e.into_inner()) = match outcome {
            Ok(affected) => Terminal::Done(affected),
            Err(error) => Terminal::Failed(error),
        };
        // Dropping sender wakes the receiver only after terminal state is set.
    }
}
