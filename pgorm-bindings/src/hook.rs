//! A pool hook whose body is a binding's callback.
//!
//! deadpool runs a hook on the runtime with the connection it is about,
//! borrowed for the hook's length. A binding's callback runs in its own
//! language meanwhile, on that language's thread, and runs its statements on
//! the connection by sending them here: the hook serves each on the borrowed
//! connection, through pgorm's [`HookConnection`], until the callback
//! finishes. Nothing the callback holds can reach the connection after that,
//! because the hook's end closes the channel its statements travel through.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use pgorm::{Hook, HookConnection, HookError};
use tokio::sync::mpsc;

/// A statement a binding's callback runs on its hook's connection: called
/// once, with the connection, on the runtime.
pub type HookJob = Box<
    dyn for<'a> FnOnce(&'a HookConnection<'a>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>
        + Send,
>;

/// Which of a pool's three hooks is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    /// A connection has just opened, before the pool lends it.
    PostCreate,
    /// A returned connection is about to be checked for reuse.
    PreRecycle,
    /// A returned connection has been checked, before the pool lends it again.
    PostRecycle,
}

/// Where a callback's statements go while its hook runs. The channel closes
/// as the hook returns, which `is_closed` on it reports.
#[derive(Debug, Clone)]
pub struct HookJobs(pub mpsc::UnboundedSender<HookJob>);

impl HookJobs {
    /// Send `run` to the hook's connection, where it is called once; `false`
    /// once the hook has returned and the connection is the pool's again,
    /// `run` dropped unrun. Taking the closure itself, rather than a boxed
    /// [`HookJob`], has its higher-ranked signature inferred at the call.
    pub fn submit<F>(&self, run: F) -> bool
    where
        F: for<'a> FnOnce(&'a HookConnection<'a>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>
            + Send
            + 'static,
    {
        let job: HookJob = Box::new(run);
        self.0.send(job).is_ok()
    }
}

/// What a callback is given about the connection its hook runs on.
#[derive(Debug)]
pub struct HookCall {
    /// The hook running, which names it in what a refusal says.
    pub event: HookEvent,
    /// Where its statements go.
    pub jobs: HookJobs,
    /// How long ago the connection opened.
    pub age: Duration,
    /// How many times the pool has recycled it.
    pub recycle_count: usize,
}

/// A hook running `callback`, which the binding starts in its language with
/// what [`HookCall`] says and resolves once that finishes. A callback that
/// fails, with what it says as the message, refuses the connection: deadpool
/// fails a `get` whose new connection a `post_create` hook refuses, and
/// discards a connection a recycle hook refuses.
// [spec:pgorm:req:bindings.pool-hooks]
pub fn hook<F, Fut>(event: HookEvent, callback: F) -> Hook
where
    F: Fn(HookCall) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), String>> + Send + 'static,
{
    let callback = Arc::new(callback);
    Hook::async_fn(move |client, metrics| {
        let (sender, mut jobs) = mpsc::unbounded_channel::<HookJob>();
        let done = callback(HookCall {
            event,
            jobs: HookJobs(sender),
            age: metrics.age(),
            recycle_count: metrics.recycle_count,
        });
        Box::pin(async move {
            let connection = HookConnection::new(client);
            tokio::pin!(done);
            loop {
                tokio::select! {
                    biased;
                    outcome = &mut done => return outcome.map_err(HookError::message),
                    Some(job) = jobs.recv() => job(&connection).await,
                }
            }
        })
    })
}
