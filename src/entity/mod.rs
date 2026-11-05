/// This modules contains types and traits for an  Entity, ActiveMode, Model, PrimaryKey, ForeignKey and Relations.
///
/// // An Entity
/// A unit struct implements [EntityTrait](crate::EntityTrait) representing a table in the database.
///
/// This trait contains the properties of an entity including
///
/// - The Table Name which is implemented by [EntityName](crate::EntityName)
/// - The Column which is implemented by [ColumnTrait](crate::ColumnTrait)
/// - A Relation which is implemented by [RelationTrait](crate::RelationTrait)
/// - The Primary Key which is implemented by [PrimaryKeyTrait](crate::PrimaryKeyTrait)
///   and [PrimaryKeyToColumn](crate::PrimaryKeyToColumn)
///
/// This trait also provides an API for CRUD actions
///
/// #### Example for creating an Entity, Model and ActiveModel
///
/// An entity is written as one `Model` struct under `#[derive(Entity)]`,
/// which writes the `Entity`, its `Column` and `PrimaryKey` enums and the
/// `ActiveModel` from it, and a `Relation` enum under `#[derive(Relation)]`.
/// ```
/// # use pgorm::entity::prelude::*;
/// #[derive(Clone, Debug, PartialEq, Entity)]
/// #[pgorm(table_name = "filling")]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub name: String,
/// }
///
/// #[derive(Copy, Clone, Debug, Relation)]
/// pub enum Relation {}
/// ```
///
/// The `Entity` derive implements [ActiveModelBehavior](crate::ActiveModelBehavior)
/// with no hooks, unless the `Model` says `#[pgorm(hooks)]` and the module
/// implements it.
mod active_enum;
mod active_model;
mod base_entity;
mod column;
mod column_casts;
mod created_range;
mod disjoint;
mod disjoint_shape;
mod key;
mod link;
mod model;
mod one_of;
mod one_of_shape;
mod partial_model;
/// Re-export common types from the entity
pub mod prelude;
mod primary_key;
mod relation;

pub use active_enum::*;
pub use active_model::*;
pub use base_entity::*;
pub use column::*;
pub use created_range::*;
pub use disjoint::*;
pub use disjoint_shape::*;
pub use key::*;
pub use link::*;
pub use model::*;
pub use one_of::*;
pub use one_of_shape::*;
pub use partial_model::*;
// pub use prelude::*;
pub use primary_key::*;
pub use relation::*;

/// The derives of the `SqlName` and `StaticName` traits `base_entity`'s glob
/// brings. pgorm-query's derives of the same names ride in with the traits and
/// expand to `pgorm_query::..` paths, which a crate depending only on pgorm
/// cannot resolve; an explicit import shadows a glob's in the macro namespace
/// alone, so these name the traits while pgorm-macros' derives, expanding to
/// `pgorm::pgorm_query::..`, are the derives.
// [spec:pgorm:req:macros.derive.names+2]
// [spec:pgorm:sem:macros.derive.iden+3]
#[cfg(feature = "macros")]
pub use pgorm_macros::{SqlName, StaticName};
