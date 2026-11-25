#![cfg(feature = "turso")]

use futures::TryStreamExt;
use pgorm::{Connection, DecodeRaw, Error, HookConnection, entity::prelude::*, set};
use pgorm_driver::{TursoDatabase, types::Type};
use pgorm_pool::ClientWrapper;
use pgorm_query::{ArrayType, Value, Values};
use std::num::NonZeroU64;

mod scalar {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "native_scalar")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub small: i16,
        pub total: i64,
        pub enabled: bool,
        pub label: String,
        pub optional: Option<String>,
        pub bytes: Vec<u8>,
        pub optional_bytes: Option<Vec<u8>>,
        pub ratio: f64,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

async fn native() -> Result<ClientWrapper, Error> {
    let database = TursoDatabase::open(":memory:").await?;
    let (client, connection) = database.connect().await?;
    let task = tokio::spawn(async move {
        connection.await.expect("native connection worker failed");
    });
    Ok(ClientWrapper::new(client, task))
}

async fn schema(db: &HookConnection<'_>) -> Result<(), Error> {
    db.batch_execute(
        "CREATE TABLE native_scalar (
            id INTEGER PRIMARY KEY, small SMALLINT, total BIGINT,
            enabled BOOLEAN, label TEXT, optional TEXT,
            bytes BYTEA, optional_bytes BYTEA, ratio DOUBLE PRECISION
        )",
    )
    .await
}

fn record(id: i32, total: i64) -> scalar::ActiveModel {
    scalar::ActiveModel {
        id: set(id),
        small: set(i16::MIN),
        total: set(total),
        enabled: set(true),
        label: set(format!("record {id}")),
        optional: set(None::<String>),
        bytes: set(vec![0u8, 255, id as u8]),
        optional_bytes: set(None::<Vec<u8>>),
        ratio: set(1.25f64),
    }
}

// [spec:pgorm:req:turso.parameters/test]
// [spec:pgorm:sem:turso.execution/test]
#[tokio::test]
async fn orm_crud_preserves_scalar_values() -> Result<(), Error> {
    let client = native().await?;
    let db = HookConnection::new(&client);
    schema(&db).await?;
    let first = record(1, 9_007_199_254_740_993).insert(&db).await?;
    let last = record(2, i64::MIN).insert(&db).await?;
    assert_eq!(first.total, 9_007_199_254_740_993);
    assert_eq!(last.total, i64::MIN);
    assert_eq!(first.small, i16::MIN);
    assert_eq!(first.bytes, [0, 255, 1]);
    assert!(first.enabled);
    assert_eq!(first.ratio, 1.25);
    assert_eq!(scalar::Entity::find_by_id(1).one(&db).await?, first);

    let mut changed: scalar::ActiveModel = first.into();
    changed.label = set("updated");
    changed.total = set(i64::MAX);
    changed.enabled = set(false);
    let changed = changed.update(&db).await?;
    assert_eq!(changed.total, i64::MAX);
    assert!(!changed.enabled);
    assert_eq!(changed.label, "updated");
    assert_eq!(scalar::Entity::find().count(&db).await?, 2);
    assert_eq!(last.delete(&db).await?, 1);
    assert!(scalar::Entity::find_by_id(2).one_opt(&db).await?.is_none());
    assert_eq!(scalar::Entity::find().count(&db).await?, 1);
    Ok(())
}

// [spec:pgorm:req:turso.parameters/test]
// [spec:pgorm:sem:turso.execution/test]
#[tokio::test]
async fn orm_pagination_and_streaming_use_value_hints() -> Result<(), Error> {
    let client = native().await?;
    let db = HookConnection::new(&client);
    schema(&db).await?;
    for id in 1..=5 {
        record(id, i64::from(id)).insert(&db).await?;
    }
    let selected = scalar::Entity::find()
        .filter(scalar::Column::Id.gt(1))
        .order_by_asc(scalar::Column::Id);
    let pages = selected.clone().paginate(&db, NonZeroU64::new(2).unwrap());
    assert_eq!(pages.num_items().await?, 4);
    assert_eq!(
        pages
            .fetch_page(0)
            .await?
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(
        pages
            .fetch_page(1)
            .await?
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        [4, 5]
    );
    assert!(pages.fetch_page(2).await?.is_empty());
    let expected = selected.clone().all(&db).await?;
    let streamed: Vec<_> = selected.stream(&db).await?.try_collect().await?;
    assert_eq!(streamed, expected);
    let mut cursor = scalar::Entity::find().cursor_by(scalar::Column::Id);
    cursor.after(2);
    assert_eq!(
        cursor
            .first(2)
            .all(&db)
            .await?
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        [3, 4]
    );
    Ok(())
}

// [spec:pgorm:req:turso.parameters/test]
#[tokio::test]
async fn typed_nulls_and_empty_values_survive_orm() -> Result<(), Error> {
    let client = native().await?;
    let db = HookConnection::new(&client);
    schema(&db).await?;
    let mut nulls = record(1, 0);
    nulls.label = set("");
    nulls.bytes = set(Vec::<u8>::new());
    let nulls = nulls.insert(&db).await?;
    let mut empty = record(2, 0);
    empty.optional = set(Some(String::new()));
    empty.optional_bytes = set(Some(Vec::<u8>::new()));
    let empty = empty.insert(&db).await?;
    assert_eq!(nulls.optional, None);
    assert_eq!(nulls.optional_bytes, None);
    assert!(nulls.label.is_empty());
    assert!(nulls.bytes.is_empty());
    assert_eq!(empty.optional.as_deref(), Some(""));
    assert_eq!(empty.optional_bytes.as_deref(), Some(&[][..]));
    assert_eq!(scalar::Entity::find_by_id(1).one(&db).await?, nulls);
    assert_eq!(scalar::Entity::find_by_id(2).one(&db).await?, empty);
    let null_tuple = (
        "SELECT $1, $2, $3, $4",
        Values(vec![
            Value::SmallInt(None),
            Value::Bool(None),
            Value::Double(None),
            Value::BigUnsigned(None),
        ]),
    )
        .into_tuple::<(Option<i16>, Option<bool>, Option<f64>, Option<i64>)>()
        .one(&db)
        .await?;
    assert_eq!(null_tuple, (None, None, None, None));
    Ok(())
}

// [spec:pgorm:req:turso.parameters/test]
#[tokio::test]
async fn unsigned_hints_check_the_signed_native_limit() -> Result<(), Error> {
    let client = native().await?;
    let db = HookConnection::new(&client);
    let tuple = (
        "SELECT $1, $2, $3",
        Values(vec![
            u32::MAX.into(),
            (i64::MAX as u64).into(),
            i64::MIN.into(),
        ]),
    )
        .into_tuple::<(i64, i64, i64)>()
        .one(&db)
        .await?;
    assert_eq!(tuple, (i64::from(u32::MAX), i64::MAX, i64::MIN));
    let error = ("SELECT $1, $2", Values(vec![1i32.into(), u64::MAX.into()]))
        .into_tuple::<(i32, i64)>()
        .one(&db)
        .await
        .unwrap_err();
    assert_eq!(error.bind_refusal().unwrap().parameter, 2);
    assert!(error.sql_error().is_none());
    Ok(())
}

// [spec:pgorm:req:turso.parameters/test]
#[tokio::test]
async fn unsupported_values_fail_before_mutating() -> Result<(), Error> {
    let client = native().await?;
    let db = HookConnection::new(&client);
    db.batch_execute("CREATE TABLE refusal_probe(id INTEGER PRIMARY KEY, value TEXT)")
        .await?;
    let unsupported = [
        Value::Json(Some(Box::new(serde_json::json!({"a": 1})))),
        Value::Json(None),
        Value::Uuid(None),
        Value::Decimal(None),
        Value::Date(None),
        Value::Array(ArrayType::Int, None),
    ];
    for (index, value) in unsupported.into_iter().enumerate() {
        let error = (
            "INSERT INTO refusal_probe(id,value) VALUES ($1,$2) RETURNING id",
            Values(vec![(index as i32).into(), value]),
        )
            .into_tuple::<i32>()
            .one(&db)
            .await
            .unwrap_err();
        let refusal = error.bind_refusal().expect("indexed hint refusal");
        assert_eq!(refusal.parameter, 2);
        assert!(
            refusal
                .reason
                .as_ref()
                .is_some_and(|reason| !reason.is_empty())
        );
        assert!(error.sql_error().is_none());
    }
    assert_eq!(
        db.query_one("SELECT count(*) FROM refusal_probe", &[])
            .await?
            .get::<_, i64>(0),
        0
    );
    Ok(())
}

// [spec:pgorm:req:turso.parameters/test]
#[tokio::test]
async fn native_cache_keys_keep_ordered_parameter_types() -> Result<(), Error> {
    let client = native().await?;
    let db = HookConnection::new(&client);
    let sql = "SELECT $1 AS left_value, $2 AS right_value";
    let first = client
        .prepare_typed_cached(sql, &[Type::INT4, Type::INT8])
        .await?;
    let again = client
        .prepare_typed_cached(sql, &[Type::INT4, Type::INT8])
        .await?;
    assert!(std::ptr::eq(
        first.columns().as_ptr(),
        again.columns().as_ptr()
    ));
    let swapped = client
        .prepare_typed_cached(sql, &[Type::INT8, Type::INT4])
        .await?;
    assert_eq!(first.params(), &[Type::INT4, Type::INT8]);
    assert_eq!(swapped.params(), &[Type::INT8, Type::INT4]);
    assert!(!std::ptr::eq(
        first.columns().as_ptr(),
        swapped.columns().as_ptr()
    ));
    assert_eq!(client.statement_cache.size(), 2);
    assert_eq!(
        (sql, Values(vec![7i32.into(), i64::MAX.into()]))
            .into_tuple::<(i32, i64)>()
            .one(&db)
            .await?,
        (7, i64::MAX)
    );
    assert_eq!(
        (sql, Values(vec![i64::MIN.into(), 8i32.into()]))
            .into_tuple::<(i64, i32)>()
            .one(&db)
            .await?,
        (i64::MIN, 8)
    );
    assert_eq!(
        client.statement_cache.size(),
        2,
        "ORM hints reuse the exact typed cache entries"
    );
    assert!(
        client
            .statement_cache
            .remove(sql, &[Type::INT4, Type::INT8])
            .is_some()
    );
    assert_eq!(client.statement_cache.size(), 1);
    assert!(
        client
            .statement_cache
            .remove(sql, &[Type::INT8, Type::INT4])
            .is_some()
    );
    assert_eq!(client.statement_cache.size(), 0);
    Ok(())
}

// [spec:pgorm:req:turso.parameters/test]
#[tokio::test]
async fn raw_parameters_require_explicit_typed_preparation() -> Result<(), Error> {
    let client = native().await?;
    let db = HookConnection::new(&client);
    let error = db.query_one("SELECT $1", &[&7i32]).await.unwrap_err();
    assert!(error.sql_error().is_none());
    assert_eq!(client.statement_cache.size(), 0);
    let prepared = client
        .prepare_typed_cached("SELECT $1", &[Type::INT4])
        .await?;
    assert_eq!(
        client
            .query_one(&prepared, &[&7i32])
            .await?
            .get::<_, i32>(0),
        7
    );
    assert_eq!(client.statement_cache.size(), 1);
    Ok(())
}

// [spec:pgorm:req:conn.pool.statement-cache.invalidate+6/test]
// [spec:pgorm:sem:turso.execution/test]
#[tokio::test]
async fn native_invalidation_evicts_without_replaying() -> Result<(), Error> {
    let database = TursoDatabase::open(":memory:").await?;
    let (client, connection) = database.connect().await?;
    let client = ClientWrapper::new(
        client,
        tokio::spawn(async move {
            connection.await.expect("native connection worker failed");
        }),
    );
    let db = HookConnection::new(&client);
    db.batch_execute(
        "CREATE TABLE invalidation(value INTEGER); INSERT INTO invalidation VALUES(0)",
    )
    .await?;
    let sql = "UPDATE invalidation SET value=value+1 RETURNING *";
    assert_eq!(db.query_one(sql, &[]).await?.get::<_, i32>(0), 1);
    assert_eq!(client.statement_cache.size(), 1);
    // The idle cached connection has not observed this other connection's
    // schema change, so execution must refuse before any write can run.
    let (other, connection) = database.connect().await?;
    let other = ClientWrapper::new(
        other,
        tokio::spawn(async move {
            connection.await.expect("native connection worker failed");
        }),
    );
    other
        .batch_execute("ALTER TABLE invalidation ADD COLUMN extra TEXT")
        .await?;
    let error = db.query_one(sql, &[]).await.unwrap_err();
    assert!(matches!(&error, Error::Postgres(error) if error.is_statement_invalidated()));
    assert_eq!(client.statement_cache.size(), 0);
    assert_eq!(
        client
            .query_one("SELECT value FROM invalidation", &[])
            .await?
            .get::<_, i32>(0),
        1
    );
    let fresh = db.query_one(sql, &[]).await?;
    assert_eq!(fresh.len(), 2);
    assert_eq!(fresh.get::<_, i32>(0), 2);
    assert_eq!(fresh.get::<_, Option<String>>(1), None);
    Ok(())
}

// [spec:pgorm:req:conn.pool.statement-cache.invalidate+6/test]
#[tokio::test]
async fn cache_discards_a_flagged_native_statement() -> Result<(), Error> {
    let client = native().await?;
    client
        .batch_execute(
            "CREATE TABLE invalidation(value INTEGER); INSERT INTO invalidation VALUES(7)",
        )
        .await?;
    let sql = "SELECT * FROM invalidation";
    let stale = client.prepare_typed_cached(sql, &[]).await?;
    client
        .batch_execute("ALTER TABLE invalidation ADD COLUMN extra TEXT")
        .await?;
    assert!(stale.is_invalidated());
    let error = client.query(&stale, &[]).await.unwrap_err();
    assert!(error.is_statement_invalidated());
    assert!(stale.is_invalidated());
    let fresh = client.prepare_typed_cached(sql, &[]).await?;
    assert!(!fresh.is_invalidated());
    assert_eq!(fresh.columns().len(), 2);
    assert!(!std::ptr::eq(
        stale.columns().as_ptr(),
        fresh.columns().as_ptr()
    ));
    assert_eq!(client.statement_cache.size(), 1);
    // A delayed failure from the old handle must not evict its healthy replacement.
    client.statement_cache.remove_invalidated(sql, &[]);
    let still_fresh = client.prepare_typed_cached(sql, &[]).await?;
    assert!(std::ptr::eq(
        fresh.columns().as_ptr(),
        still_fresh.columns().as_ptr()
    ));
    assert_eq!(client.statement_cache.size(), 1);
    let row = client.query_one(&fresh, &[]).await?;
    assert_eq!(row.get::<_, i32>(0), 7);
    assert_eq!(row.get::<_, Option<String>>(1), None);
    Ok(())
}
