//! The tables of a disjoint entity, and the constraints that hold each base
//! row to exactly its own kind's table.

use super::{
    DdlStatement,
    entity::{create_enum_from_active_enum, create_table_from_entity, table_from_entity},
};
use crate::{
    ActiveEnum, ColumnPairs, ColumnTrait, ColumnType, Disjoint, EntityName, EntityTrait, Error,
    Iterable, KindModel, KindVisitor, PrimaryKeyToColumn, RelationDef, RelationType, Schema,
    StaticName, check_disjoint, executor::result_name::bounded_identifier, table_label,
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
    /// as the statements to run, in the order they run.
    ///
    /// [`check_disjoint`] runs first, and its refusal is this one's. The
    /// statements are the kind type; the base's table as
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
    // [spec:pgorm:sem:disjoint.schema+1]
    pub fn create_disjoint<D: Disjoint>(&self) -> Result<Vec<DdlStatement>, Error> {
        check_disjoint::<D>()?;
        let base = BaseTable::of::<D>()?;
        let kinds = D::Kind::iter()
            .map(|kind| D::visit_kind(kind, KindTableOf(&base)))
            .filter_map(Result::transpose)
            .collect::<Result<Vec<_>, Error>>()?;
        let alter = base.alter(&kinds)?;

        let mut create = create_table_from_entity(D::Base::default())?;
        create.unique(
            TableKey::new(Name::runtime(base.key.first.clone()))
                .cols(base.key.rest.iter().cloned().map(Name::runtime))
                .col(Name::runtime(base.kind.clone()))
                .name(Name::runtime(constraint_name(
                    "uq",
                    &base.name,
                    base.key.iter().chain([base.kind.as_str()]),
                ))),
        );
        let mut statements = vec![
            DdlStatement::CreateType(create_enum_from_active_enum::<D::Kind>()?),
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

/// What every kind's table is written against: the base's table, key and
/// kind column.
// [spec:pgorm:sem:disjoint.schema+1]
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
    fn of<D: Disjoint>() -> Result<Self, Error> {
        let entity = D::Base::default();
        let key_types = <D::Base as EntityTrait>::PrimaryKey::iter()
            .map(|key| key.into_column().def().get_column_type().clone())
            .collect();
        Ok(Self {
            table: entity.table_ref(),
            name: entity.table_name().to_owned(),
            label: table_label(&entity),
            key: KeyColumns::of(&entity)?,
            key_types,
            kind: D::kind_column().as_str().to_owned(),
            kind_type: D::Kind::db_type().get_column_type().clone(),
            declared: column_names::<D::Base>(),
        })
    }

    /// The `ALTER TABLE` adding to the base, for each kind with a table, one
    /// generated column per key column, null for every other kind, and a
    /// foreign key from them to the kind's table: refused when a name it adds
    /// is taken, and absent when no kind has a table.
    // [spec:pgorm:sem:disjoint.schema+1]
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
            let name = constraint_name("fk", &self.name, generated.iter());
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
    // [spec:pgorm:sem:disjoint.schema+1]
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

/// Writes a kind's table against the base, and nothing for a kind without
/// one.
struct KindTableOf<'a>(&'a BaseTable);

impl<D: Disjoint> KindVisitor<D> for KindTableOf<'_> {
    type Output = Result<Option<KindTable>, Error>;

    // [spec:pgorm:sem:disjoint.schema+1]
    fn table<M: KindModel<D>>(self, kind: D::Kind) -> Self::Output {
        let base = self.0;
        let entity = M::Entity::default();
        let value = kind.to_value();
        if column_names::<M::Entity>().contains(&base.kind) {
            let label = table_label(&entity);
            return Err(Error::Type(format!(
                "`{label}`.`{}`, the kind column the disjoint schema adds to the table of the \
                 kind `{value}`, is a column `{label}` declares",
                base.kind
            )));
        }
        let key = KeyColumns::of(&entity)?;
        let mut create =
            table_from_entity(entity, |relation| !is_key_to_base(relation, base, &key))?;
        let mut kind_column =
            ColumnDef::new_with_type(Name::runtime(base.kind.clone()), base.kind_type.clone());
        kind_column.generated(Expr::val(value.clone()), GeneratedKind::Stored);
        create.col(&mut kind_column);
        let name = constraint_name(
            "fk",
            entity.table_name(),
            key.iter().chain([base.kind.as_str()]),
        );
        create.foreign_key(deferrable_key(
            entity.table_ref(),
            base.table.clone(),
            key.pairs(&base.key).and(
                Name::runtime(base.kind.clone()),
                Name::runtime(base.kind.clone()),
            ),
            name,
            Some(ForeignKeyAction::Cascade),
        ));
        Ok(Some(KindTable {
            value,
            table: entity.table_ref(),
            key,
            create,
        }))
    }

    fn unit(self, _: D::Kind) -> Self::Output {
        Ok(None)
    }
}

/// A key's column names in order, the first held apart because every key
/// has one.
struct KeyColumns {
    first: String,
    rest: Vec<String>,
}

impl KeyColumns {
    /// `E`'s primary key, refused when it has no column.
    fn of<E: EntityTrait>(entity: &E) -> Result<Self, Error> {
        let mut names = E::PrimaryKey::iter().map(|key| key.into_column().as_str().to_owned());
        let Some(first) = names.next() else {
            return Err(Error::Type(format!(
                "`{}` has no primary key, which a disjoint entity's tables are keyed by",
                table_label(entity)
            )));
        };
        Ok(Self {
            first,
            rest: names.collect(),
        })
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
// [spec:pgorm:sem:disjoint.schema+1]
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

/// `{kind}-{table}-{columns}`, `-`-joined as a derived foreign key's name
/// is, and bounded to PostgreSQL's 63 bytes.
// [spec:pgorm:sem:disjoint.schema+1]
fn constraint_name<'a>(kind: &str, table: &str, columns: impl Iterator<Item = &'a str>) -> String {
    let mut name = format!("{kind}-{table}");
    for column in columns {
        name.push('-');
        name.push_str(column);
    }
    bounded_identifier(name)
}

/// The names of the columns `E` declares.
fn column_names<E: EntityTrait>() -> Vec<String> {
    E::Column::iter()
        .map(|column| column.as_str().to_owned())
        .collect()
}
