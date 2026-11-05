//! The files pgorm-codegen generates from `schema.sql`, a remark about a
//! topic or in reply to another remark, declared as the module tree they are
//! written into: the remark's `#[derive(OneOf)]` enum names its own table's
//! `Entity`, which its `Relation` enum's `SelfRef` makes `Related` to itself.

pub mod remark;
pub mod topic;
