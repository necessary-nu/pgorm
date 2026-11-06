//! Owned commands cross the task boundary; database borrows never do.

use crate::{execution::Database, expressions::Compiled};
use futures_util::future::BoxFuture;
use pgorm::{ConnectionTrait, Error, ValueHolder};
use pgorm_bindings::entity::{
    Active, EntityBackend, GraphQuery, Insertion, Insertions, Model, Read as Terminal,
    Row as GraphRow, Select, Selected, VersionWrite, Versions, Write, Written,
};
use tokio_postgres::{Row, types::ToSql};

pub(crate) enum Work {
    Execute(Compiled),
    Fetch(Compiled),
    Entity(Select, Terminal),
    /// A raw statement decoded into a registration's models.
    EntityRaw(Select, Compiled, Terminal),
    Write(Active, Write),
    Versions(std::sync::Arc<dyn EntityBackend>, Box<VersionWrite>),
    Insert(std::sync::Arc<dyn EntityBackend>, Box<Insertion>),
    Graph(GraphQuery, bool),
    /// A registered read's page, count, cursor window or grouped rows, run
    /// on whichever database the target holds.
    Read(Read),
    Sources(Selected, Terminal),
    /// `pgorm::notify`'s channel and payload.
    Notify(String, String),
    /// A script run through `batch_execute`.
    Batch(String),
    /// A statement described through `VerifyStatement::describe`.
    Describe(String),
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
    Described(Vec<pgorm::DescribedColumn>),
    /// Each root of a grouped graph read with its slot's models.
    Grouped(Vec<(Model, Vec<Model>)>),
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
            Self::EntityRaw(query, compiled, terminal) => query
                .run_raw(db, compiled.sql, compiled.values, terminal)
                .await
                .map(Output::Models),
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
            Self::Batch(script) => db.batch_execute(&script).await.map(|()| Output::Finished),
            Self::Describe(sql) => pgorm_bindings::describe(db, &sql)
                .await
                .map(Output::Described),
        }
    }
}
