//! Schema discovery -> `Entity` model: `EntityTransformer::transform`, the
//! inverse relations it synthesises, and the many-to-many conjunct relations it
//! derives from junction tables.

mod common;

use common::*;
use pgorm_codegen::Column;
use pgorm_query::{
    ColumnDef, ColumnType, ForeignKey, ForeignKeyAction, Name, Primary, Table,
    TableCreateStatement, TableKey, TableName,
};

fn fk(from_table: &str, from_col: &str, to_table: &str, to_col: &str) -> TableCreateStatement {
    Table::create(Name::runtime(from_table))
        .col(serial("id"))
        .primary_key(Name::runtime("id"))
        .col(ColumnDef::new(Name::runtime(from_col)).integer().to_owned())
        .foreign_key(ForeignKey::create(
            Name::runtime(from_table),
            Name::runtime(from_col),
            Name::runtime(to_table),
            Name::runtime(to_col),
        ))
        .to_owned()
}

fn bare(table: &str) -> TableCreateStatement {
    keyed_with(table, &["id"], vec![serial("id")])
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    one Entity per input
// statement, held in a BTreeMap so every output is ordered by table name
#[test]
fn transform_builds_entity_per_statement_ordered_by_name() {
    let generated = generate(
        vec![bare("zebra"), bare("apple"), bare("mango")],
        Opts::default(),
    );

    assert_eq!(
        generated.names(),
        ["apple.rs", "mango.rs", "zebra.rs", "mod.rs", "prelude.rs"]
    );
    let index: Vec<&str> = generated
        .file("mod.rs")
        .lines()
        .filter(|l| l.starts_with("pub mod") && !l.contains("prelude"))
        .collect();
    assert_eq!(
        index,
        ["pub mod apple ;", "pub mod mango ;", "pub mod zebra ;"]
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    the table name is unpacked
// from every `TableName` form, and the qualified form keeps its schema
#[test]
fn transform_unpacks_the_table_name_from_every_form() {
    let cake = || Name::runtime("cake");
    let schema = || Name::runtime("public");

    let names = [
        (TableName::Table(cake()), None),
        (TableName::SchemaTable(schema(), cake()), Some("public")),
    ];

    for (table_name, expected_schema) in names {
        let stmt = Table::create(table_name.clone())
            .col(serial("id"))
            .primary_key(Name::runtime("id"))
            .to_owned();
        let generated = generate(vec![stmt], Opts::default());
        assert!(
            generated.has("cake.rs"),
            "{table_name:?} should resolve to the `cake` table"
        );
        assert_contains(generated.file("cake.rs"), r#"table_name = "cake""#);
        match expected_schema {
            Some(schema) => assert_contains(
                generated.file("cake.rs"),
                &format!(r#"schema_name = "{schema}""#),
            ),
            None => assert_not_contains(generated.file("cake.rs"), "schema_name"),
        }
    }
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a column with no
// `ColumnType` is a `TransformError` naming the table and the column
#[test]
fn transform_rejects_a_column_without_column_type() {
    let untyped = Table::create(Name::runtime("cake"))
        .col(ColumnDef::new(Name::runtime("id")))
        .to_owned();

    assert_transform_error(
        vec![untyped],
        "table `cake` column `id`: column type should not be empty",
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a primary-key index naming
// a column the table does not have is a `TransformError`
#[test]
fn transform_rejects_primary_key_over_unknown_column() {
    let mismatched = Table::create(Name::runtime("cake"))
        .col(
            ColumnDef::new(Name::runtime("id"))
                .integer()
                .not_null()
                .to_owned(),
        )
        .primary_key(TableKey::new(Name::runtime("missing")))
        .to_owned();

    assert_transform_error(
        vec![mismatched],
        "table `cake`: primary key column `missing` is not a column of the table",
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a statement's temporal primary key
// becomes the entity's, its period last, and a PERIOD foreign key a relation carrying its
// period; a temporal unique key is refused, never read as its equality columns
#[test]
fn transform_reads_temporal_keys_and_refuses_temporal_unique() {
    let n = Name::runtime;
    let booking = || {
        Table::create(n("booking"))
            .col(ColumnDef::new(n("room_id")).integer().not_null())
            .col(ColumnDef::new_with_type(
                n("during"),
                ColumnType::Range(pgorm_query::RangeType::TimestampTz),
            ))
            .to_owned()
    };
    assert_transform_error(
        vec![
            booking()
                .primary_key(n("room_id"))
                .unique(TableKey::new(n("room_id")).without_overlaps(n("during")))
                .to_owned(),
        ],
        "table `booking`: an entity cannot hold a WITHOUT OVERLAPS unique key",
    );
    let room = Table::create(n("room"))
        .col(ColumnDef::new(n("id")).integer().not_null())
        .col(ColumnDef::new_with_type(
            n("valid_at"),
            ColumnType::Range(pgorm_query::RangeType::TimestampTz),
        ))
        .primary_key(TableKey::new(n("id")).without_overlaps(n("valid_at")))
        .to_owned();
    let entities = pgorm_codegen::EntityTransformer::transform(vec![
        room,
        booking()
            .primary_key(n("room_id"))
            .foreign_key(
                ForeignKey::create(n("booking"), n("room_id"), n("room"), n("id"))
                    .period(n("during"), n("valid_at"))
                    .to_owned(),
            )
            .to_owned(),
    ])
    .expect("temporal keys transform");
    let written = format!("{entities:?}");
    assert!(
        written.contains(
            r#"primary_keys: [PrimaryKey { name: "id" }, PrimaryKey { name: "valid_at" }], without_overlaps: true"#
        ),
        "{written}"
    );
    assert!(
        written.contains(r#"period: Some(("during", "valid_at"))"#),
        "{written}"
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a statement's key naming
// one column twice is refused, never read with or without its repeat; columns
// compare as the text the server reads, whatever names them
#[test]
fn a_key_naming_a_column_twice_is_refused() {
    let n = Name::runtime;
    let twice = || {
        Table::create(n("twice"))
            .col(ColumnDef::new(n("a")).integer().not_null())
            .col(ColumnDef::new(n("b")).integer().not_null())
            .to_owned()
    };
    assert_transform_error(
        vec![twice().primary_key((n("a"), n("b"), n("a"))).to_owned()],
        "table `twice`: a key names column `a` twice",
    );
    assert_transform_error(
        vec![
            twice()
                .primary_key(n("a"))
                .unique(TableKey::new(n("b")).col(pgorm_query::alias("b")))
                .to_owned(),
        ],
        "table `twice`: a key names column `b` twice",
    );
    let keyed = pgorm_codegen::EntityTransformer::transform(vec![
        twice()
            .primary_key((n("a"), n("b")))
            .unique(TableKey::new(n("b")).include([n("b")]))
            .to_owned(),
    ]);
    assert!(keyed.is_ok(), "{keyed:?}");
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a DB name with no Rust
// identifier form is a `TransformError` naming what it came from
#[test]
fn transform_rejects_names_without_identifier_form() {
    assert_transform_error(
        vec![keyed_with(
            "cake",
            &["id"],
            vec![serial("id"), typed("1", ColumnType::Integer)],
        )],
        "table `cake` column `1`: `1` is not a valid Rust identifier",
    );
    assert_transform_error(
        vec![keyed_with("-", &["id"], vec![serial("id")])],
        "table `-`: `` is not a valid Rust identifier",
    );
    assert_transform_error(
        vec![keyed_with(
            "cake",
            &["id"],
            vec![serial("id"), enum_col("tea", "tea", &["€"])],
        )],
        "enum `tea` value `€`: `€` is not a valid Rust identifier",
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a relation onto a table the
// schema does not define, or onto a column either end does not have, is a
// `TransformError` naming the table, the relation and the column
#[test]
fn transform_rejects_relations_it_cannot_resolve() {
    // the partial dump: `orders` references a `customers` nobody passed
    assert_transform_error(
        vec![fk("orders", "customer_id", "customers", "id")],
        "table `orders`: relation to `customers` names a table the schema does not define",
    );

    assert_transform_error(
        vec![
            bare("customers"),
            fk("orders", "customer_id", "customers", "code"),
        ],
        "table `orders`: relation to `customers` references column `code`, which `customers` does \
         not have",
    );

    assert_transform_error(
        vec![
            bare("customers"),
            Table::create(Name::runtime("orders"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                .foreign_key(ForeignKey::create(
                    Name::runtime("orders"),
                    Name::runtime("customer_id"),
                    Name::runtime("customers"),
                    Name::runtime("id"),
                ))
                .to_owned(),
        ],
        "table `orders`: relation to `customers` constrains column `customer_id`, which the table \
         does not have",
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    `auto_increment`,
// `not_null` and `unique` come from the matching `ColumnSpec`
#[test]
fn transform_reads_column_specs_off_the_column_definition() {
    let generated = generate(
        vec![keyed_with(
            "cake",
            &["id"],
            vec![
                // auto_increment + not_null + primary key
                serial("id"),
                // a plain nullable column carries neither
                ColumnDef::new(Name::runtime("baked_at"))
                    .timestamp()
                    .to_owned(),
                // not_null without auto_increment
                ColumnDef::new(Name::runtime("name"))
                    .text()
                    .not_null()
                    .to_owned(),
            ],
        )],
        Opts::default(),
    );
    let cake = generated.file("cake.rs");

    // auto_increment leaves the key unmarked; not_null decides the
    // `Option<..>` wrapping
    assert_contains(cake, "#[pgorm(primary_key)] pub id: i32,");
    assert_contains(cake, "pub baked_at: Option<DateTime>,");
    assert_contains(
        cake,
        "#[pgorm(column_type = ColumnType::Text)] pub name: String,",
    );

    // A column converted on its own is not unique: uniqueness is a key of the
    // table's, which the transform reads off the table's unique keys.
    let alone = Column::try_from(
        &ColumnDef::new(Name::runtime("email"))
            .string()
            .not_null()
            .to_owned(),
    )
    .expect("a typed column def should convert");
    assert_eq!(alone.get_info(), "Column `email`: String, not_null");
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a single-column unique index
// over exactly that column also marks it unique
#[test]
fn transform_marks_columns_from_single_column_unique_key() {
    let generated = generate(
        vec![
            Table::create(Name::runtime("vendor"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                .col(
                    ColumnDef::new(Name::runtime("name"))
                        .string()
                        .not_null()
                        .to_owned(),
                )
                .col(
                    ColumnDef::new(Name::runtime("region"))
                        .string()
                        .not_null()
                        .to_owned(),
                )
                .col(
                    ColumnDef::new(Name::runtime("tier"))
                        .string()
                        .not_null()
                        .to_owned(),
                )
                .unique(unique_key("vendor", "name"))
                // a multi-column unique index marks nothing
                .unique(
                    TableKey::new(Name::runtime("region"))
                        .name(Name::runtime("idx_vendor_region_tier"))
                        .col(Name::runtime("tier")),
                )
                .to_owned(),
        ],
        Opts::default(),
    );
    let vendor = generated.file("vendor.rs");

    assert_contains(vendor, "#[pgorm(unique)] pub name: String,");
    assert_contains(vendor, "pub region: String,");
    assert_not_contains(vendor, "#[pgorm(unique)] pub region: String,");
    assert_not_contains(vendor, "#[pgorm(unique)] pub tier: String,");
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    the key columns are the
// table's primary key's, one column or a tuple, in key order
#[test]
fn transform_reads_the_tables_primary_key() {
    let one = generate(
        vec![keyed_with("cake", &["id"], vec![serial("id")])],
        Opts::default(),
    );
    assert_contains(one.file("cake.rs"), "#[pgorm(primary_key)] pub id: i32,");

    let by_index = generate(
        vec![
            Table::create(Name::runtime("cake_filling"))
                .col(
                    ColumnDef::new(Name::runtime("cake_id"))
                        .integer()
                        .not_null()
                        .to_owned(),
                )
                .col(
                    ColumnDef::new(Name::runtime("filling_id"))
                        .integer()
                        .not_null()
                        .to_owned(),
                )
                .primary_key(
                    TableKey::new(Name::runtime("cake_id")).col(Name::runtime("filling_id")),
                )
                .to_owned(),
        ],
        Opts::default(),
    );
    assert_contains(
        by_index.file("cake_filling.rs"),
        "#[pgorm(primary_key, auto_increment = false)] pub cake_id: i32, \
         #[pgorm(primary_key, auto_increment = false)] pub filling_id: i32,",
    );
}

/// `wide`, keyed by a table-level primary key over all `columns` of its
/// integer columns `k1`, `k2`, ...
fn keyed_by(columns: usize) -> TableCreateStatement {
    let mut table = Table::create(Name::runtime("wide"));
    let mut key = TableKey::<Primary>::new(Name::runtime("k1"));
    for at in 1..=columns {
        let name = format!("k{at}");
        table.col(
            ColumnDef::new(Name::runtime(name.as_str()))
                .integer()
                .not_null(),
        );
        if at > 1 {
            key = key.col(Name::runtime(name.as_str()));
        }
    }
    table.primary_key(key);
    table
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a key of twelve columns
// generates its twelve key fields, and a thirteenth column is refused by name
// rather than generated as a key type the entity traits have no impl for
#[test]
fn a_key_wider_than_twelve_columns_is_refused() {
    let twelve = generate(vec![keyed_by(12)], Opts::default());
    assert_eq!(twelve.file("wide.rs").matches("primary_key").count(), 12);
    assert_transform_error(
        vec![keyed_by(13)],
        "table `wide`: a primary key of 13 columns; an entity's key has at most 12, the widest \
         tuple its value can be",
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    every enum column registers
// an `ActiveEnum` keyed by enum name, deduplicated across tables and looked
// through `Array`
#[test]
fn transform_registers_enums_once_per_name_across_tables() {
    let tea = || ColumnType::Enum {
        schema: None,
        name: Name::runtime("tea"),
        variants: vec![Name::runtime("EverydayTea"), Name::runtime("BreakfastTea")],
    };

    let generated = generate(
        vec![
            keyed_with(
                "cake",
                &["id"],
                vec![
                    serial("id"),
                    enum_col("tea", "tea", &["EverydayTea", "BreakfastTea"]),
                    enum_col("mood", "mood", &["Happy", "Sad"]),
                ],
            ),
            // the same `tea` enum again, this time as an array element type
            keyed_with(
                "biscuit",
                &["id"],
                vec![
                    serial("id"),
                    ColumnDef::new(Name::runtime("teas"))
                        .array(tea())
                        .not_null()
                        .to_owned(),
                ],
            ),
        ],
        Opts::default(),
    );

    let enums = generated.file("pgorm_active_enums.rs");
    assert_eq!(
        enums.matches("pub enum Tea").count(),
        1,
        "the `tea` enum should be registered once: {enums}"
    );
    // BTreeMap ordering: `mood` before `tea`
    assert!(position_of(enums, "pub enum Mood") < position_of(enums, "pub enum Tea"));
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    foreign keys become
// `BelongsTo` relations that keep their columns, referenced columns and
// on_update / on_delete actions
#[test]
fn transform_turns_foreign_keys_into_belongs_to_relations() {
    let generated = generate(
        vec![
            keyed_with(
                "cake",
                &["id", "kind"],
                vec![
                    ColumnDef::new(Name::runtime("id"))
                        .integer()
                        .not_null()
                        .to_owned(),
                    ColumnDef::new(Name::runtime("kind"))
                        .integer()
                        .not_null()
                        .to_owned(),
                ],
            ),
            Table::create(Name::runtime("fruit"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                .col(
                    ColumnDef::new(Name::runtime("cake_id"))
                        .integer()
                        .to_owned(),
                )
                .col(
                    ColumnDef::new(Name::runtime("cake_kind"))
                        .integer()
                        .to_owned(),
                )
                .foreign_key(
                    ForeignKey::create(
                        Name::runtime("fruit"),
                        Name::runtime("cake_id"),
                        Name::runtime("cake"),
                        Name::runtime("id"),
                    )
                    .col(Name::runtime("cake_kind"), Name::runtime("kind"))
                    .on_update(ForeignKeyAction::Cascade)
                    .on_delete(ForeignKeyAction::SetNull)
                    .to_owned(),
                )
                .to_owned(),
        ],
        Opts::default(),
    );

    assert_contains(
        generated.file("fruit.rs"),
        r#"#[pgorm(
            belongs_to = super::cake::Entity,
            from = (Column::CakeId, Column::CakeKind),
            to = (super::cake::Column::Id, super::cake::Column::Kind),
            on_update = ForeignKeyAction::Cascade,
            on_delete = ForeignKeyAction::SetNull,
        )]"#,
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a relation onto its own
// table is flagged self-referencing
#[test]
fn transform_flags_self_referencing_relations() {
    let generated = generate(
        vec![fk("users", "manager_id", "users", "id")],
        Opts::default(),
    );

    assert_contains(
        generated.file("users.rs"),
        r#"#[pgorm(belongs_to = Entity, from = Column::ManagerId, to = Column::Id,)] SelfRef,"#,
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    several FKs onto the same
// target take 1-based `num_suffix`es in declaration order; a lone FK keeps 0
#[test]
fn transform_numbers_repeated_fks_to_same_table() {
    let generated = generate(
        vec![
            bare("fruit"),
            bare("cake"),
            Table::create(Name::runtime("basket"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                .col(
                    ColumnDef::new(Name::runtime("fruit_id1"))
                        .integer()
                        .to_owned(),
                )
                .col(
                    ColumnDef::new(Name::runtime("fruit_id2"))
                        .integer()
                        .to_owned(),
                )
                .col(
                    ColumnDef::new(Name::runtime("cake_id"))
                        .integer()
                        .to_owned(),
                )
                .foreign_key(ForeignKey::create(
                    Name::runtime("basket"),
                    Name::runtime("fruit_id1"),
                    Name::runtime("fruit"),
                    Name::runtime("id"),
                ))
                .foreign_key(ForeignKey::create(
                    Name::runtime("basket"),
                    Name::runtime("fruit_id2"),
                    Name::runtime("fruit"),
                    Name::runtime("id"),
                ))
                .foreign_key(ForeignKey::create(
                    Name::runtime("basket"),
                    Name::runtime("cake_id"),
                    Name::runtime("cake"),
                    Name::runtime("id"),
                ))
                .to_owned(),
        ],
        Opts::default(),
    );
    let basket = generated.file("basket.rs");

    // both FKs to `fruit` take a distinct 1-based suffix. NB the numbering runs
    // in reverse declaration order, so `fruit_id1` lands on `Fruit2`.
    assert_contains(
        basket,
        r#"#[pgorm(
            belongs_to = super::fruit::Entity,
            from = Column::FruitId1,
            to = super::fruit::Column::Id,
        )]
        Fruit2,"#,
    );
    assert_contains(
        basket,
        r#"#[pgorm(
            belongs_to = super::fruit::Entity,
            from = Column::FruitId2,
            to = super::fruit::Column::Id,
        )]
        Fruit1,"#,
    );
    // the single FK to `cake` keeps suffix 0
    assert_contains(
        basket,
        r#"#[pgorm(
            belongs_to = super::cake::Entity,
            from = Column::CakeId,
            to = super::cake::Column::Id,
        )]
        Cake,"#,
    );
    assert_not_contains(basket, "Cake1,");
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    relations are sorted by
// referenced table name and conjunct relations by target name
#[test]
fn transform_sorts_relations_and_conjunct_relations() {
    let generated = generate(cake_schema(), Opts::default());
    let cake = generated.file("cake.rs");

    // `cake_filling` sorts before `fruit`
    assert!(position_of(cake, "CakeFilling,") < position_of(cake, "Fruit,"));

    // `users` sits between two junction targets, so its conjunct relations sort
    let generated = generate(
        vec![
            bare("users"),
            bare("apple"),
            bare("zebra"),
            junction("users_apples", ("users", "user_id"), ("apple", "apple_id")),
            junction("users_zebras", ("users", "user_id"), ("zebra", "zebra_id")),
        ],
        Opts::default(),
    );
    let users = generated.file("users.rs");
    assert!(
        position_of(users, "impl Related<super::apple::Entity> for Entity")
            < position_of(users, "impl Related<super::zebra::Entity> for Entity")
    );
}

fn junction(name: &str, left: (&str, &str), right: (&str, &str)) -> TableCreateStatement {
    Table::create(Name::runtime(name))
        .col(
            ColumnDef::new(Name::runtime(left.1))
                .integer()
                .not_null()
                .to_owned(),
        )
        .col(
            ColumnDef::new(Name::runtime(right.1))
                .integer()
                .not_null()
                .to_owned(),
        )
        .primary_key((Name::runtime(left.1), Name::runtime(right.1)))
        .foreign_key(ForeignKey::create(
            Name::runtime(name),
            Name::runtime(left.1),
            Name::runtime(left.0),
            Name::runtime("id"),
        ))
        .foreign_key(ForeignKey::create(
            Name::runtime(name),
            Name::runtime(right.1),
            Name::runtime(right.0),
            Name::runtime("id"),
        ))
        .to_owned()
}

// [spec:pgorm:sem:codegen.entity.transform.inverse+1/test]    a non-unique FK
// gives the referenced entity a `HasMany` back-reference
#[test]
fn inverse_has_many_for_non_unique_foreign_key() {
    let generated = generate(vec![cake(), fruit()], Opts::default());

    assert_contains(
        generated.file("cake.rs"),
        r#"#[pgorm(has_many = super::fruit::Entity)] Fruit,"#,
    );
    // the inverse carries no from/to
    assert_not_contains(generated.file("cake.rs"), "from = Column::CakeId");
}

// [spec:pgorm:sem:codegen.entity.transform.inverse+1/test]    a FK whose every
// column is unique in the owning table inverts to `HasOne`
#[test]
fn inverse_has_one_for_unique_foreign_key() {
    let generated = generate(
        vec![
            cake(),
            Table::create(Name::runtime("fruit"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                .col(
                    ColumnDef::new(Name::runtime("cake_id"))
                        .integer()
                        .to_owned(),
                )
                .unique(unique_key("fruit", "cake_id"))
                .foreign_key(ForeignKey::create(
                    Name::runtime("fruit"),
                    Name::runtime("cake_id"),
                    Name::runtime("cake"),
                    Name::runtime("id"),
                ))
                .to_owned(),
        ],
        Opts::default(),
    );

    assert_contains(
        generated.file("cake.rs"),
        r#"#[pgorm(has_one = super::fruit::Entity)] Fruit,"#,
    );
}

// [spec:pgorm:sem:codegen.entity.transform.inverse+1/test]    a FK whose column
// set is exactly the owning table's primary key also inverts to `HasOne`
#[test]
fn inverse_has_one_for_whole_primary_key_fk() {
    let generated = generate(
        vec![
            bare("users"),
            Table::create(Name::runtime("profile"))
                .col(
                    ColumnDef::new(Name::runtime("user_id"))
                        .integer()
                        .not_null()
                        .to_owned(),
                )
                .primary_key(Name::runtime("user_id"))
                .foreign_key(ForeignKey::create(
                    Name::runtime("profile"),
                    Name::runtime("user_id"),
                    Name::runtime("users"),
                    Name::runtime("id"),
                ))
                .to_owned(),
        ],
        Opts::default(),
    );

    assert_contains(
        generated.file("users.rs"),
        r#"#[pgorm(has_one = super::profile::Entity)] Profile,"#,
    );
}

// [spec:pgorm:sem:codegen.entity.transform.inverse+1/test]    a unique constraint
// over exactly the FK's columns constrains the key as a whole, which reading one
// column at a time cannot see: the inverse is `HasOne`
#[test]
fn inverse_has_one_for_composite_unique_foreign_key() {
    let cake = || {
        keyed_with(
            "cake",
            &["id", "kind"],
            vec![
                ColumnDef::new(Name::runtime("id"))
                    .integer()
                    .not_null()
                    .to_owned(),
                ColumnDef::new(Name::runtime("kind"))
                    .integer()
                    .not_null()
                    .to_owned(),
            ],
        )
    };
    let cake_key = |table: &'static str| {
        ForeignKey::create(
            Name::runtime(table),
            Name::runtime("cake_id"),
            Name::runtime("cake"),
            Name::runtime("id"),
        )
        .col(Name::runtime("cake_kind"), Name::runtime("kind"))
        .to_owned()
    };

    let generated = generate(
        vec![
            cake(),
            Table::create(Name::runtime("fruit"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                .col(
                    ColumnDef::new(Name::runtime("cake_id"))
                        .integer()
                        .to_owned(),
                )
                .col(
                    ColumnDef::new(Name::runtime("cake_kind"))
                        .integer()
                        .to_owned(),
                )
                .unique(
                    TableKey::new(Name::runtime("cake_id"))
                        .name(Name::runtime("idx_fruit_cake"))
                        .col(Name::runtime("cake_kind")),
                )
                .foreign_key(cake_key("fruit").to_owned())
                .to_owned(),
            // the same key under a unique index covering more than the key
            // constrains nothing about the key
            Table::create(Name::runtime("crumb"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                .col(
                    ColumnDef::new(Name::runtime("cake_id"))
                        .integer()
                        .to_owned(),
                )
                .col(
                    ColumnDef::new(Name::runtime("cake_kind"))
                        .integer()
                        .to_owned(),
                )
                .col(ColumnDef::new(Name::runtime("batch")).integer().to_owned())
                .unique(
                    TableKey::new(Name::runtime("cake_id"))
                        .name(Name::runtime("idx_crumb_cake_batch"))
                        .col(Name::runtime("cake_kind"))
                        .col(Name::runtime("batch")),
                )
                .foreign_key(cake_key("crumb").to_owned())
                .to_owned(),
        ],
        Opts::default(),
    );
    let cake_file = generated.file("cake.rs");

    assert_contains(
        cake_file,
        r#"#[pgorm(has_one = super::fruit::Entity)] Fruit,"#,
    );
    assert_contains(
        cake_file,
        r#"#[pgorm(has_many = super::crumb::Entity)] Crumb,"#,
    );
    // neither column is unique on its own
    assert_not_contains(generated.file("fruit.rs"), "#[pgorm(unique)] pub cake_id");
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]
// [spec:pgorm:sem:codegen.entity.transform.inverse+1/test]    a `UniqueKey` spec
// on the column definition marks the column unique on this path too, so its FK
// inverts to `HasOne`
#[test]
fn inverse_has_one_for_inline_unique_key_column() {
    let generated = generate(
        vec![
            cake(),
            Table::create(Name::runtime("fruit"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                .col(
                    ColumnDef::new(Name::runtime("cake_id"))
                        .integer()
                        .to_owned(),
                )
                .unique(Name::runtime("cake_id"))
                .foreign_key(ForeignKey::create(
                    Name::runtime("fruit"),
                    Name::runtime("cake_id"),
                    Name::runtime("cake"),
                    Name::runtime("id"),
                ))
                .to_owned(),
        ],
        Opts::default(),
    );

    assert_contains(
        generated.file("fruit.rs"),
        "#[pgorm(unique)] pub cake_id: Option<i32>,",
    );
    assert_contains(
        generated.file("cake.rs"),
        r#"#[pgorm(has_one = super::fruit::Entity)] Fruit,"#,
    );
}

// [spec:pgorm:sem:codegen.entity.transform.inverse+1/test]    self-referencing and
// suffixed relations produce no inverse
#[test]
fn no_inverse_for_self_referencing_or_suffixed_relations() {
    let self_ref = generate(
        vec![fk("users", "manager_id", "users", "id")],
        Opts::default(),
    );
    let users = self_ref.file("users.rs");
    // exactly the one `SelfRef` variant, no synthesised back-reference
    assert_contains(users, "SelfRef,");
    assert_not_contains(users, "has_many = Entity");
    assert_not_contains(users, "has_one = Entity");

    let suffixed = generate(
        vec![
            bare("fruit"),
            Table::create(Name::runtime("basket"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                .col(
                    ColumnDef::new(Name::runtime("fruit_id1"))
                        .integer()
                        .to_owned(),
                )
                .col(
                    ColumnDef::new(Name::runtime("fruit_id2"))
                        .integer()
                        .to_owned(),
                )
                .foreign_key(ForeignKey::create(
                    Name::runtime("basket"),
                    Name::runtime("fruit_id1"),
                    Name::runtime("fruit"),
                    Name::runtime("id"),
                ))
                .foreign_key(ForeignKey::create(
                    Name::runtime("basket"),
                    Name::runtime("fruit_id2"),
                    Name::runtime("fruit"),
                    Name::runtime("id"),
                ))
                .to_owned(),
        ],
        Opts::default(),
    );
    assert_contains(suffixed.file("fruit.rs"), "pub enum Relation { }");
}

// [spec:pgorm:sem:codegen.entity.transform.inverse+1/test]    an inverse relation
// is dropped when the target entity already relates to that table
#[test]
fn inverse_dropped_when_target_already_relates_back() {
    let generated = generate(
        vec![
            Table::create(Name::runtime("users"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                .col(
                    ColumnDef::new(Name::runtime("bill_id"))
                        .integer()
                        .to_owned(),
                )
                .foreign_key(ForeignKey::create(
                    Name::runtime("users"),
                    Name::runtime("bill_id"),
                    Name::runtime("bills"),
                    Name::runtime("id"),
                ))
                .to_owned(),
            Table::create(Name::runtime("bills"))
                .col(serial("id"))
                .primary_key(Name::runtime("id"))
                .col(
                    ColumnDef::new(Name::runtime("user_id"))
                        .integer()
                        .to_owned(),
                )
                .foreign_key(ForeignKey::create(
                    Name::runtime("bills"),
                    Name::runtime("user_id"),
                    Name::runtime("users"),
                    Name::runtime("id"),
                ))
                .to_owned(),
        ],
        Opts::default(),
    );

    // each side keeps only its own belongs_to; neither gains a has_many
    for (file, target) in [
        ("users.rs", "super::bills::Entity"),
        ("bills.rs", "super::users::Entity"),
    ] {
        let content = generated.file(file);
        assert_contains(content, &format!("belongs_to = {target}"));
        assert_not_contains(content, &format!("has_many = {target}"));
    }
}

// [spec:pgorm:sem:codegen.entity.transform.conjunct+2/test]    a table with two
// relations and two primary-key columns is a junction: both referenced entities
// gain a `ConjunctRelation`
#[test]
fn junction_table_gives_both_sides_a_conjunct_relation() {
    let generated = generate(cake_schema(), Opts::default());

    assert_contains(
        generated.file("cake.rs"),
        "impl Related<super::filling::Entity> for Entity {
            fn to() -> RelationDef { super::cake_filling::Relation::Filling.def() }
            fn via() -> Option<RelationDef> {
                Some(super::cake_filling::Relation::Cake.def().rev())
            }
        }",
    );
    assert_contains(
        generated.file("filling.rs"),
        "impl Related<super::cake::Entity> for Entity {
            fn to() -> RelationDef { super::cake_filling::Relation::Cake.def() }
            fn via() -> Option<RelationDef> {
                Some(super::cake_filling::Relation::Filling.def().rev())
            }
        }",
    );
}

// [spec:pgorm:sem:codegen.entity.transform.conjunct+2/test]    two foreign keys
// that are not the table's primary key join nothing: a table keyed by something
// of its own is a table with two references, not a junction
#[test]
fn fks_outside_the_primary_key_are_not_junctions() {
    let generated = generate(
        vec![
            bare("orgs"),
            bare("users"),
            Table::create(Name::runtime("audit_events"))
                .col(
                    ColumnDef::new(Name::runtime("id"))
                        .integer()
                        .not_null()
                        .to_owned(),
                )
                .col(
                    ColumnDef::new(Name::runtime("version"))
                        .integer()
                        .not_null()
                        .to_owned(),
                )
                .primary_key((Name::runtime("id"), Name::runtime("version")))
                .col(
                    ColumnDef::new(Name::runtime("user_id"))
                        .integer()
                        .to_owned(),
                )
                .col(ColumnDef::new(Name::runtime("org_id")).integer().to_owned())
                .foreign_key(ForeignKey::create(
                    Name::runtime("audit_events"),
                    Name::runtime("user_id"),
                    Name::runtime("users"),
                    Name::runtime("id"),
                ))
                .foreign_key(ForeignKey::create(
                    Name::runtime("audit_events"),
                    Name::runtime("org_id"),
                    Name::runtime("orgs"),
                    Name::runtime("id"),
                ))
                .to_owned(),
        ],
        Opts::default(),
    );

    for (file, other) in [("users.rs", "orgs"), ("orgs.rs", "users")] {
        let content = generated.file(file);
        assert_contains(
            content,
            r#"#[pgorm(has_many = super::audit_events::Entity)] AuditEvents,"#,
        );
        assert_not_contains(content, "fn via() -> Option<RelationDef>");
        assert_not_contains(
            content,
            &format!("impl Related<super::{other}::Entity> for Entity"),
        );
    }
}

// [spec:pgorm:sem:codegen.entity.transform.conjunct+2/test]    only the table's
// own foreign keys are junction legs: a synthesised inverse neither makes a
// junction nor stops one
#[test]
fn inbound_relations_are_not_junction_legs() {
    // `posts` has one foreign key and a two-column primary key; the `comments`
    // back-reference is `comments`' key, not a second leg of its own
    let generated = generate(
        vec![
            bare("tenants"),
            Table::create(Name::runtime("posts"))
                .col(
                    ColumnDef::new(Name::runtime("tenant_id"))
                        .integer()
                        .not_null()
                        .to_owned(),
                )
                .col(
                    ColumnDef::new(Name::runtime("id"))
                        .integer()
                        .not_null()
                        .to_owned(),
                )
                .primary_key((Name::runtime("tenant_id"), Name::runtime("id")))
                .foreign_key(ForeignKey::create(
                    Name::runtime("posts"),
                    Name::runtime("tenant_id"),
                    Name::runtime("tenants"),
                    Name::runtime("id"),
                ))
                .to_owned(),
            fk("comments", "post_id", "posts", "id"),
        ],
        Opts::default(),
    );

    assert_not_contains(
        generated.file("tenants.rs"),
        "fn via() -> Option<RelationDef>",
    );
    assert_not_contains(
        generated.file("tenants.rs"),
        "impl Related<super::comments::Entity> for Entity",
    );
    assert_not_contains(
        generated.file("comments.rs"),
        "fn via() -> Option<RelationDef>",
    );

    // and the junction below keeps its many-to-many despite the inbound
    // relation `vote_flags` gives it
    let generated = generate(
        vec![
            bare("users"),
            bare("bills"),
            junction("users_votes", ("users", "user_id"), ("bills", "bill_id")),
            fk("vote_flags", "user_id", "users_votes", "user_id"),
        ],
        Opts::default(),
    );

    assert_contains(
        generated.file("users.rs"),
        "impl Related<super::bills::Entity> for Entity {
            fn to() -> RelationDef { super::users_votes::Relation::Bills.def() }
            fn via() -> Option<RelationDef> {
                Some(super::users_votes::Relation::Users.def().rev())
            }
        }",
    );
}

// [spec:pgorm:sem:codegen.entity.transform.conjunct+2/test]    duplicated
// many-to-many paths to the same target are all removed
#[test]
fn duplicated_many_to_many_paths_generate_no_conjunct() {
    let generated = generate(
        vec![
            bare("users"),
            bare("bills"),
            junction("users_votes", ("users", "user_id"), ("bills", "bill_id")),
            junction(
                "users_saved_bills",
                ("users", "user_id"),
                ("bills", "bill_id"),
            ),
        ],
        Opts::default(),
    );

    // both sides keep the junction relations but neither gets a `via` impl
    assert_not_contains(
        generated.file("users.rs"),
        "fn via() -> Option<RelationDef>",
    );
    assert_not_contains(
        generated.file("bills.rs"),
        "fn via() -> Option<RelationDef>",
    );
    assert_contains(
        generated.file("users.rs"),
        r#"#[pgorm(has_many = super::users_votes::Entity)] UsersVotes,"#,
    );
}

// [spec:pgorm:sem:codegen.entity.transform.conjunct+2/test]    when a conjunct
// relation targets the same table as an ordinary relation, the ordinary
// relation says `related = false` in favour of the `via` impl
#[test]
fn conjunct_relation_suppresses_the_plain_related_impl() {
    let generated = generate(
        vec![
            bare("users"),
            // `bills` also has a direct FK to `users`, so `users` ends up with
            // both an ordinary relation and a conjunct relation to `bills`
            fk("bills", "user_id", "users", "id"),
            junction("users_votes", ("users", "user_id"), ("bills", "bill_id")),
        ],
        Opts::default(),
    );
    let users = generated.file("users.rs");

    // the Relation variant survives, saying the derive writes no `Related`
    // for it, which the conjunct one is instead
    assert_contains(
        users,
        r#"#[pgorm(has_many = super::bills::Entity, related = false)] Bills,"#,
    );
    assert_contains(
        users,
        "impl Related<super::bills::Entity> for Entity {
            fn to() -> RelationDef { super::users_votes::Relation::Bills.def() }
            fn via() -> Option<RelationDef> {
                Some(super::users_votes::Relation::Users.def().rev())
            }
        }",
    );
}
