use std::ffi::CString;

use pgorm::pgorm_query::{
    Asterisk, ColumnRef, ColumnType, Expr, ExprTrait, Func, IntoNamedTable, JoinType,
    JsonExistsBehavior, JsonKind, JsonQueryBehavior, JsonTableBehavior, JsonTableColumn,
    JsonValueBehavior, JsonValueType, Name, Order, Query, SelectStatement, SimpleExpr,
};
use pyo3::{prelude::*, types::PyDict};

use crate::expressions::{Compiled, PyExpr};

fn module(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let module = PyModule::new(py, "pgorm")?;
    crate::values::register(&module)?;
    crate::identifiers::register(&module)?;
    crate::expressions::register(&module)?;
    crate::statements::register(&module)?;
    crate::schema::install(&module)?;
    super::register(&module)?;
    let globals = PyDict::new(py);
    globals.set_item("p", module)?;
    Ok(globals)
}

fn parity(
    py: Python<'_>,
    globals: &Bound<'_, PyDict>,
    source: &str,
    expected: SimpleExpr,
) -> PyResult<()> {
    let actual = py.eval(&CString::new(source)?, Some(globals), None)?;
    assert_eq!(
        actual.extract::<PyRef<'_, PyExpr>>()?.inner,
        expected,
        "{source}"
    );
    let (sql, values) = Query::select().expr(expected).build();
    let built = actual.call_method0("inspect")?;
    let built = built.extract::<PyRef<'_, Compiled>>()?;
    assert_eq!(built.sql, sql, "{source}");
    assert_eq!(built.values, values, "{source}");
    Ok(())
}

fn refused(py: Python<'_>, globals: &Bound<'_, PyDict>, source: &str, message: &str) {
    let error = py
        .eval(&CString::new(source).unwrap(), Some(globals), None)
        .expect_err(source);
    assert!(
        error.is_instance_of::<crate::errors::ConstructionError>(py),
        "{source}: {error}"
    );
    assert!(error.to_string().contains(message), "{source}: {error}");
}

/// `source` refused as an argument of a type its call does not take.
fn mistyped(py: Python<'_>, globals: &Bound<'_, PyDict>, source: &str, message: &str) {
    let error = py
        .eval(&CString::new(source).unwrap(), Some(globals), None)
        .expect_err(source);
    assert!(
        error.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
        "{source}: {error}"
    );
    assert!(error.to_string().contains(message), "{source}: {error}");
}

fn doc() -> Expr {
    Expr::col(Name::runtime("doc"))
}

// [spec:pgorm:req:bindings.sql-json/test]
#[test]
fn query_functions_match_the_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let cases = [
            (
                "p.json_exists(p.col('doc'), '$.tags[*] ? (@ == $Tag)', passing={'Tag': 'blue', 'n': p.literal(2)})",
                Func::json_exists(doc(), "$.tags[*] ? (@ == $Tag)")
                    .passing("blue", Name::runtime("Tag"))
                    .passing(SimpleExpr::Constant(2i64.into()), Name::runtime("n"))
                    .into(),
            ),
            (
                "p.json_exists(p.col('doc'), '$.a', on_error=p.JsonExistsBehavior.True_)",
                Func::json_exists(doc(), "$.a")
                    .on_error(JsonExistsBehavior::True)
                    .into(),
            ),
            (
                "p.json_exists(p.col('doc'), '$.a', on_error=p.JsonExistsBehavior.False_)",
                Func::json_exists(doc(), "$.a")
                    .on_error(JsonExistsBehavior::False)
                    .into(),
            ),
            (
                "p.json_exists(p.col('doc'), '$.a', on_error=p.JsonExistsBehavior.Unknown)",
                Func::json_exists(doc(), "$.a")
                    .on_error(JsonExistsBehavior::Unknown)
                    .into(),
            ),
            (
                "p.json_exists(p.format_json(p.col('doc')), '$.a', on_error=p.JsonExistsBehavior.Error)",
                Func::json_exists(doc().format_json(), "$.a")
                    .on_error(JsonExistsBehavior::Error)
                    .into(),
            ),
            (
                "p.json_value(p.col('doc'), '$.size', returning='integer', \
                 on_empty=p.JsonDefault(0), on_error=p.JsonValueBehavior.Error)",
                Func::json_value(doc(), "$.size")
                    .returning(JsonValueType::Integer)
                    .on_empty(JsonValueBehavior::Default(0i64.into()))
                    .on_error(JsonValueBehavior::Error)
                    .into(),
            ),
            (
                "p.json_value(p.col('doc'), '$.when', passing={'x': 1}, \
                 returning=p.DataType('numeric', precision=10, scale=2), \
                 on_empty=p.JsonValueBehavior.Null, on_error=p.JsonDefault(\"it's\"))",
                Func::json_value(doc(), "$.when")
                    .passing(1i64, Name::runtime("x"))
                    .returning(JsonValueType::Decimal(Some((10, 2))))
                    .on_empty(JsonValueBehavior::Null)
                    .on_error(JsonValueBehavior::Default("it's".into()))
                    .into(),
            ),
            (
                "p.json_query(p.col('doc'), '$.tags[*]', shaping='with_wrapper', \
                 on_empty=p.JsonQueryBehavior.EmptyArray, on_error=p.JsonQueryBehavior.EmptyObject)",
                Func::json_query(doc(), "$.tags[*]")
                    .with_wrapper()
                    .on_empty(JsonQueryBehavior::EmptyArray)
                    .on_error(JsonQueryBehavior::EmptyObject)
                    .into(),
            ),
            (
                "p.json_query(p.col('doc'), '$.tags[*]', shaping='with_conditional_wrapper', \
                 returning='json', on_empty=p.JsonQueryBehavior.Null, on_error=p.JsonQueryBehavior.Error)",
                Func::json_query(doc(), "$.tags[*]")
                    .returning(ColumnType::Json)
                    .with_conditional_wrapper()
                    .on_empty(JsonQueryBehavior::Null)
                    .on_error(JsonQueryBehavior::Error)
                    .into(),
            ),
            (
                "p.json_query(p.col('doc'), '$.name', shaping='omit_quotes', returning='text', \
                 on_error=p.JsonDefault(p.Value.json({'a': 1})))",
                Func::json_query(doc(), "$.name")
                    .returning(ColumnType::Text)
                    .omit_quotes()
                    .on_error(JsonQueryBehavior::Default(
                        serde_json::json!({"a": 1}).into(),
                    ))
                    .into(),
            ),
        ];
        for (source, expected) in cases {
            parity(py, &globals, source, expected)?;
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.sql-json/test]
#[test]
fn constructors_match_the_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let id = || Expr::col(Name::runtime("id"));
        let cases = [
            ("p.json_object()", Func::json_object().into()),
            (
                "p.json_object({'id': p.col('id'), 'size': 12}, absent_on_null=True, \
                 unique_keys=True, returning='jsonb')",
                Func::json_object()
                    .entry("id", id())
                    .entry("size", 12i64)
                    .absent_on_null()
                    .with_unique_keys()
                    .returning(ColumnType::JsonBinary)
                    .into(),
            ),
            (
                "p.json_object([(p.col('k'), p.format_json(p.col('doc'))), (p.literal('n'), p.Value.null('text'))])",
                Func::json_object()
                    .entry(Expr::col(Name::runtime("k")), doc().format_json())
                    .entry(
                        SimpleExpr::Constant("n".into()),
                        pgorm::pgorm_query::Value::String(None),
                    )
                    .into(),
            ),
            (
                "p.json_array(1, 'a', p.col('id'), null_on_null=True, returning='text')",
                Func::json_array()
                    .element(1i64)
                    .element("a")
                    .element(id())
                    .null_on_null()
                    .returning(ColumnType::Text)
                    .into(),
            ),
            ("p.json_array()", Func::json_array().into()),
            (
                "p.json_array_query(p.Select(p.col('id')).from_(p.Table('t')), returning='json')",
                Func::json_array_query(Query::select().expr(id()).from(Name::runtime("t")).take())
                    .returning(ColumnType::Json)
                    .into(),
            ),
            (
                "p.json_objectagg(p.col('k'), p.col('id'), absent_on_null=True, unique_keys=True, \
                 returning='jsonb', filter=p.col('id') > 1)",
                Func::json_objectagg(Expr::col(Name::runtime("k")), id())
                    .absent_on_null()
                    .with_unique_keys()
                    .returning(ColumnType::JsonBinary)
                    .filter(id().gt(1i64))
                    .into(),
            ),
            (
                "p.json_arrayagg(p.col('id'), order_by=[p.col('k').desc(), p.col('id').asc()], \
                 null_on_null=True, returning='json', filter=p.Condition.all(p.col('id') > 1))",
                Func::json_arrayagg(id())
                    .order_by(Expr::col(Name::runtime("k")), Order::Desc)
                    .order_by(id(), Order::Asc)
                    .null_on_null()
                    .returning(ColumnType::Json)
                    .filter(pgorm::pgorm_query::Condition::all().add(id().gt(1i64)))
                    .into(),
            ),
            (
                "p.json_parse('{\"a\":1}', unique_keys=True)",
                Func::json(r#"{"a":1}"#).with_unique_keys().into(),
            ),
            ("p.json_parse(p.col('doc'))", Func::json(doc()).into()),
            ("p.json_scalar(5)", Func::json_scalar(5i64)),
            (
                "p.json_serialize(p.col('doc'), returning='bytea')",
                Func::json_serialize(doc())
                    .returning(ColumnType::Bytea)
                    .into(),
            ),
            ("p.is_json(p.col('doc'))", doc().is_json(JsonKind::Value)),
            (
                "p.is_json(p.col('doc'), p.JsonKind.Object, unique_keys=True)",
                doc().is_json(JsonKind::Object.with_unique_keys()),
            ),
            (
                "p.is_not_json(p.col('doc'), p.JsonKind.Array)",
                doc().is_not_json(JsonKind::Array),
            ),
            (
                "p.is_not_json(p.col('doc'), p.JsonKind.Scalar, unique_keys=True)",
                doc().is_not_json(JsonKind::Scalar.with_unique_keys()),
            ),
        ];
        for (source, expected) in cases {
            parity(py, &globals, source, expected)?;
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.sql-json/test]
#[test]
fn unrepresentable_choices_are_refused() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        for (source, message) in [
            (
                "p.json_value(p.col('doc'), '$.a', returning='jsonb')",
                "bug #19695",
            ),
            (
                "p.json_value(p.col('doc'), '$.a', returning='json')",
                "bug #19695",
            ),
            (
                "p.json_arrayagg(p.col('id'), order_by=[p.col('id').asc(nulls=p.Nulls.Last)])",
                "NULLS FIRST or NULLS LAST",
            ),
            (
                "p.JsonDefault(p.Value('calm', p.TypeName('Mood')))",
                "created range's cast",
            ),
        ] {
            refused(py, &globals, source, message);
        }
        for (source, message) in [
            (
                "p.json_query(p.col('doc'), '$.a', shaping='keep_quotes')",
                "shaping requires",
            ),
            (
                "p.json_value(p.col('doc'), '$.a', on_error=p.JsonQueryBehavior.EmptyArray)",
                "JsonValueBehavior or JsonDefault",
            ),
            (
                "p.json_query(p.col('doc'), '$.a', on_empty=p.JsonValueBehavior.Null)",
                "JsonQueryBehavior or JsonDefault",
            ),
            (
                "p.json_arrayagg(p.col('id'), order_by=p.col('id').asc())",
                "list or tuple of orderings",
            ),
            ("p.json_object({'a'})", "dict or a list or tuple"),
            ("p.json_object([('a', 1, 2)])", "(key, value) tuples"),
        ] {
            mistyped(py, &globals, source, message);
        }
        Ok(())
    })
}

/// The SQL and parameters a Python SELECT builds, beside the Rust builder's.
fn statement_parity(
    py: Python<'_>,
    globals: &Bound<'_, PyDict>,
    source: &str,
    expected: &SelectStatement,
) -> PyResult<()> {
    let built = py
        .eval(&CString::new(source)?, Some(globals), None)?
        .call_method0("inspect")?;
    let built = built.extract::<PyRef<'_, Compiled>>()?;
    let (sql, values) = expected.build();
    assert_eq!(built.sql, sql, "{source}");
    assert_eq!(built.values, values, "{source}");
    Ok(())
}

// [spec:pgorm:req:bindings.sql-json/test]
#[test]
fn json_table_matches_the_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let n = Name::runtime;
        py.run(
            cr#"d = p.Table('docs', alias='d')
C = p.JsonTableColumn
jt = p.json_table(
    d.col('doc'), '$.items[*] ? (@.n > $Min)', C.ordinality('i'),
    C.value('n', 'integer', path='$.n', on_empty=p.JsonDefault(-1), on_error=p.JsonValueBehavior.Error),
    C.query('Tags', 'jsonb', shaping='with_conditional_wrapper',
            on_empty=p.JsonQueryBehavior.EmptyArray, on_error=p.JsonQueryBehavior.Null),
    C.exists('flag', 'boolean', path='strict $.flag', on_error=p.JsonExistsBehavior.Unknown),
    C.nested('$.parts[*]', C.value('part', 'text', path='$'), C.ordinality('j'), path_name='parts'),
    alias='jt', passing={'Min': 1}, path_name='root', on_error=p.JsonTableBehavior.Error)
"#,
            Some(&globals),
            None,
        )?;
        let doc = || Expr::col((n("d"), n("doc")));
        let table = Func::json_table(
            doc(),
            "$.items[*] ? (@.n > $Min)",
            JsonTableColumn::ordinality(n("i")),
        )
        .column(
            JsonTableColumn::value(n("n"), ColumnType::Integer)
                .path("$.n")
                .on_empty(JsonValueBehavior::Default((-1i64).into()))
                .on_error(JsonValueBehavior::Error),
        )
        .column(
            JsonTableColumn::query(n("Tags"), ColumnType::JsonBinary)
                .with_conditional_wrapper()
                .on_empty(JsonQueryBehavior::EmptyArray)
                .on_error(JsonQueryBehavior::Null),
        )
        .column(
            JsonTableColumn::exists(n("flag"), ColumnType::Boolean)
                .path("strict $.flag")
                .on_error(JsonExistsBehavior::Unknown),
        )
        .column(
            JsonTableColumn::nested(
                "$.parts[*]",
                JsonTableColumn::value(n("part"), ColumnType::Text).path("$"),
            )
            .column(JsonTableColumn::ordinality(n("j")))
            .path_name(n("parts")),
        )
        .passing(1i64, n("Min"))
        .path_name(n("root"))
        .on_error(JsonTableBehavior::Error)
        .alias(n("jt"));
        let docs = n("docs").into_named_table().alias(n("d"));
        statement_parity(
            py,
            &globals,
            "p.Select(d.col('id'), jt.col('n'), jt.star()).from_(d).from_(jt)",
            &Query::select()
                .expr(Expr::col((n("d"), n("id"))))
                .expr(Expr::col((n("jt"), n("n"))))
                .expr(Expr::col(ColumnRef::TableAsterisk(n("jt"))))
                .from(docs.clone())
                .from(table)
                .take(),
        )?;

        let labels = || {
            Func::json_table(
                doc(),
                "$.items[*]",
                JsonTableColumn::value(n("label"), ColumnType::Text),
            )
            .alias(n("l"))
        };
        py.run(
            c"l = p.json_table(d.col('doc'), '$.items[*]', C.value('label', 'text'), alias='l')",
            Some(&globals),
            None,
        )?;
        statement_parity(
            py,
            &globals,
            "p.Select(d.col('id'), l.col('label')).from_(d).join(l, p.literal(True), kind=p.Join.Left)",
            &Query::select()
                .expr(Expr::col((n("d"), n("id"))))
                .expr(Expr::col((n("l"), n("label"))))
                .from(docs.clone())
                .join(
                    JoinType::LeftJoin,
                    labels(),
                    SimpleExpr::Constant(true.into()),
                )
                .take(),
        )?;
        statement_parity(
            py,
            &globals,
            "p.Select().from_(d).cross_join(p.json_table(p.format_json(p.col('raw')), '$[*]', C.ordinality('k'), alias='r'))",
            &Query::select()
                .column(Asterisk)
                .from(docs)
                .cross_join(
                    Func::json_table(
                        Expr::col(n("raw")).format_json(),
                        "$[*]",
                        JsonTableColumn::ordinality(n("k")),
                    )
                    .alias(n("r")),
                )
                .take(),
        )
    })
}

// [spec:pgorm:req:bindings.sql-json/test]
#[test]
fn json_table_refuses_what_its_builder_cannot_take() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        refused(
            py,
            &globals,
            "p.json_table(p.col('doc'), '$', p.JsonTableColumn.ordinality(''), alias='t')",
            "identifier parts",
        );
        for (source, message) in [
            (
                "p.JsonTableColumn.value('v', 'integer', on_empty=p.JsonQueryBehavior.EmptyArray)",
                "JsonValueBehavior or JsonDefault",
            ),
            (
                "p.JsonTableColumn.query('v', 'jsonb', on_error=p.JsonValueBehavior.Error)",
                "JsonQueryBehavior or JsonDefault",
            ),
            (
                "p.JsonTableColumn.query('v', 'jsonb', shaping='keep_quotes')",
                "shaping requires",
            ),
            (
                "p.json_table(p.col('doc'), '$', p.JsonTableColumn.ordinality('i'), 'v', alias='t')",
                "require JsonTableColumn",
            ),
            (
                "p.JsonTableColumn.nested('$', p.JsonTableColumn.ordinality('i'), 'v')",
                "require JsonTableColumn",
            ),
            ("p.Select().from_('docs')", "requires a Table or FromItem"),
            (
                "p.Select().from_(p.Table('a')).join(p.col('b'), p.literal(True))",
                "requires a Table or FromItem",
            ),
        ] {
            mistyped(py, &globals, source, message);
        }
        for source in [
            "p.json_table(p.col('doc'), '$', alias='t')",
            "p.json_table(p.col('doc'), '$', p.JsonTableColumn.ordinality('i'))",
            "p.JsonTableColumn.nested('$')",
            "p.JsonTableColumn.exists('v', 'boolean', on_empty=p.JsonExistsBehavior.Error)",
            "p.json_table(p.col('doc'), '$', p.JsonTableColumn.ordinality('i'), alias='t', on_error=p.JsonValueBehavior.Null)",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
                "{source}: {error}"
            );
        }
        Ok(())
    })
}
