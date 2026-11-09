use crate::{ColumnPairs, EntityTrait, Iterable, QuerySelect, Select, unpack_table_name};
use core::marker::PhantomData;
use pgorm_query::{
    Condition, ConditionType, Deferrability, Enforcement, ForeignKeyCreateStatement, FromItem,
    IntoName, JoinType, Name, TableForeignKey, TableName, alias,
};
use std::fmt::Debug;

/// Defines the type of relationship
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelationType {
    /// An Entity has one relationship
    HasOne,
    /// An Entity has many relationships
    HasMany,
}

/// Action to perform on a foreign key whenever there are changes
/// to an ActiveModel
pub(crate) type ForeignKeyAction = pgorm_query::ForeignKeyAction;

/// Defines the relations of an Entity
// [spec:pgorm:req:entity.relation+1]
pub trait RelationTrait: Iterable + Debug + 'static {
    /// The method to call
    fn def(&self) -> RelationDef;
}

/// Checks if Entities are related
// [spec:pgorm:req:entity.relation+1]
pub trait Related<R>
where
    R: EntityTrait,
{
    /// Check if an entity is related to another entity
    fn to() -> RelationDef;

    /// Check if an entity is related through another entity
    fn via() -> Option<RelationDef> {
        None
    }

    /// Find related Entities
    fn find_related() -> Select<R> {
        Select::<R>::new().join_join_rev(JoinType::InnerJoin, Self::to(), Self::via())
    }
}

/// Defines a relationship
// [spec:pgorm:def:entity.relation.def+12]
pub struct RelationDef {
    /// The type of relationship defined in [RelationType]
    pub rel_type: RelationType,
    /// Reference from another Entity
    pub from_tbl: FromItem,
    /// Reference to another ENtity
    pub to_tbl: FromItem,
    /// The columns joined, as `(from, to)` pairs
    pub columns: ColumnPairs,
    /// Defines the owner of the Relation
    pub is_owner: bool,
    /// Defines an operation to be performed on a Foreign Key when a
    /// `DELETE` Operation is performed
    pub on_delete: Option<ForeignKeyAction>,
    /// Defines an operation to be performed on a Foreign Key when a
    /// `UPDATE` Operation is performed
    pub on_update: Option<ForeignKeyAction>,
    /// Custom join ON condition
    pub on_condition: Option<Box<dyn Fn(Name, Name) -> Condition + Send + Sync>>,
    /// The name of foreign key constraint
    pub fk_name: Option<String>,
    /// Whether the server holds rows to the foreign key, if the relation
    /// says: a `NOT ENFORCED` key is recorded and never checked.
    // [spec:pgorm:def:entity.relation.def+12]
    pub enforcement: Option<Enforcement>,
    /// When the foreign key's check runs, if the relation says.
    // [spec:pgorm:def:entity.relation.def+12]
    pub deferrability: Option<Deferrability>,
    /// Condition type of join on expression
    pub condition_type: ConditionType,
}

impl std::fmt::Debug for RelationDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("RelationDef");
        d.field("rel_type", &self.rel_type)
            .field("from_tbl", &self.from_tbl)
            .field("to_tbl", &self.to_tbl)
            .field("columns", &self.columns)
            .field("is_owner", &self.is_owner)
            .field("on_delete", &self.on_delete)
            .field("on_update", &self.on_update);
        debug_on_condition(&mut d, &self.on_condition);
        d.field("fk_name", &self.fk_name)
            .field("enforcement", &self.enforcement)
            .field("deferrability", &self.deferrability)
            .finish()
    }
}

fn debug_on_condition(
    d: &mut core::fmt::DebugStruct<'_, '_>,
    on_condition: &Option<Box<dyn Fn(Name, Name) -> Condition + Send + Sync>>,
) {
    match on_condition {
        Some(func) => {
            d.field(
                "on_condition",
                &func(Name::new(alias("left")), Name::new(alias("right"))),
            );
        }
        None => {
            d.field("on_condition", &Option::<Condition>::None);
        }
    }
}

/// The state of a [`RelationBuilder`] that has not been given its join columns
/// yet. Such a builder has no conversion into a [`RelationDef`], so a relation
/// missing its columns is a compile error rather than a panic.
// [spec:pgorm:req:entity.relation.builder+1]
#[derive(Debug)]
pub struct NoColumns;

/// Defines a helper to build a relation
///
/// `C` tracks whether the join columns have been supplied: a fresh
/// `belongs_to` builder is a `RelationBuilder<E, R, NoColumns>` and becomes a
/// `RelationBuilder<E, R, ColumnPairs>` once [`RelationBuilder::columns`] names
/// its first pair.
// [spec:pgorm:req:entity.relation.builder+1]
pub struct RelationBuilder<E, R, C = ColumnPairs>
where
    E: EntityTrait,
    R: EntityTrait,
{
    entities: PhantomData<(E, R)>,
    rel_type: RelationType,
    from_tbl: FromItem,
    to_tbl: FromItem,
    columns: C,
    is_owner: bool,
    on_delete: Option<ForeignKeyAction>,
    on_update: Option<ForeignKeyAction>,
    on_condition: Option<Box<dyn Fn(Name, Name) -> Condition + Send + Sync>>,
    fk_name: Option<String>,
    enforcement: Option<Enforcement>,
    deferrability: Option<Deferrability>,
    condition_type: ConditionType,
}

impl<E, R, C> std::fmt::Debug for RelationBuilder<E, R, C>
where
    E: EntityTrait,
    R: EntityTrait,
    C: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("RelationBuilder");
        d.field("entities", &self.entities)
            .field("rel_type", &self.rel_type)
            .field("from_tbl", &self.from_tbl)
            .field("to_tbl", &self.to_tbl)
            .field("columns", &self.columns)
            .field("is_owner", &self.is_owner)
            .field("on_delete", &self.on_delete)
            .field("on_update", &self.on_update);
        debug_on_condition(&mut d, &self.on_condition);
        d.field("fk_name", &self.fk_name)
            .field("enforcement", &self.enforcement)
            .field("deferrability", &self.deferrability)
            .finish()
    }
}

impl RelationDef {
    /// A belongs-to relation from the table `from` to the table `to`, its
    /// foreign key over `columns`, with no action, name, condition,
    /// enforcement or deferrability until its fields say otherwise: the
    /// relation a table described at run time is keyed by
    /// ([`TableShape::with_relation`](crate::TableShape::with_relation)).
    // [spec:pgorm:sem:schema.from-shape]
    pub fn belongs_to(from: TableName, to: TableName, columns: ColumnPairs) -> Self {
        Self {
            rel_type: RelationType::HasOne,
            from_tbl: FromItem::from(from),
            to_tbl: FromItem::from(to),
            columns,
            is_owner: false,
            on_delete: None,
            on_update: None,
            on_condition: None,
            fk_name: None,
            enforcement: None,
            deferrability: None,
            condition_type: ConditionType::All,
        }
    }

    /// Reverse this relation (swap from and to).
    ///
    /// An `on_condition` closure already attached keeps its authored roles:
    /// its arguments are re-swapped along with the tables, so a predicate
    /// written for `(source, target)` never silently starts receiving
    /// `(target, source)`. A closure attached *after* reversing is authored
    /// against the reversed roles, as its author sees them.
    // [spec:pgorm:def:entity.relation.def+12]
    pub fn rev(mut self) -> Self {
        let on_condition = self.on_condition.take().map(|f| {
            Box::new(move |left: Name, right: Name| f(right, left))
                as Box<dyn Fn(Name, Name) -> Condition + Send + Sync>
        });
        Self {
            rel_type: self.rel_type,
            from_tbl: self.to_tbl,
            to_tbl: self.from_tbl,
            columns: self.columns.rev(),
            is_owner: !self.is_owner,
            on_delete: self.on_delete,
            on_update: self.on_update,
            on_condition,
            fk_name: None,
            enforcement: self.enforcement,
            deferrability: self.deferrability,
            condition_type: self.condition_type,
        }
    }

    /// Express the relation from a table alias.
    ///
    /// This is a shorter and more discoverable equivalent to modifying `from_tbl` field by hand.
    ///
    /// # Examples
    ///
    /// Here's a short synthetic example.
    /// In real life you'd use aliases when the table name comes up twice and you need to disambiguate,
    /// e.g. SeaORM's discussion 2133, <https://github.com/SeaQL/sea-orm/discussions/2133>
    ///
    /// ```
    /// use pgorm::{
    ///     entity::*,
    ///     query::*,
    ///     tests_cfg::{cake, cake_filling},
    /// };
    /// use pgorm::alias;
    ///
    /// let cf = alias("cf");
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .join_as(
    ///             JoinType::LeftJoin,
    ///             cake_filling::Relation::Cake.def().rev(),
    ///             cf
    ///         )
    ///         .join(
    ///             JoinType::LeftJoin,
    ///             cake_filling::Relation::Filling.def().from_alias(cf)
    ///         )
    ///         .as_query()
    ///         .to_string(),
    ///     [
    ///         r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
    ///         r#"LEFT JOIN "cake_filling" AS "cf" ON "cake"."id" = "cf"."cake_id""#,
    ///         r#"LEFT JOIN "filling" ON "cf"."filling_id" = "filling"."id""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn from_alias<A>(mut self, alias: A) -> Self
    where
        A: IntoName,
    {
        self.from_tbl = self.from_tbl.alias(alias);
        self
    }

    /// Set custom join ON condition.
    ///
    /// This method takes a closure with two parameters
    /// denoting the left-hand side and right-hand side table in the join expression.
    ///
    /// This replaces the current condition if it is already set.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::{cake, cake_filling}};
    /// use pgorm_query::{Expr, ExprTrait, IntoCondition};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .join(
    ///             JoinType::LeftJoin,
    ///             cake_filling::Relation::Cake
    ///                 .def()
    ///                 .rev()
    ///                 .on_condition(|_left, right| {
    ///                     Expr::col((right, cake_filling::Column::CakeId))
    ///                         .gt(10i32)
    ///                         .into_condition()
    ///                 })
    ///         )
    ///         .as_query()
    ///         .to_string(),
    ///     [
    ///         r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
    ///         r#"LEFT JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id" AND "cake_filling"."cake_id" > 10"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn on_condition<F>(mut self, f: F) -> Self
    where
        F: Fn(Name, Name) -> Condition + 'static + Send + Sync,
    {
        self.on_condition = Some(Box::new(f));
        self
    }

    /// Set the condition type of join on expression
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::{cake, cake_filling}};
    /// use pgorm_query::{Expr, ExprTrait, IntoCondition, ConditionType};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .join(
    ///             JoinType::LeftJoin,
    ///             cake_filling::Relation::Cake
    ///                 .def()
    ///                 .rev()
    ///                 .condition_type(ConditionType::Any)
    ///                 .on_condition(|_left, right| {
    ///                     Expr::col((right, cake_filling::Column::CakeId))
    ///                         .gt(10i32)
    ///                         .into_condition()
    ///                 })
    ///         )
    ///         .as_query()
    ///         .to_string(),
    ///     [
    ///         r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
    ///         r#"LEFT JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id" OR "cake_filling"."cake_id" > 10"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn condition_type(mut self, condition_type: ConditionType) -> Self {
        self.condition_type = condition_type;
        self
    }
}

impl<E, R> RelationBuilder<E, R, NoColumns>
where
    E: EntityTrait,
    R: EntityTrait,
{
    pub(crate) fn new(rel_type: RelationType, from: E, to: R, is_owner: bool) -> Self {
        Self {
            entities: PhantomData,
            rel_type,
            from_tbl: from.table_ref().into(),
            to_tbl: to.table_ref().into(),
            columns: NoColumns,
            is_owner,
            on_delete: None,
            on_update: None,
            on_condition: None,
            fk_name: None,
            enforcement: None,
            deferrability: None,
            condition_type: ConditionType::All,
        }
    }

    /// Name the first pair of columns the relation joins on.
    ///
    /// A pair is the unit of the relation: a composite key is declared by
    /// following this with [`RelationBuilder::and_columns`], so the two sides
    /// can never be given different numbers of columns.
    pub fn columns(self, from: E::Column, to: R::Column) -> RelationBuilder<E, R, ColumnPairs> {
        RelationBuilder {
            entities: self.entities,
            rel_type: self.rel_type,
            from_tbl: self.from_tbl,
            to_tbl: self.to_tbl,
            columns: ColumnPairs::of_columns(from, to),
            is_owner: self.is_owner,
            on_delete: self.on_delete,
            on_update: self.on_update,
            on_condition: self.on_condition,
            fk_name: self.fk_name,
            enforcement: self.enforcement,
            deferrability: self.deferrability,
            condition_type: self.condition_type,
        }
    }
}

impl<E, R> RelationBuilder<E, R, ColumnPairs>
where
    E: EntityTrait,
    R: EntityTrait,
{
    pub(crate) fn from_rel(rel_type: RelationType, rel: RelationDef, is_owner: bool) -> Self {
        Self {
            entities: PhantomData,
            rel_type,
            from_tbl: rel.from_tbl,
            to_tbl: rel.to_tbl,
            columns: rel.columns,
            is_owner,
            on_delete: None,
            on_update: None,
            on_condition: None,
            fk_name: None,
            enforcement: None,
            deferrability: None,
            condition_type: ConditionType::All,
        }
    }

    /// Replace the columns the relation joins on with this single pair.
    pub fn columns(mut self, from: E::Column, to: R::Column) -> Self {
        self.columns = ColumnPairs::of_columns(from, to);
        self
    }

    /// Join on a further pair of columns, as a composite key requires.
    pub fn and_columns(mut self, from: E::Column, to: R::Column) -> Self {
        self.columns.push_columns(from, to);
        self
    }

    /// Match `from` to `to` as periods, after every pair matched for equality:
    /// PostgreSQL 18's temporal foreign key, `FOREIGN KEY (.., PERIOD from)
    /// REFERENCES .. (.., PERIOD to)`. The relation joins a row to each row
    /// whose period overlaps its own, of which there may be several — the
    /// versions of one key whose periods together cover it.
    // [spec:pgorm:def:entity.relation.def+12]
    pub fn period(mut self, from: E::Column, to: R::Column) -> Self {
        self.columns.set_period(from, to);
        self
    }
}

impl<E, R, C> RelationBuilder<E, R, C>
where
    E: EntityTrait,
    R: EntityTrait,
{
    /// An operation to perform on a foreign key when a delete operation occurs
    pub fn on_delete(mut self, action: ForeignKeyAction) -> Self {
        self.on_delete = Some(action);
        self
    }

    /// An operation to perform on a foreign key when an update operation occurs
    pub fn on_update(mut self, action: ForeignKeyAction) -> Self {
        self.on_update = Some(action);
        self
    }

    /// Set custom join ON condition.
    ///
    /// This method takes a closure with parameters
    /// denoting the left-hand side and right-hand side table in the join expression.
    pub fn on_condition<F>(mut self, f: F) -> Self
    where
        F: Fn(Name, Name) -> Condition + 'static + Send + Sync,
    {
        self.on_condition = Some(Box::new(f));
        self
    }

    /// Set the name of foreign key constraint
    pub fn fk_name(mut self, fk_name: &str) -> Self {
        self.fk_name = Some(fk_name.to_owned());
        self
    }

    /// Set the condition type of join on expression
    pub fn condition_type(mut self, condition_type: ConditionType) -> Self {
        self.condition_type = condition_type;
        self
    }

    /// Say whether the server holds rows to the relation's foreign key:
    /// [`Enforcement::NotEnforced`] records the key without checking it, so
    /// the table may hold rows it references nothing for.
    // [spec:pgorm:def:entity.relation.def+12]
    pub fn enforcement(mut self, enforcement: Enforcement) -> Self {
        self.enforcement = Some(enforcement);
        self
    }

    /// Say when the relation's foreign key is checked
    /// ([`Deferrability::DeferrableInitiallyDeferred`] checks it at commit).
    // [spec:pgorm:def:entity.relation.def+12]
    pub fn deferrability(mut self, deferrability: Deferrability) -> Self {
        self.deferrability = Some(deferrability);
        self
    }
}

// [spec:pgorm:req:entity.relation.builder+1]
impl<E, R> From<RelationBuilder<E, R, ColumnPairs>> for RelationDef
where
    E: EntityTrait,
    R: EntityTrait,
{
    fn from(b: RelationBuilder<E, R, ColumnPairs>) -> Self {
        RelationDef {
            rel_type: b.rel_type,
            from_tbl: b.from_tbl,
            to_tbl: b.to_tbl,
            columns: b.columns,
            is_owner: b.is_owner,
            on_delete: b.on_delete,
            on_update: b.on_update,
            on_condition: b.on_condition,
            fk_name: b.fk_name,
            enforcement: b.enforcement,
            deferrability: b.deferrability,
            condition_type: b.condition_type,
        }
    }
}

/// Build a foreign key of `$ty` from `$relation`.
///
/// A [`RelationDef`]'s column pairs are non-empty and balanced, which is exactly
/// what a foreign key is built from, so the conversion is total.
macro_rules! foreign_key_from_relation {
    ( $relation: ident, $ty: ty ) => {{
        let from_tbl = unpack_table_name(&$relation.from_tbl);
        let to_tbl = unpack_table_name(&$relation.to_tbl);
        let (from, to) = $relation.columns.first();
        let mut foreign_key =
            <$ty>::new(from_tbl.clone(), Name::clone(from), to_tbl, Name::clone(to));
        for (from, to) in $relation.columns.iter().skip(1) {
            foreign_key.col(Name::clone(from), Name::clone(to));
        }
        if let Some((from, to)) = $relation.columns.period() {
            foreign_key.period(Name::clone(from), Name::clone(to));
        }
        if let Some(action) = $relation.on_delete {
            foreign_key.on_delete(action);
        }
        if let Some(action) = $relation.on_update {
            foreign_key.on_update(action);
        }
        if let Some(deferrability) = $relation.deferrability {
            foreign_key.deferrability(deferrability);
        }
        if let Some(enforcement) = $relation.enforcement {
            foreign_key.enforcement(enforcement);
        }
        let name = if let Some(name) = &$relation.fk_name {
            name.clone()
        } else {
            let from_cols: Vec<String> = $relation
                .columns
                .from_key()
                .iter()
                .map(|from| from.to_string())
                .collect();
            crate::executor::result_name::derived_name(
                "fk",
                &from_tbl.table().to_string(),
                from_cols.iter().map(String::as_str),
            )
        };
        foreign_key.name(Name::runtime(name));
        foreign_key
    }};
}

// [spec:pgorm:req:entity.relation.fk+6]
impl From<RelationDef> for ForeignKeyCreateStatement {
    fn from(relation: RelationDef) -> Self {
        foreign_key_of(&relation)
    }
}

/// The foreign key `relation` holds, as its `From` conversion writes it, for
/// a relation the caller keeps.
// [spec:pgorm:req:entity.relation.fk+6]
pub(crate) fn foreign_key_of(relation: &RelationDef) -> ForeignKeyCreateStatement {
    foreign_key_from_relation!(relation, ForeignKeyCreateStatement)
}

/// Creates a column definition for example to update a table.
/// ```
/// use pgorm_query::{ConditionType, FromItem, IntoName, Table, TableForeignKey, TableName};
/// use pgorm::{alias, ColumnPairs, SqlName, PrimaryKeyTrait, RelationDef, RelationTrait, RelationType};
///
/// let relation = RelationDef {
///     rel_type: RelationType::HasOne,
///     from_tbl: FromItem::from(TableName::Table(alias("foo").into_name())),
///     to_tbl: FromItem::from(TableName::Table(alias("bar").into_name())),
///     columns: ColumnPairs::new(alias("bar_id"), alias("bar_id")),
///     is_owner: false,
///     on_delete: None,
///     on_update: None,
///     on_condition: None,
///     fk_name: Some("foo-bar".to_string()),
///     enforcement: None,
///     deferrability: None,
///     condition_type: ConditionType::All,
/// };
///
/// let alter_table = Table::alter(TableName::Table(alias("foo").into_name()))
///     .add_foreign_key(TableForeignKey::from(relation));
/// assert_eq!(
///     alter_table.to_string(),
///     r#"ALTER TABLE "foo" ADD CONSTRAINT "foo-bar" FOREIGN KEY ("bar_id") REFERENCES "bar" ("bar_id")"#
/// );
/// ```
// [spec:pgorm:req:entity.relation.fk+6]
impl From<RelationDef> for TableForeignKey {
    fn from(relation: RelationDef) -> Self {
        foreign_key_from_relation!(relation, Self)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        RelationBuilder, RelationDef,
        tests_cfg::{cake, fruit},
    };

    #[test]
    fn assert_relation_traits() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<RelationDef>();
        assert_send_sync::<RelationBuilder<cake::Entity, fruit::Entity>>();
    }
}
