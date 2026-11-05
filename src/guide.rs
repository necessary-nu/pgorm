// Each chapter is a markdown file under docs/guide/, listed in reading order
// by docs/guide/chapters.json, and included as the documentation of an empty
// module: rustdoc renders it beside the API it links to and runs every example
// in it as a doctest. docs/guide/README.md is the guide's index.
#![doc = include_str!("../docs/guide/README.md")]
// [spec:pgorm:req:guide.rust]    the guide, chapter by chapter, beside the API
// [spec:pgorm:req:guide.chapters]    one module per listed chapter, in order

#[doc = include_str!("../docs/guide/01-getting-started.md")]
pub mod getting_started {}

#[doc = include_str!("../docs/guide/02-entities.md")]
pub mod entities {}

#[doc = include_str!("../docs/guide/03-writing.md")]
pub mod writing {}

#[doc = include_str!("../docs/guide/04-reading.md")]
pub mod reading {}

#[doc = include_str!("../docs/guide/05-pagination.md")]
pub mod pagination {}

#[doc = include_str!("../docs/guide/06-relations.md")]
pub mod relations {}

#[doc = include_str!("../docs/guide/07-transactions.md")]
pub mod transactions {}

#[doc = include_str!("../docs/guide/08-errors.md")]
pub mod errors {}

#[doc = include_str!("../docs/guide/09-raw-sql.md")]
pub mod raw_sql {}

#[doc = include_str!("../docs/guide/10-query-builder.md")]
pub mod query_builder {}

#[doc = include_str!("../docs/guide/11-pipeline.md")]
pub mod pipeline {}

#[doc = include_str!("../docs/guide/12-values.md")]
pub mod values {}

#[doc = include_str!("../docs/guide/13-schema.md")]
pub mod schema {}

#[doc = include_str!("../docs/guide/14-disjoint.md")]
pub mod disjoint {}

#[doc = include_str!("../docs/guide/15-one-of.md")]
pub mod one_of {}

#[doc = include_str!("../docs/guide/16-copy.md")]
pub mod copy {}

#[doc = include_str!("../docs/guide/17-listen-notify.md")]
pub mod listen_notify {}

#[doc = include_str!("../docs/guide/18-statement-caches.md")]
pub mod statement_caches {}

#[doc = include_str!("../docs/guide/19-configuration.md")]
pub mod configuration {}

#[doc = include_str!("../docs/guide/20-metrics.md")]
pub mod metrics {}

#[doc = include_str!("../docs/guide/21-migrations.md")]
pub mod migrations {}

#[doc = include_str!("../docs/guide/22-codegen.md")]
pub mod codegen {}

#[doc = include_str!("../docs/guide/23-postgres-releases.md")]
pub mod postgres_releases {}

#[doc = include_str!("../docs/guide/24-bindings.md")]
pub mod bindings {}
