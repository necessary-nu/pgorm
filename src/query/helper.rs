use crate::entity::{compared_column, compared_value};
use crate::executor::result_name::bounded_identifier;
use crate::{
    ColumnPairs, ColumnTrait, EntityTrait, IntoKey, IntoSimpleExpr, Iterable, ModelTrait,
    PrimaryKeyToColumn, QueryTrait, RelationDef, RelationTrait,
};
use pgorm_query::{
    AnyWithClause, ConditionType, Expr, ExprTrait, FromItem, FunctionCall, GroupingElement,
    IntoCondition, IntoName, LockBehavior, LockType, NullOrdering, SelectExpr, SelectStatement,
    SimpleExpr, TypeName, UnionType, WindowDefinition,
};
pub use pgorm_query::{Condition, ConditionalStatement, JoinType, Name, Order, OrderedStatement};

use pgorm_query::IntoColumnRef;

// LINT: when the column does not appear in tables selected from
// LINT: when there is a group by clause, but some columns don't have aggregate functions
// LINT: when the join table or column does not exists
/// Abstract API for performing queries
// [spec:pgorm:sem:query.build.modifiers+11]
pub trait QuerySelect: Sized {
    #[allow(missing_docs)]
    type QueryStatement;

    /// The state this builder moves to once a projection expression is added.
    ///
    /// Builders whose projection is already meaningful —
    /// [`Select<E>`](crate::Select), the two-model selectors,
    /// [`Cursor`](crate::Cursor) — project onto themselves. The typestate pair
    /// produced by `select_only` uses it to step
    /// [`SelectCustom<E>`](crate::SelectCustom) forward to
    /// [`SelectProjected<E>`](crate::SelectProjected), which is where the
    /// terminal operations live.
    type Projected: QuerySelect<Projected = Self::Projected>;

    /// Add the select SQL statement
    fn query(&mut self) -> &mut SelectStatement;

    /// Move to [`QuerySelect::Projected`] once a projection expression has
    /// been written into the statement.
    #[doc(hidden)]
    fn into_projected(self) -> Self::Projected;

    /// Add a select column
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .select_only()
    ///         .column(cake::Column::Name)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."name" FROM "cake""#
    /// );
    /// ```
    ///
    /// Enum column will be casted into text
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::lunch_set};
    ///
    /// assert_eq!(
    ///     lunch_set::Entity::find()
    ///         .select_only()
    ///         .column(lunch_set::Column::Tea)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT CAST("lunch_set"."tea" AS text) FROM "lunch_set""#
    /// );
    /// ```
    fn column<C>(mut self, col: C) -> Self::Projected
    where
        C: ColumnTrait,
    {
        self.query().expr(col.select_as(col.into_expr()));
        self.into_projected()
    }

    /// Add a select column with alias
    /// ```
    /// use pgorm::{alias, entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .select_only()
    ///         .column_as(cake::Column::Id.count(), alias("count"))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT COUNT("cake"."id") AS "count" FROM "cake""#
    /// );
    /// ```
    fn column_as<C, I>(mut self, col: C, alias: I) -> Self::Projected
    where
        C: IntoSimpleExpr,
        I: IntoKey,
    {
        self.query().expr(SelectExpr::new_as(
            col.into_simple_expr(),
            Name::new(alias.into_key()),
        ));
        self.into_projected()
    }

    /// Select columns
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .select_only()
    ///         .columns([cake::Column::Id, cake::Column::Name])
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake""#
    /// );
    /// ```
    ///
    /// Conditionally select all columns expect a specific column
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .select_only()
    ///         .columns(cake::Column::iter().filter(|col| match col {
    ///             cake::Column::Id => false,
    ///             _ => true,
    ///         }))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."name" FROM "cake""#
    /// );
    /// ```
    ///
    /// Enum column will be casted into text
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::lunch_set};
    ///
    /// assert_eq!(
    ///     lunch_set::Entity::find()
    ///         .select_only()
    ///         .columns([lunch_set::Column::Name, lunch_set::Column::Tea])
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "lunch_set"."name", CAST("lunch_set"."tea" AS text) FROM "lunch_set""#
    /// );
    /// ```
    ///
    /// An empty iterator adds nothing, so this is the one projection method
    /// that can leave the select list empty; the execution-boundary guard
    /// documented on [`QuerySelect`] is what catches that.
    fn columns<C, I>(mut self, cols: I) -> Self::Projected
    where
        C: ColumnTrait,
        I: IntoIterator<Item = C>,
    {
        for col in cols.into_iter() {
            self.query().expr(col.select_as(col.into_expr()));
        }
        self.into_projected()
    }

    /// Add an offset expression. Passing in None would remove the offset.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .offset(10)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" OFFSET 10"#
    /// );
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .offset(Some(10))
    ///         .offset(Some(20))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" OFFSET 20"#
    /// );
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .offset(10)
    ///         .offset(None)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake""#
    /// );
    /// ```
    fn offset<T>(mut self, offset: T) -> Self
    where
        T: Into<Option<u64>>,
    {
        if let Some(offset) = offset.into() {
            self.query().offset(offset);
        } else {
            self.query().reset_offset();
        }
        self
    }

    /// Add a limit expression. Passing in None would remove the limit.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .limit(10)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" LIMIT 10"#
    /// );
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .limit(Some(10))
    ///         .limit(Some(20))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" LIMIT 20"#
    /// );
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .limit(10)
    ///         .limit(None)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake""#
    /// );
    /// ```
    fn limit<T>(mut self, limit: T) -> Self
    where
        T: Into<Option<u64>>,
    {
        if let Some(limit) = limit.into() {
            self.query().limit(limit);
        } else {
            self.query().reset_limit();
        }
        self
    }

    /// Add a group by column
    /// ```
    /// use pgorm::{alias, entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .select_only()
    ///         .column(cake::Column::Name)
    ///         .group_by(cake::Column::Name)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."name" FROM "cake" GROUP BY "cake"."name""#
    /// );
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .select_only()
    ///         .column_as(cake::Column::Id.count(), alias("count"))
    ///         .column_as(cake::Column::Id.sum(), alias("sum_of_id"))
    ///         .group_by(cake::Column::Name)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT COUNT("cake"."id") AS "count", SUM("cake"."id") AS "sum_of_id" FROM "cake" GROUP BY "cake"."name""#
    /// );
    /// ```
    fn group_by<C>(mut self, col: C) -> Self
    where
        C: IntoSimpleExpr,
    {
        self.query().add_group_by([col.into_simple_expr()]);
        self
    }

    /// Add a grouping element — `ROLLUP`, `CUBE`, `GROUPING SETS` or `()` —
    /// to the GROUP BY list, after anything [`group_by`](Self::group_by)
    /// already put there
    /// ```
    /// use pgorm::{entity::*, pgorm_query::{ExprTrait, Func, GroupingElement}, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .select_only()
    ///         .column(cake::Column::Name)
    ///         .expr(Func::grouping(cake::Column::Name.into_expr()))
    ///         .column_as(cake::Column::Id.count(), pgorm::alias("count"))
    ///         .group_by_element(GroupingElement::rollup([cake::Column::Name.into_expr()]))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."name", GROUPING("cake"."name"), COUNT("cake"."id") AS "count" FROM "cake" GROUP BY ROLLUP ("cake"."name")"#
    /// );
    /// ```
    fn group_by_element<G>(mut self, element: G) -> Self
    where
        G: Into<GroupingElement>,
    {
        self.query().group_by_element(element);
        self
    }

    /// Add an AND HAVING expression
    /// ```
    /// use pgorm::{alias, pgorm_query::{Expr, ExprTrait}, entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .having(cake::Column::Id.eq(4))
    ///         .having(cake::Column::Id.eq(5))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" HAVING "cake"."id" = 4 AND "cake"."id" = 5"#
    /// );
    ///
    /// let count = alias("count");
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .select_only()
    ///         .column_as(cake::Column::Id.count(), count)
    ///         .column_as(cake::Column::Id.sum(), alias("sum_of_id"))
    ///         .group_by(cake::Column::Name)
    ///         .having(Expr::col(count).gt(6))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT COUNT("cake"."id") AS "count", SUM("cake"."id") AS "sum_of_id" FROM "cake" GROUP BY "cake"."name" HAVING "count" > 6"#
    /// );
    /// ```
    fn having<F>(mut self, filter: F) -> Self
    where
        F: IntoCondition,
    {
        self.query().cond_having(filter.into_condition());
        self
    }

    /// Add a DISTINCT expression
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    /// struct Input {
    ///     name: Option<String>,
    /// }
    /// let input = Input {
    ///     name: Some("cheese".to_owned()),
    /// };
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(
    ///             Condition::all().add_option(input.name.map(|n| cake::Column::Name.contains(&n)))
    ///         )
    ///         .distinct()
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT DISTINCT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" LIKE '%cheese%'"#
    /// );
    /// ```
    fn distinct(mut self) -> Self {
        self.query().distinct();
        self
    }

    /// Add a DISTINCT ON expression
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    /// struct Input {
    ///     name: Option<String>,
    /// }
    /// let input = Input {
    ///     name: Some("cheese".to_owned()),
    /// };
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(
    ///             Condition::all().add_option(input.name.map(|n| cake::Column::Name.contains(&n)))
    ///         )
    ///         .distinct_on([(cake::Entity, cake::Column::Name)])
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT DISTINCT ON ("cake"."name") "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" LIKE '%cheese%'"#
    /// );
    /// ```
    fn distinct_on<T, I>(mut self, cols: I) -> Self
    where
        T: IntoColumnRef,
        I: IntoIterator<Item = T>,
    {
        self.query().distinct_on(cols);
        self
    }

    #[doc(hidden)]
    fn join_join(mut self, join: JoinType, rel: RelationDef, via: Option<RelationDef>) -> Self {
        if let Some(via) = via {
            self = self.join(join, via)
        }
        self.join(join, rel)
    }

    #[doc(hidden)]
    fn join_join_rev(mut self, join: JoinType, rel: RelationDef, via: Option<RelationDef>) -> Self {
        self = self.join_rev(join, rel);
        if let Some(via) = via {
            self = self.join_rev(join, via)
        }
        self
    }

    /// Join via [`RelationDef`].
    fn join(mut self, join: JoinType, rel: RelationDef) -> Self {
        self.query()
            .join(join, rel.to_tbl.clone(), join_condition(rel));
        self
    }

    /// Join via [`RelationDef`] but in reverse direction.
    /// Assume when there exist a relation A to B.
    /// You can reverse join B from A.
    fn join_rev(mut self, join: JoinType, rel: RelationDef) -> Self {
        self.query()
            .join(join, rel.from_tbl.clone(), join_condition(rel));
        self
    }

    /// Join via [`RelationDef`] with table alias.
    fn join_as<I>(mut self, join: JoinType, mut rel: RelationDef, alias: I) -> Self
    where
        I: IntoName,
    {
        let alias = alias.into_name();
        rel.to_tbl = rel.to_tbl.alias(Name::clone(&alias));
        self.query()
            .join(join, rel.to_tbl.clone(), join_condition(rel));
        self
    }

    /// Join via [`RelationDef`] with table alias but in reverse direction.
    /// Assume when there exist a relation A to B.
    /// You can reverse join B from A.
    fn join_as_rev<I>(mut self, join: JoinType, mut rel: RelationDef, alias: I) -> Self
    where
        I: IntoName,
    {
        let alias = alias.into_name();
        rel.from_tbl = rel.from_tbl.alias(Name::clone(&alias));
        self.query()
            .join(join, rel.from_tbl.clone(), join_condition(rel));
        self
    }

    /// `LEFT JOIN` the entity a relation points at.
    ///
    /// The relation names itself; the join type is in the method name and the
    /// [`RelationDef`] is taken from the relation, so neither has to be
    /// spelled at the call site.
    ///
    /// ```
    /// use pgorm::{EntityTrait, QuerySelect, QueryTrait, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .left_join_rel(cake::Relation::Fruit)
    ///         .as_query()
    ///         .to_string(),
    ///     [
    ///         r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
    ///         r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:sem:query.build.join.rel]
    fn left_join_rel<R>(self, rel: R) -> Self
    where
        R: RelationTrait,
    {
        self.join(JoinType::LeftJoin, rel.def())
    }

    /// `INNER JOIN` the entity a relation points at.
    ///
    /// ```
    /// use pgorm::{EntityTrait, QuerySelect, QueryTrait, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .inner_join_rel(cake::Relation::Fruit)
    ///         .as_query()
    ///         .to_string(),
    ///     [
    ///         r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
    ///         r#"INNER JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    fn inner_join_rel<R>(self, rel: R) -> Self
    where
        R: RelationTrait,
    {
        self.join(JoinType::InnerJoin, rel.def())
    }

    /// `RIGHT JOIN` the entity a relation points at.
    fn right_join_rel<R>(self, rel: R) -> Self
    where
        R: RelationTrait,
    {
        self.join(JoinType::RightJoin, rel.def())
    }

    /// `LEFT JOIN` the entity a relation points *from*.
    ///
    /// The `_rev` methods are [`join_rev`](QuerySelect::join_rev)'s direction:
    /// given a relation A to B, they join A while selecting from B.
    ///
    /// ```
    /// use pgorm::{EntityTrait, QuerySelect, QueryTrait, tests_cfg::{cake, fruit}};
    ///
    /// assert_eq!(
    ///     fruit::Entity::find()
    ///         .left_join_rel_rev(cake::Relation::Fruit)
    ///         .as_query()
    ///         .to_string(),
    ///     [
    ///         r#"SELECT "fruit"."id", "fruit"."name", "fruit"."cake_id" FROM "fruit""#,
    ///         r#"LEFT JOIN "cake" ON "cake"."id" = "fruit"."cake_id""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    fn left_join_rel_rev<R>(self, rel: R) -> Self
    where
        R: RelationTrait,
    {
        self.join_rev(JoinType::LeftJoin, rel.def())
    }

    /// `INNER JOIN` the entity a relation points *from*.
    fn inner_join_rel_rev<R>(self, rel: R) -> Self
    where
        R: RelationTrait,
    {
        self.join_rev(JoinType::InnerJoin, rel.def())
    }

    /// `RIGHT JOIN` the entity a relation points *from*.
    fn right_join_rel_rev<R>(self, rel: R) -> Self
    where
        R: RelationTrait,
    {
        self.join_rev(JoinType::RightJoin, rel.def())
    }

    /// Select lock
    fn lock(mut self, lock_type: LockType) -> Self {
        self.query().lock(lock_type);
        self
    }

    /// Select lock shared
    fn lock_shared(mut self) -> Self {
        self.query().lock_shared();
        self
    }

    /// Select lock exclusive
    fn lock_exclusive(mut self) -> Self {
        self.query().lock_exclusive();
        self
    }

    /// Row locking with behavior (if supported).
    ///
    /// See [`SelectStatement::lock_with_behavior`](pgorm_query::SelectStatement::lock_with_behavior).
    fn lock_with_behavior(mut self, r#type: LockType, behavior: LockBehavior) -> Self {
        self.query().lock_with_behavior(r#type, behavior);
        self
    }

    /// Add an expression to the select expression list.
    /// ```
    /// use pgorm::pgorm_query::Expr;
    /// use pgorm::{entity::*, tests_cfg::cake, QuerySelect, QueryTrait};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .select_only()
    ///         .expr(Expr::col((cake::Entity, cake::Column::Id)))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id" FROM "cake""#
    /// );
    /// ```
    fn expr<T>(mut self, expr: T) -> Self::Projected
    where
        T: Into<SelectExpr>,
    {
        self.query().expr(expr);
        self.into_projected()
    }

    /// Add select expressions from vector of [`SelectExpr`].
    /// ```
    /// use pgorm::pgorm_query::Expr;
    /// use pgorm::{entity::*, tests_cfg::cake, QuerySelect, QueryTrait};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .select_only()
    ///         .exprs([
    ///             Expr::col((cake::Entity, cake::Column::Id)),
    ///             Expr::col((cake::Entity, cake::Column::Name)),
    ///         ])
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake""#
    /// );
    /// ```
    ///
    /// Like [`columns`](QuerySelect::columns), an empty iterator adds nothing.
    fn exprs<T, I>(mut self, exprs: I) -> Self::Projected
    where
        T: Into<SelectExpr>,
        I: IntoIterator<Item = T>,
    {
        self.query().exprs(exprs);
        self.into_projected()
    }

    /// Select column.
    /// ```
    /// use pgorm::pgorm_query::{Expr, ExprTrait, Func};
    /// use pgorm::{alias, entity::*, tests_cfg::cake, QuerySelect, QueryTrait};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .expr_as(
    ///             Func::upper(Expr::col((cake::Entity, cake::Column::Name))),
    ///             alias("name_upper")
    ///         )
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name", UPPER("cake"."name") AS "name_upper" FROM "cake""#
    /// );
    /// ```
    fn expr_as<T, A>(mut self, expr: T, alias: A) -> Self::Projected
    where
        T: Into<SimpleExpr>,
        A: IntoKey,
    {
        self.query().expr_as(expr, alias.into_key());
        self.into_projected()
    }

    /// Shorthand of `expr_as(Expr::col((T, C)), A)`.
    ///
    /// ```
    /// use pgorm::pgorm_query::{Expr, ExprTrait, Func};
    /// use pgorm::{alias, entity::*, tests_cfg::cake, QuerySelect, QueryTrait};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .select_only()
    ///         .tbl_col_as((cake::Entity, cake::Column::Name), alias("cake_name"))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."name" AS "cake_name" FROM "cake""#
    /// );
    /// ```
    fn tbl_col_as<T, C, A>(mut self, (tbl, col): (T, C), alias: A) -> Self::Projected
    where
        T: IntoName + 'static,
        C: IntoName + 'static,
        A: IntoKey,
    {
        self.query()
            .expr_as(Expr::col((tbl, col)), alias.into_key());
        self.into_projected()
    }

    /// Prefix the query with a `WITH` clause — a
    /// [`WithClause`](pgorm_query::WithClause) or a
    /// [`RecursiveWithClause`](pgorm_query::RecursiveWithClause).
    ///
    /// This is [`SelectStatement::with`](pgorm_query::SelectStatement::with) at
    /// the ORM's own receiver: the clause is carried on the statement, not
    /// wrapped around it, so this returns `Self` and filters, ordering, joins,
    /// the typed terminals, the paginator and the cursor all keep working
    /// afterwards. The last call wins.
    ///
    /// A recursive CTE takes its column types from the anchor arm, where an
    /// unannotated `$n` placeholder resolves to `text`. Annotate any literal in
    /// that arm with [`cast_as`](pgorm_query::ExprTrait::cast_as) — see
    /// `[spec:pgorm:sem:sql.render.placeholder-typing+2]`.
    ///
    /// ```
    /// use pgorm::pgorm_query::{CommonTableExpression, Query, WithClause};
    /// use pgorm::{alias, entity::*, query::*, tests_cfg::cake};
    ///
    /// let cheap = CommonTableExpression::new(
    ///     alias("cheap"),
    ///     Query::select().column(cake::Column::Id).from(cake::Entity).to_owned(),
    /// );
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .with(WithClause::new(cheap))
    ///         .filter(cake::Column::Id.gt(1))
    ///         .as_query()
    ///         .to_string(),
    ///     concat!(
    ///         r#"WITH "cheap" AS (SELECT "id" FROM "cake") "#,
    ///         r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" > 1"#,
    ///     )
    /// );
    /// ```
    // [spec:pgorm:def:query.build.with+2]
    // [spec:pgorm:sem:query.build.with.attach+1]
    // [spec:pgorm:sem:sql.render.placeholder-typing+2]
    fn with<C>(mut self, clause: C) -> Self
    where
        C: Into<AnyWithClause>,
    {
        QuerySelect::query(&mut self).with(clause);
        self
    }

    /// `JOIN LATERAL (<sub>) AS <alias> ON <on>`.
    ///
    /// A lateral join mutates the select statement in place, so the builder's
    /// type — and with it the decode target — is unchanged.
    // [spec:pgorm:sem:query.build.lateral]
    fn join_lateral<T, C>(mut self, join: JoinType, sub: SelectStatement, alias: T, on: C) -> Self
    where
        T: IntoName,
        C: IntoCondition,
    {
        QuerySelect::query(&mut self).join_lateral(join, sub, alias, on);
        self
    }

    /// `JOIN LATERAL (<sub>) AS <alias> ON TRUE` — the top-N-per-group shape,
    /// where the correlation lives in the subquery's own `WHERE` and the join
    /// itself has nothing left to constrain.
    // [spec:pgorm:sem:query.build.lateral]
    fn join_lateral_on_true<T>(self, join: JoinType, sub: SelectStatement, alias: T) -> Self
    where
        T: IntoName,
    {
        self.join_lateral(join, sub, alias, SimpleExpr::Constant(true.into()))
    }

    /// Append a named window to the WINDOW clause, which the projection can
    /// refer to with `OVER "name"`: a `WindowStatement`, or an
    /// `ExtendedWindow` built on an entry appended before it. A clause
    /// PostgreSQL refuses — two entries of one name, an extended window over
    /// a base it cannot copy — is refused by every terminal before it sends
    /// anything (`SelectStatement::check_windows`).
    // [spec:pgorm:sem:query.build.window+2]
    fn window<A, W>(mut self, name: A, window: W) -> Self
    where
        A: IntoName,
        W: Into<WindowDefinition>,
    {
        QuerySelect::query(&mut self).window(name, window);
        self
    }

    /// Project a windowed aggregate: `<func>() OVER <window> AS <alias>`.
    ///
    /// The window is referenced by the name given to
    /// [`window`](QuerySelect::window). Being a projection, this steps the
    /// builder to [`Projected`](QuerySelect::Projected) exactly as
    /// [`column`](QuerySelect::column) and [`expr_as`](QuerySelect::expr_as) do.
    // [spec:pgorm:sem:query.build.window+2]
    fn window_expr_as<W, A>(mut self, func: FunctionCall, window: W, alias: A) -> Self::Projected
    where
        W: IntoName,
        A: IntoName,
    {
        QuerySelect::query(&mut self).expr_window_name_as(func, window, alias);
        self.into_projected()
    }

    /// Append a `UNION` / `UNION ALL` / `INTERSECT` / `EXCEPT` arm.
    ///
    /// Both arms are the same builder type, so both project the same columns in
    /// the same order and the combined result still decodes as whatever the
    /// first arm decoded as — which is also the arm PostgreSQL takes the result
    /// column names from.
    ///
    /// ```
    /// use pgorm::pgorm_query::UnionType;
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// let cheap = cake::Entity::find().filter(cake::Column::Id.lt(3));
    /// let dear = cake::Entity::find().filter(cake::Column::Id.gt(9));
    ///
    /// assert_eq!(
    ///     cheap.union(UnionType::All, dear).as_query().to_string(),
    ///     concat!(
    ///         r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" < 3 "#,
    ///         r#"UNION ALL (SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" > 9)"#,
    ///     )
    /// );
    /// ```
    // [spec:pgorm:sem:query.build.union]
    fn union(mut self, union_type: UnionType, other: Self) -> Self
    where
        Self: QueryTrait<QueryStatement = SelectStatement>,
    {
        let other = other.into_query();
        QuerySelect::query(&mut self).union(union_type, other);
        self
    }
}

// LINT: when the column does not appear in tables selected from
/// Performs ORDER BY operations
// [spec:pgorm:sem:query.build.modifiers+11]
pub trait QueryOrder: Sized {
    #[allow(missing_docs)]
    type QueryStatement: OrderedStatement;

    /// Add the query to perform an ORDER BY operation
    fn query(&mut self) -> &mut SelectStatement;

    /// Add an order_by expression
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .order_by(cake::Column::Id, Order::Asc)
    ///         .order_by(cake::Column::Name, Order::Desc)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" ORDER BY "cake"."id" ASC, "cake"."name" DESC"#
    /// );
    /// ```
    fn order_by<C>(mut self, col: C, ord: Order) -> Self
    where
        C: IntoSimpleExpr,
    {
        self.query().order_by_expr(col.into_simple_expr(), ord);
        self
    }

    /// Add an order_by expression (ascending)
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .order_by_asc(cake::Column::Id)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" ORDER BY "cake"."id" ASC"#
    /// );
    /// ```
    fn order_by_asc<C>(mut self, col: C) -> Self
    where
        C: IntoSimpleExpr,
    {
        self.query()
            .order_by_expr(col.into_simple_expr(), Order::Asc);
        self
    }

    /// Add an order_by expression (descending)
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .order_by_desc(cake::Column::Id)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" ORDER BY "cake"."id" DESC"#
    /// );
    /// ```
    fn order_by_desc<C>(mut self, col: C) -> Self
    where
        C: IntoSimpleExpr,
    {
        self.query()
            .order_by_expr(col.into_simple_expr(), Order::Desc);
        self
    }

    /// Add an order_by expression with nulls ordering option
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    /// use pgorm_query::NullOrdering;
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .order_by_with_nulls(cake::Column::Id, Order::Asc, NullOrdering::First)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" ORDER BY "cake"."id" ASC NULLS FIRST"#
    /// );
    /// ```
    fn order_by_with_nulls<C>(mut self, col: C, ord: Order, nulls: NullOrdering) -> Self
    where
        C: IntoSimpleExpr,
    {
        self.query()
            .order_by_expr_with_nulls(col.into_simple_expr(), ord, nulls);
        self
    }
}

// LINT: when the column does not appear in tables selected from
/// Perform a FILTER opertation on a statement
// [spec:pgorm:sem:query.build.filter+2]
pub trait QueryFilter: Sized {
    #[allow(missing_docs)]
    type QueryStatement: ConditionalStatement;

    /// Add the query to perform a FILTER on
    fn query(&mut self) -> &mut Self::QueryStatement;

    /// Add an AND WHERE expression
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Id.eq(4))
    ///         .filter(cake::Column::Id.eq(5))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = 4 AND "cake"."id" = 5"#
    /// );
    /// ```
    ///
    /// Add a condition tree.
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(
    ///             Condition::any()
    ///                 .add(cake::Column::Id.eq(4))
    ///                 .add(cake::Column::Id.eq(5))
    ///         )
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = 4 OR "cake"."id" = 5"#
    /// );
    /// ```
    ///
    /// Like above, but using the `IN` operator.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(cake::Column::Id.is_in([4, 5]))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" IN (4, 5)"#
    /// );
    /// ```
    ///
    /// Like above, but using the `ANY` operator, which binds the whole list as
    /// a single array parameter instead of one placeholder per element.
    ///
    /// ```
    /// use pgorm::pgorm_query::{Expr, ExprTrait, Func};
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(Expr::col((cake::Entity, cake::Column::Id)).eq(Func::any(vec![4, 5])))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = ANY(ARRAY [4,5])"#
    /// );
    /// ```
    ///
    /// Add a runtime-built condition tree.
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    /// struct Input {
    ///     name: Option<String>,
    /// }
    /// let input = Input {
    ///     name: Some("cheese".to_owned()),
    /// };
    ///
    /// let mut conditions = Condition::all();
    /// if let Some(name) = input.name {
    ///     conditions = conditions.add(cake::Column::Name.contains(&name));
    /// }
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(conditions)
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" LIKE '%cheese%'"#
    /// );
    /// ```
    ///
    /// Add a runtime-built condition tree, functional-way.
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    /// struct Input {
    ///     name: Option<String>,
    /// }
    /// let input = Input {
    ///     name: Some("cheese".to_owned()),
    /// };
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(
    ///             Condition::all().add_option(input.name.map(|n| cake::Column::Name.contains(&n)))
    ///         )
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" LIKE '%cheese%'"#
    /// );
    /// ```
    ///
    /// A slightly more complex example.
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake, pgorm_query::{Expr, ExprTrait}};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .filter(
    ///             Condition::all()
    ///                 .add(
    ///                     Condition::all()
    ///                         .not()
    ///                         .add(Expr::val(1).eq(1))
    ///                         .add(Expr::val(2).eq(2))
    ///                 )
    ///                 .add(
    ///                     Condition::any()
    ///                         .add(Expr::val(3).eq(3))
    ///                         .add(Expr::val(4).eq(4))
    ///                 )
    ///         )
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE (NOT (1 = 1 AND 2 = 2)) AND (3 = 3 OR 4 = 4)"#
    /// );
    /// ```
    /// Use a pgorm_query expression
    /// ```
    /// use pgorm::{entity::*, query::*, pgorm_query::{Expr, ExprTrait}, tests_cfg::fruit};
    ///
    /// assert_eq!(
    ///     fruit::Entity::find()
    ///         .filter(Expr::col(fruit::Column::CakeId).is_null())
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "fruit"."id", "fruit"."name", "fruit"."cake_id" FROM "fruit" WHERE "cake_id" IS NULL"#
    /// );
    /// ```
    fn filter<F>(mut self, filter: F) -> Self
    where
        F: IntoCondition,
    {
        self.query().cond_where(filter.into_condition());
        self
    }

    /// Apply a where condition using the model's primary key
    fn belongs_to<M>(mut self, model: &M) -> Self
    where
        M: ModelTrait,
    {
        for key in <M::Entity as EntityTrait>::PrimaryKey::iter() {
            let col = key.into_column();
            self = self.filter(col.eq(model.get(col)));
        }
        self
    }

    /// Perform a check to determine table belongs to a Model through it's name alias
    ///
    /// Each key column is compared as [`belongs_to`](QueryFilter::belongs_to)
    /// compares it, under the alias instead of the entity's own name.
    // [spec:pgorm:sem:entity.traits.column.enum-cast+17]
    fn belongs_to_tbl_alias<M>(mut self, model: &M, tbl_alias: impl IntoName) -> Self
    where
        M: ModelTrait,
    {
        let tbl_alias = tbl_alias.into_name();
        for key in <M::Entity as EntityTrait>::PrimaryKey::iter() {
            let col = key.into_column();
            let column = compared_column(&col, Expr::col((Name::clone(&tbl_alias), col)));
            let expr = column.eq(compared_value(&col, Expr::val(model.get(col))));
            self = self.filter(expr);
        }
        self
    }
}

// [spec:pgorm:sem:query.build.join+5]
pub(crate) fn join_condition(mut rel: RelationDef) -> Condition {
    // Use table alias (if any) to construct the join condition
    let from_tbl = Name::clone(rel.from_tbl.qualifier());
    let to_tbl = Name::clone(rel.to_tbl.qualifier());
    let mut condition = match rel.condition_type {
        ConditionType::All => Condition::all(),
        ConditionType::Any => Condition::any(),
    };

    condition = condition.add(join_tbl_on_condition(
        Name::clone(&from_tbl),
        Name::clone(&to_tbl),
        rel.columns,
    ));
    if let Some(f) = rel.on_condition.take() {
        condition = condition.add(f(from_tbl, to_tbl));
    }

    condition
}

// [spec:pgorm:sem:query.build.join+5]
// [spec:pgorm:sem:entity.traits.column.enum-cast+17]
fn join_tbl_on_condition(from_tbl: Name, to_tbl: Name, columns: ColumnPairs) -> Condition {
    let mut cond = Condition::all();
    for ((owner_key, foreign_key), compared) in columns.compared_pairs() {
        let from = compared_side(
            Expr::col((Name::clone(&from_tbl), Name::clone(owner_key))),
            &compared.from,
        );
        let to = compared_side(
            Expr::col((Name::clone(&to_tbl), Name::clone(foreign_key))),
            &compared.to,
        );
        cond = cond.add(from.eq(to));
    }
    // A temporal relation's periods meet where they overlap: a row is joined
    // to every row whose period shares a moment with its own.
    if let Some((from_period, to_period)) = columns.period().cloned() {
        cond =
            cond.add(Expr::col((from_tbl, from_period)).overlaps(Expr::col((to_tbl, to_period))));
    }
    cond
}

/// One side of a join pair as the equality compares it: cast to the enum when
/// its column's domain is over one, which is the only way PostgreSQL compares
/// such a column with anything.
// [spec:pgorm:sem:entity.traits.column.enum-cast+17]
fn compared_side(column: Expr, enum_type: &Option<TypeName>) -> Expr {
    match enum_type {
        Some(enum_type) => Expr::expr(column.cast_as_type(enum_type.clone())),
        None => column,
    }
}

/// The full table name a [`FromItem`] contributes to a foreign key: schema
/// qualification included, so a `REFERENCES` clause names the table the
/// relation actually points at rather than whatever `search_path` resolves.
// [spec:pgorm:sem:schema.from-entity+12]
pub(crate) fn unpack_table_name(from_item: &FromItem) -> pgorm_query::TableName {
    match from_item.table_name() {
        Some(name) => name.clone(),
        None => pgorm_query::TableName::Table(Name::clone(from_item.qualifier())),
    }
}

/// The identifier a joined table is read under when its bare name is one
/// the statement already qualifies another table's columns by, as
/// `archive.listing` beside `market.listing`, which PostgreSQL would
/// otherwise refuse (`42712`, or `42P09` for a reference to either).
///
/// It is the table's schema-qualified name as one identifier,
/// `"archive.listing"`, and where `taken` holds that too, as it does for an
/// unqualified table's bare name, the same with `_2`, `_3` and on appended;
/// each is bounded to PostgreSQL's 63 bytes as a result name is, so two long
/// names are not truncated into one.
// [spec:pgorm:req:query.graph.aliases+2]
// [spec:pgorm:sem:disjoint.read+2]
pub(crate) fn alias_apart(table: &pgorm_query::TableName, taken: &dyn Fn(&str) -> bool) -> Name {
    let label = match table.schema() {
        Some(schema) => format!("{}.{}", schema.to_string(), table.table().to_string()),
        None => table.table().to_string(),
    };
    let mut alias = bounded_identifier(label.clone());
    let mut suffix = 2;
    while taken(&alias) {
        alias = bounded_identifier(format!("{label}_{suffix}"));
        suffix += 1;
    }
    Name::runtime(alias)
}
