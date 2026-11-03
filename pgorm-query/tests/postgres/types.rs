use super::*;
use crate::oracle::{assert_eq, assert_eq_unparsed};
use pgorm_query::extension::Type;

// [spec:pgorm:req:sql.ddl.type-enum+7/test]
#[test]
// [spec:pgorm:req:sql.render.ddl.enum-type+6/test]
fn create_1() {
    assert_eq!(
        Type::create(Font::Table)
            .values(["name", "variant", "language"])
            .to_string(),
        r#"CREATE TYPE "font" AS ENUM ('name', 'variant', 'language')"#
    );
}

#[test]
fn create_2() {
    assert_eq!(
        Type::create((Name::runtime("schema"), Font::Table))
            .values(["name", "variant", "language"])
            .to_string(),
        r#"CREATE TYPE "schema"."font" AS ENUM ('name', 'variant', 'language')"#
    );
}

#[test]
fn create_3() {
    assert_eq!(
        Type::create(Tea::Enum)
            .values(["EverydayTea", "BreakfastTea"])
            .to_string(),
        r#"CREATE TYPE "tea" AS ENUM ('EverydayTea', 'BreakfastTea')"#
    );

    // The type is named by an `SqlName`; its labels are data and are written as
    // the string literals they render to.
    enum Tea {
        Enum,
    }

    impl pgorm_query::SqlName for Tea {
        fn unquoted(&self, s: &mut dyn std::fmt::Write) {
            match self {
                Self::Enum => write!(s, "tea").unwrap(),
            }
        }
    }
}

// [spec:pgorm:req:sql.ddl.type-alter-drop+7/test]
#[test]
fn drop_1() {
    assert_eq!(
        Type::drop(Font::Table).if_exists().restrict().to_string(),
        r#"DROP TYPE IF EXISTS "font" RESTRICT"#
    )
}

#[test]
fn drop_2() {
    assert_eq!(Type::drop(Font::Table).to_string(), r#"DROP TYPE "font""#);
}

#[test]
fn drop_3() {
    assert_eq!(
        Type::drop(Font::Table).if_exists().cascade().to_string(),
        r#"DROP TYPE IF EXISTS "font" CASCADE"#
    );
}

#[test]
fn drop_4() {
    assert_eq!(
        Type::drop((Name::runtime("schema"), Font::Table)).to_string(),
        r#"DROP TYPE "schema"."font""#
    );
}

// [spec:pgorm:req:sql.ddl.type-alter-drop+7/test]
#[test]
fn alter_1() {
    assert_eq!(
        Type::alter(Font::Table).add_value("weight").to_string(),
        r#"ALTER TYPE "font" ADD VALUE 'weight'"#
    )
}
#[test]
fn alter_2() {
    assert_eq!(
        Type::alter(Font::Table)
            .add_value("weight")
            .before("variant")
            .to_string(),
        r#"ALTER TYPE "font" ADD VALUE 'weight' BEFORE 'variant'"#
    )
}

#[test]
fn alter_3() {
    assert_eq!(
        Type::alter(Font::Table)
            .add_value("weight")
            .after("variant")
            .to_string(),
        r#"ALTER TYPE "font" ADD VALUE 'weight' AFTER 'variant'"#
    )
}

#[test]
fn alter_4() {
    assert_eq!(
        Type::alter(Font::Table)
            .rename_to(Name::runtime("typeface"))
            .to_string(),
        r#"ALTER TYPE "font" RENAME TO "typeface""#
    )
}

#[test]
fn alter_5() {
    assert_eq!(
        Type::alter(Font::Table)
            .rename_value("variant", "language")
            .to_string(),
        r#"ALTER TYPE "font" RENAME VALUE 'variant' TO 'language'"#
    )
}

#[test]
fn alter_6() {
    assert_eq!(
        Type::alter((Name::runtime("schema"), Font::Table))
            .rename_to(Name::runtime("typeface"))
            .to_string(),
        r#"ALTER TYPE "schema"."font" RENAME TO "typeface""#
    )
}

// [spec:pgorm:def:sql.types+9/test]    equality is the concrete type and the rendered text,
// both asked of values Rust never promised to place at one vtable address
#[test]
fn identifier_equality_is_type_and_text() {
    struct Mine;
    struct Yours;

    impl SqlName for Mine {
        fn unquoted(&self, s: &mut dyn std::fmt::Write) {
            write!(s, "same").unwrap();
        }
    }
    impl SqlName for Yours {
        fn unquoted(&self, s: &mut dyn std::fmt::Write) {
            write!(s, "same").unwrap();
        }
    }

    // Two types rendering one text are two identifiers.
    assert_eq!(Mine.to_string(), Yours.to_string());
    assert_ne!(Mine.into_name(), Yours.into_name());

    // One type rendering one text is one identifier, however the values
    // reached the trait object.
    assert_eq!(Mine.into_name(), Mine.into_name());
    assert_eq!(Name::runtime("same"), Name::runtime("same"));
    assert_eq!(Name::runtime("same"), Name::runtime("same"));
    assert_eq!(Name::runtime("same"), Name::runtime("same").clone());

    // One type rendering two texts is two identifiers.
    assert_ne!(Name::runtime("same"), Name::runtime("other"));
}

// [spec:pgorm:req:sql.ddl+12/test]    the two type statements that bind expose `build()`, and its
// pair is the SQL with `$N` placeholders plus the labels in emission order
#[test]
fn the_label_binding_type_statements_build() {
    let (sql, values) = Type::create(Font::Table)
        .values(["name", "variant"])
        .build();
    assert_eq_unparsed!(sql, r#"CREATE TYPE "font" AS ENUM ($1, $2)"#);
    assert_eq_unparsed!(
        values,
        Values(vec![Value::from("name"), Value::from("variant")])
    );

    let (sql, values) = Type::alter(Font::Table)
        .add_value("weight")
        .before("variant")
        .build();
    assert_eq_unparsed!(sql, r#"ALTER TYPE "font" ADD VALUE $1 BEFORE $2"#);
    assert_eq_unparsed!(
        values,
        Values(vec![Value::from("weight"), Value::from("variant")])
    );

    // `RENAME TO` names a type rather than a label, so it stays an identifier
    // in the bound rendering and contributes no value.
    let (sql, values) = Type::alter(Font::Table)
        .rename_to(Name::runtime("typeface"))
        .build();
    assert_eq_unparsed!(sql, r#"ALTER TYPE "font" RENAME TO "typeface""#);
    assert_eq_unparsed!(values, Values(vec![]));

    // The inlined rendering is the one PostgreSQL accepts, and the oracle
    // holds it to the grammar; the `$N` form above is deliberately not run
    // through it, because no DDL takes a bind parameter.
    assert_eq!(
        Type::create(Font::Table)
            .values(["name", "variant"])
            .to_string(),
        r#"CREATE TYPE "font" AS ENUM ('name', 'variant')"#
    );
}
