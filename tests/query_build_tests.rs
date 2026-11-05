#![allow(unused_imports, dead_code)]

//! The SQL-building contract of `src/query/`.
//!
//! Every assertion in this file renders a statement through `QueryTrait`, so
//! none of it needs a database: the claims under `query.build` are about what
//! the builders *produce*, and the produced text is the observable.
//!
//! Run the test locally:
//! cargo test --test query_build_tests

use pgorm::alias;
use pgorm::pgorm_query::{
    Asterisk, ConditionType, DeleteStatement, Expr, Func, GroupingElement, InsertStatement,
    IntoCondition, IntoName, LockBehavior, LockType, Name, NullOrdering, OnConflict, QueryBuilder,
    SelectStatement, SimpleExpr, UpdateStatement, Value, Values,
};
use pgorm::set;
use pgorm::tests_cfg::{
    active_enums::Tea, cake, cake_filling, cake_filling_price, entity_linked, filling, fruit,
    lunch_set, vendor,
};
use pgorm::{
    ActiveValue, ColumnTrait, Condition, Delete, DeleteMany, DeleteOne, EntityTrait, Error, Insert,
    IntoActiveModel, Iterable, JoinType, Linked, ManyRows, ModelTrait, OneRow, Order, QueryFilter,
    QueryOrder, QuerySelect, QueryTrait, Related, RelationTrait, Select, StaticName, TryInsert,
    Update, UpdateMany, UpdateOne,
};
use pretty_assertions::assert_eq;

fn apple() -> cake::ActiveModel {
    cake::ActiveModel {
        id: set(1),
        name: set("Apple Pie"),
    }
}

/// Exercises the three blanket traits over whichever SELECT builder is passed
/// in, proving the shared modification surface really is shared.
fn narrow<Q>(query: Q) -> Q
where
    Q: QuerySelect<QueryStatement = SelectStatement>
        + QueryOrder<QueryStatement = SelectStatement>
        + QueryFilter<QueryStatement = SelectStatement>,
{
    query
        .filter(cake::Column::Id.gt(0))
        .order_by_asc(cake::Column::Id)
        .limit(1)
}

// [spec:pgorm:req:query.build+2/test]    every builder in the inventory, each
// wrapping exactly one pgorm-query statement reachable through `QueryTrait`,
// and the blanket `QuerySelect`/`QueryOrder`/`QueryFilter` surface
#[test]
fn every_builder_wraps_one_statement() {
    let select: Select<cake::Entity> = cake::Entity::find();
    assert_eq!(
        select.as_query().to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake""#
    );
    let _: SelectStatement = select.into_query();

    let insert: Insert<cake::ActiveModel, OneRow> = Insert::one(apple());
    assert_eq!(
        insert.as_query().to_string(),
        r#"INSERT INTO "cake" ("id", "name") VALUES (1, 'Apple Pie')"#
    );
    let _: InsertStatement = insert.into_query();

    let try_insert: TryInsert<cake::ActiveModel, OneRow> =
        Insert::one(apple()).on_empty_do_nothing();
    let _: InsertStatement = try_insert.into_query();

    let update_one: UpdateOne<cake::ActiveModel> =
        Update::one(apple()).expect("the primary key is set");
    assert_eq!(
        update_one.as_query().to_string(),
        r#"UPDATE "cake" SET "name" = 'Apple Pie' WHERE "cake"."id" = 1"#
    );
    let _: UpdateStatement = update_one.into_query();

    let update_many: UpdateMany<cake::Entity> =
        Update::many(cake::Entity).col_expr(cake::Column::Name, Expr::value("Pie"));
    assert_eq!(
        update_many.as_query().to_string(),
        r#"UPDATE "cake" SET "name" = 'Pie'"#
    );
    let _: UpdateStatement = update_many.into_query();

    let delete_one: DeleteOne<cake::ActiveModel> =
        Delete::one(apple()).expect("the primary key is set");
    assert_eq!(
        delete_one.as_query().to_string(),
        r#"DELETE FROM "cake" WHERE "cake"."id" = 1"#
    );
    let _: DeleteStatement = delete_one.into_query();

    let delete_many: DeleteMany<cake::Entity> = Delete::many(cake::Entity);
    assert_eq!(delete_many.as_query().to_string(), r#"DELETE FROM "cake""#);
    let _: DeleteStatement = delete_many.into_query();

    // The same three blanket traits drive the SELECT builder and every
    // projection state it moves through.
    assert_eq!(
        narrow(cake::Entity::find()).as_query().to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" > 0 ORDER BY "cake"."id" ASC LIMIT 1"#
    );
    assert_eq!(
        narrow(cake::Entity::find().select(cake::Column::Name))
            .as_query()
            .to_string(),
        r#"SELECT "cake"."name" FROM "cake" WHERE "cake"."id" > 0 ORDER BY "cake"."id" ASC LIMIT 1"#
    );
}

// [spec:pgorm:sem:query.build.select-defaults/test]    the select list is every
// column in `Column::iter()` order through `select_as`, the FROM is the
// entity's table ref, and nothing else is applied
#[test]
fn select_new_has_only_columns_and_from() {
    assert_eq!(
        cake::Column::iter()
            .map(|c| c.as_str().to_owned())
            .collect::<Vec<_>>(),
        ["id", "name"]
    );
    assert_eq!(
        cake::Entity::find().as_query().to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake""#
    );

    // `select_as` casts an enum column to text; the rest pass through.
    assert_eq!(
        lunch_set::Entity::find().as_query().to_string(),
        [
            r#"SELECT "lunch_set"."id", "lunch_set"."name","#,
            r#"CAST("lunch_set"."tea" AS text) FROM "lunch_set""#,
        ]
        .join(" ")
    );

    // The FROM clause is `E::default().table_ref()`, schema included.
    assert_eq!(
        cake_filling_price::Entity::find().as_query().to_string(),
        [
            r#"SELECT "cake_filling_price"."cake_id", "cake_filling_price"."filling_id","#,
            r#""cake_filling_price"."price" FROM "public"."cake_filling_price""#,
        ]
        .join(" ")
    );

    // No default WHERE / ORDER BY / GROUP BY / LIMIT / OFFSET.
    let sql = cake::Entity::find().as_query().to_string();
    for clause in ["WHERE", "ORDER BY", "GROUP BY", "HAVING", "LIMIT", "OFFSET"] {
        assert!(!sql.contains(clause), "unexpected {clause} in {sql}");
    }
}

// [spec:pgorm:def:query.build.query-trait/test]    `query`, `as_query`,
// `into_query` and `build` — the last rendering PostgreSQL text with `$n`
// placeholders and a parallel `Values`, taking no backend argument
#[test]
fn query_trait_exposes_the_statement_four_ways() {
    let mut select = cake::Entity::find().filter(cake::Column::Id.eq(3));

    let (sql, values) = select.build();
    assert_eq!(
        sql,
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = $1"#
    );
    assert_eq!(values, Values(vec![Value::Int(Some(3))]));

    // `as_query` is shared access: the same statement, values inlined.
    assert_eq!(
        select.as_query().to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = 3"#
    );

    // `query` is mutable access to the very same statement.
    QueryTrait::query(&mut select).and_where(cake::Column::Name.like("%pie%"));
    assert_eq!(
        select.build().0,
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = $1 AND "cake"."name" LIKE $2"#
    );

    // `into_query` hands over ownership of that statement.
    let statement: SelectStatement = select.into_query();
    assert_eq!(
        statement.to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = 3 AND "cake"."name" LIKE '%pie%'"#
    );
}

// [spec:pgorm:def:query.build.query-trait/test]    `apply_if` runs the closure
// only for `Some`, leaving the fluent chain unbroken for `None`
#[test]
fn apply_if_runs_only_on_some() {
    assert_eq!(
        cake::Entity::find()
            .apply_if(Some(3), |query, v| query.filter(cake::Column::Id.eq(v)))
            .apply_if(Some(100), QuerySelect::limit)
            .apply_if(None, QuerySelect::offset::<Option<u64>>)
            .apply_if(None::<i32>, |query, v| query.filter(cake::Column::Id.eq(v)))
            .as_query()
            .to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = 3 LIMIT 100"#
    );
}

// [spec:pgorm:def:query.build.query-trait/test]    `IntoSimpleExpr` accepts a
// `ColumnTrait` (yielding a table-qualified column reference), an `Expr` and a
// `SimpleExpr` (identity)
#[test]
fn into_simple_expr_accepts_three_shapes() {
    let by_column = cake::Entity::find().order_by_asc(cake::Column::Name);
    let by_expr = cake::Entity::find().order_by_asc(Expr::col((cake::Entity, cake::Column::Name)));
    let by_simple: SimpleExpr = Expr::col((cake::Entity, cake::Column::Name)).into();

    let expected =
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" ORDER BY "cake"."name" ASC"#.to_owned();
    assert_eq!(by_column.as_query().to_string(), expected);
    assert_eq!(by_expr.as_query().to_string(), expected);
    assert_eq!(
        cake::Entity::find()
            .order_by_asc(by_simple)
            .as_query()
            .to_string(),
        expected
    );

    // A non-column `SimpleExpr` passes through unchanged too.
    assert_eq!(
        cake::Entity::find()
            .group_by(cake::Column::Id.count())
            .as_query()
            .to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" GROUP BY COUNT("cake"."id")"#
    );
}

// [spec:pgorm:sem:query.build.filter+2/test]    repeated `filter` calls AND
// together; condition trees, `add_option` and raw pgorm-query expressions all
// arrive through the same entry point
#[test]
fn filter_accumulates_and_accepts_trees() {
    assert_eq!(
        cake::Entity::find()
            .filter(cake::Column::Id.eq(4))
            .filter(cake::Column::Id.eq(5))
            .as_query()
            .to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = 4 AND "cake"."id" = 5"#
    );

    assert_eq!(
        cake::Entity::find()
            .filter(
                Condition::any()
                    .add(cake::Column::Id.eq(4))
                    .add(cake::Column::Id.eq(5))
            )
            .filter(Condition::all().add(cake::Column::Name.contains("cheese")))
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
            r#"WHERE ("cake"."id" = 4 OR "cake"."id" = 5) AND "cake"."name" LIKE '%cheese%'"#,
        ]
        .join(" ")
    );

    // `add_option` drops a `None` predicate without breaking the chain.
    let absent: Option<String> = None;
    assert_eq!(
        cake::Entity::find()
            .filter(Condition::all().add_option(absent.map(|n| cake::Column::Name.contains(&n))))
            .filter(cake::Column::Id.is_in([4, 5]))
            .as_query()
            .to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" IN (4, 5)"#
    );

    // A raw pgorm-query expression is accepted verbatim.
    assert_eq!(
        fruit::Entity::find()
            .filter(Expr::col(fruit::Column::CakeId).is_null())
            .as_query()
            .to_string(),
        [
            r#"SELECT "fruit"."id", "fruit"."name", "fruit"."cake_id" FROM "fruit""#,
            r#"WHERE "cake_id" IS NULL"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:entity.traits.column+9/test]    `eq_any` / `ne_all` spend one
// parameter on the whole list, so the statement text is the same at every
// cardinality, where `is_in` / `is_not_in` spend one per element
#[test]
fn eq_any_filters_through_one_parameter() {
    let built = |e: SimpleExpr| cake::Entity::find().filter(e).as_query().build();

    assert_eq!(
        built(cake::Column::Id.eq_any([4, 5])),
        (
            r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = ANY($1)"#
                .to_owned(),
            Values(vec![Value::array([4, 5])])
        )
    );
    assert_eq!(
        built(cake::Column::Id.ne_all([4, 5])),
        (
            r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" <> ALL($1)"#
                .to_owned(),
            Values(vec![Value::array([4, 5])])
        )
    );

    // The list length does not reach the SQL, so an empty list is the same
    // statement as any other and needs no constant fall-back.
    assert_eq!(
        built(cake::Column::Id.eq_any(Vec::<i32>::new())).0,
        built(cake::Column::Id.eq_any((1..=100).collect::<Vec<i32>>())).0
    );

    assert_eq!(
        cake::Entity::find()
            .filter(cake::Column::Id.eq_any(Vec::<i32>::new()))
            .as_query()
            .to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = ANY(ARRAY []::int4[])"#
    );
}

// [spec:pgorm:sem:entity.traits.column.enum-cast+15/test]    an enum column casts
// the whole array parameter to `{enum_name}[]`, where the per-element operators
// cast each element to `{enum_name}`
#[test]
fn eq_any_casts_enum_arrays_as_a_whole() {
    assert_eq!(
        lunch_set::Entity::find()
            .filter(lunch_set::Column::Tea.eq_any([Tea::EverydayTea, Tea::BreakfastTea]))
            .as_query()
            .to_string(),
        [
            r#"SELECT "lunch_set"."id", "lunch_set"."name", CAST("lunch_set"."tea" AS text)"#,
            r#"FROM "lunch_set""#,
            r#"WHERE "lunch_set"."tea" = ANY(CAST(ARRAY ['EverydayTea','BreakfastTea'] AS tea[]))"#,
        ]
        .join(" ")
    );

    // Parameterized, the cast operand is pinned to the array's own element type
    // per `sql.render.cast-param-type`, so the driver writes text where the
    // server expects text and the outer cast resolves the enum.
    assert_eq!(
        lunch_set::Entity::find()
            .filter(lunch_set::Column::Tea.eq_any([Tea::EverydayTea, Tea::BreakfastTea]))
            .as_query()
            .build()
            .0,
        [
            r#"SELECT "lunch_set"."id", "lunch_set"."name", CAST("lunch_set"."tea" AS text)"#,
            r#"FROM "lunch_set""#,
            r#"WHERE "lunch_set"."tea" = ANY(CAST($1::text[] AS tea[]))"#,
        ]
        .join(" ")
    );

    // A non-enum column is left untouched.
    assert_eq!(
        lunch_set::Entity::find()
            .filter(lunch_set::Column::Name.eq_any(["Set A".to_owned()]))
            .as_query()
            .to_string(),
        [
            r#"SELECT "lunch_set"."id", "lunch_set"."name", CAST("lunch_set"."tea" AS text)"#,
            r#"FROM "lunch_set""#,
            r#"WHERE "lunch_set"."name" = ANY(ARRAY ['Set A'])"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.filter+2/test]    `belongs_to` emits one equality
// per primary-key column of the model's entity; `belongs_to_tbl_alias`
// qualifies the same columns with a table alias
#[test]
fn belongs_to_filters_every_primary_key_column() {
    let single = cake::Model {
        id: 12,
        name: String::new(),
    };
    assert_eq!(
        fruit::Entity::find()
            .belongs_to(&single)
            .as_query()
            .to_string(),
        [
            r#"SELECT "fruit"."id", "fruit"."name", "fruit"."cake_id" FROM "fruit""#,
            r#"WHERE "cake"."id" = 12"#,
        ]
        .join(" ")
    );

    // A composite primary key contributes one equality per column, in
    // `PrimaryKey::iter()` order.
    let composite = cake_filling::Model {
        cake_id: 2,
        filling_id: 3,
    };
    assert_eq!(
        filling::Entity::find()
            .belongs_to(&composite)
            .as_query()
            .to_string(),
        [
            r#"SELECT "filling"."id", "filling"."name", "filling"."vendor_id" FROM "filling""#,
            r#"WHERE "cake_filling"."cake_id" = 2 AND "cake_filling"."filling_id" = 3"#,
        ]
        .join(" ")
    );

    assert_eq!(
        filling::Entity::find()
            .belongs_to_tbl_alias(&composite, Name::runtime("r0"))
            .as_query()
            .to_string(),
        [
            r#"SELECT "filling"."id", "filling"."name", "filling"."vendor_id" FROM "filling""#,
            r#"WHERE "r0"."cake_id" = 2 AND "r0"."filling_id" = 3"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.modifiers+10/test]    `select_only` clears the list;
// `column`/`columns` re-add through `select_as`; `column_as`, `expr_as`,
// `tbl_col_as`, `expr` and `exprs` append explicit expressions
#[test]
fn select_list_modifiers_rewrite_the_list() {
    assert_eq!(
        cake::Entity::find()
            .select_only()
            .columns([cake::Column::Id, cake::Column::Name])
            .column_as(cake::Column::Id.count(), Name::runtime("count"))
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name", COUNT("cake"."id") AS "count""#,
            r#"FROM "cake""#,
        ]
        .join(" ")
    );

    // `column` applies the same enum cast as the default select list.
    assert_eq!(
        lunch_set::Entity::find()
            .select_only()
            .column(lunch_set::Column::Tea)
            .as_query()
            .to_string(),
        r#"SELECT CAST("lunch_set"."tea" AS text) FROM "lunch_set""#
    );

    assert_eq!(
        cake::Entity::find()
            .select_only()
            .expr(Expr::col((cake::Entity, cake::Column::Id)))
            .exprs([Expr::col((cake::Entity, cake::Column::Name))])
            .expr_as(
                Func::upper(Expr::col((cake::Entity, cake::Column::Name))),
                Name::runtime("name_upper")
            )
            .tbl_col_as(
                (cake::Entity, cake::Column::Name),
                Name::runtime("cake_name")
            )
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name", UPPER("cake"."name") AS "name_upper","#,
            r#""cake"."name" AS "cake_name" FROM "cake""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.modifiers+10/test]    `select` clears the list and
// projects in one call, rendering exactly what `select_only` plus `columns`
// does, and casting an enum column the same way
#[test]
fn select_clears_and_projects_in_one_call() {
    assert_eq!(
        cake::Entity::find()
            .select([cake::Column::Id, cake::Column::Name])
            .as_query()
            .to_string(),
        cake::Entity::find()
            .select_only()
            .columns([cake::Column::Id, cake::Column::Name])
            .as_query()
            .to_string()
    );

    assert_eq!(
        cake::Entity::find()
            .select(cake::Column::Name)
            .column_as(cake::Column::Id.count(), Name::runtime("count"))
            .as_query()
            .to_string(),
        r#"SELECT "cake"."name", COUNT("cake"."id") AS "count" FROM "cake""#
    );

    assert_eq!(
        lunch_set::Entity::find()
            .select(vec![lunch_set::Column::Name, lunch_set::Column::Tea])
            .as_query()
            .to_string(),
        r#"SELECT "lunch_set"."name", CAST("lunch_set"."tea" AS text) FROM "lunch_set""#
    );

    // A second call replaces the projection rather than appending to it.
    assert_eq!(
        cake::Entity::find()
            .select(cake::Column::Id)
            .select(cake::Column::Name)
            .as_query()
            .to_string(),
        r#"SELECT "cake"."name" FROM "cake""#
    );
}

// [spec:pgorm:sem:query.build.modifiers+10/test]    a tuple is the mixed list: two
// entities' columns, a raw expression and an alias token have no common array
// element type
#[test]
fn select_tuple_mixes_columns_and_expressions() {
    let count = alias("count");

    assert_eq!(
        cake::Entity::find()
            .left_join_rel(cake::Relation::Fruit)
            .select((
                cake::Column::Name,
                fruit::Column::Name,
                Expr::col((cake::Entity, cake::Column::Id)),
            ))
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."name", "fruit"."name", "cake"."id" FROM "cake""#,
            r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
        ]
        .join(" ")
    );

    assert_eq!(
        cake::Entity::find()
            .select_only()
            .select((cake::Column::Id, count))
            .as_query()
            .to_string(),
        r#"SELECT "cake"."id", "count" FROM "cake""#
    );

    assert_eq!(
        cake::Entity::find()
            .left_join(fruit::Entity)
            .select((cake::Column::Name, fruit::Column::Name))
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."name", "fruit"."name" FROM "cake""#,
            r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.modifiers+10/test]    rendering a cleared select
// list still emits the text as written — `to_string` and `build` have no
// `Result` channel, so the empty projection is refused at execution instead
// (see `empty_select_tests.rs`)
#[test]
fn cleared_select_list_renders_verbatim() {
    let query = cake::Entity::find().select_only();

    assert_eq!(query.as_query().to_string(), r#"SELECT  FROM "cake""#);
    assert_eq!(query.build().0, r#"SELECT  FROM "cake""#);
}

// [spec:pgorm:sem:query.build.modifiers+10/test]    `limit`/`offset` take
// `Into<Option<u64>>`: the last `Some` wins and `None` removes the clause
#[test]
fn limit_and_offset_last_call_wins() {
    assert_eq!(
        cake::Entity::find()
            .limit(10)
            .offset(5)
            .as_query()
            .to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" LIMIT 10 OFFSET 5"#
    );

    assert_eq!(
        cake::Entity::find()
            .limit(Some(10))
            .limit(Some(20))
            .offset(Some(1))
            .offset(Some(2))
            .as_query()
            .to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake" LIMIT 20 OFFSET 2"#
    );

    assert_eq!(
        cake::Entity::find()
            .limit(10)
            .offset(5)
            .limit(None)
            .offset(None)
            .as_query()
            .to_string(),
        r#"SELECT "cake"."id", "cake"."name" FROM "cake""#
    );
}

// [spec:pgorm:sem:query.build.modifiers+10/test]    `group_by` adds GROUP BY,
// `having` accumulates AND-ed conditions, `distinct` / `distinct_on` and the
// four locking helpers each add their clause
#[test]
fn grouping_distinct_and_locking_clauses() {
    assert_eq!(
        cake::Entity::find()
            .select_only()
            .column_as(cake::Column::Id.count(), Name::runtime("count"))
            .group_by(cake::Column::Name)
            .having(cake::Column::Id.gt(4))
            .having(cake::Column::Id.lt(9))
            .as_query()
            .to_string(),
        [
            r#"SELECT COUNT("cake"."id") AS "count" FROM "cake""#,
            r#"GROUP BY "cake"."name" HAVING "cake"."id" > 4 AND "cake"."id" < 9"#,
        ]
        .join(" ")
    );

    assert_eq!(
        cake::Entity::find().distinct().as_query().to_string(),
        r#"SELECT DISTINCT "cake"."id", "cake"."name" FROM "cake""#
    );
    assert_eq!(
        cake::Entity::find()
            .distinct_on([(cake::Entity, cake::Column::Name)])
            .as_query()
            .to_string(),
        r#"SELECT DISTINCT ON ("cake"."name") "cake"."id", "cake"."name" FROM "cake""#
    );

    let locked = |query: Select<cake::Entity>| query.as_query().to_string();
    assert!(locked(cake::Entity::find().lock_shared()).ends_with("FOR SHARE"));
    assert!(locked(cake::Entity::find().lock_exclusive()).ends_with("FOR UPDATE"));
    assert!(
        locked(cake::Entity::find().lock(LockType::NoKeyUpdate)).ends_with("FOR NO KEY UPDATE")
    );
    assert!(
        locked(cake::Entity::find().lock_with_behavior(LockType::Update, LockBehavior::SkipLocked))
            .ends_with("FOR UPDATE SKIP LOCKED")
    );
}

// [spec:pgorm:sem:query.build.modifiers+10/test]    `group_by_element` passes a grouping
// element through to the same GROUP BY list `group_by` fills
#[test]
fn group_by_element_joins_the_group_by_list() {
    assert_eq!(
        cake::Entity::find()
            .select_only()
            .column(cake::Column::Name)
            .expr(Func::grouping(cake::Column::Name.into_expr()))
            .column_as(cake::Column::Id.count(), Name::runtime("count"))
            .group_by(cake::Column::Id)
            .group_by_element(GroupingElement::rollup([cake::Column::Name.into_expr()]))
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."name", GROUPING("cake"."name"), COUNT("cake"."id") AS "count""#,
            r#"FROM "cake" GROUP BY "cake"."id", ROLLUP ("cake"."name")"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.modifiers+10/test]    ORDER BY expressions append in
// call order and are never deduplicated
#[test]
fn order_by_appends_and_never_dedups() {
    assert_eq!(
        cake::Entity::find()
            .order_by(cake::Column::Id, Order::Asc)
            .order_by_desc(cake::Column::Name)
            .order_by_asc(cake::Column::Id)
            .order_by_with_nulls(cake::Column::Name, Order::Asc, NullOrdering::First)
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name" FROM "cake" ORDER BY "cake"."id" ASC,"#,
            r#""cake"."name" DESC, "cake"."id" ASC, "cake"."name" ASC NULLS FIRST"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.join+5/test]    `join` targets `to_tbl`, `join_rev`
// targets `from_tbl`, and `join_as`/`join_as_rev` re-alias the joined table
// first; the ON condition is one equality per declared column pair
#[test]
fn join_direction_and_alias_choice() {
    assert_eq!(
        cake::Entity::find()
            .join(JoinType::LeftJoin, cake::Relation::Fruit.def())
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
            r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
        ]
        .join(" ")
    );

    assert_eq!(
        fruit::Entity::find()
            .join_rev(JoinType::InnerJoin, cake::Relation::Fruit.def())
            .as_query()
            .to_string(),
        [
            r#"SELECT "fruit"."id", "fruit"."name", "fruit"."cake_id" FROM "fruit""#,
            r#"INNER JOIN "cake" ON "cake"."id" = "fruit"."cake_id""#,
        ]
        .join(" ")
    );

    // A composite `Key` becomes one equality per zipped column pair.
    assert_eq!(
        cake_filling::Entity::find()
            .join(
                JoinType::InnerJoin,
                cake_filling_price::Relation::CakeFilling.def()
            )
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake_filling"."cake_id", "cake_filling"."filling_id" FROM "cake_filling""#,
            r#"INNER JOIN "cake_filling" ON"#,
            r#""cake_filling_price"."cake_id" = "cake_filling"."cake_id" AND"#,
            r#""cake_filling_price"."filling_id" = "cake_filling"."filling_id""#,
        ]
        .join(" ")
    );

    // The alias, when the table ref carries one, replaces the bare identifier
    // on both sides of the ON condition.
    assert_eq!(
        cake::Entity::find()
            .join_as(JoinType::LeftJoin, cake::Relation::Fruit.def(), alias("f"))
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
            r#"LEFT JOIN "fruit" AS "f" ON "cake"."id" = "f"."cake_id""#,
        ]
        .join(" ")
    );
    assert_eq!(
        fruit::Entity::find()
            .join_as_rev(JoinType::LeftJoin, cake::Relation::Fruit.def(), alias("c"))
            .as_query()
            .to_string(),
        [
            r#"SELECT "fruit"."id", "fruit"."name", "fruit"."cake_id" FROM "fruit""#,
            r#"LEFT JOIN "cake" AS "c" ON "c"."id" = "fruit"."cake_id""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.join+5/test]    `condition_type` picks `all` or
// `any` for the ON condition, and the `on_condition` closure is added to it
#[test]
fn join_condition_type_and_custom_predicate() {
    assert_eq!(
        cake::Entity::find()
            .join(JoinType::LeftJoin, cake::Relation::TropicalFruit.def())
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name" FROM "cake" LEFT JOIN "fruit" ON"#,
            r#""cake"."id" = "fruit"."cake_id" AND "fruit"."name" LIKE '%tropical%'"#,
        ]
        .join(" ")
    );

    assert_eq!(
        cake::Entity::find()
            .join(JoinType::LeftJoin, cake::Relation::OrTropicalFruit.def())
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name" FROM "cake" LEFT JOIN "fruit" ON"#,
            r#""cake"."id" = "fruit"."cake_id" OR "fruit"."name" LIKE '%tropical%'"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.join+5/test]    the `Related` helpers join the
// junction relation first when a `via` exists, and `reverse_join` walks the
// relation backwards
#[test]
fn related_helpers_join_via_then_target() {
    assert_eq!(
        cake::Entity::find()
            .left_join(fruit::Entity)
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
            r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
        ]
        .join(" ")
    );

    // cake -> filling has a `via` junction, joined ahead of the target.
    assert_eq!(
        cake::Entity::find()
            .inner_join(filling::Entity)
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
            r#"INNER JOIN "cake_filling" ON "cake"."id" = "cake_filling"."cake_id""#,
            r#"INNER JOIN "filling" ON "cake_filling"."filling_id" = "filling"."id""#,
        ]
        .join(" ")
    );

    assert_eq!(
        cake::Entity::find()
            .right_join(fruit::Entity)
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
            r#"RIGHT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
        ]
        .join(" ")
    );

    assert_eq!(
        fruit::Entity::find()
            .reverse_join(cake::Entity)
            .as_query()
            .to_string(),
        [
            r#"SELECT "fruit"."id", "fruit"."name", "fruit"."cake_id" FROM "fruit""#,
            r#"INNER JOIN "cake" ON "cake"."id" = "fruit"."cake_id""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.join.rel/test]    each `*_join_rel` method is its
// `join(JoinType::X, rel.def())` spelling and each `*_join_rel_rev` its
// `join_rev` one, for a relation carrying whatever `on_condition` it likes
#[test]
fn relation_named_joins_match_the_long_spelling() {
    for (sugar, spelled) in [
        (
            cake::Entity::find().left_join_rel(cake::Relation::Fruit),
            cake::Entity::find().join(JoinType::LeftJoin, cake::Relation::Fruit.def()),
        ),
        (
            cake::Entity::find().inner_join_rel(cake::Relation::Fruit),
            cake::Entity::find().join(JoinType::InnerJoin, cake::Relation::Fruit.def()),
        ),
        (
            cake::Entity::find().right_join_rel(cake::Relation::Fruit),
            cake::Entity::find().join(JoinType::RightJoin, cake::Relation::Fruit.def()),
        ),
        // The relation's own `on_condition` rides along, since the def is the
        // same def.
        (
            cake::Entity::find().left_join_rel(cake::Relation::TropicalFruit),
            cake::Entity::find().join(JoinType::LeftJoin, cake::Relation::TropicalFruit.def()),
        ),
    ] {
        assert_eq!(sugar.as_query().to_string(), spelled.as_query().to_string());
    }

    for (sugar, spelled) in [
        (
            fruit::Entity::find().left_join_rel_rev(cake::Relation::Fruit),
            fruit::Entity::find().join_rev(JoinType::LeftJoin, cake::Relation::Fruit.def()),
        ),
        (
            fruit::Entity::find().inner_join_rel_rev(cake::Relation::Fruit),
            fruit::Entity::find().join_rev(JoinType::InnerJoin, cake::Relation::Fruit.def()),
        ),
        (
            fruit::Entity::find().right_join_rel_rev(cake::Relation::Fruit),
            fruit::Entity::find().join_rev(JoinType::RightJoin, cake::Relation::Fruit.def()),
        ),
    ] {
        assert_eq!(sugar.as_query().to_string(), spelled.as_query().to_string());
    }

    // The sugar is on `QuerySelect`, so it reaches every builder `join` does,
    // and composes with the rest of the surface.
    assert_eq!(
        cake::Entity::find()
            .left_join_rel(cake::Relation::Fruit)
            .filter(fruit::Column::Name.contains("cherry"))
            .as_query()
            .to_string(),
        [
            r#"SELECT "cake"."id", "cake"."name" FROM "cake""#,
            r#"LEFT JOIN "fruit" ON "cake"."id" = "fruit"."cake_id""#,
            r#"WHERE "fruit"."name" LIKE '%cherry%'"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.insert+5/test]    `Insert::new` renders a valid
// DEFAULT VALUES statement before any model is added, and `one`/`many` /
// `add`/`add_many` take anything `IntoActiveModel`
#[test]
fn insert_new_is_a_default_values_statement() {
    // `or_default_values()` renders as a one-row VALUES list of DEFAULT rather
    // than the `DEFAULT VALUES` spelling; PostgreSQL defaults the columns that
    // the list does not reach, so the statement is still valid.
    assert_eq!(
        Insert::many(std::iter::empty::<cake::ActiveModel>())
            .as_query()
            .to_string(),
        r#"INSERT INTO "cake" VALUES (DEFAULT)"#
    );

    // A Model is converted to an ActiveModel on the way in.
    let from_model = Insert::<cake::ActiveModel, OneRow>::one(cake::Model {
        id: 1,
        name: "Apple Pie".to_owned(),
    });
    assert_eq!(
        from_model.as_query().to_string(),
        r#"INSERT INTO "cake" ("id", "name") VALUES (1, 'Apple Pie')"#
    );

    assert_eq!(
        Insert::<cake::ActiveModel, ManyRows>::many([
            cake::Model {
                id: 1,
                name: "Apple Pie".to_owned(),
            },
            cake::Model {
                id: 2,
                name: "Orange Scone".to_owned(),
            },
        ])
        .as_query()
        .to_string(),
        r#"INSERT INTO "cake" ("id", "name") VALUES (1, 'Apple Pie'), (2, 'Orange Scone')"#
    );
}

// [spec:pgorm:sem:query.build.insert+5/test]    `add` writes `Set` and
// `Unchanged` columns through `col.save_as` and omits `NotSet` ones entirely
#[test]
fn insert_add_omits_not_set_columns() {
    assert_eq!(
        Insert::one(cake::ActiveModel {
            id: ActiveValue::NotSet,
            name: set("Apple Pie"),
        })
        .as_query()
        .to_string(),
        r#"INSERT INTO "cake" ("name") VALUES ('Apple Pie')"#
    );

    assert_eq!(
        Insert::one(cake::ActiveModel {
            id: ActiveValue::Unchanged(7),
            name: set("Apple Pie"),
        })
        .as_query()
        .to_string(),
        r#"INSERT INTO "cake" ("id", "name") VALUES (7, 'Apple Pie')"#
    );

    // `save_as` casts an enum value back to its database type.
    assert_eq!(
        Insert::one(lunch_set::ActiveModel {
            id: ActiveValue::NotSet,
            name: set("Set A"),
            tea: set(Tea::EverydayTea),
        })
        .as_query()
        .to_string(),
        r#"INSERT INTO "lunch_set" ("name", "tea") VALUES ('Set A', CAST('EverydayTea' AS tea))"#
    );
}

// [spec:pgorm:sem:query.build.insert+5/test]    `on_conflict` attaches the given
// pgorm-query clause verbatim
#[test]
fn insert_on_conflict_is_attached_verbatim() {
    assert_eq!(
        Insert::one(apple())
            .on_conflict(OnConflict::column(cake::Column::Name).update_column(cake::Column::Name))
            .as_query()
            .to_string(),
        [
            r#"INSERT INTO "cake" ("id", "name") VALUES (1, 'Apple Pie')"#,
            r#"ON CONFLICT ("name") DO UPDATE SET "name" = "excluded"."name""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:query.build.insert.uniform-columns+4/test]    models sharing
// a presence bitmap merge into one multi-row VALUES list
#[test]
fn insert_many_shares_one_column_list() {
    assert_eq!(
        Insert::<cake::ActiveModel, ManyRows>::many([
            cake::ActiveModel {
                id: ActiveValue::NotSet,
                name: set("Apple"),
            },
            cake::ActiveModel {
                id: ActiveValue::NotSet,
                name: set("Orange"),
            },
        ])
        .as_query()
        .to_string(),
        r#"INSERT INTO "cake" ("name") VALUES ('Apple'), ('Orange')"#
    );
}

// [spec:pgorm:req:query.build.insert.uniform-columns+4/test]    a model whose
// presence differs from the first one is recorded as a mismatch, naming the
// column it does not share, and contributes nothing to the statement
#[test]
fn insert_many_rejects_mismatched_columns() {
    let mismatched = || {
        Insert::<cake::ActiveModel, ManyRows>::many([
            cake::ActiveModel {
                id: ActiveValue::NotSet,
                name: set("Apple"),
            },
            cake::ActiveModel {
                id: set(2),
                name: set("Orange"),
            },
        ])
    };

    let err = mismatched()
        .ensure_uniform_columns()
        .expect_err("the second model sets a column the first does not");
    assert_eq!(
        err.to_string(),
        "Query Error: models added to one insert do not share a column set: \
         `id` is set in a later model but not in the first"
    );

    assert_eq!(
        mismatched().as_query().to_string(),
        r#"INSERT INTO "cake" ("name") VALUES ('Apple')"#
    );
}

// [spec:pgorm:sem:query.build.insert+5/test]    a model with nothing set
// contributes a default-values row rather than an arity-zero column and value
// list, and one such row per model
#[test]
fn all_not_set_model_renders_a_default_row() {
    let blank = || cake::ActiveModel {
        id: ActiveValue::NotSet,
        name: ActiveValue::NotSet,
    };

    assert_eq!(
        Insert::<cake::ActiveModel, OneRow>::one(blank())
            .as_query()
            .to_string(),
        r#"INSERT INTO "cake" VALUES (DEFAULT)"#
    );

    assert_eq!(
        Insert::<cake::ActiveModel, ManyRows>::many([blank(), blank(), blank()])
            .as_query()
            .to_string(),
        r#"INSERT INTO "cake" VALUES (DEFAULT), (DEFAULT), (DEFAULT)"#
    );
}

// [spec:pgorm:req:query.build.insert.uniform-columns+4/test]    a first model
// that sets nothing is a column set like any other: a later model that sets a
// column mismatches it
#[test]
fn insert_many_rejects_a_blank_first_model() {
    let err = Insert::<cake::ActiveModel, ManyRows>::many([
        cake::ActiveModel {
            id: ActiveValue::NotSet,
            name: ActiveValue::NotSet,
        },
        cake::ActiveModel {
            id: ActiveValue::NotSet,
            name: set("Orange"),
        },
    ])
    .ensure_uniform_columns()
    .expect_err("the second model sets a column the blank first one does not");

    assert_eq!(
        err.to_string(),
        "Query Error: models added to one insert do not share a column set: \
         `name` is set in a later model but not in the first"
    );
}

// [spec:pgorm:sem:query.build.insert.empty-failsafe+5/test]    `on_empty_do_nothing`
// converts to `TryInsert` without touching the statement, while
// `on_conflict_do_nothing` first attaches ON CONFLICT on the primary key
#[test]
fn try_insert_conversions_and_conflict_clause() {
    let plain = Insert::one(apple()).as_query().to_string();

    assert_eq!(
        Insert::one(apple())
            .on_empty_do_nothing()
            .as_query()
            .to_string(),
        plain
    );

    assert_eq!(
        Insert::one(apple())
            .on_conflict_do_nothing()
            .as_query()
            .to_string(),
        format!(r#"{plain} ON CONFLICT ("id") DO NOTHING"#)
    );

    // Every primary-key column takes part in the conflict target.
    assert_eq!(
        Insert::one(cake_filling::ActiveModel {
            cake_id: set(1),
            filling_id: set(2),
        })
        .on_conflict_do_nothing()
        .as_query()
        .to_string(),
        [
            r#"INSERT INTO "cake_filling" ("cake_id", "filling_id") VALUES (1, 2)"#,
            r#"ON CONFLICT ("cake_id", "filling_id") DO NOTHING"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.update+4/test]    `Update::one` filters on every
// primary-key column and SETs only `Set`, non-key columns
#[test]
fn update_one_sets_changed_non_key_columns() {
    assert_eq!(
        Update::one(cake::ActiveModel {
            id: set(1),
            name: set("Apple Pie"),
        })
        .expect("the primary key is set")
        .as_query()
        .to_string(),
        r#"UPDATE "cake" SET "name" = 'Apple Pie' WHERE "cake"."id" = 1"#
    );

    // An `Unchanged` primary key still supplies the filter, and an `Unchanged`
    // attribute is left out of the SET clause.
    assert_eq!(
        Update::one(fruit::ActiveModel {
            id: ActiveValue::Unchanged(1),
            name: set("Apple"),
            cake_id: ActiveValue::Unchanged(Some(2)),
        })
        .expect("the primary key is unchanged, not unset")
        .as_query()
        .to_string(),
        r#"UPDATE "fruit" SET "name" = 'Apple' WHERE "fruit"."id" = 1"#
    );

    // Composite keys contribute one equality each, and a `Set` primary key is
    // still never written into the SET clause.
    assert_eq!(
        Update::one(cake_filling_price::ActiveModel {
            cake_id: set(1),
            filling_id: set(2),
            price: set(rust_decimal::Decimal::ONE),
        })
        .expect("both primary-key columns are set")
        .as_query()
        .to_string(),
        [
            r#"UPDATE "public"."cake_filling_price" SET "price" = 1"#,
            r#"WHERE "cake_filling_price"."cake_id" = 1"#,
            r#"AND "cake_filling_price"."filling_id" = 2"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.update+4/test]    a `NotSet` primary key has no
// filter to contribute, so `Update::one` refuses to build the statement
#[test]
fn update_one_errs_on_unset_primary_key() {
    let err = Update::one(cake::ActiveModel {
        id: ActiveValue::NotSet,
        name: set("Apple Pie"),
    })
    .expect_err("a NotSet primary key cannot narrow the update");
    assert_eq!(err, Error::PrimaryKeyNotSet);

    // A composite key rejects the model when any one of its columns is unset.
    let err = Update::one(cake_filling_price::ActiveModel {
        cake_id: set(1),
        filling_id: ActiveValue::NotSet,
        price: set(rust_decimal::Decimal::ONE),
    })
    .expect_err("half a composite key is not a key");
    assert_eq!(err, Error::PrimaryKeyNotSet);

    // `EntityTrait::update` forwards the same error.
    let err = Update::one(cake::ActiveModel {
        id: ActiveValue::NotSet,
        name: set("Apple Pie"),
    })
    .expect_err("the entry point forwards the builder's error");
    assert_eq!(err, Error::PrimaryKeyNotSet);
}

// [spec:pgorm:sem:query.build.update+4/test]    `Update::many` adds no implicit
// filter; `set` writes `Set` columns including primary keys, `col_expr` writes
// a raw expression, and `QueryFilter` supplies the WHERE clause
#[test]
fn update_many_has_no_implicit_filter() {
    assert_eq!(
        Update::many(cake::Entity)
            .set(cake::ActiveModel {
                id: set(9),
                name: set("Pie"),
            })
            .as_query()
            .to_string(),
        r#"UPDATE "cake" SET "id" = 9, "name" = 'Pie'"#
    );

    assert_eq!(
        Update::many(cake::Entity)
            .set(cake::ActiveModel {
                id: ActiveValue::Unchanged(9),
                name: set("Pie"),
            })
            .filter(cake::Column::Name.contains("Apple"))
            .as_query()
            .to_string(),
        r#"UPDATE "cake" SET "name" = 'Pie' WHERE "cake"."name" LIKE '%Apple%'"#
    );

    assert_eq!(
        Update::many(fruit::Entity)
            .col_expr(fruit::Column::Name, Expr::value("Golden Apple"))
            .filter(fruit::Column::Name.contains("Apple"))
            .as_query()
            .to_string(),
        r#"UPDATE "fruit" SET "name" = 'Golden Apple' WHERE "fruit"."name" LIKE '%Apple%'"#
    );
}

// [spec:pgorm:sem:query.build.delete+3/test]    `Delete::one` filters on the
// primary key only — non-key attributes never reach the WHERE clause
#[test]
fn delete_one_filters_by_primary_key_only() {
    assert_eq!(
        Delete::one(cake::Model {
            id: 1,
            name: "Apple Pie".to_owned(),
        })
        .expect("the primary key is set")
        .as_query()
        .to_string(),
        r#"DELETE FROM "cake" WHERE "cake"."id" = 1"#
    );

    assert_eq!(
        Delete::one(cake::ActiveModel {
            id: ActiveValue::Unchanged(1),
            name: set("Apple Pie"),
        })
        .expect("the primary key is unchanged, not unset")
        .as_query()
        .to_string(),
        r#"DELETE FROM "cake" WHERE "cake"."id" = 1"#
    );

    assert_eq!(
        Delete::one(cake_filling::Model {
            cake_id: 1,
            filling_id: 2,
        })
        .expect("both primary-key columns are set")
        .as_query()
        .to_string(),
        [
            r#"DELETE FROM "cake_filling""#,
            r#"WHERE "cake_filling"."cake_id" = 1 AND "cake_filling"."filling_id" = 2"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:query.build.delete+3/test]    a `NotSet` primary key has no
// filter to contribute, so `Delete::one` refuses to build the statement
#[test]
fn delete_one_errs_on_unset_primary_key() {
    let err = Delete::one(cake::ActiveModel {
        id: ActiveValue::NotSet,
        name: set("Apple Pie"),
    })
    .expect_err("a NotSet primary key cannot narrow the delete");
    assert_eq!(err, Error::PrimaryKeyNotSet);

    // A composite key rejects the model when any one of its columns is unset.
    let err = Delete::one(cake_filling::ActiveModel {
        cake_id: set(1),
        filling_id: ActiveValue::NotSet,
    })
    .expect_err("half a composite key is not a key");
    assert_eq!(err, Error::PrimaryKeyNotSet);

    // `EntityTrait::delete` forwards the same error.
    let err = Delete::one(cake::ActiveModel {
        id: ActiveValue::NotSet,
        name: set("Apple Pie"),
    })
    .expect_err("the entry point forwards the builder's error");
    assert_eq!(err, Error::PrimaryKeyNotSet);
}

// [spec:pgorm:sem:query.build.delete+3/test]    `Delete::many` is bare; narrowing
// it is the caller's job through `QueryFilter`
#[test]
fn delete_many_is_unconstrained_until_filtered() {
    assert_eq!(
        Delete::many(fruit::Entity).as_query().to_string(),
        r#"DELETE FROM "fruit""#
    );

    assert_eq!(
        Delete::many(fruit::Entity)
            .filter(fruit::Column::Name.contains("Apple"))
            .as_query()
            .to_string(),
        r#"DELETE FROM "fruit" WHERE "fruit"."name" LIKE '%Apple%'"#
    );
}

// [spec:pgorm:sem:query.build.alias+2/test]    one token binding serves the
// aliasing position and every reference to the name it introduced, across the
// `IntoKey`, `IntoName` and `IntoColumnRef` conversions alike
#[test]
fn alias_token_declares_and_references_one_name() {
    let n = alias("n");

    // `column_as` takes the token through `IntoKey`, which keys on
    // `StaticName`; `having` / `order_by_asc` take it through `IntoColumnRef`.
    assert_eq!(
        cake::Entity::find()
            .select_only()
            .column_as(cake::Column::Id.count(), n)
            .group_by(cake::Column::Name)
            .having(Expr::col(n).gt(1))
            .order_by_asc(Expr::col(n))
            .as_query()
            .to_string(),
        [
            r#"SELECT COUNT("cake"."id") AS "n" FROM "cake""#,
            r#"GROUP BY "cake"."name" HAVING "n" > 1 ORDER BY "n" ASC"#,
        ]
        .join(" ")
    );

    // The same token stands as a table alias and as the qualifier of a
    // `(table, column)` pair.
    let f = alias("f");
    assert_eq!(
        cake::Entity::find()
            .select_only()
            .column_as(
                Expr::col((f, fruit::Column::Name)),
                Name::runtime("fruit_name")
            )
            .join_as(JoinType::LeftJoin, cake::Relation::Fruit.def(), f)
            .as_query()
            .to_string(),
        [
            r#"SELECT "f"."name" AS "fruit_name" FROM "cake""#,
            r#"LEFT JOIN "fruit" AS "f" ON "cake"."id" = "f"."cake_id""#,
        ]
        .join(" ")
    );

    // The ceiling: a token no projection declared still builds, and only the
    // server can object to it.
    let undeclared = cake::Entity::find()
        .filter(Expr::col(alias("nope")).gt(1))
        .as_query()
        .to_string();
    assert!(undeclared.ends_with(r#"WHERE "nope" > 1"#), "{undeclared}");
}

// [spec:pgorm:req:entity.relation.linked+4/test]    the alias a linked join
// binds its last hop to is derived from the chain, not retyped by the caller
#[test]
fn last_hop_alias_matches_the_emitted_join() {
    let three_hop = entity_linked::CakeToFillingVendor;
    let two_hop = entity_linked::CakeToFilling;

    // One hop shorter, one rung lower: the derivation moves with the chain.
    assert_eq!(three_hop.last_hop_alias().to_string(), "r2");
    assert_eq!(two_hop.last_hop_alias().to_string(), "r1");

    // And it is the alias the join actually emitted for the chain's last rung.
    let target = three_hop.last_hop_alias();
    assert_eq!(
        three_hop
            .find_linked()
            .order_by_asc(Expr::col((target, cake::Column::Id)))
            .as_query()
            .to_string()
            .split(" ORDER BY ")
            .nth(1),
        Some(r#""r2"."id" ASC"#)
    );
}
