//! A one-of reference as data: the entity's table, the column holding each
//! target's key, and each target's table and key column, which the `CHECK`,
//! a row's reference, a write of one and a load of its targets are built
//! from.
//!
//! [`OneOfShape::of`] describes a `#[derive(OneOf)]` enum and
//! [`OneOfShape::new`] tables known only at run time, as a language
//! binding's models are. [`OneOf`]'s methods and `Schema::create_one_of`
//! build theirs from [`OneOfShape::of`], so a binding's reference and a
//! derive's are one implementation.

use std::fmt;

use pgorm_query::{
    Check, Expr, Func, Name, SimpleExpr, Table, TableAlterStatement, TableName, Value,
};

use super::{column_casts::ColumnCasts, disjoint_shape::types};
use crate::{
    DisjointTable, EntityName, EntityTrait, Error, Iterable, OneOf, OneOfTarget,
    PrimaryKeyToColumn, Related, StaticName, TargetVisitor, error::query_err,
    executor::result_name::bounded_identifier, table_label,
};

/// One target of a [`OneOfShape`]: its value, the entity's column holding
/// its key, and the target's table and key column, a key read under that
/// column's casts.
// [spec:pgorm:def:disjoint.one-of+4]
#[derive(Clone)]
pub struct OneOfTargetShape {
    pub(crate) value: String,
    pub(crate) column: Name,
    pub(crate) table: TableName,
    pub(crate) label: String,
    pub(crate) key: Name,
    pub(crate) casts: ColumnCasts,
}

impl fmt::Debug for OneOfTargetShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OneOfTargetShape")
            .field("value", &self.value)
            .field("column", &self.column.to_string())
            .field("table", &self.label)
            .field("key", &self.key.to_string())
            .finish()
    }
}

impl OneOfTargetShape {
    /// The value naming the target: a derive's variant's name in
    /// snake_case.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The entity's column holding the target's key.
    pub fn column(&self) -> &Name {
        &self.column
    }

    /// The target's table, schema-qualified where it is.
    pub fn table(&self) -> &TableName {
        &self.table
    }

    /// The target's key column.
    pub fn key(&self) -> &Name {
        &self.key
    }
}

/// A reference from one entity's rows to exactly one of several targets, or
/// to at most one where it is optional, as data: the enum's name in
/// snake_case, the entity's table and key, and each target in declaration
/// order.
// [spec:pgorm:def:disjoint.one-of+4]
#[derive(Clone, Debug)]
pub struct OneOfShape {
    pub(crate) name: String,
    pub(crate) table: TableName,
    pub(crate) label: String,
    pub(crate) row_key: Vec<String>,
    pub(crate) targets: Vec<OneOfTargetShape>,
    pub(crate) optional: bool,
}

/// Describes each target for [`OneOfShape::of`].
struct Described(Vec<OneOfTargetShape>);

impl<O: OneOf> TargetVisitor<O> for Described {
    fn target<T: OneOfTarget<O>>(&mut self)
    where
        O::Entity: Related<T>,
    {
        let target = T::default();
        // The derive binds the column as an `Option` of the target's key
        // type, which a column holds only for a key of one column.
        let Some(key) = <T::PrimaryKey as Iterable>::iter().next() else {
            return;
        };
        let key = key.into_column();
        self.0.push(OneOfTargetShape {
            value: T::NAME.to_owned(),
            column: Name::new(T::COLUMN),
            table: target.table_ref(),
            label: table_label(&target),
            key: Name::new(key),
            casts: ColumnCasts::of(key),
        });
    }
}

impl OneOfShape {
    /// The reference `O` declares: its entity's table and key, and each
    /// target in declaration order, named as [`OneOfTarget::NAME`] names it.
    // [spec:pgorm:def:disjoint.one-of+4]
    pub fn of<O: OneOf>() -> Self {
        let entity = <O::Entity as Default>::default();
        let mut described = Described(Vec::new());
        O::visit_targets(&mut described);
        Self {
            name: O::name().to_owned(),
            table: entity.table_ref(),
            label: table_label(&entity),
            row_key: <<O::Entity as EntityTrait>::PrimaryKey as Iterable>::iter()
                .map(|key| key.into_column().as_str().to_owned())
                .collect(),
            targets: described.0,
            optional: O::none().is_some(),
        }
    }

    /// The reference `name` from `entity`'s rows to `targets`, each its value,
    /// the column of `entity` holding its key, and its table, keyed by one
    /// column; a row may reference none where `optional` says so.
    ///
    /// What a `#[derive(OneOf)]` does not compile is refused as
    /// [`Error::Type`]: a name past 59 bytes, no target, a value given twice,
    /// a column `entity` lacks, one that is not nullable, one named for two
    /// targets, a target keyed by other than one column or by another type
    /// than its column's, and one table the target of two values.
    // [spec:pgorm:def:disjoint.one-of+4]
    // [spec:pgorm:req:bindings.one-of+1]
    pub fn new(
        name: &str,
        entity: &DisjointTable,
        targets: &[(&str, &str, &DisjointTable)],
        optional: bool,
    ) -> Result<Self, Error> {
        let label = &entity.label;
        if name.is_empty() || name.len() > 59 {
            return Err(Error::Type(format!(
                "the one-of reference over `{label}` is named `{name}`, and its name is 1 to 59 \
                 bytes, so that the CHECK named for it keeps it whole within PostgreSQL's 63"
            )));
        }
        if targets.is_empty() {
            return Err(Error::Type(format!(
                "the one-of reference `{name}` over `{label}` names no target: it names at \
                 least one"
            )));
        }
        let mut described: Vec<OneOfTargetShape> = Vec::with_capacity(targets.len());
        for (value, column, target) in targets {
            if let Some(other) = described.iter().find(|other| other.value == *value) {
                return Err(Error::Type(format!(
                    "the one-of reference `{name}` over `{label}` names the target `{}` twice",
                    other.value
                )));
            }
            let Some((_, held)) = entity.column(column) else {
                return Err(Error::Type(format!(
                    "`{label}` has no column `{column}` to hold the key of the target `{value}`"
                )));
            };
            if let Some(other) = described
                .iter()
                .find(|other| other.column.to_string() == *column)
            {
                return Err(Error::Type(format!(
                    "`{label}`.`{column}` holds the keys of the targets `{}` and `{value}`: each \
                     target's key is held in a column of its own",
                    other.value
                )));
            }
            if !held.def.is_null() {
                return Err(Error::Type(format!(
                    "`{label}`.`{column}` is not nullable, and a one-of reference's column is \
                     null in every row referencing another target"
                )));
            }
            if let Some(other) = described.iter().find(|other| other.label == target.label) {
                return Err(Error::Type(format!(
                    "`{}` is the target of `{}` and of `{value}`: each value names a target of \
                     its own",
                    target.label, other.value
                )));
            }
            let mut keys = target.key();
            let (Some(key), None) = (keys.next(), keys.next()) else {
                return Err(Error::Type(format!(
                    "`{}`, the target `{value}`, is keyed by {} columns, and a one-of \
                     reference's column holds a key of one",
                    target.label,
                    target.key().count()
                )));
            };
            let (held_type, key_type) = (held.def.get_column_type(), key.def.get_column_type());
            if held_type != key_type {
                return Err(Error::Type(format!(
                    "`{label}`.`{column}` is ({}), not ({}), the key of `{}`, the target \
                     `{value}`",
                    types(&[held_type]),
                    types(&[key_type]),
                    target.label
                )));
            }
            described.push(OneOfTargetShape {
                value: (*value).to_owned(),
                column: Name::clone(&held.name),
                table: target.table.clone(),
                label: target.label.clone(),
                key: Name::clone(&key.name),
                casts: key.casts.clone(),
            });
        }
        Ok(Self {
            name: name.to_owned(),
            table: entity.table.clone(),
            label: label.clone(),
            row_key: entity.key().map(|column| column.name.to_string()).collect(),
            targets: described,
            optional,
        })
    }

    /// The reference's name, the enum's in snake_case.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The entity's table, schema-qualified where it is.
    pub fn table(&self) -> &TableName {
        &self.table
    }

    /// The targets, in declaration order.
    pub fn targets(&self) -> &[OneOfTargetShape] {
        &self.targets
    }

    /// Whether a row may reference none of the targets.
    pub fn is_optional(&self) -> bool {
        self.optional
    }

    /// The target named `value`, and where it stands, or [`Error::Type`]
    /// naming the reference's targets.
    // [spec:pgorm:def:disjoint.one-of+4]
    pub fn target(&self, value: &str) -> Result<(usize, &OneOfTargetShape), Error> {
        self.targets
            .iter()
            .enumerate()
            .find(|(_, target)| target.value == value)
            .ok_or_else(|| {
                Error::Type(format!(
                    "`{value}` is no target of the one-of reference `{}` over `{}`, whose \
                     targets are {}",
                    self.name,
                    self.label,
                    spelled(self.targets.iter().map(|target| target.value.as_str()))
                ))
            })
    }

    /// The `ALTER TABLE` adding the `CHECK` that holds every row to one
    /// target, or to one at most where the reference is optional, as
    /// [`Schema::create_one_of`](crate::Schema::create_one_of) describes it.
    // [spec:pgorm:def:disjoint.one-of+4]
    pub fn check(&self) -> TableAlterStatement {
        let set = self.targets.iter().fold(
            Func::named(Name::runtime("num_nonnulls")),
            |call, target| call.arg(Expr::col(Name::clone(&target.column))),
        );
        let check = match self.optional {
            true => Expr::expr(set).lte(1),
            false => Expr::expr(set).eq(1),
        };
        let short = format!("ck-{}-{}", self.table.table().to_string(), self.name);
        let name = if bounded_identifier(short.clone()) == short {
            short
        } else {
            bounded_identifier(format!("ck-{}", self.name))
        };
        Table::alter(self.table.clone()).add_check(Check::new(check).name(Name::runtime(name)))
    }

    /// The target a row references, given, for each target in order,
    /// whether its column is set, or `None` for a row referencing none where
    /// the reference is optional; `row` is the row's key.
    ///
    /// A row setting two or more columns, or none where the reference is not
    /// optional, contradicts the declaration, and is [`Error::Query`] naming
    /// the table, the row's key and the columns.
    // [spec:pgorm:def:disjoint.one-of+4]
    pub fn referenced(&self, row: &[Value], set: &[bool]) -> Result<Option<usize>, Error> {
        let columns = || spelled(self.targets.iter().map(|target| target.column.to_string()));
        let mut referenced = set
            .iter()
            .enumerate()
            .filter(|(_, set)| **set)
            .map(|(index, _)| index);
        match (referenced.next(), referenced.next()) {
            (Some(index), None) => Ok(Some(index)),
            (None, _) if self.optional => Ok(None),
            (None, _) => Err(query_err(format!(
                "{} references no target, setting none of {}, where exactly one is set",
                self.row(row),
                columns(),
            ))),
            (Some(first), Some(second)) => {
                let named = [first, second]
                    .into_iter()
                    .chain(referenced)
                    .filter_map(|index| self.targets.get(index))
                    .map(|target| target.column.to_string());
                Err(query_err(format!(
                    "{} references more than one target, setting {}, where at most one of {} \
                     is set",
                    self.row(row),
                    spelled(named),
                    columns(),
                )))
            }
        }
    }

    /// For each target in order, whether a write of a reference to `target`
    /// sets its column to the key, every other being set to null; `None`
    /// references none, which [`Error::Type`] refuses where the reference is
    /// not optional.
    // [spec:pgorm:def:disjoint.one-of+4]
    pub fn assigned(&self, target: Option<usize>) -> Result<Vec<bool>, Error> {
        if target.is_none() && !self.optional {
            return Err(Error::Type(format!(
                "the one-of reference `{}` over `{}` is not optional: a row references one of \
                 its targets",
                self.name, self.label
            )));
        }
        Ok((0..self.targets.len())
            .map(|index| Some(index) == target)
            .collect())
    }

    /// What a load reads the rows of the target at `target` by: its key
    /// among `keys`, each compared under the key column's casts, so the
    /// target's table is read once for every row referencing it.
    // [spec:pgorm:def:disjoint.one-of+4]
    pub fn load_condition(&self, target: usize, keys: Vec<Value>) -> Result<SimpleExpr, Error> {
        let target = self.target_at(target)?;
        let column = target.casts.compared_column(Expr::col((
            Name::clone(target.table.table()),
            Name::clone(&target.key),
        )));
        Ok(column.is_in(keys.into_iter().map(|key| target.casts.compared(key))))
    }

    /// The error for the row whose key is `row`, which references the row
    /// of key `key` of the target at `target`, a row its table does not hold.
    // [spec:pgorm:def:disjoint.one-of+4]
    pub fn absent(&self, target: usize, row: &[Value], key: &Value) -> Error {
        let Some(shape) = self.targets.get(target) else {
            return query_err(format!(
                "{} references a target the reference lacks",
                self.row(row)
            ));
        };
        query_err(format!(
            "{} references the `{}` row of key {key} through `{}`, and `{}` holds no such row",
            self.row(row),
            shape.label,
            shape.column.to_string(),
            shape.label,
        ))
    }

    fn target_at(&self, target: usize) -> Result<&OneOfTargetShape, Error> {
        self.targets.get(target).ok_or_else(|| {
            Error::Type(format!(
                "the one-of reference `{}` over `{}` has {} targets, not one at {target}",
                self.name,
                self.label,
                self.targets.len()
            ))
        })
    }

    /// The row whose key is `row`, as an error names it.
    fn row(&self, row: &[Value]) -> String {
        let key: Vec<String> = self
            .row_key
            .iter()
            .zip(row)
            .map(|(column, value)| format!("`{column}` = {value}"))
            .collect();
        format!("the `{}` row whose key is ({})", self.label, key.join(", "))
    }
}

/// Names as an error lists them, each in backquotes.
fn spelled<S: AsRef<str>>(names: impl Iterator<Item = S>) -> String {
    names
        .map(|name| format!("`{}`", name.as_ref()))
        .collect::<Vec<_>>()
        .join(", ")
}
