//! `CREATE DOMAIN`, `ALTER DOMAIN` and `DROP DOMAIN`.

use crate::{
    Collation, ColumnType, DropBehavior, IntoCollation, QueryBuilder, SimpleExpr,
    extension::{IntoTypeRef, TypeRef},
};

mod alter;
mod check;

pub use alter::{DomainAlterStatement, PendingDomainAlter};
pub use check::{DomainCheck, IntoDomainCheck};

pub(crate) use alter::DomainAlterAction;

/// Helper for constructing any statement over a domain: a named type over a
/// base type, with a default, a `NOT NULL` and named `CHECK`s of its own.
///
/// A domain is a type, so it is named as one is: a bare name or a
/// `(schema, name)` pair, each part a quoted identifier
/// ([`IntoTypeRef`]). A column of one is a
/// [`ColumnType::Domain`](crate::ColumnType::Domain).
// [spec:pgorm:req:sql.ddl.domain+4]
#[derive(Debug, Clone, Copy)]
pub struct Domain;

impl Domain {
    /// Construct a [`DomainCreateStatement`] over the domain it creates and
    /// the type it is over
    pub fn create<T>(name: T, base: ColumnType) -> DomainCreateStatement
    where
        T: IntoTypeRef,
    {
        DomainCreateStatement {
            name: name.into_type_ref(),
            base,
            collation: None,
            default: None,
            not_null: None,
            checks: Vec::new(),
        }
    }

    /// Name the domain a [`DomainAlterStatement`] will alter.
    ///
    /// Choosing the action on the returned [`PendingDomainAlter`] is what
    /// produces the statement: `ALTER DOMAIN "d"` with nothing after it is a
    /// syntax error, and PostgreSQL takes exactly one action per statement.
    pub fn alter<T>(name: T) -> PendingDomainAlter
    where
        T: IntoTypeRef,
    {
        PendingDomainAlter::new(name.into_type_ref())
    }

    /// Construct a [`DomainDropStatement`] over the first domain it drops
    pub fn drop<T>(name: T) -> DomainDropStatement
    where
        T: IntoTypeRef,
    {
        DomainDropStatement {
            first: name.into_type_ref(),
            rest: Vec::new(),
            if_exists: false,
            behavior: None,
        }
    }
}

/// Create a domain: `CREATE DOMAIN "name" AS <base>[ COLLATE "c"][ DEFAULT
/// <expr>][ NOT NULL | NULL][ [CONSTRAINT "n" ]CHECK (<expr>)...]`.
///
/// The name and the base type are taken by [`Domain::create`], neither being
/// optional in the grammar. The base is any [`ColumnType`], another domain
/// included; a `serial` spelling is not a type and has no `ColumnType`.
///
/// A default and a nullability are one slot each, the last call winning:
/// PostgreSQL refuses two `DEFAULT`s (`42601`, *multiple default
/// expressions*) and a `NOT NULL` beside a `NULL` (`42601`, *conflicting
/// NULL/NOT NULL constraints*). A `CHECK` added here is checked as the domain
/// is created, so it takes no `NOT VALID`, which the grammar has no place for
/// at `CREATE DOMAIN` (`42601`); [`DomainCheck::not_valid`] is what
/// [`PendingDomainAlter::add_check`] takes, and this does not:
///
/// ```compile_fail,E0277
/// use pgorm_query::*;
///
/// Domain::create(Name::runtime("d"), ColumnType::Integer)
///     .check(DomainCheck::new(Expr::domain_value().gt(0)).not_valid());
/// ```
///
/// What the server refuses, no builder value can see: a name already taken by
/// a type (`42710`), two constraints of one name (`42710`), a `COLLATE` on a
/// base type that has no collation (`42804`), a default its base type cannot
/// hold (`22P02`, `42804`), and a subquery in a default or a `CHECK`
/// (`0A000`).
///
/// ```
/// use pgorm_query::*;
///
/// assert_eq!(
///     Domain::create(
///         (Name::runtime("billing"), Name::runtime("quantity")),
///         ColumnType::Integer,
///     )
///     .default(1)
///     .not_null()
///     .check(DomainCheck::new(Expr::domain_value().gt(0)).name(Name::runtime("positive")))
///     .to_string(),
///     [
///         r#"CREATE DOMAIN "billing"."quantity" AS integer DEFAULT 1 NOT NULL"#,
///         r#"CONSTRAINT "positive" CHECK (VALUE > 0)"#,
///     ]
///     .join(" ")
/// );
/// ```
// [spec:pgorm:req:sql.ddl.domain+4]
#[derive(Debug, Clone)]
pub struct DomainCreateStatement {
    pub(crate) name: TypeRef,
    pub(crate) base: ColumnType,
    pub(crate) collation: Option<Collation>,
    pub(crate) default: Option<SimpleExpr>,
    /// `Some(true)` for `NOT NULL`, `Some(false)` for an explicit `NULL`.
    pub(crate) not_null: Option<bool>,
    pub(crate) checks: Vec<DomainCheck>,
}

impl DomainCreateStatement {
    /// `COLLATE "c"`: the collation the domain's values compare under, for a
    /// base type that has one.
    pub fn collate<C>(&mut self, collation: C) -> &mut Self
    where
        C: IntoCollation,
    {
        self.collation = Some(collation.into_collation());
        self
    }

    /// `DEFAULT <expr>`: what a column of the domain holds when an insert
    /// leaves it out and the column has no default of its own. Replaces a
    /// default already given.
    pub fn default<E>(&mut self, expr: E) -> &mut Self
    where
        E: Into<SimpleExpr>,
    {
        self.default = Some(expr.into());
        self
    }

    /// `NOT NULL`: refuse a null value (`23502`), replacing a [`null`](Self::null).
    pub fn not_null(&mut self) -> &mut Self {
        self.not_null = Some(true);
        self
    }

    /// `NULL`: allow a null value, which is also the default, replacing a
    /// [`not_null`](Self::not_null).
    pub fn null(&mut self) -> &mut Self {
        self.not_null = Some(false);
        self
    }

    /// Add a `CHECK`, an expression over [`Expr::domain_value`](crate::Expr::domain_value)
    /// that every value of the domain must pass (`23514` otherwise). Each call
    /// adds one, in order.
    pub fn check<C>(&mut self, check: C) -> &mut Self
    where
        C: IntoDomainCheck,
    {
        self.checks.push(check.into_domain_check());
        self
    }

    /// The domain created
    pub fn get_name(&self) -> &TypeRef {
        &self.name
    }

    /// The type the domain is over
    pub fn get_base(&self) -> &ColumnType {
        &self.base
    }

    /// The domain's `CHECK`s, in declaration order
    pub fn get_checks(&self) -> &[DomainCheck] {
        &self.checks
    }
}

/// Drop one or more domains: `DROP DOMAIN [IF EXISTS] "a"[, "b"...][ CASCADE
/// | RESTRICT]`.
///
/// The first name is taken by [`Domain::drop`] and every further one is
/// appended, so the list is non-empty: PostgreSQL rejects `DROP DOMAIN ` at
/// end of input. A domain a column or another domain is typed with is refused
/// under `RESTRICT`, which is also what an unsaid behavior means (`2BP01`);
/// `CASCADE` drops those columns and domains with it. A missing domain is
/// refused with `42704`, a notice under [`if_exists`](Self::if_exists).
///
/// ```
/// use pgorm_query::*;
///
/// assert_eq!(
///     Domain::drop(Name::runtime("quantity"))
///         .name((Name::runtime("billing"), Name::runtime("amount")))
///         .if_exists()
///         .cascade()
///         .to_string(),
///     r#"DROP DOMAIN IF EXISTS "quantity", "billing"."amount" CASCADE"#
/// );
/// ```
// [spec:pgorm:req:sql.ddl.domain+4]
#[derive(Debug, Clone)]
pub struct DomainDropStatement {
    pub(crate) first: TypeRef,
    pub(crate) rest: Vec<TypeRef>,
    pub(crate) if_exists: bool,
    pub(crate) behavior: Option<DropBehavior>,
}

impl DomainDropStatement {
    /// Drop a further domain
    pub fn name<T>(&mut self, name: T) -> &mut Self
    where
        T: IntoTypeRef,
    {
        self.rest.push(name.into_type_ref());
        self
    }

    /// `IF EXISTS`: a missing domain is a notice rather than an error.
    pub fn if_exists(&mut self) -> &mut Self {
        self.if_exists = true;
        self
    }

    /// `CASCADE`: drop the columns and domains typed with the domain too.
    /// The last of this and [`restrict`](Self::restrict) wins.
    pub fn cascade(&mut self) -> &mut Self {
        self.behavior = Some(DropBehavior::Cascade);
        self
    }

    /// `RESTRICT`: refuse to drop a domain anything is typed with (`2BP01`),
    /// which is also what an unsaid behavior does.
    pub fn restrict(&mut self) -> &mut Self {
        self.behavior = Some(DropBehavior::Restrict);
        self
    }

    /// The domains dropped, in declaration order, of which there is at least
    /// one
    pub fn names(&self) -> impl Iterator<Item = &TypeRef> {
        std::iter::once(&self.first).chain(&self.rest)
    }
}

macro_rules! impl_domain_display {
    ( $( $struct_name:ident => $func_name:ident ),* $(,)? ) => {
        $(
            /// Renders the statement with every value inlined as an escaped
            /// SQL literal — a default, a value a `CHECK` compares with. This
            /// is its only rendering: PostgreSQL takes no bind parameter in
            /// DDL, so there is no `build`.
            // [spec:pgorm:req:sql.ddl+12]
            impl std::fmt::Display for $struct_name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    let mut sql = String::with_capacity(128);
                    QueryBuilder.$func_name(self, &mut sql);
                    f.write_str(&sql)
                }
            }
        )*
    };
}

impl_domain_display!(
    DomainCreateStatement => prepare_domain_create_statement,
    DomainAlterStatement => prepare_domain_alter_statement,
    DomainDropStatement => prepare_domain_drop_statement,
);
