//! The schema family's cases, built with pgorm-query's DDL builders directly.
//! A DDL statement renders its values as literals and binds none.

use std::{str::FromStr, sync::Arc};

use pgorm::pgorm_query::{
    Check, ColumnDef, ColumnType, ConstraintChange, ConstraintDrop, Deferrability, Domain,
    DomainCheck, Enforcement, Expr, ExprTrait, ForeignKeyAction, ForeignKeyCreateStatement, Func,
    GeneratedKind, IdentityGeneration, Index, IndexColumn, IndexOrder, IndexType,
    NotNullConstraint, Schema, Sequence, SequenceOption, SequenceType, SimpleExpr, StringLen,
    Table, TableKey, TableName, TypeName, Unique, Values,
    extension::{Extension, RangeDefinition, Type, TypeRef},
};
use rust_decimal::Decimal;

use super::n;

macro_rules! col {
    ($name:expr) => {
        Expr::col(n($name))
    };
}

macro_rules! booking {
    () => {
        TableName::SchemaTable(n("app"), n("booking"))
    };
}

macro_rules! room {
    () => {
        TableName::SchemaTable(n("app"), n("room"))
    };
}

macro_rules! mood {
    () => {
        TypeRef::SchemaType(n("app"), n("mood"))
    };
}

fn mood_column() -> ColumnType {
    ColumnType::Enum {
        name: n("mood"),
        schema: Some(n("app")),
        variants: Vec::new(),
    }
}

/// The domain the domain cases name, qualified by its schema.
fn quantity() -> TypeRef {
    TypeRef::SchemaType(n("app"), n("quantity"))
}

fn ddl(statement: impl ToString) -> (String, Values) {
    (statement.to_string(), Values(Vec::new()))
}

macro_rules! typed {
    ($name:expr, $kind:expr $(,)?) => {
        ColumnDef::new_with_type(n($name), $kind)
    };
}

fn create_table_columns() -> (String, Values) {
    let mut identity = typed!("id", ColumnType::BigInteger);
    identity.identity_with(
        IdentityGeneration::Always,
        SequenceOption::IncrementBy(2)
            .and(SequenceOption::NoMaxValue)
            .and(SequenceOption::StartWith(10)),
    );
    let mut seats = typed!("seats", ColumnType::SmallInteger);
    seats
        .not_null()
        .not_null_named(n("seats_present"))
        .not_null_no_inherit()
        .default(Expr::val(1i64))
        .check(
            Check::new(col!("seats").gt(0i64))
                .name(n("seats_positive"))
                .enforcement(Enforcement::NotEnforced),
        );
    let mut note = typed!("note", ColumnType::Text);
    note.null().collate((n("pg_catalog"), n("C")));
    let price = Decimal::from_str("9.50").expect("a decimal");
    ddl(Table::create(booking!())
        .col(identity)
        .col(typed!("legacy", ColumnType::Integer).auto_increment())
        .col(seats)
        .col(note)
        .col(typed!("price", ColumnType::Decimal(Some((10, 2)))).default(Expr::val(price)))
        .col(
            typed!("doubled", ColumnType::Integer)
                .generated(col!("seats").mul(2i64), GeneratedKind::Stored),
        )
        .col(
            typed!("halved", ColumnType::Integer)
                .generated(col!("seats").div(2i64), GeneratedKind::Virtual),
        )
        .col(typed!("serial", ColumnType::BigInteger).identity_by_default())
        .to_owned())
}

fn create_table_keys() -> (String, Values) {
    let mut by_room = ForeignKeyCreateStatement::new(booking!(), n("room"), room!(), n("id"));
    by_room
        .col(n("id"), n("booking"))
        .name(n("booking_room"))
        .on_delete(ForeignKeyAction::Cascade)
        .on_update(ForeignKeyAction::SetNull)
        .deferrability(Deferrability::DeferrableInitiallyDeferred)
        .enforcement(Enforcement::NotEnforced);
    let mut by_period = ForeignKeyCreateStatement::new(booking!(), n("room"), room!(), n("id"));
    by_period.period(n("during"), n("open"));
    ddl(Table::create(booking!())
        .if_not_exists()
        .col(typed!("id", ColumnType::BigInteger))
        .col(typed!("room", ColumnType::Integer))
        .col(typed!(
            "during",
            ColumnType::Range(pgorm::pgorm_query::RangeType::TimestampTz),
        ))
        .primary_key(
            TableKey::new(n("id"))
                .name(n("booking_key"))
                .include([n("room")])
                .deferrability(Deferrability::DeferrableInitiallyImmediate),
        )
        .unique(
            TableKey::<Unique>::new(n("room"))
                .col(n("id"))
                .nulls_not_distinct(),
        )
        .unique(
            TableKey::<Unique>::new(n("room"))
                .name(n("no_double_booking"))
                .without_overlaps(n("during")),
        )
        .foreign_key(by_room)
        .foreign_key(by_period)
        .check(
            Check::new(col!("room").lt(1000i64))
                .name(n("room_small"))
                .no_inherit()
                .enforcement(Enforcement::Enforced),
        )
        .to_owned())
}

fn create_table_typed_columns() -> (String, Values) {
    let mut mood_type = TypeName::new(n("mood"));
    mood_type.schema = Some(n("app"));
    let calm: SimpleExpr = Expr::val("calm").cast_as_type(mood_type);
    ddl(Table::create(TableName::Table(n("feeling")))
        .col(typed!("mood", mood_column()).default(calm))
        .col(typed!("moods", ColumnType::Array(Arc::new(mood_column()))))
        .col(typed!(
            "tags",
            ColumnType::Array(Arc::new(ColumnType::String(StringLen::N(20)))),
        ))
        .col(typed!(
            "span",
            ColumnType::CreatedRange {
                name: n("floatrange"),
                schema: Some(n("app")),
                subtype: Arc::new(ColumnType::Double),
            },
        ))
        .col(typed!("token", ColumnType::Uuid).default(Func::gen_random_uuid()))
        .to_owned())
}

fn alter_table_actions() -> (String, Values) {
    let mut room_fk = ForeignKeyCreateStatement::new(booking!(), n("room"), room!(), n("id"));
    room_fk
        .name(n("room_fk"))
        .on_delete(ForeignKeyAction::Restrict);
    let mut extra = typed!("extra", ColumnType::Text);
    extra.not_null();
    ddl(Table::alter(booking!())
        .add_column_if_not_exists(extra)
        .drop_column(n("legacy"))
        .add_primary_key(TableKey::new(n("id")).name(n("booking_pkey")))
        .add_unique(
            TableKey::<Unique>::new(n("room"))
                .nulls_not_distinct()
                .deferrability(Deferrability::NotDeferrable),
        )
        .add_foreign_key(room_fk.get_foreign_key().clone().not_valid())
        .add_check(
            Check::new(col!("seats").lte(9i64))
                .name(n("few_seats"))
                .no_inherit()
                .not_valid(),
        )
        .add_not_null(
            NotNullConstraint::new(n("note"))
                .name(n("note_present"))
                .no_inherit()
                .not_valid(),
        )
        .drop_constraint(ConstraintDrop::new(n("old")).if_exists().cascade())
        .validate_constraint(n("room_fk"))
        .alter_constraint(n("room_fk"), ConstraintChange::NotEnforced)
        .alter_constraint(n("note_present"), ConstraintChange::Inherit)
        .set_expression(n("doubled"), col!("seats").mul(3i64))
        .drop_expression_if_exists(n("halved"))
        .to_owned())
}

fn alter_table_modify_column() -> (String, Values) {
    let mut note = typed!("note", ColumnType::String(StringLen::None));
    note.collate(n("C")).default(Expr::val("none")).not_null();
    let mut seats = ColumnDef::new(n("seats"));
    seats.null();
    let mut room_column = ColumnDef::new(n("room"));
    room_column.not_null().not_null_named(n("room_present"));
    let mut serial = ColumnDef::new(n("serial"));
    serial.identity_with(IdentityGeneration::Always, SequenceOption::Cache(5));
    let mut price = ColumnDef::new(n("price"));
    price.check(col!("price").gte(0i64));
    ddl(Table::alter(booking!())
        .modify_column(note)
        .modify_column(seats)
        .modify_column(room_column)
        .modify_column(serial)
        .modify_column(price)
        .to_owned())
}

fn drop_tables() -> (String, Values) {
    ddl(Table::drop(booking!())
        .table(TableName::Table(n("scratch")))
        .if_exists()
        .restrict()
        .to_owned())
}

fn create_index_entries() -> (String, Values) {
    let lowered = IndexColumn::expr(Func::lower(col!("note")))
        .operator_class(n("text_pattern_ops"))
        .order(IndexOrder::Desc);
    ddl(Index::create(booking!(), lowered)
        .name(n("booking_note"))
        .col(IndexColumn::name(n("room")))
        .col(IndexColumn::name(n("id")).order(IndexOrder::Asc))
        .unique()
        .nulls_not_distinct()
        .if_not_exists()
        .include([n("seats")])
        .cond_where(col!("seats").gt(0i64))
        .cond_where(col!("room").is_null().not())
        .to_owned())
}

fn create_index_methods() -> (String, Values) {
    let body: SimpleExpr = col!("body").into();
    let tags: SimpleExpr = col!("tags").into();
    ddl(
        Index::create(TableName::Table(n("doc")), IndexColumn::expr(body))
            .index_type(IndexType::Gin)
            .col(IndexColumn::expr(tags))
            .to_owned(),
    )
}

fn create_index_access_method() -> (String, Values) {
    ddl(Index::create(booking!(), IndexColumn::name(n("during")))
        .index_type(IndexType::Named(n("gist")))
        .to_owned())
}

fn drop_index() -> (String, Values) {
    ddl(Index::drop(n("booking_note"))
        .table(booking!())
        .if_exists()
        .to_owned())
}

fn create_composite() -> (String, Values) {
    ddl(Type::create(TypeRef::Type(n("pair")))
        .attribute(n("a"), ColumnType::Integer)
        .attribute_collated(n("b"), ColumnType::Text, (n("pg_catalog"), n("C")))
        .attribute(n("c"), mood_column())
        .to_owned())
}

fn create_range() -> (String, Values) {
    ddl(Type::create(TypeRef::SchemaType(n("app"), n("floatrange")))
        .as_range(
            RangeDefinition::new(ColumnType::Double)
                .subtype_diff(n("float8mi"))
                .subtype_opclass(n("float8_ops"))
                .multirange_type_name(TypeRef::SchemaType(n("app"), n("floatmultirange"))),
        )
        .to_owned())
}

fn alter_composite() -> (String, Values) {
    ddl(Type::alter(TypeRef::Type(n("pair")))
        .add_attribute(n("d"), ColumnType::BigInteger)
        .add_attribute_collated(n("e"), ColumnType::Text, n("C"))
        .drop_attribute_if_exists(n("a"))
        .drop_attribute(n("b"))
        .alter_attribute_collated(n("c"), ColumnType::Text, n("C"))
        .alter_attribute(n("d"), ColumnType::Integer)
        .cascade())
}

fn create_sequence() -> (String, Values) {
    ddl(
        Sequence::create(TableName::SchemaTable(n("app"), n("ticket")))
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
            .owned_by(booking!(), n("id"))
            .to_owned(),
    )
}

fn alter_sequence() -> (String, Values) {
    ddl(Sequence::alter(TableName::Table(n("ticket")))
        .restart_with(100)
        .if_exists()
        .as_type(SequenceType::BigInteger)
        .options(SequenceOption::NoCycle)
        .owned_by_none()
        .to_owned())
}

/// Every case, named as the golden file names it.
pub(super) fn cases() -> Vec<(&'static str, (String, Values))> {
    let ticket = TableName::Table(n("ticket"));
    vec![
        ("create-table-columns", create_table_columns()),
        ("create-table-keys", create_table_keys()),
        ("create-table-typed-columns", create_table_typed_columns()),
        ("alter-table-actions", alter_table_actions()),
        ("alter-table-modify-column", alter_table_modify_column()),
        ("drop-tables", drop_tables()),
        (
            "rename-table",
            ddl(Table::rename(booking!(), n("reservation"))),
        ),
        (
            "rename-column",
            ddl(Table::rename_column(booking!(), n("note"), n("remark"))),
        ),
        (
            "rename-constraint",
            ddl(Table::rename_constraint(
                booking!(),
                n("room_fk"),
                n("booking_room_fk"),
            )),
        ),
        ("truncate-table", ddl(Table::truncate(booking!()))),
        ("create-index-entries", create_index_entries()),
        ("create-index-methods", create_index_methods()),
        ("create-index-access-method", create_index_access_method()),
        ("drop-index", drop_index()),
        (
            "create-shell-type",
            ddl(Type::create(TypeRef::Type(n("later")))),
        ),
        (
            "create-enum",
            ddl(Type::create(mood!())
                .values(["calm", "O'Brien"])
                .values([""])
                .to_owned()),
        ),
        ("create-composite", create_composite()),
        (
            "create-empty-composite",
            ddl(Type::create(TypeRef::Type(n("nothing")))
                .values(["x"])
                .as_composite()
                .to_owned()),
        ),
        ("create-range", create_range()),
        (
            "create-text-range",
            ddl(Type::create(TypeRef::Type(n("textrange")))
                .as_range(RangeDefinition::new(ColumnType::Text).collation(n("C")))
                .to_owned()),
        ),
        (
            "alter-enum-add-value",
            ddl(Type::alter(mood!()).add_value("tense").after("calm")),
        ),
        (
            "alter-enum-rename-value",
            ddl(Type::alter(mood!()).rename_value("calm", "serene")),
        ),
        (
            "alter-type-rename",
            ddl(Type::alter(mood!()).rename_to(n("feeling"))),
        ),
        ("alter-composite", alter_composite()),
        (
            "rename-attribute",
            ddl(Type::alter(TypeRef::Type(n("pair")))
                .rename_attribute(n("a"), n("z"))
                .restrict()),
        ),
        (
            "drop-types",
            ddl(Type::drop(mood!())
                .name(TypeRef::Type(n("pair")))
                .if_exists()
                .cascade()
                .to_owned()),
        ),
        ("create-sequence", create_sequence()),
        ("alter-sequence", alter_sequence()),
        (
            "alter-sequence-restart",
            ddl(Sequence::alter(ticket.clone()).restart()),
        ),
        (
            "drop-sequences",
            ddl(Sequence::drop(ticket)
                .name(TableName::SchemaTable(n("app"), n("other")))
                .cascade()
                .to_owned()),
        ),
        (
            "rename-sequence",
            ddl(Sequence::rename(
                TableName::SchemaTable(n("app"), n("ticket")),
                n("voucher"),
            )),
        ),
        (
            "create-extension",
            ddl(Extension::create(n("btree_gist"))
                .if_not_exists()
                .schema(n("public"))
                .version("1.7")
                .cascade()
                .to_owned()),
        ),
        (
            "drop-extension",
            ddl(Extension::drop(n("btree_gist"))
                .if_exists()
                .restrict()
                .to_owned()),
        ),
        ("create-schema", ddl(Schema::create(n("app")))),
        (
            "create-schema-if-not-exists",
            ddl(Schema::create(n("App \"x\"")).if_not_exists().to_owned()),
        ),
        (
            "drop-schema",
            ddl(Schema::drop(n("app")).restrict().to_owned()),
        ),
        (
            "drop-schemas",
            ddl(Schema::drop(n("app"))
                .name(n("archive"))
                .if_exists()
                .cascade()
                .to_owned()),
        ),
        (
            "rename-schema",
            ddl(Schema::rename(n("app"), n("App \"x\""))),
        ),
        (
            "create-domain",
            ddl(Domain::create(quantity(), ColumnType::Integer)
                .check(DomainCheck::new(Expr::domain_value().gt(0)).name(n("positive")))
                .not_null()
                .default(1)
                .check(Expr::domain_value().lt(1000))
                .to_owned()),
        ),
        (
            "create-domain-collated",
            ddl(Domain::create(n("Code \"x\""), ColumnType::Text)
                .collate((n("pg_catalog"), n("C")))
                .null()
                .default("none")
                .to_owned()),
        ),
        (
            "create-domain-over-types",
            ddl(Domain::create(
                n("tags"),
                ColumnType::Array(Arc::new(ColumnType::String(StringLen::N(20)))),
            )),
        ),
        (
            "create-domain-over-domain",
            ddl(Domain::create(
                n("small"),
                ColumnType::Enum {
                    name: n("quantity"),
                    schema: Some(n("app")),
                    variants: Vec::new(),
                },
            )
            .check(Expr::domain_value().between(0, 10))
            .to_owned()),
        ),
        (
            "alter-domain-set-default",
            ddl(Domain::alter(quantity()).set_default(5)),
        ),
        (
            "alter-domain-drop-default",
            ddl(Domain::alter(quantity()).drop_default()),
        ),
        (
            "alter-domain-set-not-null",
            ddl(Domain::alter(quantity()).set_not_null()),
        ),
        (
            "alter-domain-drop-not-null",
            ddl(Domain::alter(quantity()).drop_not_null()),
        ),
        (
            "alter-domain-add-check",
            ddl(Domain::alter(quantity()).add_check(Expr::domain_value().ne(13))),
        ),
        (
            "alter-domain-add-check-not-valid",
            ddl(Domain::alter(quantity()).add_check(
                DomainCheck::new(Expr::domain_value().lt(100))
                    .name(n("Small \"x\""))
                    .not_valid(),
            )),
        ),
        (
            "alter-domain-drop-constraint",
            ddl(Domain::alter(quantity())
                .drop_constraint(ConstraintDrop::new(n("small")).if_exists().cascade())),
        ),
        (
            "alter-domain-rename-constraint",
            ddl(Domain::alter(quantity()).rename_constraint(n("small"), n("tiny"))),
        ),
        (
            "alter-domain-validate-constraint",
            ddl(Domain::alter(quantity()).validate_constraint(n("small"))),
        ),
        (
            "alter-domain-rename",
            ddl(Domain::alter(quantity()).rename_to(n("units"))),
        ),
        ("drop-domain", ddl(Domain::drop(n("quantity")))),
        (
            "drop-domains",
            ddl(Domain::drop(n("quantity"))
                .name(quantity())
                .if_exists()
                .restrict()
                .to_owned()),
        ),
        (
            "comment-on-table",
            ddl(pgorm::pgorm_query::Comment::on_table(
                booking!(),
                "Rooms booked, and when",
            )),
        ),
        (
            "comment-on-column",
            ddl(pgorm::pgorm_query::Comment::on_column(
                booking!(),
                n("note"),
                "it's a \\ note\nover two lines",
            )),
        ),
    ]
}
