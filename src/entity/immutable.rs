//! The columns a row is given when it is inserted and keeps: each one's
//! definition is `immutable` ([`ColumnDef::immutable`](crate::ColumnDef::immutable)),
//! and every update and upsert of an entity is held to it here. A disjoint
//! shape holds its own writes to its columns' definitions.

use crate::{Column, Entity, Error, Iterable, StaticName};
use pgorm_query::{OnConflict, UpdateStatement};

/// Refuse an `ON CONFLICT .. DO UPDATE SET` assigning a column of `E` its
/// definition marks immutable, which an update of the row it conflicted with
/// would change.
// [spec:pgorm:req:entity.immutable.writes]
pub(crate) fn immutable_on_conflict<E: Entity>(
    on_conflict: Option<&OnConflict>,
) -> Result<(), Error> {
    for name in on_conflict
        .into_iter()
        .flat_map(OnConflict::updated_columns)
    {
        immutable_named::<E>(&name.to_string())?;
    }
    Ok(())
}

/// Refuse an update whose `SET` names a column of `E` its definition marks
/// immutable.
// [spec:pgorm:req:entity.immutable.writes]
pub(crate) fn immutable_set<E: Entity>(query: &UpdateStatement) -> Result<(), Error> {
    for (name, _) in query.get_values() {
        immutable_named::<E>(&name.to_string())?;
    }
    Ok(())
}

// [spec:pgorm:req:entity.immutable.writes]
pub(crate) fn immutable<E: Entity>(column: E::Column) -> Result<(), Error> {
    if column.def().is_immutable() {
        return Err(Error::ImmutableColumn {
            table: E::default().table_name().to_owned(),
            column: column.as_str().to_owned(),
        });
    }
    Ok(())
}

/// A name that is none of `E`'s columns is the server's to refuse.
fn immutable_named<E: Entity>(name: &str) -> Result<(), Error> {
    match E::Column::iter().find(|column| column.as_str() == name) {
        Some(column) => immutable::<E>(column),
        None => Ok(()),
    }
}
