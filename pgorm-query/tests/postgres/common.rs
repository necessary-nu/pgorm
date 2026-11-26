pub use std::fmt::Write as FmtWrite;

use pgorm_query::SqlName;

/// Representation of a database table named `BloB`.
///
/// A `Enum` implemented [`SqlName`] used in rustdoc and test to demonstrate the library usage.
///
/// [`SqlName`]: crate::types::SqlName
#[derive(Debug)]
#[allow(dead_code)]
pub enum BinaryType {
    Table,
    BinaryLen,
    Binary,
    BlobSize,
    TinyBlob,
    Blob,
    MediumBlob,
    LongBlob,
}

impl SqlName for BinaryType {
    fn unquoted(&self, s: &mut dyn FmtWrite) {
        write!(
            s,
            "{}",
            match self {
                Self::Table => "binary_type",
                Self::BinaryLen => "binlen",
                Self::Binary => "bin",
                Self::BlobSize => "defb",
                Self::TinyBlob => "tb",
                Self::Blob => "b",
                Self::MediumBlob => "mb",
                Self::LongBlob => "lb",
            }
        )
        .unwrap();
    }
}
