#![allow(unused_imports, dead_code)]

//! `UPDATE .. FROM` and `DELETE .. USING` against a live PostgreSQL server.
//!
//! The render tests in pgorm-query hold the SQL text and feed it to the real
//! grammar; what they cannot see is whether the relation a write statement
//! introduces actually joins — whether the rows the server touches are the
//! rows the predicate names. That is what these exercise: each case asserts
//! both the affected-row count the statement reports and the table state it
//! leaves behind, because a clause that silently degenerated into a cross
//! product would still report a plausible count.

pub mod common;
pub use common::{TestContext, bakery_chain::*, setup::*};
use pgorm::entity::prelude::*;
use pgorm::{DatabaseConnection, entity::*, query::*};
use pgorm_query::{Expr, FromItem, Name, Query};

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("write_relation_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    update_from_reads_the_joined_table(&db).await?;
    delete_using_narrows_on_the_joined_table(&db).await?;
    update_from_a_subquery_relation(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

/// Two bakeries, and cakes belonging to each. Cakes go first: `cake.bakery_id`
/// is `ON DELETE SET NULL`, so dropping the bakeries alone would leave the
/// previous case's cakes behind with a null owner.
async fn seed(db: &DatabaseConnection) -> Result<(i32, i32), Error> {
    Delete::many(cake::Entity).exec(db).await?;
    Delete::many(bakery::Entity).exec(db).await?;

    let seaside = bakery::ActiveModel {
        name: set("Seaside Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    }
    .insert(db)
    .await?;

    let inland = bakery::ActiveModel {
        name: set("Inland Bakery"),
        profit_margin: set(5.1),
        ..Default::default()
    }
    .insert(db)
    .await?;

    for (name, bakery_id) in [
        ("Cheesecake", seaside.id),
        ("Chocolate", seaside.id),
        ("Mud Cake", inland.id),
    ] {
        cake::ActiveModel {
            name: set(name),
            price: set(rust_dec(10.25)),
            bakery_id: set(Some(bakery_id)),
            gluten_free: set(false),
            serial: set(Uuid::new_v4()),
            ..Default::default()
        }
        .insert(db)
        .await?;
    }

    Ok((seaside.id, inland.id))
}

/// `UPDATE cake SET name = .. FROM bakery WHERE cake.bakery_id = bakery.id`
/// writes a column of the joined relation into the target, and touches only
/// the rows the join predicate matches.
// [spec:pgorm:req:sql.ast.update+5/test]
// [spec:pgorm:req:sql.render.update-delete+3/test]
// [spec:pgorm:sem:query.build.update+5/test]
pub async fn update_from_reads_the_joined_table(db: &DatabaseConnection) -> Result<(), Error> {
    let (seaside_id, _inland_id) = seed(db).await?;

    // A fourth cake with no bakery: the join must leave it alone, which is the
    // assertion a cross product would fail.
    cake::ActiveModel {
        name: set("Orphan"),
        price: set(rust_dec(1.00)),
        bakery_id: set(None),
        gluten_free: set(false),
        serial: set(Uuid::new_v4()),
        ..Default::default()
    }
    .insert(db)
    .await?;

    let affected = Update::many(cake::Entity)
        .col_expr(
            cake::Column::Name,
            Expr::col((bakery::Entity, bakery::Column::Name)).into(),
        )
        .from(bakery::Entity)
        .filter(cake::Column::BakeryId.eq_col(bakery::Column::Id))
        .filter(bakery::Column::Id.eq(seaside_id))
        .exec(db)
        .await?;

    assert_eq!(affected, 2, "only the two Seaside cakes are matched");

    let mut names: Vec<(String, Option<i32>)> = Cake::find()
        .all(db)
        .await?
        .into_iter()
        .map(|c| (c.name, c.bakery_id))
        .collect();
    names.sort();

    assert_eq!(
        names,
        [
            ("Mud Cake".to_owned(), Some(_inland_id)),
            ("Orphan".to_owned(), None),
            ("Seaside Bakery".to_owned(), Some(seaside_id)),
            ("Seaside Bakery".to_owned(), Some(seaside_id)),
        ]
    );

    Ok(())
}

/// `DELETE FROM cake USING bakery WHERE ..` deletes exactly the target rows
/// the joined relation qualifies, and leaves the joined table itself intact —
/// the property that separates USING from a two-table delete.
// [spec:pgorm:def:sql.ast.delete+4/test]
// [spec:pgorm:req:sql.render.update-delete+3/test]
// [spec:pgorm:sem:query.build.delete+4/test]
pub async fn delete_using_narrows_on_the_joined_table(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let (seaside_id, inland_id) = seed(db).await?;

    let affected = Delete::many(cake::Entity)
        .using(bakery::Entity)
        .filter(cake::Column::BakeryId.eq_col(bakery::Column::Id))
        .filter(bakery::Column::Name.eq("Seaside Bakery"))
        .exec(db)
        .await?;

    assert_eq!(affected, 2, "both Seaside cakes go");

    let remaining: Vec<(String, Option<i32>)> = Cake::find()
        .all(db)
        .await?
        .into_iter()
        .map(|c| (c.name, c.bakery_id))
        .collect();
    assert_eq!(remaining, [("Mud Cake".to_owned(), Some(inland_id))]);

    // USING introduces a relation to read, not one to write: bakery is untouched.
    let bakeries = Bakery::find().all(db).await?;
    assert_eq!(bakeries.len(), 2);
    assert!(bakeries.iter().any(|b| b.id == seaside_id));

    Ok(())
}

/// The relation list takes SELECT's whole currency, so a subquery — with its
/// own aggregate and its own bound parameter — stands where a table stands,
/// and its parameters renumber into the update's parameter space.
// [spec:pgorm:req:sql.ast.update+5/test]
// [spec:pgorm:req:sql.render.update-delete+3/test]
pub async fn update_from_a_subquery_relation(db: &DatabaseConnection) -> Result<(), Error> {
    let (seaside_id, _) = seed(db).await?;

    let counts = Query::select()
        .expr_as(Expr::col(cake::Column::BakeryId), Name::runtime("owner_id"))
        .expr_as(
            Expr::col(cake::Column::Id).count(),
            Name::runtime("cake_count"),
        )
        .from(cake::Entity)
        .group_by_col(cake::Column::BakeryId)
        .take();

    let affected = Update::many(bakery::Entity)
        .col_expr(
            bakery::Column::ProfitMargin,
            Expr::col((Name::runtime("c"), Name::runtime("cake_count"))).into(),
        )
        .from(FromItem::SubQuery(counts, Name::runtime("c")))
        .filter(
            Expr::col((bakery::Entity, bakery::Column::Id))
                .equals((Name::runtime("c"), Name::runtime("owner_id"))),
        )
        .filter(bakery::Column::Id.eq(seaside_id))
        .exec(db)
        .await?;

    assert_eq!(affected, 1);

    let seaside = Bakery::find_by_id(seaside_id).one(db).await?;
    assert_eq!(seaside.profit_margin, 2.0, "Seaside owns two cakes");

    Ok(())
}
