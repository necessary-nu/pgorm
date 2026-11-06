use crate::{ActiveEnum, CreatedRangeType, Entity, KindType};
use std::collections::BTreeMap;

/// The tables a schema creates, read as every writer reads them: an entity
/// per table, in table-name order, and the enums, range types and disjoint
/// entities' kind types their columns name, each keyed by its qualified name.
///
/// [`EntityTransformer::entities`](crate::EntityTransformer::entities) reads
/// it and refuses what no writer could write; each writer refuses besides
/// what its own language cannot spell.
#[derive(Clone, Debug)]
pub struct Entities {
    pub(crate) entities: Vec<Entity>,
    /// Every enum a column names, a disjoint entity's kind type among them.
    pub(crate) enums: BTreeMap<String, ActiveEnum>,
    pub(crate) ranges: BTreeMap<String, CreatedRangeType>,
    /// Each disjoint entity's kind type, keyed as `enums` keys it, which the
    /// Rust writer's disjoint derive declares.
    #[cfg_attr(not(feature = "rust"), allow(dead_code))]
    pub(crate) kind_types: BTreeMap<String, KindType>,
}
