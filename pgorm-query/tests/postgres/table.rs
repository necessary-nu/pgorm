use super::*;
use crate::oracle::{assert_eq, assert_eq_unparsed};

// [spec:pgorm:req:sql.ddl.create-table+16/test]
// [spec:pgorm:req:sql.ddl.column-def+13/test]
#[test]
// [spec:pgorm:def:sql.render.ddl.types+8/test]
fn create_1() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Id)
                    .integer()
                    .not_null()
                    .auto_increment()
            )
            .primary_key(Glyph::Id)
            .col(ColumnDef::new(Glyph::Aspect).double().not_null())
            .col(ColumnDef::new(Glyph::Image).text())
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""id" serial NOT NULL,"#,
            r#""aspect" double precision NOT NULL,"#,
            r#""image" text,"#,
            r#"PRIMARY KEY ("id")"#,
            r#")"#,
        ]
        .join(" ")
    );
}

#[test]
fn create_2() {
    assert_eq!(
        Table::create(Font::Table)
            .col(
                ColumnDef::new(Font::Id)
                    .integer()
                    .not_null()
                    .auto_increment()
            )
            .primary_key(Font::Id)
            .col(ColumnDef::new(Font::Name).string().not_null())
            .col(ColumnDef::new(Font::Variant).string_len(255).not_null())
            .col(ColumnDef::new(Font::Language).string_len(255).not_null())
            .to_string(),
        [
            r#"CREATE TABLE "font" ("#,
            r#""id" serial NOT NULL,"#,
            r#""name" varchar NOT NULL,"#,
            r#""variant" varchar(255) NOT NULL,"#,
            r#""language" varchar(255) NOT NULL,"#,
            r#"PRIMARY KEY ("id")"#,
            r#")"#,
        ]
        .join(" ")
    );
}

#[test]
fn create_3() {
    assert_eq!(
        Table::create(Char::Table)
            .if_not_exists()
            .col(
                ColumnDef::new(Char::Id)
                    .integer()
                    .not_null()
                    .auto_increment()
            )
            .primary_key(Char::Id)
            .col(ColumnDef::new(Char::FontSize).integer().not_null())
            .col(ColumnDef::new(Char::Character).string_len(255).not_null())
            .col(ColumnDef::new(Char::SizeW).integer().not_null())
            .col(ColumnDef::new(Char::SizeH).integer().not_null())
            .col(
                ColumnDef::new(Char::FontId)
                    .integer()
                    .default(Value::Int(None))
            )
            .foreign_key(
                ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
                    .name(Name::runtime("FK_2e303c3a712662f1fc2a4d0aad6"))
                    .on_delete(ForeignKeyAction::Cascade)
                    .on_update(ForeignKeyAction::Cascade)
                    .to_owned()
            )
            .to_string(),
        [
            r#"CREATE TABLE IF NOT EXISTS "character" ("#,
            r#""id" serial NOT NULL,"#,
            r#""font_size" integer NOT NULL,"#,
            r#""character" varchar(255) NOT NULL,"#,
            r#""size_w" integer NOT NULL,"#,
            r#""size_h" integer NOT NULL,"#,
            r#""font_id" integer DEFAULT NULL,"#,
            r#"PRIMARY KEY ("id"),"#,
            r#"CONSTRAINT "FK_2e303c3a712662f1fc2a4d0aad6""#,
            r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
            r#"ON DELETE CASCADE ON UPDATE CASCADE"#,
            r#")"#,
        ]
        .join(" ")
    );
}

#[test]
fn create_4() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Image).named(Glyph::Aspect))
            .to_string(),
        r#"CREATE TABLE "glyph" ( "image" aspect )"#
    );
}

// [spec:pgorm:req:sql.ddl.column-types+8/test]
#[test]
fn create_5() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Image).json())
            .col(ColumnDef::new(Glyph::Aspect).json_binary())
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""image" json,"#,
            r#""aspect" jsonb"#,
            r#")"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.column-types+8/test]    `Char(None)` is the unbounded
// `bpchar`, and `ColumnDef::char` is SQL's `char`, which PostgreSQL reads as
// `char(1)`
#[test]
fn char_lengths_render_as_postgres_reads_them() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Image).char())
            .col(ColumnDef::new(Glyph::Aspect).char_len(3))
            .col(ColumnDef::new_with_type(Glyph::Id, ColumnType::Char(None)))
            .to_string(),
        r#"CREATE TABLE "glyph" ( "image" char(1), "aspect" char(3), "id" bpchar )"#
    );
}

// [spec:pgorm:req:sql.ddl.column-types+8/test]    `Bit(None)` is the bit type
// with no length limit, which only the quoted `"bit"` names, since a bare
// `bit` is `bit(1)`
#[test]
fn bit_lengths_render_as_postgres_reads_them() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Image).bit(None))
            .col(ColumnDef::new(Glyph::Aspect).bit(Some(1)))
            .col(ColumnDef::new(Glyph::Id).bit(Some(8)))
            .col(ColumnDef::new_with_type(
                Glyph::Tokens,
                ColumnType::Array(std::sync::Arc::new(ColumnType::Bit(None)))
            ))
            .to_string(),
        r#"CREATE TABLE "glyph" ( "image" "bit", "aspect" bit(1), "id" bit(8), "tokens" "bit"[] )"#
    );
}

#[test]
fn create_6() {
    assert_eq_unparsed!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Id)
                    .integer()
                    .not_null()
                    .raw_suffix("ANYTHING I WANT TO SAY")
            )
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""id" integer NOT NULL ANYTHING I WANT TO SAY"#,
            r#")"#,
        ]
        .join(" ")
    );
}

#[test]
fn create_7() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Aspect)
                    .interval(IntervalSpec::Any(None))
                    .not_null()
            )
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""aspect" interval NOT NULL"#,
            r#")"#,
        ]
        .join(" ")
    );
}

#[test]
fn create_8() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Aspect)
                    .interval(IntervalSpec::Fields(PgInterval::YearToMonth))
                    .not_null()
            )
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""aspect" interval YEAR TO MONTH NOT NULL"#,
            r#")"#,
        ]
        .join(" ")
    );
}

#[test]
fn create_9() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Aspect)
                    .interval(IntervalSpec::Any(Some(IntervalPrecision::P4)))
                    .not_null()
            )
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""aspect" interval(4) NOT NULL"#,
            r#")"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:sql.types.column-type+11/test]    a precision rides on the second-bearing
// field, the only place PostgreSQL takes one
#[test]
fn create_10() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Aspect)
                    .interval(IntervalSpec::Fields(PgInterval::HourToSecond(Some(
                        IntervalPrecision::P3
                    ))))
                    .not_null()
            )
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""aspect" interval HOUR TO SECOND(3) NOT NULL"#,
            r#")"#,
        ]
        .join(" ")
    );
}

#[test]
fn create_11() {
    assert_eq!(
        Table::create(Char::Table)
            .col(
                ColumnDef::new(Char::CreatedAt)
                    .timestamp_with_time_zone()
                    .not_null()
            )
            .to_string(),
        [
            r#"CREATE TABLE "character" ("#,
            r#""created_at" timestamp with time zone NOT NULL"#,
            r#")"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.column-types+8/test]
#[test]
fn create_12() {
    assert_eq!(
        Table::create(BinaryType::Table)
            .col(ColumnDef::new(BinaryType::BinaryLen).bytea())
            .col(ColumnDef::new(BinaryType::Binary).bytea())
            .to_string(),
        [
            r#"CREATE TABLE "binary_type" ("#,
            r#""binlen" bytea,"#,
            r#""bin" bytea"#,
            r#")"#,
        ]
        .join(" ")
    );
}

#[test]
fn create_14() {
    assert_eq!(
        Table::create((Name::runtime("schema"), Glyph::Table))
            .col(ColumnDef::new(Glyph::Image).named(Glyph::Aspect))
            .to_string(),
        [
            r#"CREATE TABLE "schema"."glyph" ("#,
            r#""image" aspect"#,
            r#")"#,
        ]
        .join(" ")
    );
}

#[test]
fn create_15() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Image).json())
            .col(ColumnDef::new(Glyph::Aspect).json_binary())
            .unique(
                TableKey::new(Glyph::Aspect)
                    .name(Name::runtime("idx-glyph-aspect-image"))
                    .col(Glyph::Image)
                    .nulls_not_distinct()
            )
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""image" json,"#,
            r#""aspect" jsonb,"#,
            r#"CONSTRAINT "idx-glyph-aspect-image" UNIQUE NULLS NOT DISTINCT ("aspect", "image")"#,
            r#")"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.drop-rename-truncate+4/test]
#[test]
fn drop_1() {
    assert_eq!(
        Table::drop(Glyph::Table)
            .table(Char::Table)
            .cascade()
            .to_string(),
        r#"DROP TABLE "glyph", "character" CASCADE"#
    );
}

#[test]
fn drop_2() {
    assert_eq!(
        Table::drop((Name::runtime("schema1"), Glyph::Table))
            .table((Name::runtime("schema2"), Char::Table))
            .cascade()
            .to_string(),
        r#"DROP TABLE "schema1"."glyph", "schema2"."character" CASCADE"#
    );
}

#[test]
fn truncate_1() {
    assert_eq!(
        Table::truncate(Font::Table).to_string(),
        r#"TRUNCATE TABLE "font""#
    );
}

#[test]
fn truncate_2() {
    assert_eq!(
        Table::truncate((Name::runtime("schema"), Font::Table)).to_string(),
        r#"TRUNCATE TABLE "schema"."font""#
    );
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]
#[test]
fn alter_1() {
    assert_eq!(
        Table::alter(Font::Table)
            .add_column(
                ColumnDef::new(Name::runtime("new_col"))
                    .integer()
                    .not_null()
                    .default(100)
            )
            .to_string(),
        r#"ALTER TABLE "font" ADD COLUMN "new_col" integer NOT NULL DEFAULT 100"#
    );
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]
#[test]
fn alter_2() {
    assert_eq!(
        Table::alter(Font::Table)
            .modify_column(
                ColumnDef::new(Name::runtime("new_col"))
                    .big_integer()
                    .default(999)
            )
            .to_string(),
        [
            r#"ALTER TABLE "font""#,
            r#"ALTER COLUMN "new_col" TYPE bigint,"#,
            r#"ALTER COLUMN "new_col" SET DEFAULT 999"#,
        ]
        .join(" ")
    );
}

#[test]
fn alter_3() {
    assert_eq!(
        Table::rename_column(
            Font::Table,
            Name::runtime("new_col"),
            Name::runtime("new_column")
        )
        .to_string(),
        r#"ALTER TABLE "font" RENAME COLUMN "new_col" TO "new_column""#
    );
}

#[test]
fn alter_4() {
    assert_eq!(
        Table::alter(Font::Table)
            .drop_column(Name::runtime("new_column"))
            .to_string(),
        r#"ALTER TABLE "font" DROP COLUMN "new_column""#
    );
}

#[test]
fn alter_5() {
    assert_eq!(
        Table::rename_column(
            (Name::runtime("schema"), Font::Table),
            Name::runtime("new_col"),
            Name::runtime("new_column")
        )
        .to_string(),
        r#"ALTER TABLE "schema"."font" RENAME COLUMN "new_col" TO "new_column""#
    );
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]    a rename is a statement of its own, so it
// cannot join the comma-separated options
#[test]
fn alter_7() {
    assert_eq!(
        Table::alter(Font::Table)
            .add_column(ColumnDef::new(Name::runtime("new_col")).integer())
            .drop_column(Font::Name)
            .to_string(),
        r#"ALTER TABLE "font" ADD COLUMN "new_col" integer, DROP COLUMN "name""#
    );
}

#[test]
fn alter_8() {
    assert_eq!(
        Table::alter(Font::Table)
            .modify_column(ColumnDef::new(Font::Language).null())
            .to_string(),
        [
            r#"ALTER TABLE "font""#,
            r#"ALTER COLUMN "language" DROP NOT NULL"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]    a key is added as an action of its own,
// after the column it keys
#[test]
fn alter_9() {
    // https://dbfiddle.uk/98Vd8pmn
    assert_eq!(
        Table::alter(Glyph::Table)
            .modify_column(
                ColumnDef::new(Glyph::Aspect)
                    .integer()
                    .auto_increment()
                    .not_null()
            )
            .add_unique(Glyph::Aspect)
            .add_primary_key(Glyph::Aspect)
            .to_string(),
        [
            r#"ALTER TABLE "glyph""#,
            r#"ALTER COLUMN "aspect" TYPE integer,"#,
            r#"ALTER COLUMN "aspect" SET NOT NULL,"#,
            r#"ADD UNIQUE ("aspect"),"#,
            r#"ADD PRIMARY KEY ("aspect")"#,
        ]
        .join(" ")
    );
}

#[test]
fn alter_10() {
    // https://dbfiddle.uk/BeiZPvBe
    assert_eq!(
        Table::alter(Glyph::Table)
            .add_column(
                ColumnDef::new(Glyph::Aspect)
                    .integer()
                    .auto_increment()
                    .not_null()
            )
            .add_unique(TableKey::new(Glyph::Aspect).name(Name::runtime("glyph_aspect_key")))
            .add_primary_key(TableKey::new(Glyph::Aspect).name(Name::runtime("glyph_pkey")))
            .to_string(),
        [
            r#"ALTER TABLE "glyph""#,
            r#"ADD COLUMN "aspect" serial NOT NULL,"#,
            r#"ADD CONSTRAINT "glyph_aspect_key" UNIQUE ("aspect"),"#,
            r#"ADD CONSTRAINT "glyph_pkey" PRIMARY KEY ("aspect")"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.drop-rename-truncate+4/test]
#[test]
fn rename_1() {
    assert_eq!(
        Table::rename(Font::Table, Name::runtime("font_new")).to_string(),
        r#"ALTER TABLE "font" RENAME TO "font_new""#
    );
}

#[test]
fn rename_2() {
    assert_eq!(
        Table::rename(
            (Name::runtime("schema"), Font::Table),
            Name::runtime("font_new")
        )
        .to_string(),
        r#"ALTER TABLE "schema"."font" RENAME TO "font_new""#
    );
}

#[test]
fn create_with_check_constraint() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Id)
                    .integer()
                    .not_null()
                    .check(Expr::col(Glyph::Id).gt(10))
            )
            .check(Expr::col(Glyph::Id).lt(20))
            .check(Expr::col(Glyph::Id).ne(15))
            .to_string(),
        r#"CREATE TABLE "glyph" ( "id" integer NOT NULL CHECK ("id" > 10), CHECK ("id" < 20), CHECK ("id" <> 15) )"#,
    );
}

#[test]
fn alter_with_check_constraint() {
    assert_eq!(
        Table::alter(Glyph::Table)
            .add_column(
                ColumnDef::new(Glyph::Aspect)
                    .integer()
                    .not_null()
                    .default(101)
                    .check(Expr::col(Glyph::Aspect).gt(100))
            )
            .to_string(),
        r#"ALTER TABLE "glyph" ADD COLUMN "aspect" integer NOT NULL DEFAULT 101 CHECK ("aspect" > 100)"#,
    );
}

#[test]
fn create_16() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Id)
                    .integer()
                    .not_null()
                    .auto_increment()
            )
            .primary_key(Glyph::Id)
            .col(ColumnDef::new(Glyph::Tokens).ltree())
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""id" serial NOT NULL,"#,
            r#""tokens" ltree,"#,
            r#"PRIMARY KEY ("id")"#,
            r#")"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.create-table+16/test]    a primary key is a table constraint, the one
// spelling it has here, whether built or converted from a tuple
#[test]
fn a_primary_key_is_a_table_constraint() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).integer().not_null())
            .col(ColumnDef::new(Glyph::Image).string().not_null())
            .primary_key(
                TableKey::new(Glyph::Id)
                    .name(Name::runtime("pk-glyph"))
                    .col(Glyph::Image)
            )
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""id" integer NOT NULL,"#,
            r#""image" varchar NOT NULL,"#,
            r#"CONSTRAINT "pk-glyph" PRIMARY KEY ("id", "image")"#,
            r#")"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]    a foreign key embeds by value, so the source
// survives only where the call site cloned it
#[test]
fn alter_embeds_its_foreign_key_by_value() {
    let key = TableForeignKey::new(Char::Table, Char::FontId, Font::Table, Font::Id)
        .name(Name::runtime("fk-character-font_id"))
        .to_owned();

    assert_eq!(
        Table::alter(Char::Table)
            .add_foreign_key(key.clone())
            .to_string(),
        [
            r#"ALTER TABLE "character" ADD CONSTRAINT "fk-character-font_id""#,
            r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
        ]
        .join(" ")
    );
    assert_eq!(
        Table::alter(Char::Table).add_foreign_key(key).to_string(),
        [
            r#"ALTER TABLE "character" ADD CONSTRAINT "fk-character-font_id""#,
            r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    a generated column writes its kind
// whichever it is, so neither render leans on the server's default, which 17 and
// 18 disagree about
#[test]
fn generated_column_writes_its_kind() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Aspect)
                    .integer()
                    .generated(Expr::col(Glyph::Id).mul(2), GeneratedKind::Stored)
            )
            .to_string(),
        r#"CREATE TABLE "glyph" ( "aspect" integer GENERATED ALWAYS AS ("id" * 2) STORED )"#
    );
    assert_eq!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Aspect)
                    .integer()
                    .generated(Expr::col(Glyph::Id).mul(2), GeneratedKind::Virtual)
            )
            .to_string(),
        r#"CREATE TABLE "glyph" ( "aspect" integer GENERATED ALWAYS AS ("id" * 2) VIRTUAL )"#
    );
    assert_eq!(
        Table::alter(Glyph::Table)
            .add_column(
                ColumnDef::new(Glyph::Aspect)
                    .integer()
                    .generated(Expr::col(Glyph::Id).add(1), GeneratedKind::Virtual)
            )
            .to_string(),
        r#"ALTER TABLE "glyph" ADD COLUMN "aspect" integer GENERATED ALWAYS AS ("id" + 1) VIRTUAL"#
    );
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]    a generated column's expression is
// set with its `AS` and dropped with or without `IF EXISTS`, each its own action
// beside the statement's others
#[test]
fn expression_actions_render_on_their_own() {
    assert_eq!(
        Table::alter(Glyph::Table)
            .set_expression(Glyph::Aspect, Expr::col(Glyph::Id).mul(3))
            .to_string(),
        r#"ALTER TABLE "glyph" ALTER COLUMN "aspect" SET EXPRESSION AS ("id" * 3)"#
    );
    assert_eq!(
        Table::alter(Glyph::Table)
            .drop_expression(Glyph::Aspect)
            .to_string(),
        r#"ALTER TABLE "glyph" ALTER COLUMN "aspect" DROP EXPRESSION"#
    );
    assert_eq!(
        Table::alter(Glyph::Table)
            .drop_expression_if_exists(Glyph::Aspect)
            .add_column(ColumnDef::new(Glyph::Tokens).integer())
            .set_expression(Glyph::Image, Expr::val("x"))
            .to_string(),
        [
            r#"ALTER TABLE "glyph" ALTER COLUMN "aspect" DROP EXPRESSION IF EXISTS,"#,
            r#"ADD COLUMN "tokens" integer,"#,
            r#"ALTER COLUMN "image" SET EXPRESSION AS ('x')"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]    a retype with its USING
// expression, under a collation or not, and a default dropped, each its own
// action beside the statement's others
#[test]
fn retype_using_and_drop_default_render() {
    assert_eq!(
        Table::alter(Glyph::Table)
            .alter_column_type_collated_using(
                Glyph::Image,
                ColumnType::Text,
                Name::runtime("C"),
                Expr::col(Glyph::Image).cast_as(Name::runtime("text")),
            )
            .alter_column_type_collated_using(
                Glyph::Aspect,
                ColumnType::string(Some(8)),
                (Name::runtime("pg_catalog"), Name::runtime("POSIX")),
                Expr::col(Glyph::Aspect).cast_as(Name::runtime("text")),
            )
            .to_string(),
        [
            r#"ALTER TABLE "glyph" ALTER COLUMN "image" TYPE text COLLATE "C""#,
            r#"USING CAST("image" AS text),"#,
            r#"ALTER COLUMN "aspect" TYPE varchar(8) COLLATE "pg_catalog"."POSIX""#,
            r#"USING CAST("aspect" AS text)"#,
        ]
        .join(" ")
    );
    assert_eq!(
        Table::alter(Glyph::Table)
            .alter_column_type_using(
                Glyph::Aspect,
                ColumnType::BigInteger,
                Expr::col(Glyph::Aspect).mul(10),
            )
            .to_string(),
        r#"ALTER TABLE "glyph" ALTER COLUMN "aspect" TYPE bigint USING "aspect" * 10"#
    );
    assert_eq!(
        Table::alter(Glyph::Table)
            .drop_default(Glyph::Aspect)
            .to_string(),
        r#"ALTER TABLE "glyph" ALTER COLUMN "aspect" DROP DEFAULT"#
    );
    assert_eq!(
        Table::alter(Glyph::Table)
            .drop_default(Glyph::Image)
            .alter_column_type_using(
                Glyph::Image,
                ColumnType::named("image_kind"),
                Expr::col(Glyph::Image)
                    .cast_as(Name::runtime("text"))
                    .cast_as(Name::runtime("image_kind")),
            )
            .modify_column(ColumnDef::new(Glyph::Image).default("png"))
            .to_string(),
        [
            r#"ALTER TABLE "glyph" ALTER COLUMN "image" DROP DEFAULT,"#,
            r#"ALTER COLUMN "image" TYPE image_kind"#,
            r#"USING CAST(CAST("image" AS text) AS image_kind),"#,
            r#"ALTER COLUMN "image" SET DEFAULT 'png'"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    a column's NOT NULL is one
// constraint: named or kept from children where it first stands, never twice
#[test]
fn column_not_null_is_one_constraint() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).integer().not_null())
            .col(
                ColumnDef::new(Glyph::Aspect)
                    .integer()
                    .not_null_named(Name::runtime("aspect_present"))
            )
            .col(ColumnDef::new(Glyph::Image).text().not_null_no_inherit())
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ( "id" integer NOT NULL,"#,
            r#""aspect" integer CONSTRAINT "aspect_present" NOT NULL,"#,
            r#""image" text NOT NULL NO INHERIT )"#,
        ]
        .join(" ")
    );
    let column = ColumnDef::new(Glyph::Id)
        .integer()
        .not_null()
        .default(1)
        .not_null_named(Name::runtime("first"))
        .not_null()
        .not_null_no_inherit()
        .not_null_named(Name::runtime("second"))
        .to_owned();
    assert_eq!(
        Table::create(Glyph::Table).col(column.clone()).to_string(),
        r#"CREATE TABLE "glyph" ( "id" integer CONSTRAINT "second" NOT NULL NO INHERIT DEFAULT 1 )"#
    );
    let not_nulls: Vec<_> = column
        .get_column_spec()
        .iter()
        .filter_map(|spec| match spec {
            ColumnSpec::NotNull { name, no_inherit } => {
                Some((name.as_ref().map(|name| name.to_string()), *no_inherit))
            }
            _ => None,
        })
        .collect();
    assert_eq!(not_nulls, [(Some("second".to_owned()), true)]);
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]    a NOT NULL is added at table
// level, validated and altered by name
#[test]
fn not_null_actions_render_on_their_own() {
    assert_eq!(
        Table::alter(Glyph::Table)
            .add_not_null(NotNullConstraint::new(Glyph::Aspect))
            .to_string(),
        r#"ALTER TABLE "glyph" ADD NOT NULL "aspect""#
    );
    assert_eq!(
        Table::alter(Glyph::Table)
            .add_not_null(
                NotNullConstraint::new(Glyph::Aspect)
                    .not_valid()
                    .no_inherit()
                    .name(Name::runtime("aspect_present"))
            )
            .validate_constraint(Name::runtime("aspect_present"))
            .alter_constraint(Name::runtime("aspect_present"), ConstraintChange::Inherit)
            .alter_constraint(Name::runtime("image_present"), ConstraintChange::NoInherit)
            .to_string(),
        [
            r#"ALTER TABLE "glyph" ADD CONSTRAINT "aspect_present" NOT NULL "aspect" NO INHERIT NOT VALID,"#,
            r#"VALIDATE CONSTRAINT "aspect_present","#,
            r#"ALTER CONSTRAINT "aspect_present" INHERIT,"#,
            r#"ALTER CONSTRAINT "image_present" NO INHERIT"#,
        ]
        .join(" ")
    );
    let constraint = NotNullConstraint::new(Glyph::Aspect)
        .name(Name::runtime("aspect_present"))
        .not_valid();
    assert_eq!(constraint.get_column().to_string(), "aspect");
    assert_eq!(
        constraint
            .get_name()
            .map(|name| name.to_string())
            .as_deref(),
        Some("aspect_present")
    );
    assert!(constraint.is_not_valid());
    assert!(!constraint.is_no_inherit());
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]    a constraint of any kind is dropped by
// name, with IF EXISTS and a behavior when the drop says so, the last behavior winning, and
// renamed by a statement of its own
#[test]
fn constraints_drop_and_rename_by_name() {
    assert_eq!(
        Table::alter(Glyph::Table)
            .drop_constraint(Name::runtime("glyph_aspect_not_null"))
            .drop_constraint(ConstraintDrop::new(Name::runtime("glyph_pkey")).if_exists())
            .drop_constraint(
                ConstraintDrop::new(Name::runtime("glyph_image_key"))
                    .restrict()
                    .cascade()
            )
            .drop_constraint(
                ConstraintDrop::new(Name::runtime("glyph_check"))
                    .cascade()
                    .restrict()
            )
            .to_string(),
        [
            r#"ALTER TABLE "glyph" DROP CONSTRAINT "glyph_aspect_not_null","#,
            r#"DROP CONSTRAINT IF EXISTS "glyph_pkey","#,
            r#"DROP CONSTRAINT "glyph_image_key" CASCADE,"#,
            r#"DROP CONSTRAINT "glyph_check" RESTRICT"#,
        ]
        .join(" ")
    );
    assert_eq!(
        Table::alter((Name::runtime("schema"), Char::Table))
            .drop_constraint(Name::runtime("FK_2e303c3a712662f1fc2a4d0aad6"))
            .to_string(),
        r#"ALTER TABLE "schema"."character" DROP CONSTRAINT "FK_2e303c3a712662f1fc2a4d0aad6""#
    );
    let drop = ConstraintDrop::new(Name::runtime("k"))
        .if_exists()
        .cascade();
    assert_eq!(drop.get_name().to_string(), "k");
    assert!(drop.is_if_exists());
    assert_eq!(drop.get_behavior(), Some(DropBehavior::Cascade));
    let plain = ConstraintDrop::new(Name::runtime("k"));
    assert!(!plain.is_if_exists());
    assert_eq!(plain.get_behavior(), None);

    assert_eq!(
        Table::rename_constraint(
            (Name::runtime("schema"), Glyph::Table),
            Name::runtime("glyph_pkey"),
            Name::runtime("glyph_key"),
        )
        .to_string(),
        r#"ALTER TABLE "schema"."glyph" RENAME CONSTRAINT "glyph_pkey" TO "glyph_key""#
    );
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]    a modified column's plain NOT
// NULL is SET, and its named or NO INHERIT one is added, which SET cannot say
#[test]
fn modified_named_not_null_is_added() {
    assert_eq!(
        Table::alter(Glyph::Table)
            .modify_column(ColumnDef::new(Glyph::Aspect).not_null())
            .modify_column(
                ColumnDef::new(Glyph::Image)
                    .text()
                    .not_null_named(Name::runtime("image_present"))
            )
            .modify_column(ColumnDef::new(Glyph::Tokens).not_null_no_inherit())
            .to_string(),
        [
            r#"ALTER TABLE "glyph" ALTER COLUMN "aspect" SET NOT NULL,"#,
            r#"ALTER COLUMN "image" TYPE text,"#,
            r#"ADD CONSTRAINT "image_present" NOT NULL "image","#,
            r#"ADD NOT NULL "tokens" NO INHERIT"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.enforcement/test]    a foreign key and a CHECK write
// their enforcement last, after the foreign key's deferrability, and only when
// a caller said it
#[test]
fn enforcement_renders_after_the_constraint() {
    let key = |enforcement: Option<Enforcement>| {
        let mut key = ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id);
        key.on_delete(ForeignKeyAction::Cascade)
            .deferrability(Deferrability::DeferrableInitiallyDeferred);
        if let Some(enforcement) = enforcement {
            key.enforcement(enforcement);
        }
        key.to_string()
    };
    let head = r#"ALTER TABLE "character" ADD FOREIGN KEY ("font_id") REFERENCES "font" ("id") ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED"#;
    assert_eq!(key(None), head);
    assert_eq!(key(Some(Enforcement::Enforced)), format!("{head} ENFORCED"));
    assert_eq!(
        key(Some(Enforcement::NotEnforced)),
        format!("{head} NOT ENFORCED")
    );

    assert_eq!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Aspect).integer().check(
                    Check::new(Expr::col(Glyph::Aspect).gt(0))
                        .name(Name::runtime("aspect_positive"))
                        .enforcement(Enforcement::NotEnforced)
                )
            )
            .check(Expr::col(Glyph::Aspect).lt(100))
            .check(Check::new(Expr::col(Glyph::Aspect).ne(7)).enforcement(Enforcement::Enforced))
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ("#,
            r#""aspect" integer CONSTRAINT "aspect_positive" CHECK ("aspect" > 0) NOT ENFORCED,"#,
            r#"CHECK ("aspect" < 100), CHECK ("aspect" <> 7) ENFORCED )"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]    a CHECK is added as an action of
// its own, a modified column's CHECK is added rather than written bare, and
// ALTER CONSTRAINT changes a foreign key's enforcement
#[test]
fn check_and_enforcement_actions_render() {
    assert_eq!(
        Table::alter(Glyph::Table)
            .add_check(Expr::col(Glyph::Aspect).gt(0))
            .add_check(
                Check::new(Expr::col(Glyph::Aspect).lt(100))
                    .name(Name::runtime("aspect_small"))
                    .enforcement(Enforcement::NotEnforced)
            )
            .alter_constraint(Name::runtime("glyph_font"), ConstraintChange::Enforced)
            .alter_constraint(Name::runtime("glyph_font"), ConstraintChange::NotEnforced)
            .to_string(),
        [
            r#"ALTER TABLE "glyph" ADD CHECK ("aspect" > 0),"#,
            r#"ADD CONSTRAINT "aspect_small" CHECK ("aspect" < 100) NOT ENFORCED,"#,
            r#"ALTER CONSTRAINT "glyph_font" ENFORCED,"#,
            r#"ALTER CONSTRAINT "glyph_font" NOT ENFORCED"#,
        ]
        .join(" ")
    );
    assert_eq!(
        Table::alter(Glyph::Table)
            .modify_column(
                ColumnDef::new(Glyph::Aspect)
                    .default(1)
                    .check(Expr::col(Glyph::Aspect).gt(0))
            )
            .to_string(),
        [
            r#"ALTER TABLE "glyph" ALTER COLUMN "aspect" SET DEFAULT 1,"#,
            r#"ADD CHECK ("aspect" > 0)"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.alter-table+15/test]    a foreign key and a CHECK are added NOT
// VALID after everything else they carry, and only by the ADD actions
// [spec:pgorm:req:sql.ddl.create-table+16/test]    a CHECK's NO INHERIT is written before its
// enforcement wherever the CHECK stands
#[test]
fn not_valid_and_no_inherit_render_where_taken() {
    let mut font = TableForeignKey::new(Char::Table, Char::FontId, Font::Table, Font::Id);
    font.name(Name::runtime("character_font"))
        .on_delete(ForeignKeyAction::Cascade)
        .deferrability(Deferrability::DeferrableInitiallyDeferred)
        .enforcement(Enforcement::NotEnforced);
    let check = Check::new(Expr::col(Char::SizeW).gt(0))
        .name(Name::runtime("size_positive"))
        .no_inherit()
        .enforcement(Enforcement::Enforced);
    assert!(check.is_no_inherit());
    let not_valid = check.clone().not_valid();
    assert!(not_valid.get_constraint().is_no_inherit());
    assert_eq!(
        Table::alter(Char::Table)
            .add_foreign_key(font.clone().not_valid())
            .add_foreign_key(font)
            .add_check(not_valid)
            .add_check(Check::new(Expr::col(Char::SizeH).gt(0)).not_valid())
            .to_string(),
        [
            r#"ALTER TABLE "character" ADD CONSTRAINT "character_font""#,
            r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id") ON DELETE CASCADE"#,
            r#"DEFERRABLE INITIALLY DEFERRED NOT ENFORCED NOT VALID,"#,
            r#"ADD CONSTRAINT "character_font""#,
            r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id") ON DELETE CASCADE"#,
            r#"DEFERRABLE INITIALLY DEFERRED NOT ENFORCED,"#,
            r#"ADD CONSTRAINT "size_positive" CHECK ("size_w" > 0) NO INHERIT ENFORCED NOT VALID,"#,
            r#"ADD CHECK ("size_h" > 0) NOT VALID"#,
        ]
        .join(" ")
    );
    assert_eq!(
        Table::create(Char::Table)
            .col(
                ColumnDef::new(Char::SizeW)
                    .integer()
                    .check(Check::new(Expr::col(Char::SizeW).gt(0)).no_inherit())
            )
            .check(check)
            .to_string(),
        [
            r#"CREATE TABLE "character" ("#,
            r#""size_w" integer CHECK ("size_w" > 0) NO INHERIT,"#,
            r#"CONSTRAINT "size_positive" CHECK ("size_w" > 0) NO INHERIT ENFORCED )"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    both identity forms render, and
// the ALWAYS/BY DEFAULT choice is the only thing that differs between them
#[test]
fn identity_column_spells_both_generations() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).integer().identity())
            .primary_key(Glyph::Id)
            .to_string(),
        r#"CREATE TABLE "glyph" ( "id" integer GENERATED ALWAYS AS IDENTITY, PRIMARY KEY ("id") )"#
    );

    assert_eq!(
        Table::create(Glyph::Table)
            .col(
                ColumnDef::new(Glyph::Id)
                    .big_integer()
                    .identity_by_default()
            )
            .primary_key(Glyph::Id)
            .to_string(),
        r#"CREATE TABLE "glyph" ( "id" bigint GENERATED BY DEFAULT AS IDENTITY, PRIMARY KEY ("id") )"#
    );

    assert_eq!(IdentityGeneration::Always.keyword(), "ALWAYS");
    assert_eq!(IdentityGeneration::ByDefault.keyword(), "BY DEFAULT");
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    identity is a clause of the
// column, so it renders in insertion order among the other specs
#[test]
fn identity_renders_in_insertion_order() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).integer().not_null().identity())
            .to_string(),
        r#"CREATE TABLE "glyph" ( "id" integer NOT NULL GENERATED ALWAYS AS IDENTITY )"#
    );

    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).integer().identity().not_null())
            .to_string(),
        r#"CREATE TABLE "glyph" ( "id" integer GENERATED ALWAYS AS IDENTITY NOT NULL )"#
    );
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    the two `ALTER TABLE` positions:
// a new column carries the clause, an existing one takes the ADD GENERATED action
#[test]
fn identity_alters_both_ways() {
    assert_eq!(
        Table::alter(Glyph::Table)
            .add_column(ColumnDef::new(Glyph::Id).integer().identity())
            .to_string(),
        r#"ALTER TABLE "glyph" ADD COLUMN "id" integer GENERATED ALWAYS AS IDENTITY"#
    );

    assert_eq!(
        Table::alter(Glyph::Table)
            .modify_column(ColumnDef::new(Glyph::Id).identity_by_default())
            .to_string(),
        r#"ALTER TABLE "glyph" ALTER COLUMN "id" ADD GENERATED BY DEFAULT AS IDENTITY"#
    );
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    identity and the serial family
// are two spellings of one idea, and asking for both renders SQL the grammar
// takes but the server refuses — the boundary this rule documents rather than types
#[test]
fn identity_with_serial_is_a_documented_boundary() {
    let both = Table::create(Glyph::Table)
        .col(
            ColumnDef::new(Glyph::Id)
                .integer()
                .auto_increment()
                .identity(),
        )
        .to_string();

    assert_eq!(
        both,
        r#"CREATE TABLE "glyph" ( "id" serial GENERATED ALWAYS AS IDENTITY )"#
    );

    // The grammar has no objection; only a server with a catalog does. That is
    // why the refusal cannot live in the oracle, and lives in the live suite.
    crate::oracle::assert_parses(&both);
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    an operator expression is a column's DEFAULT
// parenthesised, in CREATE TABLE and ADD COLUMN alike, and any other expression bare, while
// SET DEFAULT, which reads the full expression grammar, takes it bare
#[test]
fn operator_defaults_are_parenthesised() {
    let both = || SimpleExpr::from(Expr::val(true)).and(Expr::val(false).into());
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).boolean().default(both()))
            .col(
                ColumnDef::new(Glyph::Aspect)
                    .integer()
                    .default(SimpleExpr::from(Expr::val(1)).add(2)),
            )
            .col(ColumnDef::new(Glyph::Image).text().default("x"))
            .to_string(),
        [
            r#"CREATE TABLE "glyph" ( "id" bool DEFAULT (TRUE AND FALSE),"#,
            r#""aspect" integer DEFAULT (1 + 2),"#,
            r#""image" text DEFAULT 'x' )"#,
        ]
        .join(" ")
    );
    assert_eq!(
        Table::alter(Glyph::Table)
            .add_column(
                ColumnDef::new(Glyph::Id)
                    .boolean()
                    .default(Expr::val(2).between(1, 3)),
            )
            .to_string(),
        r#"ALTER TABLE "glyph" ADD COLUMN "id" bool DEFAULT (2 BETWEEN 1 AND 3)"#
    );
    assert_eq!(
        Table::alter(Glyph::Table)
            .modify_column(ColumnDef::new(Glyph::Id).default(both()))
            .to_string(),
        r#"ALTER TABLE "glyph" ALTER COLUMN "id" SET DEFAULT TRUE AND FALSE"#
    );
}
