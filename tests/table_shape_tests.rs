#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, setup::*};

use pgorm::{
    ColumnPairs, ColumnTypeTrait, ConnectionTrait, Error, RelationDef, Schema, TableColumn,
    TableShape, alias,
    entity::prelude::*,
    pgorm_query::{ColumnType, Expr, ForeignKeyAction, GeneratedKind, Name, RangeType, TableName},
    tests_cfg::{cake, cake_filling_price, indexes},
};
use pretty_assertions::assert_eq;

mod noted {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(
        table_name = "noted",
        schema_name = "public",
        comment = "a noted row",
        unique(name = "noted_pair", columns = (Column::Left, Column::Right))
    )]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(indexed, comment = "the left half")]
        pub left: i32,
        pub right: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

fn table(name: &str) -> TableName {
    TableName::Table(Name::runtime(name))
}

/// Every statement each of `shape`'s four writers gives, as text.
fn statements(shape: &TableShape) -> Result<Vec<String>, Error> {
    let schema = Schema::new();
    let mut texts = vec![schema.create_table_from_shape(shape)?.to_string()];
    texts.extend(
        schema
            .create_enum_from_shape(shape)
            .iter()
            .map(|s| s.to_string()),
    );
    texts.extend(
        schema
            .create_index_from_shape(shape)
            .iter()
            .map(|s| s.to_string()),
    );
    texts.extend(
        schema
            .create_comments_from_shape(shape)
            .iter()
            .map(|s| s.to_string()),
    );
    Ok(texts)
}

// [spec:pgorm:sem:schema.from-shape/test]    an entity's shape writes the entity's own statements
#[test]
fn an_entitys_shape_writes_its_statements() -> Result<(), Error> {
    fn entity_statements<E: EntityTrait>(entity: E) -> Result<Vec<String>, Error> {
        let schema = Schema::new();
        let mut texts = vec![schema.create_table_from_entity(entity)?.to_string()];
        texts.extend(
            schema
                .create_enum_from_entity(entity)
                .iter()
                .map(|s| s.to_string()),
        );
        texts.extend(
            schema
                .create_index_from_entity(entity)
                .iter()
                .map(|s| s.to_string()),
        );
        texts.extend(
            schema
                .create_comments_from_entity(entity)
                .iter()
                .map(|s| s.to_string()),
        );
        Ok(texts)
    }

    assert_eq!(
        statements(&TableShape::of::<cake::Entity>())?,
        entity_statements(cake::Entity)?
    );
    assert_eq!(
        statements(&TableShape::of::<cake_filling_price::Entity>())?,
        entity_statements(cake_filling_price::Entity)?
    );
    assert_eq!(
        statements(&TableShape::of::<indexes::Entity>())?,
        entity_statements(indexes::Entity)?
    );
    let noted = statements(&TableShape::of::<noted::Entity>())?;
    assert_eq!(noted, entity_statements(noted::Entity)?);
    assert_eq!(
        noted,
        [
            r#"CREATE TABLE "public"."noted" ( "id" serial NOT NULL, "left" integer NOT NULL, "right" integer NOT NULL, PRIMARY KEY ("id"), CONSTRAINT "noted_pair" UNIQUE ("left", "right") )"#,
            r#"CREATE INDEX "idx-noted-left" ON "public"."noted" ("left")"#,
            r#"COMMENT ON TABLE "public"."noted" IS 'a noted row'"#,
            r#"COMMENT ON COLUMN "public"."noted"."left" IS 'the left half'"#,
        ]
    );
    Ok(())
}

/// A table described at run time with every part a shape takes: a serial
/// key, a stated default, a stored generated column, an index, comments, a
/// unique key and a foreign key with its actions.
fn order_line() -> Result<TableShape, Error> {
    let price = || ColumnType::Decimal(Some((10, 2)));
    let total = Expr::col(Name::runtime("price")).mul(Expr::col(Name::runtime("quantity")));
    let mut to_order = RelationDef::belongs_to(
        table("order_line"),
        table("order"),
        ColumnPairs::new(alias("order_id"), alias("id")),
    );
    to_order.on_delete = Some(ForeignKeyAction::Cascade);
    TableShape::new(
        table("order_line"),
        vec![
            TableColumn::new("id", ColumnType::Integer.def()),
            TableColumn::new("order_id", ColumnType::Integer.def().indexed()),
            TableColumn::new("quantity", ColumnType::Integer.def().default_value(1)),
            TableColumn::new("price", price().def()),
            TableColumn::new(
                "total",
                price()
                    .def()
                    .generated(total, GeneratedKind::Stored)
                    .comment("price times quantity"),
            ),
        ],
    )?
    .with_primary_key(&["id"])?
    .auto_increment()
    .with_unique_key(Some("one_line_per_order"), &["order_id", "price"], false)?
    .with_comment("one row per line of an order")
    .with_relation("Order", to_order)
}

// [spec:pgorm:sem:schema.from-shape/test]    a table known at run time writes the statements an entity of the same declaration would
#[test]
fn a_run_time_shape_writes_every_part() -> Result<(), Error> {
    assert_eq!(
        statements(&order_line()?)?,
        [
            r#"CREATE TABLE "order_line" ( "id" serial NOT NULL, "order_id" integer NOT NULL, "quantity" integer NOT NULL DEFAULT 1, "price" decimal(10, 2) NOT NULL, "total" decimal(10, 2) NOT NULL GENERATED ALWAYS AS ("price" * "quantity") STORED, PRIMARY KEY ("id"), CONSTRAINT "one_line_per_order" UNIQUE ("order_id", "price"), CONSTRAINT "fk-order_line-order_id" FOREIGN KEY ("order_id") REFERENCES "order" ("id") ON DELETE CASCADE )"#,
            r#"CREATE INDEX "idx-order_line-order_id" ON "order_line" ("order_id")"#,
            r#"COMMENT ON TABLE "order_line" IS 'one row per line of an order'"#,
            r#"COMMENT ON COLUMN "order_line"."total" IS 'price times quantity'"#,
        ]
    );
    Ok(())
}

/// `expr` rendered as a query's select list, an expression alone having no
/// `Display`.
fn text(expr: impl Into<pgorm::pgorm_query::SimpleExpr>) -> String {
    let query = pgorm::pgorm_query::Query::select().expr(expr).to_owned();
    query.to_string().replacen("SELECT ", "", 1)
}

/// The read of a column `c` (or "as it stands" where it reads uncast), the
/// write of `value`, and the two sides of a comparison of `c` with `value`.
fn casts(column: &TableColumn, value: Expr) -> [String; 4] {
    let c = || Expr::col(Name::runtime("c"));
    [
        column
            .select(c())
            .map_or_else(|| "as it stands".to_owned(), text),
        text(column.save(value.clone())),
        text(column.compared_column(c())),
        text(column.compared_value(value)),
    ]
}

// [spec:pgorm:sem:schema.shape-casts/test]    a run-time column reads, writes and compares as an entity's column of its type does
#[test]
fn a_columns_type_says_its_casts() {
    let flavor = ColumnType::Enum {
        name: Name::runtime("flavor"),
        schema: None,
        variants: vec![Name::runtime("lemon")],
    };
    let fancy = ColumnType::Domain {
        name: Name::runtime("fancy"),
        schema: None,
        base: std::sync::Arc::new(flavor),
    };
    let fancy = TableColumn::new("c", fancy.def());
    assert_eq!(
        casts(
            &TableColumn::new("c", ColumnType::Money.def()),
            Expr::val(1.5)
        ),
        [
            r#"CAST("c" AS numeric)"#,
            "CAST(1.5 AS money)",
            r#""c""#,
            "CAST(1.5 AS money)"
        ]
    );
    assert_eq!(
        casts(&fancy, Expr::val("lemon")),
        [
            r#"CAST("c" AS text)"#,
            "CAST('lemon' AS fancy)",
            r#"CAST("c" AS flavor)"#,
            "CAST('lemon' AS flavor)"
        ]
    );
    assert_eq!(
        fancy.compares_as().map(|t| t.to_sql_string()),
        Some("flavor".to_owned())
    );
    assert_eq!(
        casts(
            &TableColumn::new("c", ColumnType::Text.def()),
            Expr::val("x")
        ),
        ["as it stands", "'x'", r#""c""#, "'x'"]
    );
    let days = TableColumn::new("c", ColumnType::Bit(Some(7)).def())
        .with_select_as(pgorm::pgorm_query::TypeName::new(Name::runtime("bigint")))
        .with_save_as(pgorm::pgorm_query::TypeName::new(Name::runtime("bit")).modifiers([7]));
    assert_eq!(
        casts(&days, Expr::val(126)),
        [
            r#"CAST("c" AS bigint)"#,
            "CAST(126 AS bit(7))",
            r#""c""#,
            "CAST(126 AS bit(7))"
        ]
    );
}

// [spec:pgorm:sem:schema.from-shape/test]    a composite key is named, never serial, and a temporal key writes its period last
#[test]
fn composite_and_temporal_keys() -> Result<(), Error> {
    let booking = TableShape::new(
        table("booking"),
        vec![
            TableColumn::new("room", ColumnType::Integer.def()),
            TableColumn::new("during", ColumnType::Range(RangeType::TimestampTz).def()),
        ],
    )?
    .with_primary_key(&["room", "during"])?
    .auto_increment()
    .without_overlaps();
    assert_eq!(
        Schema::new().create_table_from_shape(&booking)?.to_string(),
        r#"CREATE TABLE "booking" ( "room" integer NOT NULL, "during" tstzrange NOT NULL, CONSTRAINT "pk-booking" PRIMARY KEY ("room", "during" WITHOUT OVERLAPS) )"#
    );
    Ok(())
}

fn refused<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
    match result {
        Err(Error::Type(message)) => message,
        other => panic!("expected a type error, got {other:?}"),
    }
}

// [spec:pgorm:sem:schema.from-shape/test]    what a shape cannot describe is refused as it is described
#[test]
fn a_shape_refuses_what_it_cannot_describe() -> Result<(), Error> {
    let int = || ColumnType::Integer.def();
    let two = || {
        TableShape::new(
            table("pair"),
            vec![TableColumn::new("a", int()), TableColumn::new("b", int())],
        )
    };
    assert_eq!(
        refused(TableShape::new(
            table("pair"),
            vec![TableColumn::new("a", int()), TableColumn::new("a", int())],
        )),
        "`pair` declares the column `a` twice"
    );
    assert_eq!(
        refused(two()?.with_primary_key(&[])),
        "`pair` declares a primary key with no columns"
    );
    assert_eq!(
        refused(two()?.with_primary_key(&["c"])),
        "`pair` has no column `c` to key its rows by"
    );
    assert_eq!(
        refused(two()?.with_primary_key(&["a", "a"])),
        "`pair`.`a` is named twice in its key"
    );
    assert_eq!(
        refused(two()?.with_unique_key(None, &[], false)),
        "`pair` declares a unique key with no columns"
    );
    assert_eq!(
        refused(two()?.with_unique_key(None, &["c"], false)),
        "`pair` has no column `c` for a unique key"
    );
    assert_eq!(
        refused(two()?.with_unique_key(None, &["a", "a"], false)),
        "`pair`.`a` is named twice in one unique key"
    );
    assert_eq!(
        refused(
            two()?
                .with_unique_key(Some("k"), &["a"], false)?
                .with_unique_key(Some("k"), &["b"], false)
        ),
        "`pair` names two of its keys `k`, which PostgreSQL refuses (42P07)"
    );
    assert_eq!(
        refused(two()?.with_primary_key(&["a", "b"])?.with_unique_key(
            Some("pk-pair"),
            &["a"],
            false
        )),
        "`pair` names two of its keys `pk-pair`, which PostgreSQL refuses (42P07)"
    );
    let r#virtual = TableShape::new(
        table("pair"),
        vec![TableColumn::new(
            "a",
            int().generated(Expr::val(1), GeneratedKind::Virtual),
        )],
    )?;
    assert_eq!(
        refused(r#virtual.with_unique_key(None, &["a"], false)),
        "`pair`.`a` is a VIRTUAL generated column, which PostgreSQL cannot key"
    );
    let owner = RelationDef::belongs_to(
        table("pair"),
        table("other"),
        ColumnPairs::new(alias("a"), alias("id")),
    )
    .rev();
    assert_eq!(
        refused(two()?.with_relation("Other", owner)),
        "relation `Other` of `pair` is not a belongs-to relation, so it holds no foreign key of the table"
    );
    let stray = RelationDef::belongs_to(
        table("pair"),
        table("other"),
        ColumnPairs::new(alias("c"), alias("id")),
    );
    assert_eq!(
        refused(two()?.with_relation("Other", stray)),
        "relation `Other` of `pair` pairs `c`, which the table has no column of"
    );
    let unstated = TableShape::new(
        table("pair"),
        vec![TableColumn::new("a", int()).with_unstated_default()],
    )?;
    assert_eq!(
        refused(Schema::new().create_table_from_shape(&unstated)),
        "`pair`.`a` has a default its description does not state, so its table cannot be written"
    );
    Ok(())
}

// [spec:pgorm:sem:schema.from-shape/test]    a run-time shape's statements run, and the table holds what it declares
#[pgorm_macros::test]
async fn a_run_time_shape_creates_its_table_live() -> Result<(), Error> {
    let ctx = TestContext::new("table_shape_live").await;
    let db = ctx.db.get().await?;
    db.execute(r#"CREATE TABLE "order" ( "id" serial PRIMARY KEY )"#, &[])
        .await?;
    for statement in statements(&order_line()?)? {
        db.execute(&statement, &[]).await?;
    }
    db.execute(r#"INSERT INTO "order" DEFAULT VALUES"#, &[])
        .await?;
    let row = db
        .query_one(
            r#"INSERT INTO "order_line" ("order_id", "price") VALUES (1, 2.50) RETURNING "quantity", "total"::text"#,
            &[],
        )
        .await?;
    assert_eq!(row.get::<_, i32>(0), 1);
    assert_eq!(row.get::<_, String>(1), "2.50");
    let orphan = db
        .execute(
            r#"INSERT INTO "order_line" ("order_id", "price") VALUES (9, 1)"#,
            &[],
        )
        .await;
    assert!(matches!(
        orphan.unwrap_err().sql_error(),
        Some(SqlError::ForeignKeyConstraintViolation(_))
    ));
    db.execute(r#"DELETE FROM "order""#, &[]).await?;
    let left = db
        .query_one(r#"SELECT COUNT(*) FROM "order_line""#, &[])
        .await?;
    assert_eq!(
        left.get::<_, i64>(0),
        0,
        "ON DELETE CASCADE removed the line"
    );
    let comment = db
        .query_one(
            "SELECT obj_description('order_line'::regclass, 'pg_class')",
            &[],
        )
        .await?;
    assert_eq!(
        comment.get::<_, Option<String>>(0).as_deref(),
        Some("one row per line of an order")
    );
    drop(db);
    ctx.delete().await;
    Ok(())
}
