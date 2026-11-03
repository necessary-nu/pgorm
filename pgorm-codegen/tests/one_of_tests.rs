//! Nullable foreign-key columns a `CHECK` holds to one target — the
//! `num_nonnulls(..) = 1` or `<= 1` `Schema::create_one_of` writes — generate
//! as a `#[derive(OneOf)]` enum in their table's module, from DDL text and
//! from statements built in code alike, named for the constraint; a `CHECK`
//! of any other shape, or over columns that are not such keys, leaves them
//! the ordinary nullable fields they are.
//!
//! The files the writer is held to here are compiled, and the schema they
//! were generated from created and written through against a live server, at
//! the workspace root (`tests/codegen_one_of_tests.rs`), whose module tree
//! they are.

mod common;

use common::*;
use pgorm_codegen::entities_from_sql;
use pgorm_query::{
    Check, ColumnDef, Enforcement, Expr, ForeignKey, Func, Name, SimpleExpr, Table,
    TableCreateStatement,
};

const SCHEMA: &str = include_str!("../../tests/codegen_one_of/schema.sql");

/// The tables the comment's references reach, as DDL text.
const TARGETS: &str = "CREATE TABLE post (id serial PRIMARY KEY, title varchar NOT NULL); \
                       CREATE TABLE photo (id bigserial PRIMARY KEY, url varchar NOT NULL);";

fn from_sql(sql: &str) -> Generated {
    Generated {
        files: entities_from_sql(sql, Opts::default())
            .unwrap_or_else(|error| panic!("{error}: {sql}"))
            .files
            .into_iter()
            .map(|file| (file.name, file.content))
            .collect(),
    }
}

fn refused(sql: &str) -> String {
    match entities_from_sql(sql, Opts::default()) {
        Err(pgorm_codegen::Error::TransformError(message)) => message,
        Err(other) => panic!("expected a TransformError, got {other:?}"),
        Ok(_) => panic!("expected a TransformError, got entities: {sql}"),
    }
}

/// The comment table, its columns, the post and photo it may reference, and
/// `check` among its constraints.
fn comment(check: &str) -> String {
    format!(
        "{TARGETS} CREATE TABLE comment (id serial PRIMARY KEY, body varchar NOT NULL, \
         post_id integer REFERENCES post (id), photo_id bigint REFERENCES photo (id), {check});"
    )
}

/// Assert that the generated `file` is `fixture`, rustfmt's trailing commas
/// aside.
#[track_caller]
fn holds(generated: &Generated, file: &str, fixture: &str) {
    let flat = |src: &str| norm(src).replace(" , )", " )").replace(" , }", " }");
    let written = flat(generated.file(file));
    assert!(written.contains(&flat(fixture)), "{file}: {written}");
}

/// Assert that `comment.rs` declares no one-of enum, and keeps both columns
/// as the nullable fields they are.
#[track_caller]
fn ordinary(generated: &Generated, case: &str) {
    let comment = generated.file("comment.rs");
    assert!(!comment.contains("OneOf"), "{case}: {comment}");
    assert_contains(comment, "pub post_id: Option<i32>,");
}

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn num_nonnulls(columns: &[&str]) -> SimpleExpr {
    columns
        .iter()
        .fold(Func::named(n("num_nonnulls")), |call, column| {
            call.arg(Expr::col(n(column)))
        })
        .into()
}

/// The schema's tables as described in code, the comment's holding `check`.
fn described(check: Check) -> Vec<TableCreateStatement> {
    let key = |name: &str, mut column: ColumnDef| {
        Table::create(n(name))
            .col(column.not_null().auto_increment().to_owned())
            .col(
                ColumnDef::new(n(if name == "post" { "title" } else { "url" }))
                    .string()
                    .not_null()
                    .to_owned(),
            )
            .primary_key(n("id"))
            .to_owned()
    };
    vec![
        key("post", ColumnDef::new(n("id")).integer().to_owned()),
        key("photo", ColumnDef::new(n("id")).big_integer().to_owned()),
        Table::create(n("comment"))
            .col(
                ColumnDef::new(n("id"))
                    .integer()
                    .not_null()
                    .auto_increment()
                    .to_owned(),
            )
            .col(ColumnDef::new(n("body")).string().not_null().to_owned())
            .col(ColumnDef::new(n("post_id")).integer().to_owned())
            .col(ColumnDef::new(n("photo_id")).big_integer().to_owned())
            .primary_key(n("id"))
            .foreign_key(
                ForeignKey::create(n("comment"), n("photo_id"), n("photo"), n("id"))
                    .name(n("fk-comment-photo_id"))
                    .to_owned(),
            )
            .foreign_key(
                ForeignKey::create(n("comment"), n("post_id"), n("post"), n("id"))
                    .name(n("fk-comment-post_id"))
                    .to_owned(),
            )
            .check(check)
            .to_owned(),
    ]
}

fn commentable() -> Check {
    Check::new(Expr::expr(num_nonnulls(&["post_id", "photo_id"])).lte(1))
        .name(n("ck-comment-commentable"))
}

// [spec:pgorm:def:disjoint.one-of+1/test]    the statements `create_table_from_entity` and `create_one_of` write for the comment, read as DDL text, generate its module with a `#[derive(OneOf)]` enum after its `Relation` enum, named for the constraint after `ck-comment-`, a variant per column in the CHECK's order named for its target and holding its entity, and for `<= 1` the unit variant `None`
// [spec:pgorm:sem:codegen.ddl.tables+12/test]    a CHECK added by an ALTER TABLE is read into the table's statement
#[test]
fn a_one_of_check_generates_its_enum() {
    let generated = from_sql(SCHEMA);
    assert_eq!(
        generated.names(),
        ["comment.rs", "photo.rs", "post.rs", "mod.rs", "prelude.rs"]
    );
    for (file, fixture) in [
        (
            "comment.rs",
            include_str!("../../tests/codegen_one_of/comment.rs"),
        ),
        (
            "post.rs",
            include_str!("../../tests/codegen_one_of/post.rs"),
        ),
        (
            "photo.rs",
            include_str!("../../tests/codegen_one_of/photo.rs"),
        ),
    ] {
        holds(&generated, file, fixture);
    }
}

// [spec:pgorm:def:disjoint.one-of+1/test]    a schema described in code, each table whole with its CHECK, generates as the same DDL text does
#[test]
fn described_tables_generate_as_ddl_text_does() {
    let described = generate(described(commentable()), Opts::default());
    assert_eq!(described.files, from_sql(SCHEMA).files);
}

// [spec:pgorm:def:disjoint.one-of+1/test]    the enum is named for the whole name of a constraint not written as `ck-{table}-..`, and an unnamed CHECK as PostgreSQL names it, `{table}_check` or `{table}_{column}_check` over one column, numbered where an earlier CHECK took the name; `= 1` declares no unit variant
#[test]
fn the_enum_is_named_for_the_constraint() {
    let named = from_sql(&comment(
        "CONSTRAINT comment_target CHECK (num_nonnulls(post_id, photo_id) = 1)",
    ));
    assert_contains(
        named.file("comment.rs"),
        "pub enum CommentTarget { \
         #[pgorm(column = Column::PostId)] Post(super::post::Entity), \
         #[pgorm(column = Column::PhotoId)] Photo(super::photo::Entity), }",
    );

    let unnamed = from_sql(&comment(
        "CHECK (num_nonnulls(post_id, photo_id) <= 1), \
         CHECK (num_nonnulls(photo_id, post_id) = 1), \
         CHECK (num_nonnulls(post_id) <= 1)",
    ));
    let file = unnamed.file("comment.rs");
    assert_contains(
        file,
        "pub enum CommentCheck { \
         #[pgorm(column = Column::PostId)] Post(super::post::Entity), \
         #[pgorm(column = Column::PhotoId)] Photo(super::photo::Entity), None, }",
    );
    assert_contains(
        file,
        "pub enum CommentCheck1 { \
         #[pgorm(column = Column::PhotoId)] Photo(super::photo::Entity), \
         #[pgorm(column = Column::PostId)] Post(super::post::Entity), }",
    );
    assert_contains(
        file,
        "pub enum CommentPostIdCheck { \
         #[pgorm(column = Column::PostId)] Post(super::post::Entity), None, }",
    );
}

// [spec:pgorm:def:disjoint.one-of+1/test]    near misses generate the ordinary nullable fields: a CHECK naming a column twice, over a column with no foreign key, over a column not nullable, `>= 1`, `= 2`, operands the other way round, `num_nulls`, an extra conjunct, NOT ENFORCED, a column of another type than its target's key, a key onto a column that is not the target's primary key, and a target the table reaches by two keys
#[test]
fn near_misses_keep_the_ordinary_fields() {
    let cases: Vec<(&str, String)> = vec![
        (
            "a column twice",
            comment("CHECK (num_nonnulls(post_id, post_id) = 1)"),
        ),
        (
            "a column with no foreign key",
            comment("CHECK (num_nonnulls(post_id, body) <= 1)"),
        ),
        (
            "a column not nullable",
            format!(
                "{TARGETS} CREATE TABLE comment (id serial PRIMARY KEY, \
                 post_id integer REFERENCES post (id), \
                 photo_id bigint NOT NULL REFERENCES photo (id), \
                 CHECK (num_nonnulls(post_id, photo_id) <= 1));"
            ),
        ),
        (
            ">= 1",
            comment("CHECK (num_nonnulls(post_id, photo_id) >= 1)"),
        ),
        (
            "= 2",
            comment("CHECK (num_nonnulls(post_id, photo_id) = 2)"),
        ),
        (
            "the operands the other way round",
            comment("CHECK (1 = num_nonnulls(post_id, photo_id))"),
        ),
        (
            "num_nulls",
            comment("CHECK (num_nulls(post_id, photo_id) = 1)"),
        ),
        (
            "an extra conjunct",
            comment("CHECK (num_nonnulls(post_id, photo_id) = 1 AND body <> '')"),
        ),
        (
            "NOT ENFORCED",
            comment("CHECK (num_nonnulls(post_id, photo_id) = 1) NOT ENFORCED"),
        ),
        (
            "a column of another type than its target's key",
            format!(
                "{TARGETS} CREATE TABLE comment (id serial PRIMARY KEY, \
                 post_id integer REFERENCES post (id), \
                 photo_id integer REFERENCES photo (id), \
                 CHECK (num_nonnulls(post_id, photo_id) = 1));"
            ),
        ),
        (
            "a key onto a column that is not the target's primary key",
            "CREATE TABLE post (id serial PRIMARY KEY, code integer UNIQUE); \
             CREATE TABLE comment (id serial PRIMARY KEY, \
             post_id integer REFERENCES post (code), \
             CHECK (num_nonnulls(post_id) = 1));"
                .to_owned(),
        ),
        (
            "a target the table reaches by two keys",
            format!(
                "{TARGETS} CREATE TABLE comment (id serial PRIMARY KEY, \
                 post_id integer REFERENCES post (id), \
                 pinned_id integer REFERENCES post (id), \
                 photo_id bigint REFERENCES photo (id), \
                 CHECK (num_nonnulls(post_id, photo_id) = 1));"
            ),
        ),
    ];
    for (case, sql) in cases {
        ordinary(&from_sql(&sql), case);
    }
}

// [spec:pgorm:def:disjoint.one-of+1/test]    near misses described in code generate the ordinary fields as their DDL text does: `>= 1`, an extra conjunct and NOT ENFORCED
#[test]
fn described_near_misses_keep_the_ordinary_fields() {
    let count = || Expr::expr(num_nonnulls(&["post_id", "photo_id"]));
    for (case, check) in [
        (">= 1", Check::new(count().gte(1))),
        (
            "an extra conjunct",
            Check::new(count().eq(1).and(Expr::col(n("body")).ne(""))),
        ),
        (
            "NOT ENFORCED",
            Check::new(count().eq(1)).enforcement(Enforcement::NotEnforced),
        ),
    ] {
        ordinary(&generate(described(check), Opts::default()), case);
    }
}

// [spec:pgorm:def:disjoint.one-of+1/test]    an enum, key enum, model enum or variant named as a name the module already declares or its code names otherwise is refused, naming both
#[test]
fn clashing_names_are_refused() {
    assert_eq!(
        refused(&comment(
            r#"CONSTRAINT "ck-comment-model" CHECK (num_nonnulls(post_id, photo_id) = 1)"#
        )),
        "table `comment`: its one-of reference's enum `Model` is named as the entity's own \
         `Model`"
    );
    assert_eq!(
        refused(&comment(
            r#"CONSTRAINT "ck-comment-target" CHECK (num_nonnulls(post_id, photo_id) = 1),
               CONSTRAINT "ck-comment-target_key" CHECK (num_nonnulls(post_id, photo_id) <= 1)"#
        )),
        "table `comment`: its one-of reference's enum `TargetKey` is named as its one-of \
         reference `Target`'s key enum `TargetKey`"
    );
    assert_eq!(
        refused(&comment(
            r#"CONSTRAINT "ck-comment-string" CHECK (num_nonnulls(post_id, photo_id) = 1)"#
        )),
        "table `comment`: its one-of reference's enum `String` is named as the `String` its \
         module's code names"
    );
    assert_eq!(
        refused(
            r#"CREATE TYPE mood AS ENUM ('calm');
               CREATE TABLE post (id serial PRIMARY KEY, mood mood);
               CREATE TABLE comment (id serial PRIMARY KEY,
                 post_id integer REFERENCES post (id),
                 CONSTRAINT "ck-comment-mood" CHECK (num_nonnulls(post_id) = 1));"#
        ),
        "table `comment`: its one-of reference's enum `Mood` is named as the type enum `mood` \
         generates"
    );
    assert_eq!(
        refused(
            r#"CREATE TABLE "none" (id serial PRIMARY KEY);
               CREATE TABLE comment (id serial PRIMARY KEY,
                 none_id integer REFERENCES "none" (id),
                 CONSTRAINT "ck-comment-target" CHECK (num_nonnulls(none_id) <= 1));"#
        ),
        "table `comment`: its one-of reference's variant `None` is named as its unit variant"
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+12/test]    any table-level CHECK within the expression subset is read, named or not, and generates nothing in an entity that is no one-of reference
#[test]
fn other_checks_are_read_and_generate_nothing() {
    let generated = from_sql(
        "CREATE TABLE t (id int PRIMARY KEY, n int NOT NULL, CHECK (n > 0), \
         CONSTRAINT small CHECK (n < 10) NOT ENFORCED);",
    );
    assert!(!generated.file("t.rs").contains("CHECK"));
    assert!(!generated.file("t.rs").contains("OneOf"));
}

// [spec:pgorm:req:codegen.ddl.unsupported+20/test]    a table-level CHECK naming a column its table lacks is refused as unresolved
#[test]
fn a_check_on_a_missing_column_is_refused() {
    assert_eq!(
        refused("CREATE TABLE t (id int PRIMARY KEY, CHECK (num_nonnulls(x) = 1));"),
        "statement 1: table `t` has no column `x`"
    );
}
