//! A declared model's foreign keys, as pgorm's `TableShape` writes them.

use pgorm::{
    ColumnPairs, RelationDef, TableShape,
    pgorm_query::{ForeignKeyAction, Name, TableName},
};

use crate::Failure;

/// One `belongs_to` relation of a declared model, as a binding read it: the
/// columns it pairs at each end, the table it reaches, its actions and its
/// constraint's name.
#[derive(Debug)]
pub struct BelongsTo {
    /// The model's own columns the key holds, in order.
    pub from: Vec<String>,
    /// The columns of `target` they reference, in the same order.
    pub to: Vec<String>,
    /// The table the key references.
    pub target: TableName,
    /// The key's `ON DELETE` action, or none for PostgreSQL's default.
    pub on_delete: Option<ForeignKeyAction>,
    /// The key's `ON UPDATE` action, or none for PostgreSQL's default.
    pub on_update: Option<ForeignKeyAction>,
    /// The constraint's name, or none for the one pgorm gives it.
    pub fk_name: Option<String>,
}

/// `shape` with `relation`'s foreign key, labelled in what pgorm refuses by
/// the table it reaches and the columns it pairs there.
// [spec:pgorm:req:bindings.schema-from-model]
pub fn with_relation(shape: TableShape, relation: BelongsTo) -> Result<TableShape, Failure> {
    let (Some(first_from), Some(first_to)) = (relation.from.first(), relation.to.first()) else {
        return Err(Failure::Construction(
            "a relation pairs at least one column".to_owned(),
        ));
    };
    let mut pairs = ColumnPairs::new(Name::runtime(first_from), Name::runtime(first_to));
    for (from, to) in relation.from.iter().zip(&relation.to).skip(1) {
        pairs.push(Name::runtime(from), Name::runtime(to));
    }
    let label = format!(
        "{}({})",
        relation.target.table().to_string(),
        relation.from.join(", ")
    );
    let mut def = RelationDef::belongs_to(shape.table().clone(), relation.target, pairs);
    def.on_delete = relation.on_delete;
    def.on_update = relation.on_update;
    def.fk_name = relation.fk_name;
    shape
        .with_relation(label, def)
        .map_err(|error| Failure::Construction(error.to_string()))
}
