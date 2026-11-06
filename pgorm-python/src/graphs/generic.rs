use crate::{
    entities::reads::{Decode, Keyset, Pages},
    execution::Database,
    transactions::work::Read,
};
use futures_util::future::BoxFuture;
use pgorm::pgorm_query::Values;
use pgorm::{
    EntityTrait, Error, GraphItem, GraphRow, QueryFilter, QueryOrder, QueryTrait, SelectGraph,
    SelectorTrait,
};
use pyo3::PyResult;
use std::{marker::PhantomData, sync::Arc};

use super::{backend::*, slots::GraphSlots};

pub(crate) struct GraphFactory<E: EntityTrait, S, Build> {
    pub(crate) info: Arc<Info>,
    pub(crate) build: Build,
    pub(crate) marker: PhantomData<(E, S)>,
}

impl<E: EntityTrait, S, Build> std::fmt::Debug for GraphFactory<E, S, Build> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphFactory")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl<E, S, Build> Factory for GraphFactory<E, S, Build>
where
    E: EntityTrait + Send + Sync + 'static,
    S: GraphSlots<E>,
    GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
    GraphItem<E, S>: Send + 'static,
    Build: Fn(&[String]) -> SelectGraph<E, S> + Send + Sync + 'static,
{
    fn info(&self) -> &Arc<Info> {
        &self.info
    }

    fn find(&self, aliases: Vec<String>) -> Query {
        Arc::new(EntityGraphQuery::<E, S> {
            query: (self.build)(&aliases),
            info: self.info.clone(),
            aliases,
        })
    }
}

struct EntityGraphQuery<E: EntityTrait, S> {
    query: SelectGraph<E, S>,
    info: Arc<Info>,
    aliases: Vec<String>,
}

impl<E: EntityTrait, S> std::fmt::Debug for EntityGraphQuery<E, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntityGraphQuery")
            .field("info", &self.info)
            .field("aliases", &self.aliases)
            .finish_non_exhaustive()
    }
}

// [spec:pgorm:req:bindings.entity-graphs+4]
impl<E, S> QueryBackend for EntityGraphQuery<E, S>
where
    E: EntityTrait + Send + Sync + 'static,
    S: GraphSlots<E>,
    GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
    GraphItem<E, S>: Send + 'static,
{
    fn info(&self) -> &Arc<Info> {
        &self.info
    }
    fn aliases(&self) -> &[String] {
        &self.aliases
    }

    fn change(&self, change: Change) -> Query {
        let query = self.query.clone();
        let query = match change {
            Change::Filter(condition) => query.filter(condition),
            Change::Order(expr, order, None) => query.order_by(expr, order),
            Change::Order(expr, order, Some(nulls)) => {
                query.order_by_with_nulls(expr, order, nulls)
            }
        };
        Arc::new(Self {
            query,
            info: self.info.clone(),
            aliases: self.aliases.clone(),
        })
    }

    fn compile(&self, optional: bool) -> (String, Values) {
        let mut query = self.query.clone();
        if optional {
            QueryTrait::query(&mut query).narrow_limit_to_one();
        }
        query.build()
    }

    fn run<'a>(
        &'a self,
        db: Database<'a>,
        optional: bool,
    ) -> BoxFuture<'a, Result<Vec<Row>, Error>> {
        Box::pin(self.read(db, optional))
    }

    fn cursor(&self, source: usize, columns: &[String]) -> PyResult<Arc<Keyset>> {
        S::keyset(&self.query, source, columns, &self.info.bindings).map(|keyset| keyset.0)
    }

    fn pages(&self) -> Arc<Pages> {
        let info = self.info.clone();
        Arc::new(pgorm::PagedRead::new(self.query.clone(), move |row| {
            S::models(row, &info.bindings)
        }))
    }

    fn decode(&self) -> Decode {
        let info = self.info.clone();
        Arc::new(move |row| {
            let item = GraphRow::<E, S>::from_raw_query_result(row)?;
            Ok(S::models(item, &info.bindings))
        })
    }

    fn grouped(&self) -> PyResult<Read> {
        S::grouped(self.query.clone(), &self.info.bindings).map(|read| read.0)
    }
}

impl<E, S> EntityGraphQuery<E, S>
where
    E: EntityTrait + Send + Sync + 'static,
    S: GraphSlots<E>,
    GraphRow<E, S>: SelectorTrait,
    GraphItem<E, S>: Send + 'static,
{
    async fn read(&self, db: Database<'_>, optional: bool) -> Result<Vec<Row>, Error> {
        let rows = if optional {
            self.query.clone().one_opt(&db).await?.into_iter().collect()
        } else {
            self.query.clone().all(&db).await?
        };
        Ok(rows
            .into_iter()
            .map(|row| S::models(row, &self.info.bindings))
            .collect())
    }
}
