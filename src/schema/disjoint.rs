//! The tables of a disjoint entity, and the constraints that hold each base
//! row to exactly its own kind's table.

use super::{
    DdlStatement,
    entity::{column_ddl, create_enum_from_column_type, write_primary_key},
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
    Table, TableAlterStatement, TableCreateStatement, TableKey, TableName,
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
    // [spec:pgorm:sem:disjoint.unkinded]
    pub fn create_disjoint<D: KindedDisjoint>(&self) -> Result<Vec<DdlStatement>, Error> {
        DisjointShape::of::<D>().create()
    }
}

impl DisjointShape {
    /// The statements creating this shape's kind type and tables, in the
    /// order they run, as [`Schema::create_disjoint`] describes them;
    /// [`check`](Self::check) runs first, and its refusal is this one's.
    ///
    /// A table a shape described at run time is written from its columns
    /// alone, as `create_table_from_entity` writes an entity's columns and
    /// primary key: it declares no relation, index or comment. A column whose
    /// default the description does not state has no `DEFAULT` to write, so
    /// a table holding one is refused with [`Error::Type`] naming it, and so
    /// is an unkinded shape, whose schema pgorm does not write.
    // [spec:pgorm:sem:disjoint.schema+2]
    // [spec:pgorm:req:bindings.disjoint+1]
    // [spec:pgorm:sem:disjoint.unkinded]
    pub fn create(&self) -> Result<Vec<DdlStatement>, Error> {
        let kinded = self.kinded()?;
        self.check()?;
        self.keyable()?;
        let base = BaseTable::of(self, kinded);
        let kinds = self
            .kinds
            .iter()
            .filter_map(|kind| kind.table.as_ref().map(|table| (kind, table)))
            .map(|(kind, table)| kind_table(&base, kind, table))
            .collect::<Result<Vec<_>, Error>>()?;
        let alter = base.alter(&kinds)?;

        let mut create = body(&self.base, &|_| true)?;
        create.unique(
            TableKey::new(Name::runtime(base.key.first.clone()))
                .cols(base.key.rest.iter().cloned().map(Name::runtime))
                .col(Name::runtime(base.kind.clone()))
                .name(Name::runtime(derived_name(
                    "uq",
                    &base.name,
                    base.key.iter().chain([base.kind.as_str()]),
                ))),
        );
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
/// relations `keep` keeps, or from its columns alone.
// [spec:pgorm:sem:disjoint.schema+2]
fn body(
    table: &DisjointTable,
    keep: &dyn Fn(&RelationDef) -> bool,
) -> Result<TableCreateStatement, Error> {
    match &table.body {
        TableBody::Entity(write) => write(keep),
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
            Ok(stmt)
        }
    }
}

/// What every kind's table is written against: the base's table, key and
/// kind column.
// [spec:pgorm:sem:disjoint.schema+2]
struct BaseTable {
    table: TableName,
    name: String,
    label: String,
    key: KeyColumns,
    key_types: Vec<ColumnType>,
    kind: String,
    kind_type: ColumnType,
    declared: Vec<String>,
}

impl BaseTable {
    fn of(shape: &DisjointShape, kinded: &KindedBy) -> Self {
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

    /// The `ALTER TABLE` adding to the base, for each kind with a table, one
    /// generated column per key column, null for every other kind, and a
    /// foreign key from them to the kind's table: refused when a name it adds
    /// is taken, and absent when no kind has a table.
    // [spec:pgorm:sem:disjoint.schema+2]
    fn alter(&self, kinds: &[KindTable]) -> Result<Option<TableAlterStatement>, Error> {
        let mut added: Vec<(String, String)> = Vec::new();
        let mut columns = Vec::new();
        let mut keys = Vec::new();
        for kind in kinds {
            let generated = self
                .key
                .map(|column| bounded_identifier(format!("{}_{column}", kind.value)));
            let sources = self.key.iter().zip(&self.key_types);
            for (name, (column, column_type)) in generated.iter().zip(sources) {
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
            let name = derived_name("fk", &self.name, generated.iter());
            keys.push(deferrable_key(
                self.table.clone(),
                kind.table.clone(),
                generated.pairs(&kind.key),
                name,
                None,
            ));
        }
        let mut columns = columns.into_iter();
        let Some(first) = columns.next() else {
            return Ok(None);
        };
        let mut alter = Table::alter(self.table.clone()).add_column(first);
        for column in columns {
            alter.add_column(column);
        }
        for key in keys {
            alter.add_foreign_key(key);
        }
        Ok(Some(alter))
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

/// A kind with a table: its value, its table and key, and the statement
/// creating the table.
struct KindTable {
    value: String,
    table: TableName,
    key: KeyColumns,
    create: TableCreateStatement,
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
    create.col(&mut kind_column);
    let name = derived_name(
        "fk",
        &table.name().to_string(),
        key.iter().chain([base.kind.as_str()]),
    );
    create.foreign_key(deferrable_key(
        table.table.clone(),
        base.table.clone(),
        key.pairs(&base.key).and(
            Name::runtime(base.kind.clone()),
            Name::runtime(base.kind.clone()),
        ),
        name,
        Some(ForeignKeyAction::Cascade),
    ));
    Ok(KindTable {
        value,
        table: table.table.clone(),
        key,
        create,
    })
}

/// A key's column names in order, the first held apart because every key
/// has one.
struct KeyColumns {
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

    fn iter(&self) -> impl Iterator<Item = &str> {
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
