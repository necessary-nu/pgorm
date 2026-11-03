use crate::{SelectGetableValue, SelectorRaw, error::*};
use std::error::Error as _;

#[cfg(feature = "postgres-array")]
#[path = "query_domain.rs"]
mod domain;
#[path = "query_range.rs"]
mod range;
pub(crate) use range::{multirange_from_sql, range_from_sql};

/// Defines the result of a query operation on a Model
// [spec:pgorm:def:exec.decode+3]
#[derive(Debug)]
#[repr(transparent)]
pub struct QueryResult {
    pub(crate) row: Row,
}

/// A row of a statement a caller sent itself, as the decoders read it.
impl From<Row> for QueryResult {
    fn from(row: Row) -> Self {
        Self { row }
    }
}

impl QueryResult {
    /// The row the result reads, for a caller decoding it by its own means.
    pub fn into_row(self) -> Row {
        self.row
    }
}

/// An interface to get a value from the query result
// [spec:pgorm:def:exec.decode+3]
pub trait TryGetable: Sized {
    /// Get a value from the query result with an RowIndex
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        index: I,
    ) -> Result<Self, TryGetError>;

    /// Get a value from the query result with prefixed column name
    fn try_get(res: &QueryResult, pre: &str, col: &str) -> Result<Self, TryGetError> {
        // tracing::debug!("try_get: pre={}, col={}", pre, col);
        if pre.is_empty() {
            Self::try_get_by(res, col)
        } else {
            Self::try_get_by(
                res,
                super::result_name::result_column_name(pre, col).as_str(),
            )
        }
    }

    /// Get a value from the query result based on the order in the select expressions
    fn try_get_by_index(res: &QueryResult, index: usize) -> Result<Self, TryGetError> {
        Self::try_get_by(res, index)
    }

    /// Whether a column of PostgreSQL type `ty` can be decoded into this type.
    ///
    /// This answers what `FromSql::accepts` answers for the type the decode
    /// actually reads, and is what
    /// [`VerifyStatement::verify`](crate::VerifyStatement::verify) checks a
    /// statement's result columns against. The default is permissive — an
    /// implementation that does not override it accepts every column type — so
    /// a decode path pgorm cannot see is left unreported rather than guessed
    /// at.
    // [spec:pgorm:sem:exec.verify.accepts+1]    the type-acceptance hook
    fn accepts(ty: &Type) -> bool {
        let _ = ty;
        true
    }
}

/// An error from trying to get a row from a Model
#[derive(Debug)]
#[non_exhaustive]
pub enum TryGetError {
    /// A database error was encountered as defined in [crate::Error]
    Db(Error),
    /// A null value was encountered
    Null(String),
}

// [spec:pgorm:sem:exec.decode.null+1]
impl TryGetError {
    fn postgres(value: tokio_postgres::Error) -> Self {
        let Some(source) = value.source() else {
            return TryGetError::Db(Error::Postgres(value));
        };

        if let Some(WasNull) = source.downcast_ref() {
            return TryGetError::Null(format!("{}", value));
        }

        TryGetError::Db(Error::Postgres(value))
    }
}

impl From<TryGetError> for Error {
    fn from(e: TryGetError) -> Error {
        match e {
            TryGetError::Db(e) => e,
            TryGetError::Null(s) => {
                type_err(format!("A null value was encountered while decoding {s}"))
            }
        }
    }
}

impl From<Error> for TryGetError {
    fn from(e: Error) -> TryGetError {
        Self::Db(e)
    }
}

// QueryResult //

impl QueryResult {
    /// Get a value from the query result with an RowIndex
    pub fn try_get_by<T, I>(&self, index: I) -> Result<T, Error>
    where
        T: TryGetable,
        I: RowIndex + std::fmt::Display,
    {
        Ok(T::try_get_by(self, index)?)
    }

    /// Get a value from the query result with prefixed column name
    pub fn try_get<T>(&self, pre: &str, col: &str) -> Result<T, Error>
    where
        T: TryGetable,
    {
        Ok(T::try_get(self, pre, col)?)
    }

    /// Get a value from the query result based on the order in the select expressions
    pub fn try_get_by_index<T>(&self, idx: usize) -> Result<T, Error>
    where
        T: TryGetable,
    {
        Ok(T::try_get_by_index(self, idx)?)
    }

    /// Get a tuple value from the query result with prefixed column name
    pub fn try_get_many<T>(&self, pre: &str, cols: &[String]) -> Result<T, Error>
    where
        T: TryGetableMany,
    {
        Ok(T::try_get_many(self, pre, cols)?)
    }

    /// Get a tuple value from the query result based on the order in the select expressions
    pub fn try_get_many_by_index<T>(&self) -> Result<T, Error>
    where
        T: TryGetableMany,
    {
        Ok(T::try_get_many_by_index(self)?)
    }

    /// Retrieves the names of the columns in the result set
    pub fn column_names(&self) -> Vec<String> {
        self.row
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect()
    }

    /// Whether every one of `cols`, read under `pre` exactly as
    /// [`TryGetable::try_get`] reads it, is a column of this row holding SQL
    /// `NULL`.
    ///
    /// The answer is `false` when the list is empty, when the result set does
    /// not carry one of the named columns, and when any of them holds a value
    /// — including a value of a PostgreSQL type the caller cannot decode,
    /// which is read here without being interpreted.
    ///
    /// This is the witness
    /// [`from_query_result_optional`](crate::FromQueryResult::from_query_result_optional)
    /// tests to tell an outer join that matched nothing from a related row
    /// that is present but fails to decode.
    // [spec:pgorm:req:exec.decode.absent]
    pub fn all_null<'c>(&self, pre: &str, cols: impl IntoIterator<Item = &'c str>) -> bool {
        let mut named_any = false;
        for col in cols {
            named_any = true;
            let present = if pre.is_empty() {
                self.row.try_get::<_, Option<AnyValue>>(col)
            } else {
                self.row.try_get::<_, Option<AnyValue>>(
                    super::result_name::result_column_name(pre, col).as_str(),
                )
            };
            if !matches!(present, Ok(None)) {
                return false;
            }
        }
        named_any
    }

    /// Whether every column of the result set whose name starts with `pre`
    /// holds SQL `NULL`.
    ///
    /// The answer is `false` when no column carries the prefix at all. An
    /// empty `pre` names every column of the row.
    ///
    /// This is the witness
    /// [`from_query_result_optional`](crate::FromQueryResult::from_query_result_optional)
    /// falls back on for a target that does not report the columns it reads
    /// ([`expected_columns`](crate::FromQueryResult::expected_columns)).
    // [spec:pgorm:req:exec.decode.absent]
    pub fn all_null_under(&self, pre: &str) -> bool {
        let mut named_any = false;
        for (idx, column) in self.row.columns().iter().enumerate() {
            if !column.name().starts_with(pre) {
                continue;
            }
            named_any = true;
            if !matches!(self.row.try_get::<_, Option<AnyValue>>(idx), Ok(None)) {
                return false;
            }
        }
        named_any
    }
}

/// A decode target that accepts every PostgreSQL type and reads none of the
/// bytes, so `Option<AnyValue>` answers whether a column held SQL `NULL` and
/// nothing else.
// [spec:pgorm:req:exec.decode.absent]
#[derive(Debug)]
struct AnyValue;

// [spec:pgorm:req:exec.decode.absent]
impl<'a> FromSql<'a> for AnyValue {
    fn from_sql(
        _ty: &Type,
        _raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self)
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }
}

// TryGetable //

// [spec:pgorm:sem:exec.decode.null+1]
impl<T: TryGetable> TryGetable for Option<T> {
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        index: I,
    ) -> Result<Self, TryGetError> {
        match T::try_get_by(res, index) {
            Ok(v) => Ok(Some(v)),
            Err(TryGetError::Null(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    // [spec:pgorm:sem:exec.verify.accepts+1]    the payload's acceptance, not the column's nullability
    fn accepts(ty: &Type) -> bool {
        T::accepts(ty)
    }
}

// [spec:pgorm:def:exec.decode.types+3]
macro_rules! try_getable_all {
    ( $type: ty ) => {
        impl TryGetable for $type {
            #[allow(unused_variables)]
            fn try_get_by<I: RowIndex + std::fmt::Display>(
                res: &QueryResult,
                idx: I,
            ) -> Result<Self, TryGetError> {
                let result: Result<$type, _> = res.row.try_get(idx);
                result.map_err(TryGetError::postgres)
            }

            // [spec:pgorm:sem:exec.verify.accepts+1]
            fn accepts(ty: &Type) -> bool {
                <$type as FromSql>::accepts(ty)
            }
        }
    };
}

try_getable_all!(bool);
try_getable_all!(i8);
try_getable_all!(i16);
try_getable_all!(i32);
try_getable_all!(i64);
try_getable_all!(f32);
try_getable_all!(f64);
try_getable_all!(String);
try_getable_all!(Vec<u8>);

#[cfg(feature = "with-json")]
try_getable_all!(serde_json::Value);

#[cfg(feature = "with-jiff")]
try_getable_all!(jiff::civil::Date);

#[cfg(feature = "with-jiff")]
try_getable_all!(jiff::civil::Time);

#[cfg(feature = "with-jiff")]
try_getable_all!(jiff::civil::DateTime);

#[cfg(feature = "with-jiff")]
try_getable_all!(jiff::Timestamp);

use rust_decimal::Decimal;

impl TryGetable for Decimal {
    #[allow(unused_variables)]
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, TryGetError> {
        let result: Decimal = res.row.try_get(idx).map_err(TryGetError::postgres)?;
        Ok(result)
    }

    // [spec:pgorm:sem:exec.verify.accepts+1]
    fn accepts(ty: &Type) -> bool {
        <Decimal as FromSql>::accepts(ty)
    }
}

use pgorm_query::{Interval, IpNetwork, MacAddress, Values, Vector};
#[cfg(feature = "with-json")]
use tokio_postgres::types::Json;
use tokio_postgres::{
    Row,
    row::RowIndex,
    types::{FromSql, Oid, Type, WasNull},
};

#[allow(unused_macros)]
macro_rules! try_getable_uuid {
    ( $type: ty, $conversion_fn: expr ) => {
        #[allow(unused_variables, unreachable_code)]
        impl TryGetable for $type {
            fn try_get_by<I: RowIndex + std::fmt::Display>(
                res: &QueryResult,
                idx: I,
            ) -> Result<Self, TryGetError> {
                let res: Result<uuid::Uuid, TryGetError> = res
                    .row
                    .try_get(idx)
                    .map_err(|e| TryGetError::postgres(e).into());
                res.map($conversion_fn)
            }

            // [spec:pgorm:sem:exec.verify.accepts+1]    the uuid actually read
            fn accepts(ty: &Type) -> bool {
                <uuid::Uuid as FromSql>::accepts(ty)
            }
        }
    };
}

#[cfg(feature = "with-uuid")]
try_getable_uuid!(uuid::Uuid, Into::into);

#[cfg(feature = "with-uuid")]
try_getable_uuid!(uuid::fmt::Braced, uuid::Uuid::braced);

#[cfg(feature = "with-uuid")]
try_getable_uuid!(uuid::fmt::Hyphenated, uuid::Uuid::hyphenated);

#[cfg(feature = "with-uuid")]
try_getable_uuid!(uuid::fmt::Simple, uuid::Uuid::simple);

#[cfg(feature = "with-uuid")]
try_getable_uuid!(uuid::fmt::Urn, uuid::Uuid::urn);

/// `ipnetwork::IpNetwork` ships no `FromSql` impl and the orphan rule forbids
/// writing one for it here, so decoding routes through a local newtype that
/// reads the wire format with `postgres_protocol` and rebuilds the network.
// [spec:pgorm:def:exec.decode.types+3]
#[derive(Debug)]
struct InetSql(IpNetwork);

// [spec:pgorm:def:exec.decode.types+3]
impl<'a> FromSql<'a> for InetSql {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let inet = postgres_protocol::types::inet_from_sql(raw)?;
        Ok(Self(IpNetwork::new(inet.addr(), inet.netmask())?))
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::INET || *ty == Type::CIDR
    }
}

/// The `mac_address::MacAddress` counterpart of [`InetSql`].
// [spec:pgorm:def:exec.decode.types+3]
#[derive(Debug)]
struct MacAddrSql(MacAddress);

// [spec:pgorm:def:exec.decode.types+3]
impl<'a> FromSql<'a> for MacAddrSql {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let bytes = postgres_protocol::types::macaddr_from_sql(raw)?;
        Ok(Self(MacAddress::new(bytes)))
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::MACADDR
    }
}

// [spec:pgorm:def:exec.decode.types+3]
impl TryGetable for IpNetwork {
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, TryGetError> {
        let result: InetSql = res.row.try_get(idx).map_err(TryGetError::postgres)?;
        Ok(result.0)
    }

    // [spec:pgorm:sem:exec.verify.accepts+1]    the newtype that reads the wire format
    fn accepts(ty: &Type) -> bool {
        <InetSql as FromSql>::accepts(ty)
    }
}

// [spec:pgorm:def:exec.decode.types+3]
impl TryGetable for MacAddress {
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, TryGetError> {
        let result: MacAddrSql = res.row.try_get(idx).map_err(TryGetError::postgres)?;
        Ok(result.0)
    }

    // [spec:pgorm:sem:exec.verify.accepts+1]    the newtype that reads the wire format
    fn accepts(ty: &Type) -> bool {
        <MacAddrSql as FromSql>::accepts(ty)
    }
}

// [spec:pgorm:req:exec.decode.interval+1]
try_getable_all!(Interval);

// [spec:pgorm:def:exec.decode.types+3]
impl TryGetable for Vector {
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, TryGetError> {
        res.row.try_get(idx).map_err(TryGetError::postgres)
    }

    // [spec:pgorm:sem:exec.verify.accepts+1]
    fn accepts(ty: &Type) -> bool {
        <Vector as FromSql>::accepts(ty)
    }
}

// [spec:pgorm:sem:exec.decode.u32-oid]
impl TryGetable for u32 {
    #[allow(unused_variables)]
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, TryGetError> {
        let result: Result<Oid, _> = res.row.try_get(idx);
        result.map_err(TryGetError::postgres)
    }

    // [spec:pgorm:sem:exec.verify.accepts+1]    `OID`, not `INT4`
    fn accepts(ty: &Type) -> bool {
        <Oid as FromSql>::accepts(ty)
    }
}

// [spec:pgorm:sem:exec.decode.null-context+1]
#[allow(dead_code)]
fn err_null_idx_col<I: RowIndex + std::fmt::Display>(_idx: I) -> TryGetError {
    TryGetError::Null("TODO".into()) //format!("{_idx:?}"))
}

// [spec:pgorm:def:exec.decode.array+2]
#[cfg(feature = "postgres-array")]
mod postgres_array {
    use super::domain::Transparent;
    use super::*;

    #[allow(unused_macros)]
    macro_rules! try_getable_postgres_array {
        ( $type: ty ) => {
            #[allow(unused_variables)]
            impl TryGetable for Vec<$type> {
                fn try_get_by<I: RowIndex + std::fmt::Display>(
                    res: &QueryResult,
                    idx: I,
                ) -> Result<Self, TryGetError> {
                    let result: Transparent<Vec<$type>> =
                        res.row.try_get(idx).map_err(TryGetError::postgres)?;
                    Ok(result.0)
                }

                // [spec:pgorm:sem:exec.verify.accepts+1]
                fn accepts(ty: &Type) -> bool {
                    <Transparent<Vec<$type>> as FromSql>::accepts(ty)
                }
            }
        };
    }

    try_getable_postgres_array!(bool);
    try_getable_postgres_array!(i8);
    try_getable_postgres_array!(i16);
    try_getable_postgres_array!(i32);
    try_getable_postgres_array!(i64);
    try_getable_postgres_array!(f32);
    try_getable_postgres_array!(f64);
    try_getable_postgres_array!(String);

    #[cfg(feature = "with-json")]
    try_getable_postgres_array!(serde_json::Value);

    #[cfg(feature = "with-jiff")]
    try_getable_postgres_array!(jiff::civil::Date);

    #[cfg(feature = "with-jiff")]
    try_getable_postgres_array!(jiff::civil::Time);

    #[cfg(feature = "with-jiff")]
    try_getable_postgres_array!(jiff::civil::DateTime);

    #[cfg(feature = "with-jiff")]
    try_getable_postgres_array!(jiff::Timestamp);

    try_getable_postgres_array!(rust_decimal::Decimal);

    // [spec:pgorm:req:exec.decode.interval+1]
    try_getable_postgres_array!(Interval);

    #[allow(unused_macros)]
    macro_rules! try_getable_postgres_array_uuid {
        ( $type: ty, $conversion_fn: expr ) => {
            #[allow(unused_variables, unreachable_code)]
            impl TryGetable for Vec<$type> {
                fn try_get_by<I: RowIndex + std::fmt::Display>(
                    res: &QueryResult,
                    idx: I,
                ) -> Result<Self, TryGetError> {
                    let res: Transparent<Vec<uuid::Uuid>> =
                        res.row.try_get(idx).map_err(TryGetError::postgres)?;
                    Ok(res.0.into_iter().map($conversion_fn).collect())
                }

                // [spec:pgorm:sem:exec.verify.accepts+1]
                fn accepts(ty: &Type) -> bool {
                    <Transparent<Vec<uuid::Uuid>> as FromSql>::accepts(ty)
                }
            }
        };
    }

    #[cfg(feature = "with-uuid")]
    try_getable_postgres_array_uuid!(uuid::Uuid, Into::into);

    #[cfg(feature = "with-uuid")]
    try_getable_postgres_array_uuid!(uuid::fmt::Braced, uuid::Uuid::braced);

    #[cfg(feature = "with-uuid")]
    try_getable_postgres_array_uuid!(uuid::fmt::Hyphenated, uuid::Uuid::hyphenated);

    #[cfg(feature = "with-uuid")]
    try_getable_postgres_array_uuid!(uuid::fmt::Simple, uuid::Uuid::simple);

    #[cfg(feature = "with-uuid")]
    try_getable_postgres_array_uuid!(uuid::fmt::Urn, uuid::Uuid::urn);

    // [spec:pgorm:sem:exec.decode.u32-oid]
    impl TryGetable for Vec<u32> {
        #[allow(unused_variables)]
        fn try_get_by<I: RowIndex + std::fmt::Display>(
            res: &QueryResult,
            idx: I,
        ) -> Result<Self, TryGetError> {
            let result: Vec<Oid> = res.row.try_get(idx).map_err(TryGetError::postgres)?;
            Ok(result)
        }

        // [spec:pgorm:sem:exec.verify.accepts+1]    `OID[]`, not `INT4[]`
        fn accepts(ty: &Type) -> bool {
            <Vec<Oid> as FromSql>::accepts(ty)
        }
    }
}

// impl TryGetable for Row {
//     fn try_get_by<I: RowIndex + std::fmt::Display>(res: &QueryResult, index: I) -> Result<Self, TryGetError> {
//         res.row.try_get(index).map_err(TryGetError::postgres)
//     }
// }

// TryGetableMany //

/// An interface to get a tuple value from the query result
// [spec:pgorm:def:exec.decode.many]
pub trait TryGetableMany: Sized {
    /// Get a tuple value from the query result with prefixed column name
    fn try_get_many(res: &QueryResult, pre: &str, cols: &[String]) -> Result<Self, TryGetError>;

    /// Get a tuple value from the query result based on the order in the select expressions
    fn try_get_many_by_index(res: &QueryResult) -> Result<Self, TryGetError>;

    /// Run a raw statement and decode each row into a tuple, naming the
    /// columns through a column enum, one deriving `Column`.
    ///
    /// ```no_run
    /// # #[cfg(feature = "macros")]
    /// # {
    /// # use pgorm::{error::*, query::*, Column, DatabasePool, TryGetableMany};
    /// # use pgorm::pgorm_query::Values;
    /// #
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// #[derive(Clone, Copy, Debug, Column)]
    /// enum ResultCol {
    ///     Name,
    ///     NumOfCakes,
    /// }
    ///
    /// let db = pool.get().await?;
    ///
    /// let res: Vec<(String, i64)> = <(String, i64)>::find_by_statement::<ResultCol>(
    ///     r#"SELECT "cake"."name", count("cake"."id") AS "num_of_cakes" FROM "cake" GROUP BY "cake"."name""#
    ///         .to_owned(),
    ///     Values(vec![]),
    /// )
    /// .all(&db)
    /// .await?;
    /// # Ok(())
    /// # }
    /// # }
    /// ```
    fn find_by_statement<C>(
        stmt: String,
        values: Values,
    ) -> SelectorRaw<SelectGetableValue<Self, C>>
    where
        C: strum::IntoEnumIterator + pgorm_query::SqlName,
    {
        SelectorRaw::<SelectGetableValue<Self, C>>::with_columns(stmt, values)
    }
}

impl<T> TryGetableMany for T
where
    T: TryGetable,
{
    fn try_get_many(res: &QueryResult, pre: &str, cols: &[String]) -> Result<Self, TryGetError> {
        try_get_many_with_slice_len_of(1, cols)?;
        T::try_get(res, pre, &cols[0])
    }

    fn try_get_many_by_index(res: &QueryResult) -> Result<Self, TryGetError> {
        T::try_get_by_index(res, 0)
    }
}

impl<T> TryGetableMany for (T,)
where
    T: TryGetableMany,
{
    fn try_get_many(res: &QueryResult, pre: &str, cols: &[String]) -> Result<Self, TryGetError> {
        T::try_get_many(res, pre, cols).map(|r| (r,))
    }

    fn try_get_many_by_index(res: &QueryResult) -> Result<Self, TryGetError> {
        T::try_get_many_by_index(res).map(|r| (r,))
    }
}

// [spec:pgorm:def:exec.decode.many]
macro_rules! impl_try_get_many {
    ( $LEN:expr, $($T:ident : $N:expr),+ $(,)? ) => {
        impl< $($T),+ > TryGetableMany for ( $($T),+ )
        where
            $($T: TryGetable),+
        {
            fn try_get_many(res: &QueryResult, pre: &str, cols: &[String]) -> Result<Self, TryGetError> {
                try_get_many_with_slice_len_of($LEN, cols)?;
                Ok((
                    $($T::try_get(res, pre, &cols[$N])?),+
                ))
            }

            fn try_get_many_by_index(res: &QueryResult) -> Result<Self, TryGetError> {
                Ok((
                    $($T::try_get_by_index(res, $N)?),+
                ))
            }
        }
    };
}

#[rustfmt::skip]
mod impl_try_get_many {
    use super::*;

    impl_try_get_many!( 2, T0:0, T1:1);
    impl_try_get_many!( 3, T0:0, T1:1, T2:2);
    impl_try_get_many!( 4, T0:0, T1:1, T2:2, T3:3);
    impl_try_get_many!( 5, T0:0, T1:1, T2:2, T3:3, T4:4);
    impl_try_get_many!( 6, T0:0, T1:1, T2:2, T3:3, T4:4, T5:5);
    impl_try_get_many!( 7, T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6);
    impl_try_get_many!( 8, T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7);
    impl_try_get_many!( 9, T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8);
    impl_try_get_many!(10, T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8, T9:9);
    impl_try_get_many!(11, T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8, T9:9, T10:10);
    impl_try_get_many!(12, T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8, T9:9, T10:10, T11:11);
}

// [spec:pgorm:req:exec.decode.many-arity]
fn try_get_many_with_slice_len_of(len: usize, cols: &[String]) -> Result<(), TryGetError> {
    if cols.len() < len {
        Err(type_err(format!(
            "Expect {} column names supplied but got slice of length {}",
            len,
            cols.len()
        ))
        .into())
    } else {
        Ok(())
    }
}

/// An interface to get an array of values from the query result.
/// A type can only implement `ActiveEnum` or `TryGetableFromJson`, but not both.
/// A blanket impl is provided for `TryGetableFromJson`, while the impl for `ActiveEnum`
/// is provided by the `ActiveEnum` macro. So as an end user you won't normally
/// touch this trait.
// [spec:pgorm:def:exec.decode.json+1]
pub trait TryGetableArray: Sized {
    /// Just a delegate
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        index: I,
    ) -> Result<Vec<Self>, TryGetError>;
}

impl<T> TryGetable for Vec<T>
where
    T: TryGetableArray,
{
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        index: I,
    ) -> Result<Self, TryGetError> {
        T::try_get_by(res, index)
    }
}

// TryGetableFromJson //

/// An interface to get a JSON from the query result
// [spec:pgorm:def:exec.decode.json+1]
#[cfg(feature = "with-json")]
pub trait TryGetableFromJson: Sized
where
    for<'de> Self: serde::Deserialize<'de>,
{
    /// Get a JSON from the query result with prefixed column name
    #[allow(unused_variables, unreachable_code)]
    fn try_get_from_json<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, TryGetError> {
        let result: Result<Json<Self>, _> = res.row.try_get(idx);
        result.map_err(TryGetError::postgres).map(|x| x.0)
    }

    /// Get a `Vec<Self>` from an Array of Json
    fn from_json_vec(value: serde_json::Value) -> Result<Vec<Self>, TryGetError> {
        match value {
            serde_json::Value::Array(values) => {
                let mut res = Vec::new();
                for item in values {
                    res.push(serde_json::from_value(item).map_err(json_err)?);
                }
                Ok(res)
            }
            _ => Err(TryGetError::Db(Error::Json(
                "Value is not an Array".to_owned(),
            ))),
        }
    }
}

#[cfg(feature = "with-json")]
impl<T> TryGetable for T
where
    T: TryGetableFromJson,
{
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        index: I,
    ) -> Result<Self, TryGetError> {
        T::try_get_from_json(res, index)
    }

    // [spec:pgorm:sem:exec.verify.accepts+1]    the `Json<T>` wrapper's own types
    fn accepts(ty: &Type) -> bool {
        *ty == Type::JSON || *ty == Type::JSONB
    }
}

#[cfg(feature = "with-json")]
impl<T> TryGetableArray for T
where
    T: TryGetableFromJson,
{
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        index: I,
    ) -> Result<Vec<T>, TryGetError> {
        T::from_json_vec(serde_json::Value::try_get_by(res, index)?)
    }
}

// TryFromU64 //
/// Try to convert a type to a u64
// [spec:pgorm:def:exec.decode.from-u64+3]
pub trait TryFromU64: Sized {
    /// The method to convert the type to a u64
    fn try_from_u64(n: u64) -> Result<Self, Error>;
}

macro_rules! try_from_u64_err {
    ( $type: ty ) => {
        impl TryFromU64 for $type {
            fn try_from_u64(_: u64) -> Result<Self, Error> {
                Err(Error::ConvertFromU64(stringify!($type)))
            }
        }
    };

    ( $($gen_type: ident),* ) => {
        impl<$( $gen_type, )*> TryFromU64 for ($( $gen_type, )*)
        where
            $( $gen_type: TryFromU64, )*
        {
            fn try_from_u64(_: u64) -> Result<Self, Error> {
                Err(Error::ConvertFromU64(stringify!($($gen_type,)*)))
            }
        }
    };
}

#[rustfmt::skip]
mod try_from_u64_err {
    use super::*;

    try_from_u64_err!(T0, T1);
    try_from_u64_err!(T0, T1, T2);
    try_from_u64_err!(T0, T1, T2, T3);
    try_from_u64_err!(T0, T1, T2, T3, T4);
    try_from_u64_err!(T0, T1, T2, T3, T4, T5);
    try_from_u64_err!(T0, T1, T2, T3, T4, T5, T6);
    try_from_u64_err!(T0, T1, T2, T3, T4, T5, T6, T7);
    try_from_u64_err!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
    try_from_u64_err!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);
    try_from_u64_err!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
    try_from_u64_err!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
}

macro_rules! try_from_u64_numeric {
    ( $type: ty ) => {
        impl TryFromU64 for $type {
            fn try_from_u64(n: u64) -> Result<Self, Error> {
                use std::convert::TryInto;
                n.try_into().map_err(|e| Error::Conversion {
                    from: stringify!(u64),
                    into: stringify!($type),
                    source: Box::new(e),
                })
            }
        }
    };
}

try_from_u64_numeric!(i8);
try_from_u64_numeric!(i16);
try_from_u64_numeric!(i32);
try_from_u64_numeric!(i64);
try_from_u64_numeric!(u8);
try_from_u64_numeric!(u16);
try_from_u64_numeric!(u32);
try_from_u64_numeric!(u64);

macro_rules! try_from_u64_string {
    ( $type: ty ) => {
        impl TryFromU64 for $type {
            fn try_from_u64(n: u64) -> Result<Self, Error> {
                Ok(n.to_string())
            }
        }
    };
}

try_from_u64_string!(String);

try_from_u64_err!(bool);
try_from_u64_err!(f32);
try_from_u64_err!(f64);
try_from_u64_err!(Vec<u8>);

#[cfg(feature = "with-json")]
try_from_u64_err!(serde_json::Value);

#[cfg(feature = "with-jiff")]
try_from_u64_err!(jiff::civil::Date);

#[cfg(feature = "with-jiff")]
try_from_u64_err!(jiff::civil::Time);

#[cfg(feature = "with-jiff")]
try_from_u64_err!(jiff::civil::DateTime);

#[cfg(feature = "with-jiff")]
try_from_u64_err!(jiff::Timestamp);

try_from_u64_err!(rust_decimal::Decimal);

#[cfg(feature = "with-uuid")]
try_from_u64_err!(uuid::Uuid);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_try_get_error() {
        // TryGetError::Db
        let try_get_error = TryGetError::Db(Error::Query(RuntimeError::Internal(
            "expected error message".to_owned(),
        )));
        assert_eq!(
            Error::from(try_get_error),
            Error::Query(RuntimeError::Internal("expected error message".to_owned()))
        );

        // TryGetError::Null
        let try_get_error = TryGetError::Null("column".to_owned());
        let expected = "A null value was encountered while decoding column".to_owned();
        assert_eq!(Error::from(try_get_error), Error::Type(expected));
    }

    // [spec:pgorm:def:exec.decode.types+3/test]
    #[test]
    fn decodes_inet_wire_format() {
        let v4 = InetSql::from_sql(&Type::INET, &[2, 24, 0, 4, 10, 0, 0, 1]).unwrap();
        assert_eq!(v4.0, "10.0.0.1/24".parse::<IpNetwork>().unwrap());

        let v6 = InetSql::from_sql(
            &Type::INET,
            &[
                3, 128, 0, 16, 0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5,
            ],
        )
        .unwrap();
        assert_eq!(v6.0, "2001:db8::5/128".parse::<IpNetwork>().unwrap());

        assert!(InetSql::from_sql(&Type::INET, &[9, 24, 0, 4, 10, 0, 0, 1]).is_err());
        assert!(<InetSql as FromSql>::accepts(&Type::INET));
        assert!(<InetSql as FromSql>::accepts(&Type::CIDR));
        assert!(!<InetSql as FromSql>::accepts(&Type::MACADDR));
    }

    // [spec:pgorm:def:exec.decode.types+3/test]
    #[test]
    fn decodes_macaddr_wire_format() {
        let mac = MacAddrSql::from_sql(&Type::MACADDR, &[0, 0x11, 0x22, 0x33, 0x44, 0x55]).unwrap();
        assert_eq!(mac.0, "00:11:22:33:44:55".parse::<MacAddress>().unwrap());

        assert!(MacAddrSql::from_sql(&Type::MACADDR, &[0, 0x11, 0x22]).is_err());
        assert!(<MacAddrSql as FromSql>::accepts(&Type::MACADDR));
        assert!(!<MacAddrSql as FromSql>::accepts(&Type::MACADDR8));
    }

    #[test]
    fn build_with_query() {
        use pgorm_query::*;

        let id = alias("id");
        let depth = alias("depth");
        let next = alias("next");
        let value = alias("value");
        let table = alias("table");
        let traversal = alias("cte_traversal");

        let base_query = SelectStatement::new()
            .column(id)
            .expr(1i32)
            .column(next)
            .column(value)
            .from(table)
            .to_owned();

        let cte_referencing = SelectStatement::new()
            .column(id)
            .expr(Expr::col(depth).add(1i32))
            .column(next)
            .column(value)
            .from(table)
            .join(
                JoinType::InnerJoin,
                traversal,
                Expr::col((traversal, next)).equals((table, id)),
            )
            .to_owned();

        let common_table_expression = CommonTableExpression::new(
            traversal,
            base_query
                .clone()
                .union(UnionType::All, cte_referencing)
                .to_owned(),
        )
        .columns([id, depth, next, value])
        .to_owned();

        let mut select = SelectStatement::new()
            .column(ColumnRef::Asterisk)
            .from(traversal)
            .to_owned();

        let with_clause = RecursiveWithClause::new(common_table_expression)
            .cycle(Cycle::new(
                SimpleExpr::Column(ColumnRef::Column(id.into_name())),
                alias("looped"),
                alias("traversal_path"),
            ))
            .to_owned();

        let with_query = select.with(with_clause);

        assert_eq!(
            with_query.to_string(),
            [
                r#"WITH RECURSIVE "cte_traversal" ("id", "depth", "next", "value") AS"#,
                r#"(SELECT "id", 1, "next", "value" FROM "table" UNION ALL"#,
                r#"(SELECT "id", "depth" + 1, "next", "value" FROM "table""#,
                r#"INNER JOIN "cte_traversal" ON "cte_traversal"."next" = "table"."id"))"#,
                r#"CYCLE "id" SET "looped" USING "traversal_path""#,
                r#"SELECT * FROM "cte_traversal""#,
            ]
            .join(" ")
        );
    }
}
