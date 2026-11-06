use crate::execution::Database;
use futures_util::future::BoxFuture;
use pgorm::{
    Error,
    pgorm_query::{Condition, NullOrdering, Order, SimpleExpr, Values},
};
use std::{fmt::Debug, sync::Arc};

use super::slots::GraphBindings;
use crate::{
    entities::{
        PyEntityModel,
        reads::{Decode, Keyset, Pages},
    },
    transactions::work::Read,
};

pub(crate) type Row = Vec<Option<PyEntityModel>>;
pub(crate) type Query = Arc<dyn QueryBackend>;

#[derive(Clone, Debug)]
pub(crate) struct Info {
    pub(crate) name: String,
    pub(crate) rust_shape: &'static str,
    pub(crate) bindings: GraphBindings,
}

impl Info {
    /// The graph as `describe()` reports it: its name, the Rust shape, and
    /// each source's entity and slot kind.
    // [spec:pgorm:req:bindings.entity-graphs+4]
    pub(crate) fn describe(&self) -> serde_json::Value {
        serde_json::json!({"name": self.name, "rust_shape": self.rust_shape,
            "sources": self.bindings.sources.iter().enumerate().map(|(index, source)| {
                serde_json::json!({"index":index, "entity":source.entity.name,
                    "slot": if index == 0 { "root" } else if source.optional { "Opt" } else { "Req" }})
            }).collect::<Vec<_>>()})
    }
}

pub(crate) trait Factory: Debug + Send + Sync {
    fn info(&self) -> &Arc<Info>;
    fn find(&self, aliases: Vec<String>) -> Query;
}

#[derive(Clone, Debug)]
pub(crate) enum Change {
    Filter(Condition),
    Order(SimpleExpr, Order, Option<NullOrdering>),
}

pub(crate) trait QueryBackend: Debug + Send + Sync {
    fn info(&self) -> &Arc<Info>;
    fn aliases(&self) -> &[String];
    fn change(&self, change: Change) -> Query;
    fn compile(&self, optional: bool) -> (String, Values);
    fn run<'a>(
        &'a self,
        db: Database<'a>,
        optional: bool,
    ) -> BoxFuture<'a, Result<Vec<Row>, Error>>;
    /// `cursor_by` over root columns when `source` is 0, otherwise
    /// `cursor_by_on` over the columns of slot `source`.
    fn cursor(&self, source: usize, columns: &[String]) -> pyo3::PyResult<Arc<Keyset>>;
    fn pages(&self) -> Arc<Pages>;
    /// The decode `SelectGraph::stream` runs on each row.
    fn decode(&self) -> Decode;
    /// `all_grouped`, which only a graph of one `Opt` slot has.
    fn grouped(&self) -> pyo3::PyResult<Read>;
}
