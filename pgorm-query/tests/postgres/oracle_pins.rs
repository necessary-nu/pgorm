//! Renders the PostgreSQL grammar rejects today, and the ones it used to.
//!
//! Each pin asserts the rejection rather than hiding it, so the defect is
//! documented and the pin fails loudly — "oracle pin is stale" — the moment the
//! plan node named in its comment lands and the render becomes valid. Sites that
//! produce these renders are marked with `assert_eq_unparsed!` in the test suite.
//! A pin whose defect is fixed stays here as the positive assertion it became,
//! so the render that was once rejected is held to the grammar instead.

use super::*;
use crate::oracle::{assert_parses, assert_rejected};
use pgorm_query::extension::{Extension, Type};

// Fixed by plan node `bug.oracle-findings`: the frame renderer writes a space
// between the bound value and the keyword, so the offset reads as an offset
// rather than as trailing junk after a numeric literal.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.render.window+7/test]
#[test]
fn window_frame_offset_renders_spaced() {
    let sql = Query::select()
        .from(Char::Table)
        .expr_window(
            Func::count(Expr::col(Char::Id)),
            WindowStatement::partition_by(Char::FontSize)
                .frame(FrameType::Rows.preceding(2))
                .take(),
        )
        .to_string();

    assert!(sql.contains("ROWS 2 PRECEDING"));
    assert_parses(&sql);
}

// Fixed by plan node `unrep.over-function`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: the four `expr_window*`
// constructors take a `FunctionCall`, the only production PostgreSQL admits
// `OVER` after, so a windowed column reference no longer typechecks. The
// rejection it used to be pinned to is proved by the `compile_fail` doctest on
// `SelectStatement::expr_window`.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:def:sql.ast.window-statement+8/test]
// [spec:pgorm:req:sql.render.window+7/test]
#[test]
fn over_attaches_only_to_function_calls() {
    let sql = Query::select()
        .from(Char::Table)
        .expr_window(
            Func::count(Expr::col(Char::Id)),
            WindowStatement::partition_by(Char::FontSize),
        )
        .to_string();

    assert_eq!(
        sql,
        r#"SELECT COUNT("id") OVER ( PARTITION BY "font_size" ) FROM "character""#
    );
    assert_parses(&sql);
}

// The MySQL-era table options (`ENGINE=`, `COLLATE=`, `DEFAULT CHARSET=`) have
// no PostgreSQL spelling, and `TableCreateStatement` carries no options for them
// to come from: nothing but the caller's own `extra` string can follow the
// closing parenthesis. Deletion-proof, so there is no rejection left to pin.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.create-table+16/test]
#[test]
fn create_table_renders_no_trailing_options() {
    let sql = Table::create(Glyph::Table)
        .col(ColumnDef::new(Glyph::Id).integer())
        .to_string();

    assert_eq!(sql, r#"CREATE TABLE "glyph" ( "id" integer )"#);
    assert_parses(&sql);
}

// Fixed by plan node `unrep.on-conflict`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: a clause is a target paired with
// an action or a bare `DO NOTHING`, the update assignments are non-empty by
// construction, and only the update carries a filter. The three renders this
// pin held — a target with no action, an empty `DO UPDATE SET`, and a filter on
// `DO NOTHING` — no longer typecheck; the `compile_fail` doctests on
// `OnConflict` prove it. What remains is the shapes the grammar does accept.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.render.on-conflict+2/test]
#[test]
fn on_conflict_renders_only_valid_shapes() {
    let insert = |conflict: OnConflict| {
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect])
            .values_panic([1.into()])
            .on_conflict(conflict)
            .to_string()
    };

    let bare = insert(OnConflict::do_nothing());
    let targeted = insert(OnConflict::column(Glyph::Id).do_nothing());
    let updated = insert(
        OnConflict::column(Glyph::Id)
            .and_where(Expr::col(Glyph::Aspect).is_null())
            .update_column(Glyph::Aspect)
            .and_where(Expr::col(Glyph::Image).gt(0))
            .into(),
    );

    let named = insert(
        OnConflict::constraint(Name::runtime("glyph_key"))
            .update_column(Glyph::Aspect)
            .and_where(Expr::col(Glyph::Image).gt(0))
            .into(),
    );

    assert!(bare.ends_with("ON CONFLICT DO NOTHING"));
    assert!(targeted.ends_with(r#"ON CONFLICT ("id") DO NOTHING"#));
    assert!(updated.ends_with(
        r#"ON CONFLICT ("id") WHERE "aspect" IS NULL DO UPDATE SET "aspect" = "excluded"."aspect" WHERE "image" > 0"#
    ));
    assert!(named.ends_with(
        r#"ON CONFLICT ON CONSTRAINT "glyph_key" DO UPDATE SET "aspect" = "excluded"."aspect" WHERE "image" > 0"#
    ));
    assert_parses(&bare);
    assert_parses(&targeted);
    assert_parses(&updated);
    assert_parses(&named);
}

// Fixed by plan node `unrep.dml-order-limit`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: PostgreSQL admits ORDER BY and
// LIMIT on neither UPDATE nor DELETE, so the two statements carry neither and
// implement no `OrderedStatement`. The renders this pin held no longer build —
// the `compile_fail` doctests on `UpdateStatement` and `DeleteStatement` prove
// it — leaving the shape the grammar does accept, with the ordering and the
// limit where they belong: the SELECT that chooses the rows.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.render.update-delete+3/test]
// [spec:pgorm:req:sql.ast.update+5/test]
// [spec:pgorm:def:sql.ast.delete+4/test]
#[test]
fn writes_take_order_and_limit_by_subquery() {
    let rows = || {
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .order_by(Glyph::Id, Order::Asc)
            .limit(1)
            .take()
    };
    let update = Query::update()
        .table(Glyph::Table)
        .value(Glyph::Aspect, 1)
        .and_where(Expr::col(Glyph::Id).in_subquery(rows()))
        .to_string();
    let delete = Query::delete()
        .from_table(Glyph::Table)
        .and_where(Expr::col(Glyph::Id).in_subquery(rows()))
        .to_string();

    assert_eq!(
        update,
        r#"UPDATE "glyph" SET "aspect" = 1 WHERE "id" IN (SELECT "id" FROM "glyph" ORDER BY "id" ASC LIMIT 1)"#
    );
    assert_eq!(
        delete,
        r#"DELETE FROM "glyph" WHERE "id" IN (SELECT "id" FROM "glyph" ORDER BY "id" ASC LIMIT 1)"#
    );
    assert_parses(&update);
    assert_parses(&delete);
}

// Fixed by plan node `bug.oracle-findings`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: the constraint travels inside
// `JoinKind`, so a cross join carries none to render and `JoinType` no longer
// spells one.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.render.joins+2/test]
// [spec:pgorm:req:sql.ast.select.join+1/test]
#[test]
fn cross_join_renders_without_on_clause() {
    let sql = Query::select()
        .column(Char::Id)
        .from(Char::Table)
        .cross_join(Font::Table)
        .to_string();

    assert_eq!(sql, r#"SELECT "id" FROM "character" CROSS JOIN "font""#);
    assert_parses(&sql);
}

// Fixed by plan node `bug.oracle-findings`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: PostgreSQL admits RENAME only as
// the sole action of an ALTER TABLE, so a column rename is a statement of its
// own and cannot be listed beside an ADD COLUMN.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.alter-table+15/test]
#[test]
fn column_rename_is_its_own_statement() {
    let added = Table::alter(Font::Table)
        .add_column(ColumnDef::new(Name::runtime("new_col")).integer())
        .to_string();
    let renamed =
        Table::rename_column(Font::Table, Font::Name, Name::runtime("name_new")).to_string();

    assert_eq!(
        renamed,
        r#"ALTER TABLE "font" RENAME COLUMN "name" TO "name_new""#
    );
    assert_parses(&added);
    assert_parses(&renamed);
}

// Fixed by plan node `bug.oracle-findings`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: the target is a `Name` rather
// than a `TableName`, so the qualified form a rename cannot honour — the table
// stays in the schema it is already in — no longer typechecks.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.drop-rename-truncate+4/test]
#[test]
fn table_rename_target_is_bare_name() {
    let sql = Table::rename(
        (Name::runtime("schema"), Font::Table),
        Name::runtime("font_new"),
    )
    .to_string();

    assert_eq!(sql, r#"ALTER TABLE "schema"."font" RENAME TO "font_new""#);
    assert_parses(&sql);
}

// Fixed by plan node `bug.oracle-findings`: the `RENAME TO` target is a type
// name, not an enum label, so it leaves the value pipeline and renders as the
// quoted identifier the grammar wants.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.type-alter-drop+7/test]
#[test]
fn alter_type_rename_emits_identifier() {
    let sql = Type::alter(Font::Table)
        .rename_to(Name::runtime("typeface"))
        .to_string();

    assert_eq!(sql, r#"ALTER TYPE "font" RENAME TO "typeface""#);
    assert_parses(&sql);
}

// Fixed by plan node `bug.oracle-findings`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: PostgreSQL takes a precision only
// where the trailing field is SECOND, so the precision rides on the
// second-bearing fields and `interval HOUR(43)` has no spelling to render.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:def:sql.render.ddl.types+8/test]
// [spec:pgorm:def:sql.types.column-type+11/test]
#[test]
fn interval_precision_rides_on_seconds() {
    let hour = Table::create(Glyph::Table)
        .col(ColumnDef::new(Glyph::Aspect).interval(IntervalSpec::Fields(PgInterval::Hour)))
        .to_string();
    let seconds = Table::create(Glyph::Table)
        .col(ColumnDef::new(Glyph::Aspect).interval(IntervalSpec::Fields(
            PgInterval::HourToSecond(Some(IntervalPrecision::P3)),
        )))
        .to_string();

    assert!(hour.contains("interval HOUR"));
    assert!(seconds.contains("interval HOUR TO SECOND(3)"));
    assert_eq!(IntervalPrecision::new(43), None);
    assert_parses(&hour);
    assert_parses(&seconds);
}

// Fixed by plan node `bug.oracle-findings`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: PostgreSQL takes at most one drop
// behaviour, so the two spellings share one slot and the later call wins.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.extension+5/test]
#[test]
fn extension_drop_takes_one_behaviour() {
    let sql = Extension::drop(Name::runtime("ltree"))
        .cascade()
        .restrict()
        .to_string();

    assert_eq!(sql, r#"DROP EXTENSION "ltree" RESTRICT"#);
    assert_parses(&sql);
}

// Half fixed by plan node `bug.oracle-findings`: the extension name and schema
// route through identifier quoting and the version through the string-literal
// escape, so neither an odd version nor an embedded quote escapes the grammar.
// `ColumnDef::extra` stays raw by design — it is the escape hatch for column SQL
// the type vocabulary cannot spell, documented as caller responsibility by
// `sql.ddl.column-def`, so it keeps its pin.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:sem:sql.render.ddl.extension+3/test]
// [spec:pgorm:req:sql.ddl.column-def+13/test]
#[test]
fn oracle_pins_extra_interpolated_raw() {
    let version = Extension::create(Name::runtime("ltree"))
        .version("v0.1.0")
        .to_string();
    let injected = Extension::create(Name::runtime(r#"pg"weird ext"#)).to_string();
    let extra = Table::create(Glyph::Table)
        .col(
            ColumnDef::new(Glyph::Id)
                .integer()
                .raw_suffix("ANYTHING I WANT TO SAY"),
        )
        .to_string();

    assert_eq!(version, r#"CREATE EXTENSION "ltree" VERSION 'v0.1.0'"#);
    assert_eq!(injected, r#"CREATE EXTENSION "pg""weird ext""#);
    assert_parses(&version);
    assert_parses(&injected);
    assert!(assert_rejected(&extra).contains("ANYTHING"));
}

// Fixed by plan node `bug.oracle-findings`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: `NullAlias` existed only as a
// placeholder inside `ColumnDef::take`, which now clones the name, so the empty
// identifier PostgreSQL rejects has no constructor left.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.render.ident-quoting+7/test]
// [spec:pgorm:def:sql.ast.keywords+6/test]
#[test]
fn alias_identifiers_are_never_empty() {
    let sql = Query::select()
        .expr_as(Expr::col(Glyph::Aspect), Name::runtime("ratio"))
        .from(Glyph::Table)
        .to_string();
    let taken = ColumnDef::new(Glyph::Id).integer().take();

    assert_eq!(sql, r#"SELECT "aspect" AS "ratio" FROM "glyph""#);
    assert_eq!(taken.get_column_name(), "id");
    assert_parses(&sql);
}

// Fixed by plan node `bug.oracle-findings`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: ESCAPE left the operator lexicon
// for `SimpleExpr::LikePattern`, the one place the grammar admits it, so it can
// no longer be applied to two arbitrary operands.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:def:sql.render.operators+8/test]
// [spec:pgorm:def:sql.types.opers+6/test]
#[test]
fn escape_renders_only_inside_like() {
    let sql = Query::select()
        .expr(Expr::col(Glyph::Image).like(LikeExpr::new("a%").escape('\\')))
        .to_string();

    assert!(sql.contains(r#"LIKE 'a%' ESCAPE E'\\'"#));
    assert_parses(&sql);
}

// The oracle's ceiling, recorded rather than pinned: this render is grammatical,
// so no parser can catch it. An empty select list is valid PostgreSQL and is
// closed as an ORM-layer error by `sql.empty-select-list`. The other members of
// this family are closed by the type system instead: `money(12, 2)` — which
// only a resolved type modifier rejects — is gone with `ColumnType::Money`'s
// precision argument, a cross-database reference is gone with the
// database-qualified `TableName` form, and `ON CONFLICT DO UPDATE` with no
// inference specification — which parse analysis rejects with "requires
// inference specification or constraint name" — is gone with `OnConflict`'s
// targeted variant.
// [spec:pgorm:req:sql.render.oracle+4/test]
#[test]
fn oracle_records_parse_valid_defects() {
    let empty_select_list = Query::select().from(Glyph::Table).to_string();

    assert_eq!(empty_select_list, r#"SELECT  FROM "glyph""#);
    assert_parses(&empty_select_list);
    assert_parses(
        r#"INSERT INTO "glyph" ("aspect") VALUES (1) ON CONFLICT DO UPDATE SET "aspect" = 1"#,
    );
}

// Fixed by plan node `unrep.ddl-empty-builders`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: PostgreSQL parses neither an
// action-less `ALTER TABLE` nor an empty index column list, so `Table::alter`
// yields a `PendingTableAlter` that only an action turns into a statement, and
// `Index::create` takes the first column. The `No alter option found` panic went
// with the state it guarded — the strings below have no builder left to produce
// them.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.alter-table+15/test]
// [spec:pgorm:req:sql.ddl.index-create+11/test]
// [spec:pgorm:sem:sql.ddl.panics+4/test]
#[test]
fn empty_ddl_collections_do_not_construct() {
    let altered = Table::alter(Font::Table)
        .drop_column(Font::Name)
        .to_string();
    let indexed = Index::create(Glyph::Table, Glyph::Aspect)
        .name(Name::runtime("idx"))
        .to_string();

    assert_eq!(altered, r#"ALTER TABLE "font" DROP COLUMN "name""#);
    assert_eq!(indexed, r#"CREATE INDEX "idx" ON "glyph" ("aspect")"#);
    assert_parses(&altered);
    assert_parses(&indexed);
    assert!(assert_rejected(r#"ALTER TABLE "font""#).contains("end of input"));
    assert_rejected(r#"CREATE INDEX "idx" ON "glyph" ()"#);
    assert_rejected(r#"CREATE TABLE "glyph" ( "id" integer, PRIMARY KEY () )"#);
}

// The oracle's verdict on the third member of the empty-builder family, recorded
// rather than designed out: `CREATE TABLE t ()` is valid PostgreSQL — a table
// with no attributes is a real table — so a column-less create statement stays
// buildable and is documented by `sql.ddl.create-table` instead.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.create-table+16/test]
#[test]
fn create_table_with_no_columns_is_valid() {
    let sql = Table::create(Glyph::Table).to_string();

    assert_eq!(sql, r#"CREATE TABLE "glyph" (  )"#);
    assert_parses(&sql);
}

// Fixed by plan node `unrep.ddl-table-target`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: every DDL statement whose render
// names a target takes that target in its constructor, so the absent name
// PostgreSQL rejects at the token after it has nowhere to come from. The
// `compile_fail` doctests on each statement type prove the constructors refuse.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.create-table+16/test]
// [spec:pgorm:req:sql.ddl.index-create+11/test]
// [spec:pgorm:req:sql.ddl.index-drop+3/test]
// [spec:pgorm:req:sql.ddl.drop-rename-truncate+4/test]
// [spec:pgorm:req:sql.ddl.alter-table+15/test]
// [spec:pgorm:req:sql.ddl.foreign-key+10/test]
#[test]
fn ddl_targets_are_taken_by_construction() {
    let rendered = [
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).integer())
            .to_string(),
        Table::drop(Glyph::Table).to_string(),
        Table::truncate(Glyph::Table).to_string(),
        Table::rename(Glyph::Table, Name::runtime("g")).to_string(),
        Table::rename_column(Glyph::Table, Glyph::Id, Name::runtime("gid")).to_string(),
        Index::create(Glyph::Table, Glyph::Aspect)
            .name(Name::runtime("idx"))
            .to_string(),
        Index::drop(Name::runtime("idx")).to_string(),
        Table::rename_constraint(Char::Table, Name::runtime("fk"), Name::runtime("fk2"))
            .to_string(),
    ];

    for sql in &rendered {
        assert_parses(sql);
    }
    assert_rejected(r#"CREATE TABLE  ( "id" integer )"#);
    assert_rejected("DROP TABLE ");
    assert_rejected("TRUNCATE TABLE ");
    assert_rejected(r#"ALTER TABLE  RENAME TO "g""#);
    assert_rejected(r#"ALTER TABLE "glyph" RENAME COLUMN  TO "gid""#);
    assert_rejected(r#"CREATE INDEX "idx" ON  ("aspect")"#);
    assert_rejected("DROP INDEX ");
    assert_rejected(r#"ALTER TABLE  DROP CONSTRAINT "fk""#);
}

// The two targets the oracle says PostgreSQL does not need, recorded rather than
// required: an index name is derived by the server when `CREATE INDEX` omits it,
// and `DROP INDEX` names a schema-scoped index rather than a table, so both stay
// optional where the rest of the family moved into the constructor.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.index-create+11/test]
// [spec:pgorm:req:sql.ddl.index-drop+3/test]
#[test]
fn index_name_and_drop_table_stay_optional() {
    let unnamed = Index::create(Glyph::Table, Glyph::Aspect).to_string();
    let untabled = Index::drop(Name::runtime("idx")).to_string();

    assert_eq!(unnamed, r#"CREATE INDEX  ON "glyph" ("aspect")"#);
    assert_eq!(untabled, r#"DROP INDEX "idx""#);
    assert_parses(&unnamed);
    assert_parses(&untabled);
}

// Fixed by plan node `unrep.foreign-key-shape`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: a foreign key names both tables
// and at least one `(column, referenced column)` pair by construction, and its
// two column lists are one list of pairs, so `ALTER TABLE  ADD FOREIGN KEY ()`
// and `REFERENCES  ()` have no fields left to come from. An embedded key is
// restamped onto the table it sits inside, as an embedded index is. The
// `compile_fail` doctests on `ForeignKeyCreateStatement` prove the constructor
// refuses the half-named key.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.foreign-key+10/test]
// [spec:pgorm:req:sql.ddl.create-table+16/test]
#[test]
fn foreign_keys_name_two_tables_and_a_pair() {
    let standalone = ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
        .name(Name::runtime("fk"))
        .to_string();
    let composite = ForeignKey::create(Char::Table, Char::FontId, Glyph::Table, Glyph::Id)
        .col(Char::Id, Glyph::Aspect)
        .to_string();
    let mut create = Table::create(Char::Table);
    create
        .col(ColumnDef::new(Char::FontId).integer())
        .foreign_key(ForeignKey::create(
            Font::Table,
            Char::FontId,
            Font::Table,
            Font::Id,
        ));
    let embedded = create.to_string();

    assert_eq!(
        standalone,
        r#"ALTER TABLE "character" ADD CONSTRAINT "fk" FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#
    );
    assert!(
        composite.contains(r#"FOREIGN KEY ("font_id", "id") REFERENCES "glyph" ("id", "aspect")"#)
    );
    assert_eq!(
        create.get_foreign_key_create_stmts()[0]
            .get_foreign_key()
            .get_table(),
        &Char::Table.into_table_name()
    );
    assert_parses(&standalone);
    assert_parses(&composite);
    assert_parses(&embedded);
    assert_rejected(r#"ALTER TABLE  ADD FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#);
    assert_rejected(r#"ALTER TABLE "character" ADD FOREIGN KEY () REFERENCES "font" ("id")"#);
    assert_rejected(r#"ALTER TABLE "character" ADD FOREIGN KEY ("font_id") REFERENCES  ()"#);
    assert_rejected(r#"ALTER TABLE "character" ADD FOREIGN KEY ("font_id") REFERENCES "font" ()"#);
    // The oracle's ceiling on this family: mismatched arity is grammatical and
    // only parse analysis rejects it, so the paired carrier — not the parser —
    // is what closes it.
    assert_parses(
        r#"ALTER TABLE "character" ADD FOREIGN KEY ("font_id", "id") REFERENCES "font" ("id")"#,
    );
}

// Fixed by plan node `unrep.foreign-key-shape`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: every type and extension
// statement takes its name in the constructor, `Type::drop` takes the first of
// its names, and `Type::alter` yields a `PendingTypeAlter` that only an option
// turns into a statement — so the nameless renders, the empty `DROP TYPE` list
// and the option-less `ALTER TYPE` PostgreSQL rejects have nowhere to come
// from. The `compile_fail` doctests on each statement type prove it.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.type-enum+7/test]
// [spec:pgorm:req:sql.ddl.type-alter-drop+7/test]
// [spec:pgorm:req:sql.ddl.extension+5/test]
#[test]
fn type_and_extension_names_are_taken() {
    let rendered = [
        Type::create(Name::runtime("font_family"))
            .values(["serif"])
            .to_string(),
        Type::drop(Name::runtime("font_family")).to_string(),
        Type::alter(Name::runtime("font_family"))
            .add_value("sans")
            .to_string(),
        Extension::create(Name::runtime("ltree")).to_string(),
        Extension::drop(Name::runtime("ltree")).to_string(),
    ];

    for sql in &rendered {
        assert_parses(sql);
    }
    assert_rejected("CREATE TYPE ");
    assert_rejected("DROP TYPE ");
    assert_rejected("ALTER TYPE ");
    assert_rejected(r#"ALTER TYPE "font_family""#);
    assert_rejected(r#"CREATE EXTENSION """#);
    assert_rejected(r#"DROP EXTENSION """#);
}

// The two renders of this family the oracle accepts, recorded rather than
// designed out: `CREATE TYPE "t" AS ENUM ()` is a real empty enumeration and
// `CREATE TYPE "t"` a real shell type. It was the missing parentheses, not the
// missing values, that PostgreSQL rejected, so the list is always parenthesised
// once the type is an enum and both accepted shapes stay buildable.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ddl.type-enum+7/test]
#[test]
fn empty_enum_and_shell_type_are_valid() {
    let shell = Type::create(Name::runtime("font_family")).to_string();
    let empty = Type::create(Name::runtime("font_family"))
        .as_enum()
        .to_string();

    assert_eq!(shell, r#"CREATE TYPE "font_family""#);
    assert_eq!(empty, r#"CREATE TYPE "font_family" AS ENUM ()"#);
    assert_parses(&shell);
    assert_parses(&empty);
    assert_rejected(r#"CREATE TYPE "font_family" AS ENUM"#);
}

// `ANY` / `SOME` / `ALL` are quantifiers over the right operand of a comparison,
// not functions, so `Func::any`'s render is only SQL once something compares to
// it: PostgreSQL rejects the bare projection at the quantifier itself. Nothing
// to fix — the comparison is the whole construct — so this pin records the
// boundary that `eq_any` / `ne_all` keep callers on the right side of.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.ast.expr.eq-any+2/test]
#[test]
fn quantifiers_are_valid_only_after_a_comparison() {
    let compared = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .and_where(Expr::col(Glyph::Aspect).eq_any([1, 2]))
        .to_string();

    assert_eq!(
        compared,
        r#"SELECT "id" FROM "glyph" WHERE "aspect" = ANY(ARRAY [1,2])"#
    );
    assert_parses(&compared);

    let bare = Query::select()
        .expr(Func::any(Value::array([1, 2])))
        .from(Glyph::Table)
        .to_string();

    assert_eq!(bare, r#"SELECT ANY(ARRAY [1,2]) FROM "glyph""#);
    assert_rejected(&bare);
}

// Fixed by plan node `searched-case-needs-arm`, at the type level per
// [dec:pgorm:invalid-states-unrepresentable]: PostgreSQL's grammar requires at
// least one `WHEN`, and neither form can now be begun without one. The searched
// form's only constructor, `Expr::case`, takes its first arm, and the simple
// form's `Expr::case_of` yields a `CaseOperand` that converts into nothing
// until its first `when`. The armless `(CASE ELSE 'x' END)` this was pinned to
// is proved unbuildable by the `compile_fail` doctests on `CaseStatement`.
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:def:sql.ast.case+3/test]
#[test]
fn a_case_needs_at_least_one_arm() {
    let searched = Query::select()
        .expr(Expr::case(Expr::col(Glyph::Aspect).gt(1), "big").finally("x"))
        .to_string();

    assert_eq!(
        searched,
        r#"SELECT (CASE WHEN ("aspect" > 1) THEN 'big' ELSE 'x' END)"#
    );
    let case = &crate::oracle::parsed_nodes(&searched, "CaseExpr")[0];
    assert_eq!(case["args"].as_array().map(Vec::len), Some(1), "{searched}");

    let simple = Query::select()
        .expr(
            Expr::case_of(Expr::col(Glyph::Aspect))
                .when(1, "one")
                .finally("x"),
        )
        .to_string();
    assert_parses(&simple);
}
