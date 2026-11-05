//! Tables keyed by a foreign key from their primary key onto a base's, with
//! no kind column, generate as an unkinded disjoint entity where the caller
//! names the set — the base's entity holding a `#[derive(Disjoint)]` enum
//! declared `unkinded`, and each kind's entity as the ordinary entity it is —
//! and as ordinary entities where it does not; a named set without the shape
//! is refused, naming why.
//!
//! The files the writer is held to here are compiled, and read through
//! against a live server, at the workspace root
//! (`tests/codegen_disjoint_unkinded_tests.rs`), whose module tree they are.

mod common;

use common::*;
use pgorm_codegen::{TableIdent, UnkindedDisjoint, WithSerde};

const SCHEMA: &str = include_str!("../../tests/codegen_disjoint_unkinded/schema.sql");

fn table(name: &str) -> TableIdent {
    TableIdent {
        table: name.to_owned(),
        schema: None,
    }
}

fn crm(name: &str) -> TableIdent {
    TableIdent {
        table: name.to_owned(),
        schema: Some("crm".to_owned()),
    }
}

fn set(base: TableIdent, kinds: Vec<TableIdent>, unit: Option<&str>) -> UnkindedDisjoint {
    UnkindedDisjoint {
        base,
        kinds,
        unit: unit.map(str::to_owned),
    }
}

/// The two sets the fixtures name: the shop's, with a unit kind, and the
/// account's composite one, with none.
fn sets() -> Vec<UnkindedDisjoint> {
    vec![
        set(
            table("shop"),
            vec![table("website"), table("storefront")],
            Some("dormant"),
        ),
        set(crm("account"), vec![crm("person"), crm("company")], None),
    ]
}

fn with(unkinded_disjoint: Vec<UnkindedDisjoint>) -> Opts {
    Opts {
        unkinded_disjoint,
        ..Default::default()
    }
}

// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1/test]    each named set generates its base's entity holding a `#[derive(Disjoint)]` enum declared `unkinded`, a variant per kind's table in the order named and the named unit variant last, and each kind's entity as the ordinary entity it is, its foreign key its `belongs_to` and the base's inverse `has_one`, a composite key in a schema of its own included
#[test]
fn named_sets_generate_unkinded_enums() {
    let generated = from_sql_with(SCHEMA, with(sets()));
    assert_eq!(
        generated.names(),
        [
            "account.rs",
            "company.rs",
            "person.rs",
            "shop.rs",
            "storefront.rs",
            "website.rs",
            "mod.rs",
            "prelude.rs"
        ]
    );
    for (file, fixture) in [
        ("shop.rs", "../../tests/codegen_disjoint_unkinded/shop.rs"),
        (
            "website.rs",
            "../../tests/codegen_disjoint_unkinded/website.rs",
        ),
        (
            "storefront.rs",
            "../../tests/codegen_disjoint_unkinded/storefront.rs",
        ),
        (
            "account.rs",
            "../../tests/codegen_disjoint_unkinded/account.rs",
        ),
        (
            "person.rs",
            "../../tests/codegen_disjoint_unkinded/person.rs",
        ),
        (
            "company.rs",
            "../../tests/codegen_disjoint_unkinded/company.rs",
        ),
    ] {
        generated.holds(file, fixture);
    }
}

// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1/test]    a set is named, never guessed: unnamed, the same tables generate the ordinary entities alone, and naming one set leaves the other ordinary
#[test]
fn unnamed_sets_generate_ordinary_entities() {
    let ordinary = from_sql_with(SCHEMA, Opts::default());
    for (_, content) in &ordinary.files {
        assert_not_contains(content, "Disjoint");
    }
    let named = from_sql_with(SCHEMA, with(sets()));
    for file in ["website.rs", "storefront.rs", "person.rs", "company.rs"] {
        assert_eq!(named.file(file), ordinary.file(file), "{file}");
    }
    let shop_only = from_sql_with(SCHEMA, with(sets()[..1].to_vec()));
    assert_contains(shop_only.file("shop.rs"), "unkinded");
    assert_eq!(shop_only.file("account.rs"), ordinary.file("account.rs"));
}

// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1/test]    under serde the enum derives what the kinded enum derives, and no `kind_attrs`, since no model holds its kind enum
#[test]
fn unkinded_enum_under_serde_takes_no_kind_attrs() {
    let generated = from_sql_with(
        SCHEMA,
        Opts {
            with_serde: WithSerde::Both,
            unkinded_disjoint: sets(),
            ..Default::default()
        },
    );
    assert_contains(
        generated.file("shop.rs"),
        r#"#[derive(Clone, Debug, PartialEq, Eq, Disjoint, Serialize, Deserialize)]
        #[pgorm(base = Entity, unkinded)]
        pub enum Shop {
            Website(super::website::Model),
            Storefront(super::storefront::Model),
            Dormant,
        }"#,
    );
    assert_not_contains(generated.file("shop.rs"), "kind_attrs");
}

/// The shop's schema with `from` replaced by `to` once, the replacement
/// asserted to happen.
fn shop_with(from: &str, to: &str) -> String {
    let sql = SCHEMA.replacen(from, to, 1);
    assert_ne!(sql, SCHEMA, "the schema is changed");
    sql
}

fn shop_set(kinds: &[&str], unit: Option<&str>) -> Vec<UnkindedDisjoint> {
    vec![set(
        table("shop"),
        kinds.iter().copied().map(table).collect(),
        unit,
    )]
}

// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1/test]    a named set the schema does not hold is refused naming why: a table it does not have, a table named twice or the base among its kinds, no kind's table, a base named twice, a base without a primary key or with a temporal one; an unqualified name resolves as a foreign key's target does
#[test]
fn a_named_set_the_schema_lacks_is_refused() {
    let prefix = "unkinded disjoint entity over `shop`: ";
    for (sets, why) in [
        (
            vec![set(table("shop"), vec![table("online")], None)],
            "table `online` is not in the schema",
        ),
        (
            shop_set(&["website", "website"], None),
            "it names table `website` twice",
        ),
        (
            shop_set(&["website", "shop"], None),
            "it names table `shop` twice",
        ),
        (shop_set(&[], Some("dormant")), "it names no kind's table"),
        (
            [
                shop_set(&["website"], None),
                shop_set(&["storefront"], None),
            ]
            .concat(),
            "it is named twice, and a base's module holds one enum",
        ),
    ] {
        assert_eq!(refused_with(SCHEMA, with(sets)), format!("{prefix}{why}"));
    }
    assert_eq!(
        refused_with(
            SCHEMA,
            with(vec![set(table("outlet"), vec![table("website")], None)])
        ),
        "unkinded disjoint entity over `outlet`: table `outlet` is not in the schema"
    );
    let unqualified = from_sql_with(
        SCHEMA,
        with(vec![set(table("account"), vec![table("person")], None)]),
    );
    assert_contains(
        unqualified.file("account.rs"),
        "#[pgorm(base = Entity, unkinded)] pub enum Account { Person(super::person::Model) }",
    );

    let keyless = shop_with(
        r#""name" varchar NOT NULL, PRIMARY KEY ("id") );"#,
        r#""name" varchar NOT NULL, UNIQUE ("id") );"#,
    );
    assert_eq!(
        refused_with(&keyless, with(shop_set(&["website"], None))),
        format!("{prefix}its base has no primary key")
    );
    let temporal = shop_with(
        r#""name" varchar NOT NULL, PRIMARY KEY ("id") );"#,
        r#""name" varchar NOT NULL, "open" tstzrange NOT NULL, PRIMARY KEY ("id", "open" WITHOUT OVERLAPS) );"#,
    );
    assert_eq!(
        refused_with(&temporal, with(shop_set(&["website"], None))),
        format!("{prefix}its base's primary key ends WITHOUT OVERLAPS")
    );
}

// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1/test]    a named kind's table not keyed by a foreign key from exactly its primary key onto the base's, column for column and of one type, is refused rather than generated as an ordinary entity: no such key, a key from other columns, onto other columns or another table, pairing the columns otherwise, matching a PERIOD, and a key column of another type
#[test]
fn a_named_kind_without_the_key_is_refused() {
    let unkeyed = |table: &str| {
        format!(
            "unkinded disjoint entity over `shop`: the primary key of `{table}` is not a \
             foreign key onto the base's primary key, column for column"
        )
    };
    let web_key = r#"FOREIGN KEY ("shop_id") REFERENCES "shop" ("id")"#;
    for (what, from, to) in [
        ("no foreign key", format!(", {web_key}"), String::new()),
        (
            "a key from another column",
            web_key.to_owned(),
            r#""url_shop" integer, FOREIGN KEY ("url_shop") REFERENCES "shop" ("id")"#.to_owned(),
        ),
        (
            "a key onto another column",
            web_key.to_owned(),
            r#"FOREIGN KEY ("shop_id") REFERENCES "shop" ("name")"#.to_owned(),
        ),
        (
            "a key onto another table",
            web_key.to_owned(),
            r#"FOREIGN KEY ("shop_id") REFERENCES "storefront" ("id")"#.to_owned(),
        ),
        (
            "a key matching a PERIOD",
            web_key.to_owned(),
            r#""during" tstzrange NOT NULL, FOREIGN KEY ("shop_id", PERIOD "during") REFERENCES "shop" ("id", PERIOD "open")"#.to_owned(),
        ),
    ] {
        let sql = shop_with(&from, &to).replacen(
            r#""name" varchar NOT NULL, PRIMARY KEY"#,
            r#""name" varchar NOT NULL, "open" tstzrange, PRIMARY KEY"#,
            1,
        );
        assert_eq!(
            refused_with(&sql, with(shop_set(&["website"], None))),
            unkeyed("website"),
            "{what}"
        );
    }

    let crossed = SCHEMA.replacen(
        r#"FOREIGN KEY ("account_id", "tenant_id") REFERENCES "crm"."account" ("id", "tenant")"#,
        r#"FOREIGN KEY ("account_id", "tenant_id") REFERENCES "crm"."account" ("tenant", "id")"#,
        1,
    );
    assert_ne!(crossed, SCHEMA);
    assert_eq!(
        refused_with(
            &crossed,
            with(vec![set(crm("account"), vec![crm("company")], None)])
        ),
        "unkinded disjoint entity over `crm.account`: the primary key of `crm.company` is not a \
         foreign key onto the base's primary key, column for column"
    );

    let wide = shop_with(
        r#""shop_id" integer NOT NULL, "url" varchar NOT NULL"#,
        r#""shop_id" bigint NOT NULL, "url" varchar NOT NULL"#,
    );
    assert_eq!(
        refused_with(&wide, with(shop_set(&["website"], None))),
        "unkinded disjoint entity over `shop`: column `website`.`shop_id` is not of the type \
         of `shop`.`id`, which it is keyed by"
    );
}

// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1/test]    a base a kind column makes a kinded disjoint entity's is refused as unkinded, its kind column named
#[test]
fn a_kinded_base_named_unkinded_is_refused() {
    let kinded = include_str!("../../tests/codegen_disjoint/schema.sql")
        .split("CREATE SCHEMA")
        .next()
        .expect("the schema opens with the vehicle's statements");
    assert_eq!(
        refused_with(
            kinded,
            with(vec![set(table("vehicle"), vec![table("car")], None)])
        ),
        "unkinded disjoint entity over `vehicle`: its base is the base of a disjoint entity \
         whose kind column `kind` holds each row's kind"
    );
}

// [spec:pgorm:sem:codegen.entity.disjoint.unkinded+1/test]    the unit variant is refused where its name has no Rust form or is a kind's variant's, and the enum's kind enum is held to the base module's names as the enum is
#[test]
fn the_unit_and_kind_enum_names_are_held() {
    assert_eq!(
        refused_with(SCHEMA, with(shop_set(&["website"], Some("9 lives")))),
        "unkinded disjoint entity over `shop`: its unit variant: `9Lives` is not a valid Rust \
         identifier"
    );
    assert_eq!(
        refused_with(
            SCHEMA,
            with(shop_set(&["website", "storefront"], Some("website")))
        ),
        "unkinded disjoint entity over `shop`: its unit variant `Website` is named as the \
         variant of `website`"
    );

    let shop_kind = format!(
        "CREATE TYPE \"shop_kind\" AS ENUM ('online', 'street');\n{}",
        shop_with(
            r#""name" varchar NOT NULL, PRIMARY KEY"#,
            r#""name" varchar NOT NULL, "class" shop_kind NOT NULL, PRIMARY KEY"#,
        )
    );
    assert_eq!(
        refused_with(&shop_kind, with(shop_set(&["website"], None))),
        "table `shop`: its disjoint entity's kind enum `ShopKind` is named as the type enum \
         `shop_kind` generates"
    );
}
