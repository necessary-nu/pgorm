//! Parity for the models JavaScript declares: each case of
//! `tests/parity/models.json` built here with pgorm's own entities — their
//! `find`, `insert`, `update_many` and `delete_many`, and `SelectGraph` —
//! or, where pgorm composes a statement inside a terminal (a cursor's keyset,
//! a paginator's page and count, the RETURNING list of a write's two
//! versions), with pgorm-query as that terminal composes it.

use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{
    Asterisk, Condition, Expr, Func, NamedTable, OnConflict, Order, Query, ReturningClause,
    SelectStatement, SimpleExpr, TableName, TypeName, Values,
};
use pgorm::{Delete, Insert, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Update, set};

use crate::statements::parity::{check, n};

mod account {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "account", schema_name = "app")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        pub name: String,
        pub display_name: Option<String>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(has_many = super::post::Entity)]
        Post,
    }
}

mod post {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "post", schema_name = "app")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i64,
        pub author_id: i64,
        pub title: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::account::Entity,
            from = Column::AuthorId,
            to = super::account::Column::Id
        )]
        Author,
    }
}

mod tag {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "tag", schema_name = "app")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod post_tag {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "post_tag", schema_name = "app")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub post_id: i64,
        #[pgorm(primary_key, auto_increment = false)]
        pub tag_id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::post::Entity,
            from = Column::PostId,
            to = super::post::Column::Id
        )]
        Post,
        #[pgorm(
            belongs_to = super::tag::Entity,
            from = Column::TagId,
            to = super::tag::Column::Id
        )]
        Tag,
    }
}

mod version {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "version", schema_name = "app")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub doc: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub rev: i32,
        pub body: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod ticket {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "ticket")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i64,
        pub note: Option<String>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod long_names {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "long_names")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub column_with_a_name_long_enough_to_need_bounding_in_a_graph_xx: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

fn built<Q: QueryTrait>(query: &Q) -> (String, Values)
where
    Q::QueryStatement: pgorm::pgorm_query::QueryStatementBuilder,
{
    use pgorm::pgorm_query::QueryStatementBuilder;
    query.as_query().build()
}

/// A column of `table`, qualified by its name as an entity's columns are.
fn col(table: &str, column: &str) -> Expr {
    Expr::col((n(table), n(column)))
}

fn app(table: &str) -> NamedTable {
    NamedTable::from(TableName::SchemaTable(n("app"), n(table)))
}

/// `account::Entity::find()`'s statement, a cursor's or paginator's base.
fn accounts() -> SelectStatement {
    account::Entity::find().as_query().to_owned()
}

/// The RETURNING list of both versions of every account column, as pgorm's
/// `exec_returning_change(s)` and `exec_returning_upsert(s)` write it.
fn both_versions() -> ReturningClause {
    let items = [("pgorm_old", "o_"), ("pgorm_new", "n_")]
        .into_iter()
        .flat_map(|(row, prefix)| {
            ["id", "name", "display_name"]
                .into_iter()
                .map(move |column| (col(row, column).into(), n(&format!("{prefix}{column}"))))
        })
        .collect::<Vec<(SimpleExpr, _)>>();
    Query::returning()
        .exprs_as(items)
        .old_as(n("pgorm_old"))
        .new_as(n("pgorm_new"))
}

/// A cursor's keyset boundary over `columns`, written out as pgorm's
/// `Cursor` writes it: n disjuncts, the k-th holding k-1 columns equal.
fn boundary(
    columns: &[(&str, &str)],
    values: &[pgorm::pgorm_query::Value],
    past: bool,
) -> Condition {
    (1..=values.len())
        .rev()
        .fold(Condition::any(), |disjunction, reach| {
            disjunction.add(columns.iter().zip(values).take(reach).enumerate().fold(
                Condition::all(),
                |conjunction, (index, ((table, column), value))| {
                    let column = col(table, column);
                    let value = Expr::val(value.clone());
                    conjunction.add(if index + 1 < reach {
                        column.eq(value)
                    } else if past {
                        column.gt(value)
                    } else {
                        column.lt(value)
                    })
                },
            ))
        })
}

fn queries() -> Vec<(&'static str, (String, Values))> {
    vec![
        ("find", built(&account::Entity::find())),
        (
            "find-filtered",
            built(
                &account::Entity::find()
                    .filter(account::Column::Id.gte(10i64))
                    .order_by_desc(account::Column::Name)
                    .limit(5)
                    .offset(10),
            ),
        ),
        (
            "select-fields",
            built(
                &account::Entity::find()
                    .select_only()
                    .columns([account::Column::Id, account::Column::Name]),
            ),
        ),
        ("find-by-key", built(&account::Entity::find_by_id(7i64))),
        (
            "find-by-composite-key",
            built(&version::Entity::find_by_id((1, 2))),
        ),
        (
            "find-joined",
            built(
                &post::Entity::find()
                    .inner_join(account::Entity)
                    .filter(account::Column::Name.eq("Ann")),
            ),
        ),
        (
            "enum-comparison",
            Query::select()
                .expr(col("moody", "id"))
                .expr(col("moody", "mood"))
                .from(app("moody"))
                .and_where(
                    col("moody", "mood").eq(SimpleExpr::from("calm")
                        .cast_as_type(TypeName::new(n("mood")).schema(n("app")))),
                )
                .build(),
        ),
        (
            "relation-find",
            Query::select()
                .exprs([
                    col("post", "id"),
                    col("post", "author_id"),
                    col("post", "title"),
                ])
                .from(app("post"))
                .cond_where(Condition::all().add(col("post", "author_id").eq(3i64)))
                .build(),
        ),
    ]
}

fn writes() -> Vec<(&'static str, (String, Values))> {
    let named = |name: &str| account::ActiveModel {
        name: set(name),
        ..Default::default()
    };
    vec![
        (
            "insert",
            built(&Insert::one(account::ActiveModel {
                name: set("Ann"),
                display_name: set(None::<String>),
                ..Default::default()
            })),
        ),
        (
            "insert-many",
            built(&Insert::many([named("Ann"), named("Bob")])),
        ),
        (
            "insert-defaults",
            built(&Insert::one(<ticket::ActiveModel as Default>::default())),
        ),
        (
            "insert-returning",
            Query::insert()
                .into_table(app("account"))
                .columns([n("name"), n("display_name")])
                .values_panic(["Ann".into(), "A".into()])
                .returning(Query::returning().columns([n("id"), n("name")]))
                .build(),
        ),
        (
            "update",
            built(
                &Update::many(account::Entity)
                    .col_expr(account::Column::Name, Expr::value("x"))
                    .filter(account::Column::Id.eq(1i64)),
            ),
        ),
        (
            "delete",
            built(&Delete::many(account::Entity).filter(account::Column::Name.eq("x"))),
        ),
        (
            "update-changes",
            Query::update()
                .table(app("account"))
                .value(n("name"), "x")
                .and_where(col("account", "id").gte(2i64))
                .returning(both_versions())
                .build(),
        ),
        (
            "insert-upserts",
            Query::insert()
                .into_table(app("account"))
                .columns([n("id"), n("name")])
                .values_panic([1i64.into(), "Ann".into()])
                .on_conflict(OnConflict::column(n("id")).update_column(n("name")))
                .returning(both_versions())
                .build(),
        ),
    ]
}

fn graphs() -> Vec<(&'static str, (String, Values))> {
    use pgorm::RelationTrait;
    vec![
        (
            "graph",
            built(&post::Entity::graph().join_one::<account::Entity>(post::Relation::Author.def())),
        ),
        (
            "graph-maybe-alias",
            built(
                &account::Entity::graph()
                    .join_maybe_as::<post::Entity>(account::Relation::Post.def(), n("p")),
            ),
        ),
        (
            "graph-via",
            built(
                &post::Entity::graph()
                    .via(post_tag::Relation::Post.def().rev())
                    .join_maybe::<tag::Entity>(post_tag::Relation::Tag.def()),
            ),
        ),
        (
            "graph-filtered",
            built(
                &post::Entity::graph()
                    .join_one::<account::Entity>(post::Relation::Author.def())
                    .filter(post::Column::Title.eq("x"))
                    .order_by_asc(post::Column::Id),
            ),
        ),
        ("graph-long-name", built(&long_names::Entity::graph())),
        (
            "graph-grouped",
            built(
                &account::Entity::graph()
                    .join_maybe::<post::Entity>(account::Relation::Post.def())
                    .order_by_desc(account::Column::Name)
                    .order_by_asc(account::Column::Id),
            ),
        ),
    ]
}

fn cursors() -> Vec<(&'static str, (String, Values))> {
    use pgorm::RelationTrait;
    let graph = || {
        post::Entity::graph()
            .join_maybe::<account::Entity>(post::Relation::Author.def())
            .as_query()
            .to_owned()
    };
    vec![
        (
            "cursor-after-first",
            accounts()
                .cond_where(boundary(&[("account", "id")], &[5i64.into()], true))
                .order_by((n("account"), n("id")), Order::Asc)
                .limit(10)
                .build(),
        ),
        (
            "cursor-composite-last-desc",
            accounts()
                .cond_where(boundary(
                    &[("account", "name"), ("account", "id")],
                    &["m".into(), 4i64.into()],
                    true,
                ))
                .order_by((n("account"), n("name")), Order::Asc)
                .order_by((n("account"), n("id")), Order::Asc)
                .limit(3)
                .build(),
        ),
        (
            "graph-cursor-with",
            graph()
                .cond_where(boundary(
                    &[("post", "title"), ("post", "id"), ("account", "id")],
                    &["t".into(), 1i64.into(), 2i64.into()],
                    true,
                ))
                .order_by((n("post"), n("title")), Order::Asc)
                .order_by((n("post"), n("id")), Order::Asc)
                .order_by((n("account"), n("id")), Order::Asc)
                .limit(2)
                .build(),
        ),
        (
            "page",
            accounts()
                .order_by((n("account"), n("id")), Order::Asc)
                .limit(10)
                .offset(20)
                .build(),
        ),
        (
            "count",
            Query::select()
                .expr_as(Func::count(Expr::col(Asterisk)), n("num_items"))
                .from_subquery(
                    accounts()
                        .and_where(col("account", "name").ne("x"))
                        .to_owned(),
                    n("sub_query"),
                )
                .build(),
        ),
    ]
}

/// An account read as a line of a list, the shape a binding's `PartialModel`
/// declares: a column under its own name, an expression, a column under
/// another name, and a field read from no column.
#[derive(Debug, PartialModel)]
#[pgorm(entity = account::Entity)]
#[allow(dead_code)]
struct AccountLine {
    id: i64,
    #[pgorm(from_expr = Func::upper(Expr::col((account::Entity, account::Column::Name))))]
    shout: String,
    #[pgorm(from_col = account::Column::DisplayName)]
    display: Option<String>,
    #[pgorm(skip)]
    note: String,
}

/// A model query's clauses and projections, as an entity's `Select` writes
/// them: DISTINCT and DISTINCT ON, GROUP BY and HAVING, a row lock, a right
/// join and a reverse one, a projection of columns and expressions, and a
/// partial model's.
fn clauses() -> Vec<(&'static str, (String, Values))> {
    use pgorm::pgorm_query::{JoinType, LockBehavior, LockType};
    use pgorm::{PartialModelTrait, RelationTrait};
    let post_count = || Func::count(col("post", "id"));
    vec![
        ("find-distinct", built(&account::Entity::find().distinct())),
        (
            "find-distinct-on",
            built(
                &account::Entity::find()
                    .distinct_on([(account::Entity, account::Column::Name)])
                    .order_by_asc(account::Column::Name)
                    .order_by_asc(account::Column::Id),
            ),
        ),
        (
            "find-grouped",
            built(
                &account::Entity::find()
                    .group_by(account::Column::Id)
                    .having(account::Column::Name.ne("x")),
            ),
        ),
        (
            "find-locked",
            built(
                &account::Entity::find()
                    .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked),
            ),
        ),
        (
            "find-lock-shared",
            built(&account::Entity::find().lock_shared()),
        ),
        (
            "find-right-joined",
            built(&post::Entity::find().join(JoinType::RightJoin, post::Relation::Author.def())),
        ),
        (
            "find-joined-rev",
            built(
                &account::Entity::find()
                    .join_rev(JoinType::InnerJoin, post::Relation::Author.def())
                    .filter(post::Column::Title.eq("x")),
            ),
        ),
        (
            "projection-columns",
            built(
                &account::Entity::find()
                    .select_only()
                    .column(account::Column::Id)
                    .column_as(account::Column::Name, n("label")),
            ),
        ),
        (
            "projection-grouped",
            built(
                &post::Entity::find()
                    .select_only()
                    .column(post::Column::AuthorId)
                    .expr_as(post_count(), n("posts"))
                    .group_by(post::Column::AuthorId)
                    .having(Expr::expr(post_count()).gt(1i64)),
            ),
        ),
        (
            "projection-joined",
            built(
                &post::Entity::find()
                    .select_only()
                    .column(post::Column::Title)
                    .column_as(account::Column::Name, n("author"))
                    .join(JoinType::InnerJoin, post::Relation::Author.def()),
            ),
        ),
        (
            "partial-model",
            built(&AccountLine::select_cols(
                account::Entity::find()
                    .filter(account::Column::Id.eq(1i64))
                    .select_only(),
            )),
        ),
    ]
}

/// A cursor with a secondary ordering after its key, as pgorm's `Cursor`
/// writes it: `after` takes the key alone, `after_with` the whole ordering.
fn secondary_cursors() -> Vec<(&'static str, (String, Values))> {
    let statement = |with: bool| {
        let mut cursor = account::Entity::find().cursor_by(account::Column::Name);
        cursor.add_secondary_order_by(account::Entity, account::Column::Id);
        match with {
            true => cursor.after_with(("m", 4i64)),
            false => cursor.after("m"),
        };
        cursor
            .first(3)
            .statement()
            .expect("the cursor's statement is composed")
            .build()
    };
    vec![
        ("cursor-secondary-after", statement(false)),
        ("cursor-secondary-after-with", statement(true)),
    ]
}

// [spec:pgorm:req:bindings.models+6/test]
// [spec:pgorm:req:bindings.model-reads/test]
// [spec:pgorm:req:bindings.model-writes+3/test]
// [spec:pgorm:req:bindings.relations+2/test]
// [spec:pgorm:req:bindings.graphs+1/test]
// [spec:pgorm:req:bindings.cursors/test]
// [spec:pgorm:req:bindings.pagination/test]
// [spec:pgorm:req:bindings.model-clauses/test]
// [spec:pgorm:req:bindings.model-projections/test]
// [spec:pgorm:req:bindings.partial-models/test]
// [spec:pgorm:req:bindings.cursor-secondary-order/test]
#[test]
fn model_family_matches_its_golden_file() {
    let cases = [
        queries(),
        writes(),
        graphs(),
        cursors(),
        clauses(),
        secondary_cursors(),
    ]
    .concat();
    check(include_str!("../../tests/parity/models.json"), cases);
}
