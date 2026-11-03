use super::*;
use crate::oracle::{assert_parses, assert_query_eq};
use pgorm_query::extension::{Extension, Type};
use std::sync::Arc;

fn sweep(statements: impl IntoIterator<Item = String>) {
    for sql in statements {
        assert_parses(&sql);
    }
}

fn base() -> SelectStatement {
    Query::select().column(Glyph::Id).from(Glyph::Table).take()
}

// [spec:pgorm:req:sql.render.oracle+4/test]    the select clause vocabulary
// [spec:pgorm:req:sql.render.select-order+7/test]
#[test]
fn sweep_select_clause_shapes() {
    sweep([
        base().to_string(),
        Query::select()
            .column(Asterisk)
            .from(Glyph::Table)
            .to_string(),
        Query::select()
            .distinct()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .to_string(),
        Query::select()
            .distinct_on([Glyph::Aspect])
            .column(Glyph::Id)
            .from(Glyph::Table)
            .to_string(),
        Query::select()
            .expr_as(Expr::col(Glyph::Id), Name::runtime("glyph id"))
            .from((Name::runtime("public"), Glyph::Table))
            .to_string(),
        base()
            .and_where(Expr::col(Glyph::Aspect).gt(1))
            .and_where(Expr::col(Glyph::Image).is_not_null())
            .add_group_by([Expr::col(Glyph::Id).into()])
            .and_having(Expr::col(Glyph::Aspect).lt(9))
            .to_string(),
        base()
            .expr(Func::grouping(Expr::col(Glyph::Aspect)).arg(Expr::col(Glyph::Image)))
            .group_by_col(Glyph::Id)
            .group_by_element(
                GroupingElement::sets(GroupingElement::rollup([
                    Expr::col(Glyph::Aspect),
                    Expr::col(Glyph::Image),
                ]))
                .add(GroupingElement::cube([Expr::col(Glyph::Aspect)]))
                .add(GroupingElement::empty()),
            )
            .to_string(),
        base()
            .order_by(Glyph::Id, Order::Desc)
            .order_by_with_nulls(Glyph::Aspect, Order::Asc, NullOrdering::First)
            .limit(10)
            .offset(5)
            .to_string(),
        base()
            .order_by_with_nulls(
                Glyph::Id,
                Order::Field(Values(vec![1.into(), 2.into()])),
                NullOrdering::Last,
            )
            .to_string(),
        Query::select()
            .column((Glyph::Table, Glyph::Id))
            .column((Name::runtime("public"), Glyph::Table, Glyph::Aspect))
            .from(Glyph::Table)
            .to_string(),
    ]);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    expression rendering, including the parenthesis
// elision of `sql.render.precedence`
// [spec:pgorm:req:sql.render.parens+5/test]
#[test]
fn sweep_expression_shapes() {
    let exprs: Vec<SimpleExpr> = vec![
        Expr::col(Glyph::Aspect).add(1).mul(2),
        Expr::col(Glyph::Aspect)
            .gt(1)
            .and(Expr::col(Glyph::Id).lt(9))
            .or(Expr::col(Glyph::Id).eq(3)),
        Expr::col(Glyph::Aspect).between(1, 9),
        Expr::col(Glyph::Aspect).not_between(1, 9),
        Expr::col(Glyph::Image).like("a%"),
        Expr::col(Glyph::Image).like(LikeExpr::new("a%").escape('\\')),
        Expr::col(Glyph::Id).is_in([1, 2, 3]),
        Expr::col(Glyph::Id).is_in::<i32, _>([]),
        Expr::col(Glyph::Id).is_not_in([1, 2]),
        Expr::col(Glyph::Id).eq_any([1, 2, 3]),
        Expr::col(Glyph::Id).eq_any(Vec::<i32>::new()),
        Expr::col(Glyph::Id).ne_all([1, 2]),
        Expr::col(Glyph::Id).ne_all(Vec::<i32>::new()),
        Expr::col(Glyph::Aspect).cast_as(Name::runtime("text")),
        Expr::raw("now()"),
        Expr::template("$1 + $2", [1, 2]).expect("template arity"),
        Expr::tuple([Expr::val(1).into(), Expr::val(2).into()]).into(),
        Expr::col(Glyph::Tokens).get_json_field("a"),
        Expr::col(Glyph::Tokens).cast_json_field("b"),
        Expr::col(Glyph::Image).concat(Expr::val("x")),
        Expr::col(Glyph::Tokens).get_json_path(["a", "b"]),
        Expr::col(Glyph::Tokens).cast_json_path(["a", "b"]),
        Expr::col(Glyph::Tokens).get_json_path(Vec::<String>::new()),
        Expr::col(Glyph::Tokens).has_json_key("a"),
        Expr::col(Glyph::Tokens).has_any_json_keys(["a", "b"]),
        Expr::col(Glyph::Tokens).has_all_json_keys(["a", "b"]),
        Expr::col(Glyph::Tokens).has_any_json_keys(Vec::<String>::new()),
        Expr::col(Glyph::Tokens)
            .has_json_key("a")
            .and(Expr::col(Glyph::Tokens).has_all_json_keys(["b"])),
        SimpleExpr::Unary(UnOper::Not, Box::new(Expr::col(Glyph::Id).eq(1))),
        Expr::exists(base()),
        Expr::col(Glyph::Id).eq(Expr::any(base())),
        Expr::array(base()),
        Expr::case(Expr::col(Glyph::Aspect).gt(1), Expr::val("big"))
            .finally(Expr::val("small"))
            .into(),
        Expr::case_of(Expr::col(Glyph::Aspect))
            .when(1, Expr::val("one"))
            .finally(Expr::val("other"))
            .into(),
        Expr::col(Glyph::Tokens).index(1).index(2).into(),
        Expr::col(Glyph::Tokens)
            .slice(1, 2)
            .slice_from(3)
            .gt(Expr::val(0)),
        Func::count(Expr::col(Glyph::Id)).into(),
        Func::coalesce([Expr::col(Glyph::Aspect).into(), Expr::val(0).into()]).into(),
        Expr::val("1").cast_as(Name::runtime("int4")),
    ];

    sweep(
        exprs
            .into_iter()
            .map(|expr| Query::select().expr(expr).from(Glyph::Table).to_string()),
    );
}

// [spec:pgorm:req:sql.render.oracle+4/test]    the join vocabulary
// [spec:pgorm:req:sql.render.joins+2/test]
#[test]
fn sweep_join_shapes() {
    let joins = [
        JoinType::Join,
        JoinType::InnerJoin,
        JoinType::LeftJoin,
        JoinType::RightJoin,
        JoinType::FullOuterJoin,
    ];

    let mut statements: Vec<String> = joins
        .into_iter()
        .map(|join| {
            Query::select()
                .column((Char::Table, Char::Id))
                .from(Char::Table)
                .join(
                    join,
                    Font::Table,
                    Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id)),
                )
                .to_string()
        })
        .collect();

    statements.push(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .cross_join(Font::Table)
            .to_string(),
    );
    statements.push(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .join_as(
                JoinType::LeftJoin,
                Font::Table,
                Name::runtime("f"),
                Expr::col((Char::Table, Char::FontId)).equals((Name::runtime("f"), Font::Id)),
            )
            .to_string(),
    );
    statements.push(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .join_lateral(
                JoinType::LeftJoin,
                base(),
                Name::runtime("sub"),
                Expr::val(1).eq(1),
            )
            .to_string(),
    );
    statements.push(
        Query::select()
            .column(Glyph::Id)
            .from_subquery(base(), Name::runtime("sub"))
            .to_string(),
    );
    statements.push(
        Query::select()
            .column(Name::runtime("column1"))
            .from_values([(1, "a"), (2, "b")], Name::runtime("v"))
            .to_string(),
    );
    statements.push(
        Query::select()
            .column(Asterisk)
            .from_function(
                Func::named(Name::runtime("generate_series")).arg(1),
                Name::runtime("g"),
            )
            .to_string(),
    );

    sweep(statements);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    set operations and row locking
// [spec:pgorm:sem:sql.render.locking+1/test]
#[test]
fn sweep_union_and_locking_shapes() {
    let unions = [
        UnionType::Distinct,
        UnionType::All,
        UnionType::Intersect,
        UnionType::IntersectAll,
        UnionType::Except,
        UnionType::ExceptAll,
    ];

    let mut statements: Vec<String> = unions
        .into_iter()
        .map(|union| base().union(union, base()).to_string())
        .collect();

    let locks = [
        LockType::Update,
        LockType::NoKeyUpdate,
        LockType::Share,
        LockType::KeyShare,
    ];
    statements.extend(locks.into_iter().map(|lock| base().lock(lock).to_string()));

    statements.push(
        base()
            .lock_with_tables(LockType::Update, [Glyph::Table])
            .to_string(),
    );
    statements.push(
        base()
            .lock_with_behavior(LockType::Update, LockBehavior::Nowait)
            .to_string(),
    );
    statements.push(
        base()
            .lock_with_tables_behavior(LockType::Share, [Glyph::Table], LockBehavior::SkipLocked)
            .to_string(),
    );

    sweep(statements);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    common table expressions
// [spec:pgorm:req:sql.render.cte+5/test]
#[test]
fn sweep_cte_shapes() {
    let named = |name: &str| {
        CommonTableExpression::new(Name::runtime(name), base())
            .column(Glyph::Id)
            .to_owned()
    };
    let cte = || named("cte");
    let outer = || {
        Query::select()
            .column(Glyph::Id)
            .from(Name::runtime("cte"))
            .take()
    };

    sweep([
        outer().with(WithClause::new(cte())).to_string(),
        outer()
            .with(WithClause::new(cte().materialized(true).to_owned()))
            .to_string(),
        outer()
            .with(WithClause::new(cte().materialized(false).to_owned()))
            .to_string(),
        outer()
            .with(WithClause::new(cte()).cte(named("other")).to_owned())
            .to_string(),
        outer().with(RecursiveWithClause::new(cte())).to_string(),
    ]);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    window functions over a real function call
// [spec:pgorm:req:sql.render.window+7/test]
#[test]
fn sweep_window_function_shapes() {
    let over = |window: WindowStatement| {
        Query::select()
            .from(Char::Table)
            .expr_window(Func::count(Expr::col(Char::Id)), window)
            .to_string()
    };

    sweep([
        over(WindowStatement::new()),
        over(WindowStatement::partition_by(Char::FontSize)),
        over(
            WindowStatement::partition_by(Char::FontSize)
                .order_by(Char::Id, Order::Asc)
                .take(),
        ),
        over(
            WindowStatement::partition_by(Char::FontSize)
                .frame(FrameType::Rows.unbounded_preceding())
                .take(),
        ),
        over(
            WindowStatement::partition_by(Char::FontSize)
                .frame(FrameType::Range.current_row())
                .take(),
        ),
        over(
            WindowStatement::partition_by(Char::FontSize)
                .frame(
                    FrameType::Rows
                        .unbounded_preceding()
                        .and_unbounded_following()
                        .exclude(FrameExclusion::Ties),
                )
                .take(),
        ),
        over(
            WindowStatement::partition_by(Char::FontSize)
                .order_by(Char::SizeW, Order::Asc)
                .frame(
                    FrameType::Range
                        .preceding(Expr::val("1 day").cast_as(Name::runtime("interval")))
                        .and_following(Expr::val(1).add(1)),
                )
                .take(),
        ),
        over(
            WindowStatement::partition_by(Char::FontSize)
                .order_by(Char::SizeW, Order::Asc)
                .frame(FrameType::Groups.following(1).and_unbounded_following())
                .take(),
        ),
        Query::select()
            .from(Char::Table)
            .expr_window_name_as(
                Func::count(Expr::col(Char::Id)),
                Name::runtime("w"),
                Name::runtime("n"),
            )
            .to_string(),
    ]);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    INSERT, including ON CONFLICT and RETURNING
// [spec:pgorm:req:sql.render.insert+2/test]
// [spec:pgorm:req:sql.render.on-conflict+2/test]
// [spec:pgorm:req:sql.render.returning+4/test]
#[test]
fn sweep_insert_shapes() {
    let insert = || {
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic([1.into(), "a".into()])
            .to_owned()
    };

    sweep([
        insert().to_string(),
        insert().values_panic([2.into(), "b".into()]).to_string(),
        Query::insert()
            .into_table(Glyph::Table)
            .or_default_values_many(3)
            .to_string(),
        insert().returning(Query::returning().all()).to_string(),
        insert()
            .returning(Query::returning().columns([Glyph::Id, Glyph::Image]))
            .to_string(),
        insert()
            .returning(Query::returning().exprs([Expr::col(Glyph::Id).add(1)]))
            .to_string(),
        insert().on_conflict(OnConflict::do_nothing()).to_string(),
        insert()
            .on_conflict(OnConflict::column(Glyph::Id).do_nothing())
            .to_string(),
        insert()
            .on_conflict(
                OnConflict::column(Glyph::Id)
                    .and_column(Glyph::Aspect)
                    .update_column(Glyph::Image),
            )
            .to_string(),
        insert()
            .on_conflict(
                OnConflict::column(Glyph::Id).value(Glyph::Aspect, Expr::col(Glyph::Aspect).add(1)),
            )
            .to_string(),
        insert()
            .on_conflict(
                OnConflict::expr(Func::lower(Expr::col(Glyph::Tokens)))
                    .and_where(Expr::col(Glyph::Aspect).gt(0))
                    .update_column(Glyph::Image)
                    .and_where(Expr::col(Glyph::Id).gt(0)),
            )
            .to_string(),
        insert()
            .on_conflict(OnConflict::constraint(Name::runtime("glyph_key")).do_nothing())
            .to_string(),
        insert()
            .on_conflict(
                OnConflict::constraint(Name::runtime("glyph_key"))
                    .update_column(Glyph::Image)
                    .and_where(Expr::col(Glyph::Id).gt(0)),
            )
            .to_string(),
    ]);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    UPDATE and DELETE
// [spec:pgorm:req:sql.render.update-delete+3/test]
#[test]
fn sweep_update_and_delete_shapes() {
    sweep([
        Query::update()
            .table(Glyph::Table)
            .value(Glyph::Aspect, Expr::col(Glyph::Aspect).add(1))
            .values([(Glyph::Image, "a".into())])
            .and_where(Expr::col(Glyph::Id).eq(1))
            .to_string(),
        Query::update()
            .table(Glyph::Table)
            .value(Glyph::Aspect, 1)
            .returning(Query::returning().all())
            .to_string(),
        Query::delete()
            .from_table(Glyph::Table)
            .and_where(Expr::col(Glyph::Id).is_in([1, 2]))
            .to_string(),
        Query::delete()
            .from_table(Glyph::Table)
            .returning(Query::returning().columns([Glyph::Id]))
            .to_string(),
    ]);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    MERGE
// [spec:pgorm:req:sql.render.merge+1/test]
#[test]
fn sweep_merge_shapes() {
    let on = || Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id));
    let merge = || Query::merge(Glyph::Table, Font::Table, on());
    let name = || Expr::col((Font::Table, Font::Name));
    sweep([
        merge().when_matched(MatchedAction::Delete).to_string(),
        merge().when_matched(MatchedAction::DoNothing).to_string(),
        merge()
            .when_matched(MergeUpdate::value(Glyph::Image, name()).and_value(Glyph::Aspect, 1))
            .to_string(),
        merge()
            .when_not_matched(NotMatchedAction::InsertDefaultValues)
            .to_string(),
        merge()
            .when_not_matched(NotMatchedAction::DoNothing)
            .to_string(),
        merge()
            .when_matched_and(name().is_null(), MatchedAction::Delete)
            .when_matched(MergeUpdate::value(Glyph::Image, name()))
            .when_not_matched_and(
                Expr::col((Font::Table, Font::Language)).eq("en"),
                MergeInsert::value(Glyph::Id, Expr::col((Font::Table, Font::Id)))
                    .overriding(Overriding::SystemValue),
            )
            .when_not_matched(
                MergeInsert::value(Glyph::Id, Expr::col((Font::Table, Font::Id)))
                    .overriding(Overriding::UserValue),
            )
            .to_string(),
        Query::merge(
            (Name::runtime("public"), Glyph::Table)
                .into_named_table()
                .alias(Name::runtime("g")),
            FromItem::SubQuery(
                Query::select()
                    .columns([Font::Id, Font::Name])
                    .from(Font::Table)
                    .take(),
                Name::runtime("s"),
            ),
            Expr::col((Name::runtime("g"), Glyph::Id)).equals((Name::runtime("s"), Font::Id)),
        )
        .when_matched(MatchedAction::Delete)
        .only()
        .with(WithClause::new(CommonTableExpression::new(
            Name::runtime("w"),
            base(),
        )))
        .to_string(),
        merge()
            .when_not_matched_by_source_and(name().is_null(), MatchedAction::DoNothing)
            .when_not_matched_by_source(MergeUpdate::value(Glyph::Aspect, 0))
            .returning_action()
            .to_string(),
        merge()
            .when_matched(MatchedAction::Delete)
            .returning(
                Query::returning()
                    .exprs([
                        Expr::col((Name::runtime("o"), Asterisk)),
                        Expr::col((ReturningRow::New, Asterisk)),
                    ])
                    .old_as(Name::runtime("o")),
            )
            .returning_action()
            .to_string(),
        Query::select()
            .column(Asterisk)
            .from(Name::runtime("m"))
            .with(WithClause::new(CommonTableExpression::new(
                Name::runtime("m"),
                merge()
                    .when_not_matched_by_source(MatchedAction::Delete)
                    .returning(Query::returning().all())
                    .to_owned(),
            )))
            .to_string(),
    ]);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    table DDL
// [spec:pgorm:req:sql.ddl.create-table+16/test]
// [spec:pgorm:req:sql.ddl.alter-table+14/test]
#[test]
fn sweep_table_ddl_shapes() {
    sweep([
        Table::create(Glyph::Table)
            .if_not_exists()
            .col(
                ColumnDef::new(Glyph::Id)
                    .integer()
                    .not_null()
                    .auto_increment(),
            )
            .col(ColumnDef::new(Glyph::Aspect).double().default(1.0))
            .col(ColumnDef::new(Glyph::Image).text())
            .col(
                ColumnDef::new(Glyph::Tokens)
                    .json_binary()
                    .check(Expr::col(Glyph::Aspect).gt(0)),
            )
            .primary_key(TableKey::new(Glyph::Id).name(Name::runtime("glyph_pk")))
            .unique(Glyph::Image)
            .unique(
                TableKey::new(Glyph::Aspect)
                    .col(Glyph::Image)
                    .include([Glyph::Tokens])
                    .nulls_not_distinct()
                    .deferrability(Deferrability::DeferrableInitiallyDeferred),
            )
            .foreign_key(
                ForeignKey::create(Glyph::Table, Glyph::Id, Font::Table, Font::Id)
                    .on_delete(ForeignKeyAction::Cascade)
                    .to_owned(),
            )
            .to_string(),
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Aspect)
                    .integer()
                    .generated(Expr::val(1), GeneratedKind::Stored),
            )
            .col(
                ColumnDef::new(Glyph::Tokens)
                    .integer()
                    .generated(Expr::col(Glyph::Id).add(1), GeneratedKind::Virtual),
            )
            .to_string(),
        Table::alter(Glyph::Table)
            .add_column(ColumnDef::new(Name::runtime("added")).integer().not_null())
            .to_string(),
        Table::alter(Glyph::Table)
            .set_expression(Glyph::Aspect, Expr::col(Glyph::Id).add(1))
            .drop_expression(Glyph::Tokens)
            .drop_expression_if_exists(Glyph::Image)
            .to_string(),
        Table::alter(Glyph::Table)
            .drop_default(Glyph::Image)
            .alter_column_type_using(
                Glyph::Image,
                ColumnType::named("image_kind"),
                Expr::col(Glyph::Image)
                    .cast_as(Name::runtime("text"))
                    .cast_as(Name::runtime("image_kind")),
            )
            .to_string(),
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Id)
                    .integer()
                    .not_null_named(Name::runtime("id_present")),
            )
            .col(
                ColumnDef::new(Glyph::Aspect)
                    .integer()
                    .not_null_no_inherit(),
            )
            .col(
                ColumnDef::new(Glyph::Image)
                    .text()
                    .default("x")
                    .not_null_no_inherit()
                    .not_null_named(Name::runtime("image_present")),
            )
            .to_string(),
        Table::alter(Glyph::Table)
            .add_column(
                ColumnDef::new(Name::runtime("added"))
                    .integer()
                    .not_null_named(Name::runtime("added_present"))
                    .default(0),
            )
            .add_not_null(NotNullConstraint::new(Glyph::Aspect))
            .add_not_null(
                NotNullConstraint::new(Glyph::Image)
                    .name(Name::runtime("image_present"))
                    .no_inherit()
                    .not_valid(),
            )
            .validate_constraint(Name::runtime("image_present"))
            .alter_constraint(Name::runtime("image_present"), ConstraintChange::Inherit)
            .alter_constraint(Name::runtime("added_present"), ConstraintChange::NoInherit)
            .to_string(),
        Table::alter(Glyph::Table)
            .modify_column(
                ColumnDef::new(Glyph::Aspect)
                    .big_integer()
                    .not_null_named(Name::runtime("aspect_present"))
                    .not_null_no_inherit(),
            )
            .to_string(),
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Aspect).integer().check(
                    Check::new(Expr::col(Glyph::Aspect).gt(0))
                        .name(Name::runtime("aspect_positive"))
                        .enforcement(Enforcement::NotEnforced),
                ),
            )
            .col(ColumnDef::new(Glyph::Id).integer())
            .check(Check::new(Expr::col(Glyph::Aspect).lt(100)).enforcement(Enforcement::Enforced))
            .foreign_key(
                ForeignKey::create(Glyph::Table, Glyph::Id, Font::Table, Font::Id)
                    .deferrability(Deferrability::DeferrableInitiallyDeferred)
                    .enforcement(Enforcement::NotEnforced)
                    .to_owned(),
            )
            .to_string(),
        Table::alter(Glyph::Table)
            .add_check(Expr::col(Glyph::Aspect).gt(0))
            .add_check(
                Check::new(Expr::col(Glyph::Aspect).lt(100))
                    .name(Name::runtime("aspect_small"))
                    .enforcement(Enforcement::NotEnforced),
            )
            .add_foreign_key(
                TableForeignKey::new(Glyph::Table, Glyph::Id, Font::Table, Font::Id)
                    .enforcement(Enforcement::Enforced)
                    .to_owned(),
            )
            .modify_column(
                ColumnDef::new(Glyph::Tokens).check(Expr::col(Glyph::Tokens).is_not_null()),
            )
            .alter_constraint(Name::runtime("glyph_font"), ConstraintChange::Enforced)
            .alter_constraint(Name::runtime("glyph_font"), ConstraintChange::NotEnforced)
            .to_string(),
        Table::alter(Glyph::Table)
            .modify_column(ColumnDef::new(Glyph::Aspect).big_integer())
            .to_string(),
        Table::alter(Glyph::Table)
            .modify_column(ColumnDef::new(Glyph::Aspect).null())
            .to_string(),
        Table::alter(Glyph::Table)
            .add_primary_key((Glyph::Id, Glyph::Aspect))
            .add_unique(
                TableKey::new(Glyph::Image)
                    .name(Name::runtime("glyph_image"))
                    .include([Glyph::Tokens])
                    .nulls_not_distinct()
                    .deferrability(Deferrability::DeferrableInitiallyImmediate),
            )
            .to_string(),
        Table::rename_column(Glyph::Table, Glyph::Aspect, Name::runtime("ratio")).to_string(),
        Table::alter(Glyph::Table)
            .drop_column(Glyph::Aspect)
            .to_string(),
        Table::alter(Char::Table)
            .add_foreign_key(
                TableForeignKey::new(Char::Table, Char::FontId, Font::Table, Font::Id)
                    .name(Name::runtime("fk"))
                    .enforcement(Enforcement::NotEnforced)
                    .to_owned()
                    .not_valid(),
            )
            .add_check(
                Check::new(Expr::col(Char::SizeW).gt(0))
                    .no_inherit()
                    .enforcement(Enforcement::NotEnforced)
                    .not_valid(),
            )
            .to_string(),
        Table::create(Char::Table)
            .col(
                ColumnDef::new(Char::SizeW)
                    .integer()
                    .check(Check::new(Expr::col(Char::SizeW).gt(0)).no_inherit()),
            )
            .check(Check::new(Expr::col(Char::SizeW).lt(9)).no_inherit())
            .to_string(),
        Table::alter(Char::Table)
            .drop_constraint(Name::runtime("fk"))
            .drop_constraint(
                ConstraintDrop::new(Name::runtime("pk"))
                    .if_exists()
                    .cascade(),
            )
            .drop_constraint(ConstraintDrop::new(Name::runtime("ck")).restrict())
            .to_string(),
        Table::rename_constraint(Char::Table, Name::runtime("fk"), Name::runtime("fk2"))
            .to_string(),
        Table::rename_constraint(
            (Name::runtime("public"), Char::Table),
            Name::runtime("fk"),
            Name::runtime("fk2"),
        )
        .to_string(),
        Table::rename(Glyph::Table, Name::runtime("glyph_old")).to_string(),
        Table::truncate(Glyph::Table).to_string(),
        Table::drop(Glyph::Table).if_exists().cascade().to_string(),
        Table::drop(Glyph::Table)
            .table(Font::Table)
            .restrict()
            .to_string(),
    ]);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    index, foreign-key, type, extension and comment DDL
// [spec:pgorm:req:sql.ddl.index-create+11/test]
#[test]
fn sweep_schema_object_ddl_shapes() {
    sweep([
        Index::create(Glyph::Table, Glyph::Aspect)
            .name(Name::runtime("idx"))
            .to_string(),
        Index::create((Name::runtime("public"), Glyph::Table), Glyph::Aspect)
            .if_not_exists()
            .unique()
            .nulls_not_distinct()
            .name(Name::runtime("idx"))
            .col(Glyph::Image)
            .to_string(),
        Index::create(Glyph::Table, Glyph::Tokens)
            .name(Name::runtime("idx"))
            .index_type(IndexType::Gin)
            .to_string(),
        Index::create(Glyph::Table, Glyph::Aspect)
            .name(Name::runtime("idx"))
            .index_type(IndexType::Hash)
            .to_string(),
        Index::drop(Name::runtime("idx")).to_string(),
        ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
            .name(Name::runtime("fk"))
            .on_delete(ForeignKeyAction::SetNull)
            .on_update(ForeignKeyAction::NoAction)
            .to_string(),
        Type::create(Name::runtime("tea"))
            .values(["breakfast", "earl grey"])
            .to_string(),
        Type::alter(Name::runtime("tea"))
            .add_value("oolong")
            .to_string(),
        Type::alter(Name::runtime("tea"))
            .add_value("oolong")
            .after("breakfast")
            .to_string(),
        Type::alter(Name::runtime("tea"))
            .rename_value("oolong", "wulong")
            .to_string(),
        Type::alter(Name::runtime("address"))
            .add_attribute_collated(Name::runtime("zip"), ColumnType::Text, Name::runtime("C"))
            .drop_attribute_if_exists(Name::runtime("no"))
            .alter_attribute(Name::runtime("street"), ColumnType::string(Some(200)))
            .cascade()
            .to_string(),
        Type::alter(Name::runtime("address"))
            .rename_attribute(Name::runtime("no"), Name::runtime("number"))
            .restrict()
            .to_string(),
        Type::drop(Name::runtime("tea"))
            .if_exists()
            .cascade()
            .to_string(),
        Extension::create(Name::runtime("ltree")).to_string(),
        Extension::create(Name::runtime("ltree"))
            .schema(Name::runtime("public"))
            .if_not_exists()
            .cascade()
            .to_string(),
        Extension::drop(Name::runtime("ltree"))
            .if_exists()
            .cascade()
            .to_string(),
        Comment::on_table(Glyph::Table, "one row per glyph").to_string(),
        Comment::on_column(
            (Name::runtime("public"), Glyph::Table),
            Glyph::Aspect,
            "it's fine",
        )
        .to_string(),
    ]);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    every `ColumnType` that has a PostgreSQL spelling
// [spec:pgorm:def:sql.render.ddl.types+8/test]
#[test]
fn sweep_column_type_vocabulary() {
    let types = [
        ColumnType::Char(None),
        ColumnType::Char(Some(4)),
        ColumnType::String(StringLen::None),
        ColumnType::String(StringLen::N(255)),
        ColumnType::Text,
        ColumnType::SmallInteger,
        ColumnType::Integer,
        ColumnType::BigInteger,
        ColumnType::Float,
        ColumnType::Double,
        ColumnType::Decimal(None),
        ColumnType::Decimal(Some((12, 2))),
        ColumnType::Timestamp,
        ColumnType::TimestampWithTimeZone,
        ColumnType::Time,
        ColumnType::Date,
        ColumnType::Interval(IntervalSpec::Any(None)),
        ColumnType::Interval(IntervalSpec::Any(Some(IntervalPrecision::P6))),
        ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth)),
        ColumnType::Interval(IntervalSpec::Fields(PgInterval::MinuteToSecond(Some(
            IntervalPrecision::P2,
        )))),
        ColumnType::Bytea,
        ColumnType::Bit(None),
        ColumnType::Bit(Some(8)),
        ColumnType::VarBit(8),
        ColumnType::Boolean,
        ColumnType::Money,
        ColumnType::Json,
        ColumnType::JsonBinary,
        ColumnType::Uuid,
        ColumnType::Array(Arc::new(ColumnType::Integer)),
        ColumnType::Vector(Some(3)),
        ColumnType::Cidr,
        ColumnType::Inet,
        ColumnType::MacAddr,
        ColumnType::LTree,
        ColumnType::Range(RangeType::Int4),
        ColumnType::Range(RangeType::Int8),
        ColumnType::Range(RangeType::Numeric),
        ColumnType::Range(RangeType::Date),
        ColumnType::Range(RangeType::Timestamp),
        ColumnType::Range(RangeType::TimestampTz),
        ColumnType::Multirange(RangeType::Int4),
        ColumnType::Multirange(RangeType::TimestampTz),
    ];

    sweep(types.into_iter().map(|column_type| {
        Table::create(Glyph::Table)
            .col(ColumnDef::new_with_type(Name::runtime("c"), column_type))
            .to_string()
    }));
}

// [spec:pgorm:req:sql.render.oracle+4/test]    the binary operator vocabulary, minus `Escape`, which
// is only grammatical inside LIKE and is pinned in `oracle_pins.rs`
// [spec:pgorm:def:sql.render.operators+8/test]
#[test]
fn sweep_binary_operator_vocabulary() {
    let opers = [
        BinOper::And,
        BinOper::Or,
        BinOper::Like,
        BinOper::NotLike,
        BinOper::ILike,
        BinOper::NotILike,
        BinOper::Is,
        BinOper::IsNot,
        BinOper::In,
        BinOper::NotIn,
        BinOper::Equal,
        BinOper::NotEqual,
        BinOper::SmallerThan,
        BinOper::GreaterThan,
        BinOper::SmallerThanOrEqual,
        BinOper::GreaterThanOrEqual,
        BinOper::Add,
        BinOper::Sub,
        BinOper::Mul,
        BinOper::Div,
        BinOper::Mod,
        BinOper::LShift,
        BinOper::RShift,
        BinOper::Matches,
        BinOper::Contains,
        BinOper::Contained,
        BinOper::Concatenate,
        BinOper::Overlap,
        BinOper::Similarity,
        BinOper::WordSimilarity,
        BinOper::StrictWordSimilarity,
        BinOper::SimilarityDistance,
        BinOper::WordSimilarityDistance,
        BinOper::StrictWordSimilarityDistance,
        BinOper::GetJsonField,
        BinOper::CastJsonField,
        BinOper::Regex,
        BinOper::RegexCaseInsensitive,
        BinOper::NotRegex,
        BinOper::NotRegexCaseInsensitive,
        BinOper::JsonPathExists,
        BinOper::NotExtendRightOf,
        BinOper::NotExtendLeftOf,
        BinOper::AdjacentTo,
        BinOper::EuclideanDistance,
        BinOper::NegativeInnerProduct,
        BinOper::CosineDistance,
        BinOper::Raw("~~"),
    ];

    sweep(opers.into_iter().map(|oper| {
        let right: SimpleExpr = match oper {
            BinOper::Is | BinOper::IsNot => SimpleExpr::Keyword(Keyword::Null),
            BinOper::In | BinOper::NotIn => {
                Expr::tuple([Expr::val(1).into(), Expr::val(2).into()]).into()
            }
            _ => Expr::val(1).into(),
        };
        Query::select()
            .expr(Expr::col(Glyph::Aspect).binary(oper, right))
            .from(Glyph::Table)
            .to_string()
    }));
}

// [spec:pgorm:req:sql.render.oracle+4/test]    the `build()` path: `$N` placeholders parse as
// PostgreSQL parameter references
// [spec:pgorm:req:sql.render.placeholders+1/test]
#[test]
fn sweep_placeholder_builds() {
    let (select, _) = base()
        .and_where(Expr::col(Glyph::Aspect).eq(1))
        .and_where(Expr::col(Glyph::Image).like("a%"))
        .limit(10)
        .offset(2)
        .build();
    let (insert, _) = Query::insert()
        .into_table(Glyph::Table)
        .columns([Glyph::Aspect, Glyph::Image])
        .values_panic([1.into(), "a".into()])
        .on_conflict(OnConflict::column(Glyph::Id).update_column(Glyph::Image))
        .returning(Query::returning().all())
        .build();
    let (update, _) = Query::update()
        .table(Glyph::Table)
        .value(Glyph::Aspect, 1)
        .and_where(Expr::col(Glyph::Id).eq(2))
        .build();
    let (delete, _) = Query::delete()
        .from_table(Glyph::Table)
        .and_where(Expr::col(Glyph::Id).eq(2))
        .build();
    let (cast, _) = Query::select()
        .expr(Expr::val(1).cast_as(Name::runtime("text")))
        .build();
    let (merge, _) = Query::merge(
        Glyph::Table,
        Font::Table,
        Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
    )
    .when_matched_and(
        Expr::col(Glyph::Aspect).gt(1),
        MergeUpdate::value(Glyph::Aspect, 2),
    )
    .when_not_matched(MergeInsert::value(Glyph::Id, 3))
    .build();

    sweep([select, insert, update, delete, cast, merge]);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    `assert_query_eq` is the paired helper: it holds a
// rendered statement to both its expected spelling and the grammar
#[test]
fn oracle_pairs_text_and_grammar_checks() {
    assert_query_eq(&base().to_string(), r#"SELECT "id" FROM "glyph""#);
    assert_query_eq(
        &Table::truncate(Glyph::Table).to_string(),
        r#"TRUNCATE TABLE "glyph""#,
    );
}

// [spec:pgorm:req:sql.render.oracle+4/test]    both arms of the shim's type dispatch reach the
// oracle, so a silent regression in method resolution cannot mute the retrofitted assertions
#[test]
#[should_panic(expected = "PostgreSQL rejected")]
fn oracle_shim_fires_on_a_string() {
    let rendered = Table::create(Glyph::Table)
        .col(
            ColumnDef::new(Glyph::Id)
                .integer()
                .raw_suffix("ANYTHING I WANT TO SAY"),
        )
        .to_string();
    crate::oracle::assert_eq!(rendered, rendered.clone());
}

// [spec:pgorm:req:sql.render.oracle+4/test]
#[test]
#[should_panic(expected = "PostgreSQL rejected")]
fn oracle_shim_fires_on_a_str_slice() {
    crate::oracle::assert_eq!("SELECT FROM WHERE", "SELECT FROM WHERE");
}

// [spec:pgorm:req:sql.render.oracle+4/test]    a render that opens with a parenthesised query is
// a statement, so a grouped set operation is held to the grammar through the shim
#[test]
#[should_panic(expected = "PostgreSQL rejected")]
fn oracle_shim_fires_on_a_parenthesised_query() {
    let grouped = "((SELECT 1 UNION (SELECT FROM WHERE)) INTERSECT (SELECT 2)";
    crate::oracle::assert_eq!(grouped, grouped);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    a grouped set operation's render passes the shim's
// grammar check, and a parenthesised fragment that is no query is not parsed
#[test]
fn oracle_shim_reads_a_grouped_set_operation() {
    let arm = |table: &str| {
        Query::select()
            .column(Glyph::Id)
            .from(Name::runtime(table))
            .take()
    };
    let grouped = arm("a")
        .union(UnionType::Distinct, arm("b"))
        .union(UnionType::Intersect, arm("c"))
        .to_string();
    assert!(crate::oracle::looks_like_statement(&grouped), "{grouped}");
    crate::oracle::assert_eq!(
        grouped,
        r#"(SELECT "id" FROM "a" UNION (SELECT "id" FROM "b")) INTERSECT (SELECT "id" FROM "c")"#
    );

    for fragment in [
        r#"("image" COLLATE "C")"#,
        "(1, 2)",
        "(CASE WHEN TRUE THEN 1 END)",
        r#"( "selected" )"#,
    ] {
        assert!(!crate::oracle::looks_like_statement(fragment), "{fragment}");
    }
}

// [spec:pgorm:req:sql.render.oracle+4/test]    every query keyword opens a statement whether or not
// parentheses come first, so a render that opens bare with VALUES or TABLE passes the shim's
// grammar check as one opening with SELECT or WITH does
#[test]
fn oracle_shim_reads_a_bare_query() {
    for query in [
        "SELECT 1",
        "VALUES (1, 'a'), (2, 'b')",
        r#"TABLE "glyph""#,
        r#"WITH "v" AS (VALUES (1)) TABLE "v""#,
    ] {
        assert!(crate::oracle::looks_like_statement(query), "{query}");
        crate::oracle::assert_eq!(query, query);
    }
}

// [spec:pgorm:req:sql.render.oracle+4/test]    a broken render that opens with VALUES fails the shim
#[test]
#[should_panic(expected = "PostgreSQL rejected")]
fn oracle_shim_fires_on_a_values_list() {
    let broken = "VALUES (1, 'a'), (2";
    crate::oracle::assert_eq!(broken, broken);
}

// [spec:pgorm:req:sql.render.oracle+4/test]    a broken render that opens with TABLE fails the shim
#[test]
#[should_panic(expected = "PostgreSQL rejected")]
fn oracle_shim_fires_on_a_table_query() {
    let broken = r#"TABLE "glyph" WHERE "id" = 1"#;
    crate::oracle::assert_eq!(broken, broken);
}
