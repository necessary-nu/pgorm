use crate::entity::entity_table;
use crate::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, Error, Iterable, PrimaryKeyToColumn,
    QueryFilter, QueryTrait,
};
use core::marker::PhantomData;
use pgorm_query::{Expr, IntoFromItem, IntoName, SimpleExpr, UpdateStatement};

/// Defines a structure to perform UPDATE query operations on a ActiveModel
#[derive(Clone, Debug)]
pub struct Update;

/// Defines an UPDATE operation on one ActiveModel
#[derive(Clone, Debug)]
pub struct UpdateOne<A>
where
    A: ActiveModelTrait,
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
    E: EntityTrait,
{
    pub(crate) query: UpdateStatement,
    pub(crate) entity: PhantomData<E>,
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
        E: EntityTrait,
        A: ActiveModelTrait<Entity = E>,
    {
        Self::one_in(E::default(), model)
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
    // [spec:pgorm:sem:query.build.update+5]
    pub fn one_in<E, A>(entity: E, model: A) -> Result<UpdateOne<A>, Error>
    where
        E: EntityTrait,
        A: ActiveModelTrait<Entity = E>,
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
    /// use pgorm::{entity::*, query::*, pgorm_query::{Expr, ExprTrait}, tests_cfg::fruit};
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
        E: EntityTrait,
    {
        UpdateMany {
            query: UpdateStatement::new()
                .table(entity_table(&entity))
                .to_owned(),
            entity: PhantomData,
        }
    }
}

// [spec:pgorm:sem:query.build.update+5]
impl<A> UpdateOne<A>
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

    fn prepare_values(mut self) -> Result<Self, Error> {
        for col in <A::Entity as EntityTrait>::Column::iter() {
            if <A::Entity as EntityTrait>::PrimaryKey::from_column(col).is_some() {
                continue;
            }
            match self.model.get(col)? {
                ActiveValue::Set(value) => {
                    let expr = col.save_as(Expr::val(value));
                    self.query.value(col, expr);
                }
                ActiveValue::Unchanged(_) | ActiveValue::NotSet => {}
            }
        }
        Ok(self)
    }
}

impl<A> QueryFilter for UpdateOne<A>
where
    A: ActiveModelTrait,
{
    type QueryStatement = UpdateStatement;

    fn query(&mut self) -> &mut UpdateStatement {
        &mut self.query
    }
}

impl<E> QueryFilter for UpdateMany<E>
where
    E: EntityTrait,
{
    type QueryStatement = UpdateStatement;

    fn query(&mut self) -> &mut UpdateStatement {
        &mut self.query
    }
}

impl<A> QueryTrait for UpdateOne<A>
where
    A: ActiveModelTrait,
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

impl<E> QueryTrait for UpdateMany<E>
where
    E: EntityTrait,
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

// [spec:pgorm:sem:query.build.update+5]
impl<E> UpdateMany<E>
where
    E: EntityTrait,
{
    /// Add the models to update to Self
    ///
    /// Only `Set` columns are written: `Unchanged` and `NotSet` ones contribute
    /// nothing, and so does a column this model does not carry at all — the
    /// `Error::Type` [`ActiveModelTrait::get`] reports for one is a statement
    /// about the model, not a value to write.
    pub fn set<A>(mut self, model: A) -> Self
    where
        A: ActiveModelTrait<Entity = E>,
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
    // [spec:pgorm:sem:query.build.update+5]
    pub fn from<R>(mut self, tbl_ref: R) -> Self
    where
        R: IntoFromItem,
    {
        self.query.from(tbl_ref);
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
