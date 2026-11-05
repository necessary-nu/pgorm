#![allow(unused_imports, dead_code)]

//! Array subscripts and slices, against a live PostgreSQL server.
//!
//! The render tests settle that `"a"[1]`, `"a"[2:]` and `(f(x))[1]` parse as
//! subscripts. What only the server settles is what they answer, and
//! PostgreSQL's arrays differ from every language a caller is likely to be
//! thinking in: they count from 1, an index out of range is NULL rather than
//! an error, a slice out of range is cut to the bounds rather than NULL, and a
//! chain that mixes an index into a slice reads the index as a slice too.
//! Each case below pins one of those against a value a zero-based or
//! error-raising reading would give instead.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Expr, Func, Name, Order, OrderedStatement, Query, SelectStatement, SimpleExpr, Subscript,
    TypeName,
};
use pgorm::{ConnectionTrait, SelectGetableTuple, SelectorRaw, entity::prelude::*};
use tokio_postgres::error::SqlState;

/// One row: a one-dimensional array, a two-dimensional one, and a text to
/// split into an array by a function.
const SEED: &str = "
    CREATE TABLE sample (id int primary key, a int[], m int[][], csv text);
    INSERT INTO sample (id, a, m, csv) VALUES
        (1, '{10,20,30}', '{{1,2,3},{4,5,6}}', 'x,y,z');
";

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("array_subscript_tests").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SEED).await?;

    indexes_count_from_one(&db).await?;
    an_index_out_of_range_is_null(&db).await?;
    slices_take_either_bound_or_neither(&db).await?;
    slice_out_of_range_is_cut_not_null(&db).await?;
    chained_subscripts_address_one_multidimensional_element(&db).await?;
    index_beside_a_slice_reads_as_a_slice(&db).await?;
    computed_arrays_are_parenthesised_before_subscripting(&db).await?;
    a_bound_index_is_an_int4_parameter(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn col(name: &'static str) -> Expr {
    Expr::col(Name::runtime(name))
}

fn from_sample(expr: impl Into<SimpleExpr>) -> String {
    Query::select()
        .expr(expr)
        .from(Name::runtime("sample"))
        .to_string()
}

async fn int(db: &DatabaseConnection, expr: impl Into<SimpleExpr>) -> Result<Option<i32>, Error> {
    Ok(db.query_one(from_sample(expr).as_str(), &[]).await?.get(0))
}

async fn ints(db: &DatabaseConnection, expr: impl Into<SimpleExpr>) -> Result<Vec<i32>, Error> {
    Ok(db.query_one(from_sample(expr).as_str(), &[]).await?.get(0))
}

/// The array literal as the server prints it, bounds included when they are
/// not the default `[1:n]`.
async fn text(db: &DatabaseConnection, expr: impl Into<SimpleExpr>) -> Result<String, Error> {
    let sql = from_sample(Expr::expr(expr).cast_as(Name::runtime("text")));
    Ok(db.query_one(sql.as_str(), &[]).await?.get(0))
}

/// `a[1]` is the first element, not the second: a zero-based reading would
/// answer 20 here, and `a[3]` would be out of range.
// [spec:pgorm:req:sql.ast.expr.subscript+2/test]    against a live server: indexes count from 1
// [spec:pgorm:req:sql.render.subscript/test]
// [spec:pgorm:req:sql.scope+23/test]
async fn indexes_count_from_one(db: &DatabaseConnection) -> Result<(), Error> {
    assert_eq!(int(db, col("a").index(1)).await?, Some(10));
    assert_eq!(int(db, col("a").index(2)).await?, Some(20));
    assert_eq!(int(db, col("a").index(3)).await?, Some(30));

    Ok(())
}

/// An index outside the bounds — zero, negative, or past the end — answers
/// NULL. The query succeeds; nothing raises.
// [spec:pgorm:req:sql.ast.expr.subscript+2/test]    against a live server: out of range is NULL
async fn an_index_out_of_range_is_null(db: &DatabaseConnection) -> Result<(), Error> {
    assert_eq!(int(db, col("a").index(0)).await?, None);
    assert_eq!(int(db, col("a").index(4)).await?, None);
    assert_eq!(int(db, col("a").index(-1)).await?, None);

    Ok(())
}

/// Both bounds inclusive; an omitted bound runs to that end of the array;
/// omitting both is the whole array.
// [spec:pgorm:req:sql.ast.expr.subscript+2/test]    against a live server: bounds are inclusive,
// and an omitted one runs to the array's end
async fn slices_take_either_bound_or_neither(db: &DatabaseConnection) -> Result<(), Error> {
    assert_eq!(ints(db, col("a").slice(2, 3)).await?, [20, 30]);
    assert_eq!(ints(db, col("a").slice(2, 2)).await?, [20]);
    assert_eq!(ints(db, col("a").slice_from(2)).await?, [20, 30]);
    assert_eq!(ints(db, col("a").slice_to(2)).await?, [10, 20]);
    assert_eq!(
        ints(db, col("a").subscript(Subscript::Slice(None, None))).await?,
        [10, 20, 30]
    );

    Ok(())
}

/// Where an index past the end is NULL, a slice past the end is cut to the
/// bounds, and one entirely outside them is the empty array — never NULL.
// [spec:pgorm:req:sql.ast.expr.subscript+2/test]    against a live server: a slice out of range is
// cut to the bounds, and empty rather than NULL
async fn slice_out_of_range_is_cut_not_null(db: &DatabaseConnection) -> Result<(), Error> {
    assert_eq!(ints(db, col("a").slice(2, 9)).await?, [20, 30]);
    assert_eq!(ints(db, col("a").slice(0, 1)).await?, [10]);
    assert_eq!(ints(db, col("a").slice(5, 9)).await?, Vec::<i32>::new());
    assert_eq!(text(db, col("a").slice(5, 9)).await?, "{}");

    Ok(())
}

/// `.index(2).index(3)` is `m[2][3]`, one access into a two-dimensional
/// array — row 2, column 3. The parenthesised `(m[2])[3]` would be something
/// else: PostgreSQL types `m[2]` as the element type, `int4`, so that
/// spelling is refused outright, and at runtime a single index on a
/// two-dimensional array is NULL besides.
// [spec:pgorm:req:sql.ast.expr.subscript+2/test]    against a live server: chained subscripts are
// one multi-dimensional access
// [spec:pgorm:req:sql.render.subscript/test]    a subscripted base is not parenthesised
async fn chained_subscripts_address_one_multidimensional_element(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let element = col("m").index(2).index(3);
    assert_eq!(
        from_sample(element.clone()),
        r#"SELECT "m"[2][3] FROM "sample""#
    );
    assert_eq!(int(db, element).await?, Some(6));

    assert_eq!(
        int(db, col("m").index(2)).await?,
        None,
        "one index, two dimensions"
    );
    let nested = db
        .query_one(r#"SELECT ("m"[2])[3] FROM "sample""#, &[])
        .await
        .expect_err("the spelling the renderer does not produce");
    match &nested {
        Error::Postgres(e) => assert_eq!(e.code(), Some(&SqlState::DATATYPE_MISMATCH)),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }

    assert_eq!(
        text(db, col("m").slice(1, 2).slice(2, 3)).await?,
        "{{2,3},{5,6}}",
        "two slices are one rectangular region"
    );

    Ok(())
}

/// Once any subscript in a chain is a slice, PostgreSQL reads every one as a
/// slice, and a plain index `i` as `1:i`. So `.slice(1, 2).index(2)` is the
/// region `[1:2][1:2]`, not the second element of each row.
// [spec:pgorm:req:sql.ast.expr.subscript+2/test]    against a live server: an index beside a slice
// is read as `1:i`
async fn index_beside_a_slice_reads_as_a_slice(db: &DatabaseConnection) -> Result<(), Error> {
    assert_eq!(
        text(db, col("m").slice(1, 2).index(2)).await?,
        "{{1,2},{4,5}}"
    );
    assert_eq!(
        text(db, col("m").slice(1, 2).slice(1, 2)).await?,
        "{{1,2},{4,5}}",
        "the same region spelled as two slices"
    );

    Ok(())
}

/// A function result or a cast takes a subscript only once parenthesised —
/// `string_to_array(..)[2]` and `CAST(.. AS int[])[2]` are syntax errors —
/// so these run at all only because the renderer wrapped them.
// [spec:pgorm:req:sql.render.subscript/test]    against a live server: a function result and a
// cast are parenthesised before the subscript
async fn computed_arrays_are_parenthesised_before_subscripting(
    db: &DatabaseConnection,
) -> Result<(), Error> {
    let split = Func::named(Name::runtime("string_to_array"))
        .arg(col("csv"))
        .arg(",");
    let second = Expr::expr(split).index(2);
    assert!(from_sample(second.clone()).contains(r#"(string_to_array("csv", ','))[2]"#));
    let answer: Option<String> = db
        .query_one(from_sample(second).as_str(), &[])
        .await?
        .get(0);
    assert_eq!(answer.as_deref(), Some("y"));

    let cast = Expr::val("{7,8,9}").cast_as_type(TypeName::new(Name::runtime("int4")).array());
    assert_eq!(int(db, Expr::expr(cast).index(2)).await?, Some(8));

    Ok(())
}

/// The `build()` path: the index is a `$N` placeholder the server types as
/// `int4`, the type array subscripts take, and an `i32` binds to it. A bound
/// array base, though, is a placeholder of no type until a cast supplies one.
// [spec:pgorm:req:sql.render.subscript/test]    against a live server: a bound index and bound
// slice bounds
async fn a_bound_index_is_an_int4_parameter(db: &DatabaseConnection) -> Result<(), Error> {
    let (sql, values) = Query::select()
        .expr(col("a").index(3))
        .expr(col("a").slice(1, 2))
        .from(Name::runtime("sample"))
        .build();
    assert_eq!(sql, r#"SELECT "a"[$1], "a"[$2:$3] FROM "sample""#);

    let rows: Vec<(Option<i32>, Vec<i32>)> = SelectorRaw::<
        SelectGetableTuple<(Option<i32>, Vec<i32>)>,
    >::into_tuple::<(Option<i32>, Vec<i32>)>(
        sql, values
    )
    .all(db)
    .await?;
    assert_eq!(rows, [(Some(30), vec![10, 20])]);

    // A bound *array* has no type until the caller gives it one: the server
    // will not subscript a placeholder of unknown type.
    let (sql, values) = Query::select()
        .expr(Expr::expr(Expr::value(vec![1, 2, 3])).index(1))
        .build();
    assert_eq!(sql, r#"SELECT ($1)[$2]"#);
    let untyped =
        SelectorRaw::<SelectGetableTuple<Option<i32>>>::into_tuple::<Option<i32>>(sql, values)
            .all(db)
            .await
            .expect_err("an untyped placeholder cannot be subscripted");
    match &untyped {
        Error::Postgres(e) => assert_eq!(e.code(), Some(&SqlState::DATATYPE_MISMATCH)),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }

    let typed = Expr::val(vec![1, 2, 3]).cast_as_type(TypeName::new(Name::runtime("int4")).array());
    let (sql, values) = Query::select().expr(Expr::expr(typed).index(3)).build();
    let answer: Vec<Option<i32>> =
        SelectorRaw::<SelectGetableTuple<Option<i32>>>::into_tuple::<Option<i32>>(sql, values)
            .all(db)
            .await?;
    assert_eq!(
        answer,
        [Some(3)],
        "cast_as_type gives the placeholder its type"
    );

    Ok(())
}
