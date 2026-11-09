//! The build-time collection of an application's concrete registrations,
//! pgorm-bindings' own, refusing as Rust's `RegistrationError`. Its module
//! owns it once installed; JavaScript cannot add a generic instantiation,
//! only use the ones compiled in.

use pgorm::{
    Disjoint, EntityTrait, GraphItem, GraphRow, IntoActiveModel, OneOf, SelectGraph, SelectorTrait,
    pipeline::SourceList,
};
use pgorm_bindings::{
    Failure,
    entity::{GraphSlots, SourceTypes},
};

use crate::connect::DIALECT;

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

/// The concrete Rust entities, graph shapes, source tuples, disjoint enums
/// and one-of enums one native module carries, which JavaScript reaches by
/// name through `entity()`, `graph()`, `pipeline.sources()`,
/// `disjointEntity()` and `oneOfEntity()`.
#[derive(Debug)]
pub struct Registry(pub(crate) pgorm_bindings::entity::Registry);

impl Default for Registry {
    fn default() -> Self {
        Self(pgorm_bindings::entity::Registry::new(DIALECT))
    }
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
        self.0.entity_in(name, entity)?;
        Ok(self)
    }

    /// Register a `SelectGraph` shape and the factory that builds it. The
    /// factory receives one alias per joined slot, in tuple order, and must
    /// join each slot under its alias; every source entity is registered
    /// first.
    // [spec:pgorm:req:bindings.entity-graphs+4]
    pub fn graph<E, S, Build>(
        &mut self,
        name: &str,
        build: Build,
    ) -> Result<&mut Self, RegistrationError>
    where
        E: EntityTrait + Send + Sync + 'static,
        S: GraphSlots<E>,
        GraphRow<E, S>: SelectorTrait + Send + Sync + 'static,
        GraphItem<E, S>: Send + 'static,
        Build: Fn(&[String]) -> SelectGraph<E, S> + Send + Sync + 'static,
    {
        self.0.graph(name, build)?;
        Ok(self)
    }

    /// Register a tuple of one to six entity types a pipeline's
    /// `select_sources` decodes rows as, each entity registered first.
    // [spec:pgorm:req:bindings.pipeline-sources+3]
    pub fn sources<T: SourceTypes>(&mut self, name: &str) -> Result<&mut Self, RegistrationError>
    where
        <T::Selection as SourceList>::Row: Send,
    {
        self.0.sources::<T>(name)?;
        Ok(self)
    }

    /// Register the `#[derive(Disjoint)]` enum `D` under `name`, its base and
    /// each kind's entity registered before it. A name or an enum registered
    /// twice, an entity not registered, and a declaration `check_disjoint`
    /// refuses are each refused, and nothing is added. An unkinded enum
    /// registers the same way and is read-only: its shape refuses every
    /// write and its schema.
    // [spec:pgorm:req:bindings.disjoint+2]
    // [spec:pgorm:req:bindings.disjoint-unkinded]
    pub fn disjoint<D: Disjoint + 'static>(
        &mut self,
        name: &str,
    ) -> Result<&mut Self, RegistrationError> {
        self.0.disjoint::<D>(name)?;
        Ok(self)
    }

    /// Register the `#[derive(OneOf)]` enum `O` under `name`, its entity and
    /// each target's registered before it. A name or an enum registered
    /// twice, and an entity not registered, are each refused, and nothing is
    /// added.
    // [spec:pgorm:req:bindings.one-of+1]
    pub fn one_of<O: OneOf>(&mut self, name: &str) -> Result<&mut Self, RegistrationError> {
        self.0.one_of::<O>(name)?;
        Ok(self)
    }
}
