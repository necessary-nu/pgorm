use super::*;
use std::error::Error as _;

fn connection() -> turso_pg::PgConnection {
    let (_, database) = turso_pg::open_database(
        ":memory:",
        None,
        turso_pg::OpenFlags::default(),
        turso_pg::tursopg_database_opts(),
    )
    .unwrap();
    turso_pg::PgConnection::connect(&database).unwrap()
}

fn types(connection: &turso_pg::PgConnection, sql: &str, params: &[Type]) -> Vec<Type> {
    let statement = connection.prepare(sql).unwrap();
    let (columns, codecs) = describe_columns(&statement, params).unwrap();
    assert_eq!(columns.len(), codecs.len());
    columns.into_iter().map(|column| column.r#type).collect()
}

// [spec:pgorm:req:turso.rows/test]
#[test]
fn declared_types_do_not_depend_on_rows() {
    let connection = connection();
    connection.execute("CREATE TABLE typed(s smallint, i integer, b bigint, flag boolean, t text, v varchar(8), bytes bytea, r real)").unwrap();
    let expected = vec![
        Type::INT2,
        Type::INT4,
        Type::INT8,
        Type::BOOL,
        Type::TEXT,
        Type::VARCHAR,
        Type::BYTEA,
        Type::FLOAT8,
    ];
    assert_eq!(
        types(&connection, "SELECT * FROM typed WHERE false", &[]),
        expected
    );
    connection
        .execute("INSERT INTO typed DEFAULT VALUES")
        .unwrap();
    assert_eq!(types(&connection, "SELECT * FROM typed", &[]), expected);
}

// [spec:pgorm:req:turso.rows/test]
#[test]
fn count_and_predicates_use_compiled_expressions() {
    let connection = connection();
    connection
        .execute("CREATE TABLE typed(i integer, flag boolean)")
        .unwrap();
    assert_eq!(
        types(
            &connection,
            "SELECT count(*) AS misleading, count(i) FROM typed",
            &[]
        ),
        vec![Type::INT8, Type::INT8],
    );
    assert_eq!(
        types(
            &connection,
            "SELECT EXISTS(SELECT 1 FROM typed), i=1, i IS NULL, NOT flag, flag AND true FROM typed",
            &[]
        ),
        vec![Type::BOOL, Type::BOOL, Type::BOOL, Type::BOOL, Type::BOOL],
    );
    assert_eq!(
        types(
            &connection,
            "SELECT 2147483648, 1 AS bool, $1 AS bigint",
            &[Type::BYTEA]
        ),
        vec![Type::INT8, Type::INT4, Type::BYTEA]
    );
}

// [spec:pgorm:req:turso.rows/test]
#[test]
fn erased_cast_types_use_documented_mapping() {
    let connection = connection();
    assert_eq!(
        types(
            &connection,
            "SELECT NULL::smallint, NULL::varchar, NULL::bpchar, NULL::float4, NULL::float8, NULL::bytea, NULL::boolean, NULL::bigint",
            &[]
        ),
        vec![
            Type::INT4,
            Type::TEXT,
            Type::TEXT,
            Type::FLOAT8,
            Type::FLOAT8,
            Type::BYTEA,
            Type::BOOL,
            Type::INT8
        ],
    );
    connection.execute("CREATE TABLE typed(c bpchar)").unwrap();
    assert_eq!(
        types(&connection, "SELECT c FROM typed", &[]),
        vec![Type::TEXT]
    );
    // These original identities are absent from both native metadata and the
    // compiled cast. Advertising UUID/JSON/NUMERIC here would invent fidelity.
    assert_eq!(
        types(
            &connection,
            "SELECT NULL::uuid, NULL::json, NULL::jsonb, NULL::numeric",
            &[]
        ),
        vec![Type::TEXT, Type::TEXT, Type::TEXT, Type::FLOAT8],
    );
}

// [spec:pgorm:req:turso.rows/test]
#[test]
fn erased_uuid_cast_refuses_uuid_decoding() {
    #[derive(Debug)]
    struct UuidIdentity;
    impl<'a> crate::types::FromSql<'a> for UuidIdentity {
        fn from_sql(
            _: &Type,
            _: &'a [u8],
        ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
            panic!("a UUID decoder must not receive a TEXT result");
        }
        fn accepts(ty: &Type) -> bool {
            *ty == Type::UUID
        }
    }
    let connection = connection();
    let native = connection.prepare("SELECT NULL::uuid").unwrap();
    let (columns, _) = describe_columns(&native, &[]).unwrap();
    let row = crate::Row::from_native(
        crate::Statement::unnamed(Vec::new(), columns),
        bytes::Bytes::new(),
        vec![None],
    );
    let error = row.try_get::<_, Option<UuidIdentity>>(0).unwrap_err();
    assert!(error.source().unwrap().is::<crate::types::WrongType>());
}

// [spec:pgorm:req:turso.rows/test]
// [spec:pgorm:req:turso.errors/test]
#[test]
fn unresolved_types_fail_before_consuming_rows() {
    let connection = connection();
    connection
        .execute(
            "CREATE TABLE typed(i integer, a integer[], n numeric(18,4), u uuid, j json, jb jsonb)",
        )
        .unwrap();
    for sql in [
        "SELECT NULL",
        "SELECT 1+2 AS int4",
        "SELECT 1.25",
        "SELECT i+1 FROM typed",
        "SELECT a FROM typed",
        "SELECT n FROM typed",
        "SELECT u FROM typed",
        "SELECT j FROM typed",
        "SELECT jb FROM typed",
    ] {
        let statement = connection.prepare(sql).unwrap();
        let error = describe_columns(&statement, &[]).unwrap_err();
        assert!(error.code().is_none(), "{sql}");
        assert!(error.to_string().contains("result column typing"), "{sql}");
        assert!(
            error.source().unwrap().to_string().contains("column 1"),
            "{sql}"
        );
    }
}
