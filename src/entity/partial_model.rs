use crate::{FromQueryResult, QuerySelect};

/// A trait for a part of [Model](super::model::ModelTrait)
// [spec:pgorm:def:entity.traits.from-query-result+5]
pub trait PartialModelTrait: FromQueryResult {
    /// Select specific columns this partial model needs.
    ///
    /// The return type is [`QuerySelect::Projected`], not `S`: a partial
    /// model that selects nothing cannot satisfy it, so a field-less
    /// `PartialModel` is a compile error rather than a query with an
    /// empty projection.
    fn select_cols<S: QuerySelect>(select: S) -> S::Projected;
}
