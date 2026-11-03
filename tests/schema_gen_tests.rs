#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, setup::*};

use pgorm::{Error, Schema, entity::prelude::*};
use pgorm_query::{
    ColumnDef, ColumnSpec, ColumnType, IdentityGeneration, QueryBuilder, TableCreateStatement,
    TableKey,
};
use pretty_assertions::assert_eq;

mod factory {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "factory")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod gadget {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "gadget")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(indexed)]
        pub batch: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod widget {
    use pgorm::entity::prelude::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, ActiveEnum)]
    #[pgorm(enum_name = "widget_grade")]
    pub enum Grade {
        #[pgorm(string_value = "Gold")]
        Gold,
        #[pgorm(string_value = "Silver")]
        Silver,
        #[pgorm(string_value = "Bronze")]
        Bronze,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(
        table_name = "widget",
        schema_name = "public",
        comment = "one row per widget"
    )]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(unique)]
        pub code: String,
        #[pgorm(indexed)]
        pub batch: i32,
        pub note: Option<String>,
        #[pgorm(default_value = "unknown", comment = "supplier of record")]
        pub origin: String,
        pub grade: Grade,
        pub spare_grade: Grade,
        pub factory_id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::factory::Entity,
            from = Column::FactoryId,
            to = super::factory::Column::Id
        )]
        Factory,
        #[pgorm(has_many = super::widget_tag::Entity)]
        WidgetTag,
    }
}

mod widget_tag {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "widget_tag")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub widget_id: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub tag: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::widget::Entity,
            from = Column::WidgetId,
            to = super::widget::Column::Id
        )]
        Widget,
    }
}

/// A composite key whose second column the database generates.
mod tenant_row {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "tenant_row")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key, identity)]
        pub id: i64,
        pub note: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A one-column key filled by an identity rather than the serial family.
mod identity_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "identity_key")]
    pub struct Model {
        #[pgorm(primary_key, identity_by_default)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A one-column integer key with a default of its own, which the serial
/// family would collide with.
mod defaulted_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "defaulted_key")]
    pub struct Model {
        #[pgorm(primary_key, default_value = 7)]
        pub id: i32,
        pub note: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A bare (unqualified) entity whose comments contain the one character the
/// comment literal has to escape.
mod quirk {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "quirk", comment = "it's a table")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(comment = "don't drop it")]
        pub note: String,
        pub plain: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// An entity whose only route to a database enum is an array column — the
/// scalar form of `Hue` appears on no column of it.
mod palette {
    use pgorm::entity::prelude::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, ActiveEnum)]
    #[pgorm(enum_name = "palette_hue")]
    pub enum Hue {
        #[pgorm(string_value = "Red")]
        Red,
        #[pgorm(string_value = "Blue")]
        Blue,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "palette")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub hues: Vec<Hue>,
        pub spare_hues: Option<Vec<Hue>>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A table holding two composite unique keys: one named, one left for
/// PostgreSQL to name and treating nulls as equal.
mod roster {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(
        table_name = "roster",
        unique(name = "uq-roster-team-shirt", columns = (Column::Team, Column::Shirt)),
        unique(nulls_not_distinct, columns = (Column::Team, Column::Nickname))
    )]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub team: i32,
        pub shirt: i16,
        pub nickname: Option<String>,
        #[pgorm(unique)]
        pub badge: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// An entity whose names are written by hand, its `EntityName` describing its
/// unique keys too: one over two columns, and one over none, which no table
/// can hold.
mod hand_keyed {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, EntityTrait)]
    pub struct Entity;

    impl EntityName for Entity {
        fn table_name(&self) -> &str {
            "hand_keyed"
        }

        fn unique_keys(&self) -> &'static [pgorm::UniqueKey] {
            &[
                pgorm::UniqueKey {
                    name: Some("uq-hand"),
                    columns: &["a", "b"],
                    nulls_not_distinct: false,
                },
                pgorm::UniqueKey {
                    name: None,
                    columns: &[],
                    nulls_not_distinct: false,
                },
            ]
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub a: i32,
        pub b: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

mod depot {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "depot")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A table whose name is long enough that every name schema generation
/// derives on it runs to 63 bytes or past them, and two of each kind would
/// truncate to one: its composite key, its two foreign keys and its two
/// indexes.
mod parcel {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "parcels_waiting_for_collection_at_every_regional_depot_today")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub tenant: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(indexed)]
        pub origin_depot: i32,
        #[pgorm(indexed)]
        pub destination_depot: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::depot::Entity,
            from = Column::OriginDepot,
            to = super::depot::Column::Id
        )]
        Origin,
        #[pgorm(
            belongs_to = super::depot::Entity,
            from = Column::DestinationDepot,
            to = super::depot::Column::Id
        )]
        Destination,
    }
}

/// An `ActiveEnum` whose `db_type()` is not `ColumnType::Enum`.
#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
#[pgorm(rs_type = String, db_type = ColumnType::String(StringLen::N(1)))]
pub enum Size {
    #[pgorm(string_value = "B")]
    Big,
    #[pgorm(string_value = "S")]
    Small,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Flags {
    not_null: bool,
    auto_increment: bool,
    default: bool,
    identity: Option<IdentityGeneration>,
    comment: Option<String>,
}

fn column<'a>(stmt: &'a TableCreateStatement, name: &str) -> &'a ColumnDef {
    stmt.get_columns()
        .iter()
        .find(|col| col.get_column_name() == name)
        .unwrap_or_else(|| panic!("no column {name} in the projected statement"))
}

fn flags(col: &ColumnDef) -> Flags {
    let mut flags = Flags::default();
    for spec in col.get_column_spec() {
        match spec {
            ColumnSpec::NotNull { .. } => flags.not_null = true,
            ColumnSpec::AutoIncrement => flags.auto_increment = true,
            ColumnSpec::Default(_) => flags.default = true,
            ColumnSpec::Identity(generation, _) => flags.identity = Some(*generation),
            ColumnSpec::Comment(comment) => flags.comment = Some(comment.clone()),
            _ => {}
        }
    }
    flags
}

fn key_columns<K>(key: &TableKey<K>) -> Vec<String> {
    key.get_columns()
        .iter()
        .map(|name| name.to_string())
        .collect()
}

// [spec:pgorm:sem:schema.from-entity+12/test]    table ref, comment, per-column projection, single-column key, unique keys, belongs-to foreign keys
#[test]
fn create_table_from_entity_projects_columns() {
    let schema = Schema::new();
    let stmt = schema
        .create_table_from_entity(widget::Entity)
        .expect("the entity's table is generated");

    // Table ref comes from `entity.table_ref()`, hence schema-qualified.
    assert_eq!(
        stmt.get_table_name(),
        &widget::Entity.table_ref(),
        "the statement targets the entity's table ref"
    );
    assert_eq!(
        stmt.get_comment().map(String::as_str),
        Some("one row per widget")
    );

    // One column per `Column` variant, in declaration order.
    let names: Vec<String> = stmt
        .get_columns()
        .iter()
        .map(|col| col.get_column_name())
        .collect();
    assert_eq!(
        names,
        widget::Column::iter()
            .map(|col| col.to_string())
            .collect::<Vec<_>>()
    );

    // Single-column primary key: the table's key over it, left for PostgreSQL
    // to name, and the serial family on the column.
    assert!(widget::PrimaryKey::auto_increment());
    assert_eq!(
        flags(column(&stmt, "id")),
        Flags {
            not_null: true,
            auto_increment: true,
            ..Default::default()
        }
    );
    let key = stmt.get_primary_key().expect("the entity's key");
    assert_eq!(key_columns(key), ["id"]);
    assert!(key.get_name().is_none(), "a one-column key is not named");

    // Declared column types are carried through.
    assert_eq!(
        column(&stmt, "batch").get_column_type(),
        Some(&ColumnType::Integer)
    );
    assert!(matches!(
        column(&stmt, "code").get_column_type(),
        Some(ColumnType::String(_))
    ));

    // `unique` columns gain a unique key of their own; `indexed` ones gain
    // nothing here.
    let unique: Vec<Vec<String>> = stmt.get_unique_keys().iter().map(key_columns).collect();
    assert_eq!(unique, [["code"]]);

    // NOT NULL unless the column is nullable.
    assert!(flags(column(&stmt, "code")).not_null);
    assert!(!flags(column(&stmt, "note")).not_null);

    // Default value and column comment ride along.
    let origin = flags(column(&stmt, "origin"));
    assert!(origin.default);
    assert_eq!(origin.comment.as_deref(), Some("supplier of record"));
    assert_eq!(flags(column(&stmt, "code")).comment, None);

    // `ColumnType::Enum` is carried through as itself: the renderer spells the
    // type from the enum's own identity, so nothing has to pre-flatten it into
    // a name-carrying `Custom` first.
    for name in ["grade", "spare_grade"] {
        match column(&stmt, name).get_column_type() {
            Some(ColumnType::Enum { name: ty, .. }) => assert_eq!(ty.to_string(), "widget_grade"),
            other => panic!("expected the enum type for {name}, got {other:?}"),
        }
    }
    assert!(matches!(
        widget::Column::Grade.def().get_column_type(),
        ColumnType::Enum { .. }
    ));

    // Foreign keys come from the belongs-to side only.
    let fks = stmt.get_foreign_key_create_stmts();
    assert_eq!(
        fks.len(),
        1,
        "only the belongs-to relation produces a constraint: {:?}",
        fks.iter().map(|fk| fk.to_string()).collect::<Vec<_>>()
    );
    let fk = fks[0].to_string();
    assert!(fk.contains("\"factory_id\""), "{fk}");
    assert!(fk.contains("\"factory\""), "{fk}");
    assert!(
        !fk.contains("widget_tag"),
        "the has_many (owner) side produces nothing: {fk}"
    );
    assert!(widget::Relation::WidgetTag.def().is_owner);
    assert!(!widget::Relation::Factory.def().is_owner);
}

// [spec:pgorm:sem:schema.from-entity+12/test]    a composite key is the table's key, named pk-{table}
#[test]
fn create_table_composite_key_emits_index() {
    let schema = Schema::new();
    let stmt = schema
        .create_table_from_entity(widget_tag::Entity)
        .expect("the entity's table is generated");

    for name in ["widget_id", "tag"] {
        assert!(!flags(column(&stmt, name)).auto_increment);
    }

    let key = stmt.get_primary_key().expect("the entity's key");
    assert_eq!(key_columns(key), ["widget_id", "tag"]);
    assert_eq!(
        key.get_name().map(|name| name.to_string()).as_deref(),
        Some("pk-widget_tag")
    );
    let rendered = stmt.to_string();
    assert!(rendered.contains("\"pk-widget_tag\""), "{rendered}");
    assert!(rendered.contains("PRIMARY KEY"), "{rendered}");
    assert!(
        rendered.contains("\"widget_id\", \"tag\""),
        "both key columns are in the table-level index: {rendered}"
    );
}

// [spec:pgorm:sem:schema.from-entity+12/test]    the entity comment first, then the commented
// columns in Column order, targeting entity.table_ref() with the text quoted
#[test]
fn create_comments_from_entity_emits_statements() {
    let schema = Schema::new();

    let rendered: Vec<String> = schema
        .create_comments_from_entity(widget::Entity)
        .iter()
        .map(|stmt| stmt.to_string())
        .collect();
    assert_eq!(
        rendered,
        vec![
            r#"COMMENT ON TABLE "public"."widget" IS 'one row per widget'"#.to_owned(),
            r#"COMMENT ON COLUMN "public"."widget"."origin" IS 'supplier of record'"#.to_owned(),
        ],
        "comments target the same qualified name the table projection uses"
    );

    // The text rides on the create statement too, and renders off it: the
    // create's own SQL is one preparable command, so the comments come out
    // beside it as the same statements this projection builds.
    let table = schema
        .create_table_from_entity(widget::Entity)
        .expect("the entity's table is generated");
    assert_eq!(
        table.get_comment().map(String::as_str),
        Some("one row per widget")
    );
    assert_eq!(
        table
            .comments()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        rendered,
        "both projections of one entity's comments agree"
    );
    let create = table.to_string();
    assert!(!create.contains("one row per widget"), "{create}");
    assert!(!create.contains("supplier of record"), "{create}");

    // An entity with no schema stays unqualified, and a quote in the text is doubled.
    let rendered: Vec<String> = schema
        .create_comments_from_entity(quirk::Entity)
        .iter()
        .map(|stmt| stmt.to_string())
        .collect();
    assert_eq!(
        rendered,
        vec![
            r#"COMMENT ON TABLE "quirk" IS 'it''s a table'"#.to_owned(),
            r#"COMMENT ON COLUMN "quirk"."note" IS 'don''t drop it'"#.to_owned(),
        ]
    );

    assert!(
        schema
            .create_comments_from_entity(factory::Entity)
            .is_empty(),
        "an entity that declares no comment produces no statements"
    );
}

// [spec:pgorm:sem:schema.from-entity.index+2/test]    one statement per indexed column, named idx-{table}-{column}
#[test]
fn create_index_from_entity_names_indexed_columns() {
    let schema = Schema::new();

    let stmts = schema.create_index_from_entity(widget::Entity);
    assert_eq!(stmts.len(), 1, "only `batch` is flagged `indexed`");
    assert_eq!(
        stmts[0].to_string(),
        r#"CREATE INDEX "idx-widget-batch" ON "public"."widget" ("batch")"#
    );

    // Unique columns are not covered here: `code` gets a unique key from the
    // table projection, not an index statement of its own.
    let table = schema
        .create_table_from_entity(widget::Entity)
        .expect("the entity's table is generated");
    assert_eq!(key_columns(&table.get_unique_keys()[0]), ["code"]);
    assert!(
        !stmts.iter().any(|stmt| stmt.to_string().contains("code")),
        "uniqueness is not emitted as a separate index"
    );

    // No indexed column at all yields an empty Vec.
    assert!(
        schema.create_index_from_entity(factory::Entity).is_empty(),
        "an entity with no indexed column produces no statements"
    );
}

// [spec:pgorm:sem:schema.from-entity.index+2/test]    the index targets entity.table_ref(), so it is schema-qualified exactly when the entity declares one
#[test]
fn create_index_from_entity_uses_table_ref() {
    let schema = Schema::new();

    assert_eq!(widget::Entity.schema_name(), Some("public"));
    let qualified = schema.create_index_from_entity(widget::Entity);
    assert_eq!(
        qualified[0].to_string(),
        r#"CREATE INDEX "idx-widget-batch" ON "public"."widget" ("batch")"#
    );
    let table = schema
        .create_table_from_entity(widget::Entity)
        .expect("the entity's table is generated");
    assert_eq!(
        table.get_table_name(),
        &widget::Entity.table_ref(),
        "index and table projections agree on the target ref"
    );

    assert_eq!(gadget::Entity.schema_name(), None);
    let bare = schema.create_index_from_entity(gadget::Entity);
    assert_eq!(
        bare[0].to_string(),
        r#"CREATE INDEX "idx-gadget-batch" ON "gadget" ("batch")"#
    );
}

// [spec:pgorm:sem:schema.from-entity.enum+3/test]    one statement per resolved enum,
// variant order preserved, deduplicated by type name across the entity's columns
#[test]
fn create_enum_from_entity_dedups_by_type_name() {
    let schema = Schema::new();

    let stmts = schema.create_enum_from_entity(widget::Entity);
    let rendered: Vec<String> = stmts.iter().map(|stmt| stmt.to_string()).collect();
    assert_eq!(
        rendered,
        vec![r#"CREATE TYPE "widget_grade" AS ENUM ('Gold', 'Silver', 'Bronze')"#.to_owned()],
        "grade and spare_grade share one enum type, so it is created once"
    );

    assert!(
        schema.create_enum_from_entity(factory::Entity).is_empty(),
        "an entity with no enum column produces no statements"
    );
}

// [spec:pgorm:sem:schema.from-entity.enum+3/test]    an enum reached only through an
// array column is resolved through the array, and deduplicated the same way
#[test]
fn create_enum_from_entity_looks_through_arrays() {
    let schema = Schema::new();

    assert!(matches!(
        palette::Column::Hues.def().get_column_type(),
        ColumnType::Array(_)
    ));
    let rendered: Vec<String> = schema
        .create_enum_from_entity(palette::Entity)
        .iter()
        .map(|stmt| stmt.to_string())
        .collect();
    assert_eq!(
        rendered,
        vec![r#"CREATE TYPE "palette_hue" AS ENUM ('Red', 'Blue')"#.to_owned()],
        "the enum both array columns are over is created once"
    );
}

// [spec:pgorm:sem:schema.from-entity.enum+3/test]    the single-ActiveEnum form builds the same statement from A::db_type()
#[test]
fn create_enum_from_active_enum_uses_db_type() {
    let schema = Schema::new();

    assert_eq!(
        schema
            .create_enum_from_active_enum::<widget::Grade>()
            .expect("Grade resolves to ColumnType::Enum")
            .to_string(),
        r#"CREATE TYPE "widget_grade" AS ENUM ('Gold', 'Silver', 'Bronze')"#
    );
    assert!(matches!(
        widget::Grade::db_type().get_column_type(),
        ColumnType::Enum { .. }
    ));
}

// [spec:pgorm:sem:schema.from-entity.enum+3/test]    errors when the resolved column type is not an enum
#[test]
fn create_enum_from_active_enum_errs_non_enum() {
    assert!(!matches!(
        Size::db_type().get_column_type(),
        ColumnType::Enum { .. }
    ));
    let err = Schema::new()
        .create_enum_from_active_enum::<Size>()
        .expect_err("a String-backed ActiveEnum has no type to create");
    assert!(
        matches!(err, Error::Type(ref msg) if msg.contains("not backed by a database enum")),
        "expected a type error naming the enum, got {err:?}"
    );
}

// [spec:pgorm:sem:schema.from-entity+12/test]    the projected DDL is accepted by Postgres and enforces what it declares
// [spec:pgorm:sem:schema.from-entity.index+2/test]    the schema-qualified index executes and reaches pg_indexes under its generated name
// [spec:pgorm:sem:schema.from-entity.enum+3/test]    the projected type is a usable Postgres enum
#[pgorm_macros::test]
async fn generated_schema_executes_on_postgres() -> Result<(), Error> {
    let ctx = TestContext::new("schema_gen_executes_schemagen").await;
    let db = ctx.db.get().await?;
    let schema = Schema::new();

    // Deduplicated by type name, so the whole stream executes: Postgres has no
    // `CREATE TYPE IF NOT EXISTS` to fall back on.
    let enums = schema.create_enum_from_entity(widget::Entity);
    assert_eq!(enums.len(), 1);
    for stmt in &enums {
        db.execute(&stmt.to_string(), &[]).await?;
    }

    for entity_stmt in [
        schema.create_table_from_entity(factory::Entity)?,
        schema.create_table_from_entity(widget::Entity)?,
        schema.create_table_from_entity(widget_tag::Entity)?,
    ] {
        db.execute(&entity_stmt.to_string(), &[]).await?;
    }
    for index in schema.create_index_from_entity(widget::Entity) {
        let sql = index.to_string();
        assert!(sql.contains(r#"ON "public"."widget""#), "{sql}");
        db.execute(&sql, &[]).await?;
    }

    let indexes: Vec<String> = db
        .query_all(
            "SELECT indexname FROM pg_indexes WHERE schemaname = 'public' AND tablename = 'widget'",
            &[],
        )
        .await?
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect();
    assert!(
        indexes.contains(&"idx-widget-batch".to_owned()),
        "the generated index is on the schema-qualified table: {indexes:?}"
    );

    for comment in schema.create_comments_from_entity(widget::Entity) {
        db.execute(&comment.to_string(), &[]).await?;
    }
    let table_comment: Option<String> = db
        .query_one(
            "SELECT obj_description('public.widget'::regclass, 'pg_class')",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(
        table_comment.as_deref(),
        Some("one row per widget"),
        "the qualified comment target resolves to the same table"
    );

    let factory = factory::ActiveModel {
        name: set("Acme"),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    // `origin` is omitted, so the DDL default has to supply it; `note` is nullable.
    let widget = widget::ActiveModel {
        code: set("W-1"),
        batch: set(7),
        grade: set(widget::Grade::Gold),
        spare_grade: set(widget::Grade::Bronze),
        factory_id: set(factory.id),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    assert_eq!(widget.origin, "unknown");
    assert_eq!(widget.note, None);
    assert_eq!(widget.grade, widget::Grade::Gold);

    // The unique key on `code` is enforced.
    let unique = widget::ActiveModel {
        code: set("W-1"),
        batch: set(8),
        grade: set(widget::Grade::Silver),
        spare_grade: set(widget::Grade::Silver),
        factory_id: set(factory.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect_err("code is unique");
    assert!(matches!(
        unique.sql_error(),
        Some(pgorm::SqlError::UniqueConstraintViolation(_))
    ));

    // The belongs-to foreign key is enforced.
    let fk = widget::ActiveModel {
        code: set("W-2"),
        batch: set(9),
        grade: set(widget::Grade::Silver),
        spare_grade: set(widget::Grade::Silver),
        factory_id: set(factory.id + 1_000),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect_err("factory_id has no matching factory");
    assert!(matches!(
        fk.sql_error(),
        Some(pgorm::SqlError::ForeignKeyConstraintViolation(_))
    ));

    // The composite primary key is enforced across both columns.
    let tag = widget_tag::ActiveModel {
        widget_id: set(widget.id),
        tag: set("shiny"),
    };
    tag.clone().insert(&db).await?;
    widget_tag::ActiveModel {
        widget_id: set(widget.id),
        tag: set("matte"),
    }
    .insert(&db)
    .await?;
    let composite = tag
        .insert(&db)
        .await
        .expect_err("(widget_id, tag) is the primary key");
    assert!(matches!(
        composite.sql_error(),
        Some(pgorm::SqlError::UniqueConstraintViolation(_))
    ));

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:schema.from-entity.enum+3/test]    an entity reaching an enum only
// through an array column carries the CREATE TYPE its own table needs: on a fresh
// database the two statement streams are sufficient on their own
#[pgorm_macros::test]
async fn array_only_enum_schema_executes_on_postgres() -> Result<(), Error> {
    let ctx = TestContext::new("schema_gen_array_enum_schemagen").await;
    let db = ctx.db.get().await?;
    let schema = Schema::new();

    for stmt in schema.create_enum_from_entity(palette::Entity) {
        db.execute(&stmt.to_string(), &[]).await?;
    }
    let table = schema
        .create_table_from_entity(palette::Entity)?
        .to_string();
    assert!(table.contains(r#""hues" palette_hue[]"#), "{table}");
    db.execute(&table, &[]).await?;

    let row = palette::ActiveModel {
        hues: set(vec![palette::Hue::Red, palette::Hue::Blue]),
        spare_hues: set(Some(vec![palette::Hue::Blue])),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    assert_eq!(row.hues, [palette::Hue::Red, palette::Hue::Blue]);
    assert_eq!(row.spare_hues, Some(vec![palette::Hue::Blue]));

    let stored: String = db
        .query_one(
            "SELECT format_type(atttypid, atttypmod) FROM pg_attribute \
             WHERE attrelid = 'palette'::regclass AND attname = 'hues'",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(stored, "palette_hue[]");

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:schema.from-entity+12/test]    a composite key's generated
// column carries its identity, the supplied column nothing, and neither the
// serial family nor an inline PRIMARY KEY
#[test]
fn composite_key_identity_lands_on_its_column() {
    let stmt = Schema::new()
        .create_table_from_entity(tenant_row::Entity)
        .expect("the entity's table is generated");
    assert_eq!(
        flags(column(&stmt, "tenant_id")),
        Flags {
            not_null: true,
            ..Default::default()
        }
    );
    assert_eq!(
        flags(column(&stmt, "id")),
        Flags {
            not_null: true,
            identity: Some(IdentityGeneration::Always),
            ..Default::default()
        }
    );
    assert_eq!(
        stmt.to_string(),
        [
            r#"CREATE TABLE "tenant_row" ( "tenant_id" integer NOT NULL,"#,
            r#""id" bigint NOT NULL GENERATED ALWAYS AS IDENTITY,"#,
            r#""note" varchar NOT NULL,"#,
            r#"CONSTRAINT "pk-tenant_row" PRIMARY KEY ("tenant_id", "id") )"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:schema.from-entity+12/test]    a one-column key that fills
// itself — an identity or a default of its own — is not also drawn from the
// serial family, which PostgreSQL refuses beside either (42601)
#[test]
fn key_with_its_own_fill_is_not_serial() {
    let identity = Schema::new()
        .create_table_from_entity(identity_key::Entity)
        .expect("the entity's table is generated");
    assert!(identity_key::PrimaryKey::auto_increment());
    assert_eq!(
        flags(column(&identity, "id")),
        Flags {
            not_null: true,
            identity: Some(IdentityGeneration::ByDefault),
            ..Default::default()
        }
    );
    assert_eq!(
        key_columns(identity.get_primary_key().expect("a key")),
        ["id"]
    );

    let defaulted = Schema::new()
        .create_table_from_entity(defaulted_key::Entity)
        .expect("the entity's table is generated");
    assert!(defaulted_key::PrimaryKey::auto_increment());
    assert_eq!(
        flags(column(&defaulted, "id")),
        Flags {
            not_null: true,
            default: true,
            ..Default::default()
        }
    );
    assert_eq!(
        key_columns(defaulted.get_primary_key().expect("a key")),
        ["id"]
    );
}

// [spec:pgorm:sem:schema.from-entity+12/test]    the tables a key that fills
// itself projects to are ones the server builds, and fill the key unasked
#[pgorm_macros::test]
async fn self_filling_keys_execute_on_postgres() -> Result<(), Error> {
    let ctx = TestContext::new("schema_gen_self_filling_keys").await;
    let db = ctx.db.get().await?;
    let schema = Schema::new();

    for stmt in [
        schema.create_table_from_entity(tenant_row::Entity)?,
        schema.create_table_from_entity(identity_key::Entity)?,
        schema.create_table_from_entity(defaulted_key::Entity)?,
    ] {
        db.execute(&stmt.to_string(), &[]).await?;
    }

    let row: (i32, i64) = db
        .query_one(
            r#"INSERT INTO "tenant_row" ("tenant_id", "note") VALUES (5, 'n') RETURNING "tenant_id", "id""#,
            &[],
        )
        .await
        .map(|row| (row.get(0), row.get(1)))?;
    assert_eq!(row, (5, 1));
    let id: i32 = db
        .query_one(
            r#"INSERT INTO "identity_key" DEFAULT VALUES RETURNING "id""#,
            &[],
        )
        .await?
        .get(0);
    assert_eq!(id, 1);
    let id: i32 = db
        .query_one(
            r#"INSERT INTO "defaulted_key" ("note") VALUES ('n') RETURNING "id""#,
            &[],
        )
        .await?
        .get(0);
    assert_eq!(id, 7);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:schema.from-entity+12/test]    the comment statements execute, and only they
// attach anything: the text arrives in pg_description exactly as declared
#[pgorm_macros::test]
async fn entity_comments_land_in_pg_description() -> Result<(), Error> {
    const TABLE_COMMENT: &str = "SELECT obj_description('quirk'::regclass, 'pg_class')";
    const COLUMN_COMMENT: &str = "SELECT col_description('quirk'::regclass, attnum) \
         FROM pg_attribute WHERE attrelid = 'quirk'::regclass AND attname = 'note'";

    let ctx = TestContext::new("schema_gen_comments_schemagen").await;
    let db = ctx.db.get().await?;
    let schema = Schema::new();

    db.execute(
        &schema.create_table_from_entity(quirk::Entity)?.to_string(),
        &[],
    )
    .await?;

    let after_create: Option<String> = db.query_one(TABLE_COMMENT, &[]).await?.get(0);
    assert_eq!(
        after_create, None,
        "creating the table attaches no comment on its own"
    );
    let after_create: Option<String> = db.query_one(COLUMN_COMMENT, &[]).await?.get(0);
    assert_eq!(after_create, None);

    for comment in schema.create_comments_from_entity(quirk::Entity) {
        db.execute(&comment.to_string(), &[]).await?;
    }

    let table_comment: Option<String> = db.query_one(TABLE_COMMENT, &[]).await?.get(0);
    assert_eq!(
        table_comment.as_deref(),
        Some("it's a table"),
        "the doubled quote round-trips as one quote"
    );
    let column_comment: Option<String> = db.query_one(COLUMN_COMMENT, &[]).await?.get(0);
    assert_eq!(column_comment.as_deref(), Some("don't drop it"));

    drop(db);
    ctx.delete().await;

    Ok(())
}

const PARCEL: &str = "parcels_waiting_for_collection_at_every_regional_depot_today";

/// The names generation derives on `parcel`, each bounded as a composed
/// result name is: its key, its two foreign keys, then its two indexes.
fn parcel_names() -> [String; 5] {
    [
        format!("pk-{PARCEL}"),
        format!("fk-{PARCEL}-origin_depot"),
        format!("fk-{PARCEL}-destination_depot"),
        format!("idx-{PARCEL}-origin_depot"),
        format!("idx-{PARCEL}-destination_depot"),
    ]
    .map(|full| pgorm::result_column_name("", &full))
}

// [spec:pgorm:sem:schema.from-entity+12/test]    a derived key or foreign key name that would reach 63 bytes is bounded as composed result names are, its head kept, so two long names stay two
// [spec:pgorm:sem:schema.from-entity.index+2/test]
// [spec:pgorm:req:entity.relation.fk+6/test]
#[test]
fn derived_names_past_the_bound_are_bounded() {
    let names = parcel_names();
    for name in &names {
        assert_eq!(name.len(), 63, "{name}");
    }
    assert!(names[0].starts_with(&format!("pk-{PARCEL}")[..47]));
    assert_eq!(names[1][..47], names[2][..47]);
    assert_ne!(names[1], names[2]);

    let schema = Schema::new();
    let table = schema
        .create_table_from_entity(parcel::Entity)
        .expect("the table is generated")
        .to_string();
    for name in &names[..3] {
        assert!(
            table.contains(&format!(r#"CONSTRAINT "{name}""#)),
            "{name} in {table}"
        );
    }
    let indexes: Vec<String> = schema
        .create_index_from_entity(parcel::Entity)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        indexes,
        [
            format!(
                r#"CREATE INDEX "{}" ON "{PARCEL}" ("origin_depot")"#,
                names[3]
            ),
            format!(
                r#"CREATE INDEX "{}" ON "{PARCEL}" ("destination_depot")"#,
                names[4]
            ),
        ]
    );

    let short = schema
        .create_table_from_entity(widget_tag::Entity)
        .expect("the table is generated")
        .to_string();
    assert!(
        short.contains(r#"CONSTRAINT "fk-widget_tag-widget_id""#),
        "{short}"
    );
}

// [spec:pgorm:sem:schema.from-entity+12/test]    the server creates every derived constraint and index under exactly the name generation gives it, none truncated and none colliding
// [spec:pgorm:sem:schema.from-entity.index+2/test]
#[pgorm_macros::test]
async fn bounded_derived_names_are_the_servers_live() -> Result<(), Error> {
    let ctx = TestContext::new("schema_gen_bounded_names").await;
    let db = ctx.db.get().await?;
    let schema = Schema::new();
    db.execute(
        &schema.create_table_from_entity(depot::Entity)?.to_string(),
        &[],
    )
    .await?;
    db.execute(
        &schema.create_table_from_entity(parcel::Entity)?.to_string(),
        &[],
    )
    .await?;
    for index in schema.create_index_from_entity(parcel::Entity) {
        db.execute(&index.to_string(), &[]).await?;
    }

    let [key, origin, destination, origin_index, destination_index] = parcel_names();
    let mut constraints: Vec<String> = db
        .query_all(
            "SELECT conname::text FROM pg_constraint WHERE conrelid = $1::text::regclass \
             AND contype IN ('p', 'f')",
            &[&format!("\"{PARCEL}\"")],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    constraints.sort();
    let mut expected = vec![key, origin, destination];
    expected.sort();
    assert_eq!(constraints, expected);
    let mut indexes: Vec<String> = db
        .query_all(
            "SELECT indexname::text FROM pg_indexes WHERE tablename = $1 \
             AND indexname NOT IN (SELECT conname FROM pg_constraint)",
            &[&PARCEL],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    indexes.sort();
    let mut expected = vec![origin_index, destination_index];
    expected.sort();
    assert_eq!(indexes, expected);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:schema.from-entity+12/test]    an entity's composite unique keys are written after its columns' own, in declaration order, under the name each declares or none
// [spec:pgorm:req:entity.traits.entity-name+5/test]
// [spec:pgorm:syn:macros.derive.entity-model.attrs+9/test]
#[test]
fn composite_unique_keys_are_written() {
    let table = Schema::new()
        .create_table_from_entity(roster::Entity)
        .expect("the table is generated")
        .to_string();
    assert_eq!(
        table,
        [
            r#"CREATE TABLE "roster" ("#,
            r#""id" serial NOT NULL,"#,
            r#""team" integer NOT NULL,"#,
            r#""shirt" smallint NOT NULL,"#,
            r#""nickname" varchar,"#,
            r#""badge" integer NOT NULL,"#,
            r#"PRIMARY KEY ("id"),"#,
            r#"UNIQUE ("badge"),"#,
            r#"CONSTRAINT "uq-roster-team-shirt" UNIQUE ("team", "shirt"),"#,
            r#"UNIQUE NULLS NOT DISTINCT ("team", "nickname")"#,
            r#")"#,
        ]
        .join(" ")
    );
    assert_eq!(roster::Entity.unique_keys().len(), 2);
    assert!(gadget::Entity.unique_keys().is_empty());
}

// [spec:pgorm:sem:schema.from-entity+12/test]    a unique key an `EntityName` written by hand describes with no columns is refused as `Error::Type` naming the entity, since PostgreSQL cannot write `UNIQUE ()`
// [spec:pgorm:req:entity.traits.entity-name+5/test]
#[test]
fn a_unique_key_of_no_columns_is_refused() {
    match Schema::new().create_table_from_entity(hand_keyed::Entity) {
        Err(Error::Type(message)) => {
            assert_eq!(
                message,
                "`hand_keyed` declares a unique key with no columns"
            )
        }
        other => panic!("expected the table refused, got {other:?}"),
    }
}

// [spec:pgorm:sem:schema.from-entity+12/test]    the server holds each composite unique key as declared: a repeated pair is refused under the declared name, and under the key treating nulls as equal two null nicknames in one team are too
#[pgorm_macros::test]
async fn composite_unique_keys_hold_live() -> Result<(), Error> {
    use pgorm::set;
    use tokio_postgres::error::SqlState;

    let ctx = TestContext::new("schema_gen_composite_unique").await;
    let db = ctx.db.get().await?;
    db.execute(
        &Schema::new()
            .create_table_from_entity(roster::Entity)?
            .to_string(),
        &[],
    )
    .await?;
    let player = |team: i32, shirt: i16, nickname: Option<&str>, badge: i32| roster::ActiveModel {
        team: set(team),
        shirt: set(shirt),
        nickname: set(nickname.map(str::to_owned)),
        badge: set(badge),
        ..Default::default()
    };
    let refused_by = |outcome: Result<roster::Model, Error>| match outcome {
        Err(Error::Postgres(error)) => {
            assert_eq!(error.code(), Some(&SqlState::UNIQUE_VIOLATION), "{error}");
            error
                .as_db_error()
                .and_then(|db| db.constraint())
                .map(str::to_owned)
        }
        other => panic!("expected a unique violation, got {other:?}"),
    };
    player(1, 9, None, 1).insert(&db).await?;
    player(2, 9, None, 2).insert(&db).await?;
    player(1, 10, Some("ace"), 3).insert(&db).await?;
    assert_eq!(
        refused_by(player(1, 9, Some("twin"), 4).insert(&db).await),
        Some("uq-roster-team-shirt".to_owned())
    );
    assert_eq!(
        refused_by(player(1, 11, None, 5).insert(&db).await),
        Some("roster_team_nickname_key".to_owned())
    );
    player(3, 11, Some("ace"), 6).insert(&db).await?;
    assert_eq!(
        refused_by(player(3, 12, Some("ace"), 7).insert(&db).await),
        Some("roster_team_nickname_key".to_owned())
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
