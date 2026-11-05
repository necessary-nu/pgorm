//! The identifier render oracle: a registry of every public API that renders
//! a caller-supplied name into SQL text, a hostile-name corpus, and the
//! structural property each site is held to. See `docs/spec/ident-oracle.md`.

pub mod corpus;
pub mod fixtures;
pub mod live;
pub mod live_capture;
pub mod oracle;
pub mod pins;
pub mod registry;
pub mod registry_ddl;
pub mod registry_disjoint;
pub mod registry_orm;
