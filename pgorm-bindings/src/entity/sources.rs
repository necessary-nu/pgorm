//! Registered source tuples for a pipeline's `select_sources`: one to six
//! concrete entity types fixed in Rust, which a pipeline's last stage
//! projects under qualifiers of the program's choosing and whose rows Rust
//! decodes through each entity's absence witness, every position optional.

use std::{fmt, marker::PhantomData, sync::Arc};

use futures_util::future::BoxFuture;
use pgorm::{
    EntityTrait, Error, IntoActiveModel,
    pgorm_query::{Name, Values},
    pipeline::{self as pl, PipelineError, SourceList},
};
use serde_json::Value as Json;

use super::{EntityInfo, Read, Registry, model_of, object, reads::Row};
use crate::{Database, Dialect, Failure};

mod sealed {
    pub trait Sealed {}
}

/// The registered entities of a source tuple, in order.
#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub struct SourceBindings {
    pub entities: Vec<Arc<EntityInfo>>,
}

/// A tuple of one to six concrete entity types a pipeline's rows decode as.
/// pgorm's own `SourceList` fixes the arities; this is sealed the same way.
// [spec:pgorm:req:bindings.pipeline-sources+3]
pub trait SourceTypes: sealed::Sealed + Send + Sync + 'static {
    #[doc(hidden)]
    type Selection: SourceList + Send + Sync + 'static;
    #[doc(hidden)]
    fn bindings(registry: &Registry) -> Result<SourceBindings, Failure>;
    #[doc(hidden)]
    fn select(
        pipeline: pl::Pipeline,
        qualifiers: &[String],
    ) -> pl::SelectedSources<Self::Selection>;
    #[doc(hidden)]
    fn models(row: <Self::Selection as SourceList>::Row, bindings: &SourceBindings) -> Row;
}

fn decoded<E>(value: Option<E::Model>, bindings: &SourceBindings, at: usize) -> Option<super::Model>
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    let info = bindings.entities.get(at)?;
    value.map(|value| model_of::<E>(value, info))
}

impl<E> sealed::Sealed for (E,) {}

impl<E> SourceTypes for (E,)
where
    E: EntityTrait + Send + Sync + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
    E::ActiveModel: Send + Sync + 'static,
{
    type Selection = pl::Named<E>;

    fn bindings(registry: &Registry) -> Result<SourceBindings, Failure> {
        Ok(SourceBindings {
            entities: vec![registry.registered::<E>()?],
        })
    }

    fn select(
        pipeline: pl::Pipeline,
        qualifiers: &[String],
    ) -> pl::SelectedSources<Self::Selection> {
        let qualifier = Name::runtime(qualifiers.first().map_or("", String::as_str));
        pipeline.select_sources(pl::named_runtime(E::default(), qualifier))
    }

    fn models(row: Option<E::Model>, bindings: &SourceBindings) -> Row {
        vec![decoded::<E>(row, bindings, 0)]
    }
}

macro_rules! sources {
    ($($entity:ident @ $index:tt),+) => {
        impl<$($entity),+> sealed::Sealed for ($($entity,)+) {}

        impl<$($entity),+> SourceTypes for ($($entity,)+)
        where $(
            $entity: EntityTrait + Send + Sync + 'static,
            $entity::Model: IntoActiveModel<$entity::ActiveModel> + Sync + 'static,
            $entity::ActiveModel: Send + Sync + 'static,
        )+ {
            type Selection = ($(pl::Named<$entity>,)+);

            fn bindings(registry: &Registry) -> Result<SourceBindings, Failure> {
                Ok(SourceBindings { entities: vec![$(registry.registered::<$entity>()?,)+] })
            }

            fn select(pipeline: pl::Pipeline, qualifiers: &[String]) -> pl::SelectedSources<Self::Selection> {
                let qualifier = |at: usize| Name::runtime(qualifiers.get(at).map_or("", String::as_str));
                pipeline.select_sources(($(pl::named_runtime($entity::default(), qualifier($index)),)+))
            }

            fn models(row: <Self::Selection as SourceList>::Row, bindings: &SourceBindings) -> Row {
                vec![$(decoded::<$entity>(row.$index, bindings, $index),)+]
            }
        }
    };
}

sources!(E1 @ 0, E2 @ 1);
sources!(E1 @ 0, E2 @ 1, E3 @ 2);
sources!(E1 @ 0, E2 @ 1, E3 @ 2, E4 @ 3);
sources!(E1 @ 0, E2 @ 1, E3 @ 2, E4 @ 3, E5 @ 4);
sources!(E1 @ 0, E2 @ 1, E3 @ 2, E4 @ 3, E5 @ 4, E6 @ 5);

/// A registered source tuple: its name, the Rust shape and its entities.
#[derive(Debug)]
#[allow(missing_docs)]
pub struct SourcesInfo {
    pub name: String,
    pub shape: &'static str,
    pub bindings: SourceBindings,
    pub(crate) dialect: Dialect,
}

impl SourcesInfo {
    /// The source tuple as `describe()` reports it: its name, the Rust shape,
    /// and each source's entity.
    // [spec:pgorm:req:bindings.pipeline-sources+3]
    pub fn describe(&self) -> Json {
        object(
            self.dialect,
            [
                ("name", self.name.clone().into()),
                ("rust_shape", self.shape.into()),
                (
                    "entities",
                    self.bindings
                        .entities
                        .iter()
                        .map(|entity| entity.name.clone())
                        .collect::<Vec<_>>()
                        .into(),
                ),
            ],
        )
    }
}

impl SourcesInfo {
    /// The qualifiers each source is read under: the ones `given`, one
    /// identifier per source, or else the sources' tables.
    // [spec:pgorm:req:bindings.pipeline-sources+3]
    pub fn qualifiers(&self, given: Option<Vec<String>>) -> Result<Vec<String>, Failure> {
        let entities = &self.bindings.entities;
        let Some(given) = given else {
            return Ok(entities.iter().map(|entity| entity.table.clone()).collect());
        };
        if given.len() != entities.len() {
            return Err(Failure::Construction(format!(
                "the tuple has {} sources, and {} qualifiers were given",
                entities.len(),
                given.len()
            )));
        }
        if given
            .iter()
            .any(|name| name.is_empty() || name.len() > 63 || name.contains('\0'))
        {
            return Err(Failure::Construction(
                "a qualifier is 1–63 UTF-8 bytes without NUL".to_owned(),
            ));
        }
        Ok(given)
    }
}

/// A pipeline's rows selected as a registered source tuple.
pub type Selected = Arc<dyn SelectedBackend>;

/// A registered source tuple, which a pipeline's rows are selected as.
#[allow(missing_docs)]
pub trait SourcesBackend: fmt::Debug + Send + Sync {
    fn info(&self) -> &Arc<SourcesInfo>;
    /// `pipeline`'s rows selected as the tuple, each position under its
    /// qualifier.
    fn select(&self, pipeline: pl::Pipeline, qualifiers: Vec<String>) -> Selected;
}

/// A pipeline's selected sources, `SelectedSources<T>`.
#[allow(missing_docs)]
pub trait SelectedBackend: fmt::Debug + Send + Sync {
    fn info(&self) -> &Arc<SourcesInfo>;
    /// The statement, narrowed to one row by `take(1)` for any read but
    /// `all`.
    fn compile(&self, read: Read) -> Result<(String, Values), PipelineError>;
    /// The read terminal of `SelectedSources`.
    fn run<'a>(&'a self, db: Database<'a>, read: Read) -> BoxFuture<'a, Result<Vec<Row>, Error>>;
}

pub(crate) struct SourcesFactory<T> {
    pub(crate) info: Arc<SourcesInfo>,
    pub(crate) marker: PhantomData<T>,
}

impl<T> fmt::Debug for SourcesFactory<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourcesFactory")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl<T: SourceTypes> SourcesBackend for SourcesFactory<T>
where
    <T::Selection as SourceList>::Row: Send,
{
    fn info(&self) -> &Arc<SourcesInfo> {
        &self.info
    }

    fn select(&self, pipeline: pl::Pipeline, qualifiers: Vec<String>) -> Selected {
        Arc::new(SourcesRead::<T> {
            pipeline,
            qualifiers,
            info: self.info.clone(),
            marker: PhantomData,
        })
    }
}

struct SourcesRead<T> {
    pipeline: pl::Pipeline,
    qualifiers: Vec<String>,
    info: Arc<SourcesInfo>,
    marker: PhantomData<T>,
}

impl<T> fmt::Debug for SourcesRead<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourcesRead")
            .field("info", &self.info)
            .field("qualifiers", &self.qualifiers)
            .finish_non_exhaustive()
    }
}

// [spec:pgorm:req:bindings.pipeline-sources+3]
impl<T: SourceTypes> SelectedBackend for SourcesRead<T>
where
    <T::Selection as SourceList>::Row: Send,
{
    fn info(&self) -> &Arc<SourcesInfo> {
        &self.info
    }

    fn compile(&self, read: Read) -> Result<(String, Values), PipelineError> {
        let pipeline = match read {
            Read::All => self.pipeline.clone(),
            Read::One | Read::Optional => self.pipeline.clone().take(1),
        };
        T::select(pipeline, &self.qualifiers).into_sql()
    }

    fn run<'a>(&'a self, db: Database<'a>, read: Read) -> BoxFuture<'a, Result<Vec<Row>, Error>> {
        Box::pin(async move {
            let query = T::select(self.pipeline.clone(), &self.qualifiers);
            let models = |row| T::models(row, &self.info.bindings);
            Ok(match read {
                Read::All => query.all(&db).await?.into_iter().map(models).collect(),
                Read::One => vec![models(query.one(&db).await?)],
                Read::Optional => query.one_opt(&db).await?.map(models).into_iter().collect(),
            })
        })
    }
}
