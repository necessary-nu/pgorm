use crate::{
    ConnectionTrait, EntityTrait, Error, Iterable, ModelTrait, OneOfShape, PrimaryKeyToColumn,
    PrimaryKeyTrait, QueryFilter, Related, StaticName, entity::disjoint::table_label,
    error::query_err,
};
use async_trait::async_trait;
use pgorm_query::Value;
use std::{fmt::Debug, future::Future, pin::Pin};

/// The key a target entity's rows are keyed by: what its variant of
/// `<Enum>Key` holds and its column in the referencing entity holds.
pub type TargetKey<T> = <<T as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType;

/// A reference from one entity to exactly one of several others, stored as
/// one nullable column per target, each a foreign key the entity's own
/// `belongs_to` relation declares.
///
/// `#[derive(OneOf)]` implements it on an enum whose variants each name a
/// target entity and the column holding its key, `#[pgorm(column =
/// Column::PostId)] Post(post::Entity)`, and at most one unit variant, a row
/// referencing none of them. It generates `<Enum>Key`, each variant holding
/// its target's key, and `<Enum>Model`, each holding its target's `Model`,
/// and implements [`OneOfTarget`] for each target entity.
///
/// A row's reference reads as its `<Enum>Key` with [`key`](OneOf::key),
/// is written with [`set`](OneOf::set), and is loaded as its `<Enum>Model`
/// with [`load`](OneOf::load). `Schema::create_one_of` writes the `CHECK`
/// that holds every row to one target, or to one at most where the enum has
/// a unit variant.
///
/// ```no_run
/// # use pgorm::{entity::prelude::*, tests_cfg::comment::{self, Commentable, CommentableKey}};
/// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
/// let db = pool.get().await?;
///
/// let comments = comment::Entity::find().all(&db).await?;
/// let targets = Commentable::load(&comments, &db).await?;
///
/// let mut active = comments[0].clone().into_active();
/// Commentable::set(&mut active, CommentableKey::Photo(2))?;
/// active.update(&db).await?;
/// # Ok(())
/// # }
/// ```
// [spec:pgorm:def:disjoint.one-of+4]
#[async_trait]
pub trait OneOf: Sized + Send + Sync + 'static {
    /// The entity whose rows hold the reference.
    type Entity: EntityTrait<Model: Sync>;

    /// `<Enum>Key`: each variant naming a target holds its key, and the unit
    /// variant holds nothing.
    type Key: Clone + Debug + PartialEq + Send + Sync + 'static;

    /// `<Enum>Model`: each variant naming a target holds its `Model`, and the
    /// unit variant holds nothing.
    type Model: Clone + Debug + Send + 'static;

    /// The enum's name in snake_case, `commentable` for `Commentable`, which
    /// names the `CHECK` `Schema::create_one_of` writes.
    fn name() -> &'static str;

    /// The unit variants of `<Enum>Key` and `<Enum>Model`, a row referencing
    /// none of the targets, or `None` where the enum has no unit variant and
    /// every row references one.
    fn none() -> Option<(Self::Key, Self::Model)>;

    /// Call [`TargetVisitor::target`] for each variant naming a target, in
    /// declaration order.
    fn visit_targets<V: TargetVisitor<Self>>(visitor: &mut V);

    /// The entity's columns holding the targets' keys, in declaration order.
    fn columns() -> Vec<<Self::Entity as EntityTrait>::Column> {
        let mut columns = Columns(Vec::new());
        Self::visit_targets(&mut columns);
        columns.0
    }

    /// The reference `model` holds: the variant of `<Enum>Key` whose column
    /// is set, or the unit variant where none is and the enum has one, as
    /// [`OneOfShape::referenced`] decides it.
    ///
    /// A row with two or more columns set, or none where the enum has no
    /// unit variant, contradicts the declaration, and is [`Error::Query`]
    /// naming its table, its key and the columns.
    // [spec:pgorm:def:disjoint.one-of+4]
    fn key(model: &<Self::Entity as EntityTrait>::Model) -> Result<Self::Key, Error> {
        let mut read = Read {
            model,
            keys: Vec::new(),
        };
        Self::visit_targets(&mut read);
        let set: Vec<bool> = read.keys.iter().map(Option::is_some).collect();
        let referenced = OneOfShape::of::<Self>().referenced(&row_key(model), &set)?;
        let key = match referenced {
            Some(index) => read.keys.into_iter().nth(index).flatten(),
            None => Self::none().map(|(key, _)| key),
        };
        key.ok_or_else(|| query_err("a row's reference was read as a variant it does not hold"))
    }

    /// Set `active`'s reference to `key`: its variant's column to its key,
    /// and every other column to null; for the unit variant, every column to
    /// null, as [`OneOfShape::assigned`] assigns them.
    ///
    /// An `ActiveModel` refusing a column is its [`ActiveModelTrait::set`](crate::ActiveModelTrait::set)'s
    /// error.
    // [spec:pgorm:def:disjoint.one-of+4]
    fn set(
        active: &mut <Self::Entity as EntityTrait>::ActiveModel,
        key: Self::Key,
    ) -> Result<(), Error> {
        let mut variant = Variant {
            key: &key,
            index: 0,
            found: None,
        };
        Self::visit_targets(&mut variant);
        let assigned = OneOfShape::of::<Self>().assigned(variant.found)?;
        let mut write = Write {
            active,
            key: &key,
            assigned: &assigned,
            index: 0,
            written: Ok(()),
        };
        Self::visit_targets(&mut write);
        write.written
    }

    /// The targets `models` reference, each as its `<Enum>Model`, in the
    /// order of `models`.
    ///
    /// Each target table is read once, in one statement selecting the rows
    /// whose key is among those its variant holds,
    /// [`OneOfShape::load_condition`], and each row read is decoded as the
    /// target's `Model`. The entity's relation to each target is the foreign
    /// key holding the reference, so one that joins anything but exactly the
    /// variant's column to the target's key, or that carries an
    /// `on_condition`, is refused as [`Error::Type`] before anything is
    /// sent. A row's reference is read as [`key`](Self::key) reads it, and
    /// its errors are this one's; a key its target table does not hold is
    /// [`OneOfShape::absent`]'s [`Error::Query`].
    // [spec:pgorm:def:disjoint.one-of+4]
    async fn load<C>(
        models: &[<Self::Entity as EntityTrait>::Model],
        db: &C,
    ) -> Result<Vec<Self::Model>, Error>
    where
        C: ConnectionTrait,
    {
        let keys = models
            .iter()
            .map(Self::key)
            .collect::<Result<Vec<_>, _>>()?;
        let shape = OneOfShape::of::<Self>();
        let loads = {
            let mut load = Load::<Self, C> {
                shape: &shape,
                models,
                keys: &keys,
                db,
                index: 0,
                loads: Vec::new(),
                refused: Ok(()),
            };
            Self::visit_targets(&mut load);
            load.refused?;
            load.loads
        };
        let mut loaded: Vec<Option<Self::Model>> = keys.iter().map(|_| None).collect();
        for target in loads {
            for (row, model) in target.await? {
                if let Some(slot) = loaded.get_mut(row) {
                    *slot = Some(model);
                }
            }
        }
        loaded
            .into_iter()
            .map(|model| match model {
                Some(model) => Ok(model),
                None => Self::none().map(|(_, none)| none).ok_or_else(|| {
                    query_err("a row whose reference names no target was read as one")
                }),
            })
            .collect()
    }
}

/// What [`OneOf::visit_targets`] calls for each variant naming a target.
// [spec:pgorm:def:disjoint.one-of+4]
pub trait TargetVisitor<O: OneOf> {
    /// The variant whose target is `T`, its key held in
    /// [`OneOfTarget::COLUMN`] and the relation reading it the entity's
    /// `Related<T>`.
    fn target<T: OneOfTarget<O>>(&mut self)
    where
        O::Entity: Related<T>;
}

/// An entity one variant of `O` names as its target: the column of `O`'s
/// entity holding its key, and the variants of `<Enum>Key` and
/// `<Enum>Model` holding its key and its `Model`.
///
/// `#[derive(OneOf)]` implements it, once for each target, so two variants
/// naming one entity are two impls of this trait for one type, which
/// coherence refuses: loading reads a target through the entity's one
/// relation to it.
///
/// ```compile_fail,E0119
/// # use pgorm::{entity::prelude::*, tests_cfg::post};
/// pub mod pinned {
///     use pgorm::entity::prelude::*;
///
///     #[derive(Clone, Debug, PartialEq, Entity)]
///     #[pgorm(table_name = "pinned")]
///     pub struct Model {
///         #[pgorm(primary_key)]
///         pub id: i32,
///         pub post_id: Option<i32>,
///         pub draft_id: Option<i32>,
///     }
///
///     #[derive(Copy, Clone, Debug, Relation)]
///     pub enum Relation {
///         #[pgorm(belongs_to = super::post::Entity, from = Column::PostId, to = super::post::Column::Id)]
///         Post,
///     }
/// }
///
/// #[derive(OneOf)]
/// #[pgorm(entity = pinned::Entity)]
/// pub enum Pinned {
///     #[pgorm(column = pinned::Column::PostId)]
///     Post(post::Entity),
///     #[pgorm(column = pinned::Column::DraftId)]
///     Draft(post::Entity),
/// }
/// # fn main() {}
/// ```
///
/// Its column is the entity's, so another entity's does not compile,
///
/// ```compile_fail,E0308
/// # use pgorm::{entity::prelude::*, tests_cfg::{comment, fruit, post}};
/// #[derive(OneOf)]
/// #[pgorm(entity = comment::Entity)]
/// pub enum Commentable {
///     #[pgorm(column = fruit::Column::CakeId)]
///     Post(post::Entity),
/// }
/// # fn main() {}
/// ```
///
/// nor does a column that is not nullable,
///
/// ```compile_fail,E0308
/// # use pgorm::{entity::prelude::*, tests_cfg::{cake, fruit}};
/// #[derive(OneOf)]
/// #[pgorm(entity = fruit::Entity)]
/// pub enum OnCake {
///     #[pgorm(column = fruit::Column::Id)]
///     Cake(cake::Entity),
/// }
/// # fn main() {}
/// ```
///
/// one of another type than its target's key,
///
/// ```compile_fail,E0308
/// # use pgorm::{entity::prelude::*, tests_cfg::{comment, post}};
/// #[derive(OneOf)]
/// #[pgorm(entity = comment::Entity)]
/// pub enum Commentable {
///     #[pgorm(column = comment::Column::PhotoId)]
///     Post(post::Entity),
/// }
/// # fn main() {}
/// ```
///
/// one that two variants name,
///
/// ```compile_fail,E0080
/// # use pgorm::{entity::prelude::*, tests_cfg::{cake, post}};
/// pub mod note {
///     use pgorm::entity::prelude::*;
///
///     #[derive(Clone, Debug, PartialEq, Entity)]
///     #[pgorm(table_name = "note")]
///     pub struct Model {
///         #[pgorm(primary_key)]
///         pub id: i32,
///         pub post_id: Option<i32>,
///         pub cake_id: Option<i32>,
///     }
///
///     #[derive(Copy, Clone, Debug, Relation)]
///     pub enum Relation {
///         #[pgorm(belongs_to = super::post::Entity, from = Column::PostId, to = super::post::Column::Id)]
///         Post,
///         #[pgorm(belongs_to = super::cake::Entity, from = Column::CakeId, to = super::cake::Column::Id)]
///         Cake,
///     }
/// }
///
/// #[derive(OneOf)]
/// #[pgorm(entity = note::Entity)]
/// pub enum Noted {
///     #[pgorm(column = note::Column::PostId)]
///     Post(post::Entity),
///     #[pgorm(column = note::Column::PostId)]
///     Cake(cake::Entity),
/// }
/// # fn main() {}
/// ```
///
/// or a target the entity has no relation to.
///
/// ```compile_fail,E0277
/// # use pgorm::{entity::prelude::*, tests_cfg::{cake, comment}};
/// #[derive(OneOf)]
/// #[pgorm(entity = comment::Entity)]
/// pub enum Commentable {
///     #[pgorm(column = comment::Column::PostId)]
///     Cake(cake::Entity),
/// }
/// # fn main() {}
/// ```
// [spec:pgorm:def:disjoint.one-of+4]
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a target of `{O}`",
    label = "a `OneOf` variant names its target entity, `Post(post::Entity)`"
)]
pub trait OneOfTarget<O: OneOf>: EntityTrait<Model: Sync> {
    /// The target's name in `O`: its variant's name in snake_case, which a
    /// [`OneOfShape`](crate::OneOfShape) names it by.
    const NAME: &'static str;

    /// The column of `O`'s entity holding this target's key: nullable, and
    /// of the target's key type, which the derive holds it to.
    const COLUMN: <O::Entity as EntityTrait>::Column;

    /// The key `model`'s column holds, `None` where it is null.
    fn read(model: &<O::Entity as EntityTrait>::Model) -> Option<TargetKey<Self>>;

    /// Set the column in `active` to `key`, null for `None`.
    fn write(
        active: &mut <O::Entity as EntityTrait>::ActiveModel,
        key: Option<&TargetKey<Self>>,
    ) -> Result<(), Error>;

    /// This target's key in `key`, where `key` is its variant.
    fn of_key(key: &O::Key) -> Option<&TargetKey<Self>>;

    /// The variant of `<Enum>Key` holding `key`.
    fn key(key: TargetKey<Self>) -> O::Key;

    /// The variant of `<Enum>Model` holding `model`.
    fn model(model: Self::Model) -> O::Model;
}

/// A row's key values, in its entity's key order.
fn row_key<M: ModelTrait>(model: &M) -> Vec<Value> {
    <<M::Entity as EntityTrait>::PrimaryKey as Iterable>::iter()
        .map(|key| model.get(key.into_column()))
        .collect()
}

struct Columns<O: OneOf>(Vec<<O::Entity as EntityTrait>::Column>);

impl<O: OneOf> TargetVisitor<O> for Columns<O> {
    fn target<T: OneOfTarget<O>>(&mut self)
    where
        O::Entity: Related<T>,
    {
        self.0.push(T::COLUMN);
    }
}

/// Each variant's key a row holds, `None` where its column is null, in
/// declaration order.
struct Read<'a, O: OneOf> {
    model: &'a <O::Entity as EntityTrait>::Model,
    keys: Vec<Option<O::Key>>,
}

impl<O: OneOf> TargetVisitor<O> for Read<'_, O> {
    fn target<T: OneOfTarget<O>>(&mut self)
    where
        O::Entity: Related<T>,
    {
        self.keys.push(T::read(self.model).map(T::key));
    }
}

/// Where in declaration order the variant of `key` stands, `None` for the
/// unit variant.
struct Variant<'a, O: OneOf> {
    key: &'a O::Key,
    index: usize,
    found: Option<usize>,
}

impl<O: OneOf> TargetVisitor<O> for Variant<'_, O> {
    fn target<T: OneOfTarget<O>>(&mut self)
    where
        O::Entity: Related<T>,
    {
        if T::of_key(self.key).is_some() {
            self.found = Some(self.index);
        }
        self.index += 1;
    }
}

/// Each variant's column written: `key` where `assigned` says so, null
/// otherwise, until one is refused.
struct Write<'a, O: OneOf> {
    active: &'a mut <O::Entity as EntityTrait>::ActiveModel,
    key: &'a O::Key,
    assigned: &'a [bool],
    index: usize,
    written: Result<(), Error>,
}

impl<O: OneOf> TargetVisitor<O> for Write<'_, O> {
    fn target<T: OneOfTarget<O>>(&mut self)
    where
        O::Entity: Related<T>,
    {
        let key = match self.assigned.get(self.index) {
            Some(true) => T::of_key(self.key),
            _ => None,
        };
        if self.written.is_ok() {
            self.written = T::write(self.active, key);
        }
        self.index += 1;
    }
}

/// One target table's read: each row referencing it, by its position, with
/// its target's `<Enum>Model`.
type TargetLoad<'a, M> = Pin<Box<dyn Future<Output = Result<Vec<(usize, M)>, Error>> + Send + 'a>>;

/// The reads of a load, one per target some row references, built before
/// any is sent so that a relation refused refuses the whole load.
struct Load<'a, O: OneOf, C> {
    shape: &'a OneOfShape,
    models: &'a [<O::Entity as EntityTrait>::Model],
    keys: &'a [O::Key],
    db: &'a C,
    index: usize,
    loads: Vec<TargetLoad<'a, O::Model>>,
    refused: Result<(), Error>,
}

impl<'a, O: OneOf, C: ConnectionTrait> TargetVisitor<O> for Load<'a, O, C> {
    fn target<T: OneOfTarget<O>>(&mut self)
    where
        O::Entity: Related<T>,
    {
        let index = self.index;
        self.index += 1;
        if self.refused.is_ok() {
            self.refused = reads_column::<O, T>();
        }
        let wanted: Vec<(usize, Value)> = self
            .models
            .iter()
            .zip(self.keys)
            .enumerate()
            .filter(|(_, (_, key))| T::of_key(key).is_some())
            .map(|(row, (model, _))| (row, model.get(T::COLUMN)))
            .collect();
        if wanted.is_empty() || self.refused.is_err() {
            return;
        }
        let condition = match self
            .shape
            .load_condition(index, wanted.iter().map(|(_, key)| key.clone()).collect())
        {
            Ok(condition) => condition,
            Err(error) => {
                self.refused = Err(error);
                return;
            }
        };
        let (shape, models, db) = (self.shape, self.models, self.db);
        self.loads.push(Box::pin(async move {
            let found = T::find().filter(condition).all(db).await?;
            let key = <T::PrimaryKey as Iterable>::iter()
                .next()
                .map(PrimaryKeyToColumn::into_column);
            wanted
                .into_iter()
                .map(|(row, wanted)| {
                    let target =
                        key.and_then(|key| found.iter().find(|target| target.get(key) == wanted));
                    match target {
                        Some(target) => Ok((row, T::model(target.clone()))),
                        None => Err(shape.absent(
                            index,
                            &models.get(row).map(row_key).unwrap_or_default(),
                            &wanted,
                        )),
                    }
                })
                .collect()
        }));
    }
}

/// Refuse a relation to `T` that is not the foreign key holding the
/// reference: one joining other than exactly `T`'s column to `T`'s key, or
/// through a `via` or a period, would hold another reference than the
/// variant's, and an `on_condition` could only make a key its foreign key
/// holds look absent.
// [spec:pgorm:def:disjoint.one-of+4]
fn reads_column<O, T>() -> Result<(), Error>
where
    O: OneOf,
    T: OneOfTarget<O>,
    O::Entity: Related<T>,
{
    let column = T::COLUMN.as_str().to_owned();
    let key: Vec<String> = <T::PrimaryKey as Iterable>::iter()
        .map(|key| key.into_column().as_str().to_owned())
        .collect();
    let relation = <O::Entity as Related<T>>::to();
    let pairs: Vec<(String, String)> = relation
        .columns
        .iter()
        .map(|(from, to)| (from.to_string(), to.to_string()))
        .collect();
    let entity = <O::Entity as Default>::default();
    let target = <T as Default>::default();
    let (table, target_table) = (table_label(&entity), table_label(&target));
    if relation.on_condition.is_some() {
        return Err(Error::Type(format!(
            "`{table}`'s relation to `{target_table}` carries an on_condition, and the \
             foreign key holding `{table}`.`{column}` holds its target row whatever a \
             condition says: the relation a one-of reference's target is loaded by joins \
             `{column}` to `{target_table}`'s key alone"
        )));
    }
    let reads = <O::Entity as Related<T>>::via().is_none()
        && relation.columns.period().is_none()
        && matches!(pairs.as_slice(), [(from, to)] if *from == column && key == [to.clone()]);
    if reads {
        return Ok(());
    }
    Err(Error::Type(format!(
        "`{table}`.`{column}` holds the key of `{target_table}`, but `{table}`'s relation to \
         `{target_table}` joins {joined}, and that relation is the foreign key holding the \
         reference: it must join `{column}` to `{target_table}`'s key `{key}` alone",
        joined = pairs
            .iter()
            .map(|(from, to)| format!("`{from}` to `{to}`"))
            .collect::<Vec<_>>()
            .join(", "),
        key = key.join("`, `"),
    )))
}
