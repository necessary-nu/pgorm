use pgorm::entity::prelude::*;
use pgorm::tests_cfg::{photo, post};

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum NamedFields {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId)]
    Post { post: post::Entity },
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum TwoFields {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId)]
    Post(post::Entity, photo::Entity),
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
#[repr(u8)]
pub enum Discriminated {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId)]
    Post(post::Entity),
    Detached = 1,
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum NoColumn {
    Post(post::Entity),
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum UnitColumn {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId)]
    Post(post::Entity),
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PhotoId)]
    Detached,
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum TwoUnits {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId)]
    Post(post::Entity),
    Detached,
    Orphaned,
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum PayloadKey {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId)]
    Post(#[pgorm(ignore)] post::Entity),
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum QuotedColumn {
    #[pgorm(column = "pgorm::tests_cfg::comment::Column::PostId")]
    Post(post::Entity),
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum UnknownVariantKey {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId, value = "post")]
    Post(post::Entity),
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum ColumnTwice {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId, column = pgorm::tests_cfg::comment::Column::PostId)]
    Post(post::Entity),
}

fn main() {}
