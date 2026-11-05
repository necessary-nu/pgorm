use crate::{
    ConnectionTrait, Delete, EntityTrait, Insert, Iterable, PrimaryKeyArity, PrimaryKeyToColumn,
    PrimaryKeyTrait, Update, Value, error::*,
};
use async_trait::async_trait;
use pgorm_query::{Nullable, ValueTuple};
use std::fmt::Debug;

pub use ActiveValue::NotSet;

/// Defines a stateful value used in ActiveModel.
///
/// There are three possible state represented by three enum variants.
/// - [ActiveValue::Set]: A defined [Value] actively being set
/// - [ActiveValue::Unchanged]: A defined [Value] remain unchanged
/// - [ActiveValue::NotSet]: An undefined [Value]
///
/// Write [`set(value)`][set] to construct the `Set` state: it converts into the
/// field's type, so a `&str` literal reaches an `ActiveValue<String>` without a
/// `.to_owned()`. The [`Set`][ActiveValue::Set] variant is spelled out when
/// matching on the state, where a function call cannot appear.
///
/// The stateful value is useful when constructing UPDATE SQL statement,
/// see an example below.
///
/// # Examples
///
/// ```
/// use pgorm::tests_cfg::{cake, fruit};
/// use pgorm::{entity::*, query::*};
///
/// // The code snipped below does an UPDATE operation on a `ActiveValue`
/// assert_eq!(
///     Update::one(fruit::ActiveModel {
///         id: set(1),
///         name: set("Orange"),
///         cake_id: NotSet,
///     })
///     .expect("the primary key is set")
///     .as_query()
///     .to_string(),
///     r#"UPDATE "fruit" SET "name" = 'Orange' WHERE "fruit"."id" = 1"#
/// );
/// ```
// [spec:pgorm:def:entity.active-model.active-value+1]
#[derive(Clone, Debug)]
pub enum ActiveValue<V>
where
    V: Into<Value>,
{
    /// A defined [Value] actively being set
    Set(V),
    /// A defined [Value] remain unchanged
    Unchanged(V),
    /// An undefined [Value]
    NotSet,
}

/// Construct an [`ActiveValue::Set`] from anything that converts into the
/// column's type.
///
/// The [`Set`][ActiveValue::Set] variant and
/// [`ActiveValue::set`][ActiveValue::set] both pin their argument to the field
/// type exactly; this free function does not, so the ordinary conversions apply
/// at the call site:
///
/// ```
/// use pgorm::tests_cfg::fruit;
/// use pgorm::entity::*;
///
/// let am = fruit::ActiveModel {
///     id: set(1),
///     // `&str`, not `String` — no `.to_owned()`
///     name: set("Orange"),
///     cake_id: NotSet,
/// };
/// assert_eq!(am.name, ActiveValue::Set("Orange".to_owned()));
/// ```
// [spec:pgorm:req:entity.active-model.from-sugar+1]
pub fn set<V, T>(value: T) -> ActiveValue<V>
where
    V: Into<Value>,
    T: Into<V>,
{
    ActiveValue::Set(value.into())
}

// [spec:pgorm:req:entity.active-model.from-sugar+1]
impl<V: Into<Value>> From<V> for ActiveValue<V> {
    fn from(value: V) -> Self {
        ActiveValue::Set(value)
    }
}

/// Targeted borrowed-to-owned conversions the blanket `From<V>` above cannot
/// express: it pins the source type to the field type exactly, so `&str` only
/// ever reaches an `ActiveValue<&str>`. Each target below differs from the one
/// the blanket produces for the same source, so they cohere with it.
// [spec:pgorm:req:entity.active-model.from-sugar+1]
impl From<&str> for ActiveValue<String> {
    fn from(value: &str) -> Self {
        ActiveValue::Set(value.to_owned())
    }
}

// [spec:pgorm:req:entity.active-model.from-sugar+1]
impl From<&str> for ActiveValue<Option<String>> {
    fn from(value: &str) -> Self {
        ActiveValue::Set(Some(value.to_owned()))
    }
}

// [spec:pgorm:req:entity.active-model.from-sugar+1]
impl From<&[u8]> for ActiveValue<Vec<u8>> {
    fn from(value: &[u8]) -> Self {
        ActiveValue::Set(value.to_vec())
    }
}

/// A Trait for ActiveModel to perform Create, Update or Delete operation.
/// The type must also implement the [EntityTrait].
///
/// Writes state their intent: pick [`ActiveModelTrait::insert`] or
/// [`ActiveModelTrait::update`]. There is no `save` that guesses between the two
/// from the primary-key state, because `Set` and `Unchanged` keys are
/// indistinguishable to such a guess. For insert-or-update in one statement,
/// attach an `ON CONFLICT` clause with [`Insert::on_conflict`](crate::Insert::on_conflict).
///
/// Each write is one statement on the connection it is handed, so an `Err` from
/// an *after* hook reaches the caller with the row already written — see
/// [`ActiveModelBehavior`] for what the hooks can and cannot undo.
///
/// See module level docs [crate::entity] for a full example
// [spec:pgorm:req:entity.active-model+4]
// [spec:pgorm:req:entity.active-model.save+1]
// [spec:pgorm:req:entity.active-model.hooks+1]
#[async_trait]
pub trait ActiveModelTrait: Clone + Debug {
    /// The Entity this ActiveModel belongs to
    type Entity: EntityTrait;

    /// Remove the [ActiveValue] from an ActiveModel, leaving [`NotSet`] in its
    /// place and returning what was there.
    ///
    /// A column this model does not carry is [`Error::Type`], on the same terms
    /// as [`Self::set`]: asking for a column that does not exist is a different
    /// answer from a column that is merely [`NotSet`], and the two MUST NOT
    /// arrive spelled the same way.
    fn take(
        &mut self,
        c: <Self::Entity as EntityTrait>::Column,
    ) -> Result<ActiveValue<Value>, Error>;

    /// Read a copy of the [ActiveValue] an ActiveModel holds, reporting a column
    /// this model does not carry as [`Error::Type`].
    fn get(&self, c: <Self::Entity as EntityTrait>::Column) -> Result<ActiveValue<Value>, Error>;

    /// Set the Value into an ActiveModel, reporting a column this model does not
    /// carry, or a value of the wrong type for it, as [`Error::Type`].
    fn set(&mut self, c: <Self::Entity as EntityTrait>::Column, v: Value) -> Result<(), Error>;

    /// Set the state of an [ActiveValue] to the not set state
    fn not_set(&mut self, c: <Self::Entity as EntityTrait>::Column);

    /// Check the state of a [ActiveValue]
    fn is_not_set(&self, c: <Self::Entity as EntityTrait>::Column) -> bool;

    /// The default implementation of the ActiveModel
    fn default() -> Self;

    /// Reset the value from [ActiveValue::Unchanged] to [ActiveValue::Set],
    /// leaving [ActiveValue::NotSet] untouched.
    fn reset(&mut self, c: <Self::Entity as EntityTrait>::Column);

    /// Set the Value into an ActiveModel unless the column holds it
    /// [`Unchanged`](ActiveValue::Unchanged), as
    /// [`ActiveValue::set_if_not_equals`] sets a field, so
    /// [`is_changed`](Self::is_changed) then says whether there is anything to
    /// write. A column this model does not carry, or a value of the wrong type
    /// for it, is [`Error::Type`], as [`Self::set`] reports them.
    ///
    /// ```
    /// use pgorm::{entity::*, tests_cfg::cake};
    ///
    /// let mut cake = cake::Model { id: 1, name: "Cheese".to_owned() }.into_active();
    /// cake.set_if_not_equals(cake::Column::Name, "Cheese".into())?;
    /// assert!(!cake.is_changed());
    /// cake.set_if_not_equals(cake::Column::Name, "Apple".into())?;
    /// assert!(cake.is_changed());
    /// # Ok::<(), pgorm::Error>(())
    /// ```
    // [spec:pgorm:req:entity.active-model.set-if-not-equals]
    fn set_if_not_equals(
        &mut self,
        c: <Self::Entity as EntityTrait>::Column,
        v: Value,
    ) -> Result<(), Error> {
        let mut slot = self.get(c)?;
        slot.set_if_not_equals(v);
        match slot {
            ActiveValue::Set(value) => self.set(c, value),
            _ => Ok(()),
        }
    }

    /// Reset all values from [ActiveValue::Unchanged] to [ActiveValue::Set],
    /// leaving [ActiveValue::NotSet] untouched.
    fn reset_all(mut self) -> Self {
        for col in <Self::Entity as EntityTrait>::Column::iter() {
            self.reset(col);
        }
        self
    }

    /// Get the primary key of the ActiveModel
    ///
    /// `None` when any key column is [`NotSet`] — and likewise when a
    /// hand-written implementation does not carry one of its entity's key
    /// columns at all, since either way there is no whole key to return.
    fn get_primary_key_value(&self) -> Option<ValueTuple> {
        let arity = <<<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType as PrimaryKeyArity>::ARITY;
        let mut cols = <Self::Entity as EntityTrait>::PrimaryKey::iter();
        let mut values = Vec::with_capacity(arity);
        for _ in 0..arity {
            values.push(self.get(cols.next()?.into_column()).ok()?.into_value()?);
        }
        Some(ValueTuple::from(values))
    }

    /// Perform an `INSERT` operation on the ActiveModel
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use pgorm::{entity::*, error::*, query::*, tests_cfg::cake, DatabasePool};
    /// #
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// let db = pool.get().await?;
    ///
    /// let apple = cake::ActiveModel {
    ///     name: set("Apple Pie"),
    ///     ..Default::default()
    /// };
    ///
    /// // The insert carries `RETURNING` over every column, so the fully
    /// // populated model comes back without a second round-trip.
    /// let apple: cake::Model = apple.insert(&db).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// An `Err` from [`ActiveModelBehavior::before_save`] returns before any SQL
    /// is sent; an `Err` from [`ActiveModelBehavior::after_save`] returns with
    /// the row already inserted. Pass a [`DatabaseTransaction`][txn] as `db` when
    /// the two must stand or fall together.
    ///
    /// [txn]: crate::DatabaseTransaction
    // [spec:pgorm:req:entity.active-model.persistence+3]
    // [spec:pgorm:req:entity.active-model.hooks+1]
    #[doc(alias = "save")]
    async fn insert<'a, C>(self, db: &'a C) -> Result<<Self::Entity as EntityTrait>::Model, Error>
    where
        <Self::Entity as EntityTrait>::Model: IntoActiveModel<Self>,
        Self: ActiveModelBehavior + 'a,
        C: ConnectionTrait,
    {
        self.insert_in(Self::Entity::default(), db).await
    }

    /// [`insert`](ActiveModelTrait::insert) into the table `entity` names,
    /// hooks and all, as [`Insert::one_in`] writes it: for an entity whose
    /// names are computed at run time.
    // [spec:pgorm:req:entity.active-model.persistence+3]
    // [spec:pgorm:req:entity.active-model.hooks+1]
    async fn insert_in<'a, C>(
        self,
        entity: Self::Entity,
        db: &'a C,
    ) -> Result<<Self::Entity as EntityTrait>::Model, Error>
    where
        <Self::Entity as EntityTrait>::Model: IntoActiveModel<Self>,
        Self: ActiveModelBehavior + 'a,
        C: ConnectionTrait,
    {
        let am = ActiveModelBehavior::before_save(self, db, true).await?;
        let model = Insert::one_in(entity, am).exec_returning_model(db).await?;
        Self::after_save(model, db, true).await
    }

    /// Perform the `UPDATE` operation on an ActiveModel
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use pgorm::{entity::*, error::*, query::*, tests_cfg::fruit, DatabasePool};
    /// #
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// let db = pool.get().await?;
    ///
    /// let orange = fruit::ActiveModel {
    ///     id: set(1),
    ///     name: set("Orange"),
    ///     ..Default::default()
    /// };
    ///
    /// // Fails with `Error::RecordNotFound` when the primary key matches no row.
    /// let orange: fruit::Model = orange.update(&db).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// An `Err` from [`ActiveModelBehavior::before_save`] returns before any SQL
    /// is sent; an `Err` from [`ActiveModelBehavior::after_save`] returns with
    /// the row already updated. Pass a [`DatabaseTransaction`][txn] as `db` when
    /// the two must stand or fall together.
    ///
    /// [txn]: crate::DatabaseTransaction
    // [spec:pgorm:req:entity.active-model.persistence+3]
    // [spec:pgorm:req:entity.active-model.hooks+1]
    #[doc(alias = "save")]
    async fn update<'a, C>(self, db: &'a C) -> Result<<Self::Entity as EntityTrait>::Model, Error>
    where
        <Self::Entity as EntityTrait>::Model: IntoActiveModel<Self>,
        Self: ActiveModelBehavior + 'a,
        C: ConnectionTrait,
    {
        self.update_in(Self::Entity::default(), db).await
    }

    /// [`update`](ActiveModelTrait::update) of a row of the table `entity`
    /// names, hooks and all, as [`Update::one_in`] writes it.
    // [spec:pgorm:req:entity.active-model.persistence+3]
    // [spec:pgorm:req:entity.active-model.hooks+1]
    async fn update_in<'a, C>(
        self,
        entity: Self::Entity,
        db: &'a C,
    ) -> Result<<Self::Entity as EntityTrait>::Model, Error>
    where
        <Self::Entity as EntityTrait>::Model: IntoActiveModel<Self>,
        Self: ActiveModelBehavior + 'a,
        C: ConnectionTrait,
    {
        let am = ActiveModelBehavior::before_save(self, db, false).await?;
        let model: <Self::Entity as EntityTrait>::Model =
            Update::one_in(entity, am)?.exec_returning_model(db).await?;
        Self::after_save(model, db, false).await
    }

    /// Delete an active model by its primary key
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use pgorm::{entity::*, error::*, query::*, tests_cfg::fruit, DatabasePool};
    /// #
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// let db = pool.get().await?;
    ///
    /// let orange = fruit::ActiveModel {
    ///     id: set(3),
    ///     ..Default::default()
    /// };
    ///
    /// let rows_affected: u64 = orange.delete(&db).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// An `Err` from [`ActiveModelBehavior::before_delete`] returns before any
    /// SQL is sent; an `Err` from [`ActiveModelBehavior::after_delete`] returns
    /// with the row already gone. Pass a [`DatabaseTransaction`][txn] as `db`
    /// when the two must stand or fall together.
    ///
    /// [txn]: crate::DatabaseTransaction
    // [spec:pgorm:req:entity.active-model.hooks+1]
    async fn delete<'a, C>(self, db: &'a C) -> Result<u64, Error>
    where
        Self: ActiveModelBehavior + 'a,
        C: ConnectionTrait,
    {
        self.delete_in(Self::Entity::default(), db).await
    }

    /// [`delete`](ActiveModelTrait::delete) from the table `entity` names,
    /// hooks and all, as [`Delete::one_in`] deletes from it.
    // [spec:pgorm:req:entity.active-model.hooks+1]
    async fn delete_in<'a, C>(self, entity: Self::Entity, db: &'a C) -> Result<u64, Error>
    where
        Self: ActiveModelBehavior + 'a,
        C: ConnectionTrait,
    {
        let am = ActiveModelBehavior::before_delete(self, db).await?;
        let am_clone = am.clone();
        let delete_res = Delete::one_in(entity, am)?.exec(db).await?;
        ActiveModelBehavior::after_delete(am_clone, db).await?;
        Ok(delete_res)
    }

    /// Set the corresponding attributes in the ActiveModel from a JSON value
    ///
    /// The primary-key values in the ActiveModel are not altered, and neither is
    /// anything else when the JSON does not parse: the value is converted into a
    /// whole new ActiveModel first, and only a conversion that succeeded reaches
    /// `self`.
    // [spec:pgorm:req:entity.active-model.json+4]
    #[cfg(feature = "with-json")]
    fn set_from_json(&mut self, json: serde_json::Value) -> Result<(), Error>
    where
        <<Self as ActiveModelTrait>::Entity as EntityTrait>::Model: IntoActiveModel<Self>,
        for<'de> <<Self as ActiveModelTrait>::Entity as EntityTrait>::Model:
            serde::de::Deserialize<'de>,
    {
        use crate::Iterable;

        // Convert first: a failure here must leave `self` as it was.
        let mut incoming = Self::from_json(json)?;

        // Carry this model's primary key over to the replacement
        for pk in <<Self::Entity as EntityTrait>::PrimaryKey>::iter() {
            let col = pk.into_column();
            match self.get(col)? {
                ActiveValue::Unchanged(v) | ActiveValue::Set(v) => incoming.set(col, v)?,
                NotSet => incoming.not_set(col),
            }
        }

        *self = incoming;

        Ok(())
    }

    /// Create ActiveModel from a JSON value
    // [spec:pgorm:req:entity.active-model.json+4]
    #[cfg(feature = "with-json")]
    fn from_json(json: serde_json::Value) -> Result<Self, Error>
    where
        <<Self as ActiveModelTrait>::Entity as EntityTrait>::Model: IntoActiveModel<Self>,
        for<'de> <<Self as ActiveModelTrait>::Entity as EntityTrait>::Model:
            serde::de::Deserialize<'de>,
    {
        use crate::{ColumnTrait, Iterable};

        // Mark down which attribute exists in the JSON object, keyed the way
        // the deserialization below reads it
        let json_keys: Vec<(<Self::Entity as EntityTrait>::Column, bool)> =
            <<Self::Entity as EntityTrait>::Column>::iter()
                .map(|col| {
                    let present = json.get(col.json_key()).is_some();
                    (col, present)
                })
                .collect();

        // Convert JSON object into ActiveModel via Model
        let model: <Self::Entity as EntityTrait>::Model =
            serde_json::from_value(json).map_err(json_err)?;
        let mut am = model.into_active_model();

        // Transform attribute that exists in JSON object into ActiveValue::Set, otherwise ActiveValue::NotSet
        for (col, json_key_exists) in json_keys {
            match (json_key_exists, am.get(col)?) {
                (true, ActiveValue::Set(value) | ActiveValue::Unchanged(value)) => {
                    am.set(col, value)?;
                }
                _ => {
                    am.not_set(col);
                }
            }
        }

        Ok(am)
    }

    /// Return `true` if any attribute of `ActiveModel` is `Set`
    ///
    /// A column the model does not carry holds no value and so changes
    /// nothing, which is the answer an [`Error::Type`] from [`Self::get`]
    /// counts as here.
    fn is_changed(&self) -> bool {
        <Self::Entity as EntityTrait>::Column::iter().any(|col| {
            self.get(col)
                .is_ok_and(|value| value.is_set() && !value.is_unchanged())
        })
    }
}

/// A Trait for overriding the ActiveModel behavior
///
/// ### Example
/// ```
/// # #[cfg(feature = "macros")]
/// # {
/// use pgorm::entity::prelude::*;
///
/// // `#[derive(Entity)]` writes the Entity, Column, PrimaryKey and ActiveModel,
/// // and leaves the behaviour to this module where the model says `hooks`
/// #[derive(Clone, Debug, PartialEq, Eq, Entity)]
/// #[pgorm(table_name = "cake", hooks)]
/// pub struct Model {
///     #[pgorm(primary_key)]
///     pub id: i32,
///     pub name: String,
/// }
///
/// #[derive(Copy, Clone, Debug, Relation)]
/// pub enum Relation {}
///
/// // Every hook has a no-op default; override only what you need
/// #[async_trait::async_trait]
/// impl ActiveModelBehavior for ActiveModel {
///     async fn before_save<C>(mut self, _db: &C, _insert: bool) -> Result<Self, Error>
///     where
///         C: ConnectionTrait,
///     {
///         if let ActiveValue::Set(name) = &self.name {
///             self.name = set(name.trim());
///         }
///         Ok(self)
///     }
/// }
/// # }
/// ```
///
/// Without `hooks`, the `Entity` derive implements this trait with every
/// default.
///
/// # What a hook error does
///
/// A `before` hook runs ahead of the statement, so its `Err` aborts the write
/// outright — nothing reaches the database.
///
/// An `after` hook runs once the statement has already executed. Its `Err` is
/// returned to the caller, but the row stays written: the operation opens no
/// transaction of its own, and pgorm will not silently start one around a write
/// the caller asked for on a plain connection. An `after` hook is therefore the
/// place for work that may fail *without* invalidating the write — auditing,
/// cache invalidation, publishing an event. When the hook and the write must
/// stand or fall together, run the operation inside a transaction the caller
/// owns and let the `Err` reach the rollback:
///
/// ```no_run
/// # use pgorm::{entity::*, error::*, query::*, tests_cfg::cake, DatabasePool, TransactionError, TransactionTrait};
/// #
/// # async fn example(pool: &DatabasePool) -> Result<(), TransactionError<Error>> {
/// let mut db = pool.get().await.map_err(TransactionError::Connection)?;
///
/// // An `Err` from `after_save` propagates out of the closure, so the
/// // transaction rolls back and the insert with it.
/// let apple = db
///     .transaction(async |txn| {
///         cake::ActiveModel {
///             name: set("Apple Pie"),
///             ..Default::default()
///         }
///         .insert(&txn)
///         .await
///     })
///     .await?;
/// # Ok(())
/// # }
/// ```
///
/// See module level docs [crate::entity] for a full example
// [spec:pgorm:req:entity.active-model.hooks+1]
#[allow(unused_variables)]
#[async_trait]
pub trait ActiveModelBehavior: ActiveModelTrait {
    /// Create a new ActiveModel with default values. Also used by `Default::default()`.
    fn new() -> Self {
        <Self as ActiveModelTrait>::default()
    }

    /// Will be called before `ActiveModel::insert` (`insert: true`) and
    /// `ActiveModel::update` (`insert: false`). An `Err` aborts the write before
    /// any SQL is sent.
    async fn before_save<C>(self, db: &C, insert: bool) -> Result<Self, Error>
    where
        C: ConnectionTrait,
    {
        Ok(self)
    }

    /// Will be called after `ActiveModel::insert` (`insert: true`) and
    /// `ActiveModel::update` (`insert: false`). An `Err` is returned to the
    /// caller but does not undo the row already written.
    async fn after_save<C>(
        model: <Self::Entity as EntityTrait>::Model,
        db: &C,
        insert: bool,
    ) -> Result<<Self::Entity as EntityTrait>::Model, Error>
    where
        C: ConnectionTrait,
    {
        Ok(model)
    }

    /// Will be called before `ActiveModel::delete`. An `Err` aborts the delete
    /// before any SQL is sent.
    async fn before_delete<C>(self, db: &C) -> Result<Self, Error>
    where
        C: ConnectionTrait,
    {
        Ok(self)
    }

    /// Will be called after `ActiveModel::delete`. An `Err` is returned to the
    /// caller but does not bring the deleted row back.
    async fn after_delete<C>(self, db: &C) -> Result<Self, Error>
    where
        C: ConnectionTrait,
    {
        Ok(self)
    }
}

/// A Trait for any type that can be converted into an ActiveModel
// [spec:pgorm:req:entity.active-model.into+3]
pub trait IntoActiveModel<A>
where
    A: ActiveModelTrait,
{
    /// Method to call to perform the conversion
    fn into_active_model(self) -> A;
}

impl<A> IntoActiveModel<A> for A
where
    A: ActiveModelTrait,
{
    fn into_active_model(self) -> A {
        self
    }
}

/// Any type that can be converted into an [ActiveValue]
// [spec:pgorm:req:entity.active-model.into+3]
pub trait IntoActiveValue<V>
where
    V: Into<Value>,
{
    /// Method to perform the conversion
    fn into_active_value(self) -> ActiveValue<V>;
}

impl<V> IntoActiveValue<Option<V>> for Option<V>
where
    V: IntoActiveValue<V> + Into<Value> + Nullable,
{
    fn into_active_value(self) -> ActiveValue<Option<V>> {
        match self {
            Some(value) => ActiveValue::Set(Some(value)),
            None => NotSet,
        }
    }
}

impl<V> IntoActiveValue<Option<V>> for Option<Option<V>>
where
    V: IntoActiveValue<V> + Into<Value> + Nullable,
{
    fn into_active_value(self) -> ActiveValue<Option<V>> {
        match self {
            Some(value) => ActiveValue::Set(value),
            None => NotSet,
        }
    }
}

macro_rules! impl_into_active_value {
    ($ty: ty) => {
        impl IntoActiveValue<$ty> for $ty {
            fn into_active_value(self) -> ActiveValue<$ty> {
                ActiveValue::Set(self)
            }
        }
    };
}

impl_into_active_value!(bool);
impl_into_active_value!(i8);
impl_into_active_value!(i16);
impl_into_active_value!(i32);
impl_into_active_value!(i64);
impl_into_active_value!(u32);
impl_into_active_value!(u64);
impl_into_active_value!(f32);
impl_into_active_value!(f64);
impl_into_active_value!(&'static str);
impl_into_active_value!(String);
impl_into_active_value!(Vec<u8>);

#[cfg(feature = "with-json")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-json")))]
impl_into_active_value!(crate::prelude::Json);

#[cfg(feature = "with-jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-jiff")))]
impl_into_active_value!(crate::prelude::Date);

#[cfg(feature = "with-jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-jiff")))]
impl_into_active_value!(crate::prelude::Time);

#[cfg(feature = "with-jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-jiff")))]
impl_into_active_value!(crate::prelude::DateTime);

#[cfg(feature = "with-jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-jiff")))]
impl_into_active_value!(crate::prelude::DateTimeWithTimeZone);

impl_into_active_value!(crate::prelude::Decimal);

// [spec:pgorm:req:exec.decode.interval+1]
impl_into_active_value!(crate::prelude::Interval);

#[cfg(feature = "with-uuid")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-uuid")))]
impl_into_active_value!(crate::prelude::Uuid);

// [spec:pgorm:def:sql.value.range+5]
impl_into_active_value!(crate::prelude::Range<i32>);
impl_into_active_value!(crate::prelude::Range<i64>);
impl_into_active_value!(crate::prelude::Range<crate::prelude::Decimal>);
impl_into_active_value!(crate::prelude::Multirange<i32>);
impl_into_active_value!(crate::prelude::Multirange<i64>);
impl_into_active_value!(crate::prelude::Multirange<crate::prelude::Decimal>);

#[cfg(feature = "with-jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-jiff")))]
impl_into_active_value!(crate::prelude::Range<crate::prelude::Date>);

#[cfg(feature = "with-jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-jiff")))]
impl_into_active_value!(crate::prelude::Range<crate::prelude::DateTime>);

#[cfg(feature = "with-jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-jiff")))]
impl_into_active_value!(crate::prelude::Range<crate::prelude::DateTimeWithTimeZone>);

#[cfg(feature = "with-jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-jiff")))]
impl_into_active_value!(crate::prelude::Multirange<crate::prelude::Date>);

#[cfg(feature = "with-jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-jiff")))]
impl_into_active_value!(crate::prelude::Multirange<crate::prelude::DateTime>);

#[cfg(feature = "with-jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "with-jiff")))]
impl_into_active_value!(crate::prelude::Multirange<crate::prelude::DateTimeWithTimeZone>);

impl<V> Default for ActiveValue<V>
where
    V: Into<Value>,
{
    /// Create an [ActiveValue::NotSet]
    fn default() -> Self {
        Self::NotSet
    }
}

// [spec:pgorm:sem:entity.active-model.active-value.ops]
impl<V> ActiveValue<V>
where
    V: Into<Value>,
{
    /// Create an [ActiveValue::Set]
    pub fn set(value: V) -> Self {
        Self::Set(value)
    }

    /// Check if the [ActiveValue] is [ActiveValue::Set]
    pub fn is_set(&self) -> bool {
        matches!(self, Self::Set(_))
    }

    /// Create an [ActiveValue::Unchanged]
    pub fn unchanged(value: V) -> Self {
        Self::Unchanged(value)
    }

    /// Check if the [ActiveValue] is [ActiveValue::Unchanged]
    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged(_))
    }

    /// Create an [ActiveValue::NotSet]
    pub fn not_set() -> Self {
        Self::default()
    }

    /// Check if the [ActiveValue] is [ActiveValue::NotSet]
    pub fn is_not_set(&self) -> bool {
        matches!(self, Self::NotSet)
    }

    /// Remove the value this [ActiveValue] holds, leaving
    /// [ActiveValue::NotSet] in its place, and return it — `None` when there
    /// was none to remove.
    pub fn take(&mut self) -> Option<V> {
        match std::mem::take(self) {
            ActiveValue::Set(value) | ActiveValue::Unchanged(value) => Some(value),
            ActiveValue::NotSet => None,
        }
    }

    /// Get an owned value of the [ActiveValue]
    ///
    /// # Panics
    ///
    /// Panics if it is [ActiveValue::NotSet]
    pub fn unwrap(self) -> V {
        match self {
            ActiveValue::Set(value) | ActiveValue::Unchanged(value) => value,
            ActiveValue::NotSet => panic!("Cannot unwrap ActiveValue::NotSet"),
        }
    }

    /// Consume the [ActiveValue] and return the [Value] it held, or `None` for
    /// [ActiveValue::NotSet].
    pub fn into_value(self) -> Option<Value> {
        match self {
            ActiveValue::Set(value) | ActiveValue::Unchanged(value) => Some(value.into()),
            ActiveValue::NotSet => None,
        }
    }

    /// Wrap the [Value] into a `ActiveValue<Value>`
    pub fn into_wrapped_value(self) -> ActiveValue<Value> {
        match self {
            Self::Set(value) => ActiveValue::set(value.into()),
            Self::Unchanged(value) => ActiveValue::unchanged(value.into()),
            Self::NotSet => ActiveValue::not_set(),
        }
    }

    /// Reset the value from [ActiveValue::Unchanged] to [ActiveValue::Set],
    /// leaving [ActiveValue::NotSet] untouched.
    pub fn reset(&mut self) {
        *self = match self.take() {
            Some(value) => ActiveValue::Set(value),
            None => ActiveValue::NotSet,
        };
    }

    /// `Set(value)`, except when [`self.is_unchanged()`][ActiveValue#method.is_unchanged]
    /// and `value` equals the current [Unchanged][ActiveValue::Unchanged] value.
    ///
    /// This is useful when you have an [Unchanged][ActiveValue::Unchanged] value from the database,
    /// then update it using this method,
    /// and then use [`.is_unchanged()`][ActiveValue#method.is_unchanged] to see whether it has *actually* changed.
    ///
    /// The same nice effect applies to the entire `ActiveModel`.
    /// You can now meaningfully use [ActiveModelTrait::is_changed][ActiveModelTrait#method.is_changed]
    /// to see whether are any changes that need to be saved to the database.
    ///
    /// ## Examples
    ///
    /// ```
    /// # use pgorm::ActiveValue;
    /// #
    /// let mut value = ActiveValue::Unchanged("old");
    ///
    /// // This wouldn't be the case if we used plain `value = Set("old");`
    /// value.set_if_not_equals("old");
    /// assert!(value.is_unchanged());
    ///
    /// // Only when we change the actual `&str` value, it becomes `Set`
    /// value.set_if_not_equals("new");
    /// assert_eq!(value.is_unchanged(), false);
    /// assert_eq!(value, ActiveValue::Set("new"));
    /// ```
    pub fn set_if_not_equals(&mut self, value: V)
    where
        V: PartialEq,
    {
        match self {
            ActiveValue::Unchanged(current) if &value == current => {}
            _ => *self = ActiveValue::Set(value),
        }
    }

    /// Get the inner value, unless `self` is [NotSet][ActiveValue::NotSet].
    ///
    /// There's also a panicking version: [ActiveValue::as_ref].
    ///
    /// ## Examples
    ///
    /// ```
    /// # use pgorm::ActiveValue;
    /// #
    /// assert_eq!(ActiveValue::Unchanged(42).try_as_ref(), Some(&42));
    /// assert_eq!(ActiveValue::Set(42).try_as_ref(), Some(&42));
    /// assert_eq!(ActiveValue::NotSet.try_as_ref(), None::<&i32>);
    /// ```
    pub fn try_as_ref(&self) -> Option<&V> {
        match self {
            ActiveValue::Set(value) | ActiveValue::Unchanged(value) => Some(value),
            ActiveValue::NotSet => None,
        }
    }
}

// [spec:pgorm:sem:entity.active-model.active-value.ops]
impl<V> std::convert::AsRef<V> for ActiveValue<V>
where
    V: Into<Value>,
{
    /// # Panics
    ///
    /// Panics if it is [ActiveValue::NotSet].
    ///
    /// See [ActiveValue::try_as_ref] for a fallible non-panicking version.
    fn as_ref(&self) -> &V {
        match self {
            ActiveValue::Set(value) | ActiveValue::Unchanged(value) => value,
            ActiveValue::NotSet => panic!("Cannot borrow ActiveValue::NotSet"),
        }
    }
}

impl<V> PartialEq for ActiveValue<V>
where
    V: Into<Value> + std::cmp::PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ActiveValue::Set(l), ActiveValue::Set(r)) => l == r,
            (ActiveValue::Unchanged(l), ActiveValue::Unchanged(r)) => l == r,
            (ActiveValue::NotSet, ActiveValue::NotSet) => true,
            _ => false,
        }
    }
}

// [spec:pgorm:req:entity.active-model.from-sugar+1]
impl<V> From<ActiveValue<V>> for ActiveValue<Option<V>>
where
    V: Into<Value> + Nullable,
{
    fn from(value: ActiveValue<V>) -> Self {
        match value {
            ActiveValue::Set(value) => ActiveValue::set(Some(value)),
            ActiveValue::Unchanged(value) => ActiveValue::unchanged(Some(value)),
            ActiveValue::NotSet => ActiveValue::not_set(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ActiveValue::{NotSet, Unchanged};
    use crate::{Error, entity::*, tests_cfg::*};
    use pretty_assertions::assert_eq;

    #[cfg(feature = "with-json")]
    use serde_json::json;

    #[test]
    #[cfg(feature = "macros")]
    fn test_derive_into_active_model_1() {
        mod my_fruit {
            pub use super::fruit::*;
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(IntoActiveModel)]
            pub struct NewFruit {
                // id is omitted
                pub name: String,
                // it is required as opposed to optional in Model
                pub cake_id: i32,
            }
        }

        assert_eq!(
            my_fruit::NewFruit {
                name: "Apple".to_owned(),
                cake_id: 1,
            }
            .into_active_model(),
            fruit::ActiveModel {
                id: NotSet,
                name: set("Apple"),
                cake_id: set(Some(1)),
            }
        );
    }

    #[test]
    #[cfg(feature = "macros")]
    fn test_derive_into_active_model_2() {
        mod my_fruit {
            pub use super::fruit::*;
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(IntoActiveModel)]
            pub struct UpdateFruit {
                pub cake_id: Option<Option<i32>>,
            }
        }

        assert_eq!(
            my_fruit::UpdateFruit {
                cake_id: Some(Some(1)),
            }
            .into_active_model(),
            fruit::ActiveModel {
                id: NotSet,
                name: NotSet,
                cake_id: set(Some(1)),
            }
        );

        assert_eq!(
            my_fruit::UpdateFruit {
                cake_id: Some(None),
            }
            .into_active_model(),
            fruit::ActiveModel {
                id: NotSet,
                name: NotSet,
                cake_id: set(None),
            }
        );

        assert_eq!(
            my_fruit::UpdateFruit { cake_id: None }.into_active_model(),
            fruit::ActiveModel {
                id: NotSet,
                name: NotSet,
                cake_id: NotSet,
            }
        );
    }

    #[test]
    #[cfg(feature = "macros")]
    fn test_derive_try_into_model_1() {
        mod my_fruit {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Entity)]
            #[pgorm(table_name = "fruit")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                pub name: String,
                pub cake_id: Option<i32>,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }
        assert_eq!(
            my_fruit::ActiveModel {
                id: set(1),
                name: set("Pineapple"),
                cake_id: set(None),
            }
            .try_into_model()
            .unwrap(),
            my_fruit::Model {
                id: 1,
                name: "Pineapple".to_owned(),
                cake_id: None,
            }
        );

        assert_eq!(
            my_fruit::ActiveModel {
                id: set(2),
                name: set("Apple"),
                cake_id: set(Some(1)),
            }
            .try_into_model()
            .unwrap(),
            my_fruit::Model {
                id: 2,
                name: "Apple".to_owned(),
                cake_id: Some(1),
            }
        );

        assert_eq!(
            my_fruit::ActiveModel {
                id: set(1),
                name: NotSet,
                cake_id: set(None),
            }
            .try_into_model(),
            Err(Error::AttrNotSet(String::from("name")))
        );

        assert_eq!(
            my_fruit::ActiveModel {
                id: set(1),
                name: set("Pineapple"),
                cake_id: NotSet,
            }
            .try_into_model(),
            Err(Error::AttrNotSet(String::from("cake_id")))
        );
    }

    #[test]
    #[cfg(feature = "macros")]
    fn test_derive_try_into_model_2() {
        mod my_fruit {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Entity)]
            #[pgorm(table_name = "fruit")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                pub name: String,
                #[pgorm(ignore)]
                pub cake_id: Option<i32>,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }
        assert_eq!(
            my_fruit::ActiveModel {
                id: set(1),
                name: set("Pineapple"),
            }
            .try_into_model()
            .unwrap(),
            my_fruit::Model {
                id: 1,
                name: "Pineapple".to_owned(),
                cake_id: None,
            }
        );
    }

    #[test]
    #[cfg(feature = "macros")]
    fn test_derive_try_into_model_3() {
        mod my_fruit {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Entity)]
            #[pgorm(table_name = "fruit")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                #[pgorm(ignore)]
                pub name: String,
                pub cake_id: Option<i32>,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }
        assert_eq!(
            my_fruit::ActiveModel {
                id: set(1),
                cake_id: set(Some(1)),
            }
            .try_into_model()
            .unwrap(),
            my_fruit::Model {
                id: 1,
                name: "".to_owned(),
                cake_id: Some(1),
            }
        );
    }

    #[test]
    #[cfg(feature = "with-json")]
    #[should_panic(
        expected = r#"called `Result::unwrap()` on an `Err` value: Json("missing field `id`")"#
    )]
    fn test_active_model_set_from_json_1() {
        let mut cake: cake::ActiveModel = Default::default();

        cake.set_from_json(json!({
            "name": "Apple Pie",
        }))
        .unwrap();
    }

    #[test]
    #[cfg(feature = "with-json")]
    fn test_active_model_set_from_json_2() -> Result<(), Error> {
        let mut fruit: fruit::ActiveModel = Default::default();

        fruit.set_from_json(json!({
            "name": "Apple",
        }))?;
        assert_eq!(
            fruit,
            fruit::ActiveModel {
                id: ActiveValue::NotSet,
                name: set("Apple"),
                cake_id: ActiveValue::NotSet,
            }
        );

        assert_eq!(
            fruit::ActiveModel::from_json(json!({
                "name": "Apple",
            }))?,
            fruit::ActiveModel {
                id: ActiveValue::NotSet,
                name: set("Apple"),
                cake_id: ActiveValue::NotSet,
            }
        );

        fruit.set_from_json(json!({
            "name": "Apple",
            "cake_id": null,
        }))?;
        assert_eq!(
            fruit,
            fruit::ActiveModel {
                id: ActiveValue::NotSet,
                name: set("Apple"),
                cake_id: set(None),
            }
        );

        fruit.set_from_json(json!({
            "id": null,
            "name": "Apple",
            "cake_id": 1,
        }))?;
        assert_eq!(
            fruit,
            fruit::ActiveModel {
                id: ActiveValue::NotSet,
                name: set("Apple"),
                cake_id: set(Some(1)),
            }
        );

        fruit.set_from_json(json!({
            "id": 2,
            "name": "Apple",
            "cake_id": 1,
        }))?;
        assert_eq!(
            fruit,
            fruit::ActiveModel {
                id: ActiveValue::NotSet,
                name: set("Apple"),
                cake_id: set(Some(1)),
            }
        );

        let mut fruit = fruit::ActiveModel {
            id: set(1),
            name: ActiveValue::NotSet,
            cake_id: ActiveValue::NotSet,
        };
        fruit.set_from_json(json!({
            "id": 8,
            "name": "Apple",
            "cake_id": 1,
        }))?;
        assert_eq!(
            fruit,
            fruit::ActiveModel {
                id: set(1),
                name: set("Apple"),
                cake_id: set(Some(1)),
            }
        );

        Ok(())
    }

    #[test]
    fn test_active_model_is_changed() {
        let mut fruit: fruit::ActiveModel = Default::default();
        assert!(!fruit.is_changed());

        fruit
            .set(fruit::Column::Name, "apple".into())
            .expect("Name is a column of fruit::ActiveModel");
        assert!(fruit.is_changed());
    }

    #[test]
    fn test_reset_1() {
        assert_eq!(
            fruit::Model {
                id: 1,
                name: "Apple".into(),
                cake_id: None,
            }
            .into_active_model(),
            fruit::ActiveModel {
                id: Unchanged(1),
                name: Unchanged("Apple".into()),
                cake_id: Unchanged(None)
            },
        );

        assert_eq!(
            fruit::Model {
                id: 1,
                name: "Apple".into(),
                cake_id: None,
            }
            .into_active_model()
            .reset_all(),
            fruit::ActiveModel {
                id: set(1),
                name: set("Apple"),
                cake_id: set(None)
            },
        );

        assert_eq!(
            fruit::Model {
                id: 1,
                name: "Apple".into(),
                cake_id: Some(2),
            }
            .into_active_model(),
            fruit::ActiveModel {
                id: Unchanged(1),
                name: Unchanged("Apple".into()),
                cake_id: Unchanged(Some(2)),
            },
        );

        assert_eq!(
            fruit::Model {
                id: 1,
                name: "Apple".into(),
                cake_id: Some(2),
            }
            .into_active_model()
            .reset_all(),
            fruit::ActiveModel {
                id: set(1),
                name: set("Apple"),
                cake_id: set(Some(2)),
            },
        );
    }
}
