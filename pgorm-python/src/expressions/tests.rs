use std::ffi::CString;

use pgorm::pgorm_query::{
    BinOper, Condition, Expr, Func, LikeExpr, Name, Query, SimpleExpr, Subscript, TypeName, Value,
};
use pyo3::{prelude::*, types::PyDict};

use super::{Compiled, PyExpr};

fn module(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let module = PyModule::new(py, "pgorm")?;
    crate::values::register(&module)?;
    crate::identifiers::register(&module)?;
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
    let source = CString::new(source)?;
    let actual = py.eval(&source, Some(globals), None)?;
    assert_eq!(actual.extract::<PyRef<'_, PyExpr>>()?.inner, expected);
    let (sql, values) = Query::select().expr(expected).build();
    let built = actual.call_method0("inspect")?;
    let built = built.extract::<PyRef<'_, Compiled>>()?;
    assert_eq!(built.sql, sql);
    assert_eq!(built.values, values);
    Ok(())
}

// [spec:pgorm:req:bindings.delegation/test]
// [spec:pgorm:req:bindings.expressions+3/test]
#[test]
fn expression_structure_matches_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let column: SimpleExpr = Expr::col((
            Name::runtime("schema"),
            Name::runtime("table"),
            Name::runtime("x"),
        ))
        .into();
        let cases = [
            ("p.col('x', table='table', schema='schema')", column.clone()),
            (
                "p.bind(p.Value(7, 'i16')) + p.literal(3)",
                Expr::value(7i16).add(SimpleExpr::Constant(3i64.into())),
            ),
            (
                "(p.col('a') == 1) & ((p.col('b') > 2) | p.col('c').is_null())",
                Expr::col(Name::runtime("a")).eq(1i64).and(
                    Expr::col(Name::runtime("b"))
                        .gt(2i64)
                        .or(Expr::col(Name::runtime("c")).is_null()),
                ),
            ),
            (
                "~(p.col('x') < 3)",
                Expr::col(Name::runtime("x")).lt(3i64).not(),
            ),
            (
                "p.col('x').between(p.literal(1), 2)",
                Expr::col(Name::runtime("x"))
                    .between(SimpleExpr::Constant(1i64.into()), Expr::value(2i64)),
            ),
            (
                "p.col('x').not_between(1, 2)",
                Expr::col(Name::runtime("x")).not_between(1i64, 2i64),
            ),
            (
                "p.col('x').is_in([1, p.literal(2)])",
                Expr::col(Name::runtime("x"))
                    .is_in([Expr::value(1i64), SimpleExpr::Constant(2i64.into())]),
            ),
            (
                "p.col('x').is_in([])",
                Expr::col(Name::runtime("x")).is_in(Vec::<SimpleExpr>::new()),
            ),
            (
                "p.col('x').is_not_in([])",
                Expr::col(Name::runtime("x")).is_not_in(Vec::<SimpleExpr>::new()),
            ),
            (
                "p.tuple_expr(p.col('x'), 2)",
                Expr::tuple([Expr::col(Name::runtime("x")).into(), Expr::value(2i64)]).into(),
            ),
        ];
        for (source, expected) in cases {
            parity(py, &globals, source, expected)?;
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.identifiers+3/test]
#[test]
fn qualified_casts_preserve_source_typed_parameters() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        parity(
            py,
            &globals,
            "p.bind(p.Value('calm', p.TypeName('Mood', schema='Tenant')))",
            Expr::value("calm")
                .cast_as_type(TypeName::new(Name::runtime("Mood")).schema(Name::runtime("Tenant"))),
        )?;
        parity(
            py,
            &globals,
            "p.literal(p.Value('calm', p.TypeName('Mood', schema='Tenant')))",
            SimpleExpr::Constant(Value::from("calm"))
                .cast_as_type(TypeName::new(Name::runtime("Mood")).schema(Name::runtime("Tenant"))),
        )?;
        parity(
            py,
            &globals,
            "p.col('x').cast(p.TypeName('integer'), array=True)",
            Expr::col(Name::runtime("x"))
                .cast_as_type(TypeName::new(Name::runtime("integer")).array()),
        )
    })
}

// [spec:pgorm:req:bindings.expressions+3/test]
#[test]
fn substring_and_pattern_paths_use_rust_functions() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let text: SimpleExpr = Expr::col(Name::runtime("text")).into();
        parity(
            py,
            &globals,
            "p.col('text').starts_with('%_')",
            Func::starts_with(text.clone(), "%_").into(),
        )?;
        let position = Func::named(Name::runtime("strpos")).args([text.clone(), Expr::value("%_")]);
        parity(
            py,
            &globals,
            "p.col('text').contains_text('%_')",
            Expr::expr(position).gt(SimpleExpr::Constant(0i32.into())),
        )?;
        let suffix =
            Func::named(Name::runtime("right")).args([text, Func::char_length("%_").into()]);
        parity(
            py,
            &globals,
            "p.col('text').ends_with('%_')",
            Expr::expr(suffix).eq("%_"),
        )?;
        parity(
            py,
            &globals,
            "p.col('text').like(p.LikeExpr('!%_').escape('!'))",
            Expr::col(Name::runtime("text")).like(LikeExpr::new("!%_").escape('!')),
        )?;
        parity(
            py,
            &globals,
            "p.col('text').ilike(p.LikeExpr('!%_').escape('!'))",
            Expr::col(Name::runtime("text")).ilike(LikeExpr::new("!%_").escape('!')),
        )?;
        parity(
            py,
            &globals,
            "p.col('text').like('a%')",
            Expr::col(Name::runtime("text")).like("a%"),
        )?;
        parity(
            py,
            &globals,
            "p.col('text').not_like(p.LikeExpr('a\\\\%').no_escape())",
            Expr::col(Name::runtime("text")).not_like(LikeExpr::new("a\\%").no_escape()),
        )
    })
}

// [spec:pgorm:req:bindings.expressions+3/test]    each comparison, concatenation,
// [spec:pgorm:req:bindings.ranges/test]
// regular-expression, containment, range position and document search method is the BinOper or
// Rust method of its meaning, its operand bound unless it is an expression, the path bound as text
// cast to jsonpath, and BETWEEN's symmetric form the Rust builder's
#[test]
fn operator_methods_are_their_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let x = || Expr::col(Name::runtime("x"));
        let y = || Expr::col(Name::runtime("y"));
        let cases = [
            ("regex", BinOper::Regex),
            ("not_regex", BinOper::NotRegex),
            ("iregex", BinOper::RegexCaseInsensitive),
            ("not_iregex", BinOper::NotRegexCaseInsensitive),
            ("matches", BinOper::Matches),
            ("concat", BinOper::Concatenate),
        ];
        for (method, operator) in cases {
            parity(
                py,
                &globals,
                &format!("p.col('x').{method}('^a')"),
                x().binary(operator, "^a"),
            )?;
        }
        let cases = [
            ("contains", BinOper::Contains),
            ("contained_by", BinOper::Contained),
            ("overlaps", BinOper::Overlap),
            ("strictly_left_of", BinOper::LShift),
            ("strictly_right_of", BinOper::RShift),
            ("not_extend_right_of", BinOper::NotExtendRightOf),
            ("not_extend_left_of", BinOper::NotExtendLeftOf),
            ("adjacent_to", BinOper::AdjacentTo),
        ];
        for (method, operator) in cases {
            parity(
                py,
                &globals,
                &format!("p.col('x').{method}(p.col('y'))"),
                x().binary(operator, y()),
            )?;
        }
        parity(
            py,
            &globals,
            "p.col('x').is_distinct_from(1)",
            x().is_distinct_from(1i64),
        )?;
        parity(
            py,
            &globals,
            "p.col('x').is_not_distinct_from(p.col('y'))",
            x().is_not_distinct_from(y()),
        )?;
        parity(
            py,
            &globals,
            "p.col('x').between(2, 1, symmetric=True)",
            x().between_symmetric(2i64, 1i64),
        )?;
        parity(
            py,
            &globals,
            "p.col('x').not_between(2, 1, symmetric=True)",
            x().not_between_symmetric(2i64, 1i64),
        )?;
        parity(
            py,
            &globals,
            "p.col('x').path_exists('$.a')",
            x().json_path_exists("$.a"),
        )
    })
}

// [spec:pgorm:req:bindings.expressions+3/test]    at is Rust's Expr::index and slice its
// Subscript::Slice, each bound unless it is an expression, None leaving a slice's end open
#[test]
fn subscripts_are_rust_index_and_slice() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let tags = || Expr::col(Name::runtime("tags"));
        let cases = [
            ("p.col('tags').at(2)", tags().index(2i64).into()),
            (
                "p.col('tags').at(p.col('i')).at(1)",
                tags()
                    .index(Expr::col(Name::runtime("i")))
                    .index(1i64)
                    .into(),
            ),
            (
                "p.col('tags').slice(1, p.literal(2))",
                tags().slice(1i64, SimpleExpr::Constant(2i64.into())).into(),
            ),
            ("p.col('tags').slice(2)", tags().slice_from(2i64).into()),
            ("p.col('tags').slice(upper=3)", tags().slice_to(3i64).into()),
            (
                "p.col('tags').slice()",
                tags().subscript(Subscript::Slice(None, None)).into(),
            ),
        ];
        for (source, expected) in cases {
            parity(py, &globals, source, expected)?;
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.expressions+3/test]
#[test]
fn function_calls_use_named_rust_constructors() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let x = Expr::col(Name::runtime("x"));
        let cases = [
            ("lower", Func::lower(x.clone())),
            ("upper", Func::upper(x.clone())),
            ("abs", Func::abs(x.clone())),
            ("char_length", Func::char_length(x.clone())),
            ("count", Func::count(x.clone())),
            ("count_distinct", Func::count_distinct(x.clone())),
            ("sum", Func::sum(x.clone())),
            ("avg", Func::avg(x.clone())),
            ("min", Func::min(x.clone())),
            ("max", Func::max(x.clone())),
            ("round", Func::round(x.clone())),
            ("coalesce", Func::coalesce([x.clone().into()])),
        ];
        for (name, expected) in cases {
            parity(
                py,
                &globals,
                &format!("p.call('{name}', p.col('x'))"),
                expected.into(),
            )?;
        }
        parity(py, &globals, "p.call('random')", Func::random().into())?;
        parity(
            py,
            &globals,
            "p.call('gen_random_uuid')",
            Func::gen_random_uuid().into(),
        )?;
        parity(py, &globals, "p.call('uuidv4')", Func::uuidv4().into())?;
        parity(py, &globals, "p.call('uuidv7')", Func::uuidv7().into())?;
        parity(
            py,
            &globals,
            "p.call('uuidv7', p.col('x'))",
            Func::uuidv7_shifted(x.clone()).into(),
        )?;
        parity(
            py,
            &globals,
            "p.call('uuid_extract_timestamp', p.col('x'))",
            Func::uuid_extract_timestamp(x.clone()).into(),
        )?;
        parity(
            py,
            &globals,
            "p.call('uuid_extract_version', p.col('x'))",
            Func::uuid_extract_version(x.clone()).into(),
        )
    })
}

// [spec:pgorm:req:bindings.expressions+3/test]
// [spec:pgorm:req:bindings.statements/test]
#[test]
fn conditions_keep_rust_empty_and_grouping_semantics() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let cases = [
            ("p.Condition.all()", Condition::all()),
            ("p.Condition.any()", Condition::any()),
            ("~p.Condition.all()", Condition::all().not()),
            (
                "p.Condition.all(p.col('a') == 1, p.Condition.any(p.col('b') == 2, p.col('c') == 3))",
                Condition::all()
                    .add(Expr::col(Name::runtime("a")).eq(1i64))
                    .add(
                        Condition::any()
                            .add(Expr::col(Name::runtime("b")).eq(2i64))
                            .add(Expr::col(Name::runtime("c")).eq(3i64)),
                    ),
            ),
        ];
        for (source, condition) in cases {
            let actual = py
                .eval(&CString::new(source)?, Some(&globals), None)?
                .call_method0("inspect")?;
            let actual = actual.extract::<PyRef<'_, Compiled>>()?;
            let (sql, values) = Query::select()
                .expr(SimpleExpr::Constant(true.into()))
                .cond_where(condition)
                .build();
            assert_eq!(actual.sql, sql);
            assert_eq!(actual.values, values);
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.case/test]    case_when and case_of, with further arms and an
// else_, are Rust's Expr::case and Expr::case_of, each value and result bound unless it is an
// expression; a CASE goes where an expression goes, and an operand with no arm does not
#[test]
fn case_expressions_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let x = || Expr::col(Name::runtime("x"));
        let kind = || Expr::col(Name::runtime("kind"));
        let searched = Expr::case(x().gt(0i64), "positive").case(
            Condition::all()
                .add(x().lt(0i64))
                .add(Expr::col(Name::runtime("y")).is_not_null()),
            "negative",
        );
        let cases = [
            (
                "p.case_when(p.col('x') > 0, 'positive').when(p.Condition.all(p.col('x') < 0, p.col('y').is_not_null()), 'negative')",
                searched.into(),
            ),
            (
                "p.case_when(p.col('x').is_null(), 0).else_(p.col('x'))",
                Expr::case(x().is_null(), 0i64).finally(x()).into(),
            ),
            (
                "p.case_of(p.col('kind')).when('a', 1).when(p.literal('b'), p.col('x') + 1)",
                Expr::case_of(kind())
                    .when("a", 1i64)
                    .when(SimpleExpr::Constant("b".into()), x().add(1i64))
                    .into(),
            ),
            (
                "p.case_of(p.col('kind')).when('a', 1).else_(-1) + 1",
                SimpleExpr::from(Expr::case_of(kind()).when("a", 1i64).finally(-1i64)).add(1i64),
            ),
            (
                "p.case_when(p.col('x') > 0, 1).eq(p.case_of(p.col('kind')).when('a', 1))",
                Into::<SimpleExpr>::into(Expr::case(x().gt(0i64), 1i64))
                    .binary(BinOper::Equal, Expr::case_of(kind()).when("a", 1i64)),
            ),
        ];
        for (source, expected) in cases {
            parity(py, &globals, source, expected)?;
        }
        for (source, class) in [
            ("p.case_when(1, 'x')", "TypeError"),
            ("p.case_when(p.col('x') > 0, 1).when('x', 2)", "TypeError"),
            ("p.case_of(p.col('x')).inspect()", "AttributeError"),
            ("p.case_of(p.col('x')).else_(1)", "AttributeError"),
            (
                "p.case_when(p.col('x') > 0, 1).else_(0).when(p.col('x') < 0, 2)",
                "AttributeError",
            ),
            ("p.col('x') == p.case_of(p.col('x'))", "ConstructionError"),
            ("p.case_when(p.col('x') > 0, None)", "ConstructionError"),
            ("bool(p.case_of(p.col('x')))", "ConstructionError"),
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            let name = error.get_type(py).name()?;
            assert_eq!(name.to_string(), class, "{source}: {error}");
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.expressions+3/test]
// [spec:pgorm:req:sql.ast.expr.similar-to+1/test]    both forms over a plain string, as Rust's
// IntoLikeExpr reads one, and over a LikeExpr, its ESCAPE carried
#[test]
fn similar_to_takes_like_patterns() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let text = || Expr::col(Name::runtime("text"));
        parity(
            py,
            &globals,
            "p.col('text').similar_to('(a|b)%')",
            text().similar_to("(a|b)%"),
        )?;
        parity(
            py,
            &globals,
            "p.col('text').not_similar_to(p.LikeExpr('!%%').escape('!'))",
            text().not_similar_to(LikeExpr::new("!%%").escape('!')),
        )?;
        let error = py
            .eval(
                c"p.col('text').similar_to(p.Value('a%'))",
                Some(&globals),
                None,
            )
            .expect_err("a pattern is a string or a LikeExpr");
        assert!(error.is_instance_of::<pyo3::exceptions::PyTypeError>(py));
        Ok(())
    })
}

// [spec:pgorm:req:bindings.expressions+3/test]
// [spec:pgorm:req:sql.ast.select.table-function+2/test]    `call` reaches the set-returning
// constructors at their arities, and no other
#[test]
fn set_returning_calls_use_rust_constructors() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        parity(
            py,
            &globals,
            "p.call('generate_series', p.literal(1), p.col('x'))",
            Func::generate_series(
                SimpleExpr::Constant(1i64.into()),
                Expr::col(Name::runtime("x")),
            )
            .into(),
        )?;
        parity(
            py,
            &globals,
            "p.call('generate_series', 10, 1, -3)",
            Func::generate_series_step(Expr::value(10i64), Expr::value(1i64), Expr::value(-3i64))
                .into(),
        )?;
        parity(
            py,
            &globals,
            "p.call('unnest', p.col('tags'))",
            Func::unnest(Expr::col(Name::runtime("tags"))).into(),
        )?;
        for source in [
            "p.call('generate_series', 1)",
            "p.call('generate_series', 1, 2, 3, 4)",
            "p.call('unnest')",
            "p.call('unnest', p.col('a'), p.col('b'))",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<crate::UnsupportedCapabilityError>(py),
                "{source}: {error}"
            );
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.collations/test]    bare and schema-qualified, compared, and as
// an ORDER BY key; a name that is not an identifier is refused
#[test]
fn collations_lower_into_rust_collate() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let name = || Expr::col(Name::runtime("name"));
        parity(
            py,
            &globals,
            "p.col('name').collate('C')",
            name().collate(Name::runtime("C")).into(),
        )?;
        parity(
            py,
            &globals,
            "p.col('name').collate('C \"x\"', schema='pg_catalog') < 'b'",
            name()
                .collate((Name::runtime("pg_catalog"), Name::runtime("C \"x\"")))
                .lt("b"),
        )?;
        let source = "p.col('name').collate('C').desc().expr";
        let ordered = py.eval(&CString::new(source)?, Some(&globals), None)?;
        assert_eq!(
            ordered.extract::<PyRef<'_, PyExpr>>()?.inner,
            name().collate(Name::runtime("C")).into()
        );
        for source in [
            "p.col('name').collate('')",
            "p.col('name').collate('C', schema='')",
            "p.col('name').collate('雪' * 22)",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<crate::errors::ConstructionError>(py),
                "{source}: {error}"
            );
        }
        let error = py
            .eval(c"p.col('name').collate(1)", Some(&globals), None)
            .expect_err("a collation's name is a str");
        assert!(
            error.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
            "{error}"
        );
        Ok(())
    })
}
