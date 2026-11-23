use super::*;
use crate::oracle::{assert_eq, parsed_nodes};
use pgsql_syntax::protobuf::{DropBehavior, ObjectType};

fn n(name: &str) -> Name {
    Name::runtime(name)
}

/// The one `kind` node in `sql`, as PostgreSQL's parser read it.
fn statement(sql: &str, kind: &str) -> serde_json::Value {
    let mut found = parsed_nodes(sql, kind);
    assert_eq!(found.len(), 1, "exactly one {kind} in {sql}");
    found.remove(0)
}

/// A statement's sequence options as the parser read them: each `DefElem`'s
/// name and argument, in order, with an absent argument (`NO MAXVALUE`) as
/// `null`.
fn options(node: &serde_json::Value) -> Vec<(String, serde_json::Value)> {
    node["options"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|option| {
            let def = &option["DefElem"];
            (
                def["defname"].as_str().expect("an option name").to_owned(),
                def.get("arg").cloned().unwrap_or(serde_json::Value::Null),
            )
        })
        .collect()
}

fn int(value: i64) -> serde_json::Value {
    serde_json::json!({ "Integer": { "ival": value } })
}

fn names(list: &serde_json::Value) -> Vec<&str> {
    list["List"]["items"]
        .as_array()
        .expect("a name list")
        .iter()
        .map(|item| item["String"]["sval"].as_str().expect("a name part"))
        .collect()
}

// [spec:pgorm:req:sql.ddl.sequence/test]    a bare create names the sequence and nothing else
#[test]
fn a_sequence_needs_only_its_name() {
    let sql = Sequence::create(n("ticket")).to_string();
    assert_eq!(sql, r#"CREATE SEQUENCE "ticket""#);
    let create = statement(&sql, "CreateSeqStmt");
    assert_eq!(create["sequence"]["relname"], "ticket");
    assert!(options(&create).is_empty(), "{sql}");
}

// [spec:pgorm:req:sql.ddl.sequence/test]    every create clause, in its one order, read back
// by the grammar as the option it was written as
#[test]
fn every_create_clause_renders_in_one_order() {
    let sql = Sequence::create((n("billing"), n("invoice_no")))
        .if_not_exists()
        .owned_by((n("billing"), n("invoice")), n("no"))
        .options(
            SequenceOption::Cycle
                .and(SequenceOption::Cache(20))
                .and(SequenceOption::StartWith(1000))
                .and(SequenceOption::MaxValue(9999))
                .and(SequenceOption::MinValue(1000))
                .and(SequenceOption::IncrementBy(10)),
        )
        .as_type(SequenceType::Integer)
        .to_string();
    assert_eq!(
        sql,
        [
            r#"CREATE SEQUENCE IF NOT EXISTS "billing"."invoice_no" AS integer"#,
            "INCREMENT BY 10 MINVALUE 1000 MAXVALUE 9999 START WITH 1000 CACHE 20 CYCLE",
            r#"OWNED BY "billing"."invoice"."no""#,
        ]
        .join(" ")
    );
    let create = statement(&sql, "CreateSeqStmt");
    assert_eq!(create["if_not_exists"], true);
    assert_eq!(create["sequence"]["schemaname"], "billing");
    assert_eq!(create["sequence"]["relname"], "invoice_no");
    let read = options(&create);
    let defnames: Vec<&str> = read.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        defnames,
        [
            "as",
            "increment",
            "minvalue",
            "maxvalue",
            "start",
            "cache",
            "cycle",
            "owned_by"
        ]
    );
    assert_eq!(read[1].1, int(10));
    assert_eq!(read[2].1, int(1000));
    assert_eq!(read[3].1, int(9999));
    assert_eq!(read[4].1, int(1000));
    assert_eq!(read[5].1, int(20));
    assert_eq!(
        read[6].1,
        serde_json::json!({ "Boolean": { "boolval": true } })
    );
    assert_eq!(names(&read[7].1), ["billing", "invoice", "no"]);
}

// [spec:pgorm:req:sql.ddl.sequence/test]    the three counting types, and nothing else
#[test]
fn a_sequence_counts_in_one_of_three_types() {
    for (ty, spelled, read) in [
        (SequenceType::SmallInteger, "smallint", "int2"),
        (SequenceType::Integer, "integer", "int4"),
        (SequenceType::BigInteger, "bigint", "int8"),
    ] {
        assert_eq!(ty.keyword(), spelled);
        let sql = Sequence::create(n("s")).as_type(ty).to_string();
        assert_eq!(sql, format!(r#"CREATE SEQUENCE "s" AS {spelled}"#));
        let create = statement(&sql, "CreateSeqStmt");
        let type_name = &options(&create)[0].1["TypeName"]["names"];
        assert_eq!(type_name[1]["String"]["sval"], read, "{sql}");
    }
}

// [spec:pgorm:req:sql.ddl.sequence/test]    a bound and its NO form fill one clause, as CYCLE
// and NO CYCLE do: the later replaces the earlier, so no clause is given twice
#[test]
fn a_clause_is_given_once() {
    let options_sql =
        |options: SequenceOptions| Sequence::create(n("s")).options(options).to_string();
    let sql = options_sql(
        SequenceOption::MinValue(1)
            .and(SequenceOption::MaxValue(5))
            .and(SequenceOption::Cycle)
            .and(SequenceOption::NoMinValue)
            .and(SequenceOption::NoMaxValue)
            .and(SequenceOption::NoCycle),
    );
    assert_eq!(
        sql,
        r#"CREATE SEQUENCE "s" NO MINVALUE NO MAXVALUE NO CYCLE"#
    );
    let read = options(&statement(&sql, "CreateSeqStmt"));
    assert_eq!(
        read,
        [
            ("minvalue".to_owned(), serde_json::Value::Null),
            ("maxvalue".to_owned(), serde_json::Value::Null),
            (
                "cycle".to_owned(),
                serde_json::json!({ "Boolean": { "boolval": false } })
            ),
        ]
    );

    let sql = options_sql(SequenceOption::StartWith(1).and(SequenceOption::StartWith(2)));
    assert_eq!(sql, r#"CREATE SEQUENCE "s" START WITH 2"#);

    // A second `options` call adds to the first rather than discarding it.
    let sql = Sequence::create(n("s"))
        .options(SequenceOption::StartWith(1).and(SequenceOption::Cache(4)))
        .options(SequenceOption::StartWith(9))
        .to_string();
    assert_eq!(sql, r#"CREATE SEQUENCE "s" START WITH 9 CACHE 4"#);
}

// [spec:pgorm:req:sql.ddl.sequence/test]    the numbers are integer literals across the whole
// i64 range, a negative step and both extremes included
#[test]
fn every_i64_reads_back_as_its_literal() {
    let sql = Sequence::create(n("s"))
        .options(
            SequenceOption::IncrementBy(-1)
                .and(SequenceOption::MinValue(i64::MIN))
                .and(SequenceOption::MaxValue(i64::MAX)),
        )
        .to_string();
    assert_eq!(
        sql,
        r#"CREATE SEQUENCE "s" INCREMENT BY -1 MINVALUE -9223372036854775808 MAXVALUE 9223372036854775807"#
    );
    let read = options(&statement(&sql, "CreateSeqStmt"));
    assert_eq!(read[0].1, int(-1));
    // Past `int4` the scanner keeps the digits as a numeric literal, which the
    // server reads as an `int8`; the text is the whole value.
    assert_eq!(read[1].1["Float"]["fval"], "-9223372036854775808");
    assert_eq!(read[2].1["Float"]["fval"], "9223372036854775807");
}

// [spec:pgorm:req:sql.ddl.sequence/test]    OWNED BY NONE, and an owning column whose table is
// unqualified
#[test]
fn owned_by_names_a_column_or_nothing() {
    let sql = Sequence::create(n("s"))
        .owned_by(n("t"), n("c"))
        .to_string();
    assert_eq!(sql, r#"CREATE SEQUENCE "s" OWNED BY "t"."c""#);
    assert_eq!(
        names(&options(&statement(&sql, "CreateSeqStmt"))[0].1),
        ["t", "c"]
    );

    let sql = Sequence::create(n("s")).owned_by_none().to_string();
    assert_eq!(sql, r#"CREATE SEQUENCE "s" OWNED BY NONE"#);
    assert_eq!(
        names(&options(&statement(&sql, "CreateSeqStmt"))[0].1),
        ["none"]
    );
}

// [spec:pgorm:req:sql.ddl.sequence/test]    each clause an alter can begin with is a statement
// on its own, and further clauses chain in the one order
#[test]
fn an_alter_begins_with_any_clause() {
    let alter = || Sequence::alter((n("app"), n("ticket")));
    let cases = [
        (alter().as_type(SequenceType::BigInteger), "AS bigint"),
        (
            alter().options(SequenceOption::IncrementBy(5)),
            "INCREMENT BY 5",
        ),
        (alter().restart(), "RESTART"),
        (alter().restart_with(7), "RESTART WITH 7"),
        (alter().owned_by(n("t"), n("c")), r#"OWNED BY "t"."c""#),
        (alter().owned_by_none(), "OWNED BY NONE"),
    ];
    for (statement_, tail) in cases {
        let sql = statement_.to_string();
        assert_eq!(sql, format!(r#"ALTER SEQUENCE "app"."ticket" {tail}"#));
        let read = statement(&sql, "AlterSeqStmt");
        assert_eq!(read["sequence"]["relname"], "ticket");
        assert_eq!(options(&read).len(), 1, "{sql}");
    }

    let sql = Sequence::alter(n("ticket"))
        .owned_by_none()
        .restart()
        .options(SequenceOption::Cache(2))
        .as_type(SequenceType::SmallInteger)
        .restart_with(3)
        .if_exists()
        .to_string();
    assert_eq!(
        sql,
        r#"ALTER SEQUENCE IF EXISTS "ticket" AS smallint CACHE 2 RESTART WITH 3 OWNED BY NONE"#
    );
    let read = statement(&sql, "AlterSeqStmt");
    assert_eq!(read["missing_ok"], true);
    let read = options(&read);
    assert_eq!(read[2], ("restart".to_owned(), int(3)));
}

// [spec:pgorm:req:sql.ddl.sequence/test]    a drop names every sequence it drops, and one
// behaviour
#[test]
fn a_drop_names_one_or_more_sequences() {
    let sql = Sequence::drop(n("a")).to_string();
    assert_eq!(sql, r#"DROP SEQUENCE "a""#);

    let sql = Sequence::drop(n("a"))
        .name((n("app"), n("b")))
        .if_exists()
        .cascade()
        .restrict()
        .to_string();
    assert_eq!(sql, r#"DROP SEQUENCE IF EXISTS "a", "app"."b" RESTRICT"#);
    let drop = statement(&sql, "DropStmt");
    assert_eq!(drop["remove_type"], ObjectType::ObjectSequence as i32);
    assert_eq!(drop["behavior"], DropBehavior::DropRestrict as i32);
    assert_eq!(drop["missing_ok"], true);
    let objects = drop["objects"].as_array().expect("the dropped names");
    assert_eq!(names(&objects[0]), ["a"]);
    assert_eq!(names(&objects[1]), ["app", "b"]);

    let sql = Sequence::drop(n("a")).restrict().cascade().to_string();
    assert_eq!(sql, r#"DROP SEQUENCE "a" CASCADE"#);
}

// [spec:pgorm:req:sql.ddl.sequence/test]    a rename keeps the sequence's schema, so the new
// name is bare
#[test]
fn a_rename_is_an_alter_of_its_own() {
    let sql = Sequence::rename((n("app"), n("a")), n("b")).to_string();
    assert_eq!(sql, r#"ALTER SEQUENCE "app"."a" RENAME TO "b""#);
    let rename = statement(&sql, "RenameStmt");
    assert_eq!(rename["rename_type"], ObjectType::ObjectSequence as i32);
    assert_eq!(rename["relation"]["schemaname"], "app");
    assert_eq!(rename["relation"]["relname"], "a");
    assert_eq!(rename["newname"], "b");
}

/// The options of the one identity constraint in `sql`.
fn identity_options(sql: &str) -> Vec<(String, serde_json::Value)> {
    let found: Vec<_> = parsed_nodes(sql, "Constraint")
        .into_iter()
        .filter(|constraint| {
            constraint["contype"] == pgsql_syntax::protobuf::ConstrType::ConstrIdentity as i32
        })
        .collect();
    assert_eq!(found.len(), 1, "exactly one identity in {sql}");
    options(&found[0])
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    an identity's sequence takes the same options,
// in parentheses after `AS IDENTITY`, in every place the column is written
// [spec:pgorm:req:sql.ddl.sequence/test]
#[test]
fn an_identity_takes_the_sequence_options() {
    let column = || {
        ColumnDef::new(Glyph::Id)
            .big_integer()
            .identity_with(
                IdentityGeneration::ByDefault,
                SequenceOption::StartWith(100)
                    .and(SequenceOption::IncrementBy(10))
                    .and(SequenceOption::NoCycle),
            )
            .to_owned()
    };
    let tail = "GENERATED BY DEFAULT AS IDENTITY (INCREMENT BY 10 START WITH 100 NO CYCLE)";

    let created = Table::create(Glyph::Table).col(column()).to_string();
    assert_eq!(
        created,
        format!(r#"CREATE TABLE "glyph" ( "id" bigint {tail} )"#)
    );
    let added = Table::alter(Glyph::Table).add_column(column()).to_string();
    assert_eq!(
        added,
        format!(r#"ALTER TABLE "glyph" ADD COLUMN "id" bigint {tail}"#)
    );
    let modified = Table::alter(Glyph::Table)
        .modify_column(
            ColumnDef::new(Glyph::Id).identity_with(
                IdentityGeneration::ByDefault,
                SequenceOption::StartWith(100)
                    .and(SequenceOption::IncrementBy(10))
                    .and(SequenceOption::NoCycle),
            ),
        )
        .to_string();
    assert_eq!(
        modified,
        format!(r#"ALTER TABLE "glyph" ALTER COLUMN "id" ADD {tail}"#)
    );
    for sql in [created, added, modified] {
        let read = identity_options(&sql);
        let defnames: Vec<&str> = read.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(defnames, ["increment", "start", "cycle"], "{sql}");
        assert_eq!(read[0].1, int(10));
        assert_eq!(read[1].1, int(100));
    }

    // With no options there are no parentheses: `AS IDENTITY ()` is a syntax
    // error, and none is spelled by leaving them out.
    let bare = Table::create(Glyph::Table)
        .col(ColumnDef::new(Glyph::Id).integer().identity())
        .to_string();
    assert_eq!(
        bare,
        r#"CREATE TABLE "glyph" ( "id" integer GENERATED ALWAYS AS IDENTITY )"#
    );
    assert!(identity_options(&bare).is_empty());
    crate::oracle::assert_rejected(
        r#"CREATE TABLE "glyph" ( "id" integer GENERATED ALWAYS AS IDENTITY () )"#,
    );
}
