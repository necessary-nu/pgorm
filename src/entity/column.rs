use crate::{EntityName, IntoSimpleExpr, Iterable, SqlName, StaticName};
use pgorm_query::{
    ArrayElement, BinOper, Expr, ExprTrait, Func, LikeExpr, Name, SelectStatement, SimpleExpr,
    Value,
};
use std::str::FromStr;

// The original `pgorm::ColumnType` enum was dropped since 0.11.0
// It was replaced by `pgorm_query::ColumnType`, we reexport it here to keep the `ColumnType` symbol
pub use pgorm_query::ColumnType;
#[path = "column_def.rs"]
pub(crate) mod column_def;
pub use column_def::*;

/// Defines a Column for an Entity
// [spec:pgorm:req:entity.traits.column-def+2]
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub(crate) col_type: ColumnType,
    pub(crate) null: bool,
    pub(crate) unique: bool,
    pub(crate) indexed: bool,
    pub(crate) default: Option<ColumnDefault>,
    pub(crate) comment: Option<String>,
}

macro_rules! bind_oper {
    ( $op: ident, $bin_op: ident ) => {
        #[allow(missing_docs)]
        fn $op<V>(&self, v: V) -> SimpleExpr
        where
            V: Into<Value>,
        {
            let expr = compared_value(self, Expr::val(v));
            compared_column(self, Expr::col((self.entity_name(), *self)))
                .binary(BinOper::$bin_op, expr)
        }
    };
}

macro_rules! bind_value_oper {
    ( $op: ident, $bin_op: ident, $doc: expr ) => {
        #[doc = $doc]
        ///
        /// The operand is a value of the column's own type — a range, a
        /// multirange or an array, as the column holds — passed through
        /// [`save_as`](ColumnTrait::save_as) as a comparison's is, so a
        /// range a schema created is written as its text cast to the type.
        /// An element, which `@>` and `<@` also take, is not the column's
        /// type and is an expression's to compare.
        // [spec:pgorm:def:entity.traits.column+9]
        fn $op<V>(&self, v: V) -> SimpleExpr
        where
            V: Into<Value>,
        {
            let expr = compared_value(self, Expr::val(v));
            compared_column(self, Expr::col((self.entity_name(), *self)))
                .binary(BinOper::$bin_op, expr)
        }
    };
}

macro_rules! bind_oper_col {
    ( $op: ident, $bin_op: ident, $doc: expr ) => {
        #[doc = $doc]
        ///
        /// Both operands name their own entity, so the comparison needs neither
        /// the `Expr::col` escape nor a respelling of either table. The operand
        /// is a column rather than a value, so it does not pass through
        /// `save_as`: an enum column compared against another column of the same
        /// enum type needs no cast, and a column whose domain is over an enum is
        /// cast to the enum, either side. See [`ColumnTrait::eq_col`] for an
        /// example.
        fn $op<C>(&self, other: C) -> SimpleExpr
        where
            C: ColumnTrait,
        {
            let other_column = compared_column(&other, Expr::col(other.as_column_ref()));
            compared_column(self, Expr::col((self.entity_name(), *self)))
                .binary(BinOper::$bin_op, other_column)
        }
    };
}

macro_rules! bind_func_no_params {
    ( $func: ident ) => {
        /// See also SeaQuery's method with same name.
        fn $func(&self) -> SimpleExpr {
            Expr::col((self.entity_name(), *self)).$func()
        }
    };
}

// [spec:pgorm:sem:entity.traits.column.enum-cast+17]
macro_rules! bind_compared_aggregate {
    ( $func: ident ) => {
        /// See also SeaQuery's method with same name. A column whose domain is
        /// over an enum is aggregated as the enum, PostgreSQL resolving the
        /// aggregate for the enum and not for the domain.
        fn $func(&self) -> SimpleExpr {
            compared_column(self, Expr::col((self.entity_name(), *self))).$func()
        }
    };
}

macro_rules! bind_vec_func {
    ( $func: ident ) => {
        #[allow(missing_docs)]
        #[allow(clippy::wrong_self_convention)]
        fn $func<V, I>(&self, v: I) -> SimpleExpr
        where
            V: Into<Value>,
            I: IntoIterator<Item = V>,
        {
            let v_with_enum_cast = v.into_iter().map(|v| compared_value(self, Expr::val(v)));
            compared_column(self, Expr::col((self.entity_name(), *self))).$func(v_with_enum_cast)
        }
    };
}

// [spec:pgorm:sem:entity.traits.column.enum-cast+17]
macro_rules! bind_subquery_func {
    ( $func: ident, $array_cmp: ident, $quantifier: ident ) => {
        /// Membership in what a subquery projects. A column whose domain is
        /// over an enum is compared as the enum, and so is the subquery's one
        /// projected expression — cast in place, or, where that cannot reach
        /// it (`*`, a window function's), through the subquery's `ARRAY(..)`
        /// cast to the enum's array.
        #[allow(clippy::wrong_self_convention)]
        fn $func(&self, s: SelectStatement) -> SimpleExpr {
            let column = compared_column(self, Expr::col((self.entity_name(), *self)));
            match compared_subquery(self, s) {
                ComparedSubquery::Rows(s) => column.$func(*s),
                ComparedSubquery::Array(array) => column.$array_cmp(Func::$quantifier(array)),
            }
        }
    };
}

use column_def::{created_range_type_name, enum_type_name, holds_array, under_domains};

// LINT: when the operand value does not match column type
/// API for working with a `Column`. Mostly a wrapper of the identically named methods in [`pgorm_query::Expr`]
// [spec:pgorm:def:entity.traits.column+9]
pub trait ColumnTrait: StaticName + Iterable + FromStr {
    #[allow(missing_docs)]
    type EntityName: EntityName;

    /// Define a column for an Entity
    fn def(&self) -> ColumnDef;

    /// Get the name of the entity the column belongs to
    fn entity_name(&self) -> Name {
        Name::new(Self::EntityName::default())
    }

    /// get the name of the entity the column belongs to
    fn as_column_ref(&self) -> (Name, Name) {
        (self.entity_name(), Name::new(*self))
    }

    /// The key this column occupies in a JSON object, as `serde` names it.
    ///
    /// The `with-json` conversions on `ActiveModelTrait` deserialize through the
    /// entity's `Model`, so the keys they read are `serde`'s — the model field's
    /// name, spelled the way any `serde` rename spells it — and not the SQL
    /// column name. The `Entity` derive computes this from the field the column
    /// was derived from; a hand-written `Column` has no field to read, so it
    /// falls back to the SQL name, which is what the two agree on when nothing
    /// is renamed.
    fn json_key(&self) -> &str {
        self.as_str()
    }

    bind_oper!(eq, Equal);
    bind_oper!(ne, NotEqual);
    bind_oper!(gt, GreaterThan);
    bind_oper!(gte, GreaterThanOrEqual);
    bind_oper!(lt, SmallerThan);
    bind_oper!(lte, SmallerThanOrEqual);

    /// Express an equality between this column and another column.
    ///
    /// The value-taking [`eq`][ColumnTrait::eq] cannot express this: its operand
    /// is bound by `Into<Value>` and is run through
    /// [`save_as`][ColumnTrait::save_as], which is what casts an enum operand.
    /// The column-taking siblings are separate methods rather than a widening of
    /// `eq` so that cast is never silently dropped.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::{cake, fruit}};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Id.eq_col(fruit::Column::CakeId))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = "fruit"."cake_id""#
    /// );
    /// ```
    ///
    /// Both operands name their own entity, so the comparison needs neither the
    /// `Expr::col` escape nor a respelling of either table. The operand is a
    /// column rather than a value, so it does not pass through `save_as`: an
    /// enum column compared against another column of the same enum type needs
    /// no cast, and a column whose domain is over an enum is cast to the enum,
    /// either side, PostgreSQL resolving no operator for the domain.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+17]
    fn eq_col<C>(&self, other: C) -> SimpleExpr
    where
        C: ColumnTrait,
    {
        let other_column = compared_column(&other, Expr::col(other.as_column_ref()));
        compared_column(self, Expr::col((self.entity_name(), *self)))
            .binary(BinOper::Equal, other_column)
    }

    bind_oper_col!(
        ne_col,
        NotEqual,
        "Express an inequality between this column and another column."
    );
    bind_oper_col!(
        gt_col,
        GreaterThan,
        "Express a greater-than between this column and another column."
    );
    bind_oper_col!(
        gte_col,
        GreaterThanOrEqual,
        "Express a greater-than-or-equal between this column and another column."
    );
    bind_oper_col!(
        lt_col,
        SmallerThan,
        "Express a less-than between this column and another column."
    );
    bind_oper_col!(
        lte_col,
        SmallerThanOrEqual,
        "Express a less-than-or-equal between this column and another column."
    );

    /// Express an equality between this column and an arbitrary expression.
    ///
    /// The general form of [`eq_col`][ColumnTrait::eq_col], for comparing
    /// against something computed rather than against a plain column. Like
    /// `eq_col`, the operand does not pass through
    /// [`save_as`][ColumnTrait::save_as] — an expression carries whatever cast
    /// it was built with. A column whose domain is over an enum is cast to the
    /// enum, so the expression compares with it as the enum.
    ///
    /// ```
    /// use pgorm::{entity::*, pgorm_query::{Expr, ExprTrait}, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Id.eq_expr(Expr::col((cake::Entity, cake::Column::Id)).add(1)))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = "cake"."id" + 1"#
    /// );
    /// ```
    fn eq_expr<T>(&self, other: T) -> SimpleExpr
    where
        T: Into<SimpleExpr>,
    {
        compared_column(self, Expr::col((self.entity_name(), *self)))
            .binary(BinOper::Equal, other.into())
    }

    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Id.between(2, 3))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" BETWEEN 2 AND 3"#
    /// );
    /// ```
    fn between<V>(&self, a: V, b: V) -> SimpleExpr
    where
        V: Into<Value>,
    {
        let a = compared_value(self, Expr::val(a));
        let b = compared_value(self, Expr::val(b));
        compared_column(self, Expr::col((self.entity_name(), *self))).between(a, b)
    }

    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Id.not_between(2, 3))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" NOT BETWEEN 2 AND 3"#
    /// );
    /// ```
    fn not_between<V>(&self, a: V, b: V) -> SimpleExpr
    where
        V: Into<Value>,
    {
        let a = compared_value(self, Expr::val(a));
        let b = compared_value(self, Expr::val(b));
        compared_column(self, Expr::col((self.entity_name(), *self))).not_between(a, b)
    }

    /// `BETWEEN SYMMETRIC`: [`between`][ColumnTrait::between] with its
    /// bounds sorted first, so they hold in whichever order they are given.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Id.between_symmetric(3, 2))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" BETWEEN SYMMETRIC 3 AND 2"#
    /// );
    /// ```
    // [spec:pgorm:def:entity.traits.column+9]
    fn between_symmetric<V>(&self, a: V, b: V) -> SimpleExpr
    where
        V: Into<Value>,
    {
        let a = compared_value(self, Expr::val(a));
        let b = compared_value(self, Expr::val(b));
        compared_column(self, Expr::col((self.entity_name(), *self))).between_symmetric(a, b)
    }

    /// `NOT BETWEEN SYMMETRIC`, the complement of
    /// [`between_symmetric`][ColumnTrait::between_symmetric].
    // [spec:pgorm:def:entity.traits.column+9]
    fn not_between_symmetric<V>(&self, a: V, b: V) -> SimpleExpr
    where
        V: Into<Value>,
    {
        let a = compared_value(self, Expr::val(a));
        let b = compared_value(self, Expr::val(b));
        compared_column(self, Expr::col((self.entity_name(), *self))).not_between_symmetric(a, b)
    }

    /// `IS DISTINCT FROM`: inequality that reads NULL as a value, so it
    /// answers true or false for every pair, a null operand being a `None` of
    /// the column's type (`Option::<String>::None`).
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Name.is_distinct_from("cheese"))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" IS DISTINCT FROM 'cheese'"#
    /// );
    /// ```
    // [spec:pgorm:def:entity.traits.column+9]
    #[allow(clippy::wrong_self_convention)]
    fn is_distinct_from<V>(&self, v: V) -> SimpleExpr
    where
        V: Into<Value>,
    {
        let v = compared_value(self, Expr::val(v));
        compared_column(self, Expr::col((self.entity_name(), *self))).is_distinct_from(v)
    }

    /// `IS NOT DISTINCT FROM`, the null-safe equality complementing
    /// [`is_distinct_from`][ColumnTrait::is_distinct_from].
    // [spec:pgorm:def:entity.traits.column+9]
    #[allow(clippy::wrong_self_convention)]
    fn is_not_distinct_from<V>(&self, v: V) -> SimpleExpr
    where
        V: Into<Value>,
    {
        let v = compared_value(self, Expr::val(v));
        compared_column(self, Expr::col((self.entity_name(), *self))).is_not_distinct_from(v)
    }

    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Name.like("cheese"))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" LIKE 'cheese'"#
    /// );
    /// ```
    fn like<T>(&self, s: T) -> SimpleExpr
    where
        T: Into<String>,
    {
        Expr::col((self.entity_name(), *self)).like(s)
    }

    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Name.not_like("cheese"))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" NOT LIKE 'cheese'"#
    /// );
    /// ```
    fn not_like<T>(&self, s: T) -> SimpleExpr
    where
        T: Into<String>,
    {
        Expr::col((self.entity_name(), *self)).not_like(s)
    }

    /// The search text is literal: a `%`, `_` or `\` in it is escaped rather
    /// than read as a pattern character. A pattern is [`like`]'s job.
    ///
    /// [`like`]: ColumnTrait::like
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Name.starts_with("cheese"))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" LIKE 'cheese%'"#
    /// );
    /// ```
    // [spec:pgorm:def:entity.traits.column+9]
    fn starts_with<T>(&self, s: T) -> SimpleExpr
    where
        T: Into<String>,
    {
        Expr::col((self.entity_name(), *self)).like(LikeExpr::starting_with(s))
    }

    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Name.ends_with("cheese"))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" LIKE '%cheese'"#
    /// );
    /// ```
    // [spec:pgorm:def:entity.traits.column+9]
    fn ends_with<T>(&self, s: T) -> SimpleExpr
    where
        T: Into<String>,
    {
        Expr::col((self.entity_name(), *self)).like(LikeExpr::ending_with(s))
    }

    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Name.contains("cheese"))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" LIKE '%cheese%'"#
    /// );
    /// ```
    // [spec:pgorm:def:entity.traits.column+9]
    fn contains<T>(&self, s: T) -> SimpleExpr
    where
        T: Into<String>,
    {
        Expr::col((self.entity_name(), *self)).like(LikeExpr::containing(s))
    }

    bind_compared_aggregate!(max);
    bind_compared_aggregate!(min);
    bind_func_no_params!(sum);
    bind_func_no_params!(count);
    bind_func_no_params!(is_null);
    bind_func_no_params!(is_not_null);

    /// Perform an operation if the column is null
    fn if_null<V>(&self, v: V) -> SimpleExpr
    where
        V: Into<Value>,
    {
        let v = compared_value(self, Expr::val(v));
        compared_column(self, Expr::col((self.entity_name(), *self))).if_null(v)
    }

    bind_vec_func!(is_in);
    bind_vec_func!(is_not_in);

    /// Membership against one array parameter, the PostgreSQL-idiomatic
    /// counterpart of [`ColumnTrait::is_in`]. See [`pgorm_query::ExprTrait::eq_any`]
    /// for when each applies; the short of it is that `is_in` suits a short
    /// literal list and `eq_any` a list whose length varies at runtime, because
    /// `eq_any` renders the same SQL text — and so reuses the same cached
    /// statement — for every length.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Id.eq_any([2, 3]))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = ANY(ARRAY [2,3])"#
    /// );
    ///
    /// // String slices are elements as `eq` takes one, the list a `text[]`.
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Name.eq_any(["Apple Pie", "Lemon Tart"]))
    ///         .build()
    ///         .0,
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" = ANY($1)"#
    /// );
    /// ```
    fn eq_any<V, I>(&self, v: I) -> SimpleExpr
    where
        V: ArrayElement,
        I: IntoIterator<Item = V>,
    {
        self.compare_any(BinOper::Equal, Value::array(v))
    }

    /// Non-membership against one array parameter, the complement of
    /// [`ColumnTrait::eq_any`]. See [`pgorm_query::ExprTrait::ne_all`] for why the
    /// negation is spelled `<> ALL` rather than `NOT (… = ANY …)`.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Id.ne_all([2, 3]))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" <> ALL(ARRAY [2,3])"#
    /// );
    /// ```
    fn ne_all<V, I>(&self, v: I) -> SimpleExpr
    where
        V: ArrayElement,
        I: IntoIterator<Item = V>,
    {
        self.compare_all(BinOper::NotEqual, Value::array(v))
    }

    /// `self <op> ANY (array)`: `op` holds between the column and some
    /// element of one array parameter, and none over an empty one. The array
    /// is cast as [`eq_any`](ColumnTrait::eq_any)'s is, through
    /// [`save_array_as`](ColumnTrait::save_array_as), so `money` compares
    /// with a `Vec<Decimal>`. `op` is a comparison or a range operator, an
    /// operator whose other side is a value of the column's type; a pattern
    /// is no value of it, and casting one to the column's type would change
    /// what it matches.
    ///
    /// ```
    /// use pgorm::{entity::*, pgorm_query::BinOper, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Id.compare_any(BinOper::GreaterThan, vec![2, 3]))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" > ANY(ARRAY [2,3])"#
    /// );
    /// ```
    // [spec:pgorm:def:entity.traits.column+9]
    fn compare_any<A>(&self, op: BinOper, array: A) -> SimpleExpr
    where
        A: Into<Value>,
    {
        let array = compared_array(self, Expr::val(array));
        compared_column(self, Expr::col((self.entity_name(), *self))).binary(op, Func::any(array))
    }

    /// `self <op> ALL (array)`: [`compare_any`](ColumnTrait::compare_any)'s
    /// counterpart, holding for every element, and over an empty array.
    // [spec:pgorm:def:entity.traits.column+9]
    fn compare_all<A>(&self, op: BinOper, array: A) -> SimpleExpr
    where
        A: Into<Value>,
    {
        let array = compared_array(self, Expr::val(array));
        compared_column(self, Expr::col((self.entity_name(), *self))).binary(op, Func::all(array))
    }

    bind_value_oper!(
        includes,
        Contains,
        "`@>`: the column contains the value, a range or an array of its type. \
         Named apart from [`contains`](ColumnTrait::contains), the substring \
         sugar."
    );
    bind_value_oper!(
        contained_by,
        Contained,
        "`<@`: the column is contained by the value."
    );
    bind_value_oper!(
        overlaps,
        Overlap,
        "`&&`: the column and the value share an element or a point."
    );
    bind_value_oper!(
        strictly_left_of,
        LShift,
        "`<<`: the column lies wholly left of the value."
    );
    bind_value_oper!(
        strictly_right_of,
        RShift,
        "`>>`: the column lies wholly right of the value."
    );
    bind_value_oper!(
        not_extend_right_of,
        NotExtendRightOf,
        "`&<`: the column does not extend right of the value."
    );
    bind_value_oper!(
        not_extend_left_of,
        NotExtendLeftOf,
        "`&>`: the column does not extend left of the value."
    );
    bind_value_oper!(
        adjacent_to,
        AdjacentTo,
        "`-|-`: the column and the value are adjacent, sharing no point with \
         nothing between them."
    );

    bind_subquery_func!(in_subquery, eq, any);
    bind_subquery_func!(not_in_subquery, ne, all);

    /// Construct a [`SimpleExpr::Column`] wrapped in [`Expr`].
    fn into_expr(self) -> Expr {
        Expr::expr(self.into_simple_expr())
    }

    /// Construct a returning [`Expr`].
    #[allow(clippy::match_single_binding)]
    fn into_returning_expr(self) -> Expr {
        Expr::col(self)
    }

    /// Cast column expression used in select statement.
    /// It only cast database enum as text if it's an enum column.
    fn select_as(&self, expr: Expr) -> SimpleExpr {
        self.select_enum_as(expr)
    }

    /// Cast enum column as text; do nothing if `self` is not an enum.
    ///
    /// A column of a multirange a schema created is cast to `text` too, and
    /// an array of one to `text[]`: the driver reports such a multirange as a
    /// simple type with no subtype to read its binary form by, so it is read
    /// as its text, which the newtype's `FromStr` parses. A `money` column is
    /// read as `numeric`.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+17]
    // [spec:pgorm:def:sql.value.created-range+9]
    // [spec:pgorm:req:sql.ast.cast-shape+2]
    fn select_enum_as(&self, expr: Expr) -> SimpleExpr {
        select_cast(self.def().get_column_type(), expr)
    }

    /// Cast value of a column into the correct type for database storage.
    /// It only cast text as enum type if it's an enum column.
    fn save_as(&self, val: Expr) -> SimpleExpr {
        self.save_enum_as(val)
    }

    /// Cast an array of values into the column's array type for database
    /// storage, the array counterpart of [`ColumnTrait::save_as`]: an enum
    /// column takes `{enum_name}[]`, and every other column is left untouched.
    ///
    /// A column that overrides `save_as` with a cast of its own — what
    /// `#[pgorm(save_as = "…")]` generates — overrides this too, with the
    /// array spelling of the same type.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+17]
    fn save_array_as(&self, val: Expr) -> SimpleExpr {
        self.save_enum_array_as(val)
    }

    /// Cast an array of values as the column's enum array type; do nothing if
    /// `self` is not an enum. The array counterpart of
    /// [`ColumnTrait::save_enum_as`], and like it the fallback a derived
    /// [`save_array_as`][ColumnTrait::save_array_as] override keeps for
    /// columns without a `save_as` attribute. A `money` column's array is
    /// cast to its `money[]`, and the array of a column
    /// of a range type a schema created goes through
    /// [`ExprTrait::as_range`] to the type's array, each range written as its text.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+17]
    // [spec:pgorm:def:sql.value.created-range+9]
    // [spec:pgorm:req:sql.ast.cast-shape+2]
    fn save_enum_array_as(&self, val: Expr) -> SimpleExpr {
        let col_def = self.def();
        let col_type = col_def.get_column_type();
        if let Some(money) = money_type_name(col_type) {
            return val.cast_as_type(money.array());
        }
        if let Some(created) = created_range_type_name(col_type) {
            return val.as_range(created.array());
        }
        match enum_type_name(col_type) {
            Some(type_name) => val.cast_as_type(type_name.array()),
            None => val.into(),
        }
    }

    /// Cast value of an enum column as enum type; do nothing if `self` is not an enum.
    /// Will also transform `Array(Vec<Json>)` into `Json(Vec<Json>)` if the column type is `Json`.
    ///
    /// A column of a range type a schema created takes its value through
    /// [`ExprTrait::as_range`]: a range is written as its text form and cast to the
    /// type by name, there being no cast to it from a built-in range type. A
    /// `money` column's value is cast to its `money`.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+17]
    // [spec:pgorm:def:sql.value.created-range+9]
    // [spec:pgorm:req:sql.ast.cast-shape+2]
    fn save_enum_as(&self, val: Expr) -> SimpleExpr {
        save_cast(self.def().get_column_type(), val)
    }
}

/// The read cast [`ColumnTrait::select_enum_as`] gives a column of
/// `col_type`: an enum or a created multirange, or an array of either, read
/// as `text` or `text[]`, a `money` as `numeric`, and anything else as it
/// is. A column described only at run time, as a disjoint shape's is, is
/// read by this alone.
// [spec:pgorm:sem:entity.traits.column.enum-cast+17]
// [spec:pgorm:def:sql.value.created-range+9]
// [spec:pgorm:req:sql.ast.cast-shape+2]
pub(crate) fn select_cast(col_type: &ColumnType, expr: Expr) -> SimpleExpr {
    match under_domains(col_type) {
        ColumnType::CreatedMultirange { .. } => {
            return expr.cast_as_type(pgorm_query::TypeName::new(Text));
        }
        ColumnType::Array(member)
            if matches!(under_domains(member), ColumnType::CreatedMultirange { .. }) =>
        {
            return expr.cast_as_type(pgorm_query::TypeName::new(Text).array());
        }
        _ => {}
    }
    if money_type_name(col_type).is_some() {
        return expr.cast_as_type(money_carrier(holds_array(col_type)));
    }
    cast_enum_as(expr, col_type, |col, _, col_type| {
        let text = pgorm_query::TypeName::new(Text);
        col.cast_as_type(match holds_array(col_type) {
            true => text.array(),
            false => text,
        })
    })
}

/// The write cast [`ColumnTrait::save_enum_as`] gives a value of a column of
/// `col_type`: cast to the column's enum, `money` or created range, and
/// anything else as it is.
// [spec:pgorm:sem:entity.traits.column.enum-cast+17]
// [spec:pgorm:def:sql.value.created-range+9]
// [spec:pgorm:req:sql.ast.cast-shape+2]
pub(crate) fn save_cast(col_type: &ColumnType, val: Expr) -> SimpleExpr {
    if let Some(money) = money_type_name(col_type) {
        return val.cast_as_type(money);
    }
    match created_range_type_name(col_type) {
        Some(type_name) => val.as_range(type_name),
        None => cast_enum_as(val, col_type, |col, type_name, _| {
            col.cast_as_type(type_name)
        }),
    }
}

/// The `text` read-cast target of [`ColumnTrait::select_enum_as`]; its array
/// form is the same name under [`pgorm_query::TypeName::array`].
struct Text;

impl SqlName for Text {
    fn unquoted(&self, s: &mut dyn std::fmt::Write) {
        write!(s, "text").expect("write to sql sink");
    }
}

#[cfg(test)]
mod tests {
    use crate::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryTrait, tests_cfg::*};
    use pgorm_query::Query;

    #[test]
    fn test_in_subquery_1() {
        assert_eq!(
            cake::Entity::find()
                .filter(
                    Condition::any().add(
                        cake::Column::Id.in_subquery(
                            Query::select()
                                .expr(cake::Column::Id.max())
                                .from(cake::Entity)
                                .to_owned()
                        )
                    )
                )
                .as_query()
                .to_string(),
            [
                r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
                r#"WHERE "cake"."id" IN (SELECT MAX("cake"."id") FROM "cake")"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn test_in_subquery_2() {
        assert_eq!(
            cake::Entity::find()
                .filter(
                    Condition::any().add(
                        cake::Column::Id.in_subquery(
                            Query::select()
                                .column(cake_filling::Column::CakeId)
                                .from(cake_filling::Entity)
                                .to_owned()
                        )
                    )
                )
                .as_query()
                .to_string(),
            [
                r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
                r#"WHERE "cake"."id" IN (SELECT "cake_id" FROM "cake_filling")"#,
            ]
            .join(" ")
        );
    }

    #[test]
    fn test_col_from_str() {
        use std::str::FromStr;

        assert!(matches!(
            fruit::Column::from_str("id"),
            Ok(fruit::Column::Id)
        ));
        assert!(matches!(
            fruit::Column::from_str("name"),
            Ok(fruit::Column::Name)
        ));
        assert!(matches!(
            fruit::Column::from_str("cake_id"),
            Ok(fruit::Column::CakeId)
        ));
        assert!(matches!(
            fruit::Column::from_str("cakeId"),
            Ok(fruit::Column::CakeId)
        ));
        assert!(matches!(
            fruit::Column::from_str("does_not_exist"),
            Err(crate::ColumnFromStrError(_))
        ));
    }

    #[test]
    #[cfg(feature = "macros")]
    fn entity_model_column_1() {
        use crate::prelude::*;

        mod hello {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                pub one: i32,
                #[pgorm(unique)]
                pub two: i16,
                #[pgorm(indexed)]
                pub three: i16,
                #[pgorm(nullable)]
                pub four: i32,
                #[pgorm(unique, indexed, nullable)]
                pub five: i64,
                #[pgorm(nullable)]
                pub eight: i64,
                #[pgorm(default_expr = Expr::current_timestamp())]
                pub ten: DateTimeWithTimeZone,
                #[pgorm(default_value = 7)]
                pub eleven: i16,
                #[pgorm(default_value = "twelve_value")]
                pub twelve: String,
                #[pgorm(default_expr = Expr::val("twelve_value"))]
                pub twelve_two: String,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        assert_eq!(hello::Column::One.def(), ColumnType::Integer.def());
        assert_eq!(
            hello::Column::Two.def(),
            ColumnType::SmallInteger.def().unique()
        );
        assert_eq!(
            hello::Column::Three.def(),
            ColumnType::SmallInteger.def().indexed()
        );
        assert_eq!(
            hello::Column::Four.def(),
            ColumnType::Integer.def().nullable()
        );
        assert_eq!(
            hello::Column::Five.def(),
            ColumnType::BigInteger.def().unique().indexed().nullable()
        );
        assert_eq!(
            hello::Column::Eight.def(),
            ColumnType::BigInteger.def().nullable()
        );
        assert_eq!(
            hello::Column::Ten.def(),
            ColumnType::TimestampWithTimeZone
                .def()
                .default(Expr::current_timestamp())
        );
        assert_eq!(
            hello::Column::Eleven.def(),
            ColumnType::SmallInteger.def().default(7)
        );
        assert_eq!(
            hello::Column::Twelve.def(),
            ColumnType::String(StringLen::None)
                .def()
                .default("twelve_value")
        );
        assert_eq!(
            hello::Column::TwelveTwo.def(),
            ColumnType::String(StringLen::None)
                .def()
                .default("twelve_value")
        );
    }

    #[test]
    #[cfg(feature = "macros")]
    fn column_name_1() {
        use pgorm_query::SqlName;

        mod hello {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                #[pgorm(column_name = "ONE")]
                pub one: i32,
                pub two: i32,
                #[pgorm(column_name = "3")]
                pub three: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        assert_eq!(hello::Column::One.to_string().as_str(), "ONE");
        assert_eq!(hello::Column::Two.to_string().as_str(), "two");
        assert_eq!(hello::Column::Three.to_string().as_str(), "3");
    }

    #[test]
    #[cfg(feature = "macros")]
    fn enum_name_1() {
        use pgorm_query::SqlName;

        mod hello {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                #[pgorm(variant = One1)]
                pub one: i32,
                pub two: i32,
                #[pgorm(variant = Three3)]
                pub three: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        assert_eq!(hello::Column::One1.to_string().as_str(), "one1");
        assert_eq!(hello::Column::Two.to_string().as_str(), "two");
        assert_eq!(hello::Column::Three3.to_string().as_str(), "three3");
    }

    #[test]
    #[cfg(feature = "macros")]
    fn column_name_enum_name_1() {
        use pgorm_query::SqlName;

        #[allow(clippy::enum_variant_names)]
        mod hello {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello")]
            pub struct Model {
                #[pgorm(primary_key, column_name = "ID", variant = IdentityColumn)]
                pub id: i32,
                #[pgorm(column_name = "ONE", variant = One1)]
                pub one: i32,
                pub two: i32,
                #[pgorm(column_name = "THREE", variant = Three3)]
                pub three: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        assert_eq!(hello::Column::IdentityColumn.to_string().as_str(), "ID");
        assert_eq!(hello::Column::One1.to_string().as_str(), "ONE");
        assert_eq!(hello::Column::Two.to_string().as_str(), "two");
        assert_eq!(hello::Column::Three3.to_string().as_str(), "THREE");
    }

    #[test]
    #[cfg(feature = "macros")]
    fn column_name_enum_name_3() {
        use pgorm_query::SqlName;

        mod my_entity {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "my_entity")]
            pub struct Model {
                #[pgorm(primary_key, variant = IdentityColumn, column_name = "id")]
                pub id: i32,
                #[pgorm(column_name = "type")]
                pub type_: String,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        assert_eq!(my_entity::Column::IdentityColumn.to_string().as_str(), "id");
        assert_eq!(my_entity::Column::Type.to_string().as_str(), "type");
    }

    #[test]
    #[cfg(feature = "macros")]
    fn select_as_1() {
        use crate::{ActiveModelTrait, ActiveValue, Update};

        #[allow(clippy::enum_variant_names)]
        mod hello_with {
            use crate as pgorm;
            use crate::entity::prelude::*;
            use crate::pgorm_query::SimpleExpr;

            fn as_integer(expr: Expr) -> SimpleExpr {
                expr.cast_as(alias("integer"))
            }

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                #[pgorm(variant = One1)]
                pub one: i32,
                #[pgorm(select_with = as_integer)]
                pub two: i32,
                #[pgorm(variant = Three3)]
                pub three: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        #[allow(clippy::enum_variant_names)]
        mod hello_compact {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                #[pgorm(variant = One1)]
                pub one: i32,
                #[pgorm(select_as = "integer")]
                pub two: i32,
                #[pgorm(variant = Three3)]
                pub three: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        fn assert_it<E, A>(active_model: A)
        where
            E: EntityTrait,
            A: ActiveModelTrait<Entity = E>,
        {
            assert_eq!(
                E::find().as_query().to_string(),
                r#"SELECT "hello"."id", "hello"."one1", CAST("hello"."two" AS integer), "hello"."three3" FROM "hello""#,
            );
            assert_eq!(
                Update::one(active_model)
                    .expect("the primary key is set")
                    .as_query()
                    .to_string(),
                r#"UPDATE "hello" SET "one1" = 1, "two" = 2, "three3" = 3 WHERE "hello"."id" = 1"#,
            );
        }

        assert_it(hello_with::ActiveModel {
            id: ActiveValue::set(1),
            one: ActiveValue::set(1),
            two: ActiveValue::set(2),
            three: ActiveValue::set(3),
        });
        assert_it(hello_compact::ActiveModel {
            id: ActiveValue::set(1),
            one: ActiveValue::set(1),
            two: ActiveValue::set(2),
            three: ActiveValue::set(3),
        });
    }

    #[test]
    #[cfg(feature = "macros")]
    fn save_as_1() {
        use crate::{ActiveModelTrait, ActiveValue, QueryFilter, QueryTrait, Update};

        #[allow(clippy::enum_variant_names)]
        mod hello_with {
            use crate as pgorm;
            use crate::entity::prelude::*;
            use crate::pgorm_query::{SimpleExpr, TypeName};

            fn as_text(val: Expr) -> SimpleExpr {
                val.cast_as(alias("text"))
            }

            fn as_texts(val: Expr) -> SimpleExpr {
                val.cast_as_type(TypeName::new(alias("text")).array())
            }

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                #[pgorm(variant = One1)]
                pub one: i32,
                #[pgorm(save_with = as_text, save_array_with = as_texts)]
                pub two: i32,
                #[pgorm(variant = Three3)]
                pub three: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        #[allow(clippy::enum_variant_names)]
        mod hello_compact {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                #[pgorm(variant = One1)]
                pub one: i32,
                #[pgorm(save_as = "text")]
                pub two: i32,
                #[pgorm(variant = Three3)]
                pub three: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        fn assert_it<E, A>(active_model: A)
        where
            E: EntityTrait,
            A: ActiveModelTrait<Entity = E>,
        {
            assert_eq!(
                E::find().as_query().to_string(),
                r#"SELECT "hello"."id", "hello"."one1", "hello"."two", "hello"."three3" FROM "hello""#,
            );
            assert_eq!(
                Update::one(active_model)
                    .expect("the primary key is set")
                    .as_query()
                    .to_string(),
                r#"UPDATE "hello" SET "one1" = 1, "two" = CAST(2 AS text), "three3" = 3 WHERE "hello"."id" = 1"#,
            );
        }

        assert_it(hello_with::ActiveModel {
            id: ActiveValue::set(1),
            one: ActiveValue::set(1),
            two: ActiveValue::set(2),
            three: ActiveValue::set(3),
        });
        assert_it(hello_compact::ActiveModel {
            id: ActiveValue::set(1),
            one: ActiveValue::set(1),
            two: ActiveValue::set(2),
            three: ActiveValue::set(3),
        });

        // `save_array_with` is the array operand's conversion, as `text[]` is
        // `save_as`'s.
        let any = r#"SELECT "hello"."id", "hello"."one1", "hello"."two", "hello"."three3" FROM "hello" WHERE "hello"."two" = ANY(CAST(ARRAY [1,2] AS text[]))"#;
        assert_eq!(
            hello_with::Entity::find()
                .filter(hello_with::Column::Two.eq_any([1, 2]))
                .as_query()
                .to_string(),
            any,
        );
        assert_eq!(
            hello_compact::Entity::find()
                .filter(hello_compact::Column::Two.eq_any([1, 2]))
                .as_query()
                .to_string(),
            any,
        );
    }

    #[test]
    #[cfg(feature = "macros")]
    fn save_as_covers_every_value_predicate() {
        use crate::pgorm_query::{BinOper, SimpleExpr};
        use crate::tests_cfg::{active_enums::Tea, lunch_set};
        use crate::{QueryFilter, QueryTrait};

        mod guarded {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "guarded")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                #[pgorm(save_as = "text")]
                pub two: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        let clause = |expr: SimpleExpr| {
            let sql = guarded::Entity::find().filter(expr).as_query().to_string();
            sql.split_once("WHERE ")
                .expect("a WHERE clause")
                .1
                .to_owned()
        };

        assert_eq!(
            clause(guarded::Column::Two.between(1, 2)),
            r#""guarded"."two" BETWEEN CAST(1 AS text) AND CAST(2 AS text)"#
        );
        assert_eq!(
            clause(guarded::Column::Two.not_between(1, 2)),
            r#""guarded"."two" NOT BETWEEN CAST(1 AS text) AND CAST(2 AS text)"#
        );
        assert_eq!(
            clause(guarded::Column::Two.between_symmetric(2, 1)),
            r#""guarded"."two" BETWEEN SYMMETRIC CAST(2 AS text) AND CAST(1 AS text)"#
        );
        assert_eq!(
            clause(guarded::Column::Two.not_between_symmetric(2, 1)),
            r#""guarded"."two" NOT BETWEEN SYMMETRIC CAST(2 AS text) AND CAST(1 AS text)"#
        );
        assert_eq!(
            clause(guarded::Column::Two.is_distinct_from(1)),
            r#""guarded"."two" IS DISTINCT FROM CAST(1 AS text)"#
        );
        assert_eq!(
            clause(guarded::Column::Two.is_not_distinct_from(1)),
            r#""guarded"."two" IS NOT DISTINCT FROM CAST(1 AS text)"#
        );
        assert_eq!(
            clause(guarded::Column::Two.overlaps(1)),
            r#""guarded"."two" && CAST(1 AS text)"#
        );
        assert_eq!(
            clause(guarded::Column::Two.includes(1)),
            r#""guarded"."two" @> CAST(1 AS text)"#
        );
        assert_eq!(
            clause(guarded::Column::Two.adjacent_to(1)),
            r#""guarded"."two" -|- CAST(1 AS text)"#
        );
        assert_eq!(
            clause(guarded::Column::Two.compare_any(BinOper::GreaterThan, vec![1, 2])),
            r#""guarded"."two" > ANY(CAST(ARRAY [1,2] AS text[]))"#
        );
        assert_eq!(
            clause(guarded::Column::Two.compare_all(BinOper::SmallerThan, vec![1, 2])),
            r#""guarded"."two" < ALL(CAST(ARRAY [1,2] AS text[]))"#
        );
        assert_eq!(
            clause(guarded::Column::Two.eq_any([1, 2])),
            r#""guarded"."two" = ANY(CAST(ARRAY [1,2] AS text[]))"#
        );
        assert_eq!(
            clause(guarded::Column::Two.ne_all([1, 2])),
            r#""guarded"."two" <> ALL(CAST(ARRAY [1,2] AS text[]))"#
        );
        assert_eq!(
            clause(guarded::Column::Two.is_in([1, 2])),
            r#""guarded"."two" IN (CAST(1 AS text), CAST(2 AS text))"#
        );

        let tea_clause = |expr: SimpleExpr| {
            let sql = lunch_set::Entity::find()
                .filter(expr)
                .as_query()
                .to_string();
            sql.split_once("WHERE ")
                .expect("a WHERE clause")
                .1
                .to_owned()
        };

        assert_eq!(
            tea_clause(lunch_set::Column::Tea.between(Tea::BreakfastTea, Tea::EverydayTea)),
            r#""lunch_set"."tea" BETWEEN CAST('BreakfastTea' AS tea) AND CAST('EverydayTea' AS tea)"#
        );
    }

    #[test]
    #[cfg(feature = "macros")]
    fn select_as_and_value_1() {
        use crate::{ActiveModelTrait, ActiveValue, Update};

        #[allow(clippy::enum_variant_names)]
        mod hello_with {
            use crate as pgorm;
            use crate::entity::prelude::*;
            use crate::pgorm_query::{SimpleExpr, TypeName};

            fn as_integer(expr: Expr) -> SimpleExpr {
                expr.cast_as(alias("integer"))
            }

            fn as_text(val: Expr) -> SimpleExpr {
                val.cast_as(alias("text"))
            }

            fn as_texts(val: Expr) -> SimpleExpr {
                val.cast_as_type(TypeName::new(alias("text")).array())
            }

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                #[pgorm(variant = One1)]
                pub one: i32,
                #[pgorm(select_with = as_integer, save_with = as_text, save_array_with = as_texts)]
                pub two: i32,
                #[pgorm(variant = Three3)]
                pub three: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        #[allow(clippy::enum_variant_names)]
        mod hello_compact {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                #[pgorm(variant = One1)]
                pub one: i32,
                #[pgorm(select_as = "integer", save_as = "text")]
                pub two: i32,
                #[pgorm(variant = Three3)]
                pub three: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        fn assert_it<E, A>(active_model: A)
        where
            E: EntityTrait,
            A: ActiveModelTrait<Entity = E>,
        {
            assert_eq!(
                E::find().as_query().to_string(),
                r#"SELECT "hello"."id", "hello"."one1", CAST("hello"."two" AS integer), "hello"."three3" FROM "hello""#,
            );
            assert_eq!(
                Update::one(active_model)
                    .expect("the primary key is set")
                    .as_query()
                    .to_string(),
                r#"UPDATE "hello" SET "one1" = 1, "two" = CAST(2 AS text), "three3" = 3 WHERE "hello"."id" = 1"#,
            );
        }

        assert_it(hello_with::ActiveModel {
            id: ActiveValue::set(1),
            one: ActiveValue::set(1),
            two: ActiveValue::set(2),
            three: ActiveValue::set(3),
        });
        assert_it(hello_compact::ActiveModel {
            id: ActiveValue::set(1),
            one: ActiveValue::set(1),
            two: ActiveValue::set(2),
            three: ActiveValue::set(3),
        });
    }
}
