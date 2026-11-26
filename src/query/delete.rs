use crate::entity::entity_table;
use crate::entity::{hold_statement, scope_condition};
use crate::{
    ActiveModel, ActiveValue, Column, Entity, Error, IntoActiveModel, Iterable, NoContext,
    PrimaryKeyToColumn, QueryFilter, ScopeContext, ScopeError, ScopedIn, Statement,
    UnscopedFromItem,
};
use core::marker::PhantomData;
use pgorm_query::DeleteStatement;

/// Defines the structure for a delete operation
#[derive(Clone, Debug)]
pub struct Delete;

/// Perform a delete operation on a model
#[derive(Clone, Debug)]
pub struct DeleteOne<A>
where
    A: ActiveModel,
{
    pub(crate) query: DeleteStatement,
    pub(crate) model: A,
}

/// Perform a delete operation on multiple models
#[derive(Clone, Debug)]
pub struct DeleteMany<E>
where
    E: Entity,
{
    pub(crate) query: DeleteStatement,
    pub(crate) entity: PhantomData<E>,
    /// What a context that does not hold a scope of `E` was refused with,
    /// held until the statement would run.
    pub(crate) refused: Option<ScopeError>,
}

// [spec:pgorm:sem:query.build.delete+5]
impl Delete {
    /// Delete one Model or ActiveModel
    ///
    /// Fails with [`Error::PrimaryKeyNotSet`] when a primary-key column of
    /// `model` is [`ActiveValue::NotSet`], since there would be nothing to
    /// narrow the statement to a single row.
    ///
    /// Model
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     Delete::one(cake::Model {
    ///         id: 1,
    ///         name: "Apple Pie".to_owned(),
    ///     })
    ///     .expect("the primary key is set")
    ///     .as_query()
    ///     .to_string(),
    ///     r#"DELETE FROM "cake" WHERE "cake"."id" = 1"#,
    /// );
    /// ```
    /// ActiveModel
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     Delete::one(cake::ActiveModel {
    ///         id: set(1),
    ///         name: set("Apple Pie"),
    ///     })
    ///     .expect("the primary key is set")
    ///     .as_query()
    ///     .to_string(),
    ///     r#"DELETE FROM "cake" WHERE "cake"."id" = 1"#,
    /// );
    /// ```
    ///
    /// ```
    /// use pgorm::{entity::*, error::Error, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     Delete::one(cake::ActiveModel {
    ///         id: ActiveValue::not_set(),
    ///         name: set("Apple Pie"),
    ///     })
    ///     .unwrap_err(),
    ///     Error::PrimaryKeyNotSet,
    /// );
    /// ```
    pub fn one<E, A, M>(model: M) -> Result<DeleteOne<A>, Error>
    where
        E: Entity + ScopedIn<NoContext>,
        A: ActiveModel<Entity = E>,
        M: IntoActiveModel<A>,
    {
        Self::one_in(E::default(), model)
    }

    /// [`one`](Delete::one) under the context `cx`: the row is found by its
    /// key and `cx`'s scopes side by side, so a key of a row outside them
    /// deletes nothing.
    // [spec:pgorm:sem:query.build.delete+5]
    // [spec:pgorm:sem:scope.write]
    pub fn one_for<Cx, E, A, M>(cx: &Cx, model: M) -> Result<DeleteOne<A>, Error>
    where
        Cx: ScopeContext,
        E: Entity + ScopedIn<Cx>,
        A: ActiveModel<Entity = E>,
        M: IntoActiveModel<A>,
    {
        let mut one = Self::one_unscoped(E::default(), model)?;
        if let Some(condition) = scope_condition::<E>(cx)? {
            one.query.cond_where(condition);
        }
        Ok(one)
    }

    /// [`one`](Delete::one) from the table `entity` names: the instance's
    /// schema and table rather than [`Default`]'s, for an entity whose names
    /// are computed at run time.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::tenant_cake};
    ///
    /// let acme = tenant_cake::Entity { tenant: "acme" };
    /// assert_eq!(
    ///     Delete::one_in(
    ///         acme,
    ///         tenant_cake::ActiveModel {
    ///             id: set(1),
    ///             ..Default::default()
    ///         },
    ///     )
    ///     .expect("the primary key is set")
    ///     .as_query()
    ///     .to_string(),
    ///     r#"DELETE FROM "acme"."cake" WHERE "cake"."id" = 1"#,
    /// );
    /// ```
    // [spec:pgorm:sem:query.build.delete+5]
    pub fn one_in<E, A, M>(entity: E, model: M) -> Result<DeleteOne<A>, Error>
    where
        E: Entity + ScopedIn<NoContext>,
        A: ActiveModel<Entity = E>,
        M: IntoActiveModel<A>,
    {
        Self::one_unscoped(entity, model)
    }

    /// The delete of `model`'s row of the table `entity` names, by its key
    /// alone.
    // [spec:pgorm:sem:query.build.delete+5]
    fn one_unscoped<E, A, M>(entity: E, model: M) -> Result<DeleteOne<A>, Error>
    where
        E: Entity,
        A: ActiveModel<Entity = E>,
        M: IntoActiveModel<A>,
    {
        let myself = DeleteOne {
            query: DeleteStatement::new()
                .from_table(entity_table(&entity))
                .to_owned(),
            model: model.into_active_model(),
        };
        myself.prepare_filters()
    }

    /// Delete many ActiveModel
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::fruit};
    ///
    /// assert_eq!(
    ///     Delete::many(fruit::Entity)
    ///         .filter(fruit::Column::Name.contains("Apple"))
    ///         .as_query()
    ///         .to_string(),
    ///     r#"DELETE FROM "fruit" WHERE "fruit"."name" LIKE '%Apple%'"#,
    /// );
    /// ```
    #[doc(alias = "delete_many")]
    pub fn many<E>(entity: E) -> DeleteMany<E>
    where
        E: Entity + ScopedIn<NoContext>,
    {
        Self::many_unscoped(&entity)
    }

    /// [`many`](Delete::many) under the context `cx`: `cx`'s scopes are
    /// ANDed into the `WHERE`, so only rows they hold are deleted.
    // [spec:pgorm:sem:query.build.delete+5]
    // [spec:pgorm:sem:scope.write]
    pub fn many_for<Cx, E>(cx: &Cx, entity: E) -> DeleteMany<E>
    where
        Cx: ScopeContext,
        E: Entity + ScopedIn<Cx>,
    {
        let mut many = Self::many_unscoped(&entity);
        hold_statement::<E>(cx, &mut many.query, &mut many.refused);
        many
    }

    fn many_unscoped<E>(entity: &E) -> DeleteMany<E>
    where
        E: Entity,
    {
        DeleteMany {
            query: DeleteStatement::new()
                .from_table(entity_table(entity))
                .to_owned(),
            entity: PhantomData,
            refused: None,
        }
    }
}

// [spec:pgorm:sem:query.build.delete+5]
impl<A> DeleteOne<A>
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
}

impl<A> QueryFilter for DeleteOne<A>
where
    A: ActiveModel,
{
    type QueryStatement = DeleteStatement;

    fn query(&mut self) -> &mut DeleteStatement {
        &mut self.query
    }
}

impl<E> QueryFilter for DeleteMany<E>
where
    E: Entity,
{
    type QueryStatement = DeleteStatement;

    fn query(&mut self) -> &mut DeleteStatement {
        &mut self.query
    }
}

impl<A> Statement for DeleteOne<A>
where
    A: ActiveModel,
{
    type QueryStatement = DeleteStatement;

    fn query(&mut self) -> &mut DeleteStatement {
        &mut self.query
    }

    fn as_query(&self) -> &DeleteStatement {
        &self.query
    }

    fn into_query(self) -> DeleteStatement {
        self.query
    }
}

impl<E> Statement for DeleteMany<E>
where
    E: Entity,
{
    type QueryStatement = DeleteStatement;

    fn query(&mut self) -> &mut DeleteStatement {
        &mut self.query
    }

    fn as_query(&self) -> &DeleteStatement {
        &self.query
    }

    fn into_query(self) -> DeleteStatement {
        self.query
    }
}

// [spec:pgorm:sem:query.build.delete+5]
impl<E> DeleteMany<E>
where
    E: Entity,
{
    /// What every execution path asks before sending anything: a context
    /// refused for its scopes.
    // [spec:pgorm:req:scope.refusals]
    pub(crate) fn refusal(&self) -> Result<(), Error> {
        match &self.refused {
            Some(refused) => Err(Error::Scope(refused.clone())),
            None => Ok(()),
        }
    }

    /// Add a relation to the statement's `USING` clause, so the `QueryFilter`
    /// predicates can read another table's columns.
    ///
    /// `USING` is DELETE's spelling of UPDATE's `FROM`: the join condition
    /// goes in the filter, and calling it repeatedly accumulates a
    /// comma-separated relation list.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::{cake, fruit}};
    ///
    /// assert_eq!(
    ///     Delete::many(fruit::Entity)
    ///         .using(cake::Entity)
    ///         .filter(fruit::Column::CakeId.eq_col(cake::Column::Id))
    ///         .filter(cake::Column::Name.eq("Apple Pie"))
    ///         .as_query()
    ///         .to_string(),
    ///     [
    ///         r#"DELETE FROM "fruit" USING "cake""#,
    ///         r#"WHERE "fruit"."cake_id" = "cake"."id" AND "cake"."name" = 'Apple Pie'"#,
    ///     ]
    ///     .join(" "),
    /// );
    /// ```
    ///
    /// A scoped entity is added by [`using_for`](Self::using_for), so naming
    /// one here does not compile.
    // [spec:pgorm:sem:scope.from-using]
    pub fn using<R>(mut self, tbl_ref: R) -> Self
    where
        R: UnscopedFromItem,
    {
        self.query.using(tbl_ref);
        self
    }

    /// [`using`](Self::using) of the entity `entity` under the context `cx`:
    /// its table is added to the `USING` list, under its `Default` name as the
    /// `_for` statement forms read theirs, and `cx`'s scopes on its rows are
    /// ANDed into the `WHERE`, so the delete reads no row of it outside them.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*};
    /// use pgorm::tests_cfg::scoped::{company, note, CompanyAdmin, CompanyId, Member, UserId};
    ///
    /// let admin = CompanyAdmin { company: CompanyId(7), users: Every::new() };
    /// let member = Member { company: CompanyId(7), user: UserId(3) };
    /// assert_eq!(
    ///     Delete::many_for(&admin, note::Entity)
    ///         .using_for(&member, company::Entity)
    ///         .filter(note::Column::CompanyId.eq_col(company::Column::Id))
    ///         .as_query()
    ///         .to_string(),
    ///     [
    ///         r#"DELETE FROM "note" USING "company""#,
    ///         r#"WHERE "note"."company_id" = 7 AND "company"."id" = 7"#,
    ///         r#"AND "note"."company_id" = "company"."id""#,
    ///     ]
    ///     .join(" "),
    /// );
    /// ```
    // [spec:pgorm:sem:scope.from-using]
    pub fn using_for<Cx, R>(mut self, cx: &Cx, entity: R) -> Self
    where
        Cx: ScopeContext,
        R: ScopedIn<Cx>,
    {
        self.query.using(entity_table(&entity));
        hold_statement::<R>(cx, &mut self.query, &mut self.refused);
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_cfg::{cake, fruit};
    use crate::{entity::*, query::*};

    #[test]
    fn delete_1() {
        assert_eq!(
            Delete::one(cake::Model {
                id: 1,
                name: "Apple Pie".to_owned(),
            })
            .expect("the primary key is set")
            .as_query()
            .to_string(),
            r#"DELETE FROM "cake" WHERE "cake"."id" = 1"#,
        );
        assert_eq!(
            Delete::one(cake::ActiveModel {
                id: set(1),
                name: set("Apple Pie"),
            })
            .expect("the primary key is set")
            .as_query()
            .to_string(),
            r#"DELETE FROM "cake" WHERE "cake"."id" = 1"#,
        );
    }

    #[test]
    fn delete_2() {
        assert_eq!(
            Delete::many(fruit::Entity)
                .filter(fruit::Column::Name.contains("Cheese"))
                .as_query()
                .to_string(),
            r#"DELETE FROM "fruit" WHERE "fruit"."name" LIKE '%Cheese%'"#,
        );
    }
}
