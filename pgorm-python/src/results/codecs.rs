//! Checked adapters around the PostgreSQL driver's existing Rust codecs.

use std::error::Error;

use pgorm::pgorm_query::{IpNetwork, MacAddress};
use tokio_postgres::types::{FromSql, Kind, Type};

type CodecError = Box<dyn Error + Send + Sync>;

/// Rust Value arrays cannot preserve extra dimensions or non-default bounds.
#[derive(Debug)]
pub(super) struct CheckedArray<T>(pub Vec<Option<T>>);

impl<'a, T: FromSql<'a>> FromSql<'a> for CheckedArray<T> {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, CodecError> {
        if !matches!(ty.kind(), Kind::Array(_)) {
            return Err("expected a PostgreSQL array".into());
        }
        pgorm_bindings::one_dimensional(raw)?;
        Vec::<Option<T>>::from_sql(ty, raw).map(Self)
    }

    fn accepts(ty: &Type) -> bool {
        <Vec<Option<T>> as FromSql>::accepts(ty)
    }
}

#[derive(Debug)]
pub(super) struct EnumLabel(pub String);

impl<'a> FromSql<'a> for EnumLabel {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, CodecError> {
        let label = std::str::from_utf8(raw)?;
        if !matches!(ty.kind(), Kind::Enum(labels) if labels.iter().any(|s| s == label)) {
            return Err("invalid enum label".into());
        }
        Ok(Self(label.to_owned()))
    }

    fn accepts(ty: &Type) -> bool {
        matches!(ty.kind(), Kind::Enum(_))
    }
}

/// These types lack FromSql; delegate to the same protocol codecs as TryGetable.
#[derive(Debug)]
pub(super) struct Inet(pub IpNetwork);

impl<'a> FromSql<'a> for Inet {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, CodecError> {
        let inet = postgres_protocol::types::inet_from_sql(raw)?;
        Ok(Self(IpNetwork::new(inet.addr(), inet.netmask())?))
    }

    fn accepts(ty: &Type) -> bool {
        matches!(*ty, Type::INET | Type::CIDR)
    }
}

#[derive(Debug)]
pub(super) struct Mac(pub MacAddress);

impl<'a> FromSql<'a> for Mac {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, CodecError> {
        Ok(Self(MacAddress::new(
            postgres_protocol::types::macaddr_from_sql(raw)?,
        )))
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::MACADDR
    }
}
