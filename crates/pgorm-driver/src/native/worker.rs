#[cfg(test)]
use super::rows;
use super::{commands::Command, statement_store::Store, transaction};
use crate::Error;
use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
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
    owner: Weak<Control>,
    pending: AtomicBool,
    started: AtomicBool,
    close_on_cancel: AtomicBool,
    worker_done: AtomicBool,
    acknowledged: AtomicBool,
    restoring: AtomicBool,
}
impl Operation {
    pub(super) fn transaction_boundary(&self, control: &Control) {
        // The symmetric SeqCst check in cancel closes the race between
        // cancellation and classifying an imminent native boundary step.
        self.close_on_cancel.store(true, Ordering::SeqCst);
        if self.cancelled.load(Ordering::SeqCst) {
            control.shutdown();
        }
    }

    fn cancel(&self) -> bool {
        self.cancelled.store(true, Ordering::SeqCst);
        self.cancelled_notify.notify_one();
        self.close_on_cancel.load(Ordering::SeqCst)
    }

    fn release(&self) {
        if self.pending.swap(false, Ordering::AcqRel) {
            if let Some(control) = self.owner.upgrade() {
                control.inflight.fetch_sub(1, Ordering::Release);
            }
        }
    }

    pub(super) fn begin_restore(&self) {
        if !self.restoring.swap(true, Ordering::AcqRel) {
            if let Some(control) = self.owner.upgrade() {
                control.pending_restore.fetch_add(1, Ordering::AcqRel);
            }
        }
    }

    fn finish_restore(&self) {
        if self.restoring.swap(false, Ordering::AcqRel) {
            if let Some(control) = self.owner.upgrade() {
                control.pending_restore.fetch_sub(1, Ordering::Release);
            }
        }
    }

    fn worker_complete(&self) {
        self.finish_restore();
        self.worker_done.store(true, Ordering::SeqCst);
        if !self.close_on_cancel.load(Ordering::SeqCst) || self.acknowledged.load(Ordering::SeqCst)
        {
            self.release();
        }
    }

    pub(super) fn acknowledge(&self) {
        self.acknowledged.store(true, Ordering::SeqCst);
        if self.worker_done.load(Ordering::SeqCst) {
            self.release();
        }
    }

    #[cfg(test)]
    fn detached(id: u64) -> Self {
        Self {
            id,
            cancelled: AtomicBool::new(false),
            cancelled_notify: Notify::new(),
            owner: Weak::new(),
            pending: AtomicBool::new(false),
            started: AtomicBool::new(false),
            close_on_cancel: AtomicBool::new(false),
            worker_done: AtomicBool::new(false),
            acknowledged: AtomicBool::new(false),
            restoring: AtomicBool::new(false),
        }
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        // Unsubmitted work needs only release its token. Losing a started
        // operation without publishing completion is worker failure.
        if self.pending.load(Ordering::Acquire) && self.started.load(Ordering::Acquire) {
            if let Some(control) = self.owner.upgrade() {
                control.quarantine();
            }
        }
        self.release();
        self.finish_restore();
    }
}
struct Active {
    operation: Arc<Operation>,
    connection: PgConnection,
}
pub(super) struct Control {
    stopping: Arc<AtomicBool>,
    pub(super) closed: AtomicBool,
    pub(super) shutdown: watch::Sender<bool>,
    active: Mutex<Option<Active>>,
    signal: Arc<Signal>,
    next_id: AtomicU64,
    inflight: AtomicUsize,
    pub(super) pending_restore: AtomicUsize,
    idle: AtomicBool,
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
    pub(super) fn new_operation(self: &Arc<Self>) -> Arc<Operation> {
        self.inflight.fetch_add(1, Ordering::AcqRel);
        Arc::new(Operation {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            cancelled: AtomicBool::new(false),
            cancelled_notify: Notify::new(),
            owner: Arc::downgrade(self),
            pending: AtomicBool::new(true),
            started: AtomicBool::new(false),
            close_on_cancel: AtomicBool::new(false),
            worker_done: AtomicBool::new(false),
            acknowledged: AtomicBool::new(false),
            restoring: AtomicBool::new(false),
        })
    }

    pub(super) fn cancel(&self, operation: Option<&Operation>) {
        // Latch cancellation even if this command is queued or the native VM
        // has not yet marked its first root statement active.
        let mut close = operation.is_some_and(Operation::cancel);
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = active.as_ref() {
            if operation.is_none_or(|op| op.id == active.operation.id) {
                close |= active.operation.cancel();
                active.connection.inner().interrupt();
            }
        }
        drop(active);
        if close {
            self.request_shutdown();
        }
        self.signal.notify();
    }

    pub(super) fn quarantine(&self) {
        self.closed.store(true, Ordering::Release);
    }

    fn request_shutdown(&self) {
        self.quarantine();
        self.stopping.store(true, Ordering::Release);
        self.shutdown.send_replace(true);
    }

    pub(super) fn shutdown(&self) {
        self.request_shutdown();
        self.cancel(None);
    }

    pub(super) fn cancelled(&self, operation: &Operation) -> bool {
        self.stopping.load(Ordering::Acquire) || operation.cancelled.load(Ordering::Acquire)
    }

    // [spec:pgorm:sem:turso.cleanup] publish known native state before completion can wake a borrower
    pub(super) fn complete(
        &self,
        connection: &PgConnection,
        operation: &Operation,
        store: &mut Store,
    ) {
        store.publish_invalidations(connection);
        connection.inner().set_progress_handler(0, None);
        self.idle.store(
            connection.transaction_state() == turso_pg::TransactionState::Idle,
            Ordering::Release,
        );
        *self.active.lock().unwrap_or_else(|e| e.into_inner()) = None;
        operation.worker_complete();
    }

    pub(super) fn is_clean(&self) -> bool {
        self.inflight.load(Ordering::Acquire) == 0
            && self.idle.load(Ordering::Acquire)
            && !self.closed.load(Ordering::Acquire)
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
    pub(super) operation: Arc<Operation>,
    pub(super) control: Arc<Control>,
    pub(super) complete: bool,
}
impl RequestGuard {
    pub(super) fn complete(&mut self) {
        self.operation.acknowledge();
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
    pub(super) sender: mpsc::Sender<Command>,
    pub(super) close_sender: mpsc::UnboundedSender<u64>,
    pub(super) control: Arc<Control>,
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
    let (shutdown, shutdown_receiver) = watch::channel(false);
    let control = Arc::new(Control {
        stopping: Arc::new(AtomicBool::new(false)),
        closed: AtomicBool::new(false),
        shutdown,
        active: Mutex::new(None),
        signal: Arc::new(Signal::default()),
        next_id: AtomicU64::new(1),
        inflight: AtomicUsize::new(0),
        pending_restore: AtomicUsize::new(0),
        idle: AtomicBool::new(false),
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
    control.idle.store(
        connection.transaction_state() == turso_pg::TransactionState::Idle,
        Ordering::Release,
    );
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
        operation.started.store(true, Ordering::Release);
        *control.active.lock().unwrap_or_else(|e| e.into_inner()) = Some(Active {
            operation: Arc::clone(&operation),
            connection: connection.clone(),
        });
        let fatal = super::dispatch::run(
            command,
            super::dispatch::Context {
                connection: &connection,
                database: &database,
                store: &mut store,
                operation: &operation,
                control: &control,
                shutdown: &mut shutdown,
                runtime: &runtime,
            },
        );
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

pub(super) fn synchronize_preparation(
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
    let result = match (outcome, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(Failure::recoverable(error)),
        (Ok(()), Err(error)) => Err(Failure {
            error: Error::turso_cleanup_failed(error),
            fatal: true,
        }),
        (Err(primary), Err(cleanup)) => Err(Failure {
            error: Error::turso_cleanup(primary, cleanup),
            fatal: true,
        }),
    };
    match result {
        Err(failure)
            if !failure.error.is_turso_cleanup_failure()
                && operation.close_on_cancel.load(Ordering::Acquire)
                && transaction::is_boundary(statement)
                && connection.transaction_state() != turso_pg::TransactionState::Idle =>
        {
            Err(Failure {
                error: Error::turso_cleanup(
                    failure.error,
                    Error::turso_worker(std::io::Error::other(
                        "native transaction boundary failed without establishing an ended transaction",
                    )),
                ),
                fatal: true,
            })
        }
        result => result,
    }
}

// [spec:pgorm:sem:turso.lifecycle] one worker drives the complete native operation
// [spec:pgorm:sem:turso.execution] native script parsing stops on the first failure without claiming implicit atomicity
// [spec:pgorm:req:turso.capabilities] native COPY classification is refused before its first step
pub(super) fn execute_script(
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
        if statement.result_kind() == turso_pg::StatementResultKind::CopyOut {
            return Err(Failure::recoverable(Error::turso_unsupported(
                "COPY TO STDOUT",
            )));
        }
        if statement.parameters_count() != 0 {
            return Err(Failure::recoverable(Error::parameters(
                0,
                statement.parameters_count(),
            )));
        }
        if transaction::is_boundary(&statement) {
            operation.transaction_boundary(control);
        }
        let outcome = step_statement(&mut statement, operation, control);
        finish_statement(connection, &mut statement, operation, control, outcome)?;
        install_progress(connection, operation, control);
    }
    Ok(())
}

pub(super) fn step_statement(
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
    let state = statement.execution_state();
    if interrupted && (state.is_running() || state.is_terminal()) {
        drain_interrupted(statement, control)?;
    }
    // A frontend aborted-transaction gate can refuse before the core VM
    // starts. Do not turn Init into Interrupting and then ask that same gate
    // to drain it. Frontend cleanup and checked core reset still run below.
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
#[cfg(test)]
mod transaction_tests;
