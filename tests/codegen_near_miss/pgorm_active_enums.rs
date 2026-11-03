use pgorm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
pub enum ShelfKind {
    Book,
    Map,
}
