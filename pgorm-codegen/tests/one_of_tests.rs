//! Nullable foreign-key columns a `CHECK` holds to one target — the
//! `num_nonnulls(..) = 1` or `<= 1` `Schema::create_one_of` writes — generate
//! as a `#[derive(OneOf)]` enum in their table's module, from DDL text and
//! from statements built in code alike, named for the constraint; a `CHECK`
//! of any other shape, or over columns that are not such keys, is refused by
//! name, since the entity holds no other `CHECK`, and columns no `CHECK`
//! covers stay the ordinary nullable fields they are.
//!
//! The files the writer is held to here are compiled, and the schema they
//! were generated from created and written through against a live server, at
//! the workspace root (`tests/codegen_one_of_tests.rs`), whose module tree
//! they are.

mod common;

use common::runtime_name as n;
use common::*;
use pgorm_query::{
    Check, ColumnDef, Enforcement, Expr, ForeignKey, Func, SimpleExpr, Table, TableCreateStatement,
};

const SCHEMA: &str = include_str!("../../tests/codegen_one_of/schema.sql");

/// The tables the comment's references reach, as DDL text.
const TARGETS: &str = "CREATE TABLE post (id serial PRIMARY KEY, title varchar NOT NULL); \
                       CREATE TABLE photo (id bigserial PRIMARY KEY, url varchar NOT NULL);";

/// The comment table, its columns, the post and photo it may reference, and
/// `check` among its constraints.
fn comment(check: &str) -> String {
    format!(
        "{TARGETS} CREATE TABLE comment (id serial PRIMARY KEY, body varchar NOT NULL, \
         post_id integer REFERENCES post (id), photo_id bigint REFERENCES photo (id), {check});"
    )
}

/// The refusal of the comment table's `CHECK` named `name`.
fn unheld(name: &str) -> String {
    format!(
        "table `comment`: an entity cannot hold the CHECK `{name}`, which is not a one-of reference's"
    )
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

// [spec:pgorm:def:disjoint.one-of+4/test]    the statements `create_table_from_entity` and `create_one_of` write for the comment, read as DDL text, generate its module with a `#[derive(OneOf)]` enum after its `Relation` enum, named for the constraint after `ck-comment-`, a variant per column in the CHECK's order named for its target and holding its entity, and for `<= 1` the unit variant `None`
// [spec:pgorm:sem:codegen.ddl.tables+13/test]    a CHECK added by an ALTER TABLE is read into the table's statement
#[test]
fn a_one_of_check_generates_its_enum() {
    let generated = from_sql(SCHEMA);
    assert_eq!(
        generated.names(),
        ["comment.rs", "photo.rs", "post.rs", "mod.rs", "prelude.rs"]
    );
    for (file, fixture) in [
        ("comment.rs", "../../tests/codegen_one_of/comment.rs"),
        ("post.rs", "../../tests/codegen_one_of/post.rs"),
        ("photo.rs", "../../tests/codegen_one_of/photo.rs"),
    ] {
        generated.holds(file, fixture);
    }
}

// [spec:pgorm:def:disjoint.one-of+4/test]    a CHECK whose targets include its own table generates the enum, the variant naming the module's own `Entity`, which its `Relation` enum's self-reference makes `Related` to itself (compiled and run in `tests/codegen_one_of_tests.rs`)
#[test]
fn a_self_referencing_one_of_generates_its_enum() {
    let generated = from_sql(include_str!(
        "../../tests/codegen_one_of/self_reference/schema.sql"
    ));
    for (file, fixture) in [
        (
            "remark.rs",
            "../../tests/codegen_one_of/self_reference/remark.rs",
        ),
        (
            "topic.rs",
            "../../tests/codegen_one_of/self_reference/topic.rs",
        ),
    ] {
        generated.holds(file, fixture);
    }
}

// [spec:pgorm:def:disjoint.one-of+4/test]    a CHECK named `ck-{enum}` because `ck-{table}-{enum}` would pass 63 bytes, under a table's long name or with an enum's of 59 bytes, generates the enum named for it whole (compiled and run in `tests/codegen_one_of_tests.rs`)
#[test]
fn long_named_one_of_keeps_its_enum_name() {
    let generated = from_sql(include_str!(
        "../../tests/codegen_one_of/long_names/schema.sql"
    ));
    for (file, fixture) in [
        (
            "remark_left_on_an_issue_or_a_release_in_the_project_archive.rs",
            "../../tests/codegen_one_of/long_names/remark_left_on_an_issue_or_a_release_in_the_project_archive.rs",
        ),
        ("note.rs", "../../tests/codegen_one_of/long_names/note.rs"),
        ("issue.rs", "../../tests/codegen_one_of/long_names/issue.rs"),
        (
            "release.rs",
            "../../tests/codegen_one_of/long_names/release.rs",
        ),
    ] {
        generated.holds(file, fixture);
    }
}

// [spec:pgorm:def:disjoint.one-of+4/test]    `ck-{rest}` names the enum for `rest` only where `ck-{table}-{rest}` would not be under 63 bytes, so `create_one_of` would have left the table out; under that, and for a name of 63 bytes, the bounded spelling of a longer one, the enum is named for the whole name
#[test]
fn ck_prefix_drops_as_create_one_of_writes() {
    let enum_of = |table: &str, constraint: &str| {
        let sql = format!(
            "{TARGETS} CREATE TABLE {table} (id serial PRIMARY KEY, \
             post_id integer REFERENCES post (id), photo_id bigint REFERENCES photo (id), \
             CONSTRAINT \"{constraint}\" CHECK (num_nonnulls(post_id, photo_id) = 1));"
        );
        let generated = from_sql(&sql);
        let module = generated.file(&format!("{table}.rs"));
        let one_of = &module[module.find("OneOf").expect("a one-of enum")..];
        let rest = &one_of[one_of.find("pub enum ").expect("its name") + "pub enum ".len()..];
        let at = rest.find(' ').expect("the enum's name ends");
        rest[..at].to_owned()
    };
    assert_eq!(enum_of("comment", "ck-target"), "CkTarget");
    let long = "comment_left_on_a_post_or_a_photo_in_a_gallery_archive";
    assert!(format!("ck-{long}-target").len() >= 63);
    assert!(format!("ck-{long}-tag").len() < 63);
    assert_eq!(enum_of(long, "ck-tag"), "CkTag");
    assert_eq!(enum_of(long, "ck-target"), "Target");
    let bounded = format!("ck-{}", "a".repeat(60));
    assert_eq!(bounded.len(), 63);
    assert_eq!(
        enum_of("comment", &bounded),
        format!("Ck{}", "A".to_owned() + &"a".repeat(59))
    );
}

// [spec:pgorm:def:disjoint.one-of+4/test]    a schema described in code, each table whole with its CHECK, generates as the same DDL text does
#[test]
fn described_tables_generate_as_ddl_text_does() {
    let described = generate(described(commentable()), Opts::default());
    assert_eq!(described.files, from_sql(SCHEMA).files);
}

// [spec:pgorm:def:disjoint.one-of+4/test]    the enum is named for the whole name of a constraint not written as `ck-{table}-..`, and an unnamed CHECK as PostgreSQL names it, `{table}_check` or `{table}_{column}_check` over one column, numbered where an earlier CHECK took the name; `= 1` declares no unit variant
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

// [spec:pgorm:def:disjoint.one-of+4/test]    near misses are refused, naming the CHECK: one naming a column twice, over a column with no foreign key, over a column not nullable, `>= 1`, `= 2`, operands the other way round, `num_nulls`, an extra conjunct, NOT ENFORCED, a column of another type than its target's key, a key onto a column that is not the target's primary key, and a target the table reaches by two keys; the columns of a table with no CHECK stay the ordinary nullable fields
#[test]
fn near_misses_are_refused() {
    let cases: Vec<(&str, String)> = vec![
        (
            "comment_post_id_check",
            comment("CHECK (num_nonnulls(post_id, post_id) = 1)"),
        ),
        (
            "comment_check",
            comment("CHECK (num_nonnulls(post_id, body) <= 1)"),
        ),
        (
            "comment_check",
            format!(
                "{TARGETS} CREATE TABLE comment (id serial PRIMARY KEY, \
                 post_id integer REFERENCES post (id), \
                 photo_id bigint NOT NULL REFERENCES photo (id), \
                 CHECK (num_nonnulls(post_id, photo_id) <= 1));"
            ),
        ),
        (
            "at_least_one",
            comment("CONSTRAINT at_least_one CHECK (num_nonnulls(post_id, photo_id) >= 1)"),
        ),
        (
            "comment_check",
            comment("CHECK (num_nonnulls(post_id, photo_id) = 2)"),
        ),
        (
            "comment_check",
            comment("CHECK (1 = num_nonnulls(post_id, photo_id))"),
        ),
        (
            "comment_check",
            comment("CHECK (num_nulls(post_id, photo_id) = 1)"),
        ),
        (
            "comment_check",
            comment("CHECK (num_nonnulls(post_id, photo_id) = 1 AND body <> '')"),
        ),
        (
            "ck-comment-commentable",
            comment(
                r#"CONSTRAINT "ck-comment-commentable" CHECK (num_nonnulls(post_id, photo_id) = 1) NOT ENFORCED"#,
            ),
        ),
        (
            "comment_check",
            format!(
                "{TARGETS} CREATE TABLE comment (id serial PRIMARY KEY, \
                 post_id integer REFERENCES post (id), \
                 photo_id integer REFERENCES photo (id), \
                 CHECK (num_nonnulls(post_id, photo_id) = 1));"
            ),
        ),
        (
            "comment_post_id_check",
            "CREATE TABLE post (id serial PRIMARY KEY, code integer UNIQUE); \
             CREATE TABLE comment (id serial PRIMARY KEY, \
             post_id integer REFERENCES post (code), \
             CHECK (num_nonnulls(post_id) = 1));"
                .to_owned(),
        ),
        (
            "comment_check",
            format!(
                "{TARGETS} CREATE TABLE comment (id serial PRIMARY KEY, \
                 post_id integer REFERENCES post (id), \
                 pinned_id integer REFERENCES post (id), \
                 photo_id bigint REFERENCES photo (id), \
                 CHECK (num_nonnulls(post_id, photo_id) = 1));"
            ),
        ),
    ];
    for (name, sql) in cases {
        assert_eq!(refused(&sql), unheld(name), "{sql}");
    }
    let plain = from_sql(&format!(
        "{TARGETS} CREATE TABLE comment (id serial PRIMARY KEY, body varchar NOT NULL, \
         post_id integer REFERENCES post (id), photo_id bigint REFERENCES photo (id));"
    ));
    let comment = plain.file("comment.rs");
    assert!(!comment.contains("OneOf"), "{comment}");
    assert_contains(comment, "pub post_id: Option<i32>,");
}

// [spec:pgorm:def:disjoint.one-of+4/test]    near misses described in code are refused as their DDL text is: `>= 1`, an extra conjunct and NOT ENFORCED
#[test]
fn described_near_misses_are_refused() {
    let count = || Expr::expr(num_nonnulls(&["post_id", "photo_id"]));
    for check in [
        Check::new(count().gte(1)),
        Check::new(count().eq(1).and(Expr::col(n("body")).ne(""))),
        Check::new(count().eq(1)).enforcement(Enforcement::NotEnforced),
    ] {
        assert_transform_error(described(check), &unheld("comment_check"));
    }
}

// [spec:pgorm:def:disjoint.one-of+4/test]    an enum, key enum, model enum or variant named as a name the module already declares or its code names otherwise is refused, naming both
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

// [spec:pgorm:sem:codegen.ddl.tables+13/test]    any table-level CHECK within the expression subset is read onto the statement, named or not, and one that is no one-of reference's is refused by the transform, naming it
#[test]
fn other_checks_are_read_and_refused() {
    let sql = "CREATE TABLE t (id int PRIMARY KEY, n int NOT NULL, CHECK (n > 0), \
               CONSTRAINT small CHECK (n < 10) NOT ENFORCED);";
    let statements = pgorm_codegen::parse_schema(sql).expect("the schema reads");
    let checks: Vec<Option<String>> = statements[0]
        .get_checks()
        .iter()
        .map(|check| check.get_name().map(|name| name.to_string()))
        .collect();
    assert_eq!(checks, [None, Some("small".to_owned())]);
    assert_eq!(
        refused(sql),
        "table `t`: an entity cannot hold the CHECK `t_n_check`, which is not a one-of \
         reference's"
    );
    assert_eq!(
        refused(
            "CREATE TABLE t (id int PRIMARY KEY, n int NOT NULL, \
             CONSTRAINT small CHECK (n < 10) NOT ENFORCED);"
        ),
        "table `t`: an entity cannot hold the CHECK `small`, which is not a one-of reference's"
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+21/test]    a table-level CHECK naming a column its table lacks is refused as unresolved
#[test]
fn a_check_on_a_missing_column_is_refused() {
    assert_eq!(
        refused("CREATE TABLE t (id int PRIMARY KEY, CHECK (num_nonnulls(x) = 1));"),
        "statement 1: table `t` has no column `x`"
    );
}
