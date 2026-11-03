//! Branded scalar expressions, the operators that combine them, and the
//! conversions that let a column, an alias token or a Rust literal stand in
//! for one.

use std::marker::PhantomData;

use pgorm_query::{AliasName, IntoName, Name, SqlName, TypeName};

use crate::ColumnTrait;
use crate::entity::compared_enum;

use super::adapter::{self, BinOp, PlExpr, UnOp};
use super::funcs::CastType;
use super::sources::typed_cast;

/// A scalar expression, branded with the lifetime of the pipeline stage that
/// is allowed to consume it.
///
/// Expressions built purely from names and literals — [`col`], an entity
/// column, an [`alias`](pgorm_query::alias) token, a Rust literal — are
/// brand-polymorphic and can appear in any pipeline. An expression that
/// contains a bound placeholder — anything touched by
/// [`Binder::bind`](super::Binder::bind) — is pinned to the brand of the
/// binder that minted it, so carrying it into another pipeline (whose
/// binding stages quantify over a fresh brand) does not typecheck.
// [spec:pgorm:req:pipeline.params+4]
#[derive(Debug, Clone)]
pub struct Expr<'brand> {
    pub(super) node: PlExpr,
    /// The enum an entity column whose domain is over one compares as. Only
    /// the column itself carries it: what an operator builds from the column
    /// is a new expression of its own type.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+14]
    pub(super) compares_as: Option<TypeName>,
    pub(super) _brand: PhantomData<fn(&'brand ()) -> &'brand ()>,
}

pub(super) fn branded<'brand>(node: PlExpr) -> Expr<'brand> {
    Expr {
        node,
        compares_as: None,
        _brand: PhantomData,
    }
}

impl Expr<'_> {
    /// The node an operand of a comparison or an extremum is written as: cast
    /// to the enum when it is a column whose domain is over one, PostgreSQL
    /// resolving no operator or aggregate for the domain itself.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+14]
    pub(super) fn compared(self) -> PlExpr {
        match &self.compares_as {
            Some(enum_type) => typed_cast(self.node, enum_type),
            None => self.node,
        }
    }
}

/// A bare, unqualified identifier — the pipeline's own introduced names.
pub(super) fn name<'brand>(name: &str) -> Expr<'brand> {
    branded(adapter::ident(name))
}

/// A table-qualified column reference, for when the column alone will not do.
///
/// An entity column already knows its table, so `O::Total` is the everyday
/// spelling and this is the disambiguating one: a table spelled some other
/// way — an [`alias`](pgorm_query::alias) token, an
/// [`Name`](pgorm_query::Name), a table no entity describes.
///
/// Qualification is not optional: prqlc has no catalog, so a bare column
/// name becomes ambiguous the moment a join enters the pipeline. Minting the
/// reference from a `(table, column)` [`SqlName`] pair makes the qualified form
/// the only representable one.
// [spec:pgorm:sem:pipeline.qualify+3]
pub fn col<'brand>(table: impl IntoName, column: impl IntoName) -> Expr<'brand> {
    branded(adapter::ident_in(
        vec![SqlName::to_string(&*table.into_name())],
        SqlName::to_string(&*column.into_name()),
    ))
}

/// A column of the relation being joined — PRQL's `that` — for the join
/// condition whose column name exists on both sides.
///
/// An embedded pipeline has no name the caller can write
/// (`[spec:pgorm:req:pipeline.compose]`), so when its column shares a name
/// with one of the consumer's, `that(column)` qualifies it by role instead:
/// `that(ID)` is the joined relation's `id`. The qualification is scoped to
/// the join condition; stages after the join refer to the column by its own
/// name, renamed in the embedded pipeline's projection if it collides.
// [spec:pgorm:req:pipeline.compose]
pub fn that<'brand>(column: impl IntoName) -> Expr<'brand> {
    branded(adapter::ident_in(
        vec!["that".to_owned()],
        SqlName::to_string(&*column.into_name()),
    ))
}

/// A column of the pipeline built so far — PRQL's `this` — the left-hand
/// counterpart of [`that`], for the join condition whose consumer is itself
/// an embedded pipeline with no name to qualify by. Scoped to the join
/// condition, like [`that`].
// [spec:pgorm:req:pipeline.compose]
pub fn this<'brand>(column: impl IntoName) -> Expr<'brand> {
    branded(adapter::ident_in(
        vec!["this".to_owned()],
        SqlName::to_string(&*column.into_name()),
    ))
}

/// An entity column read under `qualifier`, carrying the enum it compares as
/// when its domain is over one: what the column says of its type holds under
/// any name its relation is read by.
// [spec:pgorm:sem:pipeline.qualify+3]
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
pub(super) fn entity_column<'brand, C: ColumnTrait>(qualifier: String, column: &C) -> Expr<'brand> {
    let mut expr = branded(adapter::ident_in(
        vec![qualifier],
        SqlName::to_string(column),
    ));
    expr.compares_as = compared_enum(column.def().get_column_type());
    expr
}

/// An entity column is a table-qualified expression: the column enum carries
/// its entity, so the qualification is recovered rather than restated.
// [spec:pgorm:sem:pipeline.qualify+3]
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
impl<'brand, C: ColumnTrait> From<C> for Expr<'brand> {
    fn from(column: C) -> Self {
        entity_column(SqlName::to_string(&*column.entity_name()), &column)
    }
}

/// An alias token reads back the name it declared, unqualified — the name
/// belongs to the pipeline, not to a table.
// [spec:pgorm:req:pipeline.surface+3]
impl<'brand> From<AliasName> for Expr<'brand> {
    fn from(token: AliasName) -> Self {
        name(token.as_str())
    }
}

/// A runtime alias reads an introduced name under the same resolution rules.
// [spec:pgorm:req:bindings.pipeline-expressions+2]
impl<'brand> From<Name> for Expr<'brand> {
    fn from(alias: Name) -> Self {
        name(&SqlName::to_string(&*alias))
    }
}

macro_rules! literal {
    ($ty:ty, $make:ident, $conv:expr) => {
        /// A Rust literal is an inline SQL literal, exactly as a literal
        /// written in PRQL text would be. Runtime values belong in
        /// [`Binder::bind`](super::Binder::bind).
        // [spec:pgorm:req:pipeline.params+4]
        impl<'brand> From<$ty> for Expr<'brand> {
            fn from(value: $ty) -> Self {
                let conv = $conv;
                branded(adapter::$make(conv(value)))
            }
        }
    };
}

literal!(i32, lit_int, i64::from);
literal!(i64, lit_int, std::convert::identity);
literal!(f64, lit_float, std::convert::identity);
literal!(bool, lit_bool, std::convert::identity);
literal!(&str, lit_str, std::convert::identity);

/// A binary operator's application, each side that is a column whose domain
/// is over an enum cast to the enum ([`Expr::compared`]): the comparisons and
/// `coalesce` need it, and no other operator applies to an enum at all.
// [spec:pgorm:sem:entity.traits.column.enum-cast+14]
fn bin<'brand>(lhs: Expr<'brand>, op: BinOp, rhs: Expr<'brand>) -> Expr<'brand> {
    branded(adapter::binary(lhs.compared(), op, rhs.compared()))
}

/// Everything you can do to an expression.
///
/// The operators live on a trait rather than on [`Expr`] so that the things
/// which *are* expressions — an entity column, an
/// [`alias`](pgorm_query::alias) token, an `Expr` itself — all carry them,
/// and a comparison can be written against a Rust literal or another column
/// with no ceremony:
///
/// ```
/// # use pgorm::pipeline::ExprOps;
/// # use pgorm::pgorm_query::alias;
/// # use pgorm::tests_cfg::cake;
/// let big = alias("big");
/// let _ = cake::Column::Id.gt(10).as_(big);
/// ```
///
/// Every right-hand side takes `impl Into<Expr>`, so columns, tokens,
/// literals, bound placeholders and expressions are interchangeable there.
///
/// An entity column whose domain is over an enum is compared as the enum:
/// each operator casts it to the enum, either side, PostgreSQL resolving no
/// operator for the domain itself. Only the column carries the type, so a
/// [`col`] reference or a token naming such a column's projection compares
/// as the domain unless [`compares_as`](ExprOps::compares_as) names the enum.
///
/// The comparison names (`eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `is_null`,
/// `is_not_null`) are also [`ColumnTrait`] method names. Both can be in
/// scope at once: because these take `self` by value and `ColumnTrait`'s
/// take `&self`, a column resolves to *this* trait when both are imported,
/// and the ORM spelling is then `ColumnTrait::gt(&col, v)`. A module usually
/// speaks one of the two dialects, so importing the pipeline where it is
/// used keeps them apart.
// [spec:pgorm:req:pipeline.surface+3]
// Every operator here consumes its receiver: these build an expression, they
// never inspect one, so `is_null` takes `self` like the rest of the trait.
#[allow(clippy::wrong_self_convention)]
pub trait ExprOps<'brand>: Into<Expr<'brand>> + Sized {
    /// `=` (or `IS NULL` when compared against [`null`](super::null)).
    fn eq(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Eq, rhs.into())
    }

    /// `<>` (or `IS NOT NULL` against [`null`](super::null)).
    fn ne(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Ne, rhs.into())
    }

    /// `>`
    fn gt(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Gt, rhs.into())
    }

    /// `>=`
    fn gte(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Gte, rhs.into())
    }

    /// `<`
    fn lt(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Lt, rhs.into())
    }

    /// `<=`
    fn lte(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Lte, rhs.into())
    }

    /// Logical `AND`.
    fn and(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::And, rhs.into())
    }

    /// Logical `OR`.
    fn or(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Or, rhs.into())
    }

    /// `COALESCE(self, rhs)`. A column whose domain is over an enum is cast
    /// to the enum, as [`ColumnTrait::if_null`] casts it, so what the
    /// coalesce yields can be compared.
    fn coalesce(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Coalesce, rhs.into())
    }

    /// `+`
    fn add(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Add, rhs.into())
    }

    /// `-`
    fn sub(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Sub, rhs.into())
    }

    /// `*`
    fn mul(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Mul, rhs.into())
    }

    /// `/`
    fn div(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::DivFloat, rhs.into())
    }

    /// `%`
    fn rem(self, rhs: impl Into<Expr<'brand>>) -> Expr<'brand> {
        bin(self.into(), BinOp::Mod, rhs.into())
    }

    /// `IS NULL`.
    fn is_null(self) -> Expr<'brand> {
        bin(self.into(), BinOp::Eq, branded(adapter::lit_null()))
    }

    /// `IS NOT NULL`.
    fn is_not_null(self) -> Expr<'brand> {
        bin(self.into(), BinOp::Ne, branded(adapter::lit_null()))
    }

    /// Membership in an explicit list: `self IN (items...)`.
    ///
    /// Members are expressions, so they can be literals or bound
    /// placeholders. A column whose domain is over an enum, the operand or a
    /// member, is compared as the enum, as by the comparisons.
    fn in_array<I>(self, items: I) -> Expr<'brand>
    where
        I: IntoIterator,
        I::Item: Into<Expr<'brand>>,
    {
        let items = items
            .into_iter()
            .map(|item| item.into().compared())
            .collect::<Vec<_>>();
        branded(adapter::call(
            "in",
            vec![adapter::array(items), self.into().compared()],
        ))
    }

    /// Compare this expression as the enum `enum_type`, as an entity column
    /// whose domain is over that enum compares: every comparison, `in_array`,
    /// `coalesce`, `min` and `max` cast it to the enum, and sorting, grouping
    /// and projecting keep it as it is.
    ///
    /// The pipeline learns a column's type from its entity alone. A column it
    /// reads without one — [`col`] over a table no entity describes, an
    /// [`alias`](pgorm_query::alias) token naming a projection — is compared
    /// as the type the server gives it, and PostgreSQL resolves no operator
    /// for a domain over an enum, not even against its own values (`42883`).
    /// This names the enum such a column compares as, schema-qualified where
    /// the enum is ([`TypeName::schema`]). A relation read under a name keeps
    /// its entity's types through [`Named::col`](super::Named::col), which
    /// needs no naming.
    ///
    /// ```
    /// use pgorm::pipeline::{ExprOps, Pipeline, alias, col};
    ///
    /// let (sql, _) = Pipeline::from(alias("line"))
    ///     .filter(col(alias("line"), alias("mood")).compares_as(alias("mood")).eq("glad"))
    ///     .into_sql()?;
    /// assert_eq!(sql, "SELECT * FROM line WHERE CAST(mood AS mood) = 'glad'");
    /// # Ok::<_, pgorm::pipeline::PipelineError>(())
    /// ```
    // [spec:pgorm:sem:entity.traits.column.enum-cast+14]
    fn compares_as(self, enum_type: impl Into<TypeName>) -> Expr<'brand> {
        let mut expr = self.into();
        expr.compares_as = Some(enum_type.into());
        expr
    }

    /// `CAST(self AS type)`, over the closed [`CastType`] set.
    fn cast(self, ty: CastType) -> Expr<'brand> {
        branded(adapter::call(
            "as",
            vec![adapter::ident(ty.name()), self.into().node],
        ))
    }

    /// Name this expression in a `derive`, `select`, `aggregate` or window
    /// projection.
    ///
    /// The name is an [`alias`](pgorm_query::alias) token when anything
    /// refers back to it, and a bare `&'static str` when nothing does. It
    /// must not collide with a PRQL built-in;
    /// [`into_sql`](super::Pipeline::into_sql) refuses the closed reserved
    /// set with
    /// [`PipelineError::ReservedAlias`](super::PipelineError::ReservedAlias).
    fn as_(self, name: impl Into<AliasName>) -> Expr<'brand> {
        let name: AliasName = name.into();
        branded(adapter::aliased(self.into().node, name.as_str().to_owned()))
    }

    /// Name a projection with an owned identifier computed at runtime.
    /// Reserved-name validation is identical to [`as_`](ExprOps::as_).
    // [spec:pgorm:req:bindings.pipeline-expressions+2]
    fn as_runtime(self, name: Name) -> Expr<'brand> {
        branded(adapter::aliased(
            self.into().node,
            SqlName::to_string(&*name),
        ))
    }

    /// Mark a sort key descending.
    fn desc(self) -> Expr<'brand> {
        branded(adapter::unary(UnOp::Neg, self.into().node))
    }

    /// Mark a sort key ascending (the default; provided for symmetry).
    fn asc(self) -> Expr<'brand> {
        branded(adapter::unary(UnOp::Add, self.into().node))
    }
}

// [spec:pgorm:req:pipeline.surface+3]
impl<'brand> ExprOps<'brand> for Expr<'brand> {}

// [spec:pgorm:req:pipeline.surface+3]
impl<'brand> ExprOps<'brand> for AliasName {}

impl<'brand> ExprOps<'brand> for Name {}

// [spec:pgorm:req:pipeline.surface+3]
impl<'brand, C: ColumnTrait> ExprOps<'brand> for C {}

/// A list of expressions, as the transforms that project, group and sort
/// take one.
///
/// A single expression needs no wrapper, a homogeneous list is an array or a
/// `Vec`, and a mixed list — the usual case, a column beside an aggregate
/// beside a token — is a tuple, which is how PRQL spells a projection too.
///
/// ```
/// # use pgorm::pipeline::{ExprOps, Pipeline, sum};
/// # use pgorm::pgorm_query::alias;
/// # use pgorm::tests_cfg::{cake, cake::Column as C};
/// let total = alias("total");
/// let _ = Pipeline::from(cake::Entity)
///     .select(C::Id)                                  // one
///     .select([C::Id, C::Name])                       // homogeneous
///     .select((C::Id, sum(C::Id).as_(total), total)); // mixed
/// ```
// [spec:pgorm:req:pipeline.surface+3]
pub trait ExprList<'brand> {
    /// The expressions, in the order written.
    fn into_exprs(self) -> Vec<Expr<'brand>>;
}

impl<'brand> ExprList<'brand> for Expr<'brand> {
    fn into_exprs(self) -> Vec<Expr<'brand>> {
        vec![self]
    }
}

impl<'brand> ExprList<'brand> for AliasName {
    fn into_exprs(self) -> Vec<Expr<'brand>> {
        vec![self.into()]
    }
}

impl<'brand> ExprList<'brand> for Name {
    fn into_exprs(self) -> Vec<Expr<'brand>> {
        vec![self.into()]
    }
}

impl<'brand, C: ColumnTrait> ExprList<'brand> for C {
    fn into_exprs(self) -> Vec<Expr<'brand>> {
        vec![self.into()]
    }
}

impl<'brand, T, const N: usize> ExprList<'brand> for [T; N]
where
    T: Into<Expr<'brand>>,
{
    fn into_exprs(self) -> Vec<Expr<'brand>> {
        self.into_iter().map(Into::into).collect()
    }
}

impl<'brand, T> ExprList<'brand> for Vec<T>
where
    T: Into<Expr<'brand>>,
{
    fn into_exprs(self) -> Vec<Expr<'brand>> {
        self.into_iter().map(Into::into).collect()
    }
}

macro_rules! expr_list_tuple {
    ($($name:ident),+) => {
        impl<'brand, $($name),+> ExprList<'brand> for ($($name,)+)
        where
            $($name: Into<Expr<'brand>>),+
        {
            #[allow(non_snake_case)]
            fn into_exprs(self) -> Vec<Expr<'brand>> {
                let ($($name,)+) = self;
                vec![$($name.into()),+]
            }
        }
    };
}

expr_list_tuple!(A, B);
expr_list_tuple!(A, B, C);
expr_list_tuple!(A, B, C, D);
expr_list_tuple!(A, B, C, D, E);
expr_list_tuple!(A, B, C, D, E, F);
expr_list_tuple!(A, B, C, D, E, F, G);
expr_list_tuple!(A, B, C, D, E, F, G, H);
expr_list_tuple!(A, B, C, D, E, F, G, H, I);
expr_list_tuple!(A, B, C, D, E, F, G, H, I, J);
expr_list_tuple!(A, B, C, D, E, F, G, H, I, J, K);
expr_list_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);

pub(super) fn nodes_of<'brand>(list: impl ExprList<'brand>) -> Vec<PlExpr> {
    list.into_exprs()
        .into_iter()
        .map(|expr| expr.node)
        .collect()
}

impl<'brand, R: Into<Expr<'brand>>> std::ops::Add<R> for Expr<'brand> {
    type Output = Expr<'brand>;
    fn add(self, rhs: R) -> Expr<'brand> {
        bin(self, BinOp::Add, rhs.into())
    }
}

impl<'brand, R: Into<Expr<'brand>>> std::ops::Sub<R> for Expr<'brand> {
    type Output = Expr<'brand>;
    fn sub(self, rhs: R) -> Expr<'brand> {
        bin(self, BinOp::Sub, rhs.into())
    }
}

impl<'brand, R: Into<Expr<'brand>>> std::ops::Mul<R> for Expr<'brand> {
    type Output = Expr<'brand>;
    fn mul(self, rhs: R) -> Expr<'brand> {
        bin(self, BinOp::Mul, rhs.into())
    }
}

impl<'brand, R: Into<Expr<'brand>>> std::ops::Div<R> for Expr<'brand> {
    type Output = Expr<'brand>;
    fn div(self, rhs: R) -> Expr<'brand> {
        bin(self, BinOp::DivFloat, rhs.into())
    }
}

impl<'brand, R: Into<Expr<'brand>>> std::ops::Rem<R> for Expr<'brand> {
    type Output = Expr<'brand>;
    fn rem(self, rhs: R) -> Expr<'brand> {
        bin(self, BinOp::Mod, rhs.into())
    }
}

impl<'brand> std::ops::Neg for Expr<'brand> {
    type Output = Expr<'brand>;
    fn neg(self) -> Expr<'brand> {
        branded(adapter::unary(UnOp::Neg, self.node))
    }
}

impl<'brand> std::ops::Not for Expr<'brand> {
    type Output = Expr<'brand>;
    fn not(self) -> Expr<'brand> {
        branded(adapter::unary(UnOp::Not, self.node))
    }
}
