//! A generated column and which of PostgreSQL's two kinds it is. `ColumnDef`'s
//! file is over its function cap, so the setter sits beside it here.

use crate::SimpleExpr;

use super::{ColumnDef, ColumnSpec};

/// Which of PostgreSQL's two kinds a generated column is: computed when a row
/// is written and kept on disk, or computed each time it is read.
///
/// The kind is always written out, because the server's default is not one
/// pgorm can lean on: PostgreSQL 19 reads a generated column that says
/// nothing as `VIRTUAL`, where 17 refused one that did not say `STORED`. A
/// closed pair rather than a `stored: bool`, for the reason
/// [`IdentityGeneration`](crate::IdentityGeneration) is one
/// (`[dec:pgorm:invalid-states-unrepresentable]`): there is no third kind,
/// and a flag reads backwards at the call site.
// [spec:pgorm:req:sql.ddl.column-def+14]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedKind {
    /// `STORED`: computed when the row is inserted or updated, and kept like
    /// any other column, so it can be indexed and keyed.
    Stored,
    /// `VIRTUAL`: computed when the row is read, taking no space. PostgreSQL 18
    /// is the first release that has it, and it refuses more around one than
    /// around a stored column: an index, a key or a foreign key on it
    /// (`0A000`), a user-defined or domain type (`0A000`), and a user-defined
    /// function in its expression (`0A000`).
    Virtual,
}

impl GeneratedKind {
    /// The keyword PostgreSQL spells this kind with — `STORED` or `VIRTUAL`.
    // [spec:pgorm:req:sql.ddl.column-def+14]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Stored => "STORED",
            Self::Virtual => "VIRTUAL",
        }
    }
}

impl ColumnDef {
    /// Compute the column from the others in its row —
    /// `GENERATED ALWAYS AS (<expr>) { STORED | VIRTUAL }` — as the `kind`
    /// says.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(ColumnDef::new(Glyph::Id).integer().not_null())
    ///         .col(
    ///             ColumnDef::new(Glyph::Aspect)
    ///                 .integer()
    ///                 .generated(Expr::col(Glyph::Id).mul(2), GeneratedKind::Stored)
    ///         )
    ///         .col(
    ///             ColumnDef::new(Glyph::Tokens)
    ///                 .integer()
    ///                 .generated(Expr::col(Glyph::Id).add(1), GeneratedKind::Virtual)
    ///         )
    ///         .to_string(),
    ///     [
    ///         r#"CREATE TABLE "glyph" ( "id" integer NOT NULL,"#,
    ///         r#""aspect" integer GENERATED ALWAYS AS ("id" * 2) STORED,"#,
    ///         r#""tokens" integer GENERATED ALWAYS AS ("id" + 1) VIRTUAL )"#,
    ///     ]
    ///     .join(" "),
    /// );
    /// ```
    ///
    /// The kind has no default to fall back on: PostgreSQL's changed between
    /// releases, so the statement names the kind rather than leaving a reader
    /// to know which release's default applies.
    ///
    /// ```compile_fail,E0061
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// ColumnDef::new(Glyph::Aspect).integer().generated(Expr::val(1));
    /// ```
    ///
    /// What the server refuses is its own knowledge, and it refuses it by name
    /// when the table is created: an expression that is not immutable
    /// (`42P17`), one that reads another generated column (`42P17`) or holds a
    /// subquery (`0A000`), and a generated column that also has a `DEFAULT` or
    /// an identity (`42601`). A row cannot write the column either: an insert
    /// or update that supplies a value other than `DEFAULT` is refused
    /// (`428C9`).
    // [spec:pgorm:req:sql.ddl.column-def+14]
    pub fn generated<T>(&mut self, expr: T, kind: GeneratedKind) -> &mut Self
    where
        T: Into<SimpleExpr>,
    {
        self.spec.push(ColumnSpec::Generated {
            expr: expr.into(),
            kind,
        });
        self
    }
}
