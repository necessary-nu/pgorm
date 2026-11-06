//! The build-time collection of an application's concrete registrations.
//! Its module owns it once installed; JavaScript cannot add a generic
//! instantiation, only use the ones compiled in.

use std::{
    any::TypeId,
    collections::{BTreeMap, HashMap, HashSet},
    marker::PhantomData,
    sync::Arc,
};

use pgorm::{EntityTrait, GraphItem, GraphRow, IntoActiveModel, SelectGraph, SelectorTrait};

use super::{
    adapter::{EntityAdapter, EntityBackend},
    graph::{Factory, GraphFactory, GraphInfo, GraphSlots},
    info::{EntityInfo, registration_name},
};
use crate::errors::Failure;

/// Why a registration was refused: a duplicate name or Rust type, a name
/// out of bounds, a graph whose sources are not registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationError(pub String);

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RegistrationError {}

impl From<Failure> for RegistrationError {
    fn from(failure: Failure) -> Self {
        Self(failure.message().to_owned())
    }
}

/// The concrete Rust entities and graph shapes one native module carries,
/// which JavaScript reaches by name through `entity()` and `graph()`.
#[derive(Debug, Default)]
pub struct Registry {
    pub(crate) entities: BTreeMap<String, Arc<dyn EntityBackend>>,
    /// The first registration of each Rust entity type, which a graph or a
    /// source tuple over the type reads its metadata from.
    types: HashMap<TypeId, String>,
    /// Every registered type with the table its instance names, so one table
    /// is not registered twice under one type.
    tables: HashSet<(TypeId, Option<String>, String)>,
    pub(crate) graphs: BTreeMap<String, Arc<dyn Factory>>,
    pub(crate) sources: BTreeMap<String, Arc<dyn super::sources::SourcesFactory>>,
    pub(crate) disjoints: crate::disjoint::Registered,
    pub(crate) one_ofs: crate::one_of::Registered,
}

impl Registry {
    /// Register an entity's real `EntityTrait`, model, columns and
    /// ActiveModel under `name`, its statements naming `E::default()`'s
    /// table. A name or a Rust type registered twice is refused, and nothing
    /// is added.
    // [spec:pgorm:req:bindings.entities+6]
    // [spec:pgorm:req:napi.application+2]
    pub fn entity<E>(&mut self, name: &str) -> Result<&mut Self, RegistrationError>
    where
        E: EntityTrait + Send + Sync + 'static,
        E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static,
        E::ActiveModel: Send + Sync + 'static,
    {
        self.entity_in(name, E::default())
    }

    /// Register `entity`, an instance of an entity whose names are computed
    /// at run time, under `name`: every statement of the registration names
    /// the instance's table, as Rust's `_in` constructors do. A type may be
    /// registered once for each table its instances name; a name, or a type
    /// and table already registered, is refused, and nothing is added.
    // [spec:pgorm:req:bindings.entities+6]
    pub fn entity_in<E>(&mut self, name: &str, entity: E) -> Result<&mut Self, RegistrationError>
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
            return Err(RegistrationError(format!(
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
        Ok(self)
    }

    /// Register a `SelectGraph` shape and the factory that builds it. The
    /// factory receives one alias per joined slot, in tuple order, and must
    /// join each slot under its alias; every source entity is registered
    /// first.
    // [spec:pgorm:req:bindings.entity-graphs+3]
    pub fn graph<E, S, Build>(
        &mut self,
        name: &str,
        build: Build,
    ) -> Result<&mut Self, RegistrationError>
    where
        E: EntityTrait + Send + Sync + 'static,
        S: GraphSlots<E>,
        GraphRow<E, S>: SelectorTrait,
        GraphItem<E, S>: Send + 'static,
        Build: Fn(&[String]) -> SelectGraph<E, S> + Send + Sync + 'static,
    {
        registration_name(name)?;
        if self.graphs.contains_key(name) {
            return Err(RegistrationError(format!(
                "the graph {name:?} is already registered"
            )));
        }
        let info = Arc::new(GraphInfo {
            name: name.to_owned(),
            shape: std::any::type_name::<SelectGraph<E, S>>(),
            bindings: S::bindings(self)?,
        });
        self.graphs.insert(
            name.to_owned(),
            Arc::new(GraphFactory::<E, S, Build> {
                info,
                build,
                marker: PhantomData,
            }),
        );
        Ok(self)
    }

    /// The registered entity `E`, which a disjoint enum's base or kind is.
    pub(crate) fn backend<E: EntityTrait + 'static>(
        &self,
    ) -> Result<Arc<dyn EntityBackend>, RegistrationError> {
        self.backend_of(TypeId::of::<E>()).ok_or_else(|| {
            RegistrationError(format!(
                "register the entity {} before the disjoint entity over it",
                std::any::type_name::<E>()
            ))
        })
    }

    /// The registered entity whose Rust type is `entity`.
    pub(crate) fn backend_of(&self, entity: TypeId) -> Option<Arc<dyn EntityBackend>> {
        self.types
            .get(&entity)
            .and_then(|name| self.entities.get(name))
            .cloned()
    }

    /// The registration of `E`, which a graph's sources name.
    pub(crate) fn registered<E: EntityTrait + 'static>(
        &self,
    ) -> Result<Arc<EntityInfo>, RegistrationError> {
        self.types
            .get(&TypeId::of::<E>())
            .and_then(|name| self.entities.get(name))
            .map(|entity| entity.info().clone())
            .ok_or_else(|| {
                RegistrationError(format!(
                    "register the entity {} before a graph of it",
                    std::any::type_name::<E>()
                ))
            })
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
    fn registrations_refuse_duplicates_and_unregistered_sources() -> Result<(), RegistrationError> {
        let mut registry = Registry::default();
        registry.entity::<widget::Entity>("app.Widget")?;
        assert!(registry.entity::<gadget::Entity>("app.Widget").is_err());
        assert!(registry.entity::<widget::Entity>("again.Widget").is_err());
        assert!(registry.entity::<gadget::Entity>("").is_err());
        assert!(registry.entity::<gadget::Entity>(&"x".repeat(256)).is_err());
        assert_eq!(registry.entities.len(), 1);
        assert!(
            registry
                .graph::<gadget::Entity, (), _>("app.Gadgets", |_| gadget::Entity::graph())
                .is_err()
        );
        registry.entity::<gadget::Entity>("app.Gadget")?;
        registry.graph::<gadget::Entity, (), _>("app.Gadgets", |_| gadget::Entity::graph())?;
        assert!(
            registry
                .graph::<gadget::Entity, (), _>("app.Gadgets", |_| gadget::Entity::graph())
                .is_err()
        );
        let described = registry.entities["app.Widget"]
            .info()
            .describe()
            .unwrap_or_else(|failure| panic!("{failure}"));
        assert_eq!(described["schema"], "app");
        assert_eq!(described["primaryKey"], serde_json::json!(["id"]));
        assert_eq!(described["columns"][1]["nullable"], true);
        assert_eq!(described["columns"][1]["kind"], "text");
        Ok(())
    }
}
