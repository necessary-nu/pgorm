//! Keys, foreign keys, identities, ALTER TABLE's actions, index entries, types,
//! sequences, extensions and comments, built with pgorm-query's DDL builders
//! directly, must match the SQL Python builds for them.
use pgorm::pgorm_query::{
    BinOper, Check, ColumnDef, ColumnType, Comment, ConstraintChange, ConstraintDrop,
    Deferrability, Domain, Enforcement, Expr, ExprTrait, ForeignKeyAction,
    ForeignKeyCreateStatement, Func, GeneratedKind, IdentityGeneration, Index, IndexColumn,
    IndexOrder, IndexType, MaterializedView, Name, NotNullConstraint, Query, Schema,
    SelectStatement, Sequence, SequenceOption, SequenceType, SimpleExpr, StringLen, Table,
    TableKey, TableName, TypeName, Unique, Values, View, ViewCheckOption, error,
    extension::{Extension, RangeDefinition, Type, TypeRef},
};
use pgorm_python::expressions::Compiled;
use pyo3::prelude::*;
use rust_decimal::Decimal;
use std::{collections::BTreeMap, ffi::CString, str::FromStr, sync::Arc};

fn a(name: &str) -> Name {
    Name::runtime(name)
}

fn col(name: &str) -> Expr {
    Expr::col(a(name))
}

fn app(name: &str) -> TableName {
    TableName::SchemaTable(a("app"), a(name))
}

fn mood() -> TypeRef {
    TypeRef::SchemaType(a("app"), a("mood"))
}

fn mood_column() -> ColumnType {
    ColumnType::Enum {
        name: a("mood"),
        schema: Some(a("app")),
        variants: Vec::new(),
    }
}

fn typed(name: &str, kind: ColumnType) -> ColumnDef {
    ColumnDef::new_with_type(a(name), kind)
}

fn create_table_columns() -> String {
    let mut identity = typed("id", ColumnType::BigInteger);
    identity.identity_with(
        IdentityGeneration::Always,
        SequenceOption::IncrementBy(2)
            .and(SequenceOption::NoMaxValue)
            .and(SequenceOption::StartWith(10)),
    );
    let mut seats = typed("seats", ColumnType::SmallInteger);
    seats
        .not_null()
        .not_null_named(a("seats_present"))
        .not_null_no_inherit()
        .default(Expr::val(1i64))
        .check(
            Check::new(col("seats").gt(0i64))
                .name(a("seats_positive"))
                .enforcement(Enforcement::NotEnforced),
        );
    let mut note = typed("note", ColumnType::Text);
    note.null().collate((a("pg_catalog"), a("C")));
    let price = Decimal::from_str("9.50").unwrap_or_default();
    Table::create(app("booking"))
        .col(identity)
        .col(typed("legacy", ColumnType::Integer).auto_increment())
        .col(seats)
        .col(note)
        .col(typed("price", ColumnType::Decimal(Some((10, 2)))).default(Expr::val(price)))
        .col(
            typed("doubled", ColumnType::Integer)
                .generated(col("seats").mul(2i64), GeneratedKind::Stored),
        )
        .col(
            typed("halved", ColumnType::Integer)
                .generated(col("seats").div(2i64), GeneratedKind::Virtual),
        )
        .col(typed("serial", ColumnType::BigInteger).identity_by_default())
        .to_string()
}

fn create_table_keys() -> String {
    let mut by_room =
        ForeignKeyCreateStatement::new(app("booking"), a("room"), app("room"), a("id"));
    by_room
        .col(a("id"), a("booking"))
        .name(a("booking_room"))
        .on_delete(ForeignKeyAction::Cascade)
        .on_update(ForeignKeyAction::SetNull)
        .deferrability(Deferrability::DeferrableInitiallyDeferred)
        .enforcement(Enforcement::NotEnforced);
    let mut by_period =
        ForeignKeyCreateStatement::new(app("booking"), a("room"), app("room"), a("id"));
    by_period.period(a("during"), a("open"));
    Table::create(app("booking"))
        .if_not_exists()
        .col(typed("id", ColumnType::BigInteger))
        .col(typed("room", ColumnType::Integer))
        .col(typed(
            "during",
            ColumnType::Range(pgorm::pgorm_query::RangeType::TimestampTz),
        ))
        .primary_key(
            TableKey::new(a("id"))
                .name(a("booking_key"))
                .include([a("room")])
                .deferrability(Deferrability::DeferrableInitiallyImmediate),
        )
        .unique(
            TableKey::<Unique>::new(a("room"))
                .col(a("id"))
                .nulls_not_distinct(),
        )
        .unique(
            TableKey::<Unique>::new(a("room"))
                .name(a("no_double_booking"))
                .without_overlaps(a("during")),
        )
        .foreign_key(by_room)
        .foreign_key(by_period)
        .check(
            Check::new(col("room").lt(1000i64))
                .name(a("room_small"))
                .no_inherit()
                .enforcement(Enforcement::Enforced),
        )
        .to_string()
}

fn create_table_typed_columns() -> String {
    let mut mood_type = TypeName::new(a("mood"));
    mood_type.schema = Some(a("app"));
    let calm: SimpleExpr = Expr::val("calm").cast_as_type(mood_type);
    Table::create(TableName::Table(a("feeling")))
        .col(typed("mood", mood_column()).default(calm))
        .col(typed("moods", ColumnType::Array(Arc::new(mood_column()))))
        .col(typed(
            "tags",
            ColumnType::Array(Arc::new(ColumnType::String(StringLen::N(20)))),
        ))
        .col(typed(
            "span",
            ColumnType::CreatedRange {
                name: a("floatrange"),
                schema: Some(a("app")),
                subtype: Arc::new(ColumnType::Double),
            },
        ))
        .col(typed("token", ColumnType::Uuid).default(Func::gen_random_uuid()))
        .to_string()
}

fn alter_table_actions() -> String {
    let mut room_fk =
        ForeignKeyCreateStatement::new(app("booking"), a("room"), app("room"), a("id"));
    room_fk
        .name(a("room_fk"))
        .on_delete(ForeignKeyAction::Restrict);
    let mut extra = typed("extra", ColumnType::Text);
    extra.not_null();
    Table::alter(app("booking"))
        .add_column_if_not_exists(extra)
        .drop_column(a("legacy"))
        .add_primary_key(TableKey::new(a("id")).name(a("booking_pkey")))
        .add_unique(
            TableKey::<Unique>::new(a("room"))
                .nulls_not_distinct()
                .deferrability(Deferrability::NotDeferrable),
        )
        .add_foreign_key(room_fk.get_foreign_key().clone().not_valid())
        .add_check(
            Check::new(col("seats").lte(9i64))
                .name(a("few_seats"))
                .no_inherit()
                .not_valid(),
        )
        .add_not_null(
            NotNullConstraint::new(a("note"))
                .name(a("note_present"))
                .no_inherit()
                .not_valid(),
        )
        .drop_constraint(ConstraintDrop::new(a("old")).if_exists().cascade())
        .validate_constraint(a("room_fk"))
        .alter_constraint(a("room_fk"), ConstraintChange::NotEnforced)
        .alter_constraint(a("note_present"), ConstraintChange::Inherit)
        .set_expression(a("doubled"), col("seats").mul(3i64))
        .drop_expression_if_exists(a("halved"))
        .to_string()
}

fn alter_table_modify_column() -> String {
    let mut note = typed("note", ColumnType::String(StringLen::None));
    note.collate(a("C")).default(Expr::val("none")).not_null();
    let mut seats = ColumnDef::new(a("seats"));
    seats.null();
    let mut room = ColumnDef::new(a("room"));
    room.not_null().not_null_named(a("room_present"));
    let mut serial = ColumnDef::new(a("serial"));
    serial.identity_with(IdentityGeneration::Always, SequenceOption::Cache(5));
    let mut price = ColumnDef::new(a("price"));
    price.check(col("price").gte(0i64));
    Table::alter(app("booking"))
        .modify_column(note)
        .modify_column(seats)
        .modify_column(room)
        .modify_column(serial)
        .modify_column(price)
        .to_string()
}

fn tables() -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        ("create_table_columns", create_table_columns()),
        ("create_table_keys", create_table_keys()),
        ("create_table_typed_columns", create_table_typed_columns()),
        ("alter_table_actions", alter_table_actions()),
        ("alter_table_modify_column", alter_table_modify_column()),
        (
            "drop_tables",
            Table::drop(app("booking"))
                .table(TableName::Table(a("scratch")))
                .if_exists()
                .restrict()
                .to_string(),
        ),
    ])
}

fn indexes() -> BTreeMap<&'static str, String> {
    let lowered = IndexColumn::expr(Func::lower(col("note")))
        .operator_class(a("text_pattern_ops"))
        .order(IndexOrder::Desc);
    let body: SimpleExpr = col("body").into();
    let tags: SimpleExpr = col("tags").into();
    BTreeMap::from([
        (
            "create_index_entries",
            Index::create(app("booking"), lowered)
                .name(a("booking_note"))
                .col(IndexColumn::name(a("room")))
                .col(IndexColumn::name(a("id")).order(IndexOrder::Asc))
                .unique()
                .nulls_not_distinct()
                .if_not_exists()
                .include([a("seats")])
                .cond_where(col("seats").gt(0i64))
                .cond_where(col("room").is_null().not())
                .to_string(),
        ),
        (
            "create_index_methods",
            Index::create(TableName::Table(a("doc")), IndexColumn::expr(body))
                .index_type(IndexType::Gin)
                .col(IndexColumn::expr(tags))
                .to_string(),
        ),
        (
            "create_index_access_method",
            Index::create(app("booking"), IndexColumn::name(a("during")))
                .index_type(IndexType::Named(a("gist")))
                .to_string(),
        ),
        (
            "drop_index",
            Index::drop(a("booking_note"))
                .table(app("booking"))
                .if_exists()
                .to_string(),
        ),
    ])
}

fn types() -> BTreeMap<&'static str, String> {
    let pair = || TypeRef::Type(a("pair"));
    BTreeMap::from([
        (
            "create_shell_type",
            Type::create(TypeRef::Type(a("later"))).to_string(),
        ),
        (
            "create_enum",
            Type::create(mood())
                .values(["calm", "O'Brien"])
                .values([""])
                .to_string(),
        ),
        (
            "create_composite",
            Type::create(pair())
                .attribute(a("a"), ColumnType::Integer)
                .attribute_collated(a("b"), ColumnType::Text, (a("pg_catalog"), a("C")))
                .attribute(a("c"), mood_column())
                .to_string(),
        ),
        (
            "create_empty_composite",
            Type::create(TypeRef::Type(a("nothing")))
                .values(["x"])
                .as_composite()
                .to_string(),
        ),
        (
            "create_range",
            Type::create(TypeRef::SchemaType(a("app"), a("floatrange")))
                .as_range(
                    RangeDefinition::new(ColumnType::Double)
                        .subtype_diff(a("float8mi"))
                        .subtype_opclass(a("float8_ops"))
                        .multirange_type_name(TypeRef::SchemaType(a("app"), a("floatmultirange"))),
                )
                .to_string(),
        ),
        (
            "create_text_range",
            Type::create(TypeRef::Type(a("textrange")))
                .as_range(RangeDefinition::new(ColumnType::Text).collation(a("C")))
                .to_string(),
        ),
        (
            "alter_enum_add_value",
            Type::alter(mood())
                .add_value("tense")
                .after("calm")
                .to_string(),
        ),
        (
            "alter_composite",
            Type::alter(pair())
                .add_attribute(a("d"), ColumnType::BigInteger)
                .add_attribute_collated(a("e"), ColumnType::Text, a("C"))
                .drop_attribute_if_exists(a("a"))
                .drop_attribute(a("b"))
                .alter_attribute_collated(a("c"), ColumnType::Text, a("C"))
                .alter_attribute(a("d"), ColumnType::Integer)
                .cascade()
                .to_string(),
        ),
        (
            "rename_attribute",
            Type::alter(pair())
                .rename_attribute(a("a"), a("z"))
                .restrict()
                .to_string(),
        ),
        (
            "drop_types",
            Type::drop(mood())
                .name(pair())
                .if_exists()
                .cascade()
                .to_string(),
        ),
    ])
}

fn sequences() -> BTreeMap<&'static str, String> {
    let ticket = || TableName::Table(a("ticket"));
    BTreeMap::from([
        (
            "create_sequence",
            Sequence::create(app("ticket"))
                .if_not_exists()
                .as_type(SequenceType::Integer)
                .options(
                    SequenceOption::IncrementBy(5)
                        .and(SequenceOption::NoMinValue)
                        .and(SequenceOption::MaxValue(9_007_199_254_740_991))
                        .and(SequenceOption::StartWith(-3))
                        .and(SequenceOption::Cache(2)),
                )
                .options(SequenceOption::Cycle.and(SequenceOption::MinValue(1)))
                .owned_by(app("booking"), a("id"))
                .to_string(),
        ),
        (
            "alter_sequence",
            Sequence::alter(ticket())
                .restart_with(100)
                .if_exists()
                .as_type(SequenceType::BigInteger)
                .options(SequenceOption::NoCycle)
                .owned_by_none()
                .to_string(),
        ),
        (
            "alter_sequence_restart",
            Sequence::alter(ticket()).restart().to_string(),
        ),
        (
            "drop_sequences",
            Sequence::drop(ticket())
                .name(app("other"))
                .cascade()
                .to_string(),
        ),
        (
            "rename_sequence",
            Sequence::rename(app("ticket"), a("voucher")).to_string(),
        ),
        (
            "create_extension",
            Extension::create(a("btree_gist"))
                .if_not_exists()
                .schema(a("public"))
                .version("1.7")
                .cascade()
                .to_string(),
        ),
        (
            "drop_extension",
            Extension::drop(a("btree_gist"))
                .if_exists()
                .restrict()
                .to_string(),
        ),
    ])
}

/// The bookings of a room above 10 whose note is not `it's`: a body with a
/// number and a string, each written as a literal.
fn large_rooms() -> SelectStatement {
    Query::select()
        .expr(col("id"))
        .expr(col("room"))
        .from(app("booking"))
        .cond_where(col("room").binary(BinOper::GreaterThan, 10i64))
        .cond_where(col("note").binary(BinOper::NotEqual, "it's"))
        .to_owned()
}

fn programs() -> error::Result<BTreeMap<&'static str, String>> {
    let others = BTreeMap::from([
        (
            "create_view_repeated",
            View::create(app("large_room"), large_rooms())?
                .or_replace()
                .columns([a("booking"), a("room_no")])
                .check_option(ViewCheckOption::Local)
                .check_option(ViewCheckOption::Cascaded)
                .security_barrier()
                .security_invoker()
                .to_string(),
        ),
        (
            "drop_views",
            View::drop(TableName::Table(a("large_room")))
                .name(app("other"))
                .if_exists()
                .cascade()
                .to_string(),
        ),
        (
            "drop_materialized_views",
            MaterializedView::drop(app("room_total"))
                .name(TableName::Table(a("other")))
                .restrict()
                .to_string(),
        ),
        (
            "drop_schema_restrict",
            Schema::drop(a("app")).restrict().to_string(),
        ),
        (
            "drop_domains_restrict",
            Domain::drop(a("quantity"))
                .name(TypeRef::SchemaType(a("app"), a("quantity")))
                .if_exists()
                .restrict()
                .to_string(),
        ),
        (
            "comment_on_table",
            Comment::on_table(app("booking"), "Rooms booked, and when").to_string(),
        ),
        (
            "comment_on_column",
            Comment::on_column(app("booking"), a("note"), "it's a \\ note\nover two lines")
                .to_string(),
        ),
    ]);
    Ok(tables()
        .into_iter()
        .chain(indexes())
        .chain(types())
        .chain(sequences())
        .chain(others)
        .collect())
}

// [spec:pgorm:req:bindings.schema-tables/test]    keys with INCLUDE, deferrability and WITHOUT
// OVERLAPS, foreign keys with every option, identity columns and comments render as the Rust
// builders render them
// [spec:pgorm:req:bindings.schema-alter/test]    one ALTER TABLE holding every action, and
// modify_column's aspects, render as the Rust builder renders them
// [spec:pgorm:req:bindings.schema-indexes/test]    index entries over expressions with an order
// and operator class, INCLUDE, a partial predicate and an access method render as Rust's
// [spec:pgorm:req:bindings.schema-types/test]    shell, enum, composite and range types, ALTER
// TYPE's changes and DROP TYPE render as the Rust builders render them
// [spec:pgorm:req:bindings.schema-sequences/test]    sequences, extensions and comments render as
// the Rust builders render them
#[test]
fn objects_match_native_rust() -> Result<(), Box<dyn std::error::Error>> {
    Python::initialize();
    Python::attach(|py| -> Result<(), Box<dyn std::error::Error>> {
        let native = PyModule::new(py, "pgorm._native")?;
        pgorm_python::install(&native, Default::default())?;
        let source = CString::new(include_str!("schema_object_programs.py"))?;
        let examples = PyModule::from_code(
            py,
            &source,
            c"schema_object_programs.py",
            c"schema_object_programs",
        )?;
        let queries = examples.call_method1("programs", (&native,))?;
        let rust = programs()?;
        assert_eq!(queries.len()?, rust.len());
        for (name, sql) in rust {
            let query = queries.get_item(name)?;
            let compiled: Compiled = query
                .call_method0("inspect")?
                .extract()
                .map_err(PyErr::from)?;
            assert_eq!(compiled.sql, sql, "{name}");
            assert_eq!(compiled.values, Values(vec![]), "{name}");
        }
        Ok(())
    })
}
