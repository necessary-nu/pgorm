//! The build-time collection of an application's concrete registrations.
//! Its module owns it once installed; a program cannot add a generic
//! instantiation, only use the ones compiled in.

use std::{
    any::TypeId,
    collections::{BTreeMap, HashMap, HashSet},
    marker::PhantomData,
    sync::Arc,
};

use pgorm::{
    EntityTrait, GraphItem, GraphRow, IntoActiveModel, SelectGraph, SelectorTrait,
    pipeline::SourceList,
};

use super::{
    EntityAdapter, EntityBackend, EntityInfo,
    disjoint::DisjointCore,
    graph::{GraphBackend, GraphFactory, GraphInfo, GraphSlots},
    one_of::OneOfCore,
    registration_name,
    sources::{SourceTypes, SourcesBackend, SourcesFactory, SourcesInfo},
};
use crate::{Dialect, Failure};

/// The concrete Rust entities, graph shapes, source tuples, disjoint enums
/// and one-of enums one native module carries, which a program reaches by
/// name.
#[derive(Debug)]
pub struct Registry {
    dialect: Dialect,
    entities: BTreeMap<String, Arc<dyn EntityBackend>>,
    /// The first registration of each Rust entity type, which a graph or a
    /// source tuple over the type reads its metadata from.
    types: HashMap<TypeId, String>,
    /// Every registered type with the table its instance names, so one table
    /// is not registered twice under one type.
    tables: HashSet<(TypeId, Option<String>, String)>,
    graphs: BTreeMap<String, Arc<dyn GraphBackend>>,
    sources: BTreeMap<String, Arc<dyn SourcesBackend>>,
    pub(super) disjoints: BTreeMap<String, Arc<DisjointCore>>,
    pub(super) one_ofs: BTreeMap<String, Arc<OneOfCore>>,
}

impl Registry {
    /// An empty registry of the binding `dialect` names, whose descriptions
    /// and refusals it spells.
    pub fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            entities: BTreeMap::new(),
            types: HashMap::new(),
            tables: HashSet::new(),
            graphs: BTreeMap::new(),
            sources: BTreeMap::new(),
            disjoints: BTreeMap::new(),
            one_ofs: BTreeMap::new(),
        }
    }

    /// The binding whose spelling the registry's refusals and descriptions
    /// take.
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Register `entity`, an instance of an entity whose names may be
    /// computed at run time, under `name`: every statement of the
    /// registration names the instance's table, as Rust's `_in` constructors
    /// do. A type may be registered once for each table its instances name;
    /// a name, or a type and table already registered, is refused, and
    /// nothing is added.
    // [spec:pgorm:req:bindings.entities+6]
    pub fn entity_in<E>(&mut self, name: &str, entity: E) -> Result<(), Failure>
    where
        E: EntityTrait + Send + Sync + 'static,
        E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
        E::ActiveModel: Send + Sync + 'static,
    {
        let table = (
            TypeId::of::<E>(),
            entity.schema_name().map(str::to_owned),
            entity.table_name().to_owned(),
        );
        if self.entities.contains_key(name) || self.tables.contains(&table) {
            return Err(Failure::Construction(format!(
                "{name:?} or its Rust entity is already registered"
            )));
        }
        let info = Arc::new(EntityInfo::of(name, entity)?);
        self.entities.insert(
            name.to_owned(),
            Arc::new(EntityAdapter::<E> { info, entity }),
        );
        self.types
            .entry(TypeId::of::<E>())
            .or_insert_with(|| name.to_owned());
        self.tables.insert(table);
        Ok(())
    }

    /// Register a `SelectGraph` shape and the factory that builds it. The
    /// factory receives one alias per joined slot, in tuple order, and must
    /// join each slot under its alias; every source entity is registered
    /// first.
    // [spec:pgorm:req:bindings.entity-graphs+4]
    pub fn graph<E, S, Build>(&mut self, name: &str, build: Build) -> Result<(), Failure>
    where
        E: EntityTrait + Send + Sync + 'static,
        S: GraphSlots<E>,
        GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
        GraphItem<E, S>: Send + 'static,
        Build: Fn(&[String]) -> SelectGraph<E, S> + Send + Sync + 'static,
    {
        registration_name(name)?;
        if self.graphs.contains_key(name) {
            return Err(Failure::Construction(format!(
                "the graph {name:?} is already registered"
            )));
        }
        let info = Arc::new(GraphInfo {
            name: name.to_owned(),
            shape: std::any::type_name::<SelectGraph<E, S>>(),
            bindings: S::bindings(self)?,
            dialect: self.dialect,
        });
        self.graphs.insert(
            name.to_owned(),
            Arc::new(GraphFactory::<E, S, Build> {
                info,
                build,
                marker: PhantomData,
            }),
        );
        Ok(())
    }

    /// Register a tuple of one to six entity types a pipeline's
    /// `select_sources` decodes rows as, each entity registered first.
    // [spec:pgorm:req:bindings.pipeline-sources+3]
    pub fn sources<T: SourceTypes>(&mut self, name: &str) -> Result<(), Failure>
    where
        <T::Selection as SourceList>::Row: Send,
    {
        registration_name(name)?;
        if self.sources.contains_key(name) {
            return Err(Failure::Construction(format!(
                "the source tuple {name:?} is already registered"
            )));
        }
        let info = Arc::new(SourcesInfo {
            name: name.to_owned(),
            shape: std::any::type_name::<T>(),
            bindings: T::bindings(self)?,
            dialect: self.dialect,
        });
        self.sources.insert(
            name.to_owned(),
            Arc::new(SourcesFactory::<T> {
                info,
                marker: PhantomData,
            }),
        );
        Ok(())
    }

    /// Every registered entity, by the name it is registered under.
    pub fn entities(&self) -> &BTreeMap<String, Arc<dyn EntityBackend>> {
        &self.entities
    }

    /// Every registered graph shape, by name.
    pub fn graphs(&self) -> &BTreeMap<String, Arc<dyn GraphBackend>> {
        &self.graphs
    }

    /// Every registered source tuple, by name.
    pub fn source_tuples(&self) -> &BTreeMap<String, Arc<dyn SourcesBackend>> {
        &self.sources
    }

    /// Every registered disjoint enum, by name.
    pub fn disjoints(&self) -> &BTreeMap<String, Arc<DisjointCore>> {
        &self.disjoints
    }

    /// Every registered one-of reference, by name.
    pub fn one_ofs(&self) -> &BTreeMap<String, Arc<OneOfCore>> {
        &self.one_ofs
    }

    /// The registered entity whose Rust type is `entity`, its first
    /// registration.
    pub fn backend_of(&self, entity: TypeId) -> Option<Arc<dyn EntityBackend>> {
        self.types
            .get(&entity)
            .and_then(|name| self.entities.get(name))
            .cloned()
    }

    /// The registered entity `E`, refused as one a registration over it
    /// (`over`, "a graph or source tuple of it") needs first.
    pub(super) fn backend<E: EntityTrait + 'static>(
        &self,
        over: &str,
    ) -> Result<Arc<dyn EntityBackend>, Failure> {
        self.backend_of(TypeId::of::<E>()).ok_or_else(|| {
            Failure::Construction(format!(
                "register the entity {} before {over}",
                std::any::type_name::<E>()
            ))
        })
    }

    /// The registration of `E`, which a graph's or a source tuple's sources
    /// name.
    pub fn registered<E: EntityTrait + 'static>(&self) -> Result<Arc<EntityInfo>, Failure> {
        self.backend::<E>("a graph or source tuple of it")
            .map(|entity| entity.info().clone())
    }
}

#[cfg(test)]
mod tests {
    use pgorm::EntityTrait;

    use super::*;

    mod widget {
        use pgorm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, Entity)]
        #[pgorm(table_name = "widgets", schema_name = "app")]
        pub struct Model {
            #[pgorm(primary_key, auto_increment = false)]
            pub id: i32,
            pub label: Option<String>,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {}
    }

    mod first {
        use pgorm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, Entity)]
        #[pgorm(table_name = "g1")]
        pub struct Model {
            #[pgorm(primary_key, auto_increment = false)]
            pub id: i32,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {}
    }

    mod gadget {
        use pgorm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, Entity)]
        #[pgorm(table_name = "gadgets")]
        pub struct Model {
            #[pgorm(primary_key, auto_increment = false)]
            pub id: i64,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {}
    }

    // [spec:pgorm:req:bindings.entities+6/test]    a name or a Rust entity registered
    // twice, a name out of bounds and a graph of an unregistered entity are each
    // refused, and nothing is added
    #[test]
    fn registrations_refuse_duplicates_and_unregistered_sources() -> Result<(), Failure> {
        let mut registry = Registry::new(Dialect::JavaScript);
        registry.entity_in("app.Widget", widget::Entity)?;
        assert!(registry.entity_in("app.Widget", gadget::Entity).is_err());
        assert!(registry.entity_in("again.Widget", widget::Entity).is_err());
        assert!(registry.entity_in("", gadget::Entity).is_err());
        assert!(
            registry
                .entity_in(&"x".repeat(256), gadget::Entity)
                .is_err()
        );
        assert_eq!(registry.entities().len(), 1);
        assert!(
            registry
                .graph::<gadget::Entity, (), _>("app.Gadgets", |_| gadget::Entity::graph())
                .is_err()
        );
        registry.entity_in("app.Gadget", gadget::Entity)?;
        registry.graph::<gadget::Entity, (), _>("app.Gadgets", |_| gadget::Entity::graph())?;
        assert!(
            registry
                .graph::<gadget::Entity, (), _>("app.Gadgets", |_| gadget::Entity::graph())
                .is_err()
        );
        let described = registry.entities()["app.Widget"]
            .info()
            .describe(Dialect::JavaScript);
        assert_eq!(described["schema"], "app");
        assert_eq!(described["primaryKey"], serde_json::json!(["id"]));
        assert_eq!(described["columns"][1]["nullable"], true);
        assert_eq!(described["columns"][1]["kind"], "text");
        let described = registry.entities()["app.Widget"]
            .info()
            .describe(Dialect::Python);
        assert_eq!(described["primary_key"], serde_json::json!(["id"]));
        Ok(())
    }

    // [spec:pgorm:req:bindings.entity-graphs+4/test]    a graph's slots are joined under
    // `g1`, `g2`, .. by default, a name the root's table takes skipped; an alias count
    // other than the slots', an alias repeated, or one naming the root's table is refused
    #[test]
    fn graph_aliases_default_and_are_checked() -> Result<(), Failure> {
        let source = |info: EntityInfo| super::super::graph::Source {
            entity: Arc::new(info),
            optional: false,
        };
        let three = GraphInfo {
            name: "app.Graph".to_owned(),
            shape: "",
            bindings: super::super::graph::GraphBindings {
                sources: vec![
                    source(EntityInfo::of("app.First", first::Entity)?),
                    source(EntityInfo::of("app.Widget", widget::Entity)?),
                    source(EntityInfo::of("app.Gadget", gadget::Entity)?),
                ],
            },
            dialect: Dialect::Python,
        };
        assert_eq!(
            three.aliases(None).ok(),
            Some(vec!["g2".to_owned(), "g3".to_owned()])
        );
        assert!(three.aliases(Some(vec!["a".to_owned()])).is_err());
        assert!(
            three
                .aliases(Some(vec!["a".to_owned(), "a".to_owned()]))
                .is_err()
        );
        assert!(
            three
                .aliases(Some(vec!["g1".to_owned(), "b".to_owned()]))
                .is_err()
        );
        assert!(
            three
                .aliases(Some(vec![String::new(), "b".to_owned()]))
                .is_err()
        );
        assert_eq!(
            three
                .aliases(Some(vec!["a".to_owned(), "b".to_owned()]))
                .ok(),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        Ok(())
    }
}
