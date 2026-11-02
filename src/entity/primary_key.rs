use super::{ColumnTrait, Iterable, StaticName};
use crate::{TryFromU64, TryGetableMany};
use pgorm_query::{IntoValueTuple, TryFromValueTuple};
use std::fmt::Debug;

/// A Trait for to be used to define a Primary Key.
///
/// `DeriveEntityModel` implements it on the `PrimaryKey` enum it writes, one
/// variant per `#[pgorm(primary_key)]` field.
///
/// See module level docs [crate::entity] for a full example
// [spec:pgorm:def:entity.traits.primary-key+7]
pub trait PrimaryKeyTrait: StaticName + Iterable {
    #[allow(missing_docs)]
    type ValueType: Sized
        + Send
        + Debug
        + PartialEq
        + IntoValueTuple
        + TryFromValueTuple
        + TryGetableMany
        + TryFromU64
        + PrimaryKeyArity
        + IntoPrimaryKey<Self::ValueType>;

    /// Whether the database generates the whole key, so that an insert
    /// naming no key column still writes a row with one.
    ///
    /// For a one-column key this is the serial family schema generation draws
    /// it from — or the column's own default or identity, which it then uses
    /// instead. A composite key is generated whole only when every column of
    /// it is an identity: `(tenant_id, id)` with `id` generated reports
    /// `false`, since the caller still supplies `tenant_id`, and the generated
    /// part is that column's fact, carried by its
    /// [`ColumnDef::identity`](crate::ColumnDef::identity).
    fn auto_increment() -> bool;

    /// Whether the key's last column is its period, PostgreSQL 18's temporal
    /// key `PRIMARY KEY (.., period WITHOUT OVERLAPS)`: the other columns are
    /// compared for equality and the period, a range or multirange, for
    /// overlap, so one key holds a row per period, none overlapping.
    ///
    /// The key value is still the whole key, period included, and a lookup by
    /// it is by equality: two equal periods overlap, so at most one row holds
    /// a given value of the key, and `find_by_id`, an update or a delete by key
    /// names one version of a row. Which version covers a moment is a filter
    /// on the period (`contains`), not a key lookup.
    // [spec:pgorm:def:entity.traits.primary-key+7]
    fn without_overlaps() -> bool {
        false
    }
}

/// How to map a Primary Key to a column
pub trait PrimaryKeyToColumn {
    #[allow(missing_docs)]
    type Column: ColumnTrait;

    /// Method to map a primary key to a column in an Entity
    fn into_column(self) -> Self::Column;

    /// Method to map a primary key from a column in an Entity
    fn from_column(col: Self::Column) -> Option<Self>
    where
        Self: Sized;
}

/// A value that converts into a primary key's
/// [`ValueType`](PrimaryKeyTrait::ValueType) part by part.
///
/// A one-column key takes anything that converts into its type, as `Into`
/// would: `find_by_id("x")` against a `String` key. A composite key takes a
/// tuple of the same arity whose every part converts into the key's part in
/// that position, so `find_by_id((1, "x"))` reaches an `(i32, String)` key
/// with no `"x".to_owned()` — `Into` cannot say that of a tuple, which would
/// take an impl for every pair of tuple types. A tuple of the wrong arity has
/// no impl, so the mismatch is a compile error:
///
/// ```compile_fail,E0277
/// # use pgorm::{entity::*, tests_cfg::cake_filling};
/// // `cake_filling`'s key is `(i32, i32)`.
/// cake_filling::Entity::find_by_id((1, 2, 3));
/// ```
// [spec:pgorm:def:entity.traits.primary-key+7]
pub trait IntoPrimaryKey<V> {
    /// The key value, converted part by part.
    fn into_primary_key(self) -> V;
}

impl<V, T> IntoPrimaryKey<V> for T
where
    V: crate::TryGetable,
    T: Into<V>,
{
    fn into_primary_key(self) -> V {
        self.into()
    }
}

macro_rules! impl_into_pk {
    ($(($value:ident, $part:ident, $index:tt)),+) => {
        impl<$($value, $part,)+> IntoPrimaryKey<($($value,)+)> for ($($part,)+)
        where
            $($value: crate::TryGetableMany, $part: Into<$value>,)+
        {
            fn into_primary_key(self) -> ($($value,)+) {
                ($(self.$index.into(),)+)
            }
        }
    };
}

impl_into_pk!((V1, T1, 0));
impl_into_pk!((V1, T1, 0), (V2, T2, 1));
impl_into_pk!((V1, T1, 0), (V2, T2, 1), (V3, T3, 2));
impl_into_pk!((V1, T1, 0), (V2, T2, 1), (V3, T3, 2), (V4, T4, 3));
impl_into_pk!(
    (V1, T1, 0),
    (V2, T2, 1),
    (V3, T3, 2),
    (V4, T4, 3),
    (V5, T5, 4)
);
impl_into_pk!(
    (V1, T1, 0),
    (V2, T2, 1),
    (V3, T3, 2),
    (V4, T4, 3),
    (V5, T5, 4),
    (V6, T6, 5)
);
impl_into_pk!(
    (V1, T1, 0),
    (V2, T2, 1),
    (V3, T3, 2),
    (V4, T4, 3),
    (V5, T5, 4),
    (V6, T6, 5),
    (V7, T7, 6)
);
impl_into_pk!(
    (V1, T1, 0),
    (V2, T2, 1),
    (V3, T3, 2),
    (V4, T4, 3),
    (V5, T5, 4),
    (V6, T6, 5),
    (V7, T7, 6),
    (V8, T8, 7)
);
impl_into_pk!(
    (V1, T1, 0),
    (V2, T2, 1),
    (V3, T3, 2),
    (V4, T4, 3),
    (V5, T5, 4),
    (V6, T6, 5),
    (V7, T7, 6),
    (V8, T8, 7),
    (V9, T9, 8)
);
impl_into_pk!(
    (V1, T1, 0),
    (V2, T2, 1),
    (V3, T3, 2),
    (V4, T4, 3),
    (V5, T5, 4),
    (V6, T6, 5),
    (V7, T7, 6),
    (V8, T8, 7),
    (V9, T9, 8),
    (V10, T10, 9)
);
impl_into_pk!(
    (V1, T1, 0),
    (V2, T2, 1),
    (V3, T3, 2),
    (V4, T4, 3),
    (V5, T5, 4),
    (V6, T6, 5),
    (V7, T7, 6),
    (V8, T8, 7),
    (V9, T9, 8),
    (V10, T10, 9),
    (V11, T11, 10)
);
impl_into_pk!(
    (V1, T1, 0),
    (V2, T2, 1),
    (V3, T3, 2),
    (V4, T4, 3),
    (V5, T5, 4),
    (V6, T6, 5),
    (V7, T7, 6),
    (V8, T8, 7),
    (V9, T9, 8),
    (V10, T10, 9),
    (V11, T11, 10),
    (V12, T12, 11)
);

/// How many columns this Primary Key comprises
// [spec:pgorm:def:entity.traits.primary-key+7]
pub trait PrimaryKeyArity {
    /// Arity of the Primary Key
    const ARITY: usize;
}

impl<V> PrimaryKeyArity for V
where
    V: crate::TryGetable,
{
    const ARITY: usize = 1;
}

macro_rules! impl_pk_arity {
    ($len:expr, $($tuple_arg:ident),*) => {
        impl<$($tuple_arg: crate::TryGetableMany,)*> PrimaryKeyArity for ($($tuple_arg,)*) {
            const ARITY: usize = $len;
        }
    }
}

impl_pk_arity!(1, T1);
impl_pk_arity!(2, T1, T2);
impl_pk_arity!(3, T1, T2, T3);
impl_pk_arity!(4, T1, T2, T3, T4);
impl_pk_arity!(5, T1, T2, T3, T4, T5);
impl_pk_arity!(6, T1, T2, T3, T4, T5, T6);
impl_pk_arity!(7, T1, T2, T3, T4, T5, T6, T7);
impl_pk_arity!(8, T1, T2, T3, T4, T5, T6, T7, T8);
impl_pk_arity!(9, T1, T2, T3, T4, T5, T6, T7, T8, T9);
impl_pk_arity!(10, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
impl_pk_arity!(11, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
impl_pk_arity!(12, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "macros")]
    fn test_composite_primary_key() {
        mod primary_key_of_1 {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
            #[pgorm(table_name = "primary_key_of_1")]
            pub struct Model {
                #[pgorm(primary_key)]
                pub id: i32,
                pub owner: String,
                pub name: String,
                pub description: String,
            }

            #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
            pub enum Relation {}

            impl ActiveModelBehavior for ActiveModel {}
        }

        mod primary_key_of_2 {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
            #[pgorm(table_name = "primary_key_of_2")]
            pub struct Model {
                #[pgorm(primary_key, auto_increment = false)]
                pub id_1: i32,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_2: String,
                pub owner: String,
                pub name: String,
                pub description: String,
            }

            #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
            pub enum Relation {}

            impl ActiveModelBehavior for ActiveModel {}
        }

        mod primary_key_of_3 {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
            #[pgorm(table_name = "primary_key_of_3")]
            pub struct Model {
                #[pgorm(primary_key, auto_increment = false)]
                pub id_1: i32,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_2: String,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_3: Uuid,
                pub owner: String,
                pub name: String,
                pub description: String,
            }

            #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
            pub enum Relation {}

            impl ActiveModelBehavior for ActiveModel {}
        }

        mod primary_key_of_4 {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
            #[pgorm(table_name = "primary_key_of_4")]
            pub struct Model {
                #[pgorm(primary_key, auto_increment = false)]
                pub id_1: DateTimeWithTimeZone,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_2: Uuid,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_3: Json,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_4: Decimal,
                pub owner: String,
                pub name: String,
                pub description: String,
            }

            #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
            pub enum Relation {}

            impl ActiveModelBehavior for ActiveModel {}
        }

        mod primary_key_of_11 {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
            #[pgorm(
                rs_type = "String",
                db_type = "String(StringLen::N(1))",
                enum_name = "category"
            )]
            pub enum DeriveCategory {
                #[pgorm(string_value = "B")]
                Big,
                #[pgorm(string_value = "S")]
                Small,
            }

            #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
            #[pgorm(table_name = "primary_key_of_11")]
            pub struct Model {
                #[pgorm(primary_key, auto_increment = false)]
                pub id_1: Vec<u8>,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_2: DeriveCategory,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_3: Date,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_4: DateTime,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_5: Time,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_6: Time,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_7: DateTime,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_8: DateTime,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_9: DateTimeWithTimeZone,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_10: DateTimeWithTimeZone,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_11: DateTimeWithTimeZone,
                pub owner: String,
                pub name: String,
                pub description: String,
            }

            #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
            pub enum Relation {}

            impl ActiveModelBehavior for ActiveModel {}
        }

        mod primary_key_of_12 {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
            #[pgorm(table_name = "primary_key_of_12")]
            pub struct Model {
                #[pgorm(primary_key, auto_increment = false)]
                pub id_1: String,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_2: i16,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_3: i16,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_4: i32,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_5: i32,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_6: i64,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_7: f32,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_8: f64,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_9: bool,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_10: Uuid,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_11: Decimal,
                #[pgorm(primary_key, auto_increment = false)]
                pub id_12: Date,
                pub owner: String,
                pub name: String,
                pub description: String,
            }

            #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
            pub enum Relation {}

            impl ActiveModelBehavior for ActiveModel {}
        }
    }
}
