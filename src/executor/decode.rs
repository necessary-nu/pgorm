//! Naming the row type on the statement, once, at the end of the chain.
//!
//! The [`Selector`] and [`SelectorRaw`] constructors take the statement as an
//! argument, so a caller writes the decode target twice — once to pick the
//! `Selector`'s selector type and once as the constructor's own parameter —
//! before the statement is even mentioned. These traits turn that inside out:
//! the statement receives the call, and the target is named once.

use pgorm_query::{SelectStatement, Values};

use crate::{
    FromQueryResult, SelectGetableTuple, SelectGetableValue, SelectModel, Selector, SelectorRaw,
    TryGetableMany,
};

/// Decode a caller-built [`SelectStatement`] by naming its row type.
///
/// The statement stays a statement, so what these produce is an ordinary
/// [`Selector`]: [`one`](Selector::one) still injects its `LIMIT 1`, the
/// empty-projection guard still runs, and `all`, `stream` and the paginator
/// are the same code as for an entity query.
///
/// ```
/// # #[cfg(feature = "macros")]
/// # {
/// use pgorm::entity::prelude::*;
/// use pgorm::tests_cfg::cake;
///
/// #[derive(Debug, FromQueryResult)]
/// struct NameOnly {
///     name: String,
/// }
///
/// let statement = cake::Entity::find()
///     .select_only()
///     .column(cake::Column::Name)
///     .into_query();
///
/// let selector = statement.into_model::<NameOnly>();
/// # let _ = selector;
/// # }
/// ```
// [spec:pgorm:sem:exec.crud.selector-entry+2]
pub trait DecodeSelect {
    /// Decode each row into a [`FromQueryResult`] type.
    fn into_model<M>(self) -> Selector<SelectModel<M>>
    where
        M: FromQueryResult;

    /// Decode each row into a tuple, by column position.
    fn into_tuple<T>(self) -> Selector<SelectGetableTuple<T>>
    where
        T: TryGetableMany;

    /// Decode each row into a tuple, by the column names of a column enum, one
    /// deriving `Column`.
    fn into_values<T, C>(self) -> Selector<SelectGetableValue<T, C>>
    where
        T: TryGetableMany,
        C: strum::IntoEnumIterator + pgorm_query::SqlName;
}

// [spec:pgorm:sem:exec.crud.selector-entry+2]    statement-first entry
impl DecodeSelect for SelectStatement {
    fn into_model<M>(self) -> Selector<SelectModel<M>>
    where
        M: FromQueryResult,
    {
        Selector::<SelectModel<M>>::from_select::<M>(self)
    }

    fn into_tuple<T>(self) -> Selector<SelectGetableTuple<T>>
    where
        T: TryGetableMany,
    {
        Selector::<SelectGetableTuple<T>>::into_tuple::<T>(self)
    }

    fn into_values<T, C>(self) -> Selector<SelectGetableValue<T, C>>
    where
        T: TryGetableMany,
        C: strum::IntoEnumIterator + pgorm_query::SqlName,
    {
        Selector::<SelectGetableValue<T, C>>::with_columns::<T, C>(self)
    }
}

/// Decode a raw statement — SQL text and the values bound into it — by naming
/// its row type.
///
/// Implemented for `(sql, values)` pairs, which is what
/// [`SelectStatement::build`], [`Pipeline::into_sql`](crate::pipeline) and the
/// [`prql!`](crate::prql) macro all hand back, so the compiled query flows
/// straight into a decode without being taken apart first. The raw statement's
/// semantics are unchanged: no injected limit, and no projection guard.
///
/// ```
/// use pgorm::entity::prelude::*;
/// use pgorm::pgorm_query::Values;
///
/// let selector = (
///     r#"SELECT "id", "name" FROM "cake""#,
///     Values(vec![]),
/// )
///     .into_tuple::<(i32, String)>();
/// # let _ = selector;
/// ```
// [spec:pgorm:sem:exec.crud.selector-entry+2]
pub trait DecodeRaw {
    /// Decode each row into a [`FromQueryResult`] type.
    fn into_model<M>(self) -> SelectorRaw<SelectModel<M>>
    where
        M: FromQueryResult;

    /// Decode each row into a tuple, by column position.
    fn into_tuple<T>(self) -> SelectorRaw<SelectGetableTuple<T>>
    where
        T: TryGetableMany;

    /// Decode each row into a tuple, by the column names of a column enum, one
    /// deriving `Column`.
    fn into_values<T, C>(self) -> SelectorRaw<SelectGetableValue<T, C>>
    where
        T: TryGetableMany,
        C: strum::IntoEnumIterator + pgorm_query::SqlName;
}

// [spec:pgorm:sem:exec.crud.selector-entry+2]    raw-statement entry, over any spelling of the SQL
impl<S> DecodeRaw for (S, Values)
where
    S: Into<String>,
{
    fn into_model<M>(self) -> SelectorRaw<SelectModel<M>>
    where
        M: FromQueryResult,
    {
        SelectorRaw::<SelectModel<M>>::from_statement::<M>(self.0.into(), self.1)
    }

    fn into_tuple<T>(self) -> SelectorRaw<SelectGetableTuple<T>>
    where
        T: TryGetableMany,
    {
        SelectorRaw::<SelectGetableTuple<T>>::into_tuple::<T>(self.0.into(), self.1)
    }

    fn into_values<T, C>(self) -> SelectorRaw<SelectGetableValue<T, C>>
    where
        T: TryGetableMany,
        C: strum::IntoEnumIterator + pgorm_query::SqlName,
    {
        SelectorRaw::<SelectGetableValue<T, C>>::with_columns::<T, C>(self.0.into(), self.1)
    }
}
