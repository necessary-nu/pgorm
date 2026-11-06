use crate::entity::entity_table;
use crate::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, Error, IntoActiveModel, Iterable,
    PrimaryKeyToColumn, QueryFilter, QueryTrait,
};
use core::marker::PhantomData;
use pgorm_query::{DeleteStatement, IntoFromItem};

/// Defines the structure for a delete operation
#[derive(Clone, Debug)]
pub struct Delete;

/// Perform a delete operation on a model
#[derive(Clone, Debug)]
pub struct DeleteOne<A>
where
    A: ActiveModelTrait,
{
    pub(crate) query: DeleteStatement,
    pub(crate) model: A,
}

/// Perform a delete operation on multiple models
#[derive(Clone, Debug)]
pub struct DeleteMany<E>
where
    E: EntityTrait,
{
    pub(crate) query: DeleteStatement,
    pub(crate) entity: PhantomData<E>,
}

// [spec:pgorm:sem:query.build.delete+4]
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
        E: EntityTrait,
        A: ActiveModelTrait<Entity = E>,
        M: IntoActiveModel<A>,
    {
        Self::one_in(E::default(), model)
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
    // [spec:pgorm:sem:query.build.delete+4]
    pub fn one_in<E, A, M>(entity: E, model: M) -> Result<DeleteOne<A>, Error>
    where
        E: EntityTrait,
        A: ActiveModelTrait<Entity = E>,
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
        E: EntityTrait,
    {
        DeleteMany {
            query: DeleteStatement::new()
                .from_table(entity_table(&entity))
                .to_owned(),
            entity: PhantomData,
        }
    }
}

// [spec:pgorm:sem:query.build.delete+4]
impl<A> DeleteOne<A>
where
    A: ActiveModelTrait,
{
    fn prepare_filters(mut self) -> Result<Self, Error> {
        for key in <A::Entity as EntityTrait>::PrimaryKey::iter() {
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
    A: ActiveModelTrait,
{
    type QueryStatement = DeleteStatement;

    fn query(&mut self) -> &mut DeleteStatement {
        &mut self.query
    }
}

impl<E> QueryFilter for DeleteMany<E>
where
    E: EntityTrait,
{
    type QueryStatement = DeleteStatement;

    fn query(&mut self) -> &mut DeleteStatement {
        &mut self.query
    }
}

impl<A> QueryTrait for DeleteOne<A>
where
    A: ActiveModelTrait,
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

impl<E> QueryTrait for DeleteMany<E>
where
    E: EntityTrait,
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

// [spec:pgorm:sem:query.build.delete+4]
impl<E> DeleteMany<E>
where
    E: EntityTrait,
{
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
    pub fn using<R>(mut self, tbl_ref: R) -> Self
    where
        R: IntoFromItem,
    {
        self.query.using(tbl_ref);
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
