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
/// An entity is written as one `Model` struct under `DeriveEntityModel`, which
/// writes the `Entity`, its `Column` and `PrimaryKey` enums and the
/// `ActiveModel` from it, and a `Relation` enum under `DeriveRelation`.
/// ```
/// # use pgorm::entity::prelude::*;
/// #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
/// #[pgorm(table_name = "filling")]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub name: String,
/// }
///
/// #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
/// pub enum Relation {}
///
/// // Implement user defined operations for CREATE, UPDATE and DELETE operations
/// // to create an ActiveModel using the [ActiveModelBehavior]
/// impl ActiveModelBehavior for ActiveModel {}
/// ```
mod active_enum;
mod active_model;
mod base_entity;
mod column;
mod created_range;
mod key;
mod link;
mod model;
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
pub use key::*;
pub use link::*;
pub use model::*;
pub use partial_model::*;
// pub use prelude::*;
pub use primary_key::*;
pub use relation::*;
