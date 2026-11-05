//! The `CHECK` holding a `OneOf` reference to one target per row.

use crate::{OneOf, OneOfShape, Schema};
use pgorm_query::TableAlterStatement;

impl Schema {
    /// The `ALTER TABLE` adding the `CHECK` that holds every row of a
    /// [`OneOf`] reference's entity to one target, [`OneOfShape::check`] of
    /// [`OneOfShape::of::<O>`](OneOfShape::of): `num_nonnulls` of the
    /// targets' columns `= 1`, or `<= 1` where the enum has a unit variant,
    /// so PostgreSQL refuses a row referencing two targets, and one
    /// referencing none unless the enum says a row may.
    ///
    /// The constraint is named `ck-{table}-{enum}`, the table bare and the
    /// enum's name in snake_case, or `ck-{enum}` where that would pass
    /// PostgreSQL's 63 bytes: a `CHECK`'s name is its table's alone, so the
    /// table it is on says what the name leaves out, and the enum's name,
    /// which pgorm-codegen names the enum it generates for, is kept whole.
    /// The derive refuses an enum whose name `ck-{enum}` could not hold; one
    /// implemented by hand is bounded as the names schema generation gives
    /// its keys are. The foreign keys are the entity's relations' own, which
    /// [`create_table_from_entity`](Self::create_table_from_entity) writes.
    ///
    /// ```
    /// use pgorm::{Schema, tests_cfg::comment::Commentable};
    ///
    /// assert_eq!(
    ///     Schema::new().create_one_of::<Commentable>().to_string(),
    ///     r#"ALTER TABLE "comment" ADD CONSTRAINT "ck-comment-commentable" CHECK (num_nonnulls("post_id", "photo_id") <= 1)"#
    /// );
    /// ```
    // [spec:pgorm:def:disjoint.one-of+4]
    pub fn create_one_of<O: OneOf>(&self) -> TableAlterStatement {
        OneOfShape::of::<O>().check()
    }
}
