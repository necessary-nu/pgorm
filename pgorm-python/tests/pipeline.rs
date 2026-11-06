//! Independent Rust construction checks the Python lowering's SQL and Values.

use pgorm::pgorm_query::{Name, TypeName};
use pgorm::{
    pgorm_query::Values,
    pipeline::{self as pl, ExprOps, IntoSource, Pipeline},
};
use pgorm_python::expressions::Compiled;
use pyo3::prelude::*;
use std::{collections::BTreeMap, ffi::CString};

fn programs() -> BTreeMap<&'static str, Pipeline> {
    let base = Pipeline::from(Name::runtime("items"));
    let key = pl::col(Name::runtime("items"), Name::runtime("id"));
    let category = pl::col(Name::runtime("items"), Name::runtime("category"));
    let amount = pl::col(Name::runtime("items"), Name::runtime("amount"));
    let total = pl::alias("total");
    let rank = pl::alias("row_rank");
    let left = base
        .clone()
        .filter_with(|b| pl::col(Name::runtime("items"), Name::runtime("id")).gt(b.bind(1i32)))
        .select((
            pl::col(Name::runtime("items"), Name::runtime("id")),
            pl::col(Name::runtime("items"), Name::runtime("category")),
            pl::col(Name::runtime("items"), Name::runtime("amount")),
        ));
    let right = base
        .clone()
        .filter_with(|b| pl::col(Name::runtime("items"), Name::runtime("id")).lt(b.bind(8i32)))
        .select((
            pl::col(Name::runtime("items"), Name::runtime("id")),
            pl::col(Name::runtime("items"), Name::runtime("category")),
            pl::col(Name::runtime("items"), Name::runtime("amount")),
        ));
    BTreeMap::from([
        (
            "literal",
            base.clone()
                .filter(pl::col(Name::runtime("items"), Name::runtime("id")).gt(2i64))
                .select((
                    pl::col(Name::runtime("items"), Name::runtime("id")),
                    pl::col(Name::runtime("items"), Name::runtime("amount")),
                ))
                .sort(pl::col(Name::runtime("items"), Name::runtime("id")).desc())
                .take(3),
        ),
        (
            "repeated",
            base.clone().filter_with(|b| {
                b.bind("unused");
                let value = b.bind(2i32);
                pl::col(Name::runtime("items"), Name::runtime("id"))
                    .gt(value.clone())
                    .and(
                        pl::col(Name::runtime("items"), Name::runtime("id"))
                            .lt(value.add(b.bind(10i64))),
                    )
            }),
        ),
        (
            "literal_string",
            base.clone().filter(
                pl::col(Name::runtime("items"), Name::runtime("category")).eq("O'Brien; -- 雪"),
            ),
        ),
        (
            "derive",
            base.clone()
                .derive_with(|b| {
                    [pl::col(Name::runtime("items"), Name::runtime("amount"))
                        .add(b.bind(2i64))
                        .as_(total)]
                })
                .filter_with(|b| total.gt(b.bind(5i64))),
        ),
        (
            "derive_with",
            base.clone().derive_with(|b| {
                [pl::col(Name::runtime("items"), Name::runtime("amount"))
                    .add(b.bind(2i64))
                    .as_(total)]
            }),
        ),
        (
            "select_with",
            base.clone().select_with(|b| {
                [
                    pl::col(Name::runtime("items"), Name::runtime("id")),
                    b.bind("payload").as_("payload"),
                ]
            }),
        ),
        (
            "group",
            base.clone()
                .group(pl::col(Name::runtime("items"), Name::runtime("category")))
                .aggregate(
                    pl::sum(pl::col(Name::runtime("items"), Name::runtime("amount"))).as_(total),
                )
                .filter_with(|b| total.gt(b.bind(3i64))),
        ),
        (
            "group_with",
            base.clone()
                .group_with(|b| {
                    [pl::col(Name::runtime("items"), Name::runtime("category"))
                        .coalesce(b.bind("missing"))]
                })
                .aggregate_with(|b| {
                    [
                        pl::sum(pl::col(Name::runtime("items"), Name::runtime("amount")))
                            .add(b.bind(1i64))
                            .as_(total),
                    ]
                }),
        ),
        (
            "window",
            base.clone()
                .window(
                    pl::row_number().as_(rank),
                    pl::over()
                        .by(pl::col(Name::runtime("items"), Name::runtime("category")))
                        .sort_by(pl::col(Name::runtime("items"), Name::runtime("id")))
                        .rows(None, Some(0)),
                )
                .filter_with(|b| rank.lte(b.bind(2i64))),
        ),
        (
            "window_with",
            base.clone().window_with(
                pl::over().sort_by(pl::col(Name::runtime("items"), Name::runtime("id"))),
                |b| {
                    [
                        pl::sum(pl::col(Name::runtime("items"), Name::runtime("amount")))
                            .add(b.bind(2i64))
                            .as_(total),
                    ]
                },
            ),
        ),
        (
            "sort_with",
            base.clone()
                .sort_with(|b| {
                    [pl::col(Name::runtime("items"), Name::runtime("id")).add(b.bind(1i64))]
                })
                .take_range(2..=4),
        ),
        (
            "join",
            base.clone()
                .join(
                    pl::JoinSide::Left,
                    Name::runtime("items").named("peer"),
                    pl::col(Name::runtime("items"), Name::runtime("id"))
                        .eq(pl::col(Name::runtime("peer"), Name::runtime("id"))),
                )
                .select((
                    pl::col(Name::runtime("items"), Name::runtime("id")),
                    pl::col(Name::runtime("peer"), Name::runtime("amount")).as_("peer_amount"),
                )),
        ),
        (
            "join_with",
            base.clone()
                .join_with(pl::JoinSide::Inner, right.clone().named("peer"), |b| {
                    pl::col(Name::runtime("items"), Name::runtime("id"))
                        .eq(pl::col(Name::runtime("peer"), Name::runtime("id")))
                        .and(
                            pl::col(Name::runtime("items"), Name::runtime("amount"))
                                .gt(b.bind(1i64)),
                        )
                }),
        ),
        ("append", left.clone().append(right.clone())),
        ("intersect", left.clone().intersect(right.clone())),
        ("remove", left.remove(right)),
        ("distinct", base.clone().select(category).distinct()),
        (
            "case",
            base.clone().select_with(|b| {
                [pl::case(
                    [(
                        pl::col(Name::runtime("items"), Name::runtime("id")).gt(b.bind(1i64)),
                        b.bind("yes"),
                    )],
                    b.bind("no"),
                )
                .as_("answer")]
            }),
        ),
        (
            "in_array",
            base.clone().filter_with(|b| {
                pl::col(Name::runtime("items"), Name::runtime("id"))
                    .in_array([b.bind(1i32), b.bind(2i32)])
            }),
        ),
        (
            "cast_unary",
            base.select((
                (-amount).cast(pl::CastType::BigInt).as_("negative"),
                (!key.is_null()).as_("present"),
            )),
        ),
        (
            "qualified",
            Pipeline::from_schema(Name::runtime("schema β"), Name::runtime("items β")).select(
                pl::col(Name::runtime("items β"), Name::runtime("id β"))
                    .as_runtime(Name::runtime("out β")),
            ),
        ),
        (
            "compares_as",
            Pipeline::from(Name::runtime("items")).filter_with(|b| {
                pl::col(Name::runtime("items"), Name::runtime("category"))
                    .compares_as(TypeName::new(Name::runtime("Mood")).schema(Name::runtime("app")))
                    .in_array([b.bind("glad"), b.bind("calm")])
            }),
        ),
    ])
}

// [spec:pgorm:req:bindings.pipeline+1/test]
#[test]
fn pipeline_sql_and_parameters_match_rust() -> Result<(), Box<dyn std::error::Error>> {
    Python::initialize();
    Python::attach(|py| -> Result<(), Box<dyn std::error::Error>> {
        let native = PyModule::new(py, "pgorm._native")?;
        pgorm_python::install(&native, Default::default())?;
        let source = CString::new(include_str!("pipeline_programs.py"))?;
        let examples =
            PyModule::from_code(py, &source, c"pipeline_programs.py", c"pipeline_programs")?;
        let queries = examples.call_method1("programs", (&native,))?;
        let rust = programs();
        assert_eq!(queries.len()?, rust.len());
        for (name, pipeline) in rust {
            let query = queries.get_item(name)?;
            let compiled: Compiled = query
                .call_method0("inspect")?
                .extract()
                .map_err(PyErr::from)?;
            let expected: (String, Values) = pipeline.into_sql()?;
            assert_eq!((compiled.sql, compiled.values), expected, "{name}");
        }
        Ok(())
    })
}
