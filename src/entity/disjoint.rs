use crate::{
    ActiveEnum, ColumnTrait, ColumnType, EntityName, EntityTrait, Error, FromQueryResult,
    IntoPrimaryKey, Iterable, ModelTrait, PrimaryKeyToColumn, PrimaryKeyTrait, SelectDisjoint,
    StaticName, entity::ColumnDefault,
};
use std::{fmt::Debug, hash::Hash};

/// A Rust enum stored as a base table plus one table for each kind that
/// carries data, keyed by the base row's key.
///
/// Every row of the [`Base`](Disjoint::Base) is a value of the enum, its kind
/// in the base's [`kind_column`](Disjoint::kind_column). A kind with a table
/// is a variant holding the `Model` of the kind's entity, and each base row of
/// it has exactly one row in that entity's table; a kind without one is a unit
/// variant, whose base rows have no row anywhere else.
///
/// `#[derive(Disjoint)]` implements it, together with the kind type, the
/// `<Enum>ActiveModel` a write takes, and [`KindModel`] for each kind's
/// `Model`. A kind's entity is reached through [`visit_kind`](Disjoint::visit_kind),
/// which hands a [`KindVisitor`] the kind's `Model` as a type, so what is done
/// with it is the visitor's, statically typed: over `Self::Kind::iter()` it
/// visits every kind in declaration order.
///
/// What the types cannot tell, [`check_disjoint`] refuses.
// [spec:pgorm:def:disjoint.declare+3]
pub trait Disjoint: Sized + Send {
    /// The entity each value has a row of.
    type Base: EntityTrait;

    /// The kind type, `<Enum>Kind`: a PostgreSQL enum with one value per
    /// variant, iterating in declaration order.
    type Kind: ActiveEnum<Value = String> + Copy + Eq + Hash + Debug + Send + Sync + 'static;

    /// `<Enum>ActiveModel`, the variants each holding its kind entity's
    /// `ActiveModel`: what a write takes.
    type ActiveModel: DisjointActiveModel<Disjoint = Self>;

    /// The base's column holding each row's kind: `NOT NULL` and of the kind
    /// type, which [`check_disjoint`] holds it to.
    fn kind_column() -> <Self::Base as EntityTrait>::Column;

    /// The value of the kind column of this value's base row.
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
// [spec:pgorm:def:disjoint.declare+3]
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
// [spec:pgorm:def:disjoint.declare+3]
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
// [spec:pgorm:def:disjoint.declare+3]
pub trait DisjointActiveModel: Clone + Debug + Send {
    /// The enum whose `ActiveModel` this is: each names the other, so an
    /// enum has exactly one.
    type Disjoint: Disjoint<ActiveModel = Self>;

    /// The value of the kind column the base row written gets.
    fn kind(&self) -> <Self::Disjoint as Disjoint>::Kind;

    /// Take the value apart: [`ActiveKindVisitor::table`] with the kind
    /// entity's `ActiveModel` for a kind with a table, or
    /// [`ActiveKindVisitor::unit`] for one with none.
    fn visit<V: ActiveKindVisitor<Self::Disjoint>>(self, visitor: V) -> V::Output;
}

/// What [`DisjointActiveModel::visit`] calls with the value it takes apart.
// [spec:pgorm:def:disjoint.declare+3]
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
/// naming the table and column at fault.
///
/// The base's kind column is `NOT NULL` and of the kind type, since every base
/// row has one of its kinds, and is no generated column, since a write sets
/// it to the kind it writes. No kind entity's key is generated or filled by
/// the database — drawn from a sequence by `auto_increment`, an identity, a
/// generated column or a default — since a kind's row takes its key from its
/// base row.
/// It is what `Schema::create_disjoint` and every disjoint read and write are
/// to run before anything is sent.
// [spec:pgorm:def:disjoint.declare+3]
pub fn check_disjoint<D: Disjoint>() -> Result<(), Error> {
    let base = D::Base::default();
    let column = D::kind_column();
    let def = column.def();
    let kind_type = D::Kind::db_type();
    if def.null {
        return Err(Error::Type(format!(
            "the kind column `{}`.`{}` is nullable: every base row of a disjoint entity has a \
             kind, so the column is `NOT NULL`",
            table_label(&base),
            column.as_str()
        )));
    }
    if matches!(def.default, Some(ColumnDefault::Generated(..))) {
        return Err(Error::Type(format!(
            "the kind column `{}`.`{}` is a generated column: a write sets a base row's kind \
             to the kind it writes, so no expression computes it",
            table_label(&base),
            column.as_str()
        )));
    }
    if !same_enum(def.get_column_type(), kind_type.get_column_type()) {
        return Err(Error::Type(format!(
            "the kind column `{}`.`{}` is {}, not the kind type {}",
            table_label(&base),
            column.as_str(),
            describe(def.get_column_type()),
            describe(kind_type.get_column_type())
        )));
    }
    D::Kind::iter().try_for_each(|kind| D::visit_kind(kind, UngeneratedKey))
}

/// Refuses a kind entity whose key the database generates or fills.
struct UngeneratedKey;

impl<D: Disjoint> KindVisitor<D> for UngeneratedKey {
    type Output = Result<(), Error>;

    fn table<M: KindModel<D>>(self, kind: D::Kind) -> Result<(), Error> {
        type Key<M> = <<M as ModelTrait>::Entity as EntityTrait>::PrimaryKey;
        let entity = M::Entity::default();
        let generated = |column: &<M::Entity as EntityTrait>::Column, how: &str| {
            Error::Type(format!(
                "`{}`.`{}`, the key of the kind `{}`, is {how}: a kind's row takes its key from \
                 its base row, so the database does not generate it",
                table_label(&entity),
                column.as_str(),
                kind.to_value()
            ))
        };
        for key in Key::<M>::iter() {
            let column = key.into_column();
            match column.def().default {
                Some(ColumnDefault::Identity(_)) => return Err(generated(&column, "an identity")),
                Some(ColumnDefault::Generated(..)) => {
                    return Err(generated(&column, "a generated column"));
                }
                Some(ColumnDefault::Expr(_)) => return Err(generated(&column, "given a default")),
                None => {}
            }
        }
        match (Key::<M>::auto_increment(), Key::<M>::iter().next()) {
            (true, Some(key)) => Err(generated(
                &key.into_column(),
                "drawn from a sequence (`auto_increment`)",
            )),
            _ => Ok(()),
        }
    }

    fn unit(self, _: D) -> Result<(), Error> {
        Ok(())
    }
}

/// An entity's table as an error names it, schema-qualified where it is.
pub(crate) fn table_label<E: EntityName>(entity: &E) -> String {
    match entity.schema_name() {
        Some(schema) => format!("{schema}.{}", entity.table_name()),
        None => entity.table_name().to_owned(),
    }
}

/// Whether two column types are one PostgreSQL enum: the same name in the
/// same schema, with the same values in the same order.
fn same_enum(left: &ColumnType, right: &ColumnType) -> bool {
    let identity = |column_type: &ColumnType| match column_type {
        ColumnType::Enum {
            name,
            schema,
            variants,
        } => Some((
            name.to_string(),
            schema.as_ref().map(|schema| schema.to_string()),
            variants
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>(),
        )),
        _ => None,
    };
    matches!((identity(left), identity(right)), (Some(left), Some(right)) if left == right)
}

/// A column type as an error names it: an enum by its name and values.
fn describe(column_type: &ColumnType) -> String {
    match column_type {
        ColumnType::Enum {
            name,
            schema,
            variants,
        } => {
            let name = match schema {
                Some(schema) => format!("{}.{}", schema.to_string(), name.to_string()),
                None => name.to_string(),
            };
            let values: Vec<String> = variants
                .iter()
                .map(|value| format!("'{}'", value.to_string()))
                .collect();
            format!("`{name}` ({})", values.join(", "))
        }
        other => format!("`{other:?}`"),
    }
}
