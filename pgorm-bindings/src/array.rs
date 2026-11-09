//! A PostgreSQL array as a pgorm-query `Value` holds one: a single dimension
//! whose first element is at index 1.

use std::error::Error;

use fallible_iterator::FallibleIterator;
use postgres_protocol::types::{Array, array_from_sql};

/// `raw`, an array's wire form, read; refused unless it has at most one
/// dimension, starting at 1, since a `Value` array keeps neither another
/// dimension nor another bound and would hand the language a different array.
// [spec:pgorm:req:bindings.values]
pub fn one_dimensional(raw: &[u8]) -> Result<Array<'_>, Box<dyn Error + Send + Sync>> {
    let array = array_from_sql(raw)?;
    let dimensions = array.dimensions().collect::<Vec<_>>()?;
    if dimensions.len() > 1 || dimensions.first().is_some_and(|d| d.lower_bound != 1) {
        return Err("only one-dimensional arrays with lower bound 1 are supported".into());
    }
    Ok(array)
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use postgres_protocol::{
        IsNull,
        types::{ArrayDimension, array_to_sql, int4_to_sql},
    };

    use super::*;

    fn wire(dimensions: &[(i32, i32)], elements: &[i32]) -> BytesMut {
        let mut buf = BytesMut::new();
        let dimensions = dimensions
            .iter()
            .map(|&(len, lower_bound)| ArrayDimension { len, lower_bound });
        array_to_sql(
            dimensions,
            23,
            elements.iter().copied(),
            |element, buf| {
                int4_to_sql(element, buf);
                Ok(IsNull::No)
            },
            &mut buf,
        )
        .expect("the array encodes");
        buf
    }

    // [spec:pgorm:req:bindings.values/test]    an array of another shape is refused rather than
    // flattened or rebased
    #[test]
    fn only_a_one_dimensional_array_is_read() {
        let refused = |raw: &[u8]| one_dimensional(raw).err().map(|error| error.to_string());
        assert_eq!(refused(&wire(&[(2, 1)], &[4, 5])), None);
        assert_eq!(refused(&wire(&[], &[])), None);
        let message =
            Some("only one-dimensional arrays with lower bound 1 are supported".to_owned());
        assert_eq!(refused(&wire(&[(2, 0)], &[4, 5])), message);
        assert_eq!(refused(&wire(&[(1, 1), (2, 1)], &[4, 5])), message);
    }
}
