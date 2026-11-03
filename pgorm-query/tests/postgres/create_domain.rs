use super::*;
use crate::oracle::{assert_eq, parsed_nodes};
use pg_query::protobuf::{ConstrType, DropBehavior, ObjectType};
use std::sync::Arc;

fn n(name: &str) -> Name {
    Name::runtime(name)
}

/// The one `kind` node in `sql`, as PostgreSQL's parser read it.
fn statement(sql: &str, kind: &str) -> serde_json::Value {
    let mut found = parsed_nodes(sql, kind);
    assert_eq!(found.len(), 1, "exactly one {kind} in {sql}");
    found.remove(0)
}

/// A list of `String` nodes, as the parts of a name.
fn parts(list: &serde_json::Value) -> Vec<&str> {
    list.as_array()
        .expect("a list of names")
        .iter()
        .map(|part| part["String"]["sval"].as_str().expect("one name part"))
        .collect()
}

/// A `CreateDomainStmt`'s constraints: each one's kind and name.
fn constraints(create: &serde_json::Value) -> Vec<(i64, String)> {
    create["constraints"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|constraint| {
            let constraint = &constraint["Constraint"];
            (
                constraint["contype"].as_i64().expect("a constraint kind"),
                constraint["conname"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            )
        })
        .collect()
}

fn check(name: &str) -> (i64, String) {
    (ConstrType::ConstrCheck as i64, name.to_owned())
}

fn quantity() -> ColumnType {
    ColumnType::Domain {
        name: n("quantity"),
        schema: Some(n("billing")),
        base: Arc::new(ColumnType::Integer),
    }
}

// [spec:pgorm:req:sql.ddl.domain+3/test]    a create names its domain and its base type and
// nothing else, the base spelled as a column's type is
#[test]
fn a_domain_needs_its_name_and_base() {
    let sql = Domain::create(n("quantity"), ColumnType::Integer).to_string();
    assert_eq!(sql, r#"CREATE DOMAIN "quantity" AS integer"#);
    let create = statement(&sql, "CreateDomainStmt");
    assert_eq!(parts(&create["domainname"]), ["quantity"]);
    assert_eq!(parts(&create["type_name"]["names"]), ["pg_catalog", "int4"]);
    assert!(create.get("coll_clause").is_none(), "{sql}");
    assert!(constraints(&create).is_empty(), "{sql}");

    let sql = Domain::create(
        (n("billing"), n("tags")),
        ColumnType::Array(Arc::new(ColumnType::String(StringLen::N(20)))),
    )
    .to_string();
    assert_eq!(sql, r#"CREATE DOMAIN "billing"."tags" AS varchar(20)[]"#);
    let create = statement(&sql, "CreateDomainStmt");
    assert_eq!(parts(&create["domainname"]), ["billing", "tags"]);
    assert_eq!(
        create["type_name"]["array_bounds"].as_array().map(Vec::len),
        Some(1)
    );
}

// [spec:pgorm:req:sql.ddl.domain+3/test]    the clauses render in one order whatever order they
// were given in — collation, default, nullability, then each check as added — and the
// parser reads each as the constraint it is
#[test]
fn a_create_renders_its_clauses_in_order() {
    let sql = Domain::create((n("billing"), n("code")), ColumnType::Text)
        .check(DomainCheck::new(Expr::domain_value().ne("")).name(n("filled")))
        .not_null()
        .default("none")
        .collate(n("C"))
        .check(Expr::domain_value().like("%_%"))
        .to_string();
    assert_eq!(
        sql,
        [
            r#"CREATE DOMAIN "billing"."code" AS text COLLATE "C" DEFAULT 'none' NOT NULL"#,
            r#"CONSTRAINT "filled" CHECK (VALUE <> '') CHECK (VALUE LIKE '%_%')"#,
        ]
        .join(" ")
    );
    let create = statement(&sql, "CreateDomainStmt");
    assert_eq!(parts(&create["coll_clause"]["collname"]), ["C"]);
    assert_eq!(
        constraints(&create),
        [
            (ConstrType::ConstrDefault as i64, String::new()),
            (ConstrType::ConstrNotnull as i64, String::new()),
            check("filled"),
            check(""),
        ]
    );
}

// [spec:pgorm:req:sql.ddl.domain+3/test]    a default and a nullability are one slot each, the
// last call winning, so the two defaults and the NULL beside a NOT NULL PostgreSQL
// refuses have no rendering
#[test]
fn a_default_and_a_nullability_are_one_slot() {
    let sql = Domain::create(n("d"), ColumnType::Integer)
        .default(1)
        .default(2)
        .not_null()
        .null()
        .to_string();
    assert_eq!(sql, r#"CREATE DOMAIN "d" AS integer DEFAULT 2 NULL"#);
    assert_eq!(
        constraints(&statement(&sql, "CreateDomainStmt")),
        [
            (ConstrType::ConstrDefault as i64, String::new()),
            (ConstrType::ConstrNull as i64, String::new()),
        ]
    );

    let sql = Domain::create(n("d"), ColumnType::Integer)
        .null()
        .not_null()
        .to_string();
    assert_eq!(sql, r#"CREATE DOMAIN "d" AS integer NOT NULL"#);
}

// [spec:pgorm:req:sql.ddl.domain+3/test]    a create's default is an operand of the narrower
// expression the grammar reads there, an operator expression parenthesised and anything
// else bare, while an alter's SET DEFAULT reads the full expression
#[test]
fn a_create_default_is_always_an_operand() {
    let defaults: [(SimpleExpr, &str); 7] = [
        (
            SimpleExpr::from(Expr::val(true)).and(Expr::val(false).into()),
            "(TRUE AND FALSE)",
        ),
        (Expr::val(1).is_null(), "(1 IS NULL)"),
        (Expr::val(2).between(1, 3), "(2 BETWEEN 1 AND 3)"),
        (Expr::val(1).add(2), "(1 + 2)"),
        (Expr::val(-1).into(), "-1"),
        (Expr::current_date().into(), "CURRENT_DATE"),
        (
            Expr::val("a").collate(n("C")).into(),
            r#"('a' COLLATE "C")"#,
        ),
    ];
    for (default, spelled) in defaults {
        let sql = Domain::create(n("d"), ColumnType::Boolean)
            .default(default)
            .to_string();
        assert_eq!(
            sql,
            format!(r#"CREATE DOMAIN "d" AS bool DEFAULT {spelled}"#)
        );
        assert_eq!(
            constraints(&statement(&sql, "CreateDomainStmt")),
            [(ConstrType::ConstrDefault as i64, String::new())],
            "{sql}"
        );
    }

    assert_eq!(
        Domain::alter(n("d"))
            .set_default(SimpleExpr::from(Expr::val(true)).and(Expr::val(false).into()))
            .to_string(),
        r#"ALTER DOMAIN "d" SET DEFAULT TRUE AND FALSE"#
    );
}

// [spec:pgorm:req:sql.ddl.domain+3/test]    `Expr::domain_value` renders the keyword `VALUE`,
// which the parser reads as the name a domain's check substitutes its value for
#[test]
fn domain_value_is_the_keyword() {
    let sql = Domain::create(n("percent"), ColumnType::SmallInteger)
        .check(Expr::domain_value().between(0, 100))
        .to_string();
    assert_eq!(
        sql,
        r#"CREATE DOMAIN "percent" AS smallint CHECK (VALUE BETWEEN 0 AND 100)"#
    );
    let refs = parsed_nodes(&sql, "ColumnRef");
    assert_eq!(refs.len(), 1, "{sql}");
    assert_eq!(parts(&refs[0]["fields"]), ["value"]);
    assert!(matches!(
        SimpleExpr::from(Expr::domain_value()),
        SimpleExpr::Keyword(Keyword::Value)
    ));
}

// [spec:pgorm:req:sql.ddl.domain+3/test]    a domain's name and every constraint name are quoted
// identifiers, their own quotes doubled and their case kept, wherever a statement writes
// them
#[test]
fn domain_names_are_quoted_identifiers() {
    let odd = r#"Qty "x".v2"#;
    let sql = Domain::create((n("Bill"), n(odd)), ColumnType::Integer)
        .check(DomainCheck::new(Expr::domain_value().gt(0)).name(n("Pos \"1\"")))
        .to_string();
    assert_eq!(
        sql,
        r#"CREATE DOMAIN "Bill"."Qty ""x"".v2" AS integer CONSTRAINT "Pos ""1""" CHECK (VALUE > 0)"#
    );
    let create = statement(&sql, "CreateDomainStmt");
    assert_eq!(parts(&create["domainname"]), ["Bill", odd]);
    assert_eq!(constraints(&create), [check("Pos \"1\"")]);

    let sql = Domain::alter(n(odd)).rename_to(n("Amount")).to_string();
    assert_eq!(sql, r#"ALTER DOMAIN "Qty ""x"".v2" RENAME TO "Amount""#);
    let rename = statement(&sql, "RenameStmt");
    assert_eq!(rename["rename_type"], ObjectType::ObjectDomain as i32);
    assert_eq!(parts(&rename["object"]["List"]["items"]), [odd]);
    assert_eq!(rename["newname"], "Amount");

    let sql = Domain::drop(n(odd)).to_string();
    assert_eq!(sql, r#"DROP DOMAIN "Qty ""x"".v2""#);
}

// [spec:pgorm:req:sql.ddl.domain+3/test]    each alter carries exactly one action, read back as
// the parser's subtype for it, a check's NOT VALID as its skipped validation and a
// constraint drop's IF EXISTS and behavior as its own
#[test]
fn an_alter_carries_one_action() {
    let qualified = || Domain::alter((n("billing"), n("quantity")));
    let subtype = |sql: &str| {
        let alter = statement(sql, "AlterDomainStmt");
        assert_eq!(parts(&alter["type_name"]), ["billing", "quantity"], "{sql}");
        alter
    };
    let prefix = r#"ALTER DOMAIN "billing"."quantity""#;

    let sql = qualified().set_default(5).to_string();
    assert_eq!(sql, format!("{prefix} SET DEFAULT 5"));
    let alter = subtype(&sql);
    assert_eq!(alter["subtype"], "T");
    assert!(alter.get("def").is_some(), "{sql}");

    let sql = qualified().drop_default().to_string();
    assert_eq!(sql, format!("{prefix} DROP DEFAULT"));
    let alter = subtype(&sql);
    assert_eq!(alter["subtype"], "T");
    assert!(alter.get("def").is_none(), "{sql}");

    let sql = qualified().set_not_null().to_string();
    assert_eq!(sql, format!("{prefix} SET NOT NULL"));
    assert_eq!(subtype(&sql)["subtype"], "O");

    let sql = qualified().drop_not_null().to_string();
    assert_eq!(sql, format!("{prefix} DROP NOT NULL"));
    assert_eq!(subtype(&sql)["subtype"], "N");

    let sql = qualified()
        .add_check(Expr::domain_value().gt(0))
        .to_string();
    assert_eq!(sql, format!("{prefix} ADD CHECK (VALUE > 0)"));
    let alter = subtype(&sql);
    assert_eq!(alter["subtype"], "C");
    assert_eq!(alter["def"]["Constraint"]["skip_validation"], false);

    let sql = qualified()
        .add_check(
            DomainCheck::new(Expr::domain_value().lt(1000))
                .name(n("small"))
                .not_valid(),
        )
        .to_string();
    assert_eq!(
        sql,
        format!(r#"{prefix} ADD CONSTRAINT "small" CHECK (VALUE < 1000) NOT VALID"#)
    );
    let constraint = &subtype(&sql)["def"]["Constraint"];
    assert_eq!(constraint["conname"], "small");
    assert_eq!(constraint["skip_validation"], true);

    let sql = qualified().drop_constraint(n("small")).to_string();
    assert_eq!(sql, format!(r#"{prefix} DROP CONSTRAINT "small""#));
    let alter = subtype(&sql);
    assert_eq!(alter["subtype"], "X");
    assert_eq!(alter["name"], "small");
    assert_eq!(alter["missing_ok"], false);
    assert_eq!(alter["behavior"], DropBehavior::DropRestrict as i32);

    let sql = qualified()
        .drop_constraint(ConstraintDrop::new(n("small")).if_exists().cascade())
        .to_string();
    assert_eq!(
        sql,
        format!(r#"{prefix} DROP CONSTRAINT IF EXISTS "small" CASCADE"#)
    );
    let alter = subtype(&sql);
    assert_eq!(alter["missing_ok"], true);
    assert_eq!(alter["behavior"], DropBehavior::DropCascade as i32);

    let sql = qualified().validate_constraint(n("small")).to_string();
    assert_eq!(sql, format!(r#"{prefix} VALIDATE CONSTRAINT "small""#));
    let alter = subtype(&sql);
    assert_eq!(alter["subtype"], "V");
    assert_eq!(alter["name"], "small");

    let sql = qualified()
        .rename_constraint(n("small"), n("tiny"))
        .to_string();
    assert_eq!(
        sql,
        format!(r#"{prefix} RENAME CONSTRAINT "small" TO "tiny""#)
    );
    let rename = statement(&sql, "RenameStmt");
    assert_eq!(
        rename["rename_type"],
        ObjectType::ObjectDomconstraint as i32
    );
    assert_eq!(rename["subname"], "small");
    assert_eq!(rename["newname"], "tiny");

    let sql = qualified().rename_to(n("units")).to_string();
    assert_eq!(sql, format!(r#"{prefix} RENAME TO "units""#));
    let rename = statement(&sql, "RenameStmt");
    assert_eq!(rename["rename_type"], ObjectType::ObjectDomain as i32);
    assert_eq!(
        parts(&rename["object"]["List"]["items"]),
        ["billing", "quantity"]
    );
}

// [spec:pgorm:req:sql.ddl.domain+3/test]    a drop names one or more domains in order, with
// IF EXISTS and a behavior, the last of cascade and restrict winning
#[test]
fn a_drop_names_domains_in_order() {
    let sql = Domain::drop(n("a")).to_string();
    assert_eq!(sql, r#"DROP DOMAIN "a""#);
    let drop = statement(&sql, "DropStmt");
    assert_eq!(drop["remove_type"], ObjectType::ObjectDomain as i32);
    assert_eq!(drop["behavior"], DropBehavior::DropRestrict as i32);
    assert_eq!(drop["missing_ok"], false);

    let sql = Domain::drop(n("a"))
        .name((n("billing"), n("b")))
        .if_exists()
        .cascade()
        .restrict()
        .to_string();
    assert_eq!(sql, r#"DROP DOMAIN IF EXISTS "a", "billing"."b" RESTRICT"#);
    let drop = statement(&sql, "DropStmt");
    assert_eq!(drop["missing_ok"], true);
    let names: Vec<Vec<&str>> = drop["objects"]
        .as_array()
        .expect("the dropped names")
        .iter()
        .map(|object| parts(&object["TypeName"]["names"]))
        .collect();
    assert_eq!(names, [vec!["a"], vec!["billing", "b"]]);

    let sql = Domain::drop(n("a")).restrict().cascade().to_string();
    assert_eq!(sql, r#"DROP DOMAIN "a" CASCADE"#);
    assert_eq!(
        statement(&sql, "DropStmt")["behavior"],
        DropBehavior::DropCascade as i32
    );
}

// [spec:pgorm:def:sql.types.column-type+11/test]    a domain column renders its domain's name,
// qualified by its schema, through the part policy every type name takes, alone, as an
// array and as another domain's base; and two domain types are equal by name, schema
// and base
// [spec:pgorm:req:sql.ddl.column-types+7/test]
#[test]
fn a_domain_column_renders_its_name() {
    let sql = Table::create(n("line"))
        .col(ColumnDef::new_with_type(n("qty"), quantity()).not_null())
        .col(ColumnDef::new_with_type(
            n("history"),
            ColumnType::Array(Arc::new(quantity())),
        ))
        .col(ColumnDef::new_with_type(
            n("odd"),
            ColumnType::Domain {
                name: n("Select"),
                schema: None,
                base: Arc::new(ColumnType::Text),
            },
        ))
        .to_string();
    assert_eq!(
        sql,
        [
            r#"CREATE TABLE "line" ( "qty" billing.quantity NOT NULL,"#,
            r#""history" billing.quantity[], "odd" "Select" )"#,
        ]
        .join(" ")
    );

    assert_eq!(
        Domain::create(n("small_quantity"), quantity())
            .check(Expr::domain_value().lt(10))
            .to_string(),
        r#"CREATE DOMAIN "small_quantity" AS billing.quantity CHECK (VALUE < 10)"#
    );

    let over = |base: ColumnType| ColumnType::Domain {
        name: n("quantity"),
        schema: Some(n("billing")),
        base: Arc::new(base),
    };
    assert_eq!(quantity(), over(ColumnType::Integer));
    assert_ne!(quantity(), over(ColumnType::BigInteger));
    assert_ne!(
        quantity(),
        ColumnType::Domain {
            name: n("quantity"),
            schema: None,
            base: Arc::new(ColumnType::Integer),
        }
    );
    assert_ne!(
        quantity(),
        ColumnType::named("quantity"),
        "a named type renders the same DDL and says nothing of the values"
    );
}
