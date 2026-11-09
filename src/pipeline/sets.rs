//! The pipeline's set operations.
//!
//! All three are relation-level transforms in the fork's std, the same ones
//! PRQL text's `append`, `intersect` and `remove` call: the operands are
//! combined whole, by column position, with nothing joined over their
//! columns — so neither operand's columns have to be known, a column count
//! both projections show is refused, and a chain renders with the brackets
//! SQL's precedence needs. Upstream PRQL's `intersect` and `remove` are joins
//! over every column pair instead, which can neither expand a wildcard
//! operand nor tell a short operand from a matching one; the fork departs
//! from upstream there on purpose.

use super::adapter::{self, PlExpr};

/// A pipeline set operation: `append` is `UNION ALL`, `intersect` is
/// `INTERSECT ALL`, and `remove` is `EXCEPT ALL`.
// [spec:pgorm:req:pipeline.compose+2]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SetOperation {
    Append,
    Intersect,
    Remove,
}

impl SetOperation {
    const ALL: [SetOperation; 3] = [
        SetOperation::Append,
        SetOperation::Intersect,
        SetOperation::Remove,
    ];

    /// The std transform the stage calls, which is also the pipeline
    /// method's name an error names the stage by.
    pub(super) fn verb(self) -> &'static str {
        match self {
            SetOperation::Append => "append",
            SetOperation::Intersect => "intersect",
            SetOperation::Remove => "remove",
        }
    }

    /// The set operation a stage is, if it is one.
    pub(super) fn of_stage(stage: &PlExpr) -> Option<Self> {
        let verb = adapter::stage_verb(stage)?;
        Self::ALL
            .into_iter()
            .find(|operation| operation.verb() == verb)
    }
}
