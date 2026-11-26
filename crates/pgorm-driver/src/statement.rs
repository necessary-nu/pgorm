use crate::client::InnerClient;
use crate::codec::FrontendMessage;
use crate::connection::RequestMessages;
use crate::types::Type;
use postgres_protocol::message::frontend;
#[cfg(feature = "turso")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

#[cfg(feature = "turso")]
struct NativeStatement {
    id: u64,
    parameter_codecs: Arc<[crate::native::codecs::ScalarCodec]>,
    invalidated: AtomicBool,
}

struct StatementInner {
    client: Weak<InnerClient>,
    name: String,
    params: Vec<Type>,
    columns: Vec<Column>,
    #[cfg(feature = "turso")]
    native: Option<NativeStatement>,
}

impl Drop for StatementInner {
    fn drop(&mut self) {
        #[cfg(feature = "turso")]
        if let Some(native) = &self.native {
            if let Some(client) = self.client.upgrade()
                && let Some(actor) = client.native()
            {
                actor.close_statement(native.id);
            }
            return;
        }
        if self.name.is_empty() {
            // Unnamed statements don't need to be closed
            return;
        }
        if let Some(client) = self.client.upgrade() {
            let buf = client.with_buf(|buf| {
                frontend::close(b'S', &self.name, buf).unwrap();
                frontend::sync(buf);
                buf.split().freeze()
            });
            let _ = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)));
        }
    }
}

/// A prepared statement.
///
/// Prepared statements can only be used with the connection that created them.
#[derive(Clone)]
pub struct Statement(Arc<StatementInner>);

#[cfg(feature = "turso")]
pub(crate) struct WeakStatement(Weak<StatementInner>);

#[cfg(feature = "turso")]
impl WeakStatement {
    pub(crate) fn new(statement: &Statement) -> Self {
        Self(Arc::downgrade(&statement.0))
    }

    pub(crate) fn mark_invalidated(&self) {
        if let Some(statement) = self.0.upgrade()
            && let Some(native) = &statement.native
        {
            native.invalidated.store(true, Ordering::Release);
        }
    }
}

impl Statement {
    pub(crate) fn new(
        inner: &Arc<InnerClient>,
        name: String,
        params: Vec<Type>,
        columns: Vec<Column>,
    ) -> Statement {
        Statement(Arc::new(StatementInner {
            client: Arc::downgrade(inner),
            name,
            params,
            columns,
            #[cfg(feature = "turso")]
            native: None,
        }))
    }

    pub(crate) fn unnamed(params: Vec<Type>, columns: Vec<Column>) -> Statement {
        Statement(Arc::new(StatementInner {
            client: Weak::new(),
            name: String::new(),
            params,
            columns,
            #[cfg(feature = "turso")]
            native: None,
        }))
    }

    #[cfg(feature = "turso")]
    pub(crate) fn from_native(
        client: Weak<InnerClient>,
        prepared: crate::native::Prepared,
    ) -> Self {
        Self(Arc::new(StatementInner {
            client,
            name: String::new(),
            params: prepared.params,
            columns: prepared.columns,
            native: Some(NativeStatement {
                id: prepared.id,
                parameter_codecs: prepared.parameter_codecs,
                invalidated: AtomicBool::new(false),
            }),
        }))
    }

    #[cfg(feature = "turso")]
    // [spec:pgorm:sem:turso.execution] ownership and known invalidation are checked before parameter encoding
    pub(crate) fn native_id(&self, actor: &crate::native::Client) -> Result<u64, crate::Error> {
        let native = self.0.native.as_ref().ok_or_else(|| {
            crate::Error::turso_unsupported("a statement prepared on a PostgreSQL connection")
        })?;
        if native.invalidated.load(Ordering::Acquire) {
            return Err(crate::Error::turso_statement_invalidated());
        }
        let owner = self.0.client.upgrade().ok_or_else(crate::Error::closed)?;
        if !owner
            .native()
            .is_some_and(|creator| std::ptr::eq(creator.as_ref(), actor))
        {
            return Err(crate::Error::turso_unsupported(
                "a statement from another native connection",
            ));
        }
        if actor.is_closed() {
            return Err(crate::Error::closed());
        }
        Ok(native.id)
    }

    /// Whether the native engine has invalidated this prepared statement.
    ///
    /// An invalidated statement must be prepared again. An error reported
    /// after native execution does not establish that the operation had no
    /// effects, so callers should not blindly retry it. PostgreSQL statements
    /// always return `false` here.
    #[cfg(feature = "turso")]
    pub fn is_invalidated(&self) -> bool {
        self.0
            .native
            .as_ref()
            .is_some_and(|native| native.invalidated.load(Ordering::Acquire))
    }

    #[cfg(feature = "turso")]
    pub(crate) fn mark_invalidated(&self) {
        if let Some(native) = &self.0.native {
            native.invalidated.store(true, Ordering::Release);
        }
    }

    #[cfg(feature = "turso")]
    pub(crate) fn native_codecs(&self) -> Option<&[crate::native::codecs::ScalarCodec]> {
        self.0
            .native
            .as_ref()
            .map(|native| native.parameter_codecs.as_ref())
    }

    pub(crate) fn name(&self) -> &str {
        &self.0.name
    }

    /// Returns the expected types of the statement's parameters.
    pub fn params(&self) -> &[Type] {
        &self.0.params
    }

    /// Returns information about the columns returned when the statement is queried.
    pub fn columns(&self) -> &[Column] {
        &self.0.columns
    }
}

impl std::fmt::Debug for Statement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.debug_struct("Statement")
            .field("name", &self.0.name)
            .field("params", &self.0.params)
            .field("columns", &self.0.columns)
            .finish_non_exhaustive()
    }
}

/// Information about a column of a query.
#[derive(Debug)]
pub struct Column {
    pub(crate) name: String,
    pub(crate) table_oid: Option<u32>,
    pub(crate) column_id: Option<i16>,
    pub(crate) type_modifier: i32,
    pub(crate) r#type: Type,
}

impl Column {
    /// Returns the name of the column.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the OID of the underlying database table.
    pub fn table_oid(&self) -> Option<u32> {
        self.table_oid
    }

    /// Return the column ID within the underlying database table.
    pub fn column_id(&self) -> Option<i16> {
        self.column_id
    }

    /// Return the type modifier
    pub fn type_modifier(&self) -> i32 {
        self.type_modifier
    }

    /// Returns the type of the column.
    pub fn type_(&self) -> &Type {
        &self.r#type
    }
}
