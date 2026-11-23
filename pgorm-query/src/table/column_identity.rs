//! An identity column whose sequence takes options. `ColumnDef`'s file is over
//! its function cap, so the setter sits beside it here.

use crate::SequenceOptions;

use super::{ColumnDef, ColumnSpec, IdentityGeneration};

impl ColumnDef {
    /// Generate the column's values from a sequence defined by `options` —
    /// `GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY ( <options> )`.
    ///
    /// The options are the ones a standalone sequence takes, from the same
    /// [`SequenceOption`](crate::SequenceOption) vocabulary, so a column's
    /// sequence and a [`Sequence::create`](crate::Sequence::create) are
    /// spelled alike. Two clauses of a standalone sequence have no place here
    /// and are not options: the column's own type is what the sequence counts
    /// in, and the column owns it. [`identity`](Self::identity) and
    /// [`identity_by_default`](Self::identity_by_default) are this with no
    /// options, which write no parentheses — `AS IDENTITY ()` is a syntax
    /// error.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(
    ///             ColumnDef::new(Glyph::Id)
    ///                 .big_integer()
    ///                 .identity_with(
    ///                     IdentityGeneration::Always,
    ///                     SequenceOption::StartWith(1000).and(SequenceOption::IncrementBy(10)),
    ///                 )
    ///         )
    ///         .primary_key(Glyph::Id)
    ///         .to_string(),
    ///     [
    ///         r#"CREATE TABLE "glyph" ( "id" bigint GENERATED ALWAYS AS IDENTITY"#,
    ///         r#"(INCREMENT BY 10 START WITH 1000), PRIMARY KEY ("id") )"#,
    ///     ]
    ///     .join(" "),
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.column-def+14]
    pub fn identity_with<O>(&mut self, generation: IdentityGeneration, options: O) -> &mut Self
    where
        O: Into<SequenceOptions>,
    {
        self.spec
            .push(ColumnSpec::Identity(generation, Some(options.into())));
        self
    }
}
