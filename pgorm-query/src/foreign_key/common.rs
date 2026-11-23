use crate::{Enforcement, types::*};

/// Specification of a foreign key
///
/// A foreign key maps columns of one table onto columns of another, so both
/// tables and at least one `(column, referenced column)` pair are taken by
/// [`TableForeignKey::new`]: PostgreSQL rejects `ALTER TABLE  ADD FOREIGN KEY`,
/// `REFERENCES  ()`, `FOREIGN KEY ()` and `REFERENCES "t" ()` alike, so none of
/// the four has a field to be left unset in
/// (`[dec:pgorm:invalid-states-unrepresentable]`).
///
/// The two column lists are one list of pairs rather than two lists, so the
/// referencing and referenced sides cannot disagree in length — a mismatch the
/// grammar accepts and parse analysis rejects, and so one no parser oracle can
/// catch. Further pairs are appended with [`TableForeignKey::col`], and a
/// temporal key's `PERIOD` pair, which closes both lists, is set with
/// [`TableForeignKey::period`].
// [spec:pgorm:req:sql.ddl.foreign-key+11]
#[derive(Debug, Clone)]
pub struct TableForeignKey {
    pub(crate) name: Option<Name>,
    pub(crate) table: TableName,
    pub(crate) ref_table: TableName,
    pub(crate) first: (Name, Name),
    pub(crate) rest: Vec<(Name, Name)>,
    /// The `PERIOD` pair, kept apart from the others so it is always the last
    /// one written, on both sides at once.
    pub(crate) period: Option<(Name, Name)>,
    pub(crate) on_delete: Option<ForeignKeyAction>,
    pub(crate) on_update: Option<ForeignKeyAction>,
    pub(crate) deferrability: Option<Deferrability>,
    pub(crate) enforcement: Option<Enforcement>,
}

/// Foreign key on update & on delete actions
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum ForeignKeyAction {
    Restrict,
    Cascade,
    SetNull,
    NoAction,
    SetDefault,
}

/// When a constraint's check runs, and whether a transaction may move it.
///
/// The three variants are PostgreSQL's three reachable states, not two
/// independent flags: `INITIALLY DEFERRED` is only grammatical on a
/// `DEFERRABLE` constraint, so the pair that would name a constraint both
/// undeferrable and initially deferred does not construct
/// (`[dec:pgorm:invalid-states-unrepresentable]`).
///
/// A foreign key takes it through [`TableForeignKey::deferrability`], and a
/// primary or unique key through
/// [`TableKey::deferrability`](crate::TableKey::deferrability).
/// Nothing else can: PostgreSQL never defers a `CHECK` or `NOT NULL`
/// constraint, and `CREATE UNIQUE INDEX` has no clause for it.
// [spec:pgorm:req:sql.ddl.deferrability+4]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deferrability {
    /// `NOT DEFERRABLE` — checked at once, and no transaction can postpone
    /// it: a foreign key at the end of each statement, a unique or primary key
    /// as each row is written. This is the server's default, so it renders
    /// only because a caller said it.
    NotDeferrable,
    /// `DEFERRABLE INITIALLY IMMEDIATE` — checked at the end of each
    /// statement, so a unique key lets one statement pass through a
    /// duplicate the default would refuse mid-row, and
    /// `SET CONSTRAINTS ... DEFERRED` can move it to commit time for the rest
    /// of a transaction.
    DeferrableInitiallyImmediate,
    /// `DEFERRABLE INITIALLY DEFERRED` — checked once, at commit. This is what
    /// lets two tables reference each other, and what lets a batch reorder
    /// rows through a state no single statement could hold.
    DeferrableInitiallyDeferred,
}

impl Deferrability {
    /// The clause this state renders as, with the space that separates it
    /// from the constraint it follows.
    // [spec:pgorm:req:sql.ddl.deferrability+4]
    pub(crate) fn clause(self) -> &'static str {
        match self {
            Self::NotDeferrable => " NOT DEFERRABLE",
            Self::DeferrableInitiallyImmediate => " DEFERRABLE INITIALLY IMMEDIATE",
            Self::DeferrableInitiallyDeferred => " DEFERRABLE INITIALLY DEFERRED",
        }
    }
}

impl TableForeignKey {
    /// Construct a foreign key from the two tables it relates and the first
    /// `(column, referenced column)` pair it maps
    pub fn new<T, C, R, S>(table: T, column: C, ref_table: R, ref_column: S) -> Self
    where
        T: IntoTableName,
        C: IntoName,
        R: IntoTableName,
        S: IntoName,
    {
        Self {
            name: None,
            table: table.into_table_name(),
            ref_table: ref_table.into_table_name(),
            first: (column.into_name(), ref_column.into_name()),
            rest: Vec::new(),
            period: None,
            on_delete: None,
            on_update: None,
            deferrability: None,
            enforcement: None,
        }
    }

    /// Set foreign key name
    pub fn name<T>(&mut self, name: T) -> &mut Self
    where
        T: IntoName,
    {
        self.name = Some(name.into_name());
        self
    }

    /// Map a further column onto a further referenced column, as a composite
    /// key requires
    pub fn col<C, S>(&mut self, column: C, ref_column: S) -> &mut Self
    where
        C: IntoName,
        S: IntoName,
    {
        self.rest.push((column.into_name(), ref_column.into_name()));
        self
    }

    /// Match `column` to `ref_column` as periods — `PERIOD` on both sides,
    /// PostgreSQL 18's temporal foreign key — replacing any such pair already
    /// set; [`ForeignKeyCreateStatement::period`](crate::ForeignKeyCreateStatement::period)
    /// says what it means.
    // [spec:pgorm:req:sql.ddl.foreign-key+11]
    pub fn period<C, S>(&mut self, column: C, ref_column: S) -> &mut Self
    where
        C: IntoName,
        S: IntoName,
    {
        self.period = Some((column.into_name(), ref_column.into_name()));
        self
    }

    /// Set on delete action
    pub fn on_delete(&mut self, action: ForeignKeyAction) -> &mut Self {
        self.on_delete = Some(action);
        self
    }

    /// Set on update action
    pub fn on_update(&mut self, action: ForeignKeyAction) -> &mut Self {
        self.on_update = Some(action);
        self
    }

    /// Set when this key's check runs
    pub fn deferrability(&mut self, deferrability: Deferrability) -> &mut Self {
        self.deferrability = Some(deferrability);
        self
    }

    /// Say whether the server holds rows to this key
    /// (`[spec:pgorm:req:sql.ddl.enforcement+1]`), replacing any already set.
    // [spec:pgorm:req:sql.ddl.enforcement+1]
    pub fn enforcement(&mut self, enforcement: Enforcement) -> &mut Self {
        self.enforcement = Some(enforcement);
        self
    }

    /// Retarget this key at `table`, as an embedding into a `CREATE TABLE` does
    pub(crate) fn retarget(&mut self, table: TableName) {
        self.table = table;
    }

    /// The pairs matched for equality in declaration order, of which there is
    /// at least one. The `PERIOD` pair is not among them:
    /// [`get_period`](Self::get_period) reads it.
    pub fn columns(&self) -> impl Iterator<Item = &(Name, Name)> {
        std::iter::once(&self.first).chain(self.rest.iter())
    }

    /// The `(column, referenced column)` pair matched as periods, if this is a
    /// temporal foreign key.
    // [spec:pgorm:req:sql.ddl.foreign-key+11]
    pub fn get_period(&self) -> Option<&(Name, Name)> {
        self.period.as_ref()
    }

    pub fn get_table(&self) -> &TableName {
        &self.table
    }

    pub fn get_ref_table(&self) -> &TableName {
        &self.ref_table
    }

    pub fn get_columns(&self) -> Vec<String> {
        self.columns().map(|(col, _)| col.to_string()).collect()
    }

    pub fn get_ref_columns(&self) -> Vec<String> {
        self.columns().map(|(_, col)| col.to_string()).collect()
    }

    pub fn get_on_delete(&self) -> Option<ForeignKeyAction> {
        self.on_delete
    }

    pub fn get_on_update(&self) -> Option<ForeignKeyAction> {
        self.on_update
    }

    /// Whether the server enforces this key, if the caller said.
    pub fn get_enforcement(&self) -> Option<Enforcement> {
        self.enforcement
    }

    /// When this key's check runs, if the caller said.
    // [spec:pgorm:req:sql.ddl.foreign-key+11]
    pub fn get_deferrability(&self) -> Option<Deferrability> {
        self.deferrability
    }
}
