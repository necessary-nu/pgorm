//! Native values use the driver's existing metadata and binary codec contract.
#![cfg(feature = "turso")]

use bytes::BytesMut;
use pgorm_driver::{
    Client, Error, Statement, TursoDatabase,
    types::{Format, FromSql, IsNull, Kind, ToSql, Type},
};
use std::{
    error::Error as StdError,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::task::JoinHandle;

type CodecError = Box<dyn StdError + Send + Sync>;
type ConnectionTask = JoinHandle<Result<(), Error>>;

async fn connect() -> (Client, ConnectionTask) {
    let database = TursoDatabase::open(":memory:").await.unwrap();
    let (client, connection) = database.connect().await.unwrap();
    (client, tokio::spawn(connection))
}

async fn finish(client: Client, task: ConnectionTask) {
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

fn columns(statement: &Statement) -> Vec<(&str, Type)> {
    statement
        .columns()
        .iter()
        .map(|column| (column.name(), column.type_().clone()))
        .collect()
}

fn driver_refusal<T>(result: Result<T, Error>) -> Error {
    let error = result
        .err()
        .expect("the driver must refuse this representation");
    assert!(!error.is_closed(), "{error}");
    assert!(
        error.code().is_none(),
        "driver refusals do not invent SQLSTATEs: {error}"
    );
    error
}

#[derive(Debug)]
struct Encoded {
    bytes: Option<Vec<u8>>,
    format: Format,
    calls: Arc<AtomicUsize>,
}

impl ToSql for Encoded {
    fn to_sql(&self, _: &Type, output: &mut BytesMut) -> Result<IsNull, CodecError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.bytes {
            Some(bytes) => {
                output.extend_from_slice(bytes);
                Ok(IsNull::No)
            }
            None => Ok(IsNull::Yes),
        }
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    fn encode_format(&self, _: &Type) -> Format {
        self.format
    }

    pgorm_driver::types::to_sql_checked!();
}

#[derive(Debug)]
struct Binary<'a>(&'a [u8]);

impl<'a> FromSql<'a> for Binary<'a> {
    fn from_sql(_: &Type, bytes: &'a [u8]) -> Result<Self, CodecError> {
        Ok(Self(bytes))
    }

    fn accepts(ty: &Type) -> bool {
        matches!(*ty, Type::INT4 | Type::TEXT | Type::BYTEA)
    }
}

// [spec:pgorm:req:turso.values/test]
// [spec:pgorm:req:turso.rows/test]
#[tokio::test]
async fn scalar_columns_preserve_values_and_integer_widths() {
    let (client, task) = connect().await;
    client.batch_execute("CREATE TABLE scalar_values (b boolean, i2 smallint, i4 integer, i8 bigint, txt text, bytes bytea, real_value double precision)").await.unwrap();
    let types = [
        Type::BOOL,
        Type::INT2,
        Type::INT4,
        Type::INT8,
        Type::TEXT,
        Type::BYTEA,
        Type::FLOAT8,
    ];
    let statement = client.prepare_typed(
        "INSERT INTO scalar_values VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING b,i2,i4,i8,txt,bytes,real_value", &types,
    ).await.unwrap();
    assert_eq!(statement.params(), types);
    assert_eq!(
        columns(&statement),
        vec![
            ("b", Type::BOOL),
            ("i2", Type::INT2),
            ("i4", Type::INT4),
            ("i8", Type::INT8),
            ("txt", Type::TEXT),
            ("bytes", Type::BYTEA),
            ("real_value", Type::FLOAT8),
        ]
    );
    let cases = [
        (
            false,
            i16::MIN,
            i32::MIN,
            i64::MIN,
            "",
            vec![],
            f64::MIN_POSITIVE,
        ),
        (
            true,
            i16::MAX,
            i32::MAX,
            i64::MAX,
            "räksmörgås 🦀",
            vec![0, 255, 128, 1],
            f64::MAX,
        ),
        (
            true,
            -7,
            17,
            9_007_199_254_740_993,
            "precise",
            vec![42],
            f64::from_bits(0x3ff0000000000001),
        ),
    ];
    for (boolean, small, integer, big, text, bytes, float) in cases {
        let row = client
            .query_one(
                &statement,
                &[&boolean, &small, &integer, &big, &text, &bytes, &float],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, bool>("b"), boolean);
        assert_eq!(row.get::<_, i16>("i2"), small);
        assert_eq!(row.get::<_, i32>("i4"), integer);
        assert_eq!(row.get::<_, i64>("i8"), big);
        assert_eq!(row.get::<_, &str>("txt"), text);
        assert_eq!(row.get::<_, &[u8]>("bytes"), bytes);
        assert_eq!(row.get::<_, f64>("real_value").to_bits(), float.to_bits());
        assert_eq!(row.get::<_, i32>(2), integer);
        assert!(row.try_get::<_, i16>("i4").is_err());
        assert!(row.try_get::<_, i32>("absent").is_err());
        assert!(row.try_get::<_, i32>(row.len()).is_err());
    }
    finish(client, task).await;
}

// [spec:pgorm:req:turso.values/test]
// [spec:pgorm:req:turso.rows/test]
#[tokio::test]
async fn typed_nulls_differ_from_empty_text_and_bytes() {
    let (client, task) = connect().await;
    client.batch_execute("CREATE TABLE nullable_values (b boolean, i2 smallint, i4 integer, i8 bigint, txt text, bytes bytea, real_value double precision)").await.unwrap();
    let statement = client.prepare_typed(
        "INSERT INTO nullable_values VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING b,i2,i4,i8,txt,bytes,real_value",
        &[Type::BOOL, Type::INT2, Type::INT4, Type::INT8, Type::TEXT, Type::BYTEA, Type::FLOAT8],
    ).await.unwrap();
    let null = client
        .query_one(
            &statement,
            &[
                &None::<bool>,
                &None::<i16>,
                &None::<i32>,
                &None::<i64>,
                &None::<String>,
                &None::<Vec<u8>>,
                &None::<f64>,
            ],
        )
        .await
        .unwrap();
    assert_eq!(null.get::<_, Option<bool>>(0), None);
    assert_eq!(null.get::<_, Option<i16>>(1), None);
    assert_eq!(null.get::<_, Option<i32>>(2), None);
    assert_eq!(null.get::<_, Option<i64>>(3), None);
    assert_eq!(null.get::<_, Option<&str>>(4), None);
    assert_eq!(null.get::<_, Option<&[u8]>>(5), None);
    assert_eq!(null.get::<_, Option<f64>>(6), None);
    assert!(null.try_get::<_, String>(4).is_err());
    let empty = client
        .query_one(
            &statement,
            &[&false, &0i16, &0i32, &0i64, &"", &Vec::<u8>::new(), &0.0f64],
        )
        .await
        .unwrap();
    assert_eq!(empty.get::<_, Option<&str>>(4), Some(""));
    assert_eq!(empty.get::<_, Option<&[u8]>>(5), Some(&[][..]));
    assert_eq!(empty.get::<_, Option<bool>>(0), Some(false));
    assert_eq!(empty.get::<_, Option<i64>>(3), Some(0));
    assert_eq!(
        null.columns().iter().map(|c| c.type_()).collect::<Vec<_>>(),
        empty
            .columns()
            .iter()
            .map(|c| c.type_())
            .collect::<Vec<_>>()
    );
    finish(client, task).await;
}

// [spec:pgorm:req:turso.values/test]
#[tokio::test]
async fn float_and_text_parameters_use_native_result_types() {
    let (client, task) = connect().await;
    let statement = client.prepare_typed(
        "SELECT $1::double precision AS first, $2::double precision AS second, $3::text AS varying, $4::text AS fixed",
        &[Type::FLOAT4, Type::FLOAT8, Type::VARCHAR, Type::BPCHAR],
    ).await.unwrap();
    assert_eq!(
        columns(&statement),
        vec![
            ("first", Type::FLOAT8),
            ("second", Type::FLOAT8),
            ("varying", Type::TEXT),
            ("fixed", Type::TEXT)
        ]
    );
    let small = f32::from_bits(0x3eaaaaab);
    let large = f64::from_bits(0x3ff0000000000001);
    let row = client
        .query_one(&statement, &[&small, &large, &"λ", &"pad"])
        .await
        .unwrap();
    assert_eq!(row.get::<_, f64>(0).to_bits(), f64::from(small).to_bits());
    assert_eq!(row.get::<_, f64>(1).to_bits(), large.to_bits());
    assert_eq!(row.get::<_, &str>(2), "λ");
    assert_eq!(row.get::<_, &str>(3), "pad");
    assert!(row.try_get::<_, f32>(0).is_err());
    finish(client, task).await;
}

// [spec:pgorm:req:turso.rows/test]
#[tokio::test]
async fn empty_and_null_results_have_prepared_metadata() {
    let (client, task) = connect().await;
    client.batch_execute("CREATE TABLE metadata_values (small smallint, big bigint, flag boolean, txt text, bytes bytea)").await.unwrap();
    let expected = vec![
        ("small", Type::INT2),
        ("big", Type::INT8),
        ("flag", Type::BOOL),
        ("txt", Type::TEXT),
        ("bytes", Type::BYTEA),
    ];
    let empty = client
        .prepare("SELECT small,big,flag,txt,bytes FROM metadata_values")
        .await
        .unwrap();
    assert_eq!(columns(&empty), expected);
    assert!(client.query(&empty, &[]).await.unwrap().is_empty());
    client
        .batch_execute("INSERT INTO metadata_values VALUES (NULL,NULL,NULL,NULL,NULL)")
        .await
        .unwrap();
    let all_null = client
        .prepare("SELECT small,big,flag,txt,bytes FROM metadata_values")
        .await
        .unwrap();
    assert_eq!(columns(&all_null), expected);
    let row = client.query_one(&all_null, &[]).await.unwrap();
    assert_eq!(row.get::<_, Option<i16>>(0), None);
    let filtered = client
        .prepare("SELECT small,big,flag,txt,bytes FROM metadata_values WHERE false")
        .await
        .unwrap();
    assert_eq!(columns(&filtered), expected);
    assert!(client.query(&filtered, &[]).await.unwrap().is_empty());
    let casts = client
        .prepare("SELECT NULL::integer AS i, NULL::boolean AS b, NULL::text AS t")
        .await
        .unwrap();
    assert_eq!(
        columns(&casts),
        vec![("i", Type::INT4), ("b", Type::BOOL), ("t", Type::TEXT)]
    );
    driver_refusal(client.prepare("SELECT NULL AS unresolved").await);
    finish(client, task).await;
}

// [spec:pgorm:req:turso.rows/test]
#[tokio::test]
async fn compiled_metadata_distinguishes_counts_booleans_and_aliases() {
    let (client, task) = connect().await;
    client.batch_execute("CREATE TABLE predicate_values (id integer); INSERT INTO predicate_values VALUES (NULL),(1)").await.unwrap();
    let counts = client
        .prepare("SELECT count(*) AS total, count(id) AS populated FROM predicate_values")
        .await
        .unwrap();
    assert_eq!(
        columns(&counts),
        vec![("total", Type::INT8), ("populated", Type::INT8)]
    );
    let row = client.query_one(&counts, &[]).await.unwrap();
    assert_eq!((row.get::<_, i64>(0), row.get::<_, i64>(1)), (2, 1));
    let predicates = client.prepare("SELECT id IS NULL AS missing, id = 1 AS eq, NOT (id = 1) AS neg, (id = 1) AND true AS both, EXISTS (SELECT 1 FROM predicate_values WHERE id = 1) AS found, id AS count FROM predicate_values ORDER BY id NULLS FIRST").await.unwrap();
    assert_eq!(
        columns(&predicates),
        vec![
            ("missing", Type::BOOL),
            ("eq", Type::BOOL),
            ("neg", Type::BOOL),
            ("both", Type::BOOL),
            ("found", Type::BOOL),
            ("count", Type::INT4)
        ]
    );
    let rows = client.query(&predicates, &[]).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].get::<_, bool>(0));
    for index in 1..=3 {
        assert_eq!(rows[0].get::<_, Option<bool>>(index), None);
    }
    assert!(rows[0].get::<_, bool>(4));
    assert_eq!(rows[0].get::<_, Option<i32>>(5), None);
    assert_eq!(
        (
            rows[1].get::<_, bool>(0),
            rows[1].get::<_, bool>(1),
            rows[1].get::<_, bool>(2),
            rows[1].get::<_, bool>(3)
        ),
        (false, true, false, true)
    );
    assert!(rows[1].get::<_, bool>(4));
    assert_eq!(rows[1].get::<_, i32>(5), 1);
    driver_refusal(client.prepare("SELECT sum(id) FROM predicate_values").await);
    finish(client, task).await;
}

// [spec:pgorm:req:turso.values/test]
// [spec:pgorm:req:turso.errors/test]
#[tokio::test]
async fn malformed_and_text_codecs_fail_before_execution() {
    let (client, task) = connect().await;
    client
        .batch_execute("CREATE TABLE codec_values (id integer PRIMARY KEY); BEGIN")
        .await
        .unwrap();
    let statement = client
        .prepare_typed("INSERT INTO codec_values VALUES ($1)", &[Type::INT4])
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    for (bytes, format) in [
        (vec![0, 1, 2], Format::Binary),
        (b"1".to_vec(), Format::Text),
    ] {
        let value = Encoded {
            bytes: Some(bytes),
            format,
            calls: Arc::clone(&calls),
        };
        let error = driver_refusal(client.execute(&statement, &[&value]).await);
        assert!(error.to_string().contains("parameter"), "{error}");
    }
    assert_eq!(client.execute(&statement, &[&1i32]).await.unwrap(), 1);
    client.batch_execute("ROLLBACK").await.unwrap();
    let count = client
        .query_one("SELECT count(*) FROM codec_values", &[])
        .await
        .unwrap();
    assert_eq!(count.get::<_, i64>(0), 0);
    finish(client, task).await;
}

// [spec:pgorm:req:turso.values/test]
// [spec:pgorm:req:turso.errors/test]
// [spec:pgorm:req:turso.parameters/test]
#[tokio::test]
async fn type_and_arity_refusals_preserve_the_transaction() {
    let (client, task) = connect().await;
    client
        .batch_execute("CREATE TABLE checked_values (id integer PRIMARY KEY); BEGIN")
        .await
        .unwrap();
    let statement = client
        .prepare_typed("INSERT INTO checked_values VALUES ($1)", &[Type::INT4])
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let value = Encoded {
        bytes: Some(1i32.to_be_bytes().to_vec()),
        format: Format::Binary,
        calls: Arc::clone(&calls),
    };
    driver_refusal(client.execute(&statement, &[]).await);
    driver_refusal(client.execute(&statement, &[&value, &value]).await);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let wrong_type = driver_refusal(client.execute(&statement, &[&"wrong type"]).await);
    assert!(
        wrong_type
            .source()
            .unwrap()
            .is::<pgorm_driver::types::WrongType>()
    );
    assert_eq!(client.execute(&statement, &[&value]).await.unwrap(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    client.batch_execute("ROLLBACK").await.unwrap();
    client.check_connection().await.unwrap();
    finish(client, task).await;
}

// [spec:pgorm:req:turso.values/test]
#[tokio::test]
async fn nonfinite_parameters_are_explicit_refusals() {
    let (client, task) = connect().await;
    client
        .batch_execute("CREATE TABLE floats (value double precision)")
        .await
        .unwrap();
    let wide = client
        .prepare_typed("INSERT INTO floats VALUES ($1)", &[Type::FLOAT8])
        .await
        .unwrap();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        driver_refusal(client.execute(&wide, &[&value]).await);
    }
    let narrow = client
        .prepare_typed("INSERT INTO floats VALUES ($1)", &[Type::FLOAT4])
        .await
        .unwrap();
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        driver_refusal(client.execute(&narrow, &[&value]).await);
    }
    assert_eq!(client.execute(&wide, &[&1.25f64]).await.unwrap(), 1);
    let rows = client.query("SELECT value FROM floats", &[]).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, f64>(0), 1.25);
    finish(client, task).await;
}

// [spec:pgorm:req:turso.values/test]
// [spec:pgorm:req:turso.rows/test]
#[tokio::test]
async fn unsupported_types_do_not_bypass_checks_with_nulls() {
    let (client, task) = connect().await;
    client
        .batch_execute("CREATE TABLE unsupported_values (amount numeric(18,4), numbers integer[], identifier uuid, document json, binary_document jsonb)")
        .await
        .unwrap();
    for column in [
        "amount",
        "numbers",
        "identifier",
        "document",
        "binary_document",
    ] {
        driver_refusal(
            client
                .prepare(&format!(
                    "SELECT {column} FROM unsupported_values WHERE false"
                ))
                .await,
        );
    }
    let null = Encoded {
        bytes: None,
        format: Format::Binary,
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let domain = Type::new(
        "custom_integer".into(),
        90_001,
        Kind::Domain(Type::INT4),
        "public".into(),
    );
    for ty in [Type::NUMERIC, Type::UUID, Type::INT4_ARRAY, domain] {
        let result = match client.prepare_typed("SELECT $1::integer", &[ty]).await {
            Ok(statement) => client.query_one(&statement, &[&null]).await.map(|_| ()),
            Err(error) => Err(error),
        };
        driver_refusal(result);
    }
    client.check_connection().await.unwrap();
    finish(client, task).await;
}

// [spec:pgorm:req:turso.values/test]
// [spec:pgorm:req:turso.rows/test]
#[tokio::test]
async fn custom_codecs_and_row_clones_keep_shared_storage() {
    let (client, task) = connect().await;
    let statement = client
        .prepare_typed(
            "SELECT $1::integer AS number, 'retained'::text AS txt",
            &[Type::INT4],
        )
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let first = Encoded {
        bytes: Some(23i32.to_be_bytes().to_vec()),
        format: Format::Binary,
        calls: Arc::clone(&calls),
    };
    let row = client.query_one(&statement, &[&first]).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let clone = row.clone();
    assert_eq!(row.get::<_, Binary<'_>>(0).0, 23i32.to_be_bytes());
    assert_eq!(
        row.get::<_, Binary<'_>>(0).0.as_ptr(),
        clone.get::<_, Binary<'_>>(0).0.as_ptr()
    );
    assert_eq!(
        row.get::<_, &str>(1).as_ptr(),
        clone.get::<_, &str>(1).as_ptr()
    );
    let second = Encoded {
        bytes: Some(42i32.to_be_bytes().to_vec()),
        format: Format::Binary,
        calls: Arc::clone(&calls),
    };
    assert_eq!(
        client
            .query_one(&statement, &[&second])
            .await
            .unwrap()
            .get::<_, i32>(0),
        42
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    finish(client, task).await;
    assert_eq!(row.get::<_, i32>(0), 23);
    assert_eq!(clone.get::<_, i32>(0), 23);
    assert_eq!(row.get::<_, &str>(1), "retained");
    assert_eq!(clone.get::<_, Binary<'_>>(0).0, 23i32.to_be_bytes());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

// [spec:pgorm:req:turso.errors/test]
#[tokio::test]
async fn typed_execution_retains_native_sqlstate_and_source() {
    let (client, task) = connect().await;
    client
        .batch_execute("CREATE TABLE error_values (id integer PRIMARY KEY)")
        .await
        .unwrap();
    let statement = client
        .prepare_typed("INSERT INTO error_values VALUES ($1)", &[Type::INT4])
        .await
        .unwrap();
    client.execute(&statement, &[&7i32]).await.unwrap();
    let error = client.execute(&statement, &[&7i32]).await.unwrap_err();
    assert_eq!(error.code().unwrap().code(), "23505");
    let database = error.as_db_error().unwrap();
    assert_eq!(database.table(), Some("error_values"));
    assert_eq!(database.constraint(), Some("error_values_pkey"));
    assert!(database.detail().is_some());
    let native = database
        .source()
        .unwrap()
        .downcast_ref::<turso_pg::Error>()
        .unwrap();
    assert_eq!(native.sqlstate, "23505");
    assert!(native.source().is_some());
    client.check_connection().await.unwrap();
    finish(client, task).await;
}
