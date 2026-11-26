//! The columns a row is given when it is inserted and keeps: each one's
//! definition is `immutable` ([`ColumnDef::immutable`](crate::ColumnDef::immutable)),
//! and every update, upsert and disjoint write is held to it here.

use crate::{ActiveModel, ActiveValue, Column, Entity, Error, Iterable, StaticName};
use pgorm_query::{ConflictAction, ConflictAssignment, OnConflict, UpdateStatement};

/// Refuse an `ON CONFLICT .. DO UPDATE SET` assigning a column of `E` its
/// definition marks immutable, which an update of the row it conflicted with
/// would change.
// [spec:pgorm:req:scope.immutable]
pub(crate) fn immutable_on_conflict<E: Entity>(
    on_conflict: Option<&OnConflict>,
) -> Result<(), Error> {
    let Some(OnConflict::Targeted {
        action: ConflictAction::Update { sets, .. },
        ..
    }) = on_conflict
    else {
        return Ok(());
    };
    for assignment in sets.iter() {
        let name = match assignment {
            ConflictAssignment::Column(name) | ConflictAssignment::Expr(name, _) => name,
            _ => continue,
        };
        immutable_column::<E>(&name.to_string())?;
    }
    Ok(())
}

/// Refuse `name` where it names a column of `E` its definition marks
/// immutable.
// [spec:pgorm:req:scope.immutable]
pub(crate) fn immutable_column<E: Entity>(name: &str) -> Result<(), Error> {
    let immutable = E::Column::iter().any(|col| col.as_str() == name && col.def().is_immutable());
    if immutable {
        return Err(Error::ImmutableColumn {
            table: E::default().table_name().to_owned(),
            column: name.to_owned(),
        });
    }
    Ok(())
}

/// Refuse an update whose `SET` names a column of `E` its definition marks
/// immutable.
// [spec:pgorm:req:scope.immutable]
pub(crate) fn immutable_set<E: Entity>(query: &UpdateStatement) -> Result<(), Error> {
    for (name, _) in query.get_values() {
        immutable_column::<E>(&name.to_string())?;
    }
    Ok(())
}

// [spec:pgorm:sem:query.build.update+6]
/// Refuse a row of `E` an update writes that sets a column its definition
/// marks immutable.
// [spec:pgorm:req:scope.immutable]
pub(crate) fn immutable_unset<E: Entity>(row: &E::ActiveModel) -> Result<(), Error> {
    for column in E::Column::iter() {
        if column.def().is_immutable() && matches!(row.get(column), Ok(ActiveValue::Set(_))) {
            return Err(Error::ImmutableColumn {
                table: E::default().table_name().to_owned(),
                column: column.as_str().to_owned(),
            });
        }
    }
    Ok(())
}
