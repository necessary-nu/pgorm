use super::{
    codecs::{self, ScalarCodec},
    describe,
};
use crate::{Column, Error, types::Type};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc;
use turso_pg::PgConnection;

pub(crate) struct Prepared {
    pub(crate) id: u64,
    pub(crate) params: Vec<Type>,
    pub(crate) columns: Vec<Column>,
    pub(crate) parameter_codecs: Arc<[ScalarCodec]>,
}

pub(super) struct Entry {
    pub(super) statement: turso_pg::Statement,
    pub(super) result_codecs: Box<[ScalarCodec]>,
    // Retain the exact compiled program described to the public Statement.
    // Keeping the Arc also prevents pointer reuse from hiding a reprepare.
    described_program: Arc<turso_core::PreparedProgram>,
    described_schema_version: u32,
}

impl Entry {
    pub(super) fn metadata_is_current(&self) -> bool {
        Arc::ptr_eq(
            &self.described_program,
            self.statement.get_program().prepared(),
        )
    }

    fn validate_context(&self, connection: &PgConnection) -> Result<(), Error> {
        // Another connection can publish a newer schema while this worker is
        // idle. Respect the native transaction snapshot when refreshing it.
        connection.inner().maybe_update_schema();
        // Same-connection DDL can advance the cookie without bumping the
        // prepare-context generation. Catch it before a write can reach the
        // engine's SchemaUpdated/reprepare path inside step.
        let schema_version = connection
            .inner()
            .with_schema(turso_core::MAIN_DB_ID, |schema| schema.schema_version);
        if !self.metadata_is_current()
            || self.described_schema_version != schema_version
            || !self
                .described_program
                .is_compatible_with(connection.inner())
        {
            return Err(Error::turso_statement_invalidated());
        }
        Ok(())
    }
}

/// A response queued to an abandoned prepare future must close its statement.
/// Successful receipt transfers this responsibility to the public Statement.
pub(super) struct PreparedLease {
    id: Option<u64>,
    close: mpsc::UnboundedSender<u64>,
}
impl PreparedLease {
    pub(super) fn disarm(mut self) {
        self.id = None;
    }
}
impl Drop for PreparedLease {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            let _ = self.close.send(id);
        }
    }
}

#[derive(Default)]
pub(super) struct Store {
    entries: HashMap<u64, Entry>,
    next_id: u64,
}
impl Store {
    // [spec:pgorm:req:turso.parameters] explicit native types are established before client codecs run
    // [spec:pgorm:sem:turso.execution] native parsing enforces one command and retained handles avoid reparsing
    pub(super) fn prepare(
        &mut self,
        connection: &PgConnection,
        sql: &str,
        types: Vec<Type>,
        close: &mpsc::UnboundedSender<u64>,
    ) -> Result<(Prepared, PreparedLease), Error> {
        if turso_pg::split_statements(sql).map_err(Error::turso)?.len() > 1 {
            return Err(Error::turso_unsupported(
                "native prepared statements accept exactly one SQL command",
            ));
        }
        let statement = connection.prepare(sql).map_err(Error::turso)?;
        let count = statement.parameters_count();
        if types.len() > count {
            return Err(Error::parameters(types.len(), count));
        }
        if types.len() < count {
            return Err(Error::turso_unsupported(
                "native parameter inference is unavailable; supply every parameter type with prepare_typed",
            ));
        }
        let parameter_codecs = codecs::parameter_codecs(&types)?;
        let (columns, result_codecs) = describe::describe_columns(&statement, &types)?;
        let described_program = Arc::clone(statement.get_program().prepared());
        let described_schema_version = connection
            .inner()
            .with_schema(turso_core::MAIN_DB_ID, |schema| schema.schema_version);
        let id = self.next_id.checked_add(1).ok_or_else(|| {
            Error::turso_worker(std::io::Error::other("native statement ID space exhausted"))
        })?;
        self.next_id = id;
        self.entries.insert(
            id,
            Entry {
                statement,
                result_codecs,
                described_program,
                described_schema_version,
            },
        );
        Ok((
            Prepared {
                id,
                params: types,
                columns,
                parameter_codecs,
            },
            PreparedLease {
                id: Some(id),
                close: close.clone(),
            },
        ))
    }

    // [spec:pgorm:sem:turso.execution] reject stale context before binding or stepping a cached handle
    pub(super) fn get(&mut self, id: u64, connection: &PgConnection) -> Result<&mut Entry, Error> {
        let entry = self.entries.get_mut(&id).ok_or_else(|| {
            Error::turso_worker(std::io::Error::other(
                "native statement was closed or does not belong to this connection",
            ))
        })?;
        entry.validate_context(connection)?;
        Ok(entry)
    }

    pub(super) fn close(&mut self, id: u64) -> Result<(), Error> {
        if let Some(mut entry) = self.entries.remove(&id) {
            entry.statement.reset().map_err(Error::turso)?;
        }
        Ok(())
    }

    pub(super) fn close_all(&mut self) -> Result<(), Error> {
        let mut failure = None;
        for (_, mut entry) in self.entries.drain() {
            if let Err(error) = entry.statement.reset().map_err(Error::turso) {
                failure = Some(match failure {
                    None => error,
                    Some(primary) => Error::turso_cleanup(primary, error),
                });
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
