//! Draining a row stream the way the campaign's `effects.stream` drains one.
//!
//! A streamed step is not "the rows, eventually": it is a bounded prefix plus
//! what the stream's own lifecycle then reports. `comparison.streamed` checks
//! both — the prefix against the reference multiset, and the terminal
//! `(complete, cancelled, closed)` triple against what the declared `take` and
//! `cancel` permit. So the drain has to be as deliberate as the binding's.

use futures_util::StreamExt;
use pgorm_driver::{Row, RowStream};

use crate::Error;

/// A stream's bounded prefix and the state it was left in.
#[derive(Debug)]
pub struct Drained {
    /// The rows actually observed, at most `take` of them.
    pub rows: Vec<Row>,
    /// Whether the stream ran out before the prefix was filled, or was seen to
    /// run out immediately after.
    pub complete: bool,
    /// Whether a pending next row was abandoned rather than observed.
    pub cancelled: bool,
    /// Whether the stream was released. Always true: it is dropped here.
    pub closed: bool,
}

/// Take `take` rows, then release the stream.
///
/// When `cancel` is set and the prefix filled without the stream ending, one
/// further item is awaited and discarded. That is what distinguishes "stopped
/// early with more available" from "stopped exactly at the end" — the two cases
/// `comparison.streamed` holds to different terminal states, and the only way
/// to tell them apart is to ask the stream for one more item.
///
/// `cancelled` is never reported: abandoning a *pending* poll is a race the
/// oracle permits but does not require, and a subject that claimed it without
/// racing anything would be recording something that did not happen.
///
/// # Errors
///
/// Returns [`Error::Database`] when the server fails mid-stream.
pub async fn drain(stream: RowStream, take: usize, cancel: bool) -> Result<Drained, Error> {
    let mut stream = Box::pin(stream);
    let mut rows = Vec::with_capacity(take);
    let mut complete = false;
    while rows.len() < take {
        match stream.next().await {
            Some(row) => rows.push(row?),
            None => {
                complete = true;
                break;
            }
        }
    }
    if cancel && !complete && stream.next().await.is_none() {
        complete = true;
    }
    drop(stream);
    Ok(Drained {
        rows,
        complete,
        cancelled: false,
        closed: true,
    })
}
