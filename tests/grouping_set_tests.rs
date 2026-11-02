#![allow(unused_imports, dead_code)]

//! `GROUPING SETS`, `ROLLUP`, `CUBE` and `GROUPING()`, against a live
//! PostgreSQL server.
//!
//! The render tests settle that each element parses to a `GroupingSet` node
//! of the right kind. What they cannot settle is how many rows each yields,
//! and every wrong rendering here still parses: a `ROLLUP` flattened into a
//! plain list groups once instead of three times, a `CUBE` spelled as a
//! `ROLLUP` loses the per-column subtotals, and a composite unit written
//! without its parentheses becomes two levels of the hierarchy. So every case
//! runs over the same four rows and asserts the row count per `GROUPING()`
//! bitmask — which also pins the bitmask's bit order — with a plain
//! `GROUP BY` as the control. The table carries a NULL region in its data,
//! which is the NULL `GROUPING()` exists to tell apart from the ones a
//! subtotal row puts there.

use std::collections::BTreeMap;

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Expr, Func, Grouping, GroupingElement, Name, Order, OrderedStatement, Query, SelectStatement,
    SimpleExpr,
};
use pgorm::{ConnectionTrait, entity::prelude::*};
use tokio_postgres::error::SqlState;

/// Four sales: three regions once NULL counts as one, two products, and a
/// grand total of 100.
const SEED: &str = "
    CREATE TABLE sales (region text, product text, amount int);
    INSERT INTO sales (region, product, amount) VALUES
        ('north', 'apple', 10),
        ('north', 'pear',  20),
        ('south', 'apple', 30),
        (NULL,    'pear',  40);
    CREATE TABLE nothing (amount int);
";

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("grouping_set_tests").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SEED).await?;

    a_plain_group_by_is_the_control(&db).await?;
    rollup_adds_the_prefix_subtotals(&db).await?;
    cube_adds_every_subset(&db).await?;
    grouping_sets_group_by_exactly_the_sets_named(&db).await?;
    grouping_tells_subtotal_null_from_data_null(&db).await?;
    a_tuple_is_one_unit_of_a_rollup(&db).await?;
    element_beside_plain_expression_crosses_with_it(&db).await?;
    an_empty_rollup_is_the_grand_total(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn region() -> Expr {
    Expr::col(Name::runtime("region"))
}

fn product() -> Expr {
    Expr::col(Name::runtime("product"))
}

/// `GROUPING(region, product)`: bit 1 set when the row's set leaves `region`
/// out, bit 0 when it leaves `product` out.
fn mask() -> Grouping {
    Func::grouping(region()).arg(product())
}

/// `SELECT GROUPING(region, product), SUM(amount) FROM sales GROUP BY …`.
fn grouped(group: impl FnOnce(&mut SelectStatement) -> &mut SelectStatement) -> SelectStatement {
    let mut query = Query::select()
        .expr(mask())
        .expr(Func::sum(Expr::col(Name::runtime("amount"))))
        .from(Name::runtime("sales"))
        .take();
    group(&mut query);
    query
}

/// Rows per `GROUPING()` bitmask, and the grand total of every row's sum at
/// each mask.
async fn masks(
    db: &DatabaseConnection,
    query: &SelectStatement,
) -> Result<BTreeMap<i32, (usize, i64)>, Error> {
    let rows = db.query_all(query.to_string().as_str(), &[]).await?;
    let mut counted = BTreeMap::new();
    for row in rows {
        let mask: i32 = row.get(0);
        let sum: i64 = row.get(1);
        let entry = counted.entry(mask).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += sum;
    }
    Ok(counted)
}

/// A plain `GROUP BY region, product`: the four (region, product) groups,
/// none of them a subtotal.
// [spec:pgorm:def:sql.ast.select.grouping/test]    against a live server: the plain group-by
// methods still group once
async fn a_plain_group_by_is_the_control(db: &DatabaseConnection) -> Result<(), Error> {
    let query = grouped(|q| q.add_group_by([region().into(), product().into()]));

    assert!(
        query
            .to_string()
            .ends_with(r#"GROUP BY "region", "product""#)
    );
    assert_eq!(masks(db, &query).await?, BTreeMap::from([(0, (4, 100))]));

    Ok(())
}

/// `ROLLUP (region, product)`: the four groups, a subtotal for each of the
/// three regions (mask 1), and one grand total (mask 3) — eight rows where
/// the plain list gives four. No row groups by product alone (mask 2).
// [spec:pgorm:def:sql.ast.select.grouping/test]    against a live server: ROLLUP groups by each
// prefix
// [spec:pgorm:req:sql.render.grouping+1/test]
// [spec:pgorm:req:sql.scope+23/test]
async fn rollup_adds_the_prefix_subtotals(db: &DatabaseConnection) -> Result<(), Error> {
    let query = grouped(|q| q.group_by_element(GroupingElement::rollup([region(), product()])));

    assert!(
        query
            .to_string()
            .ends_with(r#"GROUP BY ROLLUP ("region", "product")"#)
    );
    assert_eq!(
        masks(db, &query).await?,
        BTreeMap::from([(0, (4, 100)), (1, (3, 100)), (3, (1, 100))])
    );

    Ok(())
}

/// `CUBE (region, product)`: everything `ROLLUP` gives, plus a subtotal for
/// each of the two products (mask 2) — ten rows.
// [spec:pgorm:def:sql.ast.select.grouping/test]    against a live server: CUBE groups by each
// subset
// [spec:pgorm:req:sql.render.grouping+1/test]
async fn cube_adds_every_subset(db: &DatabaseConnection) -> Result<(), Error> {
    let query = grouped(|q| q.group_by_element(GroupingElement::cube([region(), product()])));

    assert_eq!(
        masks(db, &query).await?,
        BTreeMap::from([(0, (4, 100)), (1, (3, 100)), (2, (2, 100)), (3, (1, 100))])
    );

    Ok(())
}

/// `GROUPING SETS ((region), (product))`: the region subtotals and the
/// product subtotals and nothing else — no (region, product) groups and no
/// grand total — five rows.
// [spec:pgorm:def:sql.ast.select.grouping/test]    against a live server: GROUPING SETS groups by
// exactly the sets it names
// [spec:pgorm:req:sql.render.grouping+1/test]
async fn grouping_sets_group_by_exactly_the_sets_named(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let query = grouped(|q| {
        q.group_by_element(
            GroupingElement::sets(GroupingElement::set([region()]))
                .add(GroupingElement::set([product()])),
        )
    });

    assert!(
        query
            .to_string()
            .ends_with(r#"GROUP BY GROUPING SETS ("region", "product")"#)
    );
    assert_eq!(
        masks(db, &query).await?,
        BTreeMap::from([(1, (3, 100)), (2, (2, 100))])
    );

    // Adding the empty set adds the grand total, and a nested ROLLUP
    // contributes all three of its own sets.
    let query = grouped(|q| {
        q.group_by_element(
            GroupingElement::sets(GroupingElement::rollup([region(), product()]))
                .add(GroupingElement::set([product()]))
                .add(GroupingElement::empty()),
        )
    });
    assert_eq!(
        masks(db, &query).await?,
        BTreeMap::from([(0, (4, 100)), (1, (3, 100)), (2, (2, 100)), (3, (2, 200))]),
        "GROUPING SETS keeps the duplicate grand total its two sources each give"
    );

    Ok(())
}

/// Under `ROLLUP (region, product)` two rows carry a NULL region: the
/// (NULL, pear) group and the NULL region's subtotal, both from the data,
/// and the grand total, which drops the column. `GROUPING(region)` is 0 for
/// the first two and 1 for the last — the distinction `IS NULL` cannot make.
// [spec:pgorm:def:sql.ast.func+12/test]    against a live server: GROUPING() reads which columns
// a row's set left out
async fn grouping_tells_subtotal_null_from_data_null(db: &DatabaseConnection) -> Result<(), Error> {
    let query = Query::select()
        .expr(Func::grouping(region()))
        .expr(Func::sum(Expr::col(Name::runtime("amount"))))
        .from(Name::runtime("sales"))
        .and_having(Expr::expr(region()).is_null())
        .group_by_element(GroupingElement::rollup([region(), product()]))
        .order_by_expr(Func::grouping(region()).into(), Order::Asc)
        .order_by_expr(
            Func::sum(Expr::col(Name::runtime("amount"))).into(),
            Order::Asc,
        )
        .take();

    let rows = db.query_all(query.to_string().as_str(), &[]).await?;
    let answers: Vec<(i32, i64)> = rows.iter().map(|row| (row.get(0), row.get(1))).collect();
    assert_eq!(
        answers,
        [(0, 40), (0, 40), (1, 100)],
        "two NULLs from the data, one from the grand total"
    );

    Ok(())
}

/// A tuple inside `ROLLUP` is one unit of the hierarchy: `ROLLUP ((region,
/// product))` has just two prefixes, the pair and `()` — five rows, where
/// the two-level `ROLLUP (region, product)` gives eight.
// [spec:pgorm:def:sql.ast.select.grouping/test]    against a live server: a tuple is one unit of
// a ROLLUP
async fn a_tuple_is_one_unit_of_a_rollup(db: &DatabaseConnection) -> Result<(), Error> {
    let pair: SimpleExpr = Expr::tuple([region().into(), product().into()]).into();
    let query = grouped(|q| q.group_by_element(GroupingElement::rollup([pair])));

    assert!(
        query
            .to_string()
            .ends_with(r#"GROUP BY ROLLUP (("region", "product"))"#)
    );
    assert_eq!(
        masks(db, &query).await?,
        BTreeMap::from([(0, (4, 100)), (3, (1, 100))])
    );

    Ok(())
}

/// A `GROUP BY` list crosses its items' sets: a plain `region` beside
/// `ROLLUP (product)` groups by (region, product) and (region) — seven rows
/// and no grand total.
// [spec:pgorm:def:sql.ast.select.grouping/test]    against a live server: elements and plain
// expressions share one list
async fn element_beside_plain_expression_crosses_with_it(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let query = grouped(|q| {
        q.add_group_by([region().into()])
            .group_by_element(GroupingElement::rollup([product()]))
    });

    assert!(
        query
            .to_string()
            .ends_with(r#"GROUP BY "region", ROLLUP ("product")"#)
    );
    assert_eq!(
        masks(db, &query).await?,
        BTreeMap::from([(0, (4, 100)), (1, (3, 100))])
    );

    Ok(())
}

/// `ROLLUP` over no expressions has one prefix, the empty one, so it renders
/// `()`: one grand-total row, which `ROLLUP ()` — not in the grammar —
/// would have meant. It answers one row even over an empty input, where a
/// plain aggregate-free `GROUP BY` answers none. `GROUPING()` has nothing to
/// read here: under `GROUP BY ()` no column is a grouping expression, so the
/// server refuses it.
// [spec:pgorm:req:sql.render.grouping+1/test]    against a live server: an empty ROLLUP or CUBE is
// the grand total
async fn an_empty_rollup_is_the_grand_total(db: &DatabaseConnection) -> Result<(), Error> {
    let total = |element: GroupingElement, table: &'static str| {
        Query::select()
            .expr(Func::sum(Expr::col(Name::runtime("amount"))))
            .from(Name::runtime(table))
            .group_by_element(element)
            .to_string()
    };

    for element in [
        GroupingElement::rollup(Vec::<SimpleExpr>::new()),
        GroupingElement::cube(Vec::<SimpleExpr>::new()),
        GroupingElement::empty(),
    ] {
        let sql = total(element.clone(), "sales");
        assert!(sql.ends_with("GROUP BY ()"), "{sql}");
        let rows = db.query_all(sql.as_str(), &[]).await?;
        let sums: Vec<i64> = rows.iter().map(|row| row.get(0)).collect();
        assert_eq!(sums, [100], "one grand-total row");

        let rows = db
            .query_all(total(element, "nothing").as_str(), &[])
            .await?;
        let sums: Vec<Option<i64>> = rows.iter().map(|row| row.get(0)).collect();
        assert_eq!(sums, [None], "one row even over no input");
    }

    let refused = db
        .query_all(
            grouped(|q| q.group_by_element(GroupingElement::empty()))
                .to_string()
                .as_str(),
            &[],
        )
        .await
        .expect_err("GROUPING() over a column no set groups by");
    match &refused {
        Error::Postgres(e) => assert_eq!(e.code(), Some(&SqlState::GROUPING_ERROR)),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }

    Ok(())
}
