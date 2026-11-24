use super::{
    commands::Command,
    query, rows,
    statement_store::{Prepared, Store},
};
use crate::Error;
use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};
use tokio::{
    runtime::Handle,
    sync::{Notify, mpsc, oneshot, watch},
    task::JoinHandle,
};
use turso_core::types::{IOCompletions, IOResult};
use turso_pg::{PgConnection, Statement, StepResult};

const COMMAND_CAPACITY: usize = 32;
const PROGRESS_INTERVAL: u64 = 1024;

#[derive(Default)]
struct Signal {
    mutex: Mutex<()>,
    wake: Condvar,
}
impl Signal {
    fn notify(&self) {
        let _guard = self.mutex.lock().unwrap_or_else(|e| e.into_inner());
        self.wake.notify_all();
    }

    fn wait_while(&self, duration: Duration, mut pending: impl FnMut() -> bool) {
        let guard = self.mutex.lock().unwrap_or_else(|e| e.into_inner());
        drop(
            self.wake
                .wait_timeout_while(guard, duration, |_| pending())
                .unwrap_or_else(|e| e.into_inner()),
        );
    }
}
impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        self.notify();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.notify();
    }
}

pub(super) struct Operation {
    id: u64,
    cancelled: AtomicBool,
    pub(super) cancelled_notify: Notify,
}
struct Active {
    operation: Arc<Operation>,
    connection: PgConnection,
}
pub(super) struct Control {
    stopping: Arc<AtomicBool>,
    closed: AtomicBool,
    shutdown: watch::Sender<bool>,
    active: Mutex<Option<Active>>,
    signal: Arc<Signal>,
    next_id: AtomicU64,
    #[cfg(test)]
    before_step: Mutex<Option<StepGate>>,
    #[cfg(test)]
    after_row: Mutex<Option<StepGate>>,
    #[cfg(test)]
    fail_next_cleanup: AtomicBool,
    #[cfg(test)]
    panic_next_step: AtomicBool,
    #[cfg(test)]
    produced_rows: AtomicU64,
}
impl Control {
    fn cancel(&self, operation: Option<&Operation>) {
        // Latch cancellation even if this command is queued or the native VM
        // has not yet marked its first root statement active.
        if let Some(operation) = operation {
            operation.cancelled.store(true, Ordering::Release);
            operation.cancelled_notify.notify_one();
        }
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = active.as_ref() {
            if operation.is_none_or(|op| op.id == active.operation.id) {
                active.operation.cancelled.store(true, Ordering::Release);
                active.operation.cancelled_notify.notify_one();
                active.connection.inner().interrupt();
            }
        }
        drop(active);
        self.signal.notify();
    }

    pub(super) fn quarantine(&self) {
        self.closed.store(true, Ordering::Release);
    }

    fn shutdown(&self) {
        self.quarantine();
        self.stopping.store(true, Ordering::Release);
        self.shutdown.send_replace(true);
        self.cancel(None);
    }

    pub(super) fn cancelled(&self, operation: &Operation) -> bool {
        self.stopping.load(Ordering::Acquire) || operation.cancelled.load(Ordering::Acquire)
    }
}

struct ShutdownGuard(Arc<Control>);
impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}
struct WorkerGuard(Arc<Control>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.closed.store(true, Ordering::Release);
        *self.0.active.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.0.signal.notify();
    }
}

pub(super) struct RequestGuard {
    operation: Arc<Operation>,
    control: Arc<Control>,
    complete: bool,
}
impl RequestGuard {
    pub(super) fn complete(&mut self) {
        self.complete = true;
    }
    pub(super) fn worker_lost(&self) {
        self.control.shutdown();
    }
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if !self.complete {
            self.control.cancel(Some(&self.operation));
        }
    }
}

// [spec:pgorm:sem:turso.lifecycle]
pub(crate) struct Client {
    sender: mpsc::Sender<Command>,
    close_sender: mpsc::UnboundedSender<u64>,
    control: Arc<Control>,
}
impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeClient")
            .field("closed", &self.is_closed())
            .finish()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.control.shutdown();
    }
}
impl Client {
    fn request_guard(&self) -> Result<RequestGuard, Error> {
        if self.is_closed() {
            return Err(Error::closed());
        }
        Ok(RequestGuard {
            operation: Arc::new(Operation {
                id: self.control.next_id.fetch_add(1, Ordering::Relaxed),
                cancelled: AtomicBool::new(false),
                cancelled_notify: Notify::new(),
            }),
            control: Arc::clone(&self.control),
            complete: false,
        })
    }
    async fn submit(
        &self,
        command: Command,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<(), Error> {
        if *shutdown.borrow() {
            return Err(Error::closed());
        }
        tokio::select! {
            biased;
            _ = shutdown.changed() => Err(Error::closed()),
            result = self.sender.send(command) => result.map_err(|_| Error::closed()),
        }
    }
    async fn response<T>(
        &self,
        response: oneshot::Receiver<Result<T, Error>>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<T, Error> {
        tokio::select! {
            biased;
            _ = shutdown.changed() => Err(Error::closed()),
            result = response => match result {
                Ok(result) => result,
                Err(_) => { self.control.shutdown(); Err(Error::closed()) }
            },
        }
    }
    pub(crate) async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        let mut guard = self.request_guard()?;
        let (reply, response) = oneshot::channel();
        let mut shutdown = self.control.shutdown.subscribe();
        self.submit(
            Command::Batch {
                sql: sql.to_owned(),
                operation: Arc::clone(&guard.operation),
                reply,
            },
            &mut shutdown,
        )
        .await?;
        let outcome = self.response(response, &mut shutdown).await;
        guard.complete();
        outcome
    }
    pub(crate) async fn prepare(
        &self,
        sql: &str,
        types: &[crate::types::Type],
    ) -> Result<Prepared, Error> {
        let mut guard = self.request_guard()?;
        let (reply, response) = oneshot::channel();
        let mut shutdown = self.control.shutdown.subscribe();
        self.submit(
            Command::Prepare {
                sql: sql.to_owned(),
                types: types.to_vec(),
                operation: Arc::clone(&guard.operation),
                reply,
            },
            &mut shutdown,
        )
        .await?;
        let result = self.response(response, &mut shutdown).await;
        guard.complete();
        let (prepared, lease) = result?;
        lease.disarm();
        Ok(prepared)
    }
    pub(crate) fn close_statement(&self, id: u64) {
        let _ = self.close_sender.send(id);
    }
    pub(crate) async fn query_prepared(
        &self,
        statement: crate::Statement,
        params: Vec<turso_core::Value>,
    ) -> Result<rows::Rows, Error> {
        let id = statement.native_id(self)?;
        let guard = self.request_guard()?;
        let operation = Arc::clone(&guard.operation);
        let (rows, output) = rows::channel(guard);
        let (ready, response) = oneshot::channel();
        let mut shutdown = self.control.shutdown.subscribe();
        self.submit(
            Command::Query {
                id,
                statement,
                params,
                operation,
                ready,
                output,
            },
            &mut shutdown,
        )
        .await?;
        self.response(response, &mut shutdown).await?;
        Ok(rows)
    }
    pub(crate) async fn execute_prepared(
        &self,
        statement: crate::Statement,
        params: Vec<turso_core::Value>,
    ) -> Result<u64, Error> {
        let id = statement.native_id(self)?;
        let mut guard = self.request_guard()?;
        let (reply, response) = oneshot::channel();
        let mut shutdown = self.control.shutdown.subscribe();
        self.submit(
            Command::Execute {
                id,
                params,
                operation: Arc::clone(&guard.operation),
                reply,
                _statement: statement,
            },
            &mut shutdown,
        )
        .await?;
        let outcome = self.response(response, &mut shutdown).await;
        guard.complete();
        outcome
    }
    pub(crate) async fn check_connection(&self) -> Result<(), Error> {
        self.batch_execute("SELECT 1").await
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.control.closed.load(Ordering::Acquire) || self.sender.is_closed()
    }
    pub(crate) fn shutdown(&self) {
        self.control.shutdown();
    }
    #[cfg(any(feature = "runtime", test))]
    pub(crate) fn cancel(&self) {
        self.control.cancel(None);
    }
}

/// The driver's native connection lifetime. Dropping it always stops the
/// worker, including when the future has never been polled.
// [spec:pgorm:sem:turso.lifecycle]
pub(crate) struct Connection {
    worker: JoinHandle<Result<(), Error>>,
    _shutdown: ShutdownGuard,
}
impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeConnection").finish_non_exhaustive()
    }
}
impl Future for Connection {
    type Output = Result<(), Error>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.worker).poll(cx) {
            Poll::Ready(result) => Poll::Ready(result.map_err(Error::turso_worker)?),
            Poll::Pending => Poll::Pending,
        }
    }
}

// [spec:pgorm:sem:turso.lifecycle] guard exists before the startup wait can cancel
pub(crate) async fn connect(
    database: Arc<turso_core::Database>,
) -> Result<(Arc<Client>, Connection), Error> {
    let (sender, receiver) = mpsc::channel(COMMAND_CAPACITY);
    let (close_sender, close_receiver) = mpsc::unbounded_channel();
    let prepared_close = close_sender.clone();
    let (shutdown, shutdown_receiver) = watch::channel(false);
    let control = Arc::new(Control {
        stopping: Arc::new(AtomicBool::new(false)),
        closed: AtomicBool::new(false),
        shutdown,
        active: Mutex::new(None),
        signal: Arc::new(Signal::default()),
        next_id: AtomicU64::new(1),
        #[cfg(test)]
        before_step: Mutex::new(None),
        #[cfg(test)]
        after_row: Mutex::new(None),
        #[cfg(test)]
        fail_next_cleanup: AtomicBool::new(false),
        #[cfg(test)]
        panic_next_step: AtomicBool::new(false),
        #[cfg(test)]
        produced_rows: AtomicU64::new(0),
    });
    let client = Arc::new(Client {
        sender,
        close_sender,
        control: Arc::clone(&control),
    });
    let shutdown_guard = ShutdownGuard(Arc::clone(&control));
    let (ready_sender, ready) = oneshot::channel();
    let runtime = Handle::current();
    let supervisor_guard = ShutdownGuard(Arc::clone(&control));
    let worker = tokio::task::spawn_blocking(move || {
        run_worker(WorkerInput {
            database,
            commands: receiver,
            closing: close_receiver,
            close_sender: prepared_close,
            shutdown: shutdown_receiver,
            ready: ready_sender,
            control,
            runtime,
        })
    });
    // A caller may retain an unpolled Connection across runtime destruction.
    // The runtime must therefore own a guard too; aborting the blocking task
    // itself cannot stop a closure which has started running.
    let worker = tokio::spawn(async move {
        let _guard = supervisor_guard;
        worker.await.map_err(Error::turso_worker)?
    });
    let connection = Connection {
        worker,
        _shutdown: shutdown_guard,
    };
    ready.await.unwrap_or_else(|_| Err(Error::closed()))?;
    Ok((client, connection))
}

pub(super) fn cancelled() -> Error {
    Error::turso(turso_core::LimboError::Interrupt.into())
}

/// Cleanup failure quarantines a connection independently of error wording.
pub(super) struct Failure {
    pub(super) error: Error,
    pub(super) fatal: bool,
}
impl Failure {
    pub(super) fn recoverable(error: Error) -> Self {
        Self {
            error,
            fatal: false,
        }
    }
}

struct WorkerInput {
    database: Arc<turso_core::Database>,
    commands: mpsc::Receiver<Command>,
    closing: mpsc::UnboundedReceiver<u64>,
    close_sender: mpsc::UnboundedSender<u64>,
    shutdown: watch::Receiver<bool>,
    ready: oneshot::Sender<Result<(), Error>>,
    control: Arc<Control>,
    runtime: Handle,
}

fn run_worker(input: WorkerInput) -> Result<(), Error> {
    let WorkerInput {
        database,
        mut commands,
        mut closing,
        close_sender,
        mut shutdown,
        ready,
        control,
        runtime,
    } = input;
    let _guard = WorkerGuard(Arc::clone(&control));
    if control.stopping.load(Ordering::Acquire) {
        let _ = ready.send(Err(Error::closed()));
        return Ok(());
    }
    let connection = match PgConnection::connect(&database) {
        Ok(connection) => connection,
        Err(error) => {
            let _ = ready.send(Err(Error::turso(error)));
            return Ok(());
        }
    };
    if ready.send(Ok(())).is_err() {
        control.shutdown();
    }
    let mut store = Store::default();
    let mut close_failure = None;
    enum Event {
        Command(Command),
        Close(u64),
    }
    loop {
        if control.stopping.load(Ordering::Acquire) {
            break;
        }
        let event = runtime.block_on(async {
            tokio::select! {
                biased;
                _ = shutdown.changed() => None,
                id = closing.recv() => id.map(Event::Close),
                command = commands.recv() => command.map(Event::Command),
            }
        });
        let command = match event {
            None => break,
            Some(Event::Close(id)) => {
                if let Err(error) = store.close(id) {
                    control.quarantine();
                    close_failure = Some(error);
                    break;
                }
                continue;
            }
            Some(Event::Command(command)) => command,
        };
        let operation = Arc::clone(command.operation());
        *control.active.lock().unwrap_or_else(|e| e.into_inner()) = Some(Active {
            operation: Arc::clone(&operation),
            connection: connection.clone(),
        });
        let fatal = match command {
            Command::Batch { sql, reply, .. } => {
                let result = execute_script(&connection, &sql, &operation, &control);
                let fatal = result.as_ref().err().is_some_and(|error| error.fatal);
                if fatal {
                    control.quarantine();
                }
                let _ = reply.send(result.map_err(|failure| failure.error));
                fatal
            }
            Command::Prepare {
                sql, types, reply, ..
            } => {
                let result = if control.cancelled(&operation) {
                    Err(Failure::recoverable(cancelled()))
                } else {
                    synchronize_preparation(&connection, &database, &operation, &control).and_then(
                        |()| {
                            store
                                .prepare(&connection, &sql, types, &close_sender)
                                .map_err(Failure::recoverable)
                        },
                    )
                };
                let fatal = result.as_ref().err().is_some_and(|error| error.fatal);
                if fatal {
                    control.quarantine();
                }
                let _ = reply.send(result.map_err(|failure| failure.error));
                fatal
            }
            Command::Execute {
                id,
                params,
                reply,
                _statement: statement,
                ..
            } => {
                let result = match store.get(id, &connection) {
                    Ok(entry) => {
                        let result =
                            query::execute(&connection, entry, params, &operation, &control);
                        // A native failure remains the primary error even if
                        // step reprepared first; the public handle is still
                        // stale and must leave the cache before another use.
                        if !entry.metadata_is_current() {
                            statement.mark_invalidated();
                        }
                        result
                    }
                    Err(error) => Err(Failure::recoverable(error)),
                };
                let fatal = result.as_ref().err().is_some_and(|error| error.fatal);
                if fatal {
                    control.quarantine();
                }
                if result
                    .as_ref()
                    .err()
                    .is_some_and(|failure| failure.error.is_statement_invalidated())
                {
                    statement.mark_invalidated();
                }
                let _ = reply.send(result.map_err(|failure| failure.error));
                fatal
            }
            Command::Query {
                id,
                statement,
                params,
                ready,
                output,
                ..
            } => match store.get(id, &connection) {
                Ok(entry) => query::query(
                    entry,
                    query::Request {
                        statement,
                        params,
                        ready,
                        output,
                    },
                    query::Context {
                        connection: &connection,
                        operation: &operation,
                        control: &control,
                        shutdown: &mut shutdown,
                        runtime: &runtime,
                    },
                ),
                Err(error) => {
                    if error.is_statement_invalidated() {
                        statement.mark_invalidated();
                    }
                    let _ = ready.send(Err(error));
                    output.finish(Err(Error::closed()));
                    false
                }
            },
        };
        connection.inner().set_progress_handler(0, None);
        *control.active.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if fatal {
            control.quarantine();
            break;
        }
    }
    commands.close();
    while let Ok(command) = commands.try_recv() {
        command.close();
    }
    connection.inner().set_progress_handler(0, None);
    let statements = store.close_all();
    let connection = connection.close().map_err(Error::turso);
    for result in [statements, connection] {
        if let Err(error) = result {
            close_failure = Some(match close_failure {
                None => error,
                Some(primary) => Error::turso_cleanup(primary, error),
            });
        }
    }
    match close_failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn synchronize_preparation(
    connection: &PgConnection,
    database: &turso_core::Database,
    operation: &Arc<Operation>,
    control: &Control,
) -> Result<(), Failure> {
    // Pinned engine adf3b41's maybe_update_schema COW-clones the shared schema
    // while resolving MVCC roots, then its next identity check treats that
    // clone as another schema change. Native reprepare adopts the shared Arc
    // directly. One harmless read lets that path settle before describing new
    // user SQL. This is cold preparation only, never cached execution; leave
    // active transaction snapshots alone and do not retry concurrent DDL.
    if connection.inner().get_auto_commit()
        && !Arc::ptr_eq(
            &connection.inner().current_schema(),
            &database.clone_schema(),
        )
    {
        execute_script(connection, "SELECT 1", operation, control)?;
    }
    Ok(())
}

pub(super) fn install_progress(
    connection: &PgConnection,
    operation: &Arc<Operation>,
    control: &Control,
) {
    let cancellation = Arc::clone(operation);
    let stopping = Arc::clone(&control.stopping);
    connection.inner().set_progress_handler(
        PROGRESS_INTERVAL,
        Some(Box::new(move || {
            cancellation.cancelled.load(Ordering::Acquire) || stopping.load(Ordering::Acquire)
        })),
    );
}

pub(super) fn finish_statement(
    connection: &PgConnection,
    statement: &mut Statement,
    operation: &Operation,
    control: &Control,
    outcome: Result<(), Error>,
) -> Result<(), Failure> {
    connection.inner().set_progress_handler(0, None);
    let cleanup = cleanup_statement(statement, operation, control, outcome.is_err());
    if outcome.is_err() {
        connection.abort_transaction_block_on_error();
    }
    match (outcome, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(Failure::recoverable(error)),
        (Ok(()), Err(error)) => Err(Failure { error, fatal: true }),
        (Err(primary), Err(cleanup)) => Err(Failure {
            error: Error::turso_cleanup(primary, cleanup),
            fatal: true,
        }),
    }
}

// [spec:pgorm:sem:turso.lifecycle] one worker drives the complete native operation
// [spec:pgorm:sem:turso.execution] native script parsing stops on the first failure without claiming implicit atomicity
fn execute_script(
    connection: &PgConnection,
    sql: &str,
    operation: &Arc<Operation>,
    control: &Control,
) -> Result<(), Failure> {
    install_progress(connection, operation, control);
    // Native parsing supplies script boundaries. Native scripts are sequential:
    // this does not manufacture PostgreSQL's implicit script transaction.
    for statement in connection.query_runner(sql.as_bytes()) {
        if control.cancelled(operation) {
            connection.abort_transaction_block_on_error();
            return Err(Failure::recoverable(cancelled()));
        }
        let Some(mut statement) = statement.map_err(|e| Failure::recoverable(Error::turso(e)))?
        else {
            continue;
        };
        if statement.parameters_count() != 0 {
            return Err(Failure::recoverable(Error::parameters(
                0,
                statement.parameters_count(),
            )));
        }
        let outcome = step_statement(&mut statement, operation, control);
        finish_statement(connection, &mut statement, operation, control, outcome)?;
        install_progress(connection, operation, control);
    }
    Ok(())
}

fn step_statement(
    statement: &mut Statement,
    operation: &Operation,
    control: &Control,
) -> Result<(), Error> {
    while next_row(statement, operation, control)? {}
    Ok(())
}

pub(super) fn next_row(
    statement: &mut Statement,
    operation: &Operation,
    control: &Control,
) -> Result<bool, Error> {
    loop {
        if control.cancelled(operation) {
            return Err(cancelled());
        }
        #[cfg(test)]
        if let Some(gate) = control.before_step.lock().unwrap().take() {
            gate.pause();
        }
        #[cfg(test)]
        if control.panic_next_step.swap(false, Ordering::AcqRel) {
            panic!("injected native worker failure");
        }
        match statement.step().map_err(Error::turso)? {
            StepResult::Row => {
                #[cfg(test)]
                control.produced_rows.fetch_add(1, Ordering::Relaxed);
                #[cfg(test)]
                if let Some(gate) = control.after_row.lock().unwrap().take() {
                    gate.pause();
                }
                return Ok(true);
            }
            StepResult::Done => return Ok(false),
            StepResult::Interrupt => return Err(cancelled()),
            StepResult::Busy => return Err(Error::turso(turso_core::LimboError::Busy.into())),
            StepResult::Sleep { duration } => {
                control
                    .signal
                    .wait_while(duration, || !control.cancelled(operation));
            }
            StepResult::Yield => {
                if let Err(error) = statement._io().step() {
                    return Err(Error::turso(error.into()));
                }
                std::thread::yield_now();
            }
            StepResult::IO => {
                let io = statement.io_completions().ok_or_else(|| {
                    Error::turso_worker(std::io::Error::other(
                        "native statement yielded I/O without a completion",
                    ))
                })?;
                wait_completion(statement, io, operation, control)?;
            }
        }
    }
}

fn wait_completion(
    statement: &Statement,
    io: IOCompletions,
    operation: &Operation,
    control: &Control,
) -> Result<(), Error> {
    let waker = Waker::from(Arc::clone(&control.signal));
    io.set_waker(Some(&waker));
    while !io.finished() {
        if control.cancelled(operation) {
            io.abort();
            return Err(cancelled());
        }
        statement._io().step().map_err(|e| Error::turso(e.into()))?;
        // Synchronous file completions are already ready. Engine wait
        // completions wake Signal directly; the short timeout also permits
        // progress for backends whose step must be called again.
        control.signal.wait_while(Duration::from_millis(1), || {
            !io.finished() && !control.cancelled(operation)
        });
    }
    match io.get_error() {
        Some(error) => Err(Error::turso(turso_core::LimboError::from(error).into())),
        None => Ok(()),
    }
}

/// Reach a terminal native result before reset: resetting a still-running
/// write with RETURNING can commit it. Interruption alone only changes its
/// state to Interrupting, which core still considers running.
fn drain_interrupted(statement: &mut Statement, control: &Control) -> Result<(), Error> {
    if statement.execution_state().is_terminal() {
        return Ok(());
    }
    statement.interrupt();
    if let Some(io) = statement.io_completions() {
        if !io.finished() {
            io.abort();
        }
    }
    loop {
        match statement.step() {
            Ok(StepResult::Done | StepResult::Interrupt) => {
                if statement.execution_state().is_terminal() {
                    return Ok(());
                }
                return Err(Error::turso_worker(std::io::Error::other(
                    "native cancellation returned without a terminal VM state",
                )));
            }
            Err(error) => {
                if statement.execution_state().is_terminal() {
                    return Ok(());
                }
                // A frontend transaction gate can refuse a step before the
                // VM runs. Such an error is not proof of VM termination.
                return Err(Error::turso(error));
            }
            Ok(StepResult::Row) => {}
            Ok(StepResult::Busy) => {
                // A terminal outcome has not been established. The caller
                // quarantines this connection rather than assuming rollback.
                return Err(Error::turso_worker(std::io::Error::other(
                    "native statement remained busy while draining cancellation",
                )));
            }
            Ok(StepResult::Sleep { duration }) => {
                // Cancellation is latched; mandatory cleanup still requires
                // the engine's backoff to avoid a busy loop.
                control.signal.wait_while(duration, || true);
            }
            Ok(StepResult::Yield) => {
                statement._io().step().map_err(|e| Error::turso(e.into()))?;
                std::thread::yield_now();
            }
            Ok(StepResult::IO) => {
                let io = statement.io_completions().ok_or_else(|| {
                    Error::turso_worker(std::io::Error::other(
                        "native cancellation yielded I/O without a completion",
                    ))
                })?;
                // Only synchronous PlatformIO/MemoryIO are admitted. A
                // remaining wait completion is an engine wait, which must
                // not hold cancellation hostage to another connection.
                if !io.finished() {
                    io.abort();
                }
            }
        }
    }
}

fn cleanup_statement(
    statement: &mut Statement,
    operation: &Operation,
    control: &Control,
    interrupted: bool,
) -> Result<(), Error> {
    if interrupted {
        drain_interrupted(statement, control)?;
    }
    #[cfg(test)]
    if control.fail_next_cleanup.swap(false, Ordering::AcqRel) {
        return Err(Error::turso_worker(std::io::Error::other(
            "injected native cleanup failure",
        )));
    }
    // Drain the frontend state machine before its synchronous core reset. A
    // cancelled outstanding completion is explicitly aborted, never waited on
    // indefinitely with no way to observe shutdown.
    if control.cancelled(operation) {
        if let Some(io) = statement.io_completions() {
            if !io.finished() {
                io.abort();
            }
        }
    }
    loop {
        match statement.cleanup_nonblock().map_err(Error::turso)? {
            IOResult::Done(()) => break,
            IOResult::IO(io) => wait_completion(statement, io, operation, control)?,
        }
    }
    // Pinned core reset is synchronous. It is confined to this blocking worker;
    // no bound is promised for an uninterruptible kernel syscall. Any reset
    // failure quarantines this connection, preventing pool reuse.
    statement.reset().map_err(Error::turso)
}

#[cfg(test)]
struct StepGate {
    entered: oneshot::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}
#[cfg(test)]
impl StepGate {
    fn pause(self) {
        let _ = self.entered.send(());
        let _ = self.resume.recv();
    }
}

#[cfg(test)]
mod tests;
