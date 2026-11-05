//! [`Collation`]: the name a `COLLATE` clause carries.

use super::*;

/// The name of a collation, optionally schema-qualified: what follows
/// `COLLATE` in an expression or a column definition.
///
/// A collation is looked up by name like a table, so its name is an
/// identifier and renders quoted — `"C"`, `"en-US-x-icu"`,
/// `"pg_catalog"."default"` — never as SQL. Quoting is what keeps the name the
/// caller wrote: PostgreSQL folds an unquoted identifier to lower case, so a
/// bare `C` would look for a collation `c` that does not exist, and `default`
/// is a reserved word that cannot stand bare at all.
///
/// A bare name converts through [`IntoCollation`], and a `(schema, name)` pair
/// qualifies it:
///
/// ```
/// use pgorm_query::*;
///
/// let bare = Expr::col(Name::runtime("title")).collate(Name::runtime("C"));
/// let qualified = Expr::col(Name::runtime("title"))
///     .collate((Name::runtime("pg_catalog"), Name::runtime("default")));
///
/// assert_eq!(
///     Query::select().expr(bare).expr(qualified).to_string(),
///     r#"SELECT ("title" COLLATE "C"), ("title" COLLATE "pg_catalog"."default")"#
/// );
/// ```
///
/// An expression is not a collation, so `a COLLATE b` for an arbitrary `b`
/// has no construction:
///
/// ```compile_fail,E0277
/// use pgorm_query::{*, tests_cfg::*};
///
/// Expr::col(Glyph::Image).collate(Expr::col(Glyph::Aspect));
/// ```
// [spec:pgorm:req:sql.ast.expr.collate+2]
#[derive(Debug, Clone, PartialEq)]
pub struct Collation {
    schema: Option<Name>,
    name: Name,
}

/// Anything that names a [`Collation`]: a name, a `(schema, name)` pair, or a
/// collation itself.
// [spec:pgorm:req:sql.ast.expr.collate+2]
pub trait IntoCollation {
    /// Consume `self` and produce the [`Collation`] it names.
    fn into_collation(self) -> Collation;
}

impl Collation {
    /// The collation's own name, without its schema.
    pub fn name(&self) -> &Name {
        &self.name
    }

    /// The schema qualifying the name, if it was written with one.
    pub fn schema(&self) -> Option<&Name> {
        self.schema.as_ref()
    }

    /// Write the name as a `COLLATE` clause spells it: each part quoted like
    /// every other identifier, the schema first when there is one.
    // [spec:pgorm:req:sql.render.collate]
    pub(crate) fn prepare(&self, s: &mut dyn fmt::Write) {
        if let Some(schema) = &self.schema {
            schema.prepare(s);
            write!(s, ".").unwrap();
        }
        self.name.prepare(s);
    }
}

impl IntoCollation for Collation {
    fn into_collation(self) -> Collation {
        self
    }
}

impl<T: 'static> IntoCollation for T
where
    T: IntoName,
{
    fn into_collation(self) -> Collation {
        Collation {
            schema: None,
            name: self.into_name(),
        }
    }
}

impl<S: 'static, T: 'static> IntoCollation for (S, T)
where
    S: IntoName,
    T: IntoName,
{
    fn into_collation(self) -> Collation {
        Collation {
            schema: Some(self.0.into_name()),
            name: self.1.into_name(),
        }
    }
}
