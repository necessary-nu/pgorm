use pgorm::pgorm_query::{
    Asterisk, CommonTableExpression, Cycle, Expr, FromItem, Func, IntoNamedTable, MergeUpdate,
    Name, OnConflict, Overriding, Query, RecursiveWithClause, ReturningRow, SampleMethod, Search,
    SearchOrder, SelectStatement, TableSample, UnionType, WithClause,
};
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
};

use super::tests::{module, parity, refused};
use crate::errors::ConstructionError;

fn ids(table: &str) -> SelectStatement {
    let n = Name::runtime;
    Query::select().column(n("id")).from(n(table)).to_owned()
}

// [spec:pgorm:req:bindings.write-sources/test]
// [spec:pgorm:req:bindings.write-ctes/test]
#[test]
fn insert_rows_from_a_select_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let n = Name::runtime;
        let globals = module(py)?;
        let mut expected = Query::insert();
        expected
            .into_table((n("application"), n("account")))
            .columns([n("id"), n("name")])
            .select_from(
                Query::select()
                    .column(n("id"))
                    .column(n("name"))
                    .from(n("src"))
                    .to_owned(),
            )
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        expected
            .overriding(Overriding::SystemValue)
            .on_conflict(OnConflict::do_nothing())
            .with(WithClause::new(CommonTableExpression::new(
                n("src"),
                ids("staged"),
            )));
        parity(
            py,
            &globals,
            "p.Insert(p.Table('account', schema='application')).with_(p.With('src', p.Select(p.col('id')).from_(p.Table('staged')))).columns('id', 'name').select(p.Select(p.col('id'), p.col('name')).from_(p.Table('src'))).overriding(p.Overriding.UserValue).overriding(p.Overriding.SystemValue).on_conflict(p.Conflict.do_nothing())",
            expected.build(),
        )?;
        py.run(
            c"t = p.Table('t')\nq = p.Select(p.col('x')).from_(t)",
            Some(&globals),
            None,
        )?;
        refused::<ConstructionError>(
            py,
            &globals,
            &[
                "p.Insert(t).select(q)",
                "p.Insert(t).columns('a').values(1).select(q)",
                "p.Insert(t).columns('a', 'b').select(q)",
                "p.Insert(t).columns('a').select(q).values(1)",
                "p.Insert(t).columns('a').select(q).select(q)",
                "p.Insert(t).columns('a').select(q).default_values()",
                "p.Insert(t).columns('a').select(q).columns('b')",
                "p.Insert(t).columns('a').select(q.limit_with_ties(1))",
            ],
        )?;
        refused::<PyTypeError>(
            py,
            &globals,
            &[
                "p.Insert(t).columns('a').select(t)",
                "p.Insert(t).columns('a').select(q.as_('s'))",
                "p.Insert(t).overriding('system')",
                "p.Insert(t).overriding(p.MergeAction.delete())",
                "p.Insert(t).with_(q)",
            ],
        )
    })
}

// [spec:pgorm:req:bindings.write-sources/test]
// [spec:pgorm:req:bindings.write-ctes/test]
#[test]
fn update_from_and_delete_using_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let n = Name::runtime;
        let globals = module(py)?;
        py.run(
            c"s = p.Table('staged', alias='s')\ngone = p.Select(p.col('id')).from_(p.Table('gone')).as_('g')\nw = p.With('w', p.Select(p.col('id')).from_(p.Table('t')))",
            Some(&globals),
            None,
        )?;
        let w = || WithClause::new(CommonTableExpression::new(n("w"), ids("t")));
        parity(
            py,
            &globals,
            "p.Update(p.Table('account')).set('name', s.col('name')).from_(s).where(s.col('id') == p.col('id', table='account')).with_(w)",
            Query::update()
                .table(n("account"))
                .value(n("name"), Expr::col((n("s"), n("name"))))
                .from(n("staged").into_named_table().alias(n("s")))
                .and_where(Expr::col((n("s"), n("id"))).equals((n("account"), n("id"))))
                .with(w())
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Delete(p.Table('account')).using(gone).using(s).where(p.col('id', table='g') == p.col('id', table='account')).returning(p.ReturningRow.Old.star()).with_(w)",
            Query::delete()
                .from_table(n("account"))
                .using(FromItem::SubQuery(ids("gone"), n("g")))
                .using(n("staged").into_named_table().alias(n("s")))
                .and_where(Expr::col((n("g"), n("id"))).equals((n("account"), n("id"))))
                .returning(Query::returning().expr(Expr::col((ReturningRow::Old, Asterisk))))
                .with(w())
                .build(),
        )?;
        refused::<PyTypeError>(
            py,
            &globals,
            &[
                "p.Update(p.Table('a')).from_(p.Select())",
                "p.Update(p.Table('a')).from_('staged')",
                "p.Delete(p.Table('a')).using(p.col('x'))",
                "p.Delete(p.Table('a')).with_(p.Select())",
            ],
        )
    })
}

// [spec:pgorm:req:bindings.writes+2/test]
// [spec:pgorm:req:bindings.write-ctes/test]
// [spec:pgorm:req:bindings.cte-options/test]
#[test]
fn writes_and_options_in_with_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let n = Name::runtime;
        let globals = module(py)?;
        let moved = Query::delete()
            .from_table(n("account"))
            .and_where(Expr::col(n("id")).gt(3i64))
            .returning(Query::returning().columns([n("id")]))
            .to_owned();
        let kept = Query::update()
            .table(n("log"))
            .value(n("seen"), true)
            .returning(Query::returning().columns([n("id")]))
            .to_owned();
        let added = Query::insert()
            .into_table(n("log"))
            .columns([n("id")])
            .values_panic([Expr::value(1i64)])
            .returning_all()
            .to_owned();
        let mut clause = WithClause::new(
            CommonTableExpression::new(n("moved"), moved)
                .column(n("gone"))
                .materialized(true)
                .to_owned(),
        );
        clause
            .cte(
                CommonTableExpression::new(n("kept"), kept)
                    .materialized(false)
                    .to_owned(),
            )
            .cte(CommonTableExpression::new(n("added"), added));
        parity(
            py,
            &globals,
            "p.Select().from_(p.Table('moved')).with_(p.With('moved', p.Delete(p.Table('account')).where(p.col('id') > 3).returning(p.col('id')), columns=['gone'], materialized=True).cte('kept', p.Update(p.Table('log')).set('seen', True).all_rows().returning(p.col('id')), materialized=False).cte('added', p.Insert(p.Table('log')).columns('id').values(1).returning()))",
            Query::select()
                .column(Asterisk)
                .from(n("moved"))
                .with(clause)
                .build(),
        )?;
        py.run(
            c"t = p.Table('t')\nq = p.Select(p.col('x')).from_(t)",
            Some(&globals),
            None,
        )?;
        refused::<ConstructionError>(
            py,
            &globals,
            &[
                "p.With('w', p.Insert(t))",
                "p.With('w', p.Insert(t).columns('a'))",
                "p.With('w', p.Update(t).set('a', 1))",
                "p.With('w', p.Update(t).all_rows())",
                "p.With('w', q).cte('v', p.Delete(t))",
                "p.With('w', q, columns=[''])",
            ],
        )?;
        refused::<PyTypeError>(
            py,
            &globals,
            &[
                "p.With('w', t)",
                "p.With('w', 1)",
                "p.With('w', q, columns='x')",
                "p.With('w', q, columns=[1])",
                "p.With('w', q, materialized=1)",
                "p.With('w', q, materialized='yes')",
                "p.With('w', q, recursive=True)",
            ],
        )
    })
}

// [spec:pgorm:req:bindings.cte-options/test]
#[test]
fn recursive_with_matches_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let n = Name::runtime;
        let globals = module(py)?;
        let body = Query::select()
            .expr_as(1i32, n("n"))
            .union(
                UnionType::All,
                Query::select()
                    .expr(Expr::col(n("n")).add(1i64))
                    .from(n("r"))
                    .and_where(Expr::col(n("n")).lt(5i64))
                    .to_owned(),
            )
            .to_owned();
        let clause = RecursiveWithClause::new(
            CommonTableExpression::new(n("r"), body)
                .column(n("n"))
                .materialized(true)
                .to_owned(),
        )
        .search(Search::new(SearchOrder::DEPTH, Expr::col(n("n")), n("ord")))
        .cycle(Cycle::new(Expr::col(n("n")), n("looped"), n("path")))
        .to_owned();
        py.run(
            c"body = p.Select(p.bind(p.Value(1, 'i32')).as_('n')).union_all(p.Select(p.col('n') + 1).from_(p.Table('r')).where(p.col('n') < 5))\nr = p.With.recursive('r', body, columns=('n',), materialized=True, search=p.Search(p.SearchOrder.Depth, p.col('n'), 'ord'), cycle=p.Cycle(p.col('n'), 'looped', 'path'))\nm = p.merge(p.Table('a'), p.Table('r'), p.col('id', table='a') == p.col('n', table='r')).when_matched(p.MergeAction.delete())",
            Some(&globals),
            None,
        )?;
        parity(
            py,
            &globals,
            "p.Select(p.col('n')).from_(p.Table('r')).with_(r)",
            Query::select()
                .column(n("n"))
                .from(n("r"))
                .with(clause)
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Select().from_(p.Table('r')).with_(p.With.recursive('r', body, search=p.Search(p.SearchOrder.Breadth, p.col('n'), 'ord')))",
            Query::select()
                .column(Asterisk)
                .from(n("r"))
                .with(
                    RecursiveWithClause::new(CommonTableExpression::new(
                        n("r"),
                        Query::select()
                            .expr_as(1i32, n("n"))
                            .union(
                                UnionType::All,
                                Query::select()
                                    .expr(Expr::col(n("n")).add(1i64))
                                    .from(n("r"))
                                    .and_where(Expr::col(n("n")).lt(5i64))
                                    .to_owned(),
                            )
                            .to_owned(),
                    ))
                    .search(Search::new(
                        SearchOrder::BREADTH,
                        Expr::col(n("n")),
                        n("ord"),
                    ))
                    .to_owned(),
                )
                .build(),
        )?;
        refused::<ConstructionError>(
            py,
            &globals,
            &[
                "r.cte('x', body)",
                "m.with_(r)",
                "p.With.recursive('r', body.limit_with_ties(1))",
                "p.Search(p.SearchOrder.Depth, p.col('n'), '')",
                "p.Cycle(p.col('n'), 'looped', 'p' * 64)",
            ],
        )?;
        refused::<PyTypeError>(
            py,
            &globals,
            &[
                "p.With.recursive('r', p.Insert(p.Table('t')).default_values())",
                "p.With.recursive('r', body, search=p.Cycle(p.col('n'), 'a', 'b'))",
                "p.With.recursive('r', body, cycle=p.Search(p.SearchOrder.Depth, p.col('n'), 'o'))",
                "p.Search('depth', p.col('n'), 'ord')",
                "p.Search(p.SearchOrder.Depth, p.col('n'), 1)",
                "p.Cycle(p.col('n'), 'looped', None)",
            ],
        )
    })
}

// [spec:pgorm:req:bindings.writes+2/test]
// [spec:pgorm:req:bindings.conflict-arbiters/test]
#[test]
fn conflict_arbiters_match_rust_builders() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let n = Name::runtime;
        let globals = module(py)?;
        let insert = || {
            Query::insert()
                .into_table(n("account"))
                .columns([n("id"), n("name")])
                .values_panic([Expr::value(1i64), Expr::value("x")])
                .to_owned()
        };
        py.run(
            c"row = p.Insert(p.Table('account')).columns('id', 'name').values(1, 'x')",
            Some(&globals),
            None,
        )?;
        parity(
            py,
            &globals,
            "row.on_conflict(p.Conflict.on_constraint('account_pkey').set('name', p.bind('y')).update('note').where(p.col('name') != 'z'))",
            insert()
                .on_conflict(
                    OnConflict::constraint(n("account_pkey"))
                        .value(n("name"), "y")
                        .update_column(n("note"))
                        .cond_where(Expr::col(n("name")).ne("z")),
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "row.on_conflict(p.Conflict.on_constraint('account_pkey').do_nothing())",
            insert()
                .on_conflict(OnConflict::constraint(n("account_pkey")).do_nothing())
                .build(),
        )?;
        parity(
            py,
            &globals,
            "row.on_conflict(p.Conflict.on('id', p.call('lower', p.col('name'))).where(p.col('active') == True).do_nothing())",
            insert()
                .on_conflict(
                    OnConflict::column(n("id"))
                        .and_expr(Func::lower(Expr::col(n("name"))))
                        .cond_where(Expr::col(n("active")).eq(true))
                        .do_nothing(),
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "row.on_conflict(p.Conflict.on(p.call('lower', p.col('name')), 'tenant').update('name'))",
            insert()
                .on_conflict(
                    OnConflict::expr(Func::lower(Expr::col(n("name"))))
                        .and_column(n("tenant"))
                        .update_column(n("name")),
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "row.on_conflict(p.Conflict.on('id').update('name', 'note').update('tenant', 'kind'))",
            insert()
                .on_conflict(
                    OnConflict::column(n("id"))
                        .update_column(n("name"))
                        .update_column(n("note"))
                        .update_column(n("tenant"))
                        .update_column(n("kind")),
                )
                .build(),
        )?;
        refused::<ConstructionError>(
            py,
            &globals,
            &[
                "p.Conflict.on_constraint('account_pkey').where(p.col('a') == 1)",
                "p.Conflict.on_constraint('')",
                "p.Conflict.on()",
                "p.Conflict.on('')",
                "p.Conflict.on('id').update()",
                "p.Conflict.on('id').set('a', 1).update()",
            ],
        )?;
        refused::<PyTypeError>(
            py,
            &globals,
            &[
                "p.Conflict.on(1)",
                "p.Conflict.on(p.Table('t'))",
                "p.Conflict.on(p.col('a').asc())",
                "p.Conflict.on('id').update(1)",
                "p.ConflictTarget('id')",
                "p.ConflictUpdate()",
                "p.Conflict.on_constraint(1)",
                "row.on_conflict(p.Conflict.on_constraint('account_pkey'))",
            ],
        )
    })
}

// [spec:pgorm:req:bindings.merge-sources/test]
#[test]
fn merge_reads_any_from_item_as_its_source() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let n = Name::runtime;
        let globals = module(py)?;
        let on = || Expr::col((n("t"), n("id"))).equals((n("staged"), n("id")));
        let update = || MergeUpdate::value(n("name"), Expr::col((n("staged"), n("name"))));
        let target = || n("account").into_named_table().alias(n("t"));
        py.run(
            c"t = p.Table('account', alias='t')\nstaged = p.col('id', table='staged')\non = t.col('id') == staged\nupdate = p.MergeAction.update('name', p.col('name', table='staged'))",
            Some(&globals),
            None,
        )?;
        parity(
            py,
            &globals,
            "p.merge(t, p.Table('staged').tablesample(p.SampleMethod.Bernoulli, 50), on).when_matched(update)",
            Query::merge(
                target(),
                TableSample::new(n("staged"), SampleMethod::Bernoulli, 50),
                on(),
            )
            .when_matched(update())
            .build(),
        )?;
        parity(
            py,
            &globals,
            "p.merge(t, p.Select(p.col('id'), p.col('name')).from_(p.Table('src')).as_('staged'), on).when_matched(update)",
            Query::merge(
                target(),
                FromItem::SubQuery(
                    Query::select()
                        .column(n("id"))
                        .column(n("name"))
                        .from(n("src"))
                        .to_owned(),
                    n("staged"),
                ),
                on(),
            )
            .when_matched(update())
            .build(),
        )
    })
}
