use crate::{
    ActiveModelBehavior, Column, Delete, DeleteMany, EntityScopes, FromQueryResult, IntoPrimaryKey,
    Model, NoColumns, NoContext, PrimaryKey, PrimaryKeyToColumn, QueryFilter, Related, Relation,
    RelationBuilder, RelationType, ScopeContext, ScopedIn, Select, SelectGraph, Unsliced,
};
use pgorm_query::{
    IntoName, IntoNamedTable, IntoTableName, IntoValueTuple, Name, NamedTable, TableName,
};
pub use strum::IntoEnumIterator as Iterable;

/// The base identifier contract of the entity layer: entities, columns and
/// primary keys all implement it, and it is what [`IntoKey`](crate::IntoKey)
/// keys on, so implementing it is what lets a type stand in a column position.
///
/// It is [`pgorm_query::StaticName`] itself — the entity layer used to declare
/// a second trait of its own with the same method, which meant two traits with
/// the same shape could be in scope at once and neither call site could say
/// which it meant.
// [spec:pgorm:def:entity.traits+5]
// [spec:pgorm:sem:query.build.alias+2]    AliasName is a StaticName in pgorm-query, so a
// query-introduced name reaches the key positions that key on it
pub use pgorm_query::StaticName;

// `StaticName`'s supertrait, here so that the crate root reaches both traits
// through its glob of `entity`. Each import also brings pgorm-query's derive
// of its name, which `entity` shadows with pgorm-macros'.
pub use pgorm_query::SqlName;

/// A Trait for mapping an Entity to a database table
// [spec:pgorm:req:entity.traits.entity-name+7]
pub trait EntityName: StaticName + Default {
    /// Method to get the name for the schema, defaults to [Option::None] if not set
    fn schema_name(&self) -> Option<&str> {
        None
    }

    /// Method to get the comment for the schema, defaults to [Option::None] if not set
    fn comment(&self) -> Option<&str> {
        None
    }

    /// Get the name of the table
    fn table_name(&self) -> &str;

    /// Get the name of the module from the invoking `self.table_name()`
    fn module_name(&self) -> &str {
        self.table_name()
    }

    /// Get the [TableName] from invoking the `self.schema_name()`
    fn table_ref(&self) -> TableName {
        match self.schema_name() {
            Some(schema) => (Name::runtime(schema), self.into_name()).into_table_name(),
            None => self.into_table_name(),
        }
    }

    /// The table's unique keys over two columns or more, in declaration
    /// order, which `Schema::create_table_from_entity` writes after its
    /// columns' own: what `#[pgorm(unique(..))]` declares, and none by
    /// default. A one-column key is that column's `unique`.
    fn unique_keys(&self) -> &'static [UniqueKey] {
        &[]
    }
}

/// The table a statement over `E` names for `entity`: the instance's schema
/// and table, under `E`'s default table name where the instance's differs.
///
/// Every column reference a statement over `E` writes is qualified by the
/// default's name ([`Column::entity_name`]), so a table read under any
/// other name is read under that one, as `FROM "acme"."cake_2026" AS "cake"`.
/// A schema alone needs no alias: `FROM "acme"."cake"` answers to `cake`. A
/// default naming no table, an empty name, has no name to read one under.
// [spec:pgorm:req:entity.traits.entity-name+7]
pub(crate) fn entity_table<E: EntityName>(entity: &E) -> NamedTable {
    let table = entity.table_ref().into_named_table();
    match default_alias(entity) {
        Some(alias) => table.alias(alias),
        None => table,
    }
}

/// The name a statement over `entity` reads its table under: the default's
/// table name, where it is one and the instance's differs from it.
// [spec:pgorm:req:entity.traits.entity-name+7]
pub(crate) fn default_alias<E: EntityName>(entity: &E) -> Option<Name> {
    let default = E::default();
    let named = !default.table_name().is_empty();
    (named && entity.table_name() != default.table_name()).then(|| Name::new(default))
}

/// A unique key over two columns or more of an entity's table, as
/// [`EntityName::unique_keys`] describes it: the constraint's name, or `None`
/// for PostgreSQL to name it, the columns by their SQL names in key order,
/// and whether nulls are equal under it (`NULLS NOT DISTINCT`).
///
/// It is static data, as the table's name is, so the derive writes it as a
/// constant and nothing is built until a statement is.
// [spec:pgorm:req:entity.traits.entity-name+7]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UniqueKey {
    /// The constraint's name, or `None` for PostgreSQL to choose one.
    pub name: Option<&'static str>,
    /// The key's columns, by their SQL names, in key order.
    pub columns: &'static [&'static str],
    /// Whether nulls are equal under the key.
    pub nulls_not_distinct: bool,
}

/// An abstract base class for defining Entities.
///
/// This trait provides an API for you to inspect it's properties
/// - Column (implemented [`Column`])
/// - Relation (implemented [`Relation`])
/// - Primary Key (implemented [`PrimaryKey`] and [`PrimaryKeyToColumn`])
///
/// It builds the reads [`find`](Entity::find),
/// [`graph`](Entity::graph) and [`find_by_id`](Entity::find_by_id),
/// and one delete, [`delete_by_id`](Entity::delete_by_id). Inserts and
/// updates start from [`Insert`](crate::Insert) and [`Update`](crate::Update),
/// and other deletes from [`Delete`].
///
/// Each has a `_for` form taking a [`ScopeContext`] first, which is how a
/// scoped entity's statements start: [`find_for`](Entity::find_for)
/// reads the rows the context's scopes hold, and the forms taking no context
/// are compile errors for a scoped entity, naming it and its scopes.
// [spec:pgorm:def:entity.traits+5]
// [spec:pgorm:req:entity.traits.crud+6]
// [spec:pgorm:req:scope.compile+1]
pub trait Entity: EntityName + EntityScopes {
    #[allow(missing_docs)]
    type Model: Model<Entity = Self> + FromQueryResult;

    #[allow(missing_docs)]
    type ActiveModel: ActiveModelBehavior<Entity = Self>;

    #[allow(missing_docs)]
    type Column: Column;

    #[allow(missing_docs)]
    type Relation: Relation;

    #[allow(missing_docs)]
    type PrimaryKey: PrimaryKey + PrimaryKeyToColumn<Column = Self::Column>;

    /// Check if the relation belongs to an Entity
    fn belongs_to<R>(related: R) -> RelationBuilder<Self, R, NoColumns>
    where
        R: Entity,
    {
        RelationBuilder::new(RelationType::HasOne, Self::default(), related, false)
    }

    /// Check if the entity has at least one relation
    fn has_one<R>(_: R) -> RelationBuilder<Self, R>
    where
        R: Entity + Related<Self>,
    {
        RelationBuilder::from_rel(RelationType::HasOne, R::to().rev(), true)
    }

    /// Check if the Entity has many relations
    fn has_many<R>(_: R) -> RelationBuilder<Self, R>
    where
        R: Entity + Related<Self>,
    {
        RelationBuilder::from_rel(RelationType::HasMany, R::to().rev(), true)
    }

    /// Construct select statement to find one / all models
    ///
    /// - To select columns, join tables and group by expressions, see [`QuerySelect`](crate::query::QuerySelect)
    /// - To apply where conditions / filters, see [`QueryFilter`](crate::query::QueryFilter)
    /// - To apply order by expressions, see [`QueryOrder`](crate::query::QueryOrder)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use pgorm::{entity::*, error::*, query::*, tests_cfg::cake, DatabasePool};
    /// #
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// let db = pool.get().await?;
    ///
    /// // `one` appends `LIMIT 1` and fails with `Error::RecordNotFound` when no
    /// // row matches; `one_opt` returns `None` in that case instead.
    /// let cake: cake::Model = cake::Entity::find().one(&db).await?;
    /// let maybe_cake: Option<cake::Model> = cake::Entity::find().one_opt(&db).await?;
    ///
    /// let cakes: Vec<cake::Model> = cake::Entity::find().all(&db).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// The statement selects every column of the entity:
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find().build().0,
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake""#
    /// );
    /// ```
    fn find() -> Select<Self>
    where
        Self: ScopedIn<NoContext>,
    {
        Select::new()
    }

    /// [`find`](Entity::find) under the context `cx`: every scope `cx`
    /// holds the value of puts its condition in the `WHERE`, so a scoped
    /// entity's rows are read only where they are `cx`'s.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::scoped::{note, CompanyId, Member, UserId}};
    ///
    /// let member = Member { company: CompanyId(7), user: UserId(3) };
    /// assert_eq!(
    ///     note::Entity::find_for(&member).build().0,
    ///     [
    ///         r#"SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body""#,
    ///         r#"FROM "note" WHERE "note"."company_id" = $1 AND "note"."author_id" = $2"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    ///
    /// A context that does not hold every scope the entity declares does not
    /// compile, and neither does [`find`](Entity::find), whose context
    /// is [`NoContext`]:
    ///
    /// ```compile_fail,E0277
    /// use pgorm::{entity::*, tests_cfg::scoped::{note, CompanyId}};
    ///
    /// let _ = note::Entity::find_for(&CompanyId(7));
    /// ```
    // [spec:pgorm:req:entity.traits.crud+6]
    // [spec:pgorm:sem:scope.read]
    fn find_for<Cx>(cx: &Cx) -> Select<Self>
    where
        Cx: ScopeContext,
        Self: ScopedIn<Cx>,
    {
        Select::scoped(&Self::default(), cx)
    }

    /// [`find`](Entity::find) over the table `entity` names: the
    /// instance's schema and table rather than [`Default`]'s, for an entity
    /// whose names are computed at run time, such as a schema per tenant.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::tenant_cake};
    ///
    /// let acme = tenant_cake::Entity { tenant: "acme" };
    /// assert_eq!(
    ///     tenant_cake::Entity::find_in(acme).build().0,
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "acme"."cake""#
    /// );
    /// ```
    ///
    /// Columns are qualified by the default's table name, so an instance
    /// naming another table is read under that name:
    /// `FROM "acme"."cake_2026" AS "cake"`.
    // [spec:pgorm:req:entity.traits.crud+6]
    fn find_in(entity: Self) -> Select<Self>
    where
        Self: ScopedIn<NoContext>,
    {
        Select::read(&entity)
    }

    /// Construct an N-ary relational read rooted at this entity.
    ///
    /// Where [`find`](Entity::find) produces a
    /// [`Select<E>`](crate::Select), this produces a
    /// [`SelectGraph<E>`](crate::SelectGraph): a root plus a typed list of
    /// joined, decoded sources, each declared with the join type its decode
    /// shape implies. The root is projected under the `s0_` prefix at
    /// construction, and each slot under the next.
    ///
    /// ```no_run
    /// # use pgorm::{entity::*, error::*, query::*, tests_cfg::{cake, fruit, vendor}, DatabasePool};
    /// #
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// let db = pool.get().await?;
    ///
    /// // An optional source decodes as `Option<Model>`, a required one as a
    /// // bare `Model` — the join type and the decode are one declaration.
    /// let rows: Vec<(cake::Model, Option<fruit::Model>)> = cake::Entity::graph()
    ///     .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
    ///     .all(&db)
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// There is no conversion between `Select<E>` and `SelectGraph<E, S>` in
    /// either direction: a graph's projection is generated from its
    /// declaration, never inherited from a builder whose select list a caller
    /// may have edited.
    // [spec:pgorm:def:query.graph+3]
    #[doc(alias(
        "find_also_related",
        "find_with_related",
        "find_also_linked",
        "find_with_linked"
    ))]
    fn graph() -> SelectGraph<Self>
    where
        Self: ScopedIn<NoContext>,
    {
        SelectGraph::new()
    }

    /// [`graph`](Entity::graph) under the context `cx`, which the graph
    /// keeps: the root's scopes go in the `WHERE`, and each slot and hop
    /// joined after puts its own table's scopes in its `ON`, so a slot of an
    /// entity `cx` does not hold the scopes of does not compile.
    // [spec:pgorm:def:query.graph+3]
    // [spec:pgorm:sem:scope.join]
    fn graph_for<Cx>(cx: &Cx) -> SelectGraph<Self, (), Unsliced, Cx>
    where
        Cx: ScopeContext + Clone,
        Self: ScopedIn<Cx>,
    {
        SelectGraph::rooted_under(&Self::default(), cx.clone())
    }

    /// [`graph`](Entity::graph) rooted at the table `entity` names, as
    /// [`find_in`](Entity::find_in) reads it. The slots join the tables
    /// their relations name.
    // [spec:pgorm:def:query.graph+3]
    fn graph_in(entity: Self) -> SelectGraph<Self>
    where
        Self: ScopedIn<NoContext>,
    {
        SelectGraph::rooted_under(&entity, NoContext)
    }

    /// Find a model by primary key
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use pgorm::{entity::*, error::*, query::*, tests_cfg::cake, DatabasePool};
    /// #
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// let db = pool.get().await?;
    ///
    /// let sponge_cake: cake::Model = cake::Entity::find_by_id(11).one(&db).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find_by_id(11).build().0,
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = $1"#
    /// );
    /// ```
    /// Find by composite key
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake_filling};
    ///
    /// assert_eq!(
    ///     cake_filling::Entity::find_by_id((2, 3)).build().0,
    ///     [
    ///         r#"SELECT "cake_filling"."cake_id", "cake_filling"."filling_id" FROM "cake_filling""#,
    ///         r#"WHERE "cake_filling"."cake_id" = $1 AND "cake_filling"."filling_id" = $2"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if arity of input values don't match arity of primary key
    // [spec:pgorm:req:entity.traits.crud+6]
    fn find_by_id<T>(values: T) -> Select<Self>
    where
        T: IntoPrimaryKey<<Self::PrimaryKey as PrimaryKey>::ValueType>,
        Self: ScopedIn<NoContext>,
    {
        filter_by_key::<Self, _, _>(Self::find(), values)
    }

    /// [`find_by_id`](Entity::find_by_id) under the context `cx`, as
    /// [`find_for`](Entity::find_for) reads it: the key and the scopes
    /// side by side, so a key of a row outside `cx`'s scopes finds nothing.
    ///
    /// # Panics
    ///
    /// Panics if arity of input values don't match arity of primary key
    // [spec:pgorm:req:entity.traits.crud+6]
    // [spec:pgorm:sem:scope.read]
    fn find_by_id_for<Cx, T>(cx: &Cx, values: T) -> Select<Self>
    where
        Cx: ScopeContext,
        T: IntoPrimaryKey<<Self::PrimaryKey as PrimaryKey>::ValueType>,
        Self: ScopedIn<Cx>,
    {
        filter_by_key::<Self, _, _>(Self::find_for(cx), values)
    }

    /// [`find_by_id`](Entity::find_by_id) over the table `entity`
    /// names, as [`find_in`](Entity::find_in) reads it.
    ///
    /// # Panics
    ///
    /// Panics if arity of input values don't match arity of primary key
    // [spec:pgorm:req:entity.traits.crud+6]
    fn find_by_id_in<T>(entity: Self, values: T) -> Select<Self>
    where
        T: IntoPrimaryKey<<Self::PrimaryKey as PrimaryKey>::ValueType>,
        Self: ScopedIn<NoContext>,
    {
        filter_by_key::<Self, _, _>(Self::find_in(entity), values)
    }

    /// Delete a model based on primary key
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use pgorm::{entity::*, error::*, query::*, tests_cfg::fruit, DatabasePool};
    /// #
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// let db = pool.get().await?;
    ///
    /// let delete_result = fruit::Entity::delete_by_id(1).exec(&db).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::fruit};
    ///
    /// assert_eq!(
    ///     fruit::Entity::delete_by_id(1).build().0,
    ///     r#"DELETE FROM "fruit" WHERE "fruit"."id" = $1"#
    /// );
    /// ```
    /// Delete by composite key
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake_filling};
    ///
    /// assert_eq!(
    ///     cake_filling::Entity::delete_by_id((2, 3)).build().0,
    ///     [
    ///         r#"DELETE FROM "cake_filling""#,
    ///         r#"WHERE "cake_filling"."cake_id" = $1 AND "cake_filling"."filling_id" = $2"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if arity of input values don't match arity of primary key
    fn delete_by_id<T>(values: T) -> DeleteMany<Self>
    where
        T: IntoPrimaryKey<<Self::PrimaryKey as PrimaryKey>::ValueType>,
        Self: ScopedIn<NoContext>,
    {
        filter_by_key::<Self, _, _>(Delete::many(Self::default()), values)
    }

    /// [`delete_by_id`](Entity::delete_by_id) under the context `cx`,
    /// as [`Delete::many_for`] deletes: the key and the scopes side by side,
    /// so a key of a row outside `cx`'s scopes deletes nothing.
    ///
    /// # Panics
    ///
    /// Panics if arity of input values don't match arity of primary key
    // [spec:pgorm:req:entity.traits.crud+6]
    // [spec:pgorm:sem:scope.write]
    fn delete_by_id_for<Cx, T>(cx: &Cx, values: T) -> DeleteMany<Self>
    where
        Cx: ScopeContext,
        T: IntoPrimaryKey<<Self::PrimaryKey as PrimaryKey>::ValueType>,
        Self: ScopedIn<Cx>,
    {
        filter_by_key::<Self, _, _>(Delete::many_for(cx, Self::default()), values)
    }

    /// [`delete_by_id`](Entity::delete_by_id) from the table `entity`
    /// names, as [`Delete::many`] deletes from it.
    ///
    /// # Panics
    ///
    /// Panics if arity of input values don't match arity of primary key
    // [spec:pgorm:req:entity.traits.crud+6]
    fn delete_by_id_in<T>(entity: Self, values: T) -> DeleteMany<Self>
    where
        T: IntoPrimaryKey<<Self::PrimaryKey as PrimaryKey>::ValueType>,
        Self: ScopedIn<NoContext>,
    {
        filter_by_key::<Self, _, _>(Delete::many(entity), values)
    }
}

/// `query` narrowed to the row of one primary key of `E`, each part of
/// `values` compared with its key column under the column's casts: what
/// [`Entity::find_by_id`] and [`Entity::delete_by_id`] narrow by,
/// and a disjoint entity's `change_kind`.
///
/// # Panics
///
/// Panics if the arity of `values` does not match the arity of the primary key
// [spec:pgorm:req:entity.traits.crud+6]
pub(crate) fn filter_by_key<E, Q, T>(mut query: Q, values: T) -> Q
where
    E: Entity,
    Q: QueryFilter,
    T: IntoPrimaryKey<<E::PrimaryKey as PrimaryKey>::ValueType>,
{
    let mut keys = E::PrimaryKey::iter();
    for v in values.into_primary_key().into_value_tuple() {
        if let Some(key) = keys.next() {
            query = query.filter(key.into_column().eq(v));
        } else {
            panic!("primary key arity mismatch");
        }
    }
    if keys.next().is_some() {
        panic!("primary key arity mismatch");
    }
    query
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_delete_by_id_1() {
        use crate::tests_cfg::cake;
        use crate::{entity::*, query::*};

        assert_eq!(
            cake::Entity::delete_by_id(1).as_query().to_string(),
            r#"DELETE FROM "cake" WHERE "cake"."id" = 1"#,
        );
    }

    #[test]
    fn test_delete_by_id_2() {
        use crate::tests_cfg::cake_filling_price;
        use crate::{entity::*, query::*};

        assert_eq!(
            cake_filling_price::Entity::delete_by_id((1, 2))
                .as_query()
                .to_string(),
            r#"DELETE FROM "public"."cake_filling_price" WHERE "cake_filling_price"."cake_id" = 1 AND "cake_filling_price"."filling_id" = 2"#,
        );
    }

    #[test]
    #[cfg(feature = "macros")]
    fn entity_model_1() {
        use crate::entity::*;

        mod hello {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        assert_eq!(hello::Entity.table_name(), "hello");
        assert_eq!(hello::Entity.schema_name(), None);
    }

    #[test]
    #[cfg(feature = "macros")]
    fn entity_model_2() {
        use crate::entity::*;

        mod hello {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello", schema_name = "world")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        assert_eq!(hello::Entity.table_name(), "hello");
        assert_eq!(hello::Entity.schema_name(), Some("world"));
    }

    #[test]
    #[cfg(feature = "macros")]
    fn entity_model_3() {
        use crate::{entity::*, query::*};

        use std::borrow::Cow;

        mod hello {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "hello", schema_name = "world")]
            pub struct Model {
                #[pgorm(primary_key, auto_increment = false)]
                pub id: String,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        fn delete_by_id<T>(value: T)
        where
            T: IntoPrimaryKey<<<hello::Entity as Entity>::PrimaryKey as PrimaryKey>::ValueType>,
        {
            assert_eq!(
                hello::Entity::delete_by_id(value).as_query().to_string(),
                r#"DELETE FROM "world"."hello" WHERE "hello"."id" = 'UUID'"#
            );
        }

        delete_by_id("UUID".to_string());
        delete_by_id("UUID");
        delete_by_id(Cow::from("UUID"));
    }
}
