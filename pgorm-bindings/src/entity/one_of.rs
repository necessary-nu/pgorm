//! One-of references over pgorm's [`OneOfShape`]: a reference declared over
//! models a program describes, or a downstream crate's registered
//! `#[derive(OneOf)]` enum.
//!
//! Either is one [`OneOfCore`], the shape and its description. Every
//! decision about a reference is the shape's: which target a row
//! references, which columns a write sets, the condition a target's rows are
//! loaded by, the error for a key its target does not hold, and the `CHECK`.

use std::{any::TypeId, fmt, sync::Arc};

use pgorm::pgorm_query::{SimpleExpr, TableAlterStatement, Value};
use pgorm::{
    DisjointTable, EntityTrait, Error, OneOf, OneOfShape, OneOfTarget, Related, TargetVisitor,
};
use serde_json::{Value as Json, json};

use super::{Registry, registration_name};
use crate::Failure;

/// One reference or registration: its shape and its description.
pub struct OneOfCore {
    /// The shape, which makes every decision about the reference.
    pub shape: OneOfShape,
    /// What `describe()` reports.
    pub description: Json,
    /// The registered enum's type, which no second registration may name.
    rust: Option<TypeId>,
}

impl fmt::Debug for OneOfCore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OneOfCore")
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}

/// One target of a reference declared over described models: its value, the
/// field holding its key, that field's column, and its model's name and
/// table.
#[derive(Debug)]
#[allow(missing_docs)]
pub struct DescribedTarget {
    pub value: String,
    pub field: String,
    pub column: String,
    pub model: String,
    pub table: DisjointTable,
}

/// What the shape refuses as a reference is used: a row against its
/// declaration, read back, is a `DecodeError`, and anything else, refused
/// before anything is sent, a `ConstructionError`.
fn refused(error: Error) -> Failure {
    match error {
        Error::Query(_) => Failure::Decode(error.to_string()),
        error => Failure::Construction(error.to_string()),
    }
}

// [spec:pgorm:req:bindings.one-of+1]
impl OneOfCore {
    /// The reference `name` from the described `model`, its name beside its
    /// table, to each target in order. What the shape refuses is refused.
    pub fn model(
        name: &str,
        model: (String, DisjointTable),
        targets: Vec<DescribedTarget>,
        optional: bool,
    ) -> Result<Self, Failure> {
        let (model_name, entity) = model;
        let described: Vec<Json> = targets
            .iter()
            .map(|target| {
                json!({"value": target.value, "field": target.field, "target": target.model})
            })
            .collect();
        let named: Vec<(&str, &str, &DisjointTable)> = targets
            .iter()
            .map(|target| (target.value.as_str(), target.column.as_str(), &target.table))
            .collect();
        let shape = OneOfShape::new(name, &entity, &named, optional)
            .map_err(|error| Failure::Construction(error.to_string()))?;
        Ok(Self {
            shape,
            description: json!({
                "name": name,
                "model": model_name,
                "targets": described,
                "optional": optional,
            }),
            rust: None,
        })
    }

    /// Where the target `value` stands among the reference's.
    pub fn target(&self, value: &str) -> Result<usize, Failure> {
        self.shape
            .target(value)
            .map(|(index, _)| index)
            .map_err(refused)
    }

    /// The target a row references, given its key and, for each target,
    /// whether its column is set, or `None` for a row referencing none.
    pub fn referenced(&self, row: &[Value], set: &[bool]) -> Result<Option<usize>, Failure> {
        self.shape.referenced(row, set).map_err(refused)
    }

    /// For each target, whether a write of a reference to `target` sets its
    /// column to the key, the rest null.
    pub fn assigned(&self, target: Option<usize>) -> Result<Vec<bool>, Failure> {
        self.shape.assigned(target).map_err(refused)
    }

    /// The condition the rows of the target at `target` are loaded by.
    pub fn load_condition(&self, target: usize, keys: Vec<Value>) -> Result<SimpleExpr, Failure> {
        self.shape.load_condition(target, keys).map_err(refused)
    }

    /// The `DecodeError` for the row of key `row` referencing a row of the
    /// target at `target`, of key `key`, which its table does not hold.
    pub fn absent(&self, target: usize, row: &[Value], key: &Value) -> Failure {
        Failure::Decode(self.shape.absent(target, row, key).to_string())
    }

    /// The `ALTER TABLE` adding the reference's `CHECK`.
    pub fn check(&self) -> TableAlterStatement {
        self.shape.check()
    }
}

/// Each target's registration name, in declaration order, or the first
/// target not registered.
struct Targets<'r> {
    registry: &'r Registry,
    names: Result<Vec<String>, Failure>,
}

impl<O: OneOf> TargetVisitor<O> for Targets<'_> {
    fn target<T: OneOfTarget<O>>(&mut self)
    where
        O::Entity: Related<T>,
    {
        if let Ok(names) = &mut self.names {
            match registered_name::<T>(self.registry) {
                Ok(name) => names.push(name),
                Err(failure) => self.names = Err(failure),
            }
        }
    }
}

/// The name the entity `E` is registered under, or a refusal naming it.
fn registered_name<E: EntityTrait>(registry: &Registry) -> Result<String, Failure> {
    registry
        .backend::<E>("the one-of reference over it")
        .map(|backend| backend.info().name.clone())
}

impl Registry {
    /// Register the `#[derive(OneOf)]` enum `O` under `name`, its entity and
    /// each target's registered before it. A name or an enum registered
    /// twice, and an entity not registered, are each refused, and nothing is
    /// added.
    // [spec:pgorm:req:bindings.one-of+1]
    // [spec:pgorm:req:bindings.entities+6]
    pub fn one_of<O: OneOf>(&mut self, name: &str) -> Result<(), Failure> {
        registration_name(name)?;
        let rust = TypeId::of::<O>();
        if self.one_ofs.contains_key(name)
            || self.one_ofs.values().any(|core| core.rust == Some(rust))
        {
            return Err(Failure::Construction(format!(
                "{name:?} or its Rust one-of enum is already registered"
            )));
        }
        let entity = registered_name::<O::Entity>(self)?;
        let mut targets = Targets {
            registry: self,
            names: Ok(Vec::new()),
        };
        O::visit_targets(&mut targets);
        let targets = targets.names?;
        let shape = OneOfShape::of::<O>();
        let description = json!({
            "name": name,
            "model": entity,
            "targets": shape
                .targets()
                .iter()
                .zip(&targets)
                .map(|(target, registered)| json!({
                    "value": target.value(),
                    "field": target.column().to_string(),
                    "target": registered,
                }))
                .collect::<Vec<_>>(),
            "optional": shape.is_optional(),
        });
        self.one_ofs.insert(
            name.to_owned(),
            Arc::new(OneOfCore {
                shape,
                description,
                rust: Some(rust),
            }),
        );
        Ok(())
    }
}
