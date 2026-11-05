//! Binary COPY as rows of pgorm [`Value`]s: the columns a COPY carries, as
//! the server describes them, the binary format's framing in both
//! directions, and the sink and stream that run it over [`CopyTrait`].
//!
//! Binary COPY sends no column types, only each field's bytes in its type's
//! binary send form, so the encoder and the decoder are given the columns:
//! [`CopyTrait::copy_in_columns`] and [`CopyTrait::copy_out_columns`] ask the
//! server, which knows them exactly — a domain, an enum, an array of either —
//! before the COPY starts.
//!
//! [`CopyTrait`]: crate::CopyTrait
//! [`CopyTrait::copy_in_columns`]: crate::CopyTrait::copy_in_columns
//! [`CopyTrait::copy_out_columns`]: crate::CopyTrait::copy_out_columns

use std::{
    error::Error as StdError,
    ops::Bound,
    pin::Pin,
    task::{Context, Poll, ready},
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{SinkExt, Stream, StreamExt};
use jiff::{
    Timestamp,
    civil::{Date, DateTime, Time},
};
use pgorm_pool::GenericClient;
use pgorm_query::{
    ArrayType, Asterisk, CopyFormat, CopyFromStatement, CopyToStatement, Interval, IpNetwork,
    MacAddress, Multirange, Query, Range, RangeType, StaticName, Value, Vector, base_type,
};
use postgres_protocol::types::{self as wire, RangeBound};
use tokio_postgres::types::{FromSql, IsNull, Kind, ToSql, Type};

use crate::{
    ActiveModelTrait, CopyInSink, CopyOutStream, EntityTrait, Error, ExactDecimal, Iterable,
    ModelTrait, TryIntoModel, ValueHolder,
    error::{query_err, type_err},
    executor::created_range::{
        CreatedRangeCodec, CreatedRangeText, completed_multiranges, is_created_multirange,
        is_created_range,
    },
};

mod money;

use money::{decode_money, holds_money, money_places, money_to_sql};

type WireError = Box<dyn StdError + Sync + Send>;

/// One column of a COPY's rows: its name and its PostgreSQL type, as the
/// server describes the statement, and for a `money` column the places of
/// its currency.
// [spec:pgorm:req:conn.copy.rows+5]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyColumn {
    name: String,
    type_: Type,
    money_places: Option<u32>,
}

impl CopyColumn {
    /// A column of `type_` named `name`, for an encoder or a decoder over
    /// columns known without asking the server.
    pub fn new(name: impl Into<String>, type_: Type) -> Self {
        Self {
            name: name.into(),
            type_,
            money_places: None,
        }
    }

    /// This column, its `money` counted in a currency of `places` fractional
    /// digits.
    ///
    /// A `money` travels as an `int8` count of its currency's minor units,
    /// whose places only the session's `lc_monetary` knows, so a `money`
    /// column, an array of one or a domain over either, reads and writes as
    /// a `Decimal` only with them. A description asks the server.
    // [spec:pgorm:req:conn.copy.rows+5]
    #[must_use]
    pub fn with_money_places(self, places: u32) -> Self {
        Self {
            money_places: Some(places),
            ..self
        }
    }

    /// The places of the currency a `money` column counts in, where this
    /// column holds `money` and they are known.
    pub fn money_places(&self) -> Option<u32> {
        self.money_places.filter(|_| holds_money(&self.type_))
    }

    /// The column's name as the server reports it: unquoted, its case kept.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The column's type. The server describes a domain column by the type
    /// the domain is over, whose form its values travel in. A multirange type
    /// a schema created, which the driver reports with no subtype, is
    /// described over the subtype the catalogue names for it.
    pub fn type_(&self) -> &Type {
        &self.type_
    }

    /// One field of this column, its binary form or `None` for SQL NULL, as
    /// the [`Value`] variant its type reads as.
    ///
    /// # Errors
    ///
    /// [`Error::Type`] naming the column, when its type is one no `Value`
    /// holds or the bytes are not that type's binary form.
    // [spec:pgorm:req:conn.copy.rows+5]
    pub fn decode(&self, raw: Option<&[u8]>) -> Result<Value, Error> {
        let decoded = match self.money_places() {
            Some(places) => decode_money(&self.type_, places, raw),
            None => decode(&self.type_, raw),
        };
        decoded.map_err(|error| {
            type_err(format!(
                "COPY column `{}` of type `{}` cannot be decoded: {error}",
                self.name, self.type_
            ))
        })
    }
}

/// The columns each row of a COPY FROM carries: the statement's column list
/// from its table, or every column the server does not generate.
// [spec:pgorm:req:conn.copy.rows+5]
pub(crate) async fn copy_in_columns<C, F>(
    client: &C,
    statement: &CopyFromStatement<F>,
) -> Result<Vec<CopyColumn>, Error>
where
    C: GenericClient,
    F: CopyFormat,
{
    let table = statement.get_table().clone();
    let listed = statement.get_columns();
    if listed.is_empty() {
        let sql = Query::select().column(Asterisk).from(table).to_string();
        described(client, &sql, true).await
    } else {
        let sql = Query::select()
            .columns(listed.iter().cloned())
            .from(table)
            .to_string();
        described(client, &sql, false).await
    }
}

/// The columns each row of a COPY TO carries: its table's, as a COPY FROM
/// with no column list has them, or its statement's result columns. A
/// `SELECT` source the server would refuse is refused first, as `copy_out`
/// refuses it.
// [spec:pgorm:req:conn.copy.rows+5]
pub(crate) async fn copy_out_columns<C, F>(
    client: &C,
    statement: &CopyToStatement<F>,
) -> Result<Vec<CopyColumn>, Error>
where
    C: GenericClient,
    F: CopyFormat,
{
    statement.check_source()?;
    let source = statement.get_source();
    if let Some(table) = source.get_table() {
        let sql = Query::select()
            .column(Asterisk)
            .from(table.clone())
            .to_string();
        return described(client, &sql, true).await;
    }
    let Some(query) = source.get_query() else {
        return Err(query_err("a COPY TO source is a table or a statement"));
    };
    described(client, &query.to_string(), false).await
}

/// The result columns of `sql`, prepared and never run, so a write is
/// described without writing. A whole table's `SELECT *` lists its generated
/// columns, which COPY leaves out in both directions, so those are read from
/// the catalog and dropped, and each multirange type a schema created is
/// completed with its subtype.
// [spec:pgorm:req:conn.copy.rows+5]
async fn described<C>(client: &C, sql: &str, whole_table: bool) -> Result<Vec<CopyColumn>, Error>
where
    C: GenericClient,
{
    let prepared = client.prepare(sql).await?;
    let mut columns: Vec<_> = prepared.columns().iter().collect();
    let table = columns.first().and_then(|column| column.table_oid());
    if let (true, Some(table)) = (whole_table, table) {
        let generated = client
            .query(
                "SELECT attnum FROM pg_catalog.pg_attribute \
                 WHERE attrelid = $1 AND attgenerated <> ''",
                &[&table],
            )
            .await?
            .iter()
            .map(|row| row.try_get::<_, i16>(0))
            .collect::<Result<Vec<_>, _>>()?;
        columns.retain(|column| {
            column
                .column_id()
                .is_none_or(|attribute| !generated.contains(&attribute))
        });
    }
    let mut described = Vec::with_capacity(columns.len());
    for column in columns {
        let type_ = completed_multiranges(client, column.type_()).await?;
        let type_ = type_.unwrap_or_else(|| column.type_().clone());
        described.push(CopyColumn::new(column.name(), type_));
    }
    if described.iter().any(|column| holds_money(column.type_())) {
        let places = money_places(client).await?;
        for column in &mut described {
            column.money_places = Some(places);
        }
    }
    Ok(described)
}

/// The binary format's signature, the first eleven bytes of its header.
const SIGNATURE: &[u8; 11] = b"PGCOPY\n\xff\r\n\0";

/// The header's length before its extension area: the signature, the flags
/// and the extension area's length.
const HEADER: usize = SIGNATURE.len() + 8;

/// The flags in the header's high half are ones a reader must understand —
/// bit 16 says each row carries an OID — and the format defines none other.
const CRITICAL_FLAGS: u32 = 0xFFFF_0000;

/// The binary format's writing side, over a COPY's columns: the header, each
/// row as its field count and each field's length and binary send form, and
/// the trailer.
///
/// Each field is written by its value's own [`ToSql`] for the column's type,
/// so a value that type cannot take is refused before anything is sent.
/// [`CopyRowSink`] is this over a [`CopyInSink`]; the bindings use it
/// directly, with their own values.
// [spec:pgorm:req:conn.copy.rows+5]
#[derive(Debug, Clone)]
pub struct CopyRowEncoder {
    columns: Vec<CopyColumn>,
}

impl CopyRowEncoder {
    /// An encoder for rows of `columns`, in order.
    pub fn new(columns: Vec<CopyColumn>) -> Self {
        Self { columns }
    }

    /// The columns each row holds.
    pub fn columns(&self) -> &[CopyColumn] {
        &self.columns
    }

    /// Append the header: the signature, no flags and an empty extension
    /// area. It comes once, before the first row.
    pub fn header(out: &mut BytesMut) {
        out.put_slice(SIGNATURE);
        out.put_u32(0);
        out.put_u32(0);
    }

    /// Append the trailer, a field count of -1. It comes once, after the last
    /// row.
    pub fn trailer(out: &mut BytesMut) {
        out.put_i16(-1);
    }

    /// Append `row`, one value for each column in order, each written in its
    /// column's binary form.
    ///
    /// # Errors
    ///
    /// [`Error::Type`] for a row of the wrong width, and for a value its
    /// column's type cannot take, naming the column. `out` is left as it was.
    pub fn encode<T: ToSql>(&self, row: &[T], out: &mut BytesMut) -> Result<(), Error> {
        let Some(count) = (row.len() == self.columns.len())
            .then(|| i16::try_from(row.len()).ok())
            .flatten()
        else {
            return Err(type_err(format!(
                "a COPY row of {} values for the COPY's {} columns ({})",
                row.len(),
                self.columns.len(),
                self.names()
            )));
        };
        let start = out.len();
        out.put_i16(count);
        for (value, column) in row.iter().zip(&self.columns) {
            if let Err(error) = field(value, column, out) {
                out.truncate(start);
                return Err(type_err(format!(
                    "COPY column `{}` of type `{}`: {error}",
                    column.name(),
                    column.type_()
                )));
            }
        }
        Ok(())
    }

    fn names(&self) -> String {
        self.columns
            .iter()
            .map(CopyColumn::name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// One field: its length, or -1 for NULL, and then its bytes.
fn field<T: ToSql>(value: &T, column: &CopyColumn, out: &mut BytesMut) -> Result<(), WireError> {
    let at = out.len();
    out.put_i32(0);
    let written = match column.money_places() {
        Some(places) => money_to_sql(value, column.type_(), places, out)?,
        None => value.to_sql_checked(column.type_(), out)?,
    };
    match written {
        IsNull::Yes => {
            out.truncate(at);
            out.put_i32(-1);
        }
        IsNull::No => {
            let length = i32::try_from(out.len() - at - 4)
                .map_err(|_| "the value is longer than a COPY field holds")?;
            out[at..at + 4].copy_from_slice(&length.to_be_bytes());
        }
    }
    Ok(())
}

/// Where a [`CopyRowDecoder`] is in the format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    Header,
    Rows,
    Ended,
}

/// The binary format's reading side, over a COPY's columns: fed the data in
/// chunks split anywhere, it checks the header and yields each row once all
/// of its bytes have arrived, until the trailer.
///
/// [`CopyRowStream`] is this over a [`CopyOutStream`]; the bindings use it
/// directly, decoding each field into their own values.
// [spec:pgorm:req:conn.copy.rows+5]
#[derive(Debug)]
pub struct CopyRowDecoder {
    columns: Vec<CopyColumn>,
    buffer: BytesMut,
    framing: Framing,
}

impl CopyRowDecoder {
    /// A decoder for rows of `columns`, in order.
    pub fn new(columns: Vec<CopyColumn>) -> Self {
        Self {
            columns,
            buffer: BytesMut::new(),
            framing: Framing::Header,
        }
    }

    /// The columns each row holds.
    pub fn columns(&self) -> &[CopyColumn] {
        &self.columns
    }

    /// Take the next chunk of the data.
    pub fn push(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    /// The next whole row's fields, each its bytes or `None` for SQL NULL;
    /// `None` until more data arrives, and after the trailer.
    ///
    /// # Errors
    ///
    /// [`Error::Query`] for data that is not the binary format: no signature,
    /// a header flag the format does not define, a row whose field count is
    /// not the COPY's column count, or data after the trailer.
    pub fn next_fields(&mut self) -> Result<Option<Vec<Option<Bytes>>>, Error> {
        if self.framing == Framing::Header && !self.header()? {
            return Ok(None);
        }
        if self.framing == Framing::Ended {
            return match self.buffer.is_empty() {
                true => Ok(None),
                false => Err(query_err("binary COPY data continues after its trailer")),
            };
        }
        let Some(length) = self.row_length()? else {
            return Ok(None);
        };
        let mut row = self.buffer.split_to(length).freeze();
        if row.get_i16() == -1 {
            self.framing = Framing::Ended;
            return self.next_fields();
        }
        let mut fields = Vec::with_capacity(self.columns.len());
        for _ in 0..self.columns.len() {
            let width = row.get_i32();
            fields.push(usize::try_from(width).ok().map(|width| row.split_to(width)));
        }
        Ok(Some(fields))
    }

    /// The next whole row, each field decoded by its column's type
    /// ([`CopyColumn::decode`]); `None` until more data arrives, and after the
    /// trailer.
    ///
    /// # Errors
    ///
    /// As [`next_fields`](Self::next_fields), and [`Error::Type`] for a field
    /// that does not decode.
    pub fn next_row(&mut self) -> Result<Option<Vec<Value>>, Error> {
        let Some(fields) = self.next_fields()? else {
            return Ok(None);
        };
        fields
            .iter()
            .zip(&self.columns)
            .map(|(raw, column)| column.decode(raw.as_deref()))
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    /// Check that the data, now ended, ended with the trailer.
    ///
    /// # Errors
    ///
    /// [`Error::Query`] when the data stopped before its trailer, part way
    /// through a row or before one.
    pub fn finish(&self) -> Result<(), Error> {
        match (self.framing, self.buffer.is_empty()) {
            (Framing::Ended, true) => Ok(()),
            (Framing::Ended, false) => {
                Err(query_err("binary COPY data continues after its trailer"))
            }
            _ => Err(query_err("binary COPY data ended before its trailer")),
        }
    }

    /// Consume the header once it has all arrived; `false` until then.
    fn header(&mut self) -> Result<bool, Error> {
        let available = self.buffer.len().min(SIGNATURE.len());
        if self.buffer[..available] != SIGNATURE[..available] {
            return Err(query_err(
                "binary COPY data does not begin with the PGCOPY signature",
            ));
        }
        if self.buffer.len() < HEADER {
            return Ok(false);
        }
        let mut header = &self.buffer[SIGNATURE.len()..HEADER];
        let flags = header.get_u32();
        if flags & CRITICAL_FLAGS != 0 {
            return Err(query_err(format!(
                "binary COPY data sets header flags {flags:#010x}, which this reader does not know"
            )));
        }
        let extension = usize::try_from(header.get_u32())
            .map_err(|_| query_err("binary COPY data has an invalid header extension length"))?;
        if self.buffer.len() < HEADER + extension {
            return Ok(false);
        }
        self.buffer.advance(HEADER + extension);
        self.framing = Framing::Rows;
        Ok(true)
    }

    /// The length of the next row, or of the trailer, once it has all
    /// arrived.
    fn row_length(&self) -> Result<Option<usize>, Error> {
        let mut rest = &self.buffer[..];
        if rest.len() < 2 {
            return Ok(None);
        }
        let count = rest.get_i16();
        if count == -1 {
            return Ok(Some(2));
        }
        if usize::try_from(count).ok() != Some(self.columns.len()) {
            return Err(query_err(format!(
                "a binary COPY row has {count} fields, but the COPY has {} columns",
                self.columns.len()
            )));
        }
        for _ in 0..self.columns.len() {
            if rest.len() < 4 {
                return Ok(None);
            }
            let width = rest.get_i32();
            if width == -1 {
                continue;
            }
            let Ok(width) = usize::try_from(width) else {
                return Err(query_err(format!(
                    "a binary COPY field has the length {width}"
                )));
            };
            if rest.len() < width {
                return Ok(None);
            }
            rest.advance(width);
        }
        Ok(Some(self.buffer.len() - rest.len()))
    }
}

/// A field's bytes, whatever its type, so an array's elements can be read by
/// the decoder of their own type.
struct Element<'a>(&'a [u8]);

impl<'a> FromSql<'a> for Element<'a> {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, WireError> {
        Ok(Self(raw))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

/// A field of `ty`, an array of a supported type included.
// [spec:pgorm:req:conn.copy.rows+5]
fn decode(ty: &Type, raw: Option<&[u8]>) -> Result<Value, WireError> {
    let ty = base_type(ty);
    let Kind::Array(member) = ty.kind() else {
        let kind = kind(ty)?;
        return match raw {
            None => Ok(kind.null()),
            Some(raw) => scalar(ty, &kind, raw),
        };
    };
    let member = base_type(member);
    let kind = kind(member)?;
    let Some(raw) = raw else {
        return Ok(Value::Array(kind, None));
    };
    // The driver's array codec refuses more than one dimension.
    let elements: Vec<Option<Element<'_>>> = FromSql::from_sql(ty, raw)?;
    let values = elements
        .into_iter()
        .map(|element| match element {
            None => Ok(kind.null()),
            Some(Element(raw)) => scalar(member, &kind, raw),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(kind, Some(Box::new(values))))
}

/// The built-in range type a range or multirange type is.
fn range_type(ty: &Type) -> Option<RangeType> {
    Some(match *ty {
        Type::INT4_RANGE | Type::INT4MULTI_RANGE => RangeType::Int4,
        Type::INT8_RANGE | Type::INT8MULTI_RANGE => RangeType::Int8,
        Type::NUM_RANGE | Type::NUMMULTI_RANGE => RangeType::Numeric,
        Type::DATE_RANGE | Type::DATEMULTI_RANGE => RangeType::Date,
        Type::TS_RANGE | Type::TSMULTI_RANGE => RangeType::Timestamp,
        Type::TSTZ_RANGE | Type::TSTZMULTI_RANGE => RangeType::TimestampTz,
        _ => return None,
    })
}

/// The value kind a non-array type reads as: a range or multirange type a
/// schema created reads as the `String` of its text form, the `Value` its
/// newtype is.
// [spec:pgorm:req:conn.copy.rows+5]
fn kind(ty: &Type) -> Result<ArrayType, WireError> {
    match ty.kind() {
        Kind::Enum(_) => return Ok(ArrayType::String),
        Kind::Range(_) | Kind::Multirange(_)
            if is_created_range(ty) || is_created_multirange(ty) =>
        {
            CreatedRangeCodec::of(ty)?;
            return Ok(ArrayType::String);
        }
        Kind::Range(_) | Kind::Multirange(_) => {
            let Some(range) = range_type(ty) else {
                return Err("no pgorm value holds this range type".into());
            };
            return Ok(match ty.kind() {
                Kind::Multirange(_) => ArrayType::Multirange(range),
                _ => ArrayType::Range(range),
            });
        }
        Kind::Array(_) => return Err("an array of arrays has no value".into()),
        _ => {}
    }
    Ok(match *ty {
        Type::BOOL => ArrayType::Bool,
        Type::CHAR => ArrayType::TinyInt,
        Type::INT2 => ArrayType::SmallInt,
        Type::INT4 => ArrayType::Int,
        Type::INT8 => ArrayType::BigInt,
        Type::OID => ArrayType::Unsigned,
        Type::FLOAT4 => ArrayType::Float,
        Type::FLOAT8 => ArrayType::Double,
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::XML | Type::UNKNOWN => {
            ArrayType::String
        }
        Type::BYTEA => ArrayType::Bytes,
        Type::JSON | Type::JSONB => ArrayType::Json,
        Type::UUID => ArrayType::Uuid,
        Type::DATE => ArrayType::Date,
        Type::TIME => ArrayType::Time,
        Type::TIMESTAMP => ArrayType::DateTime,
        Type::TIMESTAMPTZ => ArrayType::DateTimeWithTimeZone,
        Type::NUMERIC => ArrayType::Decimal,
        Type::INET | Type::CIDR => ArrayType::IpNetwork,
        Type::MACADDR => ArrayType::MacAddress,
        Type::INTERVAL => ArrayType::Interval,
        _ if matches!(ty.name(), "citext" | "ltree" | "lquery" | "ltxtquery") => ArrayType::String,
        _ if ty.name() == "vector" => ArrayType::Vector,
        _ => return Err("no pgorm value holds this type".into()),
    })
}

/// A non-NULL value of `kind`, from its binary form, through the driver's
/// codec for `ty` — the one pgorm's `TryGetable` decodes the same column
/// with.
fn scalar(ty: &Type, kind: &ArrayType, raw: &[u8]) -> Result<Value, WireError> {
    Ok(match kind {
        ArrayType::Bool => Value::Bool(Some(bool::from_sql(ty, raw)?)),
        ArrayType::TinyInt => Value::TinyInt(Some(i8::from_sql(ty, raw)?)),
        ArrayType::SmallInt => Value::SmallInt(Some(i16::from_sql(ty, raw)?)),
        ArrayType::Int => Value::Int(Some(i32::from_sql(ty, raw)?)),
        ArrayType::BigInt => Value::BigInt(Some(i64::from_sql(ty, raw)?)),
        ArrayType::Unsigned => Value::Unsigned(Some(u32::from_sql(ty, raw)?)),
        ArrayType::Float => Value::Float(Some(f32::from_sql(ty, raw)?)),
        ArrayType::Double => Value::Double(Some(f64::from_sql(ty, raw)?)),
        // An enum's label is sent as its text.
        ArrayType::String if matches!(ty.kind(), Kind::Enum(_)) => {
            Value::String(Some(Box::new(String::from_sql(&Type::TEXT, raw)?)))
        }
        ArrayType::String if matches!(ty.kind(), Kind::Range(_) | Kind::Multirange(_)) => {
            Value::String(Some(Box::new(CreatedRangeText::from_sql(ty, raw)?.0)))
        }
        ArrayType::String => Value::String(Some(Box::new(String::from_sql(ty, raw)?))),
        ArrayType::Bytes => Value::Bytes(Some(Box::new(Vec::<u8>::from_sql(ty, raw)?))),
        ArrayType::Json => Value::Json(Some(Box::new(FromSql::from_sql(ty, raw)?))),
        ArrayType::Uuid => Value::Uuid(Some(Box::new(FromSql::from_sql(ty, raw)?))),
        ArrayType::Date => Value::Date(Some(Box::new(Date::from_sql(ty, raw)?))),
        ArrayType::Time => Value::Time(Some(Box::new(Time::from_sql(ty, raw)?))),
        ArrayType::DateTime => Value::DateTime(Some(Box::new(DateTime::from_sql(ty, raw)?))),
        ArrayType::DateTimeWithTimeZone => {
            Value::DateTimeWithTimeZone(Some(Box::new(Timestamp::from_sql(ty, raw)?)))
        }
        // rust_decimal's own decoder rounds a `numeric` past what a
        // `Decimal` holds exactly, so a COPY TO would not write back.
        ArrayType::Decimal => Value::Decimal(Some(Box::new(ExactDecimal::from_sql(ty, raw)?.0))),
        ArrayType::IpNetwork => {
            let inet = wire::inet_from_sql(raw)?;
            Value::IpNetwork(Some(Box::new(IpNetwork::new(inet.addr(), inet.netmask())?)))
        }
        ArrayType::MacAddress => Value::MacAddress(Some(Box::new(MacAddress::new(
            wire::macaddr_from_sql(raw)?,
        )))),
        ArrayType::Vector => Value::Vector(Some(Box::new(Vector::from_sql(ty, raw)?))),
        ArrayType::Interval => Value::from(Interval::from_sql(ty, raw)?),
        ArrayType::Range(range) => {
            let Kind::Range(subtype) = ty.kind() else {
                return Err("expected a range".into());
            };
            Value::Range(*range, Some(Box::new(read_range(subtype, raw)?)))
        }
        ArrayType::Multirange(range) => {
            let Kind::Multirange(subtype) = ty.kind() else {
                return Err("expected a multirange".into());
            };
            Value::Multirange(*range, Some(Box::new(read_multirange(subtype, raw)?)))
        }
        // `kind` answers neither `BigUnsigned` nor `Char`, nor any kind it
        // does not name.
        _ => return Err("no PostgreSQL type reads as this kind".into()),
    })
}

/// A range, each bound read by the decoder of its subtype. The empty flag is
/// the empty range and nothing else; a NULL bound is a malformed message,
/// never no bound.
fn read_range(subtype: &Type, raw: &[u8]) -> Result<Range<Value>, WireError> {
    let subtype = base_type(subtype);
    let kind = kind(subtype)?;
    let bound = |bound: RangeBound<Option<&[u8]>>| -> Result<Bound<Value>, WireError> {
        Ok(match bound {
            RangeBound::Inclusive(Some(raw)) => Bound::Included(scalar(subtype, &kind, raw)?),
            RangeBound::Exclusive(Some(raw)) => Bound::Excluded(scalar(subtype, &kind, raw)?),
            RangeBound::Unbounded => Bound::Unbounded,
            RangeBound::Inclusive(None) | RangeBound::Exclusive(None) => {
                return Err("a range bound is NULL".into());
            }
        })
    };
    Ok(match wire::range_from_sql(raw)? {
        wire::Range::Empty => Range::Empty,
        wire::Range::Nonempty(lower, upper) => Range::new(bound(lower)?, bound(upper)?),
    })
}

/// A multirange: a count, then each range length-prefixed.
fn read_multirange(subtype: &Type, raw: &[u8]) -> Result<Multirange<Value>, WireError> {
    let mut raw = raw;
    if raw.len() < 4 {
        return Err("invalid message size".into());
    }
    let count = usize::try_from(raw.get_i32())?;
    let mut ranges = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        if raw.len() < 4 {
            return Err("invalid message size".into());
        }
        let length = usize::try_from(raw.get_i32())?;
        let (range, rest) = raw.split_at_checked(length).ok_or("invalid message size")?;
        raw = rest;
        ranges.push(read_range(subtype, range)?);
    }
    if !raw.is_empty() {
        return Err("invalid message size".into());
    }
    Ok(Multirange::from(ranges))
}

/// How much encoded data [`CopyRowSink`] gathers before it sends a chunk.
const GATHER: usize = 64 * 1024;

/// The rows of a binary `COPY … FROM STDIN`, written as [`Value`]s and
/// completed by [`finish`](CopyRowSink::finish); opened by
/// [`CopyTrait::copy_in_rows`](crate::CopyTrait::copy_in_rows).
///
/// Each row is checked against the COPY's [`columns`](Self::columns) as it
/// is written — its width, and each value against its column's type — and a
/// row refused is not written: the sink goes on taking rows. Rows are
/// gathered and sent in chunks. Dropping the sink unfinished aborts the COPY
/// as dropping a [`CopyInSink`] does: nothing is written.
// [spec:pgorm:req:conn.copy.rows+5]
#[derive(Debug)]
pub struct CopyRowSink<'a> {
    sink: CopyInSink<'a>,
    encoder: CopyRowEncoder,
    pending: BytesMut,
}

impl<'a> CopyRowSink<'a> {
    pub(crate) fn new(sink: CopyInSink<'a>, columns: Vec<CopyColumn>) -> Self {
        let mut pending = BytesMut::with_capacity(GATHER);
        CopyRowEncoder::header(&mut pending);
        Self {
            sink,
            encoder: CopyRowEncoder::new(columns),
            pending,
        }
    }

    /// The columns each row supplies, in order, with their types.
    pub fn columns(&self) -> &[CopyColumn] {
        self.encoder.columns()
    }

    /// Write one row, a value for each column in order.
    ///
    /// # Errors
    ///
    /// [`Error::Type`] for a row of the wrong width or a value its column's
    /// type cannot take, the row not written; and the COPY's failure if
    /// sending fails.
    pub async fn send<I>(&mut self, row: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = Value>,
    {
        let row: Vec<ValueHolder> = row.into_iter().map(ValueHolder).collect();
        self.encoder.encode(&row, &mut self.pending)?;
        if self.pending.len() >= GATHER {
            let chunk = self.pending.split().freeze();
            self.sink.send(chunk).await?;
        }
        Ok(())
    }

    /// Write `model` as one row, each column's value read from the model's
    /// field of that column name.
    ///
    /// # Errors
    ///
    /// [`Error::Type`] when the COPY has a column the model's entity does
    /// not, and as [`send`](Self::send) does.
    pub async fn send_model<M>(&mut self, model: &M) -> Result<(), Error>
    where
        M: ModelTrait,
    {
        let row = self
            .columns()
            .iter()
            .map(|column| {
                entity_column::<M::Entity>(column.name())
                    .map(|target| model.get(target))
                    .ok_or_else(|| {
                        type_err(format!(
                            "COPY column `{}` is no column of `{}`",
                            column.name(),
                            std::any::type_name::<M>()
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.send(row).await
    }

    /// Write the trailer and complete the COPY, returning the number of rows
    /// it inserted.
    ///
    /// # Errors
    ///
    /// As [`CopyInSink::finish`]: a row the server refused is reported here.
    pub async fn finish(mut self) -> Result<u64, Error> {
        CopyRowEncoder::trailer(&mut self.pending);
        let chunk = self.pending.split().freeze();
        self.sink.send(chunk).await?;
        self.sink.finish().await
    }
}

/// The entity column a COPY column of `name` is: the one whose name it is.
fn entity_column<E: EntityTrait>(name: &str) -> Option<E::Column> {
    E::Column::iter().find(|column| column.as_str() == name)
}

/// The rows of a binary `COPY … TO STDOUT`, as a [`Stream`] of rows of
/// [`Value`]s; opened by
/// [`CopyTrait::copy_out_rows`](crate::CopyTrait::copy_out_rows).
///
/// Each field is decoded by its column's type ([`CopyColumn::decode`]), and
/// [`models`](Self::models) reads each row into an entity's model instead.
/// Rows are pulled as the stream is polled, as a [`CopyOutStream`]'s chunks
/// are, and the stream ends after the last row; a failure, the server's or a
/// field that does not decode, is an `Err` item, after which it ends.
// [spec:pgorm:req:conn.copy.rows+5]
#[derive(Debug)]
pub struct CopyRowStream<'a> {
    stream: CopyOutStream<'a>,
    decoder: CopyRowDecoder,
    ended: bool,
}

impl<'a> CopyRowStream<'a> {
    pub(crate) fn new(stream: CopyOutStream<'a>, columns: Vec<CopyColumn>) -> Self {
        Self {
            stream,
            decoder: CopyRowDecoder::new(columns),
            ended: false,
        }
    }

    /// The columns each row holds, in order, with their types.
    pub fn columns(&self) -> &[CopyColumn] {
        self.decoder.columns()
    }

    /// Each row as a model of `E`: every column the entity has is set from
    /// the field of its name, and a column it does not have is left unread.
    /// A row missing one of the model's columns is [`Error::AttrNotSet`],
    /// and a field its model's type cannot hold is [`Error::Type`].
    pub fn models<E>(self) -> impl Stream<Item = Result<E::Model, Error>> + Send + 'a
    where
        E: EntityTrait,
        E::ActiveModel: TryIntoModel<E::Model> + Send,
    {
        let targets: Vec<Option<E::Column>> = self
            .columns()
            .iter()
            .map(|column| entity_column::<E>(column.name()))
            .collect();
        self.map(move |row| {
            let mut model = <E::ActiveModel as ActiveModelTrait>::default();
            for (value, target) in row?.into_iter().zip(&targets) {
                if let Some(target) = target {
                    model.set(*target, value)?;
                }
            }
            model.try_into_model()
        })
    }
}

impl Stream for CopyRowStream<'_> {
    type Item = Result<Vec<Value>, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        if this.ended {
            return Poll::Ready(None);
        }
        loop {
            match this.decoder.next_row() {
                Ok(Some(row)) => return Poll::Ready(Some(Ok(row))),
                Ok(None) => {}
                Err(error) => {
                    this.ended = true;
                    return Poll::Ready(Some(Err(error)));
                }
            }
            match ready!(this.stream.poll_next_unpin(cx)) {
                Some(Ok(chunk)) => this.decoder.push(&chunk),
                Some(Err(error)) => {
                    this.ended = true;
                    return Poll::Ready(Some(Err(error)));
                }
                None => {
                    this.ended = true;
                    return Poll::Ready(this.decoder.finish().err().map(Err));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns() -> Vec<CopyColumn> {
        vec![
            CopyColumn::new("id", Type::INT4),
            CopyColumn::new("name", Type::TEXT),
        ]
    }

    fn encoded(rows: &[Vec<Value>]) -> Result<BytesMut, Error> {
        let encoder = CopyRowEncoder::new(columns());
        let mut out = BytesMut::new();
        CopyRowEncoder::header(&mut out);
        for row in rows {
            let row: Vec<ValueHolder> = row.iter().cloned().map(ValueHolder).collect();
            encoder.encode(&row, &mut out)?;
        }
        CopyRowEncoder::trailer(&mut out);
        Ok(out)
    }

    fn rows() -> Vec<Vec<Value>> {
        vec![
            vec![Value::Int(Some(1)), "one".into()],
            vec![Value::Int(Some(2)), Value::String(None)],
        ]
    }

    /// Every row `data` decodes to, fed `step` bytes at a time.
    fn decoded(data: &[u8], step: usize) -> Result<Vec<Vec<Value>>, Error> {
        let mut decoder = CopyRowDecoder::new(columns());
        let mut rows = Vec::new();
        for chunk in data.chunks(step) {
            decoder.push(chunk);
            while let Some(row) = decoder.next_row()? {
                rows.push(row);
            }
        }
        decoder.finish()?;
        Ok(rows)
    }

    // [spec:pgorm:req:conn.copy.rows+5/test]    the framing reads rows split anywhere, a byte at a time included
    #[test]
    fn copy_rows_split_anywhere() -> Result<(), Error> {
        let data = encoded(&rows())?;
        assert_eq!(&data[..SIGNATURE.len()], SIGNATURE);
        for step in [1, 2, 3, 7, 19, 20, data.len()] {
            assert_eq!(decoded(&data, step)?, rows(), "{step} bytes at a time");
        }
        Ok(())
    }

    // [spec:pgorm:req:conn.copy.rows+5/test]    data that is not the format, or stops before its trailer, is refused
    #[test]
    fn copy_rows_malformed_data_refused() -> Result<(), Error> {
        let data = encoded(&rows())?;
        let refused =
            |data: &[u8]| matches!(decoded(data, data.len().max(1)), Err(Error::Query(_)));
        assert!(refused(b"PGCOPX\n\xff\r\n\0\0\0\0\0\0\0\0\0"));
        assert!(refused(&data[..data.len() - 2]));
        assert!(refused(&data[..data.len() - 5]));
        let mut trailing = data.clone();
        trailing.put_u8(0);
        assert!(refused(&trailing));
        let mut oids = data.clone();
        oids[SIGNATURE.len()..SIGNATURE.len() + 4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        assert!(refused(&oids));
        let mut narrow = BytesMut::new();
        CopyRowEncoder::header(&mut narrow);
        narrow.put_i16(1);
        narrow.put_i32(-1);
        assert!(refused(&narrow));

        // A header extension is skipped, and an unknown low flag ignored.
        let mut extended = BytesMut::new();
        extended.put_slice(SIGNATURE);
        extended.put_u32(1);
        extended.put_u32(3);
        extended.put_slice(b"ext");
        extended.put_slice(&data[HEADER..]);
        assert_eq!(decoded(&extended, 1)?, rows());
        Ok(())
    }

    // [spec:pgorm:req:conn.copy.rows+5/test]    a refused row leaves the encoder's output as it was
    #[test]
    fn copy_rows_refused_row_writes_nothing() {
        let encoder = CopyRowEncoder::new(columns());
        let mut out = BytesMut::from(&b"kept"[..]);
        let wrong = [
            ValueHolder(Value::Int(Some(1))),
            ValueHolder(Value::Int(Some(2))),
        ];
        assert!(matches!(
            encoder.encode(&wrong, &mut out),
            Err(Error::Type(_))
        ));
        let short = [ValueHolder(Value::Int(Some(1)))];
        assert!(matches!(
            encoder.encode(&short, &mut out),
            Err(Error::Type(_))
        ));
        assert_eq!(&out[..], b"kept");
    }

    fn multirange_column(name: &str, kind: Kind) -> CopyColumn {
        CopyColumn::new(
            name,
            Type::new(name.to_owned(), 16_400, kind, "geo".to_owned()),
        )
    }

    // [spec:pgorm:req:conn.copy.rows+5/test]    a multirange type a schema created, its kind completed, writes from its text and reads back as the text Display writes; the simple type the driver reports, and one over a subtype no RangeSubtype reads, have no value
    #[test]
    fn created_multirange_reads_as_its_text() -> Result<(), Error> {
        let reach = multirange_column("floatmultirange", Kind::Multirange(Type::FLOAT8));
        let encoder = CopyRowEncoder::new(vec![reach.clone()]);
        let mut decoder = CopyRowDecoder::new(vec![reach.clone()]);
        let mut data = BytesMut::new();
        CopyRowEncoder::header(&mut data);
        for text in ["{[1.5,2.5), [3,4]}", " {} ", "{[-Infinity,0]}"] {
            encoder.encode(&[ValueHolder(text.into())], &mut data)?;
        }
        encoder.encode(&[ValueHolder(Value::String(None))], &mut data)?;
        CopyRowEncoder::trailer(&mut data);
        decoder.push(&data);
        let mut read = Vec::new();
        while let Some(row) = decoder.next_row()? {
            read.extend(row);
        }
        decoder.finish()?;
        assert_eq!(
            read,
            [
                "{[1.5,2.5),[3,4]}".into(),
                "{}".into(),
                "{[-inf,0]}".into(),
                Value::String(None)
            ]
        );

        let empty = 0i32.to_be_bytes();
        let simple = multirange_column("floatmultirange", Kind::Simple);
        assert!(matches!(simple.decode(Some(&empty)), Err(Error::Type(_))));
        let nets = multirange_column("netmultirange", Kind::Multirange(Type::INET));
        assert!(matches!(
            nets.decode(Some(&empty)),
            Err(Error::Type(message)) if message.contains("over `inet` has no value")
        ));
        Ok(())
    }
}
