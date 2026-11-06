//! The PostgreSQL release a build targets.

/// The PostgreSQL major release this build targets: 18, the floor and the
/// default, or 19 with the `pg-19` feature.
///
/// The typed surface follows it, as do the answers the documentation and the
/// live suite hold where the two releases' servers differ. A crate cannot read
/// another crate's features through `cfg`, so a dependent that relies on 19
/// holds the build to it here, before anything runs. Built for 18 this does not
/// compile; built with `pg-19` it does:
///
#[cfg_attr(not(feature = "pg-19"), doc = "```compile_fail,E0080")]
#[cfg_attr(feature = "pg-19", doc = "```")]
/// const _: () = assert!(
///     pgorm_query::POSTGRES_TARGET >= 19,
///     "pgorm has to be built with its pg-19 feature",
/// );
/// ```
// [spec:pgorm:req:sql.target+2]
pub const POSTGRES_TARGET: u32 = if cfg!(feature = "pg-19") { 19 } else { 18 };
