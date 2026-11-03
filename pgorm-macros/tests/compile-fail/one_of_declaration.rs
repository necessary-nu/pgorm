use pgorm::entity::prelude::*;
use pgorm::tests_cfg::post;

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub struct NotAnEnum {
    post: post::Entity,
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum Generic<T> {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId)]
    Post(post::Entity),
    Other(T),
}

#[derive(OneOf)]
pub enum NoEntity {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId)]
    Post(post::Entity),
}

#[derive(OneOf)]
#[pgorm(entity = "pgorm::tests_cfg::comment::Entity")]
pub enum QuotedEntity {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId)]
    Post(post::Entity),
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity, base = pgorm::tests_cfg::comment::Entity)]
pub enum UnknownKey {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId)]
    Post(post::Entity),
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity, entity = pgorm::tests_cfg::comment::Entity)]
pub enum EntityTwice {
    #[pgorm(column = pgorm::tests_cfg::comment::Column::PostId)]
    Post(post::Entity),
}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum NoVariants {}

#[derive(OneOf)]
#[pgorm(entity = pgorm::tests_cfg::comment::Entity)]
pub enum NoTarget {
    Detached,
}

fn main() {}
