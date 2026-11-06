//! Owned commands cross the task boundary; database borrows never do.

use crate::{
    entities::backend::{
        Active, Insertion, Insertions, Model, Select, Terminal, VersionWrite, Versions, Write,
        Written,
    },
    execution::Database,
    expressions::Compiled,
    graphs::backend::{Query as Graph, Row as GraphRow},
    pipeline::selected_backend::{Query as Sources, Terminal as SourceTerminal},
};
use futures_util::future::BoxFuture;
use pgorm::{ConnectionTrait, Error, ValueHolder};
use pgorm_migration::{MigrationStatus, MigratorTrait};
use tokio_postgres::{Row, types::ToSql};

pub(crate) enum Work {
    Execute(Compiled),
    Fetch(Compiled),
    Entity(Select, Terminal),
    Write(Active, Write),
    Versions(
        std::sync::Arc<dyn crate::entities::backend::EntityBackend>,
        Box<VersionWrite>,
    ),
    Insert(
        std::sync::Arc<dyn crate::entities::backend::EntityBackend>,
        Box<Insertion>,
    ),
    Graph(Graph, bool),
    /// A registered read's page, count, cursor window or grouped rows, run
    /// on whichever database the target holds.
    Read(Read),
    Sources(Sources, SourceTerminal),
    /// `pgorm::notify`'s channel and payload.
    Notify(String, String),
    /// pgorm-migration's `install`, or its reading of every migration's
    /// status, on what the caller holds.
    Ledger(Box<pgorm_migration::Migrations>, Ledger),
}

/// What [`Work::Ledger`] does with a migrator's ledger.
pub(crate) enum Ledger {
    Install,
    Status,
}

pub(crate) type Read =
    Box<dyn for<'a> FnOnce(Database<'a>) -> BoxFuture<'a, Result<Output, Error>> + Send>;

pub(crate) enum Output {
    Count(u64),
    /// A paginator's rows and the pages they fill.
    Counted(u64, u64),
    Rows(Vec<Row>),
    Models(Vec<Model>),
    Written(Written),
    Versions(Vec<Versions>),
    Inserted(Insertions),
    Graph(Vec<GraphRow>),
    /// Each root of a grouped graph read with its slot's models.
    Grouped(
        Vec<(
            crate::entities::PyEntityModel,
            Vec<crate::entities::PyEntityModel>,
        )>,
    ),
    /// Each migration's name, and whether the ledger records it applied.
    Migrations(Vec<(String, bool)>),
    Finished,
}

impl Work {
    pub(crate) async fn run(self, db: Database<'_>) -> Result<Output, Error> {
        match self {
            Self::Execute(compiled) => {
                let values: Vec<_> = compiled.values.0.into_iter().map(ValueHolder).collect();
                let params: Vec<_> = values.iter().map(|v| v as &(dyn ToSql + Sync)).collect();
                db.execute(&compiled.sql, &params).await.map(Output::Count)
            }
            Self::Fetch(compiled) => {
                let values: Vec<_> = compiled.values.0.into_iter().map(ValueHolder).collect();
                let params: Vec<_> = values.iter().map(|v| v as &(dyn ToSql + Sync)).collect();
                db.query_all(&compiled.sql, &params).await.map(Output::Rows)
            }
            Self::Entity(query, terminal) => query.run(db, terminal).await.map(Output::Models),
            Self::Write(active, write) => active.run(db, write).await.map(Output::Written),
            Self::Versions(entity, write) => {
                entity.versions(db, *write).await.map(Output::Versions)
            }
            Self::Insert(entity, insertion) => {
                entity.insert(db, *insertion).await.map(Output::Inserted)
            }
            Self::Graph(query, optional) => query.run(db, optional).await.map(Output::Graph),
            Self::Read(read) => read(db).await,
            Self::Sources(query, terminal) => query.run(db, terminal).await.map(Output::Graph),
            Self::Notify(channel, payload) => pgorm::notify(&db, &channel, &payload)
                .await
                .map(|()| Output::Finished),
            // [spec:pgorm:req:bindings.migrations]    the runner's own install and reads
            Self::Ledger(migrations, Ledger::Install) => {
                migrations.install(&db).await.map(|()| Output::Finished)
            }
            Self::Ledger(migrations, Ledger::Status) => migrations
                .get_migration_with_status(&db)
                .await
                .map(|migrations| {
                    Output::Migrations(
                        migrations
                            .iter()
                            .map(|migration| {
                                (
                                    migration.name().to_owned(),
                                    migration.status() == MigrationStatus::Applied,
                                )
                            })
                            .collect(),
                    )
                }),
        }
    }
}
