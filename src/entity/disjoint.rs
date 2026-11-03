use crate::{
    ActiveEnum, ColumnDef, DisjointShape, DisjointValues, EntityName, EntityTrait, Error,
    FromQueryResult, IntoPrimaryKey, Iterable, ModelTrait, PrimaryKeyTrait, SelectDisjoint,
    WriteDisjoint,
};
use pgorm_query::IntoValueTuple;
use std::{fmt::Debug, hash::Hash};

/// A Rust enum stored as a base table plus one table for each kind that
/// carries data, keyed by the base row's key.
///
/// Every row of the [`Base`](Disjoint::Base) is a value of the enum. A kind
/// with a table is a variant holding the `Model` of the kind's entity, and a
/// kind without one is a unit variant. A kinded entity, which is also
/// [`KindedDisjoint`], holds each row's kind in the base's
/// [`kind_column`](Disjoint::kind_column), and the constraints
/// `Schema::create_disjoint` writes hold each base row of a kind with a table
/// to exactly one row in that table and each of a kind without one to none.
/// An unkinded entity, declared `unkinded`, has no kind column: a row's kind
/// is the kind whose table has a row for it, nothing holds it to one, and it
/// is read and never written.
///
/// `#[derive(Disjoint)]` implements it, together with `<Enum>Kind`, the
/// `<Enum>ActiveModel` a kinded entity's write takes, and [`KindModel`] for
/// each kind's `Model`. A kind's entity is reached through
/// [`visit_kind`](Disjoint::visit_kind), which hands a [`KindVisitor`] the
/// kind's `Model` as a type, so what is done with it is the visitor's,
/// statically typed: over `Self::Kind::iter()` it visits every kind in
/// declaration order.
///
/// What the types cannot tell, [`check_disjoint`] refuses.
// [spec:pgorm:def:disjoint.declare+5]
// [spec:pgorm:sem:disjoint.unkinded]
pub trait Disjoint: Sized + Send {
    /// The entity each value has a row of.
    type Base: EntityTrait;

    /// `<Enum>Kind`, one value per variant, iterating in declaration order:
    /// for a kinded entity the kind type its kind column holds, a PostgreSQL
    /// enum ([`KindedDisjoint`] makes it an [`ActiveEnum`]); for an unkinded
    /// one an enum no column holds.
    type Kind: Iterable + Copy + Eq + Hash + Debug + Send + Sync + 'static;

    /// The base's column holding each row's kind, and the kind type it
    /// holds, which [`check_disjoint`] holds it to; `None` for an unkinded
    /// entity, whose base has none.
    fn kind_column() -> Option<KindColumn<Self::Base>>;

    /// The name of `kind`, as a disjoint statement and its errors name it:
    /// its value of the kind type, or for an unkinded entity, which has no
    /// kind type, its variant's name in snake_case.
    fn kind_name(kind: Self::Kind) -> &'static str;

    /// The kind of this value: for a kinded entity, the value its base row's
    /// kind column holds.
    fn kind(&self) -> Self::Kind;

    /// Call `visitor` for `kind`: [`KindVisitor::table`] with the `Model` of
    /// the kind's entity for a kind with a table, or [`KindVisitor::unit`]
    /// with the kind's value for one with none.
    fn visit_kind<V: KindVisitor<Self>>(kind: Self::Kind, visitor: V) -> V::Output;

    /// Read every base row with its kind's row, each as the pair
    /// `(base::Model, Self)`, in one statement: the base `LEFT JOIN`ed to each
    /// kind's table on the key.
    ///
    /// The read takes what a [`Select`](crate::Select) of the base takes; see
    /// [`SelectDisjoint`].
    // [spec:pgorm:sem:disjoint.read+1]
    fn find() -> SelectDisjoint<Self> {
        SelectDisjoint::of(Self::Base::find())
    }

    /// [`find`](Self::find) narrowed to the base row of one key, composite
    /// keys included, their parts borrowed as
    /// [`EntityTrait::find_by_id`] takes them.
    // [spec:pgorm:sem:disjoint.read+1]
    fn find_by_id<T>(key: T) -> SelectDisjoint<Self>
    where
        T: IntoPrimaryKey<<<Self::Base as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType>,
    {
        SelectDisjoint::of(Self::Base::find_by_id(key))
    }

    /// Insert a value: the base row from `base`, its kind column holding
    /// `kind`'s kind, and, for a kind with a table, the kind's row from
    /// `kind`'s `ActiveModel`, its key read from the inserted base row, so a
    /// key the database generates reaches it.
    ///
    /// The base's kind column and the kind row's key columns come from
    /// `kind` and from the base row, so a value held in either — `Set` or
    /// `Unchanged` — is refused as [`Error::Type`], as is a declaration
    /// [`check_disjoint`] refuses, and nothing is sent. Every write is a
    /// [`KindedDisjoint`]'s: an unkinded entity has none.
    ///
    /// ```no_run
    /// # use pgorm::{entity::prelude::*, tests_cfg::{car, vehicle::{self, Vehicle, VehicleActiveModel}}};
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// let db = pool.get().await?;
    ///
    /// let (base, car) = Vehicle::insert(
    ///     vehicle::ActiveModel {
    ///         name: set("Mini"),
    ///         ..Default::default()
    ///     },
    ///     VehicleActiveModel::Car(car::ActiveModel {
    ///         seats: set(4_i16),
    ///         ..Default::default()
    ///     }),
    /// )?
    /// .exec_returning_model(&db)
    /// .await?;
    /// assert_eq!(car, Vehicle::Car(car::Model { id: base.id, seats: 4 }));
    /// # Ok(())
    /// # }
    /// ```
    // [spec:pgorm:sem:disjoint.write+1]
    // [spec:pgorm:sem:disjoint.unkinded]
    fn insert(
        base: <Self::Base as EntityTrait>::ActiveModel,
        kind: <Self as KindedDisjoint>::ActiveModel,
    ) -> Result<WriteDisjoint<Self>, Error>
    where
        Self: KindedDisjoint,
    {
        let shape = DisjointShape::of::<Self>();
        shape.check()?;
        let written = kind.kind().to_value();
        let base = DisjointValues::of_active(&base)?;
        let row = kind.visit(KindValues)?;
        shape
            .insert(&base, &written, row.as_ref())
            .map(WriteDisjoint::of)
    }

    /// Update a value: the set columns of the base row from `base` and of its
    /// kind's row from `kind`'s `ActiveModel`, both found by the base's key,
    /// which `base` holds `Set` or `Unchanged` as for
    /// [`Update::one`](crate::Update::one).
    ///
    /// The row's kind is `kind`'s: a row of another kind, and a key no row
    /// has, are [`Error::RecordNotFound`] from
    /// [`exec_returning_model`](WriteDisjoint::exec_returning_model), with
    /// both rows left as they were. Changing the kind is
    /// [`change_kind`](Self::change_kind), so the base's kind column `Set`,
    /// and the kind row's key columns `Set`, are refused as [`Error::Type`],
    /// as is a declaration [`check_disjoint`] refuses, and a `NotSet` key of
    /// the base is [`Error::PrimaryKeyNotSet`]; nothing is sent. With nothing
    /// set in either, the write is a read of the row of that key and kind.
    // [spec:pgorm:sem:disjoint.write+1]
    // [spec:pgorm:sem:disjoint.unkinded]
    fn update(
        base: <Self::Base as EntityTrait>::ActiveModel,
        kind: <Self as KindedDisjoint>::ActiveModel,
    ) -> Result<WriteDisjoint<Self>, Error>
    where
        Self: KindedDisjoint,
    {
        let shape = DisjointShape::of::<Self>();
        shape.check()?;
        let current = kind.kind().to_value();
        let base = DisjointValues::of_active(&base)?;
        let row = kind.visit(KindValues)?;
        shape
            .update(&base, &current, row.as_ref())
            .map(WriteDisjoint::of)
    }

    /// Change the kind of the row of `key` to `kind`'s: delete its row from
    /// every other kind's table, its current kind's among them, insert the
    /// new kind's row from `kind`'s `ActiveModel` where that kind has a table,
    /// and set the base row's kind column.
    ///
    /// A key no row has, and a row already of `kind`'s kind, are
    /// [`Error::RecordNotFound`] from
    /// [`exec_returning_model`](WriteDisjoint::exec_returning_model), with
    /// nothing changed. The new row's key columns come from the base row, so
    /// a value held in one is refused as [`Error::Type`], as is a declaration
    /// [`check_disjoint`] refuses, and nothing is sent.
    // [spec:pgorm:sem:disjoint.write+1]
    // [spec:pgorm:sem:disjoint.unkinded]
    fn change_kind<T>(
        key: T,
        kind: <Self as KindedDisjoint>::ActiveModel,
    ) -> Result<WriteDisjoint<Self>, Error>
    where
        Self: KindedDisjoint,
        T: IntoPrimaryKey<<<Self::Base as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType>,
    {
        let shape = DisjointShape::of::<Self>();
        shape.check()?;
        let new = kind.kind().to_value();
        let key = key
            .into_primary_key()
            .into_value_tuple()
            .into_iter()
            .collect();
        let row = kind.visit(KindValues)?;
        shape
            .change_kind(key, &new, row.as_ref())
            .map(WriteDisjoint::of)
    }
}

/// A kinded entity's kind column: the base's column holding each row's
/// kind, and the kind type it holds.
// [spec:pgorm:def:disjoint.declare+5]
#[derive(Clone, Debug)]
pub struct KindColumn<E: EntityTrait> {
    /// The base's column, `NOT NULL` and of the kind type, which
    /// [`check_disjoint`] holds it to.
    pub column: E::Column,
    /// The kind type, `<Enum>Kind`'s: a PostgreSQL enum of the kinds' values
    /// in declaration order.
    pub kind_type: ColumnDef,
}

/// A disjoint entity whose base holds each row's kind in a kind column, so
/// that the constraints `Schema::create_disjoint` writes hold each base row
/// to its own kind's table: what a write and the schema take.
///
/// `#[derive(Disjoint)]` implements it for every declaration but an
/// `unkinded` one, which nothing in its schema holds to one kind per row, so
/// pgorm reads it and never writes through it: a write to one does not
/// compile,
///
/// ```compile_fail,E0277
/// # use pgorm::{entity::prelude::*, tests_cfg::{shop::{self, Shop}, web_shop}};
/// fn write(site: web_shop::ActiveModel) {
///     let _ = Shop::insert(shop::ActiveModel::default(), site);
/// }
/// ```
///
/// and nor does its schema.
///
/// ```compile_fail,E0277
/// # use pgorm::{Schema, tests_cfg::shop::Shop};
/// let _ = Schema::new().create_disjoint::<Shop>();
/// ```
// [spec:pgorm:def:disjoint.declare+5]
// [spec:pgorm:sem:disjoint.unkinded]
#[diagnostic::on_unimplemented(
    message = "`{Self}` is no `KindedDisjoint`, and an unkinded disjoint entity is read-only",
    label = "writes and `create_disjoint` take a disjoint entity with a kind column",
    note = "nothing in an unkinded entity's schema holds a base row to one kind's table, so pgorm does not write through it"
)]
pub trait KindedDisjoint: Disjoint<Kind: ActiveEnum<Value = String>> {
    /// `<Enum>ActiveModel`, the variants each holding its kind entity's
    /// `ActiveModel`: what a write takes.
    type ActiveModel: DisjointActiveModel<Disjoint = Self>;
}

/// What a `<Enum>ActiveModel`'s kind row is given: its kind entity's
/// `ActiveModel`'s values, or none for a kind without a table.
struct KindValues;

impl<D: Disjoint> ActiveKindVisitor<D> for KindValues {
    type Output = Result<Option<DisjointValues>, Error>;

    fn table<M: KindModel<D>>(
        self,
        _: D::Kind,
        active: <M::Entity as EntityTrait>::ActiveModel,
    ) -> Self::Output {
        DisjointValues::of_active(&active).map(Some)
    }

    fn unit(self, _: D::Kind) -> Self::Output {
        Ok(None)
    }
}

/// The `Model` of a kind's entity: what one variant of `D` holds.
///
/// It is the `Model` of its entity rather than a further model of it, and its
/// entity's primary key is of the base's key types, in the base's order, since
/// a kind's row is keyed by its base row's key. A model meeting neither has no
/// impl, so an enum holding it does not compile; neither does an enum holding
/// one model in two variants, which would be two impls of this trait for one
/// type.
///
/// It decodes from a row, as every entity's `Model` does, and is required to
/// here because a bound on its entity's `Model` is not one on the type
/// itself: a read decodes a kind's joined row through this trait.
// [spec:pgorm:def:disjoint.declare+5]
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not the `Model` of one of `{D}`'s kinds",
    label = "a kind with a table holds its entity's `Model`"
)]
pub trait KindModel<D: Disjoint>:
    ModelTrait<
        Entity: EntityTrait<
            Model = Self,
            PrimaryKey: PrimaryKeyTrait<
                ValueType = <<D::Base as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType,
            >,
        >,
    > + FromQueryResult
{
    /// The kind whose rows this model is.
    const KIND: D::Kind;

    /// The value of `D` holding this model.
    fn into_variant(self) -> D;
}

/// What [`Disjoint::visit_kind`] calls for one kind of `D`.
// [spec:pgorm:def:disjoint.declare+5]
pub trait KindVisitor<D: Disjoint> {
    /// What the visit gives.
    type Output;

    /// A kind with a table, whose rows are `M`s of the entity `M::Entity`.
    fn table<M: KindModel<D>>(self, kind: D::Kind) -> Self::Output;

    /// A kind with no table of its own, given as its one value, whose
    /// [`kind`](Disjoint::kind) it is.
    fn unit(self, value: D) -> Self::Output;
}

/// `<Enum>ActiveModel`: a value of a disjoint entity to write, each kind with
/// a table holding its entity's `ActiveModel`.
// [spec:pgorm:def:disjoint.declare+5]
pub trait DisjointActiveModel: Clone + Debug + Send {
    /// The enum whose `ActiveModel` this is: each names the other, so an
    /// enum has exactly one.
    type Disjoint: KindedDisjoint<ActiveModel = Self>;

    /// The value of the kind column the base row written gets.
    fn kind(&self) -> <Self::Disjoint as Disjoint>::Kind;

    /// Take the value apart: [`ActiveKindVisitor::table`] with the kind
    /// entity's `ActiveModel` for a kind with a table, or
    /// [`ActiveKindVisitor::unit`] for one with none.
    fn visit<V: ActiveKindVisitor<Self::Disjoint>>(self, visitor: V) -> V::Output;
}

/// What [`DisjointActiveModel::visit`] calls with the value it takes apart.
// [spec:pgorm:def:disjoint.declare+5]
pub trait ActiveKindVisitor<D: Disjoint> {
    /// What the visit gives.
    type Output;

    /// A kind with a table, and the `ActiveModel` of its row in
    /// `M::Entity`'s table, whose key is the base row's.
    fn table<M: KindModel<D>>(
        self,
        kind: D::Kind,
        active: <M::Entity as EntityTrait>::ActiveModel,
    ) -> Self::Output;

    /// A kind with no table of its own.
    fn unit(self, kind: D::Kind) -> Self::Output;
}

/// Refuse what a disjoint entity's types cannot tell, as [`Error::Type`]
/// naming the table and column at fault: [`DisjointShape::check`] of
/// [`DisjointShape::of::<D>`](DisjointShape::of).
///
/// The base's kind column is `NOT NULL` and of the kind type, since every base
/// row has one of its kinds, and is no generated column, since a write sets
/// it to the kind it writes. No kind entity's key is generated or filled by
/// the database — drawn from a sequence by `auto_increment`, an identity, a
/// generated column or a default — since a kind's row takes its key from its
/// base row.
/// It is what `Schema::create_disjoint` and every disjoint read and write are
/// to run before anything is sent. Of an unkinded entity it refuses nothing:
/// it has no kind column, and no write fills its kinds' keys.
// [spec:pgorm:def:disjoint.declare+5]
// [spec:pgorm:sem:disjoint.unkinded]
pub fn check_disjoint<D: Disjoint>() -> Result<(), Error> {
    DisjointShape::of::<D>().check()
}

/// An entity's table as an error names it, schema-qualified where it is.
pub(crate) fn table_label<E: EntityName>(entity: &E) -> String {
    match entity.schema_name() {
        Some(schema) => format!("{schema}.{}", entity.table_name()),
        None => entity.table_name().to_owned(),
    }
}
