//! The `CHECK` holding a `OneOf` reference to one target per row.

use crate::{EntityName, OneOf, Schema, executor::result_name::derived_name};
use pgorm_query::{Check, Expr, Func, Name, Table, TableAlterStatement};

impl Schema {
    /// The `ALTER TABLE` adding the `CHECK` that holds every row of a
    /// [`OneOf`] reference's entity to one target: `num_nonnulls` of the
    /// targets' columns `= 1`, or `<= 1` where the enum has a unit variant,
    /// so PostgreSQL refuses a row referencing two targets, and one
    /// referencing none unless the enum says a row may.
    ///
    /// The constraint is named `ck-{table}-{enum}`, the table bare and the
    /// enum's name in snake_case, bounded to PostgreSQL's 63 bytes as the
    /// names schema generation gives its keys are. The foreign keys are the
    /// entity's relations' own, which
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
    // [spec:pgorm:def:disjoint.one-of+2]
    pub fn create_one_of<O: OneOf>(&self) -> TableAlterStatement {
        let entity = <O::Entity as Default>::default();
        let set = O::columns().into_iter().fold(
            Func::named(Name::runtime("num_nonnulls")),
            |call, column| call.arg(Expr::col(column)),
        );
        let check = match O::none() {
            Some(_) => Expr::expr(set).lte(1),
            None => Expr::expr(set).eq(1),
        };
        let name = derived_name("ck", entity.table_name(), [O::name()]);
        Table::alter(entity.table_ref()).add_check(Check::new(check).name(Name::runtime(name)))
    }
}
