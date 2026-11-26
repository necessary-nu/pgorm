use super::*;
use crate::postgres::oracle::{assert_eq, parsed_nodes};
use pgsql_syntax::protobuf::ConstrType;

/// `glyph`, its three columns, and whatever keys `keyed` declares on it.
fn table(keyed: impl FnOnce(&mut TableCreateStatement) -> &mut TableCreateStatement) -> String {
    let mut table = Table::create(Glyph::Table);
    table
        .col(ColumnDef::new(Glyph::Id).integer().not_null())
        .col(ColumnDef::new(Glyph::Aspect).integer().not_null())
        .col(ColumnDef::new(Glyph::Image).text());
    keyed(&mut table).to_string()
}

/// The key constraints in `sql` — `PRIMARY KEY` and `UNIQUE`, as against the
/// columns' `NOT NULL`s — as the parser read them, in order.
fn key_nodes(sql: &str) -> Vec<serde_json::Value> {
    parsed_nodes(sql, "Constraint")
        .into_iter()
        .filter(|node| {
            matches!(
                contype(node),
                ConstrType::ConstrPrimary | ConstrType::ConstrUnique
            )
        })
        .collect()
}

/// The one key constraint in `sql`, as the parser read it.
fn constraint_node(sql: &str) -> serde_json::Value {
    let mut found = key_nodes(sql);
    assert_eq!(found.len(), 1, "exactly one key constraint in {sql}");
    found.remove(0)
}

fn names(list: &serde_json::Value) -> Vec<String> {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    item["String"]["sval"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn contype(node: &serde_json::Value) -> ConstrType {
    let code = node["contype"].as_i64().expect("a constraint type");
    ConstrType::try_from(code as i32).expect("a known constraint type")
}

// [spec:pgorm:req:sql.ddl.create-table+16/test]    a column or a tuple of them converts into a
// key of either kind, which renders as the table constraint it names, its columns plain names
#[test]
fn every_key_renders_as_its_table_constraint() {
    let cases = [
        (
            table(|t| t.primary_key((Glyph::Id, Glyph::Aspect))),
            r#"PRIMARY KEY ("id", "aspect")"#,
            ConstrType::ConstrPrimary,
            vec!["id", "aspect"],
            false,
        ),
        (
            table(|t| t.primary_key(Glyph::Id)),
            r#"PRIMARY KEY ("id")"#,
            ConstrType::ConstrPrimary,
            vec!["id"],
            false,
        ),
        (
            table(|t| t.unique(Glyph::Aspect)),
            r#"UNIQUE ("aspect")"#,
            ConstrType::ConstrUnique,
            vec!["aspect"],
            false,
        ),
        (
            table(|t| t.unique((Glyph::Image,))),
            r#"UNIQUE ("image")"#,
            ConstrType::ConstrUnique,
            vec!["image"],
            false,
        ),
        (
            table(|t| {
                t.unique(
                    TableKey::new(Glyph::Image)
                        .col(Glyph::Aspect)
                        .nulls_not_distinct(),
                )
            }),
            r#"UNIQUE NULLS NOT DISTINCT ("image", "aspect")"#,
            ConstrType::ConstrUnique,
            vec!["image", "aspect"],
            true,
        ),
        (
            table(|t| t.unique(TableKey::new(Glyph::Id).cols([Glyph::Aspect, Glyph::Image]))),
            r#"UNIQUE ("id", "aspect", "image")"#,
            ConstrType::ConstrUnique,
            vec!["id", "aspect", "image"],
            false,
        ),
    ];

    for (sql, rendered, kind, columns, nulls_not_distinct) in cases {
        assert_eq!(
            sql,
            [
                r#"CREATE TABLE "glyph" ( "id" integer NOT NULL, "aspect" integer NOT NULL,"#,
                &format!(r#""image" text, {rendered} )"#),
            ]
            .join(" ")
        );
        let node = constraint_node(&sql);
        assert_eq!(contype(&node), kind, "{sql}");
        assert_eq!(names(&node["keys"]), columns, "{sql}");
        assert_eq!(node["nulls_not_distinct"], nulls_not_distinct, "{sql}");
    }

    // The tuple impls reach twelve columns, as a primary key's value does.
    let twelve = TableKey::<Primary>::new(Name::runtime("c1"))
        .cols((2..=12).map(|at| Name::runtime(format!("c{at}"))));
    assert_eq!(twelve.get_columns().len(), 12);
    let converted: TableKey<Unique> = (
        Glyph::Id,
        Glyph::Aspect,
        Glyph::Image,
        Glyph::Id,
        Glyph::Aspect,
        Glyph::Image,
        Glyph::Id,
        Glyph::Aspect,
        Glyph::Image,
        Glyph::Id,
        Glyph::Aspect,
        Glyph::Image,
    )
        .into_table_key();
    assert_eq!(converted.get_columns().len(), 12);
}

// [spec:pgorm:req:sql.ddl.create-table+16/test]    a name, INCLUDE and deferrability ride on
// any key, in the grammar's order
// [spec:pgorm:req:sql.ddl.deferrability+4/test]
#[test]
fn a_key_takes_a_name_include_and_deferrability() {
    let key = TableKey::new(Glyph::Aspect)
        .name(Name::runtime("glyph_aspect"))
        .include([Glyph::Image])
        .include([Glyph::Id])
        .deferrability(Deferrability::DeferrableInitiallyImmediate);
    let sql = table(|t| t.unique(key.clone()));
    assert!(
        sql.ends_with(
            r#""image" text, CONSTRAINT "glyph_aspect" UNIQUE ("aspect") INCLUDE ("image", "id") DEFERRABLE INITIALLY IMMEDIATE )"#
        ),
        "{sql}"
    );
    let node = constraint_node(&sql);
    assert_eq!(node["conname"], "glyph_aspect");
    assert_eq!(names(&node["including"]), ["image", "id"]);
    assert_eq!(node["deferrable"], true);
    assert_eq!(node["initdeferred"], false, "{sql}");

    assert_eq!(
        key.get_name().map(|name| name.to_string()),
        Some("glyph_aspect".to_owned())
    );
    assert_eq!(key.get_include().len(), 2);
    assert_eq!(
        key.get_deferrability(),
        Some(Deferrability::DeferrableInitiallyImmediate)
    );
    assert!(!key.is_nulls_not_distinct());

    // Unnamed, the key writes no CONSTRAINT clause and PostgreSQL derives the
    // name.
    let sql = table(|t| t.primary_key(Glyph::Id));
    assert!(
        sql.ends_with(r#""image" text, PRIMARY KEY ("id") )"#),
        "{sql}"
    );
    assert_eq!(constraint_node(&sql)["conname"], "", "{sql}");
}

// [spec:pgorm:req:sql.ddl.create-table+16/test]    the primary key is one slot a later call
// replaces, so a table renders one PRIMARY KEY however often it is declared; the unique keys
// append, and every key follows the columns, the primary key first
#[test]
fn one_primary_key_and_any_unique_keys() {
    let statement = Table::create(Glyph::Table)
        .col(ColumnDef::new(Glyph::Id).integer().not_null())
        .unique(Glyph::Image)
        .primary_key(TableKey::new(Glyph::Aspect).name(Name::runtime("first")))
        .col(ColumnDef::new(Glyph::Aspect).integer().not_null())
        .unique((Glyph::Aspect, Glyph::Image))
        .primary_key((Glyph::Id, Glyph::Aspect))
        .col(ColumnDef::new(Glyph::Image).text())
        .to_owned();
    let sql = statement.to_string();
    assert_eq!(
        sql,
        [
            r#"CREATE TABLE "glyph" ( "id" integer NOT NULL, "aspect" integer NOT NULL, "image" text,"#,
            r#"PRIMARY KEY ("id", "aspect"), UNIQUE ("image"), UNIQUE ("aspect", "image") )"#,
        ]
        .join(" ")
    );
    let kinds: Vec<ConstrType> = key_nodes(&sql).iter().map(contype).collect();
    assert_eq!(
        kinds,
        [
            ConstrType::ConstrPrimary,
            ConstrType::ConstrUnique,
            ConstrType::ConstrUnique
        ]
    );

    let key = statement
        .get_primary_key()
        .expect("the key the last call declared");
    assert_eq!(
        key.get_columns()
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>(),
        ["id", "aspect"]
    );
    assert!(
        key.get_name().is_none(),
        "the replaced key's name went with it"
    );
    let unique: Vec<Vec<String>> = statement
        .get_unique_keys()
        .iter()
        .map(|key| {
            key.get_columns()
                .iter()
                .map(|name| name.to_string())
                .collect()
        })
        .collect();
    assert_eq!(unique, [vec!["image"], vec!["aspect", "image"]]);

    assert!(Table::create(Glyph::Table).get_primary_key().is_none());
}

// [spec:pgorm:req:sql.ddl.create-table+16/test]    a key ending WITHOUT OVERLAPS writes that
// column last, after every other key column whatever order the calls come in, and beside a
// name, NULLS NOT DISTINCT, INCLUDE and deferrability in the grammar's order
#[test]
fn a_temporal_key_ends_without_overlaps() {
    let sql = table(|t| {
        t.primary_key(
            TableKey::new(Glyph::Id)
                .without_overlaps(Glyph::Image)
                .col(Glyph::Aspect),
        )
    });
    assert!(
        sql.ends_with(r#""image" text, PRIMARY KEY ("id", "aspect", "image" WITHOUT OVERLAPS) )"#),
        "{sql}"
    );
    let node = constraint_node(&sql);
    assert_eq!(contype(&node), ConstrType::ConstrPrimary);
    assert_eq!(names(&node["keys"]), ["id", "aspect", "image"]);
    assert_eq!(node["without_overlaps"], true, "{sql}");

    let key = TableKey::new(Glyph::Id)
        .without_overlaps(Glyph::Aspect)
        .without_overlaps(Glyph::Image)
        .name(Name::runtime("glyph_period"))
        .nulls_not_distinct()
        .include([Glyph::Aspect])
        .deferrability(Deferrability::DeferrableInitiallyDeferred);
    let sql = table(|t| t.unique(key.clone()));
    assert!(
        sql.ends_with(
            r#""image" text, CONSTRAINT "glyph_period" UNIQUE NULLS NOT DISTINCT ("id", "image" WITHOUT OVERLAPS) INCLUDE ("aspect") DEFERRABLE INITIALLY DEFERRED )"#
        ),
        "a later column replaces the earlier: {sql}"
    );
    let node = constraint_node(&sql);
    assert_eq!(names(&node["keys"]), ["id", "image"]);
    assert_eq!(node["without_overlaps"], true);
    assert_eq!(node["nulls_not_distinct"], true);
    assert_eq!(names(&node["including"]), ["aspect"]);
    assert_eq!(node["initdeferred"], true, "{sql}");

    assert_eq!(
        key.get_columns()
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>(),
        ["id"],
        "the equality columns, without the period"
    );
    assert_eq!(
        key.get_without_overlaps().map(|name| name.to_string()),
        Some("image".to_owned())
    );
    assert!(
        TableKey::<Unique>::new(Glyph::Id)
            .get_without_overlaps()
            .is_none()
    );

    // ALTER TABLE adds the same key, in the same spelling after ADD.
    assert_eq!(
        Table::alter(Glyph::Table)
            .add_primary_key(TableKey::new(Glyph::Id).without_overlaps(Glyph::Image))
            .to_string(),
        r#"ALTER TABLE "glyph" ADD PRIMARY KEY ("id", "image" WITHOUT OVERLAPS)"#
    );
}
