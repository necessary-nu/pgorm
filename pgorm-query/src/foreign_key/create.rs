use crate::{
    Deferrability, Enforcement, ForeignKeyAction, QueryBuilder, TableForeignKey, types::*,
};

/// Create a foreign key constraint for an existing table
///
/// The two tables and the first `(column, referenced column)` pair are taken by
/// [`ForeignKey::create`], because PostgreSQL rejects every render that leaves
/// one of them out; further pairs are appended with
/// [`ForeignKeyCreateStatement::col`]. A half-named key does not construct:
///
/// ```compile_fail,E0061
/// use pgorm_query::{*, tests_cfg::*};
///
/// ForeignKey::create().name(Name::runtime("FK_character_font"));
/// ```
///
/// and neither does one that names the referencing side alone:
///
/// ```compile_fail,E0061
/// use pgorm_query::{*, tests_cfg::*};
///
/// ForeignKey::create(Char::Table, Char::FontId);
/// ```
///
/// [`ForeignKey::create`]: crate::ForeignKey::create
///
/// # Examples
///
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// let foreign_key = ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
///     .name(Name::runtime("FK_character_font"))
///     .to_owned();
///
/// assert_eq!(
///     foreign_key.to_string(),
///     [
///         r#"ALTER TABLE "character" ADD CONSTRAINT "FK_character_font""#,
///         r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// Composite key
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// let foreign_key = ForeignKey::create(Char::Table, Char::FontId, Glyph::Table, Char::FontId)
///     .name(Name::runtime("FK_character_glyph"))
///     .col(Char::Id, Glyph::Id)
///     .to_owned();
///
/// assert_eq!(
///     foreign_key.to_string(),
///     [
///         r#"ALTER TABLE "character" ADD CONSTRAINT "FK_character_glyph""#,
///         r#"FOREIGN KEY ("font_id", "id") REFERENCES "glyph" ("font_id", "id")"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// let foreign_key = ForeignKey::create(
///     Character::Table,
///     Character::Id,
///     Character::Table,
///     Character::Id,
/// )
/// .name(Name::runtime("FK_character_id"))
/// .to_owned();
///
/// assert_eq!(
///     foreign_key.to_string(),
///     r#"ALTER TABLE "character" ADD CONSTRAINT "FK_character_id" FOREIGN KEY ("id") REFERENCES "character" ("id")"#
/// );
/// ```
///
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// let foreign_key = ForeignKey::create(
///     Character::Table,
///     Character::Id,
///     Character::Table,
///     Character::Id,
/// )
/// .name(Name::runtime("FK_character_id"))
/// .on_delete(ForeignKeyAction::Cascade)
/// .on_update(ForeignKeyAction::Cascade)
/// .to_owned();
///
/// assert_eq!(
///     foreign_key.to_string(),
///     r#"ALTER TABLE "character" ADD CONSTRAINT "FK_character_id" FOREIGN KEY ("id") REFERENCES "character" ("id") ON DELETE CASCADE ON UPDATE CASCADE"#
/// );
/// ```
// [spec:pgorm:req:sql.ddl.foreign-key+11]
#[derive(Debug, Clone)]
pub struct ForeignKeyCreateStatement {
    pub(crate) foreign_key: TableForeignKey,
}

impl ForeignKeyCreateStatement {
    /// Construct a new [`ForeignKeyCreateStatement`] over the two tables it
    /// relates and the first `(column, referenced column)` pair it maps
    pub fn new<T, C, R, S>(table: T, column: C, ref_table: R, ref_column: S) -> Self
    where
        T: IntoTableName,
        C: IntoName,
        R: IntoTableName,
        S: IntoName,
    {
        Self {
            foreign_key: TableForeignKey::new(table, column, ref_table, ref_column),
        }
    }

    /// Set foreign key name
    pub fn name<T>(&mut self, name: T) -> &mut Self
    where
        T: IntoName,
    {
        self.foreign_key.name(name);
        self
    }

    /// Map a further column onto a further referenced column, as a composite
    /// key requires
    pub fn col<C, S>(&mut self, column: C, ref_column: S) -> &mut Self
    where
        C: IntoName,
        S: IntoName,
    {
        self.foreign_key.col(column, ref_column);
        self
    }

    /// Match `column` to `ref_column` as periods, closing both column lists
    /// with `PERIOD`: PostgreSQL 18's temporal foreign key
    /// (`[spec:pgorm:req:sql.ddl.foreign-key+11]`). A later call replaces the
    /// pair.
    ///
    /// The other pairs still match by equality, and the referencing row's
    /// period, a range or multirange, must be covered by the union of the
    /// periods of the referenced rows that match them: a child row may span
    /// two adjacent parent rows, and is refused (`23503`) where no parent row
    /// covers part of it, or its period is empty. The referenced key must be a
    /// temporal one, ending in the same column
    /// [`WITHOUT OVERLAPS`](crate::TableKey::without_overlaps) (`42830`
    /// otherwise, as it is for a plain foreign key onto such a key).
    ///
    /// The pair is held apart from the others and written last, whatever
    /// order the calls come in, because PostgreSQL takes `PERIOD` on the last
    /// column alone (`42601`); one call writes both sides, which PostgreSQL
    /// refuses to mix (`42830`); and the constructor's pair comes before it,
    /// so `PERIOD` is never the only column (`42601`).
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::*;
    ///
    /// let foreign_key = ForeignKey::create(
    ///     Name::runtime("booking"),
    ///     Name::runtime("room_id"),
    ///     Name::runtime("room"),
    ///     Name::runtime("id"),
    /// )
    /// .period(Name::runtime("during"), Name::runtime("valid_at"))
    /// .to_string();
    ///
    /// assert_eq!(
    ///     foreign_key,
    ///     [
    ///         r#"ALTER TABLE "booking" ADD"#,
    ///         r#"FOREIGN KEY ("room_id", PERIOD "during")"#,
    ///         r#"REFERENCES "room" ("id", PERIOD "valid_at")"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    ///
    /// PostgreSQL 19 runs a temporal key's referential action as `NO ACTION`
    /// only, and refuses any other [`ForeignKeyAction`] on it, `ON DELETE` or
    /// `ON UPDATE` (`0A000`); deferrability and
    /// [`enforcement`](Self::enforcement) it takes as a plain key does.
    // [spec:pgorm:req:sql.ddl.foreign-key+11]
    pub fn period<C, S>(&mut self, column: C, ref_column: S) -> &mut Self
    where
        C: IntoName,
        S: IntoName,
    {
        self.foreign_key.period(column, ref_column);
        self
    }

    /// Set on delete action
    pub fn on_delete(&mut self, action: ForeignKeyAction) -> &mut Self {
        self.foreign_key.on_delete(action);
        self
    }

    /// Set on update action
    pub fn on_update(&mut self, action: ForeignKeyAction) -> &mut Self {
        self.foreign_key.on_update(action);
        self
    }

    /// Set when this key's check runs, and whether a transaction may move it
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let foreign_key = ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
    ///     .name(Name::runtime("FK_character_font"))
    ///     .deferrability(Deferrability::DeferrableInitiallyDeferred)
    ///     .to_string();
    ///
    /// assert_eq!(
    ///     foreign_key,
    ///     [
    ///         r#"ALTER TABLE "character" ADD CONSTRAINT "FK_character_font""#,
    ///         r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
    ///         r#"DEFERRABLE INITIALLY DEFERRED"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn deferrability(&mut self, deferrability: Deferrability) -> &mut Self {
        self.foreign_key.deferrability(deferrability);
        self
    }

    /// Say whether the server holds rows to this key: `NOT ENFORCED` records
    /// it and checks nothing (`[spec:pgorm:req:sql.ddl.enforcement+1]`).
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let foreign_key = ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
    ///     .name(Name::runtime("FK_character_font"))
    ///     .deferrability(Deferrability::DeferrableInitiallyDeferred)
    ///     .enforcement(Enforcement::NotEnforced)
    ///     .to_string();
    ///
    /// assert_eq!(
    ///     foreign_key,
    ///     [
    ///         r#"ALTER TABLE "character" ADD CONSTRAINT "FK_character_font""#,
    ///         r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
    ///         r#"DEFERRABLE INITIALLY DEFERRED NOT ENFORCED"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.enforcement+1]
    pub fn enforcement(&mut self, enforcement: Enforcement) -> &mut Self {
        self.foreign_key.enforcement(enforcement);
        self
    }

    pub fn get_foreign_key(&self) -> &TableForeignKey {
        &self.foreign_key
    }
}

/// Renders the statement with every value inlined as an escaped SQL literal.
/// This is its only rendering: it exposes no placeholder-emitting build, so
/// nothing here is left to bind.
// [spec:pgorm:req:sql.ddl+12]
impl std::fmt::Display for ForeignKeyCreateStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_foreign_key_create_statement(self, &mut sql);
        f.write_str(&sql)
    }
}
