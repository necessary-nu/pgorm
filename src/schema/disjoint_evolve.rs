//! Changes to a kinded disjoint entity's kinds after its tables exist,
//! written from the declaration as it will be.

use super::{
    DdlStatement,
    disjoint::{BaseTable, KindKeys, KindTable},
    entity::create_enum_from_column_type,
};
use crate::{
    ColumnDefault, DisjointShape, Error, KindedDisjoint, Schema,
    executor::result_name::{bounded_identifier, derived_name},
};
use pgorm_query::{
    Check, ColumnDef, ColumnType, Expr, ExprTrait, IntoTableName, Name, Table, TableName, TypeName,
    extension::{IntoTypeRef, Type, TypeRef},
};
use std::fmt;

/// A change to a kinded disjoint entity's kinds, as the steps that make it,
/// in the order they run.
///
/// Each step's statements run in one transaction, and a step commits before
/// the next one starts. Run through pgorm-migration, each step is one
/// migration, and every one but the last answers
/// `commits_after()`. A step after which the program writes rows before the
/// next can succeed carries the [`DisjointGap`] saying which.
///
/// A statement a connection prepared before a step that recreates the kind
/// type, and that returns a value of the type or names it alone, fails once
/// the step commits: its result type changed (`0A000`), or the old type is
/// not found (`XX000`). A program running the steps itself therefore clears
/// its pool's statement caches after each
/// ([`DatabasePool::clear_statement_caches`](crate::DatabasePool::clear_statement_caches)),
/// as pgorm-migration does after each commit.
// [spec:pgorm:sem:disjoint.evolve+2]
#[derive(Debug, Clone)]
pub struct DisjointChange {
    steps: Vec<DisjointStep>,
}

impl DisjointChange {
    /// The steps, in the order they run.
    pub fn steps(&self) -> &[DisjointStep] {
        &self.steps
    }

    /// The steps, in the order they run, taken out of the change.
    pub fn into_steps(self) -> Vec<DisjointStep> {
        self.steps
    }
}

/// One step of a [`DisjointChange`]: statements run in order in one
/// transaction, and what the program writes once it commits, if anything.
// [spec:pgorm:sem:disjoint.evolve+2]
#[derive(Debug, Clone)]
pub struct DisjointStep {
    statements: Vec<DdlStatement>,
    gap: Option<DisjointGap>,
}

impl DisjointStep {
    /// The statements, in the order they run.
    pub fn statements(&self) -> &[DdlStatement] {
        &self.statements
    }

    /// The rows the program writes after this step commits and before the
    /// next step runs, which that step refuses the change without.
    pub fn gap(&self) -> Option<&DisjointGap> {
        self.gap.as_ref()
    }
}

/// What the program writes between two steps of a [`DisjointChange`]: a row
/// in a kind's table for every base row of that kind.
// [spec:pgorm:sem:disjoint.evolve+2]
#[derive(Debug, Clone)]
pub struct DisjointGap {
    kind: String,
    table: TableName,
    label: String,
}

impl DisjointGap {
    /// The kind whose base rows each need a row.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The kind's table, which the rows are written to.
    pub fn table(&self) -> &TableName {
        &self.table
    }
}

impl fmt::Display for DisjointGap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a row in `{}` for every base row of the kind `{}`",
            self.label, self.kind
        )
    }
}

impl Schema {
    /// Add `kind`, which `D` declares and the database's kind type does not
    /// hold yet: [`DisjointShape::add_kind`] of
    /// [`DisjointShape::of::<D>`](DisjointShape::of), in two steps for a
    /// kind with a table, since PostgreSQL uses an added value only once the
    /// transaction adding it has committed.
    ///
    /// An unkinded declaration does not compile, as with
    /// [`create_disjoint`](Self::create_disjoint):
    ///
    /// ```compile_fail,E0277
    /// # use pgorm::{Schema, tests_cfg::shop::{Shop, ShopKind}};
    /// let _ = Schema::new().add_disjoint_kind::<Shop>(ShopKind::WebShop);
    /// ```
    // [spec:pgorm:sem:disjoint.evolve+2]
    pub fn add_disjoint_kind<D: KindedDisjoint>(
        &self,
        kind: D::Kind,
    ) -> Result<DisjointChange, Error> {
        DisjointShape::of::<D>().add_kind(D::kind_name(kind))
    }

    /// Add `kind` in one step that recreates the kind type and rewrites the
    /// base and every kind table once: [`DisjointShape::add_kind_recreating`] of
    /// [`DisjointShape::of::<D>`](DisjointShape::of).
    // [spec:pgorm:sem:disjoint.evolve+2]
    pub fn add_disjoint_kind_recreating<D: KindedDisjoint>(
        &self,
        kind: D::Kind,
    ) -> Result<DisjointChange, Error> {
        DisjointShape::of::<D>().add_kind_recreating(D::kind_name(kind))
    }

    /// Remove `value`, which the database's kind type holds and `D` does not
    /// declare: [`DisjointShape::remove_kind`] of
    /// [`DisjointShape::of::<D>`](DisjointShape::of).
    // [spec:pgorm:sem:disjoint.evolve+2]
    pub fn remove_disjoint_kind<D: KindedDisjoint>(
        &self,
        value: &str,
    ) -> Result<DisjointChange, Error> {
        DisjointShape::of::<D>().remove_kind(value)
    }

    /// Give `kind`, a unit kind in the database, the table `D` declares for
    /// it: [`DisjointShape::add_kind_table`] of
    /// [`DisjointShape::of::<D>`](DisjointShape::of).
    // [spec:pgorm:sem:disjoint.evolve+2]
    pub fn add_disjoint_kind_table<D: KindedDisjoint>(
        &self,
        kind: D::Kind,
    ) -> Result<DisjointChange, Error> {
        DisjointShape::of::<D>().add_kind_table(D::kind_name(kind))
    }

    /// Drop `table`, the table of `kind`, which `D` declares a unit kind:
    /// [`DisjointShape::remove_kind_table`] of
    /// [`DisjointShape::of::<D>`](DisjointShape::of).
    // [spec:pgorm:sem:disjoint.evolve+2]
    pub fn remove_disjoint_kind_table<D: KindedDisjoint>(
        &self,
        kind: D::Kind,
        table: impl IntoTableName,
    ) -> Result<DisjointChange, Error> {
        DisjointShape::of::<D>().remove_kind_table(D::kind_name(kind), table.into_table_name())
    }
}

impl DisjointShape {
    /// Add the kind `value`, which this shape declares and the database's
    /// kind type does not hold yet.
    ///
    /// The first step is `ALTER TYPE ... ADD VALUE`, the value placed `AFTER`
    /// the kind declared before it, or `BEFORE` the one after it when it is
    /// the first. For a kind with a table, the second step is that table as
    /// [`create`](Self::create) writes it and the base's generated key
    /// columns and foreign key for it, which use the value, so its step
    /// follows the commit of the first; a kind without one uses the value
    /// nowhere, and its change is the first step alone.
    ///
    /// [`check`](Self::check) runs first and its refusal is this one's, and
    /// a value the shape does not declare is refused as [`Error::Type`]
    /// naming it.
    // [spec:pgorm:sem:disjoint.evolve+2]
    pub fn add_kind(&self, value: &str) -> Result<DisjointChange, Error> {
        let evolve = Evolve::of(self)?;
        let (position, kind) = self.kind(value)?;
        let add = Type::alter(evolve.kind_type.type_ref(&evolve.kind_type.name)).add_value(value);
        let add = match (position.checked_sub(1), self.kinds.get(position + 1)) {
            (Some(before), _) => add.after(self.kinds[before].value.clone()),
            (None, Some(next)) => add.before(next.value.clone()),
            (None, None) => add,
        };
        let mut steps = vec![DisjointStep::of(vec![DdlStatement::AlterType(add)])];
        if let Some((table, keys)) = evolve.table_of(&kind.value) {
            let mut statements = vec![DdlStatement::CreateTable(table.create.clone())];
            statements.extend(
                evolve
                    .base
                    .alter([keys], false)
                    .map(DdlStatement::AlterTable),
            );
            steps.push(DisjointStep::of(statements));
        }
        Ok(DisjointChange { steps })
    }

    /// Add the kind `value`, which this shape declares and the database's
    /// kind type does not hold yet, in one step that recreates the kind type
    /// and so rewrites the base and every kind table once, holding an
    /// exclusive lock on each while it runs: the price of one atomic change.
    ///
    /// [`check`](Self::check) runs first and its refusal is this one's, and
    /// a value the shape does not declare is refused as [`Error::Type`]
    /// naming it. The step is described at [`remove_kind`](Self::remove_kind),
    /// which takes it too.
    // [spec:pgorm:sem:disjoint.evolve+2]
    pub fn add_kind_recreating(&self, value: &str) -> Result<DisjointChange, Error> {
        let evolve = Evolve::of(self)?;
        let (_, kind) = self.kind(value)?;
        Ok(DisjointChange {
            steps: vec![DisjointStep::of(evolve.recreate(Some(&kind.value)))],
        })
    }

    /// Remove the value `value`, which the database's kind type holds and
    /// this shape does not declare, in one step.
    ///
    /// There is no `ALTER TYPE ... DROP VALUE`, so the step recreates the
    /// kind type. Its first statement adds to the base a `CHECK` refusing the
    /// value, `ck-{base}-{kind column}-{value}`, which PostgreSQL validates
    /// against every row, so the change fails while any base row has the
    /// kind (`23514`); the next drops it. Then a new enum of the shape's
    /// values, in order, is created as `{type}_next`; each kind table's kind
    /// column, a generated column PostgreSQL retypes by no `USING`, is
    /// dropped with its foreign key; in one statement, which rewrites the
    /// base once, the base's generated key columns, their foreign keys and its
    /// `UNIQUE` key are dropped, its kind column loses its default and is
    /// retyped `USING` its text, and its generated key columns are written
    /// again over the new type; the type it replaces is dropped and the new
    /// one given its name; and the base's default and `UNIQUE` key, each kind
    /// table's kind column and foreign key, and the base's foreign keys from
    /// its generated key columns, which reference the kind tables, are
    /// written again as [`create`](Self::create) writes them. A kind losing
    /// its table loses it through [`remove_kind_table`](Self::remove_kind_table)
    /// first: one still there keeps the replaced type in use, and the step
    /// fails.
    ///
    /// [`check`](Self::check) runs first and its refusal is this one's, and
    /// a value the shape declares is refused as [`Error::Type`] naming it.
    // [spec:pgorm:sem:disjoint.evolve+2]
    pub fn remove_kind(&self, value: &str) -> Result<DisjointChange, Error> {
        let evolve = Evolve::of(self)?;
        if self.kind(value).is_ok() {
            return Err(Error::Type(format!(
                "the disjoint entity over `{}` declares the kind `{value}`, so it is not \
                 removed",
                self.base.label
            )));
        }
        let name = derived_name("ck", &evolve.base.name, [evolve.base.kind.as_str(), value]);
        let check = Check::new(Expr::col(Name::runtime(evolve.base.kind.clone())).ne(value))
            .name(Name::runtime(name.clone()));
        let mut statements = vec![
            DdlStatement::AlterTable(Table::alter(evolve.base.table.clone()).add_check(check)),
            DdlStatement::AlterTable(
                Table::alter(evolve.base.table.clone()).drop_constraint(Name::runtime(name)),
            ),
        ];
        statements.extend(evolve.recreate(None));
        Ok(DisjointChange {
            steps: vec![DisjointStep::of(statements)],
        })
    }

    /// Give the kind `value`, which the database holds as a kind without a
    /// table, the table this shape declares for it, in two steps with rows
    /// for the program to write between them.
    ///
    /// The first step is the table as [`create`](Self::create) writes it,
    /// and the base's generated key columns with a foreign key to it added
    /// `NOT VALID`, since every base row of the kind lacks its row until the
    /// program writes it: the step's [`gap`](DisjointStep::gap) names them.
    /// The second is `VALIDATE CONSTRAINT`, which refuses the change while a
    /// row is still missing.
    ///
    /// [`check`](Self::check) runs first and its refusal is this one's, and
    /// a value the shape does not declare, or declares without a table, is
    /// refused as [`Error::Type`] naming it.
    // [spec:pgorm:sem:disjoint.evolve+2]
    pub fn add_kind_table(&self, value: &str) -> Result<DisjointChange, Error> {
        let evolve = Evolve::of(self)?;
        let (_, kind) = self.kind(value)?;
        let Some((table, keys)) = evolve.table_of(&kind.value) else {
            return Err(Error::Type(format!(
                "the disjoint entity over `{}` declares the kind `{value}` without a table, \
                 so it is given none",
                self.base.label
            )));
        };
        let mut added = vec![DdlStatement::CreateTable(table.create.clone())];
        added.extend(
            evolve
                .base
                .alter([keys], true)
                .map(DdlStatement::AlterTable),
        );
        let validate = Table::alter(evolve.base.table.clone())
            .validate_constraint(Name::runtime(keys.generated.name.clone()));
        Ok(DisjointChange {
            steps: vec![
                DisjointStep {
                    statements: added,
                    gap: Some(DisjointGap {
                        kind: kind.value.clone(),
                        table: table.table.clone(),
                        label: table.label.clone(),
                    }),
                },
                DisjointStep::of(vec![DdlStatement::AlterTable(validate)]),
            ],
        })
    }

    /// Drop `table`, the table of the kind `value`, which this shape declares
    /// without one, and the base's generated key columns and foreign key for
    /// it, in one step. Every row of the table is lost.
    ///
    /// [`check`](Self::check) runs first and its refusal is this one's, and
    /// a value the shape does not declare, or declares with a table, is
    /// refused as [`Error::Type`] naming it.
    // [spec:pgorm:sem:disjoint.evolve+2]
    pub fn remove_kind_table(
        &self,
        value: &str,
        table: TableName,
    ) -> Result<DisjointChange, Error> {
        let evolve = Evolve::of(self)?;
        let (_, kind) = self.kind(value)?;
        if let Some(declared) = &kind.table {
            return Err(Error::Type(format!(
                "the disjoint entity over `{}` declares the kind `{value}` with the table \
                 `{}`, so its table is not dropped",
                self.base.label, declared.label
            )));
        }
        let generated = evolve.base.generated(&kind.value);
        let mut alter =
            Table::alter(evolve.base.table.clone()).drop_constraint(Name::runtime(generated.name));
        for column in generated.columns.iter() {
            alter.drop_column(Name::runtime(column));
        }
        Ok(DisjointChange {
            steps: vec![DisjointStep::of(vec![
                DdlStatement::AlterTable(alter),
                DdlStatement::DropTable(Table::drop(table).to_owned()),
            ])],
        })
    }
}

impl DisjointStep {
    fn of(statements: Vec<DdlStatement>) -> Self {
        Self {
            statements,
            gap: None,
        }
    }
}

/// What every change is written against: the base, each kind with a table,
/// its table and the base's generated key columns and foreign key for it, as
/// [`DisjointShape::create`] writes them, and the kind type.
// [spec:pgorm:sem:disjoint.evolve+2]
struct Evolve<'s> {
    shape: &'s DisjointShape,
    base: BaseTable,
    tables: Vec<(KindTable, KindKeys)>,
    kind_type: KindType,
}

/// The kind type: its name, its schema, and its values in order.
struct KindType {
    name: Name,
    schema: Option<Name>,
    variants: Vec<Name>,
}

impl KindType {
    /// The type named `name` in the kind type's schema.
    fn type_ref(&self, name: &Name) -> TypeRef {
        match &self.schema {
            Some(schema) => (schema.clone(), name.clone()).into_type_ref(),
            None => name.clone().into_type_ref(),
        }
    }

    /// The enum of the kind type's values named `name` in its schema.
    fn column_type(&self, name: &Name) -> ColumnType {
        ColumnType::Enum {
            name: name.clone(),
            schema: self.schema.clone(),
            variants: self.variants.clone(),
        }
    }
}

impl<'s> Evolve<'s> {
    /// The shape's base and kind tables, refused as [`DisjointShape::create`]
    /// refuses them: an unkinded shape, then [`DisjointShape::check`]'s
    /// refusal, a key the constraints cannot be written over, a name
    /// generated onto a declared column or twice, and a kind type that is no
    /// database enum.
    fn of(shape: &'s DisjointShape) -> Result<Self, Error> {
        let kinded = shape.kinded()?;
        shape.check()?;
        shape.keyable()?;
        shape.keys_named_apart()?;
        let ColumnType::Enum {
            name,
            schema,
            variants,
        } = &kinded.kind_type
        else {
            return Err(Error::Type(format!(
                "the kind type of the disjoint entity over `{}` is not a database enum, so \
                 there is no type to change",
                shape.base.label
            )));
        };
        let base = BaseTable::of(shape, kinded);
        let kinds = base.kind_tables(shape)?;
        let keys = base.kind_keys(&kinds)?;
        Ok(Self {
            shape,
            base,
            tables: kinds.into_iter().zip(keys).collect(),
            kind_type: KindType {
                name: name.clone(),
                schema: schema.clone(),
                variants: variants.clone(),
            },
        })
    }

    /// The table of the kind `value` and the base's generated key columns
    /// and foreign key for it, or `None` for a kind without a table.
    fn table_of(&self, value: &str) -> Option<(&KindTable, &KindKeys)> {
        self.tables
            .iter()
            .find(|(table, _)| table.value == value)
            .map(|(table, keys)| (table, keys))
    }

    /// The statements recreating the kind type with the shape's values in
    /// order, as [`DisjointShape::remove_kind`] describes them, the kind
    /// `added` new among them, with its table where it has one.
    // [spec:pgorm:sem:disjoint.evolve+2]
    fn recreate(&self, added: Option<&str>) -> Vec<DdlStatement> {
        let base = &self.base;
        let kind = || Name::runtime(base.kind.clone());
        let next = Name::runtime(bounded_identifier(format!(
            "{}_next",
            self.kind_type.name.to_string()
        )));
        let kept: Vec<&(KindTable, KindKeys)> = self
            .tables
            .iter()
            .filter(|(table, _)| Some(table.value.as_str()) != added)
            .collect();

        let mut statements: Vec<DdlStatement> =
            create_enum_from_column_type(&self.kind_type.column_type(&next))
                .map(DdlStatement::CreateType)
                .into_iter()
                .collect();
        for (table, _) in &kept {
            let mut drop = Table::alter(table.table.clone())
                .drop_constraint(Name::runtime(table.key_name.clone()));
            drop.drop_column(kind());
            statements.push(DdlStatement::AlterTable(drop));
        }
        let mut retype =
            Table::alter(base.table.clone()).drop_constraint(Name::runtime(base.unique_name()));
        for (_, keys) in &kept {
            retype.drop_constraint(Name::runtime(keys.generated.name.clone()));
            for column in keys.generated.columns.iter() {
                retype.drop_column(Name::runtime(column));
            }
        }
        let mut next_name = TypeName::new(next.clone());
        if let Some(schema) = &self.kind_type.schema {
            next_name = next_name.schema(schema.clone());
        }
        retype.drop_default(kind()).alter_column_type_using(
            kind(),
            self.kind_type.column_type(&next),
            Expr::col(kind())
                .cast_as(Name::runtime("text"))
                .cast_as_type(next_name),
        );
        // Retyping the kind column and adding a STORED column each rewrite the
        // base; in one statement PostgreSQL rewrites it once. The columns'
        // expressions take the new type, which the rename below keeps.
        for column in self.tables.iter().flat_map(|(_, keys)| &keys.columns) {
            retype.add_column(column.clone());
        }
        statements.push(DdlStatement::AlterTable(retype));
        statements.push(DdlStatement::DropType(
            Type::drop(self.kind_type.type_ref(&self.kind_type.name)).to_owned(),
        ));
        statements.push(DdlStatement::AlterType(
            Type::alter(self.kind_type.type_ref(&next)).rename_to(self.kind_type.name.clone()),
        ));

        let mut restore = Table::alter(base.table.clone()).add_unique(base.unique());
        if let Some(ColumnDefault::Expr(default)) = self
            .shape
            .kind_column()
            .and_then(|column| column.def.default.as_ref())
        {
            restore.modify_column(ColumnDef::new(kind()).default(default.clone()));
        }
        statements.push(DdlStatement::AlterTable(restore));
        for (table, _) in &kept {
            let mut add = Table::alter(table.table.clone()).add_column(table.kind_column.clone());
            add.add_foreign_key(table.foreign_key.clone());
            statements.push(DdlStatement::AlterTable(add));
        }
        if let Some((table, _)) = added.and_then(|value| self.table_of(value)) {
            statements.push(DdlStatement::CreateTable(table.create.clone()));
        }
        let mut foreign_keys = self.tables.iter().map(|(_, keys)| keys.foreign_key.clone());
        if let Some(first) = foreign_keys.next() {
            let mut keyed = Table::alter(base.table.clone()).add_foreign_key(first);
            for foreign_key in foreign_keys {
                keyed.add_foreign_key(foreign_key);
            }
            statements.push(DdlStatement::AlterTable(keyed));
        }
        statements
    }
}
