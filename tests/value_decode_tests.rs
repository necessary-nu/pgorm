#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::{Schema, SelectModel, SelectorRaw, TryGetableMany, entity::prelude::*};
use pgorm_query::Values;
use pretty_assertions::assert_eq;

mod net_decode {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "net_decode")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub label: String,
        pub ip: IpNetwork,
        pub mac: MacAddress,
        pub gateway: Option<IpNetwork>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

fn net(s: &str) -> IpNetwork {
    s.parse().expect("network literal")
}

fn mac(s: &str) -> MacAddress {
    s.parse().expect("mac literal")
}

fn models() -> Vec<net_decode::Model> {
    vec![
        net_decode::Model {
            id: 1,
            label: "alpha".to_owned(),
            ip: net("10.0.0.1/32"),
            mac: mac("00:11:22:33:44:01"),
            gateway: Some(net("10.0.0.254/32")),
        },
        net_decode::Model {
            id: 2,
            label: "bravo".to_owned(),
            ip: net("2001:db8::5/128"),
            mac: mac("aa:bb:cc:dd:ee:ff"),
            gateway: None,
        },
        net_decode::Model {
            id: 3,
            label: "charlie".to_owned(),
            ip: net("192.168.0.0/24"),
            mac: mac("00:00:00:00:00:00"),
            gateway: Some(net("2001:db8::1/128")),
        },
    ]
}

// [spec:pgorm:def:exec.decode.types+3/test]
#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("value_decode_tests_valuedecode").await;
    let db = ctx.db.get().await?;

    let schema = Schema::new();
    create_table_without_asserts(&db, &schema.create_table_from_entity(net_decode::Entity)?)
        .await?;

    round_trip_inet_and_macaddr(&db).await?;
    decode_inet_and_macaddr_as_tuple(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:exec.decode.types+3/test]
// [spec:pgorm:def:exec.cursor.binding+6/test]    `IpNetwork` and `MacAddress`
// values, and a `None` payload emitted as SQL NULL, bound through `ValueHolder`
async fn round_trip_inet_and_macaddr(db: &DatabaseConnection) -> Result<(), Error> {
    for model in models() {
        let returned = model.clone().into_active_model().insert(db).await?;
        assert_eq!(returned, model);

        let fetched = net_decode::Entity::find_by_id(model.id).one(db).await?;
        assert_eq!(fetched, model);
    }

    assert_eq!(
        net_decode::Entity::find()
            .order_by_asc(net_decode::Column::Id)
            .all(db)
            .await?,
        models()
    );

    Ok(())
}

// [spec:pgorm:def:exec.decode.types+3/test]
async fn decode_inet_and_macaddr_as_tuple(db: &DatabaseConnection) -> Result<(), Error> {
    let decoded: Vec<(IpNetwork, MacAddress, Option<IpNetwork>)> = net_decode::Entity::find()
        .select([
            net_decode::Column::Ip,
            net_decode::Column::Mac,
            net_decode::Column::Gateway,
        ])
        .order_by_asc(net_decode::Column::Id)
        .into_tuple()
        .all(db)
        .await?;

    assert_eq!(
        decoded,
        models()
            .into_iter()
            .map(|m| (m.ip, m.mac, m.gateway))
            .collect::<Vec<_>>()
    );

    Ok(())
}

fn raw<M>(sql: &str) -> SelectorRaw<SelectModel<M>>
where
    M: FromQueryResult,
{
    (sql, Values(Vec::new())).into_model::<M>()
}

struct EntryPoints {
    by_name: i32,
    by_row_ordinal: i32,
    prefixed: String,
    bare: String,
    by_index: f64,
    many_named: (i32, String),
    many_indexed: (i32, String),
    columns: Vec<String>,
}

impl FromQueryResult for EntryPoints {
    fn from_query_result(res: &QueryResult, _pre: &str) -> Result<Self, Error> {
        Ok(Self {
            by_name: res.try_get_by("num")?,
            by_row_ordinal: res.try_get_by(0_usize)?,
            prefixed: res.try_get("p_", "word")?,
            bare: res.try_get("", "word")?,
            by_index: res.try_get_by_index(3)?,
            many_named: res.try_get_many("", &["num".to_owned(), "word".to_owned()])?,
            many_indexed: res.try_get_many_by_index()?,
            columns: res.column_names(),
        })
    }
}

// [spec:pgorm:def:exec.decode+3/test]    every `QueryResult` entry point, and the
// `{pre}{col}` concatenation `try_get` performs
#[pgorm_macros::test]
async fn decode_entry_points() -> Result<(), Error> {
    let ctx = TestContext::new("value_decode_tests_entry_points").await;
    let db = ctx.db.get().await?;

    let row = raw::<EntryPoints>(
        r#"SELECT 1 AS "num", 'prefixed' AS "p_word", 'bare' AS "word", 2.5::float8 AS "ratio""#,
    )
    .one(&db)
    .await?;

    // `try_get_by` takes either a column name or an ordinal.
    assert_eq!(row.by_name, 1);
    assert_eq!(row.by_row_ordinal, 1);

    // `try_get` concatenates prefix and column with no separator; an empty
    // prefix uses the bare column name.
    assert_eq!(row.prefixed, "prefixed");
    assert_eq!(row.bare, "bare");

    // `try_get_by_index` is positional in the select list.
    assert_eq!(row.by_index, 2.5);

    // Tuple extraction by name and by ordinal.
    assert_eq!(row.many_named, (1, "bare".to_owned()));
    assert_eq!(row.many_indexed, (1, "prefixed".to_owned()));

    // `column_names` reports the result set's columns in order.
    assert_eq!(row.columns, ["num", "p_word", "word", "ratio"]);

    drop(db);
    ctx.delete().await;

    Ok(())
}

struct NullProbe {
    optional: Option<i32>,
    strict_null_message: String,
    strict_null_is_type_err: bool,
    missing_is_postgres_err: bool,
    optional_missing_is_err: bool,
    wrong_type_is_postgres_err: bool,
}

impl FromQueryResult for NullProbe {
    fn from_query_result(res: &QueryResult, _pre: &str) -> Result<Self, Error> {
        let strict_null = res
            .try_get::<i32>("", "maybe")
            .expect_err("a NULL must not decode into a bare i32");
        let strict_null_is_type_err = matches!(strict_null, Error::Type(_));
        let strict_null_message = match strict_null {
            Error::Type(message) => message,
            other => format!("{other:?}"),
        };

        Ok(Self {
            optional: res.try_get("", "maybe")?,
            strict_null_message,
            strict_null_is_type_err,
            missing_is_postgres_err: matches!(
                res.try_get::<i32>("", "nope"),
                Err(Error::Postgres(_))
            ),
            optional_missing_is_err: res.try_get::<Option<i32>>("", "nope").is_err(),
            wrong_type_is_postgres_err: matches!(
                res.try_get::<String>("", "num"),
                Err(Error::Postgres(_))
            ),
        })
    }
}

// [spec:pgorm:sem:exec.decode.null+1/test]    `Option<T>` swallows only the null
// case; every other decode error propagates
// [spec:pgorm:sem:exec.decode.null-context+1/test]    the null payload is the
// driver's ordinal-based message, not the requested column name
#[pgorm_macros::test]
async fn decode_null_handling() -> Result<(), Error> {
    let ctx = TestContext::new("value_decode_tests_null_handling").await;
    let db = ctx.db.get().await?;

    let row = raw::<NullProbe>(r#"SELECT 7 AS "num", NULL::int4 AS "maybe""#)
        .one(&db)
        .await?;

    // SQL NULL decodes to `None` through the blanket `Option<T>` impl...
    assert_eq!(row.optional, None);

    // ... and is an error for a non-`Option` target, rendered by
    // `From<TryGetError> for Error`.
    assert!(row.strict_null_is_type_err);
    assert!(
        row.strict_null_message.starts_with(
            "A null value was encountered while decoding error deserializing column 1"
        ),
        "{}",
        row.strict_null_message
    );

    // The payload carries no structured column context: it is the `Display` of
    // the underlying `tokio_postgres::Error`, which names the ordinal, never
    // the requested column name.
    assert!(!row.strict_null_message.contains("maybe"));

    // An error with no `WasNull` source stays an `Error::Postgres`, and
    // `Option<T>` propagates it rather than reporting `None`.
    assert!(row.missing_is_postgres_err);
    assert!(row.optional_missing_is_err);
    assert!(row.wrong_type_is_postgres_err);

    drop(db);
    ctx.delete().await;

    Ok(())
}

struct OidProbe {
    oid: u32,
    oids: Vec<u32>,
    int4_is_err: bool,
}

impl FromQueryResult for OidProbe {
    fn from_query_result(res: &QueryResult, _pre: &str) -> Result<Self, Error> {
        Ok(Self {
            oid: res.try_get("", "an_oid")?,
            oids: res.try_get("", "some_oids")?,
            int4_is_err: res.try_get::<u32>("", "an_int").is_err(),
        })
    }
}

// [spec:pgorm:sem:exec.decode.u32-oid/test]    `u32` and `Vec<u32>` read `OID`
// columns, and reject `INT4`
#[pgorm_macros::test]
async fn decode_u32_is_oid() -> Result<(), Error> {
    let ctx = TestContext::new("value_decode_tests_u32_oid").await;
    let db = ctx.db.get().await?;

    let row = raw::<OidProbe>(
        r#"SELECT 1247::oid AS "an_oid", ARRAY[16::oid, 17::oid] AS "some_oids", 1247::int4 AS "an_int""#,
    )
    .one(&db)
    .await?;

    assert_eq!(row.oid, 1247);
    assert_eq!(row.oids, [16, 17]);
    assert!(row.int4_is_err, "an INT4 column must not decode as u32");

    drop(db);
    ctx.delete().await;

    Ok(())
}

#[derive(Debug, FromQueryResult)]
struct Arrays {
    bools: Vec<bool>,
    chars: Vec<i8>,
    shorts: Vec<i16>,
    ints: Vec<i32>,
    longs: Vec<i64>,
    floats: Vec<f32>,
    doubles: Vec<f64>,
    words: Vec<String>,
    decimals: Vec<Decimal>,
    dates: Vec<Date>,
    stamps: Vec<DateTime>,
    docs: Vec<Json>,
    uuids: Vec<Uuid>,
    bytes: Vec<u8>,
}

// [spec:pgorm:def:exec.decode.array+2/test]    the array-decoding subset, and
// `Vec<u8>` staying bytes rather than an array
#[pgorm_macros::test]
async fn decode_arrays() -> Result<(), Error> {
    let ctx = TestContext::new("value_decode_tests_arrays").await;
    let db = ctx.db.get().await?;

    let row = raw::<Arrays>(concat!(
        r#"SELECT ARRAY[true, false] AS "bools", "#,
        r#"ARRAY['A'::"char", 'B'::"char"] AS "chars", "#,
        r#"ARRAY[1, 2]::int2[] AS "shorts", "#,
        r#"ARRAY[3, 4]::int4[] AS "ints", "#,
        r#"ARRAY[5, 6]::int8[] AS "longs", "#,
        r#"ARRAY[1.5, 2.5]::float4[] AS "floats", "#,
        r#"ARRAY[3.5, 4.5]::float8[] AS "doubles", "#,
        r#"ARRAY['one', 'two'] AS "words", "#,
        r#"ARRAY[1.25, 2.50]::numeric[] AS "decimals", "#,
        r#"ARRAY['2024-01-02'::date] AS "dates", "#,
        r#"ARRAY['2024-01-02 03:04:05'::timestamp] AS "stamps", "#,
        r#"ARRAY['{"a": 1}'::json] AS "docs", "#,
        r#"ARRAY['936da01f-9abd-4d9d-80c7-02af85c822a8'::uuid] AS "uuids", "#,
        r#"'\x0102'::bytea AS "bytes""#,
    ))
    .one(&db)
    .await?;

    assert_eq!(row.bools, [true, false]);
    assert_eq!(row.chars, [b'A' as i8, b'B' as i8]);
    assert_eq!(row.shorts, [1, 2]);
    assert_eq!(row.ints, [3, 4]);
    assert_eq!(row.longs, [5, 6]);
    assert_eq!(row.floats, [1.5, 2.5]);
    assert_eq!(row.doubles, [3.5, 4.5]);
    assert_eq!(row.words, ["one", "two"]);
    assert_eq!(row.decimals, [rust_dec("1.25"), rust_dec("2.50")]);
    assert_eq!(row.dates, ["2024-01-02".parse::<Date>().unwrap()]);
    assert_eq!(
        row.stamps,
        ["2024-01-02T03:04:05".parse::<DateTime>().unwrap()]
    );
    assert_eq!(row.docs, [serde_json::json!({ "a": 1 })]);
    assert_eq!(
        row.uuids,
        ["936da01f-9abd-4d9d-80c7-02af85c822a8"
            .parse::<Uuid>()
            .unwrap()]
    );

    // The subset is proper: `Vec<u8>` is `bytea`, not an array of `i8`.
    assert_eq!(row.bytes, [1_u8, 2_u8]);

    drop(db);
    ctx.delete().await;

    Ok(())
}

struct ManyProbe {
    single: i32,
    one_tuple: (i32,),
    pair: (i32, String),
    surplus_names_ignored: (i32, String),
    too_few_names_message: String,
    too_few_names_is_type_err: bool,
    by_index_needs_no_names: (i32, String, f64),
}

impl FromQueryResult for ManyProbe {
    fn from_query_result(res: &QueryResult, _pre: &str) -> Result<Self, Error> {
        let num = || "num".to_owned();
        let word = || "word".to_owned();
        let ratio = || "ratio".to_owned();

        let too_few = res
            .try_get_many::<(i32, String)>("", &[num()])
            .expect_err("a short column slice must be rejected");
        let too_few_names_is_type_err = matches!(too_few, Error::Type(_));
        let too_few_names_message = match too_few {
            Error::Type(message) => message,
            other => format!("{other:?}"),
        };

        Ok(Self {
            single: res.try_get_many("", &[num()])?,
            one_tuple: res.try_get_many("", &[num()])?,
            pair: res.try_get_many("", &[num(), word()])?,
            surplus_names_ignored: res.try_get_many("", &[num(), word(), ratio()])?,
            too_few_names_message,
            too_few_names_is_type_err,
            by_index_needs_no_names: res.try_get_many_by_index()?,
        })
    }
}

type Dozen = (i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32);

struct WideProbe {
    dozen: Dozen,
}

impl FromQueryResult for WideProbe {
    fn from_query_result(res: &QueryResult, _pre: &str) -> Result<Self, Error> {
        Ok(Self {
            dozen: res.try_get_many_by_index()?,
        })
    }
}

#[derive(Clone, Copy, Debug, Column)]
enum ResultCol {
    Num,
    Word,
}

// [spec:pgorm:def:exec.decode.many/test]    tuple extraction by name and by
// ordinal, plus `find_by_statement`
// [spec:pgorm:req:exec.decode.many-arity/test]    a short column slice is a
// type error, surplus names are ignored, and the index path is unchecked
// [spec:pgorm:def:exec.crud+2/test]    `find_by_statement` builds a `SelectorRaw`
// over `SelectGetableValue` through `with_columns`
#[pgorm_macros::test]
async fn decode_many() -> Result<(), Error> {
    let ctx = TestContext::new("value_decode_tests_many").await;
    let db = ctx.db.get().await?;

    const SQL: &str = r#"SELECT 1 AS "num", 'word' AS "word", 2.5::float8 AS "ratio""#;

    let row = raw::<ManyProbe>(SQL).one(&db).await?;

    // The blanket impl for a single `TryGetable` reads the first column; `(T,)`
    // delegates to it.
    assert_eq!(row.single, 1);
    assert_eq!(row.one_tuple, (1,));
    assert_eq!(row.pair, (1, "word".to_owned()));

    // Surplus column names are ignored; a short slice is a type error naming
    // both the expected arity and the slice length.
    assert_eq!(row.surplus_names_ignored, (1, "word".to_owned()));
    assert!(row.too_few_names_is_type_err);
    assert_eq!(
        row.too_few_names_message,
        "Expect 2 column names supplied but got slice of length 1"
    );

    // The index-based path performs no such check; it reads ordinals `0..N`.
    assert_eq!(row.by_index_needs_no_names, (1, "word".to_owned(), 2.5));

    // Tuples are implemented up to arity 12.
    let wide = raw::<WideProbe>("SELECT 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12")
        .one(&db)
        .await?;
    assert_eq!(wide.dozen, (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12));

    // `find_by_statement` names the columns by iterating the `C` identifier
    // enum, so the decoded tuple follows the enum's declaration order.
    let named: (i32, String) =
        <(i32, String)>::find_by_statement::<ResultCol>(SQL.to_owned(), Values(Vec::new()))
            .one(&db)
            .await?;
    assert_eq!(named, (1, "word".to_owned()));

    drop(db);
    ctx.delete().await;

    Ok(())
}

struct JsonProbe {
    single: json_vec_derive::json_struct_vec::JsonColumn,
    array: Vec<json_vec_derive::json_struct_vec::JsonColumn>,
    non_array_message: String,
    non_array_is_json_err: bool,
}

impl FromQueryResult for JsonProbe {
    fn from_query_result(res: &QueryResult, _pre: &str) -> Result<Self, Error> {
        use json_vec_derive::json_struct_vec::JsonColumn;

        let non_array = res
            .try_get::<Vec<JsonColumn>>("", "object")
            .expect_err("a JSON object must not decode as a Vec");
        let non_array_is_json_err = matches!(non_array, Error::Json(_));
        let non_array_message = match non_array {
            Error::Json(message) => message,
            other => format!("{other:?}"),
        };

        Ok(Self {
            single: res.try_get("", "object")?,
            array: res.try_get("", "array")?,
            non_array_message,
            non_array_is_json_err,
        })
    }
}

// [spec:pgorm:def:exec.decode.json+1/test]    the `TryGetableFromJson` blanket
// impl and the `TryGetableArray` `Vec<T>` path, including its non-array error
#[pgorm_macros::test]
async fn decode_json() -> Result<(), Error> {
    use json_vec_derive::json_struct_vec::JsonColumn;

    let ctx = TestContext::new("value_decode_tests_json").await;
    let db = ctx.db.get().await?;

    let row = raw::<JsonProbe>(concat!(
        r#"SELECT '{"value": "solo"}'::jsonb AS "object", "#,
        r#"'[{"value": "first"}, {"value": "second"}]'::jsonb AS "array""#,
    ))
    .one(&db)
    .await?;

    assert_eq!(
        row.single,
        JsonColumn {
            value: "solo".to_owned()
        }
    );
    assert_eq!(
        row.array,
        [
            JsonColumn {
                value: "first".to_owned()
            },
            JsonColumn {
                value: "second".to_owned()
            },
        ]
    );

    // `from_json_vec` rejects any non-array JSON value.
    assert!(row.non_array_is_json_err);
    assert_eq!(row.non_array_message, "Value is not an Array");

    drop(db);
    ctx.delete().await;

    Ok(())
}

mod big_counter {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "big_counter")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub counter: i64,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

// [spec:pgorm:req:exec.cursor.binding-coerce+2/test]    a `u64` reaches the
// server through a checked `i64::try_from`: one above `i64::MAX` is refused by
// the client rather than wrapping to a negative `int8` that would match the
// wrong rows, and one below it binds exactly
#[pgorm_macros::test]
async fn big_unsigned_binds_checked() -> Result<(), Error> {
    let ctx = TestContext::new("value_decode_tests_big_unsigned").await;
    let db = ctx.db.get().await?;

    let schema = Schema::new();
    create_table_without_asserts(&db, &schema.create_table_from_entity(big_counter::Entity)?)
        .await?;

    for (id, counter) in [(1, -1_i64), (2, 7), (3, i64::MAX)] {
        big_counter::ActiveModel {
            id: set(id),
            counter: set(counter),
        }
        .insert(&db)
        .await?;
    }

    let too_big = i64::MAX as u64 + 1;

    // Wrapped, `u64::MAX` would bind as -1 and quietly return the id-1 row.
    let wrapped = big_counter::Entity::find()
        .filter(big_counter::Column::Counter.eq(u64::MAX))
        .all(&db)
        .await;
    assert!(
        format!("{:?}", wrapped.unwrap_err())
            .contains("value `18446744073709551615` is out of range"),
        "an out-of-range u64 filter operand must be refused"
    );

    // LIMIT is a bound parameter too, and its counts are `BigUnsigned`.
    let limited = big_counter::Entity::find().limit(too_big).all(&db).await;
    assert!(
        format!("{:?}", limited.unwrap_err())
            .contains("value `9223372036854775808` is out of range"),
        "an out-of-range u64 LIMIT must be refused"
    );

    // Everything at or below `i64::MAX` binds as the number it is.
    assert_eq!(
        big_counter::Entity::find()
            .filter(big_counter::Column::Counter.eq(i64::MAX as u64))
            .all(&db)
            .await?
            .len(),
        1
    );
    assert_eq!(
        big_counter::Entity::find()
            .filter(big_counter::Column::Counter.eq(7_u64))
            .one(&db)
            .await?,
        big_counter::Model { id: 2, counter: 7 }
    );
    assert_eq!(
        big_counter::Entity::find()
            .order_by_asc(big_counter::Column::Id)
            .limit(2_u64)
            .all(&db)
            .await?
            .len(),
        2
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}
