//! A disjoint entity as data: the tables, columns, key and kinds every
//! disjoint statement is built from.
//!
//! [`DisjointShape`] is what the read, the writes, the schema and the check
//! run over. [`DisjointShape::of`] describes a `#[derive(Disjoint)]` enum,
//! each column cast by its typed column's own `select_as` and `save_as`;
//! [`DisjointShape::new`] describes tables known only at run time, as a
//! language binding's models are, each column cast as a column of its type is
//! by default. Either way the statements come from the same code.

use std::{fmt, sync::Arc};

use pgorm_query::{ColumnType, Name, TableCreateStatement, TableName, TypeName, Value};

use super::{ColumnDefault, column_casts::ColumnCasts, key::ComparedPair};
use crate::{
    ActiveModelTrait, ActiveValue, ColumnDef, ColumnTrait, Disjoint, EntityTrait, Error, Iterable,
    KindModel, KindVisitor, PrimaryKeyToColumn, PrimaryKeyTrait, RelationDef, StaticName,
    table_label,
};

/// One column of a [`DisjointTable`]: its name, its definition, and the casts
/// a disjoint statement reads and writes it under.
// [spec:pgorm:def:disjoint.declare+4]
#[derive(Clone)]
pub struct DisjointColumn {
    pub(crate) name: Name,
    pub(crate) def: ColumnDef,
    pub(crate) casts: ColumnCasts,
    pub(crate) unstated_default: bool,
}

impl fmt::Debug for DisjointColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DisjointColumn")
            .field("name", &self.name.to_string())
            .field("def", &self.def)
            .field("unstated_default", &self.unstated_default)
            .finish()
    }
}

impl DisjointColumn {
    /// A column named `name` with the definition `def`, read and written
    /// under the casts a column of its type takes by default: an enum read as
    /// its text and written cast to its type, a created range written
    /// through its text.
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn new(name: impl Into<String>, def: ColumnDef) -> Self {
        Self {
            casts: ColumnCasts::of_type(def.get_column_type()),
            name: Name::runtime(name),
            def,
            unstated_default: false,
        }
    }

    /// The column, which the database fills by a default this description
    /// does not state: an insert may leave it out, a kind's key may not be
    /// it, and no table of it can be written.
    #[must_use]
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn with_unstated_default(mut self) -> Self {
        self.unstated_default = true;
        self
    }

    fn typed<C: ColumnTrait>(col: C) -> Self {
        Self {
            name: Name::new(col),
            def: col.def(),
            casts: ColumnCasts::of(col),
            unstated_default: false,
        }
    }

    /// The name every statement gives the column.
    pub fn name(&self) -> &Name {
        &self.name
    }

    /// The column's definition, which the schema writes and whose type
    /// decides the casts a column described at run time takes.
    pub fn def(&self) -> &ColumnDef {
        &self.def
    }

    /// Whether the database fills the column by a default the description
    /// does not state.
    pub fn has_unstated_default(&self) -> bool {
        self.unstated_default
    }

    /// Whether the database fills the column itself: a default, stated or
    /// not, an identity or a generated column.
    pub(crate) fn fill(&self) -> Option<&'static str> {
        match self.def.default {
            Some(ColumnDefault::Identity(_)) => Some("an identity"),
            Some(ColumnDefault::Generated(..)) => Some("a generated column"),
            Some(ColumnDefault::Expr(_)) => Some("given a default"),
            None if self.unstated_default => Some("given a default"),
            None => None,
        }
    }
}

/// How a table's `CREATE TABLE` is written: by its entity, as
/// `Schema::create_table_from_entity` writes it with the belongs-to
/// relations the argument keeps, or from the columns alone.
#[derive(Clone)]
pub(crate) enum TableBody {
    Entity(Arc<WriteBody>),
    Columns,
}

/// An entity's `CREATE TABLE`, writing the foreign key of each belongs-to
/// relation the argument keeps.
pub(crate) type WriteBody =
    dyn Fn(&dyn Fn(&RelationDef) -> bool) -> Result<TableCreateStatement, Error> + Send + Sync;

/// One table of a [`DisjointShape`]: the base's or a kind's.
// [spec:pgorm:def:disjoint.declare+4]
#[derive(Clone)]
pub struct DisjointTable {
    pub(crate) table: TableName,
    pub(crate) label: String,
    pub(crate) columns: Vec<DisjointColumn>,
    pub(crate) key: Vec<usize>,
    pub(crate) auto_increment: bool,
    pub(crate) body: TableBody,
}

impl fmt::Debug for DisjointTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DisjointTable")
            .field("table", &self.label)
            .field("columns", &self.columns)
            .field("key", &self.key)
            .field("auto_increment", &self.auto_increment)
            .finish()
    }
}

impl DisjointTable {
    /// The table `table`, schema-qualified where it is, of `columns` in the
    /// order its statements name them, keyed by the columns `key` names in
    /// key order.
    ///
    /// Two columns of one name, a key naming a column the table lacks or
    /// one column twice, and an empty key are refused as [`Error::Type`].
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn new(
        table: TableName,
        columns: Vec<DisjointColumn>,
        key: &[&str],
    ) -> Result<Self, Error> {
        let label = label_of(&table);
        for (index, column) in columns.iter().enumerate() {
            let name = column.name.to_string();
            if columns[..index].iter().any(|c| c.name.to_string() == name) {
                return Err(Error::Type(format!(
                    "`{label}` declares the column `{name}` twice"
                )));
            }
        }
        if key.is_empty() {
            return Err(Error::Type(format!(
                "`{label}` has no primary key, which a disjoint entity's tables are keyed by"
            )));
        }
        let mut positions = Vec::with_capacity(key.len());
        for name in key {
            let Some(position) = columns.iter().position(|c| c.name.to_string() == *name) else {
                return Err(Error::Type(format!(
                    "`{label}` has no column `{name}` to key its rows by"
                )));
            };
            if positions.contains(&position) {
                return Err(Error::Type(format!(
                    "`{label}`.`{name}` is named twice in its key"
                )));
            }
            positions.push(position);
        }
        Ok(Self {
            table,
            label,
            columns,
            key: positions,
            auto_increment: false,
            body: TableBody::Columns,
        })
    }

    /// `E`'s table, each column cast by its typed column.
    fn typed<E: EntityTrait>() -> Self {
        let entity = E::default();
        let columns: Vec<DisjointColumn> = E::Column::iter().map(DisjointColumn::typed).collect();
        // Every key column is one of `E::Column`'s, which `columns` lists.
        let key = E::PrimaryKey::iter()
            .filter_map(|key| {
                let name = key.into_column();
                columns
                    .iter()
                    .position(|column| column.name.to_string() == name.as_str())
            })
            .collect();
        Self {
            table: entity.table_ref(),
            label: table_label(&entity),
            columns,
            key,
            auto_increment: E::PrimaryKey::auto_increment(),
            body: TableBody::Entity(Arc::new(|keep: &dyn Fn(&RelationDef) -> bool| {
                crate::schema::table_from_entity(E::default(), keep)
            })),
        }
    }

    /// The table, schema-qualified where it is.
    pub fn table(&self) -> &TableName {
        &self.table
    }

    /// The table's bare name, which a disjoint statement qualifies its
    /// columns by.
    pub fn name(&self) -> &Name {
        self.table.table()
    }

    /// The table as an error names it, schema-qualified where it is.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The columns, in the order the statements name them.
    pub fn columns(&self) -> &[DisjointColumn] {
        &self.columns
    }

    /// The key's columns, in key order.
    pub fn key(&self) -> impl Iterator<Item = &DisjointColumn> {
        self.key.iter().map(|index| &self.columns[*index])
    }

    /// The column named `name`, and where it stands.
    pub fn column(&self, name: &str) -> Option<(usize, &DisjointColumn)> {
        self.columns
            .iter()
            .enumerate()
            .find(|(_, column)| column.name.to_string() == name)
    }

    /// What a write gives this table's columns, each column named once:
    /// `Set`, `Unchanged` or `NotSet`, and a column left out `NotSet`. A name
    /// the table has no column of, and one named twice, are refused as
    /// [`Error::Type`].
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn values<N, I>(&self, values: I) -> Result<DisjointValues, Error>
    where
        N: AsRef<str>,
        I: IntoIterator<Item = (N, ActiveValue<Value>)>,
    {
        let mut written: Vec<Option<ActiveValue<Value>>> = vec![None; self.columns.len()];
        for (name, value) in values {
            let name = name.as_ref();
            let Some((index, _)) = self.column(name) else {
                return Err(Error::Type(format!(
                    "`{}` has no column `{name}`",
                    self.label
                )));
            };
            if written[index].replace(value).is_some() {
                return Err(Error::Type(format!(
                    "`{}`.`{name}` is given two values",
                    self.label
                )));
            }
        }
        Ok(DisjointValues {
            table: self.label.clone(),
            values: written
                .into_iter()
                .map(|value| value.unwrap_or(ActiveValue::NotSet))
                .collect(),
        })
    }

    /// `values`, refused as [`Error::Type`] unless they are this table's.
    pub(crate) fn owns<'v>(
        &self,
        values: &'v DisjointValues,
    ) -> Result<&'v [ActiveValue<Value>], Error> {
        if values.table != self.label || values.values.len() != self.columns.len() {
            return Err(Error::Type(format!(
                "the values given for `{}` are `{}`'s",
                self.label, values.table
            )));
        }
        Ok(&values.values)
    }
}

/// What a disjoint write gives one table's columns, in the table's column
/// order: each `Set`, `Unchanged` or `NotSet`, as an `ActiveModel` holds it.
// [spec:pgorm:sem:disjoint.write+1]
#[derive(Clone, Debug)]
pub struct DisjointValues {
    pub(crate) table: String,
    pub(crate) values: Vec<ActiveValue<Value>>,
}

impl DisjointValues {
    /// What `active` holds for each of its entity's columns.
    pub fn of_active<A: ActiveModelTrait>(active: &A) -> Result<Self, Error> {
        Ok(Self {
            table: table_label(&A::Entity::default()),
            values: <A::Entity as EntityTrait>::Column::iter()
                .map(|column| active.get(column))
                .collect::<Result<_, _>>()?,
        })
    }
}

/// One kind of a [`DisjointShape`]: its value, and its table, which a kind
/// without one lacks.
// [spec:pgorm:def:disjoint.declare+4]
#[derive(Clone, Debug)]
pub struct DisjointKind {
    pub(crate) value: String,
    pub(crate) table: Option<DisjointTable>,
}

impl DisjointKind {
    /// The kind's value, as the kind column holds it.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The kind's table, or `None` for a kind without one.
    pub fn table(&self) -> Option<&DisjointTable> {
        self.table.as_ref()
    }
}

/// A disjoint entity as data: its base table, the base's kind column and the
/// kind type, which an unkinded entity lacks, and each kind in declaration
/// order with its table.
///
/// Every disjoint statement is built from one: the read
/// ([`find`](Self::find)), the writes ([`insert`](Self::insert),
/// [`update`](Self::update), [`change_kind`](Self::change_kind)) and the
/// schema ([`create`](Self::create)), the last two a kinded shape's alone.
/// [`Disjoint`]'s methods build theirs from [`DisjointShape::of`]; tables
/// known only at run time are described with [`DisjointShape::new`].
// [spec:pgorm:def:disjoint.declare+4]
// [spec:pgorm:sem:disjoint.unkinded]
#[derive(Clone, Debug)]
pub struct DisjointShape {
    pub(crate) base: DisjointTable,
    pub(crate) kinded: Option<KindedBy>,
    pub(crate) kinds: Vec<DisjointKind>,
}

/// Where a kinded shape's base holds each row's kind: the index of its kind
/// column among the base's columns, and the kind type.
#[derive(Clone, Debug)]
pub(crate) struct KindedBy {
    pub(crate) column: usize,
    pub(crate) kind_type: ColumnType,
}

/// Describes each kind's table for [`DisjointShape::of`].
struct TypedKind;

impl<D: Disjoint> KindVisitor<D> for TypedKind {
    type Output = Option<DisjointTable>;

    fn table<M: KindModel<D>>(self, _: D::Kind) -> Option<DisjointTable> {
        Some(DisjointTable::typed::<M::Entity>())
    }

    fn unit(self, _: D) -> Option<DisjointTable> {
        None
    }
}

impl DisjointShape {
    /// The shape `D` declares: its base's and its kinds' tables as their
    /// entities declare them, each column cast by its typed column, its kind
    /// column where it has one, and the kinds in declaration order, each
    /// named as [`Disjoint::kind_name`] names it.
    // [spec:pgorm:sem:disjoint.unkinded]
    pub fn of<D: Disjoint>() -> Self {
        let base = DisjointTable::typed::<D::Base>();
        let kinded = D::kind_column().map(|kind_column| KindedBy {
            // The kind column is one of the base's `Column`s, every one of
            // which the base table lists, so the search always finds it.
            column: base
                .columns
                .iter()
                .position(|column| column.name.to_string() == kind_column.column.as_str())
                .unwrap_or_default(),
            kind_type: kind_column.kind_type.get_column_type().clone(),
        });
        Self {
            base,
            kinded,
            kinds: D::Kind::iter()
                .map(|kind| DisjointKind {
                    value: D::kind_name(kind).to_owned(),
                    table: D::visit_kind(kind, TypedKind),
                })
                .collect(),
        }
    }

    /// The shape of tables known only at run time: the base table, its
    /// column `kind_column` holding each row's kind, the kind type `kind_type`
    /// (an enum of the kinds' values, named and schema-qualified as it is),
    /// and the kinds in order, each its value and its table or `None`.
    ///
    /// What a `#[derive(Disjoint)]` enum's types hold, this refuses as
    /// [`Error::Type`]: no kinds, a value given twice, one table for two
    /// kinds or for a kind and the base, a base without `kind_column`, and a
    /// kind table whose key is not of the base key's types in its order.
    /// Then [`check`](Self::check) runs, and its refusal is this one's.
    // [spec:pgorm:req:bindings.disjoint+1]
    pub fn new(
        base: DisjointTable,
        kind_column: &str,
        kind_type: TypeName,
        kinds: Vec<(String, Option<DisjointTable>)>,
    ) -> Result<Self, Error> {
        let label = base.label.clone();
        if kinds.is_empty() {
            return Err(Error::Type(format!(
                "the disjoint entity over `{label}` declares no kinds: it has at least one"
            )));
        }
        let mut tables: Vec<(String, String)> = vec![(label.clone(), "the base".to_owned())];
        for (index, (value, table)) in kinds.iter().enumerate() {
            if kinds[..index].iter().any(|(other, _)| other == value) {
                return Err(Error::Type(format!(
                    "the disjoint entity over `{label}` declares the kind `{value}` twice"
                )));
            }
            let Some(table) = table else { continue };
            if let Some((_, owner)) = tables.iter().find(|(taken, _)| *taken == table.label) {
                return Err(Error::Type(format!(
                    "`{}` is the table of the kind `{value}` and of {owner}: each kind has a \
                     table of its own",
                    table.label
                )));
            }
            tables.push((table.label.clone(), format!("the kind `{value}`")));
            let base_key: Vec<&ColumnType> = base
                .key()
                .map(|column| column.def.get_column_type())
                .collect();
            let kind_key: Vec<&ColumnType> = table
                .key()
                .map(|column| column.def.get_column_type())
                .collect();
            if base_key != kind_key {
                return Err(Error::Type(format!(
                    "the key of `{}`, the table of the kind `{value}`, is ({}), not the base \
                     `{label}`'s ({}): a kind's row is keyed by its base row's key",
                    table.label,
                    types(&kind_key),
                    types(&base_key)
                )));
            }
        }
        let Some((kind, _)) = base.column(kind_column) else {
            return Err(Error::Type(format!(
                "`{label}` has no column `{kind_column}`, the kind column a disjoint entity's \
                 base holds each row's kind in"
            )));
        };
        let shape = Self {
            base,
            kinded: Some(KindedBy {
                column: kind,
                kind_type: ColumnType::Enum {
                    name: Name::clone(&kind_type.name),
                    schema: kind_type.schema.clone(),
                    variants: kinds
                        .iter()
                        .map(|(value, _)| Name::runtime(value.clone()))
                        .collect(),
                },
            }),
            kinds: kinds
                .into_iter()
                .map(|(value, table)| DisjointKind { value, table })
                .collect(),
        };
        shape.check()?;
        Ok(shape)
    }

    /// The base table, whose every row is of one kind, read as the decoded
    /// source 0.
    pub fn base(&self) -> &DisjointTable {
        &self.base
    }

    /// The base's column holding each row's kind, as a value of the kind
    /// type, or `None` for an unkinded shape, whose base has none.
    pub fn kind_column(&self) -> Option<&DisjointColumn> {
        self.kinded
            .as_ref()
            .map(|kinded| &self.base.columns[kinded.column])
    }

    /// The kind type, an enum of the kinds' values in order, or `None` for
    /// an unkinded shape.
    pub fn kind_type(&self) -> Option<&ColumnType> {
        self.kinded.as_ref().map(|kinded| &kinded.kind_type)
    }

    /// The kind type's name, schema-qualified where it is, or `None` for an
    /// unkinded shape and where the kind type is no database enum, which
    /// [`check`](Self::check) refuses.
    pub fn kind_type_name(&self) -> Option<TypeName> {
        match self.kind_type()? {
            ColumnType::Enum { name, schema, .. } => {
                let type_name = TypeName::new(Name::clone(name));
                Some(match schema {
                    Some(schema) => type_name.schema(Name::clone(schema)),
                    None => type_name,
                })
            }
            _ => None,
        }
    }

    /// The kinds, in declaration order.
    pub fn kinds(&self) -> &[DisjointKind] {
        &self.kinds
    }

    /// The kind whose value is `value`, and where it stands, or an
    /// [`Error::Type`] naming the kinds there are.
    pub fn kind(&self, value: &str) -> Result<(usize, &DisjointKind), Error> {
        self.kinds
            .iter()
            .enumerate()
            .find(|(_, kind)| kind.value == value)
            .ok_or_else(|| {
                no_such_kind(
                    &self.base.label,
                    value,
                    self.kinds.iter().map(|kind| kind.value.as_str()),
                )
            })
    }

    /// Each kind with a table beside the index of the decoded source its
    /// columns are projected under, counting from 1 in declaration order: the
    /// base's are under `s0_`.
    pub fn sources(&self) -> impl Iterator<Item = (usize, &DisjointKind, &DisjointTable)> {
        self.kinds
            .iter()
            .filter_map(|kind| kind.table.as_ref().map(|table| (kind, table)))
            .enumerate()
            .map(|(index, (kind, table))| (index + 1, kind, table))
    }

    /// Refuse what the types of a disjoint entity cannot tell, as
    /// [`Error::Type`] naming the table and column at fault.
    ///
    /// The base's kind column is `NOT NULL` and of the kind type, since every
    /// base row has one of its kinds, and is neither an identity nor a
    /// generated column, since a write sets it to the kind it writes. No
    /// kind table's key is generated or filled by the database — drawn from a
    /// sequence by `auto_increment`, an identity, a generated column or a
    /// default — since a kind's row takes its key from its base row. An
    /// unkinded shape has no kind column and is never written, so nothing of
    /// it is refused.
    // [spec:pgorm:def:disjoint.declare+4]
    // [spec:pgorm:req:bindings.disjoint+1]
    // [spec:pgorm:sem:disjoint.unkinded]
    pub fn check(&self) -> Result<(), Error> {
        let Some(kinded) = &self.kinded else {
            return Ok(());
        };
        let column = &self.base.columns[kinded.column];
        let def = &column.def;
        let label = &self.base.label;
        let name = column.name.to_string();
        if def.null {
            return Err(Error::Type(format!(
                "the kind column `{label}`.`{name}` is nullable: every base row of a disjoint \
                 entity has a kind, so the column is `NOT NULL`"
            )));
        }
        match def.default {
            Some(ColumnDefault::Generated(..)) => {
                return Err(Error::Type(format!(
                    "the kind column `{label}`.`{name}` is a generated column: a write sets a \
                     base row's kind to the kind it writes, so no expression computes it"
                )));
            }
            Some(ColumnDefault::Identity(_)) => {
                return Err(Error::Type(format!(
                    "the kind column `{label}`.`{name}` is an identity: a write sets a base \
                     row's kind to the kind it writes, so no sequence draws it"
                )));
            }
            Some(ColumnDefault::Expr(_)) | None => {}
        }
        if !same_enum(def.get_column_type(), &kinded.kind_type) {
            return Err(Error::Type(format!(
                "the kind column `{label}`.`{name}` is {}, not the kind type {}",
                describe(def.get_column_type()),
                describe(&kinded.kind_type)
            )));
        }
        for kind in &self.kinds {
            let Some(table) = &kind.table else { continue };
            let generated = |column: &DisjointColumn, how: &str| {
                Error::Type(format!(
                    "`{}`.`{}`, the key of the kind `{}`, is {how}: a kind's row takes its key \
                     from its base row, so the database does not generate it",
                    table.label,
                    column.name.to_string(),
                    kind.value
                ))
            };
            for column in table.key() {
                if let Some(how) = column.fill() {
                    return Err(generated(column, how));
                }
            }
            if table.auto_increment
                && let Some(column) = table.key().next()
            {
                return Err(generated(
                    column,
                    "drawn from a sequence (`auto_increment`)",
                ));
            }
        }
        Ok(())
    }

    /// Where a kinded shape's base holds each row's kind; an unkinded shape,
    /// which nothing holds to one kind per base row, is refused as
    /// [`Error::Type`] naming its base as read-only, so that no write and no
    /// schema is built from it.
    // [spec:pgorm:sem:disjoint.unkinded]
    pub(crate) fn kinded(&self) -> Result<&KindedBy, Error> {
        self.kinded.as_ref().ok_or_else(|| {
            Error::Type(format!(
                "the disjoint entity over `{}` is unkinded: its base has no kind column, and \
                 nothing in its schema holds a base row to one kind's table, so it is read-only",
                self.base.label
            ))
        })
    }

    /// Each base key column with `table`'s key column in the same place, in
    /// key order, and the enums the two compare as: the pairs a kind's row is
    /// keyed by its base row's key through.
    pub(crate) fn key_pairs<'a>(
        &'a self,
        table: &'a DisjointTable,
    ) -> impl Iterator<Item = (&'a DisjointColumn, &'a DisjointColumn, ComparedPair)> {
        self.base.key().zip(table.key()).map(|(base, kind)| {
            (
                base,
                kind,
                ComparedPair {
                    from: base.casts.column.clone(),
                    to: kind.casts.column.clone(),
                },
            )
        })
    }
}

/// The [`Error::Type`] naming `value` as no kind of the disjoint entity over
/// `base`, whose kinds are `kinds`.
pub(crate) fn no_such_kind<'k>(
    base: &str,
    value: &str,
    kinds: impl Iterator<Item = &'k str>,
) -> Error {
    let kinds: Vec<String> = kinds.map(|kind| format!("`{kind}`")).collect();
    Error::Type(format!(
        "`{value}` is no kind of the disjoint entity over `{base}`, whose kinds are {}",
        kinds.join(", ")
    ))
}

/// A table as an error names it, schema-qualified where it is.
fn label_of(table: &TableName) -> String {
    match table.schema() {
        Some(schema) => format!("{}.{}", schema.to_string(), table.table().to_string()),
        None => table.table().to_string(),
    }
}

/// Column types as an error lists them.
fn types(types: &[&ColumnType]) -> String {
    types
        .iter()
        .map(|column_type| describe(column_type))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether two column types are one PostgreSQL enum: the same name in the
/// same schema, with the same values in the same order.
fn same_enum(left: &ColumnType, right: &ColumnType) -> bool {
    let identity = |column_type: &ColumnType| match column_type {
        ColumnType::Enum {
            name,
            schema,
            variants,
        } => Some((
            name.to_string(),
            schema.as_ref().map(|schema| schema.to_string()),
            variants
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>(),
        )),
        _ => None,
    };
    matches!((identity(left), identity(right)), (Some(left), Some(right)) if left == right)
}

/// A column type as an error names it: an enum by its name and values.
fn describe(column_type: &ColumnType) -> String {
    match column_type {
        ColumnType::Enum {
            name,
            schema,
            variants,
        } => {
            let name = match schema {
                Some(schema) => format!("{}.{}", schema.to_string(), name.to_string()),
                None => name.to_string(),
            };
            let values: Vec<String> = variants
                .iter()
                .map(|value| format!("'{}'", value.to_string()))
                .collect();
            format!("`{name}` ({})", values.join(", "))
        }
        other => format!("`{other:?}`"),
    }
}
