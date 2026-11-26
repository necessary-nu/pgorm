//! Naming the row type on the statement, at the end of the chain.
//!
//! The [`Selector`] and [`SelectorRaw`] constructors take the statement as an
//! argument, so a caller names the decode target, as the selector type the
//! constructor belongs to, before the statement is mentioned. These traits
//! turn that inside out: the statement receives the call, and the target is
//! named after it, as the rest of a builder chain reads.

use std::ops::Deref;

use pgorm_query::{SelectStatement, Values};

use crate::{
    FromQueryResult, SelectGetableTuple, SelectGetableValue, SelectModel, Selector, SelectorRaw,
    SqlText, TryGetableMany,
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
// [spec:pgorm:sem:exec.crud.selector-entry+4]
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

// [spec:pgorm:sem:exec.crud.selector-entry+4]    statement-first entry
impl DecodeSelect for SelectStatement {
    fn into_model<M>(self) -> Selector<SelectModel<M>>
    where
        M: FromQueryResult,
    {
        Selector::from_select(self)
    }

    fn into_tuple<T>(self) -> Selector<SelectGetableTuple<T>>
    where
        T: TryGetableMany,
    {
        Selector::into_tuple(self)
    }

    fn into_values<T, C>(self) -> Selector<SelectGetableValue<T, C>>
    where
        T: TryGetableMany,
        C: strum::IntoEnumIterator + pgorm_query::SqlName,
    {
        Selector::with_columns(self)
    }
}

/// Decode a raw statement — SQL text and the values bound into it — by naming
/// its row type.
///
/// Implemented for `(sql, values)` pairs whose SQL is text as a
/// [`Connection`](crate::Connection) method takes it — a `&str`, a
/// `String`, a reference to one, or a `sql!` statement — which is what
/// [`SelectStatement::build`], [`Pipeline::into_sql`](crate::pipeline) and the
/// [`prql!`](crate::prql) macro all hand back, so the compiled query flows
/// straight into a decode without being taken apart first. The raw statement's
/// semantics are unchanged: no injected limit, and no projection guard. The
/// selector keeps the text's [`Kind`](DecodeRaw::Kind): built text is text,
/// which runs where text that may write runs, while a `prql!` read runs on
/// every connection.
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
// [spec:pgorm:sem:exec.crud.selector-entry+4]
pub trait DecodeRaw {
    /// The [`SqlText::Kind`] of the statement, which the selector carries to
    /// decide the connections it runs on.
    type Kind;

    /// Decode each row into a [`FromQueryResult`] type.
    fn into_model<M>(self) -> SelectorRaw<SelectModel<M>, Self::Kind>
    where
        M: FromQueryResult;

    /// Decode each row into a tuple, by column position.
    fn into_tuple<T>(self) -> SelectorRaw<SelectGetableTuple<T>, Self::Kind>
    where
        T: TryGetableMany;

    /// Decode each row into a tuple, by the column names of a column enum, one
    /// deriving `Column`.
    fn into_values<T, C>(self) -> SelectorRaw<SelectGetableValue<T, C>, Self::Kind>
    where
        T: TryGetableMany,
        C: strum::IntoEnumIterator + pgorm_query::SqlName;
}

// [spec:pgorm:sem:exec.crud.selector-entry+4]    raw-statement entry, over the SQL text a connection takes
impl<S> DecodeRaw for (S, Values)
where
    S: Deref,
    S::Target: SqlText,
{
    type Kind = <S::Target as SqlText>::Kind;

    fn into_model<M>(self) -> SelectorRaw<SelectModel<M>, Self::Kind>
    where
        M: FromQueryResult,
    {
        SelectorRaw::from_statement(&*self.0, self.1)
    }

    fn into_tuple<T>(self) -> SelectorRaw<SelectGetableTuple<T>, Self::Kind>
    where
        T: TryGetableMany,
    {
        SelectorRaw::into_tuple(&*self.0, self.1)
    }

    fn into_values<T, C>(self) -> SelectorRaw<SelectGetableValue<T, C>, Self::Kind>
    where
        T: TryGetableMany,
        C: strum::IntoEnumIterator + pgorm_query::SqlName,
    {
        SelectorRaw::with_columns(&*self.0, self.1)
    }
}
