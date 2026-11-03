//! The entities pgorm-codegen generates for keys the database fills or does
//! not fill, declared as the module tree they are written into, so the
//! attributes the writer gives each key column are read by the derive: a
//! serial column inside a composite key, a serial column beside a key of its
//! own, and an identity key.

pub mod badge;
pub mod coded_row;
pub mod tenant_order;
