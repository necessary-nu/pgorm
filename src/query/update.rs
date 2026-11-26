use crate::entity::entity_table;
use crate::entity::{hold_statement, immutable_column, immutable_set, scope_condition};
use crate::{
    ActiveModel, ActiveValue, Column, Entity, Error, Iterable, NoContext, PrimaryKeyToColumn,
    QueryFilter, ScopeContext, ScopeError, ScopedIn, Statement, StaticName, UnscopedFromItem,
};
use core::marker::PhantomData;
use pgorm_query::{Expr, IntoName, SimpleExpr, UpdateStatement};

/// Defines a structure to perform UPDATE query operations on a ActiveModel
#[derive(Clone, Debug)]
pub struct Update;

/// Defines an UPDATE operation on one ActiveModel
#[derive(Clone, Debug)]
pub struct UpdateOne<A>
where
    A: ActiveModel,
{
    pub(crate) query: UpdateStatement,
    pub(crate) model: A,
    /// The entity whose table the update writes, which an update with
    /// nothing to set reads its row back from.
    pub(crate) entity: A::Entity,
}

/// Defines an UPDATE operation on multiple ActiveModels
#[derive(Clone, Debug)]
pub struct UpdateMany<E>
where
    E: Entity,
{
    pub(crate) query: UpdateStatement,
    pub(crate) entity: PhantomData<E>,
    /// What a context that does not hold a scope of `E` was refused with,
    /// held until the statement would run.
    pub(crate) refused: Option<ScopeError>,
}

impl Update {
    /// Update one ActiveModel
    ///
    /// Fails with [`Error::PrimaryKeyNotSet`] when a primary-key column of
    /// `model` is [`ActiveValue::NotSet`], since there would be nothing to
    /// narrow the statement to a single row.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     Update::one(cake::ActiveModel {
    ///         id: set(1),
    ///         name: set("Apple Pie"),
    ///     })
    ///     .expect("the primary key is set")
    ///     .as_query()
    ///     .to_string(),
    ///     r#"UPDATE "cake" SET "name" = 'Apple Pie' WHERE "cake"."id" = 1"#,
    /// );
    /// ```
    ///
    /// ```
    /// use pgorm::{entity::*, error::Error, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     Update::one(cake::ActiveModel {
    ///         id: ActiveValue::not_set(),
    ///         name: set("Apple Pie"),
    ///     })
    ///     .unwrap_err(),
    ///     Error::PrimaryKeyNotSet,
    /// );
    /// ```
    pub fn one<E, A>(model: A) -> Result<UpdateOne<A>, Error>
    where
        E: Entity + ScopedIn<NoContext>,
        A: ActiveModel<Entity = E>,
    {
        Self::one_in(E::default(), model)
    }

    /// [`one`](Update::one) under the context `cx`: the row is found by its
    /// key and `cx`'s scopes side by side, so a key of a row outside them
    /// updates nothing, and [`exec_returning_model`](UpdateOne::exec_returning_model)
    /// reports [`Error::RecordNotFound`]. A scope column is immutable, so a
    /// model setting one is refused as [`Error::ImmutableColumn`].
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::scoped::{note, CompanyId, Member, UserId}};
    ///
    /// let member = Member { company: CompanyId(7), user: UserId(3) };
    /// let update = Update::one_for(
    ///     &member,
    ///     note::ActiveModel {
    ///         id: set(1),
    ///         body: set("edited"),
    ///         ..Default::default()
    ///     },
    /// )
    /// .expect("the key is set and no scope column is");
    /// assert_eq!(
    ///     update.as_query().to_string(),
    ///     [
    ///         r#"UPDATE "note" SET "body" = 'edited' WHERE "note"."id" = 1"#,
    ///         r#"AND "note"."company_id" = 7 AND "note"."author_id" = 3"#,
    ///     ]
    ///     .join(" "),
    /// );
    /// ```
    // [spec:pgorm:sem:query.build.update+6]
    // [spec:pgorm:sem:scope.write]
    pub fn one_for<Cx, E, A>(cx: &Cx, model: A) -> Result<UpdateOne<A>, Error>
    where
        Cx: ScopeContext,
        E: Entity + ScopedIn<Cx>,
        A: ActiveModel<Entity = E>,
    {
        let mut one = Self::one_unscoped(E::default(), model)?;
        if let Some(condition) = scope_condition::<E>(cx)? {
            one.query.cond_where(condition);
        }
        Ok(one)
    }

    /// [`one`](Update::one) of a row of the table `entity` names: the
    /// instance's schema and table rather than [`Default`]'s, for an entity
    /// whose names are computed at run time.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::tenant_cake};
    ///
    /// let acme = tenant_cake::Entity { tenant: "acme" };
    /// assert_eq!(
    ///     Update::one_in(
    ///         acme,
    ///         tenant_cake::ActiveModel {
    ///             id: set(1),
    ///             name: set("Apple Pie"),
    ///         },
    ///     )
    ///     .expect("the primary key is set")
    ///     .as_query()
    ///     .to_string(),
    ///     r#"UPDATE "acme"."cake" SET "name" = 'Apple Pie' WHERE "cake"."id" = 1"#,
    /// );
    /// ```
    // [spec:pgorm:sem:query.build.update+6]
    pub fn one_in<E, A>(entity: E, model: A) -> Result<UpdateOne<A>, Error>
    where
        E: Entity + ScopedIn<NoContext>,
        A: ActiveModel<Entity = E>,
    {
        Self::one_unscoped(entity, model)
    }

    /// The update of `model`'s row of the table `entity` names, by its key
    /// alone.
    // [spec:pgorm:sem:query.build.update+6]
    fn one_unscoped<E, A>(entity: E, model: A) -> Result<UpdateOne<A>, Error>
    where
        E: Entity,
        A: ActiveModel<Entity = E>,
    {
        let one = UpdateOne {
            query: UpdateStatement::new()
                .table(entity_table(&entity))
                .to_owned(),
            model,
            entity,
        };
        one.prepare_filters()?.prepare_values()
    }

    /// Update many ActiveModel
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, pgorm_query::{Expr, Expression}, tests_cfg::fruit};
    ///
    /// assert_eq!(
    ///     Update::many(fruit::Entity)
    ///         .col_expr(fruit::Column::Name, Expr::value("Golden Apple"))
    ///         .filter(fruit::Column::Name.contains("Apple"))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"UPDATE "fruit" SET "name" = 'Golden Apple' WHERE "fruit"."name" LIKE '%Apple%'"#,
    /// );
    /// ```
    #[doc(alias = "update_many")]
    pub fn many<E>(entity: E) -> UpdateMany<E>
    where
        E: Entity + ScopedIn<NoContext>,
    {
        Self::many_unscoped(&entity)
    }

    /// [`many`](Update::many) under the context `cx`: `cx`'s scopes are
    /// ANDed into the `WHERE`, so only rows they hold are updated.
    // [spec:pgorm:sem:query.build.update+6]
    // [spec:pgorm:sem:scope.write]
    pub fn many_for<Cx, E>(cx: &Cx, entity: E) -> UpdateMany<E>
    where
        Cx: ScopeContext,
        E: Entity + ScopedIn<Cx>,
    {
        let mut many = Self::many_unscoped(&entity);
        hold_statement::<E>(cx, &mut many.query, &mut many.refused);
        many
    }

    fn many_unscoped<E>(entity: &E) -> UpdateMany<E>
    where
        E: Entity,
    {
        UpdateMany {
            query: UpdateStatement::new()
                .table(entity_table(entity))
                .to_owned(),
            entity: PhantomData,
            refused: None,
        }
    }
}

impl<A> UpdateOne<A>
where
    A: ActiveModel,
{
    fn prepare_filters(mut self) -> Result<Self, Error> {
        for key in <A::Entity as Entity>::PrimaryKey::iter() {
            let col = key.into_column();
            match self.model.get(col)? {
                ActiveValue::Set(value) | ActiveValue::Unchanged(value) => {
                    self = self.filter(col.eq(value));
                }
                ActiveValue::NotSet => return Err(Error::PrimaryKeyNotSet),
            }
        }
        Ok(self)
    }

    // [spec:pgorm:req:scope.immutable]
    fn prepare_values(mut self) -> Result<Self, Error> {
        for col in <A::Entity as Entity>::Column::iter() {
            if <A::Entity as Entity>::PrimaryKey::from_column(col).is_some() {
                continue;
            }
            match self.model.get(col)? {
                ActiveValue::Set(value) => {
                    immutable_column::<A::Entity>(col.as_str())?;
                    let expr = col.save_as(Expr::val(value));
                    self.query.value(col, expr);
                }
                ActiveValue::Unchanged(_) | ActiveValue::NotSet => {}
            }
        }
        Ok(self)
    }

    /// What every execution path asks before sending anything: a `SET` of a
    /// column its definition marks immutable.
    // [spec:pgorm:req:scope.immutable]
    pub(crate) fn refusal(&self) -> Result<(), Error> {
        immutable_set::<A::Entity>(&self.query)
    }
}

impl<A> QueryFilter for UpdateOne<A>
where
    A: ActiveModel,
{
    type QueryStatement = UpdateStatement;

    fn query(&mut self) -> &mut UpdateStatement {
        &mut self.query
    }
}

impl<E> QueryFilter for UpdateMany<E>
where
    E: Entity,
{
    type QueryStatement = UpdateStatement;

    fn query(&mut self) -> &mut UpdateStatement {
        &mut self.query
    }
}

impl<A> Statement for UpdateOne<A>
where
    A: ActiveModel,
{
    type QueryStatement = UpdateStatement;

    fn query(&mut self) -> &mut UpdateStatement {
        &mut self.query
    }

    fn as_query(&self) -> &UpdateStatement {
        &self.query
    }

    fn into_query(self) -> UpdateStatement {
        self.query
    }
}

impl<E> Statement for UpdateMany<E>
where
    E: Entity,
{
    type QueryStatement = UpdateStatement;

    fn query(&mut self) -> &mut UpdateStatement {
        &mut self.query
    }

    fn as_query(&self) -> &UpdateStatement {
        &self.query
    }

    fn into_query(self) -> UpdateStatement {
        self.query
    }
}

// [spec:pgorm:sem:query.build.update+6]
impl<E> UpdateMany<E>
where
    E: Entity,
{
    /// What every execution path asks before sending anything: a context
    /// refused for its scopes, and a `SET` of a column its definition marks
    /// immutable, whether [`set`](Self::set) or
    /// [`col_expr`](Self::col_expr) named it.
    // [spec:pgorm:req:scope.refusals]
    // [spec:pgorm:req:scope.immutable]
    pub(crate) fn refusal(&self) -> Result<(), Error> {
        if let Some(refused) = &self.refused {
            return Err(Error::Scope(refused.clone()));
        }
        immutable_set::<E>(&self.query)
    }

    /// Add the models to update to Self
    ///
    /// Only `Set` columns are written: `Unchanged` and `NotSet` ones contribute
    /// nothing, and so does a column this model does not carry at all — the
    /// `Error::Type` [`ActiveModel::get`] reports for one is a statement
    /// about the model, not a value to write.
    pub fn set<A>(mut self, model: A) -> Self
    where
        A: ActiveModel<Entity = E>,
    {
        for col in E::Column::iter() {
            if let Ok(ActiveValue::Set(value)) = model.get(col) {
                let expr = col.save_as(Expr::val(value));
                self.query.value(col, expr);
            }
        }
        self
    }

    /// Creates a [SimpleExpr] from a column
    pub fn col_expr<T>(mut self, col: T, expr: SimpleExpr) -> Self
    where
        T: IntoName,
    {
        self.query.value(col, expr);
        self
    }

    /// Add a relation to the statement's `FROM` clause, so `col_expr` and the
    /// `QueryFilter` predicates can read another table's columns.
    ///
    /// The join condition goes in the filter, which is PostgreSQL's own
    /// spelling of `UPDATE .. FROM`; calling it repeatedly accumulates a
    /// comma-separated relation list.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::{cake, fruit}};
    /// use pgorm_query::Expr;
    ///
    /// assert_eq!(
    ///     Update::many(fruit::Entity)
    ///         .col_expr(
    ///             fruit::Column::Name,
    ///             Expr::col((cake::Entity, cake::Column::Name)).into(),
    ///         )
    ///         .from(cake::Entity)
    ///         .filter(fruit::Column::CakeId.eq_col(cake::Column::Id))
    ///         .as_query()
    ///         .to_string(),
    ///     [
    ///         r#"UPDATE "fruit" SET "name" = "cake"."name" FROM "cake""#,
    ///         r#"WHERE "fruit"."cake_id" = "cake"."id""#,
    ///     ]
    ///     .join(" "),
    /// );
    /// ```
    ///
    /// A scoped entity is added by [`from_for`](Self::from_for), so naming
    /// one here does not compile.
    // [spec:pgorm:sem:query.build.update+6]
    // [spec:pgorm:sem:scope.from-using]
    pub fn from<R>(mut self, tbl_ref: R) -> Self
    where
        R: UnscopedFromItem,
    {
        self.query.from(tbl_ref);
        self
    }

    /// [`from`](Self::from) of the entity `entity` under the context `cx`:
    /// its table is added to the `FROM` list, under its `Default` name as the
    /// `_for` statement forms read theirs, and `cx`'s scopes on its rows are
    /// ANDed into the `WHERE`, so the update reads no row of it outside them.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, pgorm_query::Expr};
    /// use pgorm::tests_cfg::scoped::{company, note, CompanyId, Member, UserId};
    ///
    /// let member = Member { company: CompanyId(7), user: UserId(3) };
    /// assert_eq!(
    ///     Update::many_for(&member, company::Entity)
    ///         .col_expr(company::Column::Name, Expr::col((note::Entity, note::Column::Body)).into())
    ///         .from_for(&member, note::Entity)
    ///         .filter(company::Column::Id.eq_col(note::Column::CompanyId))
    ///         .as_query()
    ///         .to_string(),
    ///     [
    ///         r#"UPDATE "company" SET "name" = "note"."body" FROM "note""#,
    ///         r#"WHERE "company"."id" = 7 AND "note"."company_id" = 7 AND "note"."author_id" = 3"#,
    ///         r#"AND "company"."id" = "note"."company_id""#,
    ///     ]
    ///     .join(" "),
    /// );
    /// ```
    // [spec:pgorm:sem:scope.from-using]
    pub fn from_for<Cx, R>(mut self, cx: &Cx, entity: R) -> Self
    where
        Cx: ScopeContext,
        R: ScopedIn<Cx>,
    {
        self.query.from(entity_table(&entity));
        hold_statement::<R>(cx, &mut self.query, &mut self.refused);
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::ActiveValue::Unchanged;
    use crate::tests_cfg::{active_enums::Tea, cake, fruit, lunch_set};
    use crate::{entity::*, query::*};
    use pgorm_query::{Expr, Value};

    #[test]
    fn update_1() {
        assert_eq!(
            Update::one(cake::ActiveModel {
                id: set(1),
                name: set("Apple Pie"),
            })
            .expect("the primary key is set")
            .as_query()
            .to_string(),
            r#"UPDATE "cake" SET "name" = 'Apple Pie' WHERE "cake"."id" = 1"#,
        );
    }

    #[test]
    fn update_2() {
        assert_eq!(
            Update::one(fruit::ActiveModel {
                id: set(1),
                name: set("Orange"),
                cake_id: ActiveValue::not_set(),
            })
            .expect("the primary key is set")
            .as_query()
            .to_string(),
            r#"UPDATE "fruit" SET "name" = 'Orange' WHERE "fruit"."id" = 1"#,
        );
    }

    #[test]
    fn update_3() {
        assert_eq!(
            Update::one(fruit::ActiveModel {
                id: set(2),
                name: ActiveValue::unchanged("Apple".to_owned()),
                cake_id: set(Some(3)),
            })
            .expect("the primary key is set")
            .as_query()
            .to_string(),
            r#"UPDATE "fruit" SET "cake_id" = 3 WHERE "fruit"."id" = 2"#,
        );
    }

    #[test]
    fn update_4() {
        assert_eq!(
            Update::many(fruit::Entity)
                .col_expr(fruit::Column::CakeId, Expr::value(Value::Int(None)))
                .filter(fruit::Column::Id.eq(2))
                .as_query()
                .to_string(),
            r#"UPDATE "fruit" SET "cake_id" = NULL WHERE "fruit"."id" = 2"#,
        );
    }

    #[test]
    fn update_5() {
        assert_eq!(
            Update::many(fruit::Entity)
                .set(fruit::ActiveModel {
                    name: set("Apple"),
                    cake_id: set(Some(3)),
                    ..Default::default()
                })
                .filter(fruit::Column::Id.eq(2))
                .as_query()
                .to_string(),
            r#"UPDATE "fruit" SET "name" = 'Apple', "cake_id" = 3 WHERE "fruit"."id" = 2"#,
        );
    }

    #[test]
    fn update_6() {
        assert_eq!(
            Update::many(fruit::Entity)
                .set(fruit::ActiveModel {
                    id: set(3),
                    ..Default::default()
                })
                .filter(fruit::Column::Id.eq(2))
                .as_query()
                .to_string(),
            r#"UPDATE "fruit" SET "id" = 3 WHERE "fruit"."id" = 2"#,
        );
    }

    #[test]
    fn update_7() {
        assert_eq!(
            Update::many(lunch_set::Entity)
                .set(lunch_set::ActiveModel {
                    tea: set(Tea::EverydayTea),
                    ..Default::default()
                })
                .filter(lunch_set::Column::Tea.eq(Tea::BreakfastTea))
                .as_query()
                .to_string(),
            r#"UPDATE "lunch_set" SET "tea" = CAST('EverydayTea' AS tea) WHERE "lunch_set"."tea" = CAST('BreakfastTea' AS tea)"#,
        );
    }

    #[test]
    fn update_8() {
        assert_eq!(
            Update::one(lunch_set::ActiveModel {
                id: Unchanged(1),
                tea: set(Tea::EverydayTea),
                ..Default::default()
            })
            .expect("the primary key is set")
            .as_query()
            .to_string(),
            r#"UPDATE "lunch_set" SET "tea" = CAST('EverydayTea' AS tea) WHERE "lunch_set"."id" = 1"#,
        );
    }
}
