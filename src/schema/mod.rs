mod disjoint;
mod entity;
mod statement;

pub(crate) use entity::table_from_entity;
pub use statement::DdlStatement;

/// This is a helper struct to convert [`EntityTrait`](crate::EntityTrait)
/// into different [`pgorm_query`] statements.
#[derive(Debug)]
pub struct Schema {}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

impl Schema {
    /// Create a helper for a specific database backend
    pub fn new() -> Self {
        Self {}
    }
}
