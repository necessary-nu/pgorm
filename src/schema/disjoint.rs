//! The tables of a disjoint entity, and the constraints that hold each base
//! row to exactly its own kind's table.

use super::{
    DdlStatement,
    entity::{column_ddl, create_enum_from_column_type, unique_key, write_primary_key},
};
use crate::{
    ColumnPairs, ColumnType, DisjointKind, DisjointShape, DisjointTable, Error, KindedDisjoint,
    RelationDef, RelationType, Schema, TableBody,
    entity::KindedBy,
    executor::result_name::{bounded_identifier, derived_name},
    unpack_table_name,
};
use pgorm_query::{
    ColumnDef, ConditionType, Deferrability, Expr, ForeignKeyAction, FromItem, GeneratedKind, Name,
    Table, TableAlterStatement, TableCreateStatement, TableForeignKey, TableKey, TableName, Unique,
};
use std::iter;

impl Schema {
    /// Create a disjoint entity's kind type and tables, with the constraints
    /// that make PostgreSQL refuse every state the enum does not describe,
    /// as the statements to run, in the order they run: the statements
    /// [`DisjointShape::create`] writes for [`DisjointShape::of::<D>`](DisjointShape::of).
    ///
    /// [`check_disjoint`](crate::check_disjoint) runs first, and its refusal
    /// is this one's. The statements are the kind type; the base's table as
    /// [`create_table_from_entity`](Self::create_table_from_entity) writes
    /// it, with a `UNIQUE` key on its key columns and its kind column; each
    /// kind's table as `create_table_from_entity` writes it, with a stored
    /// generated column holding its kind and a foreign key to the base's
    /// `UNIQUE` key, `ON DELETE CASCADE`; and, when any kind has a table, an
    /// `ALTER TABLE` adding to the base, for each such kind, its key as
    /// stored generated columns null for every other kind, and a foreign key
    /// from them to the kind's table. Every foreign key is `DEFERRABLE
    /// INITIALLY IMMEDIATE`: checked at the end of each statement, so a
    /// write sent as one statement is checked whole, and one sent as several
    /// passes once `SET CONSTRAINTS` moves the checks to `COMMIT`.
    ///
    /// A belongs-to relation a kind's entity declares from its key to the
    /// base's writes no foreign key of its own, since the kind's key to its
    /// base row with its kind implies it. The types the tables' other columns
    /// use are not among the statements:
    /// [`create_enum_from_entity`](Self::create_enum_from_entity) creates
    /// them, and creates the kind type too when given the base.
    ///
    /// Every column and constraint added is named from the tables, columns
    /// and kind values alone, bounded to PostgreSQL's 63 bytes as prefixed
    /// result names are ([`result_column_name`](crate::result_column_name)):
    /// a generated column `{value}_{column}`, the base's `UNIQUE` key
    /// `uq-{base}-{key}-{kind column}`, a kind table's foreign key
    /// `fk-{table}-{key}-{kind column}`, and the base's to it
    /// `fk-{base}-{generated columns}`, each list `-`-joined and each table
    /// bare. A name it adds to a table that the table already declares, or
    /// adds twice, is refused with [`Error::Type`] naming both.
    // [spec:pgorm:sem:disjoint.schema+2]
    // [spec:pgorm:sem:disjoint.unkinded+1]
    pub fn create_disjoint<D: KindedDisjoint>(&self) -> Result<Vec<DdlStatement>, Error> {
        DisjointShape::of::<D>().create()
    }
}

impl DisjointShape {
    /// The statements creating this shape's kind type and tables, in the
    /// order they run, as [`Schema::create_disjoint`] describes them;
    /// [`check`](Self::check) runs first, and its refusal is this one's.
    ///
    /// A table a shape described at run time is written from its columns,
    /// as `create_table_from_entity` writes an entity's columns, primary key
    /// and unique keys, the keys [`DisjointTable::with_unique_key`] gave it:
    /// it declares no relation, index or comment. A column whose
    /// default the description does not state has no `DEFAULT` to write, so
    /// a table holding one is refused with [`Error::Type`] naming it, and so
    /// is an unkinded shape, whose schema pgorm does not write.
    // [spec:pgorm:sem:disjoint.schema+2]
    // [spec:pgorm:req:bindings.disjoint+1]
    // [spec:pgorm:sem:disjoint.unkinded+1]
    pub fn create(&self) -> Result<Vec<DdlStatement>, Error> {
        let kinded = self.kinded()?;
        self.check()?;
        self.keyable()?;
        let base = BaseTable::of(self, kinded);
        let kinds = base.kind_tables(self)?;
        let alter = base.alter(&base.kind_keys(&kinds)?, false);

        let mut create = body(&self.base, &|_| true)?;
        create.unique(base.unique());
        let kind_type = create_enum_from_column_type(&kinded.kind_type).ok_or_else(|| {
            Error::Type(format!(
                "the kind type of the disjoint entity over `{}` is not a database enum, so \
                 there is no type to create",
                self.base.label
            ))
        })?;
        let mut statements = vec![
            DdlStatement::CreateType(kind_type),
            DdlStatement::CreateTable(create),
        ];
        statements.extend(
            kinds
                .into_iter()
                .map(|kind| DdlStatement::CreateTable(kind.create)),
        );
        statements.extend(alter.map(DdlStatement::AlterTable));
        Ok(statements)
    }
}

/// `table`'s `CREATE TABLE`, as its entity writes it with the belongs-to
/// relations `keep` keeps, or from its columns, with the unique keys
/// [`DisjointTable::with_unique_key`] gave it.
// [spec:pgorm:sem:disjoint.schema+2]
// [spec:pgorm:req:bindings.model-unique-keys]
fn body(
    table: &DisjointTable,
    keep: &dyn Fn(&RelationDef) -> bool,
) -> Result<TableCreateStatement, Error> {
    let mut stmt = match &table.body {
        TableBody::Entity(write) => write(keep)?,
        TableBody::Columns => {
            let mut stmt = TableCreateStatement::new(table.table.clone());
            for column in &table.columns {
                if column.unstated_default {
                    return Err(Error::Type(format!(
                        "`{}`.`{}` has a default its description does not state, so its \
                         table cannot be written",
                        table.label,
                        column.name.to_string()
                    )));
                }
                stmt.col(column_ddl(
                    Name::clone(&column.name),
                    column.def.clone(),
                    false,
                ));
            }
            let keys = table
                .key()
                .map(|column| Name::clone(&column.name))
                .collect();
            write_primary_key(&mut stmt, &table.name().to_string(), keys, None);
            stmt
        }
    };
    for key in &table.unique_keys {
        let columns: Vec<&str> = key.columns.iter().map(String::as_str).collect();
        stmt.unique(unique_key(
            &table.label,
            key.name.as_deref(),
            &columns,
            key.nulls_not_distinct,
        )?);
    }
    Ok(stmt)
}

/// What every kind's table is written against: the base's table, key and
/// kind column.
// [spec:pgorm:sem:disjoint.schema+2]
pub(super) struct BaseTable {
    pub(super) table: TableName,
    pub(super) name: String,
    label: String,
    key: KeyColumns,
    key_types: Vec<ColumnType>,
    pub(super) kind: String,
    kind_type: ColumnType,
    declared: Vec<String>,
}

impl BaseTable {
    pub(super) fn of(shape: &DisjointShape, kinded: &KindedBy) -> Self {
        let base = &shape.base;
        Self {
            table: base.table.clone(),
            name: base.name().to_string(),
            label: base.label.clone(),
            key: KeyColumns::of(base),
            key_types: base
                .key()
                .map(|column| column.def.get_column_type().clone())
                .collect(),
            kind: base.columns[kinded.column].name.to_string(),
            kind_type: kinded.kind_type.clone(),
            declared: column_names(base),
        }
    }

    /// Each kind with a table, in declaration order, its table written
    /// against the base.
    // [spec:pgorm:sem:disjoint.schema+2]
    pub(super) fn kind_tables(&self, shape: &DisjointShape) -> Result<Vec<KindTable>, Error> {
        shape
            .kinds
            .iter()
            .filter_map(|kind| kind.table.as_ref().map(|table| (kind, table)))
            .map(|(kind, table)| kind_table(self, kind, table))
            .collect()
    }

    /// The base's `UNIQUE` key on its key columns and its kind column, which
    /// each kind table's foreign key references.
    // [spec:pgorm:sem:disjoint.schema+2]
    pub(super) fn unique(&self) -> TableKey<Unique> {
        TableKey::new(Name::runtime(self.key.first.clone()))
            .cols(self.key.rest.iter().cloned().map(Name::runtime))
            .col(Name::runtime(self.kind.clone()))
            .name(Name::runtime(self.unique_name()))
    }

    pub(super) fn unique_name(&self) -> String {
        derived_name(
            "uq",
            &self.name,
            self.key.iter().chain([self.kind.as_str()]),
        )
    }

    /// The names of the generated key columns the base holds for the kind
    /// `value`, and of their foreign key to the kind's table.
    // [spec:pgorm:sem:disjoint.schema+2]
    pub(super) fn generated(&self, value: &str) -> Generated {
        let columns = self
            .key
            .map(|column| bounded_identifier(format!("{value}_{column}")));
        Generated {
            name: derived_name("fk", &self.name, columns.iter()),
            columns,
        }
    }

    /// For each of `kinds`, one generated column per key column, null for
    /// every other kind, and a foreign key from them to the kind's table:
    /// refused when a name added is taken.
    // [spec:pgorm:sem:disjoint.schema+2]
    pub(super) fn kind_keys(&self, kinds: &[KindTable]) -> Result<Vec<KindKeys>, Error> {
        let mut added: Vec<(String, String)> = Vec::new();
        let mut keys = Vec::new();
        for kind in kinds {
            let generated = self.generated(&kind.value);
            let mut columns = Vec::new();
            let sources = self.key.iter().zip(&self.key_types);
            for (name, (column, column_type)) in generated.columns.iter().zip(sources) {
                let origin = format!("the key column `{column}` of the kind `{}`", kind.value);
                self.refuse_taken(name, &origin, &added)?;
                let mut def = ColumnDef::new_with_type(Name::runtime(name), column_type.clone());
                def.generated(
                    Expr::case(
                        Expr::col(Name::runtime(self.kind.clone()))
                            .eq(Expr::val(kind.value.clone())),
                        Expr::col(Name::runtime(column)),
                    ),
                    GeneratedKind::Stored,
                );
                columns.push(def);
                added.push((name.to_owned(), origin));
            }
            let foreign_key = TableForeignKey::from(deferrable_key(
                self.table.clone(),
                kind.table.clone(),
                generated.columns.pairs(&kind.key),
                generated.name.clone(),
                None,
            ));
            keys.push(KindKeys {
                generated,
                columns,
                foreign_key,
            });
        }
        Ok(keys)
    }

    /// The `ALTER TABLE` adding to the base the generated key columns and
    /// foreign keys of `keys`, the keys `NOT VALID` where `not_valid` says
    /// so, or `None` when `keys` adds none.
    // [spec:pgorm:sem:disjoint.schema+2]
    pub(super) fn alter<'k>(
        &self,
        keys: impl IntoIterator<Item = &'k KindKeys>,
        not_valid: bool,
    ) -> Option<TableAlterStatement> {
        let keys: Vec<&KindKeys> = keys.into_iter().collect();
        let mut columns = keys.iter().flat_map(|keys| keys.columns.iter().cloned());
        let mut alter = Table::alter(self.table.clone()).add_column(columns.next()?);
        for column in columns {
            alter.add_column(column);
        }
        for keys in keys {
            let foreign_key = keys.foreign_key.clone();
            if not_valid {
                alter.add_foreign_key(foreign_key.not_valid());
            } else {
                alter.add_foreign_key(foreign_key);
            }
        }
        Some(alter)
    }

    /// Refuse `name`, the column generated for `origin`, when the base
    /// declares a column of that name or another generated column has it.
    // [spec:pgorm:sem:disjoint.schema+2]
    fn refuse_taken(
        &self,
        name: &str,
        origin: &str,
        added: &[(String, String)],
    ) -> Result<(), Error> {
        let label = &self.label;
        if self.declared.iter().any(|declared| declared == name) {
            return Err(Error::Type(format!(
                "`{label}`.`{name}`, the column the disjoint schema adds for {origin}, is a \
                 column `{label}` declares"
            )));
        }
        match added.iter().find(|(taken, _)| taken == name) {
            Some((_, other)) => Err(Error::Type(format!(
                "`{label}`.`{name}`, the column the disjoint schema adds for {origin}, is also \
                 the one it adds for {other}"
            ))),
            None => Ok(()),
        }
    }
}

/// A kind with a table: its value, its table and key, the statement
/// creating the table, and the table's kind column and foreign key to the
/// base, which that statement holds.
pub(super) struct KindTable {
    pub(super) value: String,
    pub(super) table: TableName,
    pub(super) label: String,
    key: KeyColumns,
    pub(super) create: TableCreateStatement,
    pub(super) kind_column: ColumnDef,
    pub(super) key_name: String,
    pub(super) foreign_key: TableForeignKey,
}

/// The names of the base's generated key columns for one kind, in key
/// order, and of their foreign key to the kind's table.
pub(super) struct Generated {
    pub(super) columns: KeyColumns,
    pub(super) name: String,
}

/// The base's generated key columns for one kind with a table, and their
/// foreign key to it.
pub(super) struct KindKeys {
    pub(super) generated: Generated,
    columns: Vec<ColumnDef>,
    foreign_key: TableForeignKey,
}

/// `kind`'s table written against the base.
// [spec:pgorm:sem:disjoint.schema+2]
fn kind_table(
    base: &BaseTable,
    kind: &DisjointKind,
    table: &DisjointTable,
) -> Result<KindTable, Error> {
    let value = kind.value.clone();
    if column_names(table).contains(&base.kind) {
        let label = &table.label;
        return Err(Error::Type(format!(
            "`{label}`.`{}`, the kind column the disjoint schema adds to the table of the \
             kind `{value}`, is a column `{label}` declares",
            base.kind
        )));
    }
    let key = KeyColumns::of(table);
    let mut create = body(table, &|relation| !is_key_to_base(relation, base, &key))?;
    let mut kind_column =
        ColumnDef::new_with_type(Name::runtime(base.kind.clone()), base.kind_type.clone());
    kind_column.generated(Expr::val(value.clone()), GeneratedKind::Stored);
    create.col(kind_column.clone());
    let name = derived_name(
        "fk",
        &table.name().to_string(),
        key.iter().chain([base.kind.as_str()]),
    );
    let foreign_key = || {
        deferrable_key(
            table.table.clone(),
            base.table.clone(),
            key.pairs(&base.key).and(
                Name::runtime(base.kind.clone()),
                Name::runtime(base.kind.clone()),
            ),
            name.clone(),
            Some(ForeignKeyAction::Cascade),
        )
    };
    create.foreign_key(foreign_key());
    let foreign_key = TableForeignKey::from(foreign_key());
    Ok(KindTable {
        value,
        table: table.table.clone(),
        label: table.label.clone(),
        key,
        create,
        kind_column,
        key_name: name,
        foreign_key,
    })
}

/// A key's column names in order, the first held apart because every key
/// has one.
pub(super) struct KeyColumns {
    first: String,
    rest: Vec<String>,
}

impl KeyColumns {
    /// `table`'s primary key, which a shape's table always has.
    fn of(table: &DisjointTable) -> Self {
        let mut names = table.key().map(|column| column.name.to_string());
        Self {
            first: names.next().unwrap_or_default(),
            rest: names.collect(),
        }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &str> {
        iter::once(self.first.as_str()).chain(self.rest.iter().map(String::as_str))
    }

    /// Each column renamed by `rename`, in order.
    fn map(&self, rename: impl Fn(&str) -> String) -> Self {
        Self {
            first: rename(&self.first),
            rest: self.rest.iter().map(|column| rename(column)).collect(),
        }
    }

    /// Each column paired with `to`'s in the same place: a foreign key's
    /// columns, `to` being of the same arity.
    fn pairs(&self, to: &Self) -> ColumnPairs {
        let mut pairs = ColumnPairs::new(
            Name::runtime(self.first.clone()),
            Name::runtime(to.first.clone()),
        );
        for (from, to) in self.rest.iter().zip(&to.rest) {
            pairs.push(Name::runtime(from.clone()), Name::runtime(to.clone()));
        }
        pairs
    }
}

/// Whether `relation` is a key from exactly a kind's key to the base's key,
/// which the kind's key to its base row with its kind implies.
fn is_key_to_base(relation: &RelationDef, base: &BaseTable, key: &KeyColumns) -> bool {
    let mut declared: Vec<(String, String)> = relation
        .columns
        .iter()
        .map(|(from, to)| (from.to_string(), to.to_string()))
        .collect();
    let mut implied: Vec<(String, String)> = key
        .iter()
        .zip(base.key.iter())
        .map(|(from, to)| (from.to_owned(), to.to_owned()))
        .collect();
    declared.sort();
    implied.sort();
    relation.columns.period().is_none()
        && unpack_table_name(&relation.to_tbl) == base.table
        && declared == implied
}

/// The relation whose foreign key the disjoint schema writes from `pairs`'
/// first columns, of `table`, to their second, of `ref_table`: named `name`
/// and `DEFERRABLE INITIALLY IMMEDIATE`, so its check runs at the end of
/// each statement unless a transaction defers it.
// [spec:pgorm:sem:disjoint.schema+2]
fn deferrable_key(
    table: TableName,
    ref_table: TableName,
    pairs: ColumnPairs,
    name: String,
    on_delete: Option<ForeignKeyAction>,
) -> RelationDef {
    RelationDef {
        rel_type: RelationType::HasOne,
        from_tbl: FromItem::from(table),
        to_tbl: FromItem::from(ref_table),
        columns: pairs,
        is_owner: false,
        on_delete,
        on_update: None,
        on_condition: None,
        fk_name: Some(name),
        enforcement: None,
        deferrability: Some(Deferrability::DeferrableInitiallyImmediate),
        condition_type: ConditionType::All,
    }
}

fn column_names(table: &DisjointTable) -> Vec<String> {
    table
        .columns
        .iter()
        .map(|column| column.name.to_string())
        .collect()
}
