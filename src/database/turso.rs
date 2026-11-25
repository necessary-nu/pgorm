use super::{
    DatabasePool, ManagerConfig, PoolBuilder, RecyclingMethod, pool_build_err,
    simple_manager_config,
};
use crate::Error;
use pgorm_driver::{Config, TursoDatabase};
use pgorm_pool::{Manager, Pool, TursoConnectImpl};
use std::{convert::identity, path::Path};

/// Opens an experimental embedded Turso database and verifies its first connection.
///
/// Each pool owns one shared database, including `:memory:`. Separate calls
/// opening `:memory:` create isolated databases. The default manager uses Fast
/// recycling and the normal statement-cache bound.
///
/// Opening and connection errors are reported here, before returning the pool.
/// Requires the experimental `turso` feature.
// [spec:pgorm:req:turso.pool]
pub async fn connect_turso(path: impl AsRef<Path>) -> Result<DatabasePool, Error> {
    connect_turso_with(path, simple_manager_config(None), identity).await
}

/// Opens an experimental embedded Turso pool with manager settings and hooks.
///
/// Uses the existing pool builder, statement cache and transaction API.
/// The first connection and its creation hooks finish before this returns;
/// a hook must leave the connection idle, with all native work completed.
/// Further connections share the database and open as the pool needs them.
///
/// PostgreSQL Clean recycling and notification-based cache invalidation are
/// rejected before the database is opened. Fast, Verified and Custom recycling
/// follow the native cleanup rules documented on [`RecyclingMethod`].
///
/// # Errors
///
/// Reports unsupported manager options, rejected pool configuration, database
/// opening errors and failure to establish a usable first connection.
// [spec:pgorm:req:turso.pool]
pub async fn connect_turso_with(
    path: impl AsRef<Path>,
    manager: ManagerConfig,
    build: impl FnOnce(PoolBuilder) -> PoolBuilder,
) -> Result<DatabasePool, Error> {
    if matches!(&manager.recycling_method, RecyclingMethod::Clean) {
        return Err(Error::Custom(
            "native Turso does not support PostgreSQL Clean recycling".into(),
        ));
    }
    if manager.statement_cache_channel.is_some() {
        return Err(Error::Custom(
            "native Turso does not support notification-based statement cache invalidation".into(),
        ));
    }
    let database = TursoDatabase::open(path).await?;
    let manager = Manager::from_connect(Config::new(), TursoConnectImpl { database }, manager);
    let pool = build(Pool::builder(manager))
        .build()
        .map(DatabasePool)
        .map_err(pool_build_err)?;
    if pool.status().max_size == 0 {
        return Err(Error::Custom(
            "native Turso pool initialization requires a nonzero maximum size".into(),
        ));
    }
    let connection = pool.get().await?;
    // A creation hook can retain a backpressured stream. Do not queue health
    // behind unfinished work whose consumer may be waiting for this constructor.
    if !connection.0.is_turso_clean() {
        return Err(Error::Custom(
            "native Turso pool initialization left unfinished work or an open transaction".into(),
        ));
    }
    connection.0.check_connection().await?;
    drop(connection);
    Ok(pool)
}
