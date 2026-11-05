use pgorm::pgorm_query::{
    Asterisk, Expr, ExprTrait, FromItem, IntoNamedTable, JoinType, LockBehavior, LockType,
    MergeUpdate, Name, Order, Query, SampleMethod, SelectStatement, TableSample,
};
use pyo3::{exceptions::PyTypeError, prelude::*};

use super::tests::{module, parity, refused};
use crate::errors::ConstructionError;

fn latest_order() -> SelectStatement {
    let n = Name::runtime;
    Query::select()
        .column((n("o"), n("total")))
        .column((n("o"), n("user_id")))
        .from(n("orders").into_named_table().alias(n("o")))
        .and_where(Expr::col((n("o"), n("user_id"))).equals((n("u"), n("id"))))
        .order_by((n("o"), n("total")), Order::Desc)
        .limit(1)
        .to_owned()
}

// [spec:pgorm:req:bindings.from-subqueries/test]
#[test]
fn subqueries_are_from_items_and_join_lateral() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let n = Name::runtime;
        let globals = module(py)?;
        py.run(
            c"u = p.Table('users', alias='u')\no = p.Table('orders', alias='o')\nlatest = p.Select(o.col('total'), o.col('user_id')).from_(o).where(o.col('user_id') == u.col('id')).order_by(o.col('total').desc()).limit(1)\nl = latest.as_('l')",
            Some(&globals),
            None,
        )?;
        let users = n("users").into_named_table().alias(n("u"));
        parity(
            py,
            &globals,
            "p.Select().from_(p.Select(u.col('id')).from_(u).as_('s'))",
            Query::select()
                .column(Asterisk)
                .from(FromItem::SubQuery(
                    Query::select()
                        .column((n("u"), n("id")))
                        .from(users.clone())
                        .to_owned(),
                    n("s"),
                ))
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Select(u.col('name'), l.col('total')).from_(u).join(l, l.col('user_id') == u.col('id'), kind=p.Join.Left, lateral=True)",
            Query::select()
                .column((n("u"), n("name")))
                .column((n("l"), n("total")))
                .from(users.clone())
                .join_lateral(
                    JoinType::LeftJoin,
                    latest_order(),
                    n("l"),
                    Expr::col((n("l"), n("user_id"))).equals((n("u"), n("id"))),
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.Select(l.star()).from_(u).join(l, l.col('user_id') == u.col('id'))",
            Query::select()
                .expr(Expr::col((n("l"), Asterisk)))
                .from(users.clone())
                .join(
                    JoinType::InnerJoin,
                    FromItem::SubQuery(latest_order(), n("l")),
                    Expr::col((n("l"), n("user_id"))).equals((n("u"), n("id"))),
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "p.merge(u, l, l.col('user_id') == u.col('id')).when_matched(p.MergeAction.update('name', l.col('total')))",
            Query::merge(
                users,
                FromItem::SubQuery(latest_order(), n("l")),
                Expr::col((n("l"), n("user_id"))).equals((n("u"), n("id"))),
            )
            .when_matched(MergeUpdate::value(
                n("name"),
                Expr::col((n("l"), n("total"))),
            ))
            .build(),
        )?;
        refused::<ConstructionError>(
            py,
            &globals,
            &[
                "p.Select().from_(u).join(o, o.col('user_id') == u.col('id'), lateral=True)",
                "p.Select().from_(u).join(p.Table('t').tablesample(p.SampleMethod.System, 5), p.literal(True), lateral=True)",
                "p.Select(o.col('id')).from_(o).limit_with_ties(1).as_('s')",
                "p.Select().as_('')",
            ],
        )?;
        refused::<PyTypeError>(
            py,
            &globals,
            &[
                "p.Select().from_(u).join(l, p.literal(True), lateral=1)",
                "p.Select().from_(u).join(l, p.literal(True), lateral='yes')",
                "p.Select().as_(1)",
                "p.Select().from_(latest)",
                "p.merge(u, latest, p.literal(True))",
                "p.merge(l, u, p.literal(True))",
            ],
        )
    })
}

// [spec:pgorm:req:bindings.row-locks/test]
#[test]
fn row_locks_match_the_rust_lock_builder() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let n = Name::runtime;
        let globals = module(py)?;
        py.run(
            c"a = p.Table('account', schema='application')\ne = p.Table('event', alias='e')\nq = p.Select(e.col('id')).from_(a).join(e, e.col('account_id') == a.col('id'))\nS = p.LockStrength\nW = p.LockWait",
            Some(&globals),
            None,
        )?;
        let base = || {
            Query::select()
                .column((n("e"), n("id")))
                .from((n("application"), n("account")))
                .join(
                    JoinType::InnerJoin,
                    n("event").into_named_table().alias(n("e")),
                    Expr::col((n("e"), n("account_id"))).equals((
                        n("application"),
                        n("account"),
                        n("id"),
                    )),
                )
                .to_owned()
        };
        parity(
            py,
            &globals,
            "q.lock(S.Update)",
            base().lock(LockType::Update).build(),
        )?;
        parity(
            py,
            &globals,
            "q.lock(S.KeyShare, wait=W.Nowait)",
            base()
                .lock_with_behavior(LockType::KeyShare, LockBehavior::Nowait)
                .build(),
        )?;
        parity(
            py,
            &globals,
            "q.lock(S.NoKeyUpdate, of=[a, e], wait=W.SkipLocked)",
            base()
                .lock_with_tables_behavior(
                    LockType::NoKeyUpdate,
                    [n("account"), n("e")],
                    LockBehavior::SkipLocked,
                )
                .build(),
        )?;
        parity(
            py,
            &globals,
            "q.lock(S.Update).lock(S.Share, of=(e,))",
            base().lock_with_tables(LockType::Share, [n("e")]).build(),
        )?;
        let sampled = TableSample::new(n("account"), SampleMethod::Bernoulli, 10);
        parity(
            py,
            &globals,
            "p.Select().from_(a.as_('x')).from_(p.Table('account').tablesample(p.SampleMethod.Bernoulli, 10)).lock(S.Share, of=[p.Select().as_('x'), p.Table('account').tablesample(p.SampleMethod.Bernoulli, 10)])",
            Query::select()
                .column(Asterisk)
                .from(
                    (n("application"), n("account"))
                        .into_named_table()
                        .alias(n("x")),
                )
                .from(sampled)
                .lock_with_tables(LockType::Share, [n("x"), n("account")])
                .build(),
        )?;
        refused::<ConstructionError>(
            py,
            &globals,
            &[
                "q.order_by(e.col('id').asc()).limit_with_ties(1).lock(S.Update, wait=W.SkipLocked).inspect()",
            ],
        )?;
        refused::<PyTypeError>(
            py,
            &globals,
            &[
                "q.lock('update')",
                "q.lock(W.Nowait)",
                "q.lock(S.Update, wait='nowait')",
                "q.lock(S.Update, wait=S.Share)",
                "q.lock(S.Update, of=e)",
                "q.lock(S.Update, of='e')",
                "q.lock(S.Update, of=[e.col('id')])",
                "q.lock(S.Update, skip=True)",
            ],
        )
    })
}
