//! What a file that talks to the database needs in scope.
//!
//! The set is chosen from what code actually writes: the entity trait family
//! and the derives that produce it, the query-builder traits whose methods
//! are otherwise unreachable, the `ActiveValue` vocabulary, the CRUD entry
//! points, the decode targets, and the connection types.
//!
//! One name deliberately stays out. `Order` — the `ASC`/`DESC` enum — is a
//! plausible table name, and an entity aliased `Order` next to a glob of this
//! module makes every mention of it ambiguous; `order_by_asc` / `order_by_desc`
//! cover the common case and the enum is one import away.
//!
//! `StaticName` is here — pgorm_query's, re-exported. The entity layer used to
//! declare a second trait of its own with the same method, so this module and
//! `pgorm_query`'s could not both be glob-imported without ambiguity; there is
//! now one trait and the question does not arise.
//!
//! `alias` and its `AliasName` are here for the same reason `Expr` is: a name
//! the query introduces is written at the point the query is written, and
//! writing it as a token instead of a string is only cheaper than
//! `Name::runtime` if the token is already in scope.
//!
//! `Func` is here because an entity's `default_expr` is Rust written in the
//! entity's own module, and a key filled by `Func::uuidv7()` is the default a
//! new table most often wants.
//!
//! `ArrayType`, `ConditionType`, `Deferrability`, `Enforcement` and
//! `ForeignKeyAction` are here because a derive's attributes name them as
//! written, with no prefix of their own: `on_delete = ForeignKeyAction::Cascade`,
//! `deferrability = Deferrability::DeferrableInitiallyDeferred`.

// [spec:pgorm:def:entity.prelude+11]
// [spec:pgorm:sem:query.build.alias+2]
pub use crate::{
    ActiveEnum, ActiveModelBehavior, ActiveModelTrait, ActiveValue,
    ActiveValue::{NotSet, Set, Unchanged},
    AliasName, ColumnDef, ColumnTrait, ColumnType, ColumnTypeTrait, Condition, ConnectionTrait,
    CreatedRange, CursorTrait, DatabaseConnection, DatabasePool, DatabaseTransaction, DecodeRaw,
    DecodeSelect, Delete, Disjoint, EntityName, EntityTrait, FromQueryResult, Insert,
    IntoActiveModel, IntoActiveValue, Iterable, JoinType, Linked, LoaderTrait, ModelTrait,
    PaginatorTrait, PrimaryKeyArity, PrimaryKeyToColumn, PrimaryKeyTrait, QueryFilter, QueryOrder,
    QueryResult, QuerySelect, QueryTrait, Related, RelatedLink, RelationDef, RelationTrait, Select,
    SqlName, StaticName, TransactionTrait, TryInsert, TryIntoModel, Update, Value, alias,
    error::*,
    pgorm_query::{
        ArrayType, ConditionType, Deferrability, Enforcement, Expr, ForeignKeyAction, Func, Name,
        StringLen,
    },
    set,
};
pub use std::sync::Arc;

/// The derives with no trait of their name above. The rest — `ActiveEnum`,
/// `CreatedRange`, `Disjoint`, `EntityTrait`, `FromQueryResult`,
/// `IntoActiveModel`, `SqlName` and `StaticName` — arrive with the trait they
/// implement, one import bringing both.
// [spec:pgorm:req:macros.derive.names+1]
#[cfg(feature = "macros")]
pub use crate::{Column, Entity, JsonValueType, Model, PartialModel, Relation, ValueType};

pub use async_trait;

#[cfg(feature = "with-json")]
pub use serde_json::Value as Json;

#[cfg(feature = "with-jiff")]
pub use jiff::civil::{Date, DateTime, Time};

/// An absolute instant, carried by a `timestamptz` column.
///
/// Spelled this way rather than re-exporting `jiff::Timestamp` under its own
/// name: `ColumnType::Timestamp` is PostgreSQL's *naive* `timestamp`, which is
/// what [`DateTime`] maps to, so a field written `pub at: Timestamp` inferring
/// `TimestampWithTimeZone` would read backwards at every call site.
#[cfg(feature = "with-jiff")]
pub type DateTimeWithTimeZone = jiff::Timestamp;

pub use rust_decimal::Decimal;

#[cfg(feature = "with-uuid")]
pub use uuid::Uuid;

// [spec:pgorm:def:exec.decode.types+3]
pub use crate::pgorm_query::{IpNetwork, MacAddress, Vector};

// [spec:pgorm:req:exec.decode.interval+1]
pub use crate::pgorm_query::Interval;

/// The qualifier vocabulary of an interval column, so that
/// `column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth))`
/// resolves in the module the entity is written in.
// [spec:pgorm:def:entity.prelude+11]
pub use crate::pgorm_query::{IntervalPrecision, IntervalSpec, PgInterval};

// [spec:pgorm:def:exec.decode.range+3]
pub use crate::pgorm_query::{Multirange, Range, RangeType};
