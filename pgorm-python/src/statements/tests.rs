use std::ffi::CString;

use pgorm::pgorm_query::{
    ArrayType, Asterisk, BinOper, ColumnType, CommonTableExpression, Condition, Expr,
    ExtendedWindow, FrameExclusion, FrameType, FromItem, Func, GroupingElement, IntoNamedTable,
    JoinType, MatchedAction, MergeInsert, MergeUpdate, Name, NotMatchedAction, NullOrdering,
    OnConflict, Order, OverStatement, Overriding, Query, ReturningRow, SampleMethod, SimpleExpr,
    SubQueryStatement, TableFunction, TableSample, UnionType, Value, Values, WindowStatement,
    WithClause,
};
use pyo3::{prelude::*, types::PyDict};

use crate::expressions::Compiled;

pub(super) fn module(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let module = PyModule::new(py, "pgorm")?;
    crate::values::register(&module)?;
    crate::identifiers::register(&module)?;
    crate::expressions::register(&module)?;
    crate::json::register(&module)?;
    super::register(&module)?;
    let globals = PyDict::new(py);
    globals.set_item("p", module)?;
    Ok(globals)
}

pub(super) fn parity(
    py: Python<'_>,
    globals: &Bound<'_, PyDict>,
    source: &str,
    expected: (String, Values),
) -> PyResult<()> {
    let object = py.eval(&CString::new(source)?, Some(globals), None)?;
    let inspected = object.call_method0("inspect")?;
    let inspected = inspected.extract::<PyRef<'_, Compiled>>()?;
    assert_eq!(inspected.sql, expected.0);
    assert_eq!(inspected.values, expected.1);
    let executable = super::compile(&object)?;
    assert_eq!(executable.sql, inspected.sql);
    assert_eq!(executable.values, inspected.values);
    Ok(())
}

pub(super) fn refused<E: pyo3::PyTypeInfo>(
    py: Python<'_>,
    globals: &Bound<'_, PyDict>,
    sources: &[&str],
) -> PyResult<()> {
    for source in sources {
        let error = py
            .eval(&CString::new(*source)?, Some(globals), None)
            .expect_err(source);
        assert!(error.is_instance_of::<E>(py), "{source}: {error}");
    }
    Ok(())
}

// [spec:pgorm:req:bindings.select+4/test]
#[test]
fn select_structure_matches_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let a = (Name::runtime("application"), Name::runtime("account"))
            .into_named_table()
            .alias(Name::runtime("a"));
        let e = (Name::runtime("application"), Name::runtime("event"))
            .into_named_table()
            .alias(Name::runtime("e"));
        let count: SimpleExpr =
            Func::count(Expr::col((Name::runtime("e"), Name::runtime("id")))).into();
        let expected = Query::select()
            .column((Name::runtime("a"), Name::runtime("name")))
            .expr_as(count.clone(), Name::runtime("events"))
            .from(a)
            .join(
                JoinType::LeftJoin,
                e,
                Expr::col((Name::runtime("a"), Name::runtime("id")))
                    .equals((Name::runtime("e"), Name::runtime("account_id"))),
            )
            .cond_where(
                Condition::all()
                    .add(Expr::col((Name::runtime("a"), Name::runtime("active"))).eq(true))
                    .add(
                        Condition::any()
                            .add(Expr::col((Name::runtime("e"), Name::runtime("kind"))).eq("click"))
                            .add(Expr::col((Name::runtime("e"), Name::runtime("id"))).is_null()),
                    ),
            )
            .add_group_by([Expr::col((Name::runtime("a"), Name::runtime("name"))).into()])
            .cond_having(Expr::expr(count.clone()).gt(2i64))
            .order_by_expr_with_nulls(count, Order::Desc, NullOrdering::Last)
            .order_by((Name::runtime("a"), Name::runtime("name")), Order::Asc)
            .limit(5)
            .offset(1)
            .build();
        py.run(c"a = p.Table('account', schema='application', alias='a')\ne = p.Table('event', schema='application', alias='e')\nn = p.call('count', e.col('id'))", Some(&globals), None)?;
        parity(
            py,
            &globals,
            "p.Select(a.col('name'), n.as_('events')).from_(a).join(e, a.col('id') == e.col('account_id'), kind=p.Join.Left).where(p.Condition.all(a.col('active') == True, p.Condition.any(e.col('kind') == 'click', e.col('id').is_null()))).group_by(a.col('name')).having(n > 2).order_by(n.desc(nulls=p.Nulls.Last), a.col('name').asc()).limit(5).offset(1)",
            expected,
        )
    })
}

// [spec:pgorm:req:bindings.writes+2/test]
#[test]
fn inserts_and_defaults_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let mut expected = Query::insert();
        expected
            .into_table((Name::runtime("application"), Name::runtime("account")))
            .columns([Name::runtime("id"), Name::runtime("name")]);
        expected
            .values([Expr::value(1i64), Expr::value("Alice")])
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        expected
            .values([SimpleExpr::Constant(2i64.into()), Expr::value("O'Brien")])
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        expected
            .returning(Query::returning().columns([Name::runtime("id"), Name::runtime("name")]));
        parity(
            py,
            &globals,
            "p.Insert(p.Table('account', schema='application')).columns('id', 'name').values(1, 'Alice').values(p.literal(2), \"O'Brien\").returning(p.col('id'), p.col('name'))",
            expected.build(),
        )?;
        parity(
            py,
            &globals,
            "p.Insert(p.Table('account')).default_values().returning()",
            Query::insert()
                .into_table(Name::runtime("account"))
                .or_default_values()
                .returning_all()
                .build(),
        )
    })
}

// [spec:pgorm:req:bindings.writes+2/test]
#[test]
fn conflict_actions_keep_rust_typed_states() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let target = OnConflict::column(Name::runtime("id"))
            .and_where(Expr::col(Name::runtime("id")).gt(0i64));
        let action = target
            .update_column(Name::runtime("name"))
            .value(Name::runtime("visits"), SimpleExpr::Constant(1i64.into()))
            .and_where(Expr::col(Name::runtime("active")).eq(true));
        let mut expected = Query::insert();
        expected
            .into_table(
                Name::runtime("account")
                    .into_named_table()
                    .alias(Name::runtime("a")),
            )
            .columns([Name::runtime("id"), Name::runtime("name")]);
        expected
            .values([Expr::value(1i64), Expr::value("Alice")])
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        expected.on_conflict(action);
        parity(
            py,
            &globals,
            "p.Insert(p.Table('account', alias='a')).columns('id', 'name').values(1, 'Alice').on_conflict(p.Conflict.on('id').where(p.col('id') > 0).update('name').set('visits', p.literal(1)).where(p.col('active') == True))",
            expected.build(),
        )
    })
}

// [spec:pgorm:req:bindings.writes+2/test]
#[test]
fn update_and_delete_keep_builder_parameter_order() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let table = (Name::runtime("application"), Name::runtime("account"))
            .into_named_table()
            .alias(Name::runtime("a"));
        parity(
            py,
            &globals,
            "p.Update(p.Table('account', schema='application', alias='a')).set('name', \"new' name\").where(p.col('id', table='a') == 4).returning(p.col('id'))",
            Query::update()
                .table(table.clone())
                .value(Name::runtime("name"), "new' name")
                .and_where(Expr::col((Name::runtime("a"), Name::runtime("id"))).eq(4i64))
                .returning_col(Name::runtime("id"))
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Delete(p.Table('account', schema='application', alias='a')).where(p.col('id', table='a') == 4).returning()",
            Query::delete()
                .from_table(table)
                .and_where(Expr::col((Name::runtime("a"), Name::runtime("id"))).eq(4i64))
                .returning_all()
                .build(),
        )
    })
}

// [spec:pgorm:req:python.raw/test]
#[test]
fn raw_templates_use_the_rust_lexer() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let template = "SELECT $2::text, $1::int8, '$9', $$ $8 $$ /* $7 */";
        let values = Values(vec![1i64.into(), "O'Brien".into()]);
        globals.set_item("template", template)?;
        let query = py.eval(
            c"p.RawSQL(template, [1, \"O'Brien\"])",
            Some(&globals),
            None,
        )?;
        let compiled = super::compile(&query)?;
        assert_eq!(compiled.sql, template);
        assert_eq!(compiled.values, values);
        let rendered = query.call_method0("inline_sql")?.extract::<String>()?;
        let expected = pgorm::pgorm_query::inject_parameters(template, values.0)
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        assert_eq!(rendered, expected);
        Ok(())
    })
}

// [spec:pgorm:req:bindings.merge+2/test]
#[test]
fn merge_arms_match_the_rust_typestate() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let t = |column: &str| Expr::col((Name::runtime("t"), Name::runtime(column)));
        let s = |column: &str| Expr::col((Name::runtime("s"), Name::runtime(column)));
        let mut expected = Query::merge(
            (Name::runtime("application"), Name::runtime("account"))
                .into_named_table()
                .alias(Name::runtime("t")),
            Name::runtime("staged")
                .into_named_table()
                .alias(Name::runtime("s")),
            t("id").equals((Name::runtime("s"), Name::runtime("id"))),
        )
        .when_matched(MergeUpdate::value(Name::runtime("name"), s("name")))
        .to_owned();
        expected
            .when_matched_and(s("visits").gt(10i64), MatchedAction::Delete)
            .when_matched_and(s("visits").gt(1i64), MatchedAction::DoNothing)
            .when_not_matched_and(
                s("name").eq("O'Brien"),
                MergeInsert::value(Name::runtime("id"), s("id"))
                    .and_value(Name::runtime("name"), SimpleExpr::Constant("x".into()))
                    .overriding(Overriding::UserValue),
            )
            .when_not_matched(NotMatchedAction::InsertDefaultValues)
            .when_not_matched_by_source_and(t("visits").eq(0i64), MatchedAction::Delete)
            .when_not_matched_by_source(
                MergeUpdate::value(Name::runtime("visits"), 0i64)
                    .and_value(Name::runtime("name"), "gone"),
            )
            .returning_action()
            .returning(
                Query::returning()
                    .exprs::<SimpleExpr, _>([
                        Expr::col((ReturningRow::Old, Name::runtime("visits"))).into(),
                        Expr::col((ReturningRow::New, Asterisk)).into(),
                    ])
                    .old_as(Name::runtime("before")),
            )
            .only();
        py.run(
            c"t = p.Table('account', schema='application', alias='t')\ns = p.Table('staged', alias='s')",
            Some(&globals),
            None,
        )?;
        parity(
            py,
            &globals,
            "p.merge(t, s, t.col('id') == s.col('id'))\
             .when_matched(p.MergeAction.update('name', s.col('name')))\
             .when_matched(p.MergeAction.delete(), condition=s.col('visits') > 10)\
             .when_matched(p.MergeAction.do_nothing(), condition=s.col('visits') > 1)\
             .when_not_matched(p.MergeAction.insert('id', s.col('id')).set('name', p.literal('x'))\
                 .overriding(p.Overriding.UserValue), condition=s.col('name') == \"O'Brien\")\
             .when_not_matched(p.MergeAction.insert_defaults())\
             .when_not_matched_by_source(p.MergeAction.delete(), condition=t.col('visits') == 0)\
             .when_not_matched_by_source(p.MergeAction.update('visits', 0).set('name', 'gone'))\
             .returning_action()\
             .returning(p.ReturningRow.Old.col('visits'), p.ReturningRow.New.star(), old_as='before')\
             .only()",
            expected.build(),
        )
    })
}

// [spec:pgorm:req:bindings.merge+2/test]
// [spec:pgorm:req:bindings.with+1/test]
#[test]
fn merge_reads_and_feeds_common_table_expressions() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let staged = Query::select()
            .expr(Expr::col(Name::runtime("id")))
            .from(Name::runtime("source"))
            .to_owned();
        let merge = Query::merge(
            Name::runtime("account"),
            Name::runtime("staged"),
            Expr::col((Name::runtime("account"), Name::runtime("id")))
                .equals((Name::runtime("staged"), Name::runtime("id"))),
        )
        .when_matched(MatchedAction::Delete)
        .with(WithClause::new(CommonTableExpression::new(
            Name::runtime("staged"),
            staged,
        )))
        .returning(Query::returning().all())
        .to_owned();
        let expected = Query::select()
            .expr(Expr::col(Asterisk))
            .from(Name::runtime("m"))
            .with(WithClause::new(CommonTableExpression::new(
                Name::runtime("m"),
                merge,
            )))
            .build();
        py.run(
            c"a = p.Table('account')\nstaged = p.Table('staged')\nm = p.merge(a, staged, a.col('id') == staged.col('id')).when_matched(p.MergeAction.delete()).with_(p.With('staged', p.Select(p.col('id')).from_(p.Table('source')))).returning()",
            Some(&globals),
            None,
        )?;
        parity(
            py,
            &globals,
            "p.Select().from_(p.Table('m')).with_(p.With('m', m))",
            expected,
        )
    })
}

// [spec:pgorm:req:bindings.writes+2/test]
#[test]
fn returning_reads_and_renames_row_versions() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        parity(
            py,
            &globals,
            "p.Update(p.Table('account')).set('visits', 1).where(p.col('id') == 4)\
             .returning(p.ReturningRow.Old.col('visits'), p.col('visits', table='n').as_('after'), new_as='n')",
            Query::update()
                .table(Name::runtime("account"))
                .value(Name::runtime("visits"), 1i64)
                .and_where(Expr::col(Name::runtime("id")).eq(4i64))
                .returning(
                    Query::returning()
                        .exprs([
                            Expr::col((ReturningRow::Old, Name::runtime("visits"))).into(),
                            Expr::col((Name::runtime("n"), Name::runtime("visits")))
                                .binary(BinOper::As, Expr::col(Name::runtime("after"))),
                        ])
                        .new_as(Name::runtime("n")),
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Delete(p.Table('account')).all_rows().returning(old_as='o', new_as='n')",
            Query::delete()
                .from_table(Name::runtime("account"))
                .returning(
                    Query::returning()
                        .all()
                        .old_as(Name::runtime("o"))
                        .new_as(Name::runtime("n")),
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Insert(p.Table('account')).columns('id').values(1).returning(p.ReturningRow.Old.star())",
            Query::insert()
                .into_table(Name::runtime("account"))
                .columns([Name::runtime("id")])
                .values_panic([Expr::value(1i64)])
                .returning(Query::returning().expr(Expr::col((ReturningRow::Old, Asterisk))))
                .build(),
        )
    })
}

// [spec:pgorm:req:bindings.merge+2/test]
// [spec:pgorm:req:bindings.argument-errors+1/test]
#[test]
fn merge_actions_are_checked_against_their_row() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        py.run(
            c"a = p.Table('a')\nb = p.Table('b')\npending = p.merge(a, b, a.col('id') == b.col('id'))",
            Some(&globals),
            None,
        )?;
        refused::<crate::errors::ConstructionError>(
            py,
            &globals,
            &[
                "pending.when_matched(p.MergeAction.insert('id', 1))",
                "pending.when_matched(p.MergeAction.insert_defaults())",
                "pending.when_not_matched_by_source(p.MergeAction.insert('id', 1))",
                "pending.when_not_matched_by_source(p.MergeAction.insert_defaults())",
                "pending.when_not_matched(p.MergeAction.update('id', 1))",
                "pending.when_not_matched(p.MergeAction.delete())",
                "pending.when_matched(p.MergeAction.delete()).when_not_matched(p.MergeAction.delete())",
            ],
        )?;
        refused::<pyo3::exceptions::PyTypeError>(
            py,
            &globals,
            &[
                "pending.when_matched('delete')",
                "pending.when_not_matched(None)",
                "p.MergeAction()",
                "p.MergeUpdate('id', 1)",
                "p.MergeInsert('id', 1)",
                "p.With('w', a)",
            ],
        )?;
        let pending = py.eval(c"pending", Some(&globals), None)?;
        let error = super::compile(&pending).expect_err("a pending MERGE has no statement");
        assert!(error.to_string().contains("needs a WHEN arm"));
        assert!(!pending.hasattr("inspect")?);
        Ok(())
    })
}

// [spec:pgorm:req:bindings.windows/test]
#[test]
fn windows_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let n = Name::runtime;
        let r = |column: &'static str| Expr::col((n("r"), n(column)));
        py.run(
            cr#"r = p.Table('reading', alias='r')
F = p.FrameType
w = (p.Window().partition_by(r.col('kind'), r.col('grade') + 1)
     .order_by(r.col('at').desc(nulls=p.Nulls.Last), r.col('id').asc())
     .frame(F.Groups.preceding(1).and_following(2).exclude(p.FrameExclusion.Ties)))
"#,
            Some(&globals),
            None,
        )?;
        let window = || {
            WindowStatement::new()
                .partition_by((n("r"), n("kind")))
                .add_partition_by(r("grade").add(1i64))
                .order_by_expr_with_nulls(r("at").into(), Order::Desc, NullOrdering::Last)
                .order_by_expr(r("id").into(), Order::Asc)
                .frame(
                    FrameType::Groups
                        .preceding(1i64)
                        .and_following(2i64)
                        .exclude(FrameExclusion::Ties),
                )
                .take()
        };
        let reading = n("reading").into_named_table().alias(n("r"));
        parity(
            py,
            &globals,
            "p.Select(r.col('id'), p.call('sum', r.col('weight')).over(w), p.call('count', r.col('id')).over(w).as_('N'), p.window_function('lag', r.col('weight'), p.literal(1), 0).over('W').as_('prev'), p.json_arrayagg(r.col('weight'), returning='jsonb').over(p.Window().order_by(r.col('id').asc())), p.json_objectagg(r.col('kind'), r.col('weight')).over('W')).from_(r).window('W', p.Window().frame(F.Rows.current_row()))",
            Query::select()
                .expr(r("id"))
                .expr_window(Func::sum(r("weight")), window())
                .expr_window_as(Func::count(r("id")), window(), n("N"))
                .expr_window_name_as(
                    Func::named(n("lag")).args([
                        r("weight").into(),
                        SimpleExpr::Constant(1i64.into()),
                        Expr::value(0i64),
                    ]),
                    n("W"),
                    n("prev"),
                )
                .expr_window(
                    Func::json_arrayagg(r("weight")).returning(ColumnType::JsonBinary),
                    WindowStatement::new()
                        .order_by_expr(r("id").into(), Order::Asc)
                        .take(),
                )
                .expr_window_name(Func::json_objectagg(r("kind"), r("weight")), n("W"))
                .from(reading)
                .window(
                    n("W"),
                    WindowStatement::new()
                        .frame(FrameType::Rows.current_row())
                        .take(),
                )
                .build(),
        )
    })
}

// [spec:pgorm:req:bindings.windows/test]
#[test]
fn windows_refuse_what_the_grammar_refuses() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        for source in [
            "p.col('weight').over(p.Window())",
            "(p.col('weight') + 1).over(p.Window())",
            "p.call('sum', p.col('weight')).cast(p.TypeName('numeric')).over(p.Window())",
            "p.json_value(p.col('doc'), '$.a').over(p.Window())",
            "p.call('sum', p.col('weight')).over(p.Window()).as_('')",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<crate::errors::ConstructionError>(py),
                "{source}: {error}"
            );
        }
        for source in [
            "p.Window().frame(p.FrameType.Rows.following(1))",
            "p.Window().frame(p.col('id'))",
            "p.Window().partition_by(1)",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
                "{source}: {error}"
            );
        }
        for source in [
            "p.FrameType.Rows.current_row().and_preceding(1)",
            "p.FrameType.Rows.following(1).and_current_row()",
            "p.FrameType.Rows.following(1).exclude(p.FrameExclusion.Ties)",
            "p.FrameType.Rows.unbounded_following()",
            "p.FrameType.Rows.unbounded_preceding().and_unbounded_preceding()",
            "p.Window().exclude(p.FrameExclusion.Ties)",
            "p.window_function('rank').eq(1)",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<pyo3::exceptions::PyAttributeError>(py),
                "{source}: {error}"
            );
        }
        for source in [
            "p.window_function('row_number', 1)",
            "p.window_function('lag')",
            "p.window_function('lag', 1, 2, 3, 4)",
            "p.window_function('nth_value', 1)",
            "p.window_function('percentile_cont', 1)",
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

// [spec:pgorm:req:bindings.windows/test]    the WINDOW clause holds every window `window`
// declares, an `ExtendedWindow` among them, an item's inline window may build on one, and
// `check_windows` refuses at inspection and as the statement is read into another
#[test]
fn window_clause_matches_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let n = Name::runtime;
        let r = |column: &'static str| Expr::col((n("r"), n(column)));
        py.run(
            cr#"r = p.Table('reading', alias='r')
F = p.FrameType
def over(name):
    return p.Select(p.call('count', r.col('id')).over(name)).from_(r)
"#,
            Some(&globals),
            None,
        )?;
        parity(
            py,
            &globals,
            "p.Select(p.call('sum', r.col('weight')).over('running'), p.call('count', r.col('id')).over('by kind')).from_(r).window('by kind', p.Window().partition_by(r.col('kind'))).window('running', p.ExtendedWindow('by kind').order_by(r.col('id').asc()).frame(F.Rows.preceding(1).and_current_row()))",
            Query::select()
                .expr_window_name(Func::sum(r("weight")), n("running"))
                .expr_window_name(Func::count(r("id")), n("by kind"))
                .from(n("reading").into_named_table().alias(n("r")))
                .window(
                    n("by kind"),
                    WindowStatement::partition_by((n("r"), n("kind"))),
                )
                .window(
                    n("running"),
                    ExtendedWindow::new(n("by kind"))
                        .order_by_expr(r("id").into(), Order::Asc)
                        .frame(FrameType::Rows.preceding(1i64).and_current_row())
                        .to_owned(),
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Select(p.call('sum', r.col('weight')).over(p.ExtendedWindow('by kind').order_by(r.col('id').asc())).as_('running')).from_(r).window('by kind', p.Window().partition_by(r.col('kind')))",
            Query::select()
                .expr_window_as(
                    Func::sum(r("weight")),
                    ExtendedWindow::new(n("by kind"))
                        .order_by_expr(r("id").into(), Order::Asc)
                        .to_owned(),
                    n("running"),
                )
                .from(n("reading").into_named_table().alias(n("r")))
                .window(
                    n("by kind"),
                    WindowStatement::partition_by((n("r"), n("kind"))),
                )
                .build(),
        )?;
        for (source, refusal) in [
            (
                "over('w').window('w', p.Window()).window('w', p.Window())",
                "names two windows",
            ),
            (
                "over(p.ExtendedWindow('v')).window('w', p.Window())",
                "the window after OVER extends \"v\"",
            ),
            (
                "over(p.ExtendedWindow('v')).window('v', p.Window().frame(F.Rows.current_row()))",
                "which has a frame",
            ),
            (
                "over(p.ExtendedWindow('v').order_by(r.col('id').desc())).window('v', p.Window().order_by(r.col('id').asc()))",
                "already orders its rows",
            ),
            ("over('w').window('v', p.Window())", "no window \"w\""),
            (
                "over('w').window('w', p.ExtendedWindow('v')).window('v', p.Window())",
                "no earlier entry",
            ),
            (
                "over('w').window('v', p.Window().frame(F.Rows.current_row())).window('w', p.ExtendedWindow('v'))",
                "which has a frame",
            ),
            (
                "over('w').window('v', p.Window().order_by(r.col('id').asc())).window('w', p.ExtendedWindow('v').order_by(r.col('id').desc()))",
                "already orders its rows",
            ),
        ] {
            let select = py.eval(&CString::new(source)?, Some(&globals), None)?;
            let inspected = select.call_method0("inspect").expect_err(source);
            let compiled = super::compile(&select).expect_err(source);
            globals.set_item("refused", &select)?;
            let embedded = py
                .eval(c"over('x').union(refused)", Some(&globals), None)
                .expect_err(source);
            for error in [inspected, compiled, embedded] {
                assert!(
                    error.is_instance_of::<crate::errors::ConstructionError>(py)
                        && error.to_string().contains(refusal),
                    "{source}: {error}"
                );
            }
        }
        let error = py
            .eval(c"over('w').window('w', r.col('id'))", Some(&globals), None)
            .expect_err("a column is no window");
        assert!(
            error.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
            "{error}"
        );
        let error = py
            .eval(
                c"p.ExtendedWindow('w').partition_by(r.col('id'))",
                Some(&globals),
                None,
            )
            .expect_err("an extended window has no PARTITION BY");
        assert!(
            error.is_instance_of::<pyo3::exceptions::PyAttributeError>(py),
            "{error}"
        );
        let error = py
            .eval(c"bool(p.ExtendedWindow('w'))", Some(&globals), None)
            .expect_err("a window is no boolean");
        assert!(
            error.is_instance_of::<crate::errors::ConstructionError>(py),
            "{error}"
        );
        Ok(())
    })
}

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:sql.ast.select.tie-limit+2/test]    the tie limit shares `limit`'s slot, either
// replacing the other and `None` clearing both
#[test]
fn tie_limits_share_the_rust_limit_slot() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let n = Name::runtime;
        py.run(
            c"s = p.Select(p.col('image')).from_(p.Table('glyph')).order_by(p.col('aspect').desc())",
            Some(&globals),
            None,
        )?;
        let ordered = || {
            Query::select()
                .column(n("image"))
                .from(n("glyph"))
                .order_by_expr(Expr::col(n("aspect")).into(), Order::Desc)
                .take()
        };
        parity(
            py,
            &globals,
            "s.limit_with_ties(3).offset(1)",
            ordered().limit_with_ties(3).offset(1).build(),
        )?;
        parity(
            py,
            &globals,
            "s.limit_with_ties(3).limit(2)",
            ordered().limit(2).build(),
        )?;
        parity(
            py,
            &globals,
            "s.limit(2).limit_with_ties(3)",
            ordered().limit_with_ties(3).build(),
        )?;
        parity(
            py,
            &globals,
            "s.limit_with_ties(3).limit(None)",
            ordered().build(),
        )?;
        parity(
            py,
            &globals,
            "p.Select(p.col('image')).from_(p.Table('glyph')).limit_with_ties(3).order_by(p.col('aspect').desc())",
            ordered().limit_with_ties(3).build(),
        )
    })
}

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:sql.ast.select.tie-limit+2/test]    a tie limit with no ORDER BY is refused at
// inspection, at execution's compile and as a CTE body or subquery reads it, and its count is
// checked as `limit`'s is
#[test]
fn tie_limits_without_order_are_refused() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let unordered = py.eval(
            c"p.Select(p.col('image')).from_(p.Table('glyph')).limit_with_ties(3)",
            Some(&globals),
            None,
        )?;
        let error = unordered
            .call_method0("inspect")
            .expect_err("no ORDER BY to measure ties on");
        assert!(error.is_instance_of::<crate::errors::ConstructionError>(py));
        assert!(error.to_string().contains("needs an ORDER BY"), "{error}");
        let error = super::compile(&unordered).expect_err("execution renders through inspect");
        assert!(error.is_instance_of::<crate::errors::ConstructionError>(py));
        globals.set_item("unordered", unordered)?;
        py.run(
            c"ordered = unordered.order_by(p.col('aspect').desc())",
            Some(&globals),
            None,
        )?;
        for source in [
            "p.With('w', unordered)",
            "p.With('w', ordered).cte('v', unordered)",
            "p.json_array_query(unordered)",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<crate::errors::ConstructionError>(py),
                "{source}: {error}"
            );
        }
        parity(
            py,
            &globals,
            "p.Select().from_(p.Table('w')).with_(p.With('w', ordered))",
            Query::select()
                .column(Asterisk)
                .from(Name::runtime("w"))
                .with(WithClause::new(CommonTableExpression::new(
                    Name::runtime("w"),
                    Query::select()
                        .column(Name::runtime("image"))
                        .from(Name::runtime("glyph"))
                        .order_by_expr(Expr::col(Name::runtime("aspect")).into(), Order::Desc)
                        .limit_with_ties(3)
                        .take(),
                )))
                .build(),
        )?;
        for source in [
            "p.Select().limit_with_ties(-1)",
            "p.Select().limit_with_ties(2 ** 63)",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<crate::errors::ConstructionError>(py),
                "{source}: {error}"
            );
        }
        for source in [
            "p.Select().limit_with_ties(True)",
            "p.Select().limit_with_ties(1.5)",
            "p.Select().limit_with_ties(None)",
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

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:sql.ast.select.group-distinct/test]    grouping elements interleave with
// expressions in call order, and DISTINCT quantifies the whole list
#[test]
fn grouping_elements_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let n = Name::runtime;
        let a = || -> SimpleExpr { Expr::col(n("a")).into() };
        let b = || -> SimpleExpr { Expr::col(n("b")).into() };
        py.run(
            c"G = p.GroupingElement\na = p.col('a')\nb = p.col('b')",
            Some(&globals),
            None,
        )?;
        parity(
            py,
            &globals,
            "p.Select(a).from_(p.Table('t')).group_by(a, G.rollup(a, b), b, G.sets(G.set(a, b), G.empty(), b, G.cube(a, b), G.set())).group_by(G.rollup()).group_by_distinct()",
            Query::select()
                .expr(a())
                .from(n("t"))
                .add_group_by([a()])
                .group_by_element(GroupingElement::rollup([a(), b()]))
                .add_group_by([b()])
                .group_by_element(
                    GroupingElement::sets(GroupingElement::set([a(), b()]))
                        .add(GroupingElement::empty())
                        .add(GroupingElement::set([b()]))
                        .add(GroupingElement::cube([a(), b()]))
                        .add(GroupingElement::set(Vec::<SimpleExpr>::new())),
                )
                .group_by_element(GroupingElement::rollup(Vec::<SimpleExpr>::new()))
                .group_by_distinct()
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Select(a).from_(p.Table('t')).group_by_distinct()",
            Query::select()
                .expr(a())
                .from(n("t"))
                .group_by_distinct()
                .build(),
        )?;
        for source in [
            "p.Select().group_by(1)",
            "p.Select().group_by(a.asc())",
            "G.rollup(1)",
            "G.set(a, 'b')",
            "G.sets(1)",
            "G.sets(a, None)",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
                "{source}: {error}"
            );
        }
        let error = py
            .eval(c"G.sets()", Some(&globals), None)
            .expect_err("GROUPING SETS takes its first element");
        assert!(error.is_instance_of::<pyo3::exceptions::PyTypeError>(py));
        Ok(())
    })
}

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:sql.ast.select.table-function+2/test]    one call, ROWS FROM, ordinality and
// column names, the lone plain call as Rust's FunctionCall item, and anything but a call refused
#[test]
fn table_functions_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let n = Name::runtime;
        let one = || SimpleExpr::Constant(1i64.into());
        let three = || SimpleExpr::Constant(3i64.into());
        py.run(
            c"T = p.TableFunction\nseries = p.call('generate_series', p.literal(1), p.literal(3))\ntags = p.call('unnest', p.col('tags'))",
            Some(&globals),
            None,
        )?;
        parity(
            py,
            &globals,
            "p.Select().from_(T(series).with_ordinality().columns('v', p.Identifier('N')).alias('t'))",
            Query::select()
                .column(Asterisk)
                .from(
                    TableFunction::new(Func::generate_series(one(), three()))
                        .with_ordinality()
                        .columns([n("v"), n("N")])
                        .alias(n("t")),
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Select().from_(T(series).and_(tags).columns('x', 'y').alias('t'))",
            Query::select()
                .column(Asterisk)
                .from(
                    TableFunction::new(Func::generate_series(one(), three()))
                        .and(Func::unnest(Expr::col(n("tags"))))
                        .columns([n("x"), n("y")])
                        .alias(n("t")),
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Select().from_(p.Table('a')).cross_join(T.rows_from(tags).alias('u'))",
            Query::select()
                .column(Asterisk)
                .from(n("a"))
                .cross_join(
                    TableFunction::rows_from(Func::unnest(Expr::col(n("tags")))).alias(n("u")),
                )
                .build(),
        )?;
        let plain = py.eval(c"T(tags).alias('u')", Some(&globals), None)?;
        let plain = plain.extract::<PyRef<'_, super::PyFromItem>>()?;
        assert_eq!(
            plain.inner,
            FromItem::FunctionCall(Func::unnest(Expr::col(n("tags"))), n("u"))
        );
        let alias = py.eval(
            c"T(series).with_ordinality().alias('t').alias",
            Some(&globals),
            None,
        )?;
        assert_eq!(alias.extract::<String>()?, "t");
        for source in [
            "T(p.col('x'))",
            "T(p.col('x') + 1)",
            "T.rows_from(p.literal(1))",
            "T(series).and_(p.col('y'))",
            "T(series).columns('')",
            "T(series).alias('')",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<crate::errors::ConstructionError>(py),
                "{source}: {error}"
            );
        }
        for source in ["T(1)", "T(p.Table('t'))"] {
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

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:sql.ast.select.table-sample/test]    both methods, the percentage and seed
// bound as doubles, and numbers a double cannot hold exactly refused
#[test]
fn table_samples_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let n = Name::runtime;
        parity(
            py,
            &globals,
            "p.Select(p.col('image', table='g')).from_(p.Table('glyph', schema='public', alias='g').tablesample(p.SampleMethod.System, 12.5, repeatable=42))",
            Query::select()
                .column((n("g"), n("image")))
                .from(
                    TableSample::new(
                        (n("public"), n("glyph")).into_named_table().alias(n("g")),
                        SampleMethod::System,
                        12.5,
                    )
                    .repeatable(42),
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Select(p.col('image')).from_(p.Table('glyph').tablesample(p.SampleMethod.Bernoulli, 50)).join(p.Table('font'), p.col('id', table='glyph') == p.col('id', table='font'))",
            Query::select()
                .column(n("image"))
                .from(TableSample::new(n("glyph"), SampleMethod::Bernoulli, 50))
                .join(
                    JoinType::InnerJoin,
                    n("font"),
                    Expr::col((n("glyph"), n("id"))).equals((n("font"), n("id"))),
                )
                .build(),
        )?;
        let star = py.eval(
            c"p.Table('glyph', schema='public').tablesample(p.SampleMethod.Bernoulli, 1).col('id').inspect().sql",
            Some(&globals),
            None,
        )?;
        assert_eq!(star.extract::<String>()?, r#"SELECT "glyph"."id""#);
        for source in [
            "p.Table('t').tablesample(p.SampleMethod.Bernoulli, 2 ** 53 + 1)",
            "p.Table('t').tablesample(p.SampleMethod.Bernoulli, 10, repeatable=10 ** 400)",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<crate::errors::ConstructionError>(py),
                "{source}: {error}"
            );
        }
        for source in [
            "p.Table('t').tablesample(p.SampleMethod.Bernoulli, True)",
            "p.Table('t').tablesample(p.SampleMethod.Bernoulli, '10')",
            "p.Table('t').tablesample(p.SampleMethod.Bernoulli, None)",
            "p.Table('t').tablesample(p.SampleMethod.Bernoulli, 10, repeatable=False)",
        ] {
            let error = py
                .eval(&CString::new(source)?, Some(&globals), None)
                .expect_err(source);
            assert!(
                error.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
                "{source}: {error}"
            );
        }
        let error = py
            .eval(
                c"p.Table('t').tablesample('bernoulli', 10)",
                Some(&globals),
                None,
            )
            .expect_err("the method is a SampleMethod");
        assert!(error.is_instance_of::<pyo3::exceptions::PyTypeError>(py));
        Ok(())
    })
}

// [spec:pgorm:req:bindings.select+4/test]    each operator appends one parenthesised arm in
// call order, and the receiver's ORDER BY, limits and WITH clause cover the combined rows
#[test]
fn set_operations_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let n = Name::runtime;
        let ids = |name: &'static str| Query::select().column(n("id")).from(n(name)).take();
        py.run(
            c"ids = lambda name: p.Select(p.col('id')).from_(p.Table(name))",
            Some(&globals),
            None,
        )?;
        parity(
            py,
            &globals,
            "ids('t').union(ids('u')).union_all(ids('v')).intersect(ids('w')).intersect_all(ids('x')).except_(ids('y')).except_all(ids('z'))",
            ids("t")
                .union(UnionType::Distinct, ids("u"))
                .union(UnionType::All, ids("v"))
                .union(UnionType::Intersect, ids("w"))
                .union(UnionType::IntersectAll, ids("x"))
                .union(UnionType::Except, ids("y"))
                .union(UnionType::ExceptAll, ids("z"))
                .build(),
        )?;
        parity(
            py,
            &globals,
            "ids('t').where(p.col('id') > 1).order_by(p.col('id').desc()).union_all(ids('u').where(p.col('id') < 9).order_by(p.col('id').asc()).limit(2)).limit(3).offset(1)",
            ids("t")
                .and_where(Expr::col(n("id")).gt(1i64))
                .order_by(n("id"), Order::Desc)
                .union(
                    UnionType::All,
                    ids("u")
                        .and_where(Expr::col(n("id")).lt(9i64))
                        .order_by(n("id"), Order::Asc)
                        .limit(2)
                        .take(),
                )
                .limit(3)
                .offset(1)
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Select().from_(p.Table('w')).except_(p.Select().from_(p.Table('w')).where(p.col('id') == 2)).with_(p.With('w', ids('t'))).order_by(p.col('id').desc()).limit_with_ties(1)",
            Query::select()
                .column(Asterisk)
                .from(n("w"))
                .union(
                    UnionType::Except,
                    Query::select()
                        .column(Asterisk)
                        .from(n("w"))
                        .and_where(Expr::col(n("id")).eq(2i64))
                        .take(),
                )
                .with(WithClause::new(CommonTableExpression::new(
                    n("w"),
                    ids("t"),
                )))
                .order_by(n("id"), Order::Desc)
                .limit_with_ties(1)
                .build(),
        )
    })
}

// [spec:pgorm:req:bindings.select+4/test]    an arm is read into the statement, so its tie
// limit is checked as it is read; anything but a Select is a TypeError
#[test]
fn set_operation_arms_are_checked_when_read() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        py.run(
            c"s = p.Select(p.col('id')).from_(p.Table('t'))\nunordered = s.limit_with_ties(1)",
            Some(&globals),
            None,
        )?;
        for method in [
            "union",
            "union_all",
            "intersect",
            "intersect_all",
            "except_",
            "except_all",
        ] {
            let source = format!("s.{method}(unordered)");
            let error = py
                .eval(&CString::new(source.as_str())?, Some(&globals), None)
                .expect_err(&source);
            assert!(
                error.is_instance_of::<crate::errors::ConstructionError>(py),
                "{source}: {error}"
            );
            assert!(error.to_string().contains("needs an ORDER BY"), "{error}");
            for other in ["p.Table('t')", "p.RawSQL('SELECT 1')", "'SELECT 1'", "None"] {
                let source = format!("s.{method}({other})");
                let error = py
                    .eval(&CString::new(source.as_str())?, Some(&globals), None)
                    .expect_err(&source);
                assert!(
                    error.is_instance_of::<pyo3::exceptions::PyTypeError>(py),
                    "{source}: {error}"
                );
            }
        }
        let receiver = py.eval(c"unordered.union(s)", Some(&globals), None)?;
        let error = receiver
            .call_method0("inspect")
            .expect_err("the combined rows have no ORDER BY to measure ties on");
        assert!(error.is_instance_of::<crate::errors::ConstructionError>(py));
        Ok(())
    })
}

// [spec:pgorm:req:bindings.expressions+2/test]    array(select) is Rust's Expr::array over the
// subquery as it embeds, projected and compared as one value, and an ordered-with-ties subquery
// is refused as any embedded one is
#[test]
fn array_subquery_matches_rust_expr_array() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let events = || {
            Query::select()
                .expr(Expr::col((Name::runtime("e"), Name::runtime("id"))))
                .from(
                    Name::runtime("event")
                        .into_named_table()
                        .alias(Name::runtime("e")),
                )
                .cond_where(
                    Expr::col((Name::runtime("e"), Name::runtime("account_id")))
                        .eq(Expr::col((Name::runtime("a"), Name::runtime("id")))),
                )
                .to_owned()
        };
        let collected = Expr::array(events());
        let expected = Query::select()
            .expr(Expr::col((Name::runtime("a"), Name::runtime("id"))))
            .expr_as(collected.clone(), Name::runtime("events"))
            .from(
                Name::runtime("account")
                    .into_named_table()
                    .alias(Name::runtime("a")),
            )
            .cond_where(
                Expr::expr(collected.cast_as_type(
                    pgorm::pgorm_query::TypeName::new(Name::runtime("int8")).array(),
                ))
                .is_not_null(),
            )
            .build();
        parity(
            py,
            &globals,
            "(lambda a, e: p.Select(a.col('id'), p.array(p.Select(e.col('id')).from_(e).where(e.col('account_id') == a.col('id'))).as_('events')).from_(a).where(p.array(p.Select(e.col('id')).from_(e).where(e.col('account_id') == a.col('id'))).cast(p.TypeName('int8'), array=True).is_not_null()))(p.Table('account').as_('a'), p.Table('event').as_('e'))",
            expected,
        )?;
        let refused = py.eval(
            c"p.array(p.Select(p.col('id')).from_(p.Table('t')).limit_with_ties(1))",
            Some(&globals),
            None,
        );
        assert!(
            refused
                .is_err_and(|error| error.is_instance_of::<crate::errors::ConstructionError>(py))
        );
        Ok(())
    })
}

// [spec:pgorm:req:bindings.expressions+2/test]    exists, scalar, IN a Select and any_ / all_
// [spec:pgorm:req:bindings.quantifiers+2/test]
// are Rust's Expr::exists, the scalar SubQuery, in_subquery / not_in_subquery and Expr::any /
// Expr::all over a subquery or Func::any / Func::all over an array, a quantifier taken by each
// comparison and refused by arithmetic, IS DISTINCT FROM, a membership list and bind
#[test]
fn subquery_expressions_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let ids = |table: &str| {
            Query::select()
                .expr(Expr::col(Name::runtime("id")))
                .from(Name::runtime(table))
                .to_owned()
        };
        let id = || Expr::col(Name::runtime("id"));
        let scalar =
            SimpleExpr::SubQuery(None, Box::new(SubQueryStatement::SelectStatement(ids("v"))));
        let array = Value::Array(
            ArrayType::BigInt,
            Some(Box::new(vec![1i64.into(), 2i64.into()])),
        );
        let expected = Query::select()
            .column(Asterisk)
            .from(Name::runtime("t"))
            .cond_where(
                Condition::all()
                    .add(Expr::exists(ids("u")))
                    .add(id().eq(scalar))
                    .add(id().in_subquery(ids("w")))
                    .add(id().not_in_subquery(ids("x")))
                    .add(id().binary(BinOper::Equal, Func::any(Expr::val(array))))
                    .add(id().binary(BinOper::NotEqual, Func::all(Expr::array(ids("y")))))
                    .add(id().binary(BinOper::GreaterThan, Expr::all(ids("z"))))
                    .add(id().binary(BinOper::SmallerThanOrEqual, Expr::any(ids("u"))))
                    .add(Expr::col(Name::runtime("a")).binary(
                        BinOper::GreaterThanOrEqual,
                        Func::any(Expr::col(Name::runtime("b"))),
                    )),
            )
            .build();
        parity(
            py,
            &globals,
            "(lambda ids, i: p.Select().from_(p.Table('t')).where(p.Condition.all(p.exists(ids('u')), i == p.scalar(ids('v')), i.is_in(ids('w')), i.is_not_in(ids('x')), i == p.any_([1, 2]), i.ne(p.all_(p.array(ids('y')))), i > p.all_(ids('z')), i.lte(p.any_(ids('u'))), p.col('a') >= p.any_(p.col('b')))))(lambda name: p.Select(p.col('id')).from_(p.Table(name)), p.col('id'))",
            expected,
        )?;
        for refused in [
            "p.col('id') + p.any_(p.col('b'))",
            "p.col('id').is_distinct_from(p.all_(p.col('b')))",
            "p.col('id').is_in([p.any_(p.col('b'))])",
            "p.bind(p.any_(p.col('b')))",
            "p.exists(p.Select(p.col('id')).from_(p.Table('t')).limit_with_ties(1))",
            "p.any_(p.Select(p.col('id')).from_(p.Table('t')).limit_with_ties(1))",
            "bool(p.any_(p.col('b')))",
        ] {
            let outcome = py.eval(&CString::new(refused)?, Some(&globals), None);
            assert!(
                outcome.is_err_and(
                    |error| error.is_instance_of::<crate::errors::ConstructionError>(py)
                ),
                "{refused} is refused"
            );
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.quantifiers+2/test]    any_ and all_ are the right operand of LIKE's
// and ILIKE's family, the regular-expression matches, containment, the range positions, @@ and
// @?, each Rust's operator over Func::any / Func::all or Expr::any / Expr::all, and every other
// operator and operand position refuses one
#[test]
fn quantifiers_follow_each_operator_postgres_reads() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let globals = module(py)?;
        let a = || Expr::col(Name::runtime("a"));
        let rows = Query::select()
            .expr(Expr::col(Name::runtime("p")))
            .from(Name::runtime("u"))
            .to_owned();
        let filtered = |predicate: SimpleExpr| {
            Query::select()
                .column(Asterisk)
                .from(Name::runtime("t"))
                .cond_where(predicate)
                .build()
        };
        for (method, operator) in [
            ("like", BinOper::Like),
            ("not_like", BinOper::NotLike),
            ("ilike", BinOper::ILike),
            ("not_ilike", BinOper::NotILike),
            ("regex", BinOper::Regex),
            ("not_regex", BinOper::NotRegex),
            ("iregex", BinOper::RegexCaseInsensitive),
            ("not_iregex", BinOper::NotRegexCaseInsensitive),
            ("contains", BinOper::Contains),
            ("contained_by", BinOper::Contained),
            ("overlaps", BinOper::Overlap),
            ("strictly_left_of", BinOper::LShift),
            ("strictly_right_of", BinOper::RShift),
            ("not_extend_right_of", BinOper::NotExtendRightOf),
            ("not_extend_left_of", BinOper::NotExtendLeftOf),
            ("adjacent_to", BinOper::AdjacentTo),
            ("matches", BinOper::Matches),
            ("path_exists", BinOper::JsonPathExists),
        ] {
            let quantified = Func::any(Expr::col(Name::runtime("b")));
            parity(
                py,
                &globals,
                &format!(
                    "p.Select().from_(p.Table('t')).where(p.col('a').{method}(p.any_(p.col('b'))))"
                ),
                filtered(a().binary(operator, quantified)),
            )?;
            parity(
                py,
                &globals,
                &format!(
                    "p.Select().from_(p.Table('t')).where(p.col('a').{method}(p.all_(p.Select(p.col('p')).from_(p.Table('u')))))"
                ),
                filtered(a().binary(operator, Expr::all(rows.clone()))),
            )?;
        }
        let patterns = Value::Array(
            ArrayType::String,
            Some(Box::new(vec!["a%".into(), "b%".into()])),
        );
        parity(
            py,
            &globals,
            "p.Select().from_(p.Table('t')).where(p.col('a').like(p.any_(['a%', 'b%'])))",
            filtered(a().binary(BinOper::Like, Func::any(Expr::val(patterns)))),
        )?;
        for (source, class) in [
            (
                "p.col('a').similar_to(p.any_(p.col('b')))",
                "ConstructionError",
            ),
            (
                "p.col('a').not_similar_to(p.all_(p.col('b')))",
                "ConstructionError",
            ),
            (
                "p.col('a').is_not_distinct_from(p.any_(p.col('b')))",
                "ConstructionError",
            ),
            (
                "p.col('a').between(p.any_(p.col('b')), 1)",
                "ConstructionError",
            ),
            ("p.col('a').concat(p.any_(p.col('b')))", "ConstructionError"),
            ("p.col('a') % p.all_(p.col('b'))", "ConstructionError"),
            (
                "p.col('a').starts_with(p.any_(p.col('b')))",
                "ConstructionError",
            ),
            ("p.col('a').at(p.any_(p.col('b')))", "ConstructionError"),
            ("p.col('a').like('a%')", "TypeError"),
            ("p.col('a').path_exists(1)", "TypeError"),
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
