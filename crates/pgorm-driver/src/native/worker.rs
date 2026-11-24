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
    sync::{mpsc, oneshot, watch},
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

struct Operation {
    id: u64,
    cancelled: AtomicBool,
}
struct Active {
    operation: Arc<Operation>,
    connection: PgConnection,
}
struct Control {
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
}
impl Control {
    fn cancel(&self, operation: Option<&Operation>) {
        // Latch cancellation even if this command is queued or the native VM
        // has not yet marked its first root statement active.
        if let Some(operation) = operation {
            operation.cancelled.store(true, Ordering::Release);
        }
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = active.as_ref() {
            if operation.is_none_or(|op| op.id == active.operation.id) {
                active.operation.cancelled.store(true, Ordering::Release);
                active.connection.inner().interrupt();
            }
        }
        drop(active);
        self.signal.notify();
    }

    fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        self.stopping.store(true, Ordering::Release);
        self.shutdown.send_replace(true);
        self.cancel(None);
    }

    fn cancelled(&self, operation: &Operation) -> bool {
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

struct RequestGuard {
    operation: Arc<Operation>,
    control: Arc<Control>,
    complete: bool,
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if !self.complete {
            self.control.cancel(Some(&self.operation));
        }
    }
}

struct Command {
    sql: String,
    operation: Arc<Operation>,
    reply: oneshot::Sender<Result<(), Error>>,
}

// [spec:pgorm:sem:turso.lifecycle]
pub(crate) struct Client {
    sender: mpsc::Sender<Command>,
    control: Arc<Control>,
}
impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeClient")
            .field("closed", &self.is_closed())
            .finish()
    }
}
impl Client {
    pub(crate) async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        if self.is_closed() {
            return Err(Error::closed());
        }
        let operation = Arc::new(Operation {
            id: self.control.next_id.fetch_add(1, Ordering::Relaxed),
            cancelled: AtomicBool::new(false),
        });
        let mut guard = RequestGuard {
            operation: Arc::clone(&operation),
            control: Arc::clone(&self.control),
            complete: false,
        };
        let (reply, response) = oneshot::channel();
        let command = Command {
            sql: sql.to_owned(),
            operation,
            reply,
        };
        let mut shutdown = self.control.shutdown.subscribe();
        if *shutdown.borrow() {
            return Err(Error::closed());
        }
        tokio::select! {
            biased;
            _ = shutdown.changed() => return Err(Error::closed()),
            result = self.sender.send(command) => result.map_err(|_| Error::closed())?,
        }
        // A dropped sender is never successful completion: this also covers
        // worker panic and requests discarded during connection shutdown.
        let outcome = tokio::select! {
            biased;
            _ = shutdown.changed() => Err(Error::closed()),
            result = response => result.unwrap_or_else(|_| Err(Error::closed())),
        };
        guard.complete = true;
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
    });
    let client = Arc::new(Client {
        sender,
        control: Arc::clone(&control),
    });
    let shutdown_guard = ShutdownGuard(Arc::clone(&control));
    let (ready_sender, ready) = oneshot::channel();
    let runtime = Handle::current();
    let supervisor_guard = ShutdownGuard(Arc::clone(&control));
    let worker = tokio::task::spawn_blocking(move || {
        run_worker(
            database,
            receiver,
            shutdown_receiver,
            ready_sender,
            control,
            runtime,
        )
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

fn cancelled() -> Error {
    Error::turso(turso_core::LimboError::Interrupt.into())
}

/// Cleanup failure quarantines a connection independently of error wording.
struct Failure {
    error: Error,
    fatal: bool,
}
impl Failure {
    fn recoverable(error: Error) -> Self {
        Self {
            error,
            fatal: false,
        }
    }
}

fn run_worker(
    database: Arc<turso_core::Database>,
    mut commands: mpsc::Receiver<Command>,
    mut shutdown: watch::Receiver<bool>,
    ready: oneshot::Sender<Result<(), Error>>,
    control: Arc<Control>,
    runtime: Handle,
) -> Result<(), Error> {
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
    loop {
        if control.stopping.load(Ordering::Acquire) {
            break;
        }
        let command = runtime.block_on(async {
            tokio::select! {
                biased;
                _ = shutdown.changed() => None,
                command = commands.recv() => command,
            }
        });
        let Some(command) = command else {
            break;
        };
        *control.active.lock().unwrap_or_else(|e| e.into_inner()) = Some(Active {
            operation: Arc::clone(&command.operation),
            connection: connection.clone(),
        });
        let result = execute_script(&connection, &command.sql, &command.operation, &control);
        connection.inner().set_progress_handler(0, None);
        *control.active.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let fatal = result.as_ref().err().is_some_and(|error| error.fatal);
        if fatal {
            control.closed.store(true, Ordering::Release);
        }
        let _ = command.reply.send(result.map_err(|failure| failure.error));
        if fatal {
            break;
        }
    }
    commands.close();
    while let Ok(command) = commands.try_recv() {
        let _ = command.reply.send(Err(Error::closed()));
    }
    connection.inner().set_progress_handler(0, None);
    connection.close().map_err(Error::turso)
}

// [spec:pgorm:sem:turso.lifecycle] one worker drives the complete native operation
fn execute_script(
    connection: &PgConnection,
    sql: &str,
    operation: &Arc<Operation>,
    control: &Control,
) -> Result<(), Failure> {
    let cancellation = Arc::clone(operation);
    let stopping = Arc::clone(&control.stopping);
    // Connection::interrupt ignores idle roots. This latch is also checked by
    // the VM after startup, closing the cancel-before-first-step race.
    connection.inner().set_progress_handler(
        PROGRESS_INTERVAL,
        Some(Box::new(move || {
            cancellation.cancelled.load(Ordering::Acquire) || stopping.load(Ordering::Acquire)
        })),
    );
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
        let outcome = step_statement(connection, &mut statement, operation, control);
        // Cancellation must not interrupt mandatory rollback/reset.
        connection.inner().set_progress_handler(0, None);
        let cleanup = cleanup_statement(&mut statement, operation, control, outcome.is_err());
        if outcome.is_err() {
            connection.abort_transaction_block_on_error();
        }
        match (outcome, cleanup) {
            (Ok(()), Ok(())) => {}
            (Err(error), Ok(())) => return Err(Failure::recoverable(error)),
            (Ok(()), Err(error)) => return Err(Failure { error, fatal: true }),
            (Err(primary), Err(cleanup)) => {
                return Err(Failure {
                    error: Error::turso_cleanup(primary, cleanup),
                    fatal: true,
                });
            }
        }
        let cancellation = Arc::clone(operation);
        let stopping = Arc::clone(&control.stopping);
        connection.inner().set_progress_handler(
            PROGRESS_INTERVAL,
            Some(Box::new(move || {
                cancellation.cancelled.load(Ordering::Acquire) || stopping.load(Ordering::Acquire)
            })),
        );
    }
    Ok(())
}

fn step_statement(
    _connection: &PgConnection,
    statement: &mut Statement,
    operation: &Operation,
    control: &Control,
) -> Result<(), Error> {
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
            StepResult::Row =>
            {
                #[cfg(test)]
                if let Some(gate) = control.after_row.lock().unwrap().take() {
                    gate.pause();
                }
            }
            StepResult::Done => return Ok(()),
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
mod tests {
    use super::*;
    use std::path::Path;

    const LONG: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT sum(x) FROM n";

    async fn actor() -> (Arc<Client>, JoinHandle<Result<(), Error>>) {
        let database = super::super::Database::open(Path::new(":memory:"))
            .await
            .unwrap();
        let (client, connection) = database.connect().await.unwrap();
        (client, tokio::spawn(connection))
    }

    fn gate() -> (StepGate, oneshot::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered, ready) = oneshot::channel();
        let (resume, paused) = std::sync::mpsc::channel();
        (
            StepGate {
                entered,
                resume: paused,
            },
            ready,
            resume,
        )
    }

    async fn finish(client: Arc<Client>, task: JoinHandle<Result<(), Error>>) {
        drop(client);
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    // [spec:pgorm:sem:turso.lifecycle/test] cancellation between check and first VM step is latched
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancellation_before_first_native_step_is_not_lost() {
        let (client, task) = actor().await;
        let (gate, entered, resume) = gate();
        *client.control.before_step.lock().unwrap() = Some(gate);
        let op_client = Arc::clone(&client);
        let operation = tokio::spawn(async move { op_client.batch_execute(LONG).await });
        entered.await.unwrap();
        // Connection::interrupt ignores this idle native root; only the
        // progress-handler latch can cancel the impending CPU-heavy step.
        client.cancel();
        resume.send(()).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(3), operation)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code().unwrap().code(), "57014");
        client.check_connection().await.unwrap();
        finish(client, task).await;
    }

    // [spec:pgorm:sem:turso.lifecycle/test] interrupted RETURNING must terminate before reset can commit
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_returning_does_not_commit_during_reset() {
        let (client, task) = actor().await;
        client
            .batch_execute("CREATE TABLE writes(id integer PRIMARY KEY)")
            .await
            .unwrap();
        let (gate, entered, resume) = gate();
        *client.control.after_row.lock().unwrap() = Some(gate);
        let op_client = Arc::clone(&client);
        let operation = tokio::spawn(async move {
            op_client
                .batch_execute("INSERT INTO writes VALUES(1) RETURNING id")
                .await
        });
        entered.await.unwrap();
        client.cancel();
        resume.send(()).unwrap();
        assert_eq!(
            operation.await.unwrap().unwrap_err().code().unwrap().code(),
            "57014"
        );
        // A commit hidden in reset would cause a duplicate-key violation.
        client
            .batch_execute("INSERT INTO writes VALUES(1)")
            .await
            .unwrap();
        finish(client, task).await;
    }

    // [spec:pgorm:sem:turso.lifecycle/test] cleanup failure preserves primary cause and forbids reuse
    #[tokio::test]
    async fn cleanup_failure_quarantines_and_keeps_primary_error() {
        let (client, task) = actor().await;
        client
            .batch_execute(
                "CREATE TABLE writes(id integer PRIMARY KEY); INSERT INTO writes VALUES(1)",
            )
            .await
            .unwrap();
        client
            .control
            .fail_next_cleanup
            .store(true, Ordering::Release);
        let error = client
            .batch_execute("INSERT INTO writes VALUES(1)")
            .await
            .unwrap_err();
        assert_eq!(error.code().unwrap().code(), "23505");
        assert!(client.is_closed());
        assert!(client.check_connection().await.unwrap_err().is_closed());
        finish(client, task).await;
    }

    // [spec:pgorm:sem:turso.lifecycle/test] unexpected worker termination never becomes successful completion
    #[tokio::test]
    async fn worker_panic_fails_active_request() {
        let (client, task) = actor().await;
        client
            .control
            .panic_next_step
            .store(true, Ordering::Release);
        assert!(client.check_connection().await.unwrap_err().is_closed());
        assert!(client.is_closed());
        drop(client);
        assert!(task.await.unwrap().is_err());
    }

    // [spec:pgorm:sem:turso.lifecycle/test] a stale request drop cannot cancel another operation
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stale_cancellation_does_not_affect_current_operation() {
        let (client, task) = actor().await;
        let (gate, entered, resume) = gate();
        *client.control.before_step.lock().unwrap() = Some(gate);
        let op_client = Arc::clone(&client);
        let operation = tokio::spawn(async move { op_client.check_connection().await });
        entered.await.unwrap();
        let stale = Operation {
            id: u64::MAX,
            cancelled: AtomicBool::new(false),
        };
        client.control.cancel(Some(&stale));
        resume.send(()).unwrap();
        operation.await.unwrap().unwrap();
        finish(client, task).await;
    }

    // [spec:pgorm:sem:turso.lifecycle/test] pending native engine waits observe cancellation
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pending_native_completion_is_aborted_on_cancel() {
        let database = super::super::Database::open(Path::new(":memory:"))
            .await
            .unwrap();
        let native = PgConnection::connect(&database.inner).unwrap();
        let statement = native.prepare("SELECT 1").unwrap();
        let (client, lifetime) = database.connect().await.unwrap();
        let task = tokio::spawn(lifetime);
        let operation = Arc::new(Operation {
            id: u64::MAX,
            cancelled: AtomicBool::new(false),
        });
        let completion = turso_core::Completion::new_wait();
        let pending = completion.clone();
        let operation_wait = Arc::clone(&operation);
        let control_wait = Arc::clone(&client.control);
        let waiting = tokio::task::spawn_blocking(move || {
            wait_completion(
                &statement,
                IOCompletions(pending),
                &operation_wait,
                &control_wait,
            )
        });
        client.control.cancel(Some(&operation));
        let error = tokio::time::timeout(Duration::from_secs(3), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code().unwrap().code(), "57014");
        assert!(completion.finished());
        native.close().unwrap();
        finish(client, task).await;
    }
}
