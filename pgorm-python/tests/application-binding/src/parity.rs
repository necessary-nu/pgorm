use std::ffi::CString;

use pgorm::pgorm_query::{Expr, ExprTrait, Name, NullOrdering, Order, Query};
use pgorm::{
    ActiveModelBehavior, ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, Iterable,
    QueryFilter, QueryOrder, QuerySelect, QueryTrait, StaticName,
};
use pgorm_python::{entities::ActiveState, expressions::Compiled, values::PyValue};
use pyo3::{prelude::*, types::PyDict};

use crate::{
    account::{self, Column},
    temper,
};

fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    let module = PyModule::new(py, "pgorm._native")?;
    crate::_native(&module)?;
    Ok(module)
}

// [spec:pgorm:req:bindings.entity-reads+4/test]
#[test]
fn query_sql_and_values_match_rust() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let module = module(py)?;
        let entity = module.call_method1("entity", ("app.Account",))?;
        let id = entity.call_method1("col", ("id",))?;
        let mood = entity.call_method1("col", ("mood",))?;
        let descending = id.call_method0("expr")?.call_method0("desc")?;
        let comparisons = [
            ("eq", Column::Id.eq(2)),
            ("ne", Column::Id.ne(2)),
            ("gt", Column::Id.gt(2)),
            ("gte", Column::Id.gte(2)),
            ("lt", Column::Id.lt(2)),
            ("lte", Column::Id.lte(2)),
        ];
        for (method, predicate) in comparisons {
            let query = entity
                .call_method0("find")?
                .call_method1("where", (id.call_method1(method, (2,))?,))?
                .call_method1("where", (mood.call_method1("eq", ("busy",))?,))?
                .call_method1("order_by", (&descending,))?
                .call_method1("limit", (4,))?
                .call_method1("offset", (1,))?;
            let rust = account::Entity::find()
                .filter(predicate)
                .filter(Column::Mood.eq("busy"))
                .order_by(Column::Id, Order::Desc)
                .limit(4)
                .offset(1);
            for terminal in ["all", "one", "optional"] {
                let kwargs = PyDict::new(py);
                kwargs.set_item("terminal", terminal)?;
                let inspected = query.call_method("inspect", (), Some(&kwargs))?;
                let native = inspected.extract::<PyRef<'_, Compiled>>()?;
                let mut expected = rust.clone();
                if terminal != "all" {
                    QueryTrait::query(&mut expected).narrow_limit_to_one();
                }
                let expected = expected.build();
                assert_eq!(native.sql, expected.0, "{method}/{terminal}");
                assert_eq!(native.values, expected.1, "{method}/{terminal}");
            }
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("nulls", module.getattr("Nulls")?.getattr("First")?)?;
        let order = id
            .call_method0("expr")?
            .call_method("asc", (), Some(&kwargs))?;
        let native: Compiled = entity
            .call_method0("find")?
            .call_method1("order_by", (order,))?
            .call_method1("limit", (1,))?
            .call_method0("limit")?
            .call_method1("offset", (1,))?
            .call_method0("offset")?
            .call_method0("inspect")?
            .extract()?;
        let expected = account::Entity::find()
            .order_by_with_nulls(Column::Id, Order::Asc, NullOrdering::First)
            .build();
        assert_eq!((native.sql, native.values), expected);
        Ok(())
    })
}

// [spec:pgorm:req:bindings.entity-reads+4/test]    a column's is_in and is_not_in are its
// ColumnTrait membership: is_in / is_not_in over values of its declared kind, and in_subquery /
// not_in_subquery over a Select's rows, so a column whose domain is over an enum is a member as
// the enum, the subquery's projection cast beside it, and a list holding an expression is a member
// as written
#[test]
fn column_membership_matches_column_trait() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = PyDict::new(py);
        globals.set_item("p", module(py)?)?;
        let column = |name: &str| {
            Query::select()
                .expr(Expr::col(Name::runtime(name)))
                .from(Name::runtime("t"))
                .to_owned()
        };
        let cases = [
            (
                "p.entity('app.Account').col('id').is_in([1, 2])",
                Column::Id.is_in([1, 2]),
            ),
            (
                "p.entity('app.Account').col('id').is_in([])",
                Column::Id.is_in(Vec::<i32>::new()),
            ),
            (
                "p.entity('app.Account').col('mood').is_not_in(('calm',))",
                Column::Mood.is_not_in(["calm"]),
            ),
            (
                "p.entity('app.Account').col('id').is_not_in(p.Select(p.col('id')).from_(p.Table('t')))",
                Column::Id.not_in_subquery(column("id")),
            ),
            (
                "p.entity('app.Temper').col('mood').is_in(['calm', 'busy'])",
                temper::Column::Mood.is_in(["calm", "busy"]),
            ),
            (
                "p.entity('app.Temper').col('mood').is_in(p.Select(p.col('mood')).from_(p.Table('t')))",
                temper::Column::Mood.in_subquery(column("mood")),
            ),
            (
                "p.entity('app.Account').col('id').is_in([p.col('x')])",
                Expr::col(Column::Id.as_column_ref()).is_in([Expr::col(Name::runtime("x"))]),
            ),
        ];
        for (source, expected) in cases {
            let built = py.eval(&CString::new(source)?, Some(&globals), None)?;
            let built = built.call_method0("inspect")?;
            let built = built.extract::<PyRef<'_, Compiled>>()?;
            let (sql, values) = Query::select().expr(expected).build();
            assert_eq!(built.sql, sql, "{source}");
            assert_eq!(built.values, values, "{source}");
        }
        for refused in [
            "p.entity('app.Account').col('id').is_in('12')",
            "p.entity('app.Account').col('id').is_in(['one'])",
        ] {
            assert!(
                py.eval(&CString::new(refused)?, Some(&globals), None)
                    .is_err(),
                "{refused} is refused"
            );
        }
        Ok(())
    })
}

fn assert_active(native: &Bound<'_, PyAny>, rust: &account::ActiveModel) -> PyResult<()> {
    for column in Column::iter() {
        let native = native.call_method1("get", (column.as_str(),))?;
        let state = native.getattr("state")?.extract::<ActiveState>()?;
        let value = native.getattr("value")?;
        let expected = match rust
            .get(column)
            .expect("account carries every column of its entity")
        {
            ActiveValue::NotSet => {
                assert_eq!(state, ActiveState::NotSet);
                assert!(value.is_none());
                continue;
            }
            ActiveValue::Set(value) => {
                assert_eq!(state, ActiveState::Set);
                value
            }
            ActiveValue::Unchanged(value) => {
                assert_eq!(state, ActiveState::Unchanged);
                value
            }
        };
        assert_eq!(
            value.extract::<PyRef<'_, PyValue>>()?.rust_value(),
            &expected
        );
    }
    Ok(())
}

// [spec:pgorm:req:bindings.entity-writes+1/test]
#[test]
fn active_state_and_values_match_rust() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let module = module(py)?;
        let entity = module.call_method1("entity", ("app.Account",))?;
        let initial = entity.call_method0("active")?;
        let mut rust = account::ActiveModel::new();
        assert_active(&initial, &rust)?;
        let active = initial
            .call_method1("set", ("id", 41))?
            .call_method1("set", ("display name", "O'Brien"))?
            .call_method1("set", ("note", py.None()))?
            .call_method1("set", ("mood", "busy"))?;
        rust.set(Column::Id, 41.into()).unwrap();
        rust.set(Column::Name, "O'Brien".into()).unwrap();
        rust.set(Column::Note, None::<String>.into()).unwrap();
        rust.set(Column::Mood, "busy".into()).unwrap();
        assert_active(&active, &rust)?;
        assert_active(&initial, &account::ActiveModel::new())?;
        let active = active
            .call_method1("not_set", ("note",))?
            .call_method1("reset", ("id",))?;
        rust.not_set(Column::Note);
        rust.reset(Column::Id);
        assert_active(&active, &rust)
    })
}
