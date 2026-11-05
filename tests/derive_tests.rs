#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::TestContext;
use pgorm::{
    DecodeRaw, Error, FromQueryResult, PartialModel, SelectModel, SelectorRaw, TryGetable,
};
use pgorm_query::{Expr, Values};

#[derive(FromQueryResult)]
struct SimpleTest {
    _foo: i32,
    _bar: String,
}

#[derive(FromQueryResult)]
struct GenericTest<T: TryGetable> {
    _foo: i32,
    _bar: T,
}

#[derive(FromQueryResult)]
struct DoubleGenericTest<T: TryGetable, F: TryGetable> {
    _foo: T,
    _bar: F,
}

#[derive(FromQueryResult)]
struct BoundsGenericTest<T: TryGetable + Copy + Clone + 'static> {
    _foo: T,
}

#[derive(FromQueryResult)]
struct WhereGenericTest<T>
where
    T: TryGetable + Copy + Clone + 'static,
{
    _foo: T,
}

#[derive(FromQueryResult)]
struct AlreadySpecifiedBoundsGenericTest<T: TryGetable> {
    _foo: T,
}

#[derive(FromQueryResult)]
struct MixedGenericTest<T: TryGetable + Clone, F>
where
    F: TryGetable + Copy + Clone + 'static,
{
    _foo: T,
    _bar: F,
}

trait MyTrait {
    type Item: TryGetable;
}

#[derive(FromQueryResult)]
struct TraitAssociateTypeTest<T>
where
    T: MyTrait,
{
    _foo: T::Item,
}

#[derive(FromQueryResult)]
struct FromQueryAttributeTests {
    #[pgorm(skip)]
    _foo: i32,
    _bar: String,
}

// The column name is the *un-rawed* field identifier.
#[derive(FromQueryResult)]
struct RawIdentifierTest {
    r#type: String,
}

// `skip` accumulates rather than being recomputed, so it holds wherever it is
// written: ahead of another key in the same list, and split across two
// `#[pgorm(...)]` attributes. Both fields here are skipped, by the projection
// and by the `FromQueryResult` `PartialModel` writes, so only `_baz` is read.
#[derive(PartialModel)]
struct SkipAnywhereTest {
    #[pgorm(skip, from_expr = Expr::val(1))]
    _foo: i32,
    #[pgorm(from_expr = Expr::val(2))]
    #[pgorm(skip)]
    _bar: i32,
    #[pgorm(from_expr = Expr::val(3))]
    _baz: i32,
}

fn raw<M>(sql: &str) -> SelectorRaw<SelectModel<M>>
where
    M: FromQueryResult,
{
    (sql, Values(Vec::new())).into_model::<M>()
}

// [spec:pgorm:def:exec.crud+2/test]    `SelectorRaw::from_statement` decoding
// through `SelectModel`, i.e. `SelectorTrait::from_raw_query_result` with an
// empty prefix
// [spec:pgorm:sem:macros.derive.from-query-result+4/test]    field-named reads, generics, and `skip`
#[pgorm_macros::test]
async fn from_query_result_derive() -> Result<(), Error> {
    let ctx = TestContext::new("derive_tests_from_query_result").await;
    let db = ctx.db.get().await?;

    let row = raw::<SimpleTest>(r#"SELECT 1 AS "_foo", 'one' AS "_bar""#)
        .one(&db)
        .await?;
    assert_eq!(row._foo, 1);
    assert_eq!(row._bar, "one");

    let row = raw::<GenericTest<String>>(r#"SELECT 2 AS "_foo", 'two' AS "_bar""#)
        .one(&db)
        .await?;
    assert_eq!(row._foo, 2);
    assert_eq!(row._bar, "two");

    let row =
        raw::<DoubleGenericTest<bool, f64>>(r#"SELECT true AS "_foo", 1.5::float8 AS "_bar""#)
            .one(&db)
            .await?;
    assert!(row._foo);
    assert_eq!(row._bar, 1.5);

    // A skipped field is never read from the row; it takes `Default::default()`.
    let row = raw::<FromQueryAttributeTests>(r#"SELECT 'three' AS "_bar""#)
        .one(&db)
        .await?;
    assert_eq!(row._foo, 0);
    assert_eq!(row._bar, "three");

    // The column is named after the un-rawed identifier: `type`, not `r#type`.
    let row = raw::<RawIdentifierTest>(r#"SELECT 'four' AS "type""#)
        .one(&db)
        .await?;
    assert_eq!(row.r#type, "four");

    // `skip` holds from any position, so neither skipped field is read and the
    // statement needs only the third's column.
    let row = raw::<SkipAnywhereTest>(r#"SELECT 5 AS "unrelated", 3 AS "_baz""#)
        .one(&db)
        .await?;
    assert_eq!(row._foo, 0);
    assert_eq!(row._bar, 0);
    assert_eq!(row._baz, 3);

    drop(db);
    ctx.delete().await;

    Ok(())
}
