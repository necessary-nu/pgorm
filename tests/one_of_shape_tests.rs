//! A one-of reference described at run time, as a language binding describes
//! one from its models: it writes the `CHECK` the derive's does, reads and
//! assigns a row's reference and loads its targets by the same rules, and
//! refuses what the derive does not compile.
//!
//! Run locally:
//! `cargo nextest run --test one_of_shape_tests`

use pgorm::pgorm_query::{ColumnType, Name, SelectStatement, StringLen, TableName, Value};
use pgorm::tests_cfg::comment::Commentable;
use pgorm::{ColumnTypeTrait, DisjointColumn, DisjointTable, Error, OneOfShape, Schema};
use pretty_assertions::assert_eq;

fn table(name: &str, columns: Vec<DisjointColumn>) -> DisjointTable {
    DisjointTable::new(TableName::Table(Name::runtime(name)), columns, &["id"])
        .expect("the table is described")
}

fn comment() -> DisjointTable {
    table(
        "comment",
        vec![
            DisjointColumn::new("id", ColumnType::Integer.def()),
            DisjointColumn::new("body", ColumnType::String(StringLen::None).def()),
            DisjointColumn::new("post_id", ColumnType::Integer.def().nullable()),
            DisjointColumn::new("photo_id", ColumnType::BigInteger.def().nullable()),
        ],
    )
}

fn post() -> DisjointTable {
    table(
        "post",
        vec![
            DisjointColumn::new("id", ColumnType::Integer.def()),
            DisjointColumn::new("title", ColumnType::String(StringLen::None).def()),
        ],
    )
}

fn photo() -> DisjointTable {
    table(
        "photo",
        vec![
            DisjointColumn::new("id", ColumnType::BigInteger.def()),
            DisjointColumn::new("url", ColumnType::String(StringLen::None).def()),
        ],
    )
}

/// `Commentable` described at run time.
fn commentable() -> OneOfShape {
    OneOfShape::new(
        "commentable",
        &comment(),
        &[
            ("post", "post_id", &post()),
            ("photo", "photo_id", &photo()),
        ],
        true,
    )
    .expect("the reference is described")
}

fn refusal<T: std::fmt::Debug>(outcome: Result<T, Error>) -> String {
    match outcome {
        Err(Error::Type(message)) => message,
        Err(Error::Query(message)) => message.to_string(),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// [spec:pgorm:def:disjoint.one-of+4/test]    a reference described at run time is the derive's: the same CHECK, the same targets named by their variants, and the same condition loading a target's rows by key
#[test]
fn a_described_reference_is_the_derives() {
    let typed = OneOfShape::of::<Commentable>();
    let described = commentable();
    assert_eq!(
        described.check().to_string(),
        Schema::new().create_one_of::<Commentable>().to_string()
    );
    assert_eq!(typed.check().to_string(), described.check().to_string());
    let targets = |shape: &OneOfShape| {
        shape
            .targets()
            .iter()
            .map(|target| {
                (
                    target.value().to_owned(),
                    target.column().to_string(),
                    target.table().table().to_string(),
                    target.key().to_string(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(targets(&typed), targets(&described));
    assert_eq!(
        targets(&typed)[0],
        ("post".into(), "post_id".into(), "post".into(), "id".into())
    );
    assert!(typed.is_optional() && described.is_optional());
    let read = |shape: &OneOfShape| {
        let mut select = SelectStatement::new();
        select
            .column(Name::runtime("id"))
            .from(Name::runtime("photo"))
            .and_where(
                shape
                    .load_condition(1, vec![Value::BigInt(Some(2)), Value::BigInt(Some(1))])
                    .expect("the target is the reference's"),
            );
        select.to_string()
    };
    assert_eq!(read(&typed), read(&described));
    assert_eq!(
        read(&described),
        r#"SELECT "id" FROM "photo" WHERE "photo"."id" IN (2, 1)"#
    );
}

// [spec:pgorm:def:disjoint.one-of+4/test]    a row's reference is the one target whose column it sets, none only where the reference is optional, and two set refused naming the row and the columns; a write sets its target's column and nulls the rest
#[test]
fn a_row_references_one_target() {
    let shape = commentable();
    let row = [Value::Int(Some(7))];
    assert_eq!(shape.referenced(&row, &[false, true]).ok(), Some(Some(1)));
    assert_eq!(shape.referenced(&row, &[false, false]).ok(), Some(None));
    assert_eq!(
        refusal(shape.referenced(&row, &[true, true])),
        "the `comment` row whose key is (`id` = 7) references more than one target, setting \
         `post_id`, `photo_id`, where at most one of `post_id`, `photo_id` is set"
    );
    let required = OneOfShape::new(
        "attached",
        &comment(),
        &[
            ("post", "post_id", &post()),
            ("photo", "photo_id", &photo()),
        ],
        false,
    )
    .expect("the reference is described");
    assert_eq!(
        refusal(required.referenced(&row, &[false, false])),
        "the `comment` row whose key is (`id` = 7) references no target, setting none of \
         `post_id`, `photo_id`, where exactly one is set"
    );
    assert_eq!(shape.assigned(Some(0)).ok(), Some(vec![true, false]));
    assert_eq!(shape.assigned(None).ok(), Some(vec![false, false]));
    assert_eq!(
        refusal(required.assigned(None)),
        "the one-of reference `attached` over `comment` is not optional: a row references one \
         of its targets"
    );
    assert_eq!(shape.target("photo").map(|(index, _)| index).ok(), Some(1));
    assert_eq!(
        refusal(shape.target("video")),
        "`video` is no target of the one-of reference `commentable` over `comment`, whose \
         targets are `post`, `photo`"
    );
    assert_eq!(
        shape.absent(0, &row, &Value::Int(Some(9))).to_string(),
        "Query Error: the `comment` row whose key is (`id` = 7) references the `post` row of \
         key 9 through `post_id`, and `post` holds no such row"
    );
}

// [spec:pgorm:def:disjoint.one-of+4/test]    what the derive does not compile is refused as a reference is described: a name past 59 bytes, no target, a value twice, a column the entity lacks, one not nullable, one for two targets, a target keyed by other than one column or by another type, and one table for two values
#[test]
fn a_described_reference_refuses_what_the_derive_refuses() {
    let (comment, post, photo) = (comment(), post(), photo());
    let describe = |targets: &[(&str, &str, &DisjointTable)]| {
        refusal(OneOfShape::new("commentable", &comment, targets, true))
    };
    assert_eq!(
        refusal(OneOfShape::new(
            &"n".repeat(60),
            &comment,
            &[("post", "post_id", &post)],
            true
        )),
        format!(
            "the one-of reference over `comment` is named `{}`, and its name is 1 to 59 bytes, \
             so that the CHECK named for it keeps it whole within PostgreSQL's 63",
            "n".repeat(60)
        )
    );
    assert_eq!(
        describe(&[]),
        "the one-of reference `commentable` over `comment` names no target: it names at least one"
    );
    assert_eq!(
        describe(&[("post", "post_id", &post), ("post", "photo_id", &photo)]),
        "the one-of reference `commentable` over `comment` names the target `post` twice"
    );
    assert_eq!(
        describe(&[("post", "video_id", &post)]),
        "`comment` has no column `video_id` to hold the key of the target `post`"
    );
    assert_eq!(
        describe(&[("post", "post_id", &post), ("photo", "post_id", &photo)]),
        "`comment`.`post_id` holds the keys of the targets `post` and `photo`: each target's \
         key is held in a column of its own"
    );
    assert_eq!(
        describe(&[("post", "id", &post)]),
        "`comment`.`id` is not nullable, and a one-of reference's column is null in every row \
         referencing another target"
    );
    assert_eq!(
        describe(&[("post", "post_id", &post), ("photo", "photo_id", &post)]),
        "`post` is the target of `post` and of `photo`: each value names a target of its own"
    );
    let pair = DisjointTable::new(
        TableName::Table(Name::runtime("pair")),
        vec![
            DisjointColumn::new("a", ColumnType::Integer.def()),
            DisjointColumn::new("b", ColumnType::Integer.def()),
        ],
        &["a", "b"],
    )
    .expect("the table is described");
    assert_eq!(
        describe(&[("pair", "post_id", &pair)]),
        "`pair`, the target `pair`, is keyed by 2 columns, and a one-of reference's column \
         holds a key of one"
    );
    assert_eq!(
        describe(&[("photo", "post_id", &photo)]),
        "`comment`.`post_id` is (`Integer`), not (`BigInteger`), the key of `photo`, the \
         target `photo`"
    );
}
