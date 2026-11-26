//! Reading an array of a domain as an array of the type the domain is over.

use pgorm_driver::types::{FromSql, Kind, Type};
use pgorm_query::base_type;
use std::borrow::Cow;

/// An array read as an array of the type its elements' domain is over.
///
/// PostgreSQL reports a domain column by its base type, so a scalar
/// needs nothing, but an array of a domain by the domain's own array
/// type, whose element `postgres-types` checks by identity and refuses.
/// A domain's values are its base type's on the wire, so the array is
/// read as though its elements were that type.
// [spec:pgorm:def:exec.decode.array+2]
#[derive(Debug)]
pub(super) struct Transparent<T>(pub(super) T);

/// `ty` with every domain under it replaced by the type it is over.
// [spec:pgorm:def:exec.decode.array+2]
fn without_domains(ty: &Type) -> Cow<'_, Type> {
    let ty = base_type(ty);
    let Kind::Array(member) = ty.kind() else {
        return Cow::Borrowed(ty);
    };
    let element = without_domains(member);
    if std::ptr::eq(element.as_ref(), member) {
        return Cow::Borrowed(ty);
    }
    Cow::Owned(Type::new(
        ty.name().to_owned(),
        ty.oid(),
        Kind::Array(element.into_owned()),
        ty.schema().to_owned(),
    ))
}

impl<'a, T: FromSql<'a>> FromSql<'a> for Transparent<T> {
    fn from_sql(
        ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        T::from_sql(&without_domains(ty), raw).map(Self)
    }

    fn accepts(ty: &Type) -> bool {
        T::accepts(&without_domains(ty))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use pgorm_driver::types::ToSql;

    fn named(name: &str, kind: Kind) -> Type {
        Type::new(name.to_owned(), 16_384, kind, "public".to_owned())
    }

    // [spec:pgorm:def:exec.decode.array+2/test]    an array of a domain, of a domain over a
    // domain included, is read as an array of the type at the bottom, and a type with no
    // domain in it is passed through as it is
    #[test]
    fn reads_a_domain_array_as_its_base() {
        let quantity = named("quantity", Kind::Domain(Type::INT4));
        let small = named("small_quantity", Kind::Domain(quantity.clone()));
        let array = named("_small_quantity", Kind::Array(small));
        assert!(!<Vec<i32> as FromSql>::accepts(&array));
        assert!(<Transparent<Vec<i32>> as FromSql>::accepts(&array));
        assert!(!<Transparent<Vec<i64>> as FromSql>::accepts(&array));
        assert_eq!(without_domains(&array).kind(), &Kind::Array(Type::INT4));
        assert!(matches!(
            without_domains(&Type::INT4_ARRAY),
            Cow::Borrowed(_)
        ));

        let mut raw = BytesMut::new();
        vec![1i32, 2, 3]
            .to_sql(&Type::INT4_ARRAY, &mut raw)
            .expect("an int4 array encodes");
        let read = <Transparent<Vec<i32>> as FromSql>::from_sql(&array, &raw)
            .expect("the domain array decodes");
        assert_eq!(read.0, [1, 2, 3]);
    }
}
