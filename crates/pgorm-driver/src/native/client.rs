use super::{
    commands::Command,
    rows,
    worker::{Client, RequestGuard},
};
use crate::Error;
use std::sync::{Arc, Weak, atomic::Ordering};
use tokio::sync::{oneshot, watch};

impl Client {
    pub(crate) fn has_pending_restore(&self) -> bool {
        self.control.pending_restore.load(Ordering::Acquire) != 0
    }
    pub(super) fn request_guard(&self) -> Result<RequestGuard, Error> {
        if self.is_closed() {
            return Err(Error::closed());
        }
        Ok(RequestGuard {
            operation: self.control.new_operation(),
            control: Arc::clone(&self.control),
            complete: false,
        })
    }
    pub(super) async fn submit(
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
    pub(super) async fn response<T>(
        &self,
        response: oneshot::Receiver<Result<T, Error>>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<T, Error> {
        self.response_raw(response, shutdown).await?
    }
    pub(super) async fn response_raw<T>(
        &self,
        response: oneshot::Receiver<T>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<T, Error> {
        tokio::select! {
            biased;
            result = response => match result {
                Ok(result) => Ok(result),
                Err(_) => { self.control.shutdown(); Err(Error::closed()) }
            },
            _ = shutdown.changed() => Err(Error::closed()),
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
        owner: Weak<crate::client::InnerClient>,
        sql: &str,
        types: &[crate::types::Type],
    ) -> Result<crate::Statement, Error> {
        let mut guard = self.request_guard()?;
        let (reply, response) = oneshot::channel();
        let mut shutdown = self.control.shutdown.subscribe();
        self.submit(
            Command::Prepare {
                owner,
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
        result
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
    pub(crate) fn is_clean(&self) -> bool {
        !self.sender.is_closed() && self.control.is_clean()
    }
    pub(crate) fn shutdown(&self) {
        self.control.shutdown();
    }
    #[cfg(any(feature = "runtime", test))]
    pub(crate) fn cancel(&self) {
        self.control.cancel(None);
    }
}
