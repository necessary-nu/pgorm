//! A table as data: the columns, keys, relations and comment every
//! statement [`Schema`](crate::Schema) creates a table with is written from.
//!
//! [`TableShape::of`] describes an entity, as
//! [`create_table_from_entity`](crate::Schema::create_table_from_entity) and
//! its siblings read it; [`TableShape::new`] describes a table known only at
//! run time, as a language binding's models are. Either way the statements
//! come from the same code.

use std::fmt;

use pgorm_query::{Expr, ExprTrait, GeneratedKind, Name, SimpleExpr, SqlName, TableName, TypeName};

use super::{
    ColumnDefault, cast_of, compared_enum, compared_value_as, save_array_cast, save_cast,
    select_cast,
};
use crate::{
    ColumnDef, ColumnTrait, DisjointTable, EntityTrait, Error, Iterable, PrimaryKeyToColumn,
    PrimaryKeyTrait, RelationDef, RelationTrait, executor::result_name::derived_name,
};

/// One column of a [`TableShape`]: its name and its definition, which says
/// its type, nullability, default, comment and index.
// [spec:pgorm:sem:schema.from-shape]
#[derive(Clone)]
pub struct TableColumn {
    pub(crate) name: Name,
    pub(crate) def: ColumnDef,
    pub(crate) unstated_default: bool,
    /// The type a read casts the column to in place of its type's own read.
    select_as: Option<TypeName>,
    /// The type a write casts each value to in place of its type's own.
    save_as: Option<TypeName>,
}

impl fmt::Debug for TableColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableColumn")
            .field("name", &self.name.to_string())
            .field("def", &self.def)
            .field("unstated_default", &self.unstated_default)
            .field("select_as", &self.select_as)
            .field("save_as", &self.save_as)
            .finish()
    }
}

impl TableColumn {
    /// The column `name` of the definition `def`.
    // [spec:pgorm:sem:schema.from-shape]
    pub fn new(name: impl Into<String>, def: ColumnDef) -> Self {
        Self {
            name: Name::runtime(name),
            def,
            unstated_default: false,
            select_as: None,
            save_as: None,
        }
    }

    /// The column, which the database fills by a default this description
    /// does not state: a shape holding one has no `CREATE TABLE` to write.
    #[must_use]
    // [spec:pgorm:sem:schema.from-shape]
    pub fn with_unstated_default(mut self) -> Self {
        self.unstated_default = true;
        self
    }

    fn typed<C: ColumnTrait>(column: C) -> Self {
        Self {
            name: Name::new(column),
            def: column.def(),
            unstated_default: false,
            select_as: None,
            save_as: None,
        }
    }

    /// The column, read cast to `type_name` in place of what its type reads
    /// as: an entity column's `select_as = ".."`.
    #[must_use]
    // [spec:pgorm:sem:schema.shape-casts+1]
    pub fn with_select_as(mut self, type_name: TypeName) -> Self {
        self.select_as = Some(type_name);
        self
    }

    /// The column, each value written to it cast to `type_name` in place of
    /// what its type writes: an entity column's `save_as = ".."`, which a
    /// comparison's value takes too unless the column compares as an enum.
    #[must_use]
    // [spec:pgorm:sem:schema.shape-casts+1]
    pub fn with_save_as(mut self, type_name: TypeName) -> Self {
        self.save_as = Some(type_name);
        self
    }

    /// The name the table gives the column.
    pub fn name(&self) -> &Name {
        &self.name
    }

    /// `expr`, a read of the column, as a projection reads a column of its
    /// type: what an entity's column's
    /// [`select_as`](crate::ColumnTrait::select_as) does by default, `money`
    /// read as `numeric`, an enum and a domain over one as its text, a
    /// created multirange as its text. `None` where the column is read as it
    /// stands, so a projection can name it with no alias.
    // [spec:pgorm:sem:schema.shape-casts+1]
    pub fn select(&self, expr: Expr) -> Option<SimpleExpr> {
        if let Some(type_name) = &self.select_as {
            return Some(expr.cast_as_type(type_name.clone()));
        }
        let stored = SimpleExpr::from(expr.clone());
        let read = select_cast(self.def.get_column_type(), expr);
        (read != stored).then_some(read)
    }

    /// `value` as it is written to the column: what an entity's column's
    /// [`save_as`](crate::ColumnTrait::save_as) does by default, cast to
    /// `money`, to an enum or a domain, or to a created range's type through
    /// its text, and as it is otherwise.
    // [spec:pgorm:sem:schema.shape-casts+1]
    pub fn save(&self, value: Expr) -> SimpleExpr {
        match &self.save_as {
            Some(type_name) => value.cast_as_type(type_name.clone()),
            None => save_cast(self.def.get_column_type(), value),
        }
    }

    /// `column`, a read of the column, as a comparison takes it: cast to the
    /// enum when its type is a domain over one, which PostgreSQL resolves no
    /// comparison operator for, and as it is otherwise.
    // [spec:pgorm:sem:schema.shape-casts+1]
    pub fn compared_column(&self, column: Expr) -> Expr {
        match compared_enum(self.def.get_column_type()) {
            Some(enum_type) => Expr::expr(column.cast_as_type(enum_type)),
            None => column,
        }
    }

    /// The enum the column compares as, which
    /// [`compared_column`](Self::compared_column) casts it to: its type's,
    /// where that is a domain over an enum, through further domains.
    // [spec:pgorm:sem:schema.shape-casts+1]
    pub fn compares_as(&self) -> Option<TypeName> {
        compared_enum(self.def.get_column_type())
    }

    /// `value` as a comparison with the column takes it: cast to the enum
    /// when the column's domain is over one, so it meets
    /// [`compared_column`](Self::compared_column) as the enum, and written as
    /// [`save`](Self::save) writes it otherwise.
    // [spec:pgorm:sem:schema.shape-casts+1]
    pub fn compared_value(&self, value: Expr) -> SimpleExpr {
        compared_value_as(self.def.get_column_type(), value, |value| self.save(value))
    }

    /// The array of values `= ANY` and `<> ALL` compare the column with, as
    /// an entity column's `eq_any` and `ne_all` take it: cast to the enum's
    /// array when the column's domain is over one, to the array of the type
    /// a `save_as` names, and otherwise as the column's type writes an array
    /// of its values — the array of its `money` or enum, a created range's
    /// array through its text — or as it is.
    // [spec:pgorm:sem:schema.shape-casts+1]
    pub fn compared_array(&self, array: Expr) -> SimpleExpr {
        let col_type = self.def.get_column_type();
        if let Some(enum_type) = compared_enum(col_type) {
            return array.cast_as_type(enum_type.array());
        }
        match &self.save_as {
            Some(type_name) => array.cast_as_type(type_name.clone().array()),
            None => save_array_cast(col_type, array),
        }
    }

    /// The type [`select`](Self::select) casts the column to, which a reader
    /// of the column's rows reads it as: `None` where it is read as it
    /// stands.
    // [spec:pgorm:sem:schema.shape-casts+1]
    pub fn read_cast(&self) -> Option<TypeName> {
        cast_of(|operand| {
            self.select(operand.clone())
                .unwrap_or_else(|| operand.into())
        })
    }

    /// The type [`compared_value`](Self::compared_value) casts a compared
    /// value to, where what it writes is a cast of the value itself: `None`
    /// for a column whose values travel bare, and for a created range's,
    /// written through its text.
    // [spec:pgorm:sem:schema.shape-casts+1]
    pub fn compared_value_cast(&self) -> Option<TypeName> {
        cast_of(|operand| self.compared_value(operand))
    }

    /// What the table writes for the column after its name: its type,
    /// nullability, default and comment, and whether it is indexed.
    pub fn def(&self) -> &ColumnDef {
        &self.def
    }
}

/// A unique key of a [`TableShape`]: the constraint's name, or `None` for
/// PostgreSQL to choose one, its columns' names in key order, and whether
/// nulls are equal under it.
#[derive(Clone, Debug)]
pub(crate) struct ShapeKey {
    pub(crate) name: Option<String>,
    pub(crate) columns: Vec<String>,
    pub(crate) nulls_not_distinct: bool,
}

/// A belongs-to relation of a [`TableShape`], whose foreign key the table is
/// written with, beside the label a refusal names it by.
pub(crate) struct ShapeRelation {
    pub(crate) label: String,
    pub(crate) def: RelationDef,
}

/// A table as data: what [`Schema`](crate::Schema)'s
/// `create_table_from_shape`, `create_enum_from_shape`,
/// `create_index_from_shape` and `create_comments_from_shape` write, which
/// are the statements `create_table_from_entity` and its siblings write for
/// the entity [`of`](Self::of) describes.
// [spec:pgorm:sem:schema.from-shape]
pub struct TableShape {
    pub(crate) table: TableName,
    /// The table as a refusal names it.
    pub(crate) label: String,
    pub(crate) comment: Option<String>,
    pub(crate) columns: Vec<TableColumn>,
    /// The primary key's columns, by position, in key order.
    pub(crate) key: Vec<usize>,
    /// Whether a one-column key is drawn from the serial family.
    pub(crate) auto_increment: bool,
    /// Whether the key's last column is its period, `WITHOUT OVERLAPS`.
    pub(crate) without_overlaps: bool,
    pub(crate) unique_keys: Vec<ShapeKey>,
    pub(crate) relations: Vec<ShapeRelation>,
}

impl fmt::Debug for TableShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let relations: Vec<&str> = self.relations.iter().map(|r| r.label.as_str()).collect();
        f.debug_struct("TableShape")
            .field("table", &self.label)
            .field("comment", &self.comment)
            .field("columns", &self.columns)
            .field("key", &self.key)
            .field("auto_increment", &self.auto_increment)
            .field("without_overlaps", &self.without_overlaps)
            .field("unique_keys", &self.unique_keys)
            .field("relations", &relations)
            .finish()
    }
}

impl TableShape {
    /// The table `table`, schema-qualified where it is, of `columns` in the
    /// order its `CREATE TABLE` writes them, with no key, relation or
    /// comment until the builders below give it one. Two columns of one name
    /// are refused as [`Error::Type`].
    // [spec:pgorm:sem:schema.from-shape]
    pub fn new(table: TableName, columns: Vec<TableColumn>) -> Result<Self, Error> {
        let label = table_label_of(&table);
        for (index, column) in columns.iter().enumerate() {
            let name = column.name.to_string();
            if columns[..index].iter().any(|c| c.name.to_string() == name) {
                return Err(Error::Type(format!(
                    "`{label}` declares the column `{name}` twice"
                )));
            }
        }
        Ok(Self {
            table,
            label,
            comment: None,
            columns,
            key: Vec::new(),
            auto_increment: false,
            without_overlaps: false,
            unique_keys: Vec::new(),
            relations: Vec::new(),
        })
    }

    /// `E`'s table: its columns, primary key, unique keys, the belongs-to
    /// relations of `E::Relation` and its comment, as
    /// [`create_table_from_entity`](crate::Schema::create_table_from_entity)
    /// reads them.
    // [spec:pgorm:sem:schema.from-shape]
    pub fn of<E: EntityTrait>() -> Self {
        Self::of_entity(E::default())
    }

    /// `entity`'s table, as [`of`](Self::of) describes it, its name, schema,
    /// comment and unique keys read from this value of `E`, as an entity
    /// whose names are computed gives them.
    // [spec:pgorm:sem:schema.from-shape]
    pub fn of_entity<E: EntityTrait>(entity: E) -> Self {
        let columns: Vec<TableColumn> = E::Column::iter().map(TableColumn::typed).collect();
        // Every key column is one of `E::Column`'s, which `columns` lists.
        let key = E::PrimaryKey::iter()
            .filter_map(|key| {
                let name = key.into_column().to_string();
                columns.iter().position(|c| c.name.to_string() == name)
            })
            .collect();
        let unique_keys = entity
            .unique_keys()
            .iter()
            .map(|key| ShapeKey {
                name: key.name.map(str::to_owned),
                columns: key
                    .columns
                    .iter()
                    .map(|&column| column.to_owned())
                    .collect(),
                nulls_not_distinct: key.nulls_not_distinct,
            })
            .collect();
        let relations = E::Relation::iter()
            .filter_map(|relation| {
                let def = relation.def();
                (!def.is_owner).then(|| ShapeRelation {
                    label: format!("{relation:?}"),
                    def,
                })
            })
            .collect();
        Self {
            table: entity.table_ref(),
            label: entity.to_string(),
            comment: entity.comment().map(str::to_owned),
            columns,
            key,
            auto_increment: E::PrimaryKey::auto_increment(),
            without_overlaps: E::PrimaryKey::without_overlaps(),
            unique_keys,
            relations,
        }
    }

    /// The table keyed by the columns `columns` names, in key order,
    /// replacing any key it had. No column, a name the table has no column
    /// of and a column named twice are refused as [`Error::Type`].
    // [spec:pgorm:sem:schema.from-shape]
    pub fn with_primary_key(mut self, columns: &[&str]) -> Result<Self, Error> {
        let label = &self.label;
        if columns.is_empty() {
            return Err(Error::Type(format!(
                "`{label}` declares a primary key with no columns"
            )));
        }
        let mut key = Vec::with_capacity(columns.len());
        for name in columns {
            let Some(position) = self.position(name) else {
                return Err(Error::Type(format!(
                    "`{label}` has no column `{name}` to key its rows by"
                )));
            };
            if key.contains(&position) {
                return Err(Error::Type(format!(
                    "`{label}`.`{name}` is named twice in its key"
                )));
            }
            key.push(position);
        }
        self.key = key;
        if let Some(name) = self.primary_key_name() {
            let taken = self
                .unique_keys
                .iter()
                .any(|key| key.name.as_deref() == Some(&*name));
            if taken {
                return Err(Error::Type(format!(
                    "`{label}` names two of its keys `{name}`, which PostgreSQL refuses (42P07)"
                )));
            }
        }
        Ok(self)
    }

    /// The table, a one-column key of which is drawn from the serial family
    /// (`serial`, `bigserial`, `smallserial`) unless its column fills itself,
    /// as an entity's is when
    /// [`auto_increment`](crate::PrimaryKeyTrait::auto_increment) is true. A
    /// key of several columns is never drawn from it.
    #[must_use]
    // [spec:pgorm:sem:schema.from-shape]
    pub fn auto_increment(mut self) -> Self {
        self.auto_increment = true;
        self
    }

    /// The table, whose key's last column is its period, written `WITHOUT
    /// OVERLAPS`: a PostgreSQL 18 temporal key, as an entity's is when
    /// [`without_overlaps`](crate::PrimaryKeyTrait::without_overlaps) is
    /// true.
    #[must_use]
    // [spec:pgorm:sem:schema.from-shape]
    pub fn without_overlaps(mut self) -> Self {
        self.without_overlaps = true;
        self
    }

    /// The table with a unique key over the columns `columns` names, in key
    /// order, named `name` or left for PostgreSQL to name, nulls equal under
    /// it where `nulls_not_distinct` says so: what an entity's
    /// `#[pgorm(unique(..))]` declares. Its `CREATE TABLE` writes the keys
    /// after its primary key, in the order they were given.
    ///
    /// No column, a name the table has no column of, a column named twice
    /// (`42701`) and a `VIRTUAL` generated column, which PostgreSQL cannot
    /// key (`0A000`), are refused as [`Error::Type`], and so is a name another
    /// of its unique keys has (`42P07`).
    // [spec:pgorm:sem:schema.from-shape]
    pub fn with_unique_key(
        mut self,
        name: Option<&str>,
        columns: &[&str],
        nulls_not_distinct: bool,
    ) -> Result<Self, Error> {
        let label = &self.label;
        if let Some(name) = name {
            self.refuse_taken(name)?;
        }
        keyable(label, columns, |name| {
            self.position(name).map(|index| &self.columns[index].def)
        })?;
        self.unique_keys.push(ShapeKey {
            name: name.map(str::to_owned),
            columns: columns.iter().map(|&column| column.to_owned()).collect(),
            nulls_not_distinct,
        });
        Ok(self)
    }

    /// The table with a comment of its own, which
    /// `create_comments_from_shape` writes first.
    #[must_use]
    // [spec:pgorm:sem:schema.from-shape]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// The table with the foreign key of `relation`, a belongs-to relation
    /// from it, which a refusal names `label`: its `CREATE TABLE` writes the
    /// keys after its unique keys, in the order they were given, each as
    /// [`ForeignKeyCreateStatement::from`](pgorm_query::ForeignKeyCreateStatement)
    /// writes a relation's.
    ///
    /// An owner relation (`has_one`, `has_many`), which holds no key of this
    /// table, and one pairing a column this table lacks are refused as
    /// [`Error::Type`].
    // [spec:pgorm:sem:schema.from-shape]
    pub fn with_relation(
        mut self,
        label: impl Into<String>,
        relation: RelationDef,
    ) -> Result<Self, Error> {
        let label = label.into();
        if relation.is_owner {
            return Err(Error::Type(format!(
                "relation `{label}` of `{}` is not a belongs-to relation, so it holds no \
                 foreign key of the table",
                self.label
            )));
        }
        for (from, _) in relation.columns.iter() {
            let from = from.to_string();
            if self.position(&from).is_none() {
                return Err(Error::Type(format!(
                    "relation `{label}` of `{}` pairs `{from}`, which the table has no column of",
                    self.label
                )));
            }
        }
        self.relations.push(ShapeRelation {
            label,
            def: relation,
        });
        Ok(self)
    }

    /// `table`'s shape, which its `CREATE TABLE` is written from when no
    /// entity writes it: its columns, key and unique keys.
    // [spec:pgorm:sem:disjoint.schema+3]
    pub(crate) fn of_disjoint(table: &DisjointTable) -> Self {
        let columns = table.columns.iter().map(|column| TableColumn {
            name: Name::clone(&column.name),
            def: column.def.clone(),
            unstated_default: column.unstated_default,
            select_as: None,
            save_as: None,
        });
        Self {
            table: table.table.clone(),
            label: table.label.clone(),
            comment: None,
            columns: columns.collect(),
            key: table.key.clone(),
            auto_increment: table.auto_increment,
            without_overlaps: table.without_overlaps,
            unique_keys: table.unique_keys.clone(),
            relations: Vec::new(),
        }
    }

    /// The table, schema-qualified where it is.
    pub fn table(&self) -> &TableName {
        &self.table
    }

    /// The columns, in the order the table writes them.
    pub fn columns(&self) -> &[TableColumn] {
        &self.columns
    }

    /// The key's columns, in key order.
    pub fn key(&self) -> impl Iterator<Item = &TableColumn> {
        self.key.iter().map(|index| &self.columns[*index])
    }

    /// The name a composite key is written under, `pk-{table}`, or `None`
    /// for a key of one column or none, which PostgreSQL names.
    fn primary_key_name(&self) -> Option<String> {
        (self.key.len() > 1).then(|| derived_name("pk", &self.table.table().to_string(), []))
    }

    /// Refuse `name` for a unique key where another key of the table has it.
    fn refuse_taken(&self, name: &str) -> Result<(), Error> {
        let taken = self
            .unique_keys
            .iter()
            .any(|key| key.name.as_deref() == Some(name))
            || self.primary_key_name().as_deref() == Some(name);
        match taken {
            true => Err(Error::Type(format!(
                "`{}` names two of its keys `{name}`, which PostgreSQL refuses (42P07)",
                self.label
            ))),
            false => Ok(()),
        }
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name.to_string() == name)
    }
}

/// Refuse a key over `columns` of the table `label`, each column's
/// definition found by `def`, that PostgreSQL cannot write: no column, one
/// it lacks, one named twice (`42701`), and a `VIRTUAL` generated column,
/// which it cannot key (`0A000`).
// [spec:pgorm:sem:schema.from-shape]
// [spec:pgorm:req:bindings.model-unique-keys+2]
pub(crate) fn keyable<'d>(
    label: &str,
    columns: &[&str],
    def: impl Fn(&str) -> Option<&'d ColumnDef>,
) -> Result<(), Error> {
    if columns.is_empty() {
        return Err(Error::Type(format!(
            "`{label}` declares a unique key with no columns"
        )));
    }
    for (index, name) in columns.iter().enumerate() {
        let Some(column) = def(name) else {
            return Err(Error::Type(format!(
                "`{label}` has no column `{name}` for a unique key"
            )));
        };
        if columns[..index].contains(name) {
            return Err(Error::Type(format!(
                "`{label}`.`{name}` is named twice in one unique key"
            )));
        }
        if let Some(ColumnDefault::Generated(_, GeneratedKind::Virtual)) = column.default {
            return Err(Error::Type(format!(
                "`{label}`.`{name}` is a VIRTUAL generated column, which PostgreSQL cannot key"
            )));
        }
    }
    Ok(())
}

/// `table` as an error names it, schema-qualified where it is.
pub(crate) fn table_label_of(table: &TableName) -> String {
    match table.schema() {
        Some(schema) => format!("{}.{}", schema.to_string(), table.table().to_string()),
        None => table.table().to_string(),
    }
}
