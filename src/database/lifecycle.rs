use pgorm_pool::Object;

use super::{DatabaseConnection, DatabasePool};

impl DatabasePool {
    /// Close the pool, rejecting waiters and discarding idle connections.
    /// Checked-out connections remain usable until their owners release them.
    /// A pool listening on a
    /// [`statement_cache_channel`](crate::ManagerConfig::statement_cache_channel)
    /// closes that connection too.
    // [spec:pgorm:req:bindings.connections+3]
    // [spec:pgorm:req:conn.pool.statement-cache.listen+1]    closing the pool ends the listener
    pub fn close(&self) {
        self.0.close();
        self.0.manager().stop_statement_cache_listener();
    }

    /// Whether this pool has been explicitly closed.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    /// Drop every connection's cached prepared statements, so each statement is
    /// prepared again the next time a connection runs it.
    ///
    /// Call this after DDL that changes what a prepared statement was planned
    /// with. PostgreSQL re-plans most such changes itself, but not a domain's
    /// changed default: an insert prepared before `ALTER DOMAIN … SET DEFAULT`
    /// keeps writing the old one. A cached statement whose result type DDL
    /// changed is rejected with SQLSTATE `0A000` on its next use, which inside
    /// a transaction reaches the caller; clearing first spares that.
    ///
    /// An idle connection's statements are cleared before the pool next lends
    /// it. A connection checked out at the time keeps its statements until it
    /// is returned, and is cleared before it is lent again, so a caller holding
    /// one across the DDL returns it and takes another to see the change. The
    /// cleared statements are closed on the server, not only forgotten by the
    /// client.
    ///
    /// This reaches only this process's pool. Another process's pool clears
    /// when it hears a notification on its
    /// [`statement_cache_channel`](crate::ManagerConfig::statement_cache_channel),
    /// which pgorm-migration's runner sends.
    // [spec:pgorm:req:conn.pool.statement-cache.clear+1]
    pub fn clear_statement_caches(&self) {
        self.0.manager().statement_caches.clear_before_reuse();
    }
}

impl DatabaseConnection {
    /// Discard this connection instead of returning it to the pool.
    ///
    /// Use this when cancellation leaves the outcome or session state unknown.
    /// Dropping the detached client aborts its connection task.
    // [spec:pgorm:req:bindings.connections+3]
    pub fn discard(self) {
        drop(Object::take(self.0));
    }
}
