//! The DDL bridge: `schema.sql` in, the same entities the statement-building
//! path produces out — and a named error for everything else in the file.

mod common;

use common::*;
use pgorm::{
    ColumnTrait, ColumnTypeTrait, EntityTrait, JoinType, ModelTrait, PrimaryKeyTrait, QuerySelect,
    QueryTrait, RelationTrait,
};
use pgorm_codegen::{Error, WriterOutput, entities_from_sql, parse_schema};
use pgorm_query::extension::Type;
use pgorm_query::{ColumnSpec, ColumnType, Deferrability, Enforcement, TableName};

const SCHEMA: &str = include_str!("sql/schema.sql");

/// The entity the bridge generates for an identity inside a composite key,
/// compiled here as well as compared: the derive accepts what the writer
/// emits.
#[path = "sql/tenant_ticket.rs"]
mod tenant_ticket;

const TENANT_TICKET: &str = "CREATE TABLE tenant_ticket (
    tenant_id int,
    id int GENERATED ALWAYS AS IDENTITY,
    title text NOT NULL,
    PRIMARY KEY (tenant_id, id)
);";

/// The entities the bridge generates for a two-column foreign key onto a
/// two-column key, compiled here as well as compared: the relation the writer
/// emits pairs both columns, and the derive accepts it.
#[path = "sql/tenant_task.rs"]
mod tenant_task;

/// The entities the bridge generates for a foreign key that is deferred and
/// not enforced, compiled here as well as compared: the relation carries both,
/// and the derive accepts them.
#[path = "sql/key_room.rs"]
mod key_room;
#[path = "sql/key_stay.rs"]
mod key_stay;
/// The entities the bridge generates for a temporal key and a foreign key
/// matching on its period, compiled here as well as compared.
#[path = "sql/temporal_room.rs"]
mod temporal_room;
#[path = "sql/temporal_stay.rs"]
mod temporal_stay;

const TEMPORAL: &str = "CREATE TABLE temporal_room (
    id int NOT NULL,
    valid_at int4range NOT NULL,
    rate int NOT NULL,
    PRIMARY KEY (id, valid_at WITHOUT OVERLAPS)
);
CREATE TABLE temporal_stay (
    id int PRIMARY KEY,
    room_id int NOT NULL,
    during int4range NOT NULL,
    FOREIGN KEY (room_id, PERIOD during) REFERENCES temporal_room (id, PERIOD valid_at)
);";

const KEY_STAY: &str = "CREATE TABLE key_room (id int PRIMARY KEY);
CREATE TABLE key_stay (
    id int PRIMARY KEY,
    room_id int NOT NULL REFERENCES key_room (id) DEFERRABLE INITIALLY DEFERRED NOT ENFORCED
);";
#[path = "sql/tenant_task_note.rs"]
mod tenant_task_note;

const TENANT_TASKS: &str = "CREATE TABLE tenant_task (
    tenant_id int NOT NULL,
    id int NOT NULL,
    title text NOT NULL,
    PRIMARY KEY (tenant_id, id)
);
CREATE TABLE tenant_task_note (
    tenant_id int NOT NULL,
    id int NOT NULL,
    task_id int NOT NULL,
    body text NOT NULL,
    PRIMARY KEY (tenant_id, id),
    CONSTRAINT tenant_task_note_task FOREIGN KEY (tenant_id, task_id)
        REFERENCES tenant_task (tenant_id, id) ON DELETE CASCADE
);";

fn from_sql(sql: &str) -> Generated {
    Generated {
        files: files(entities_from_sql(sql, Opts::default()).expect("schema should generate")),
    }
}

fn files(output: WriterOutput) -> Vec<(String, String)> {
    output
        .files
        .into_iter()
        .map(|file| (file.name, file.content))
        .collect()
}

#[track_caller]
fn error(sql: &str) -> String {
    match entities_from_sql(sql, Opts::default()) {
        Err(Error::TransformError(message)) => message,
        Err(other) => panic!("expected a TransformError, got {other:?}"),
        Ok(_) => panic!("expected an error, got generated entities"),
    }
}

#[track_caller]
fn assert_error(sql: &str, expected: &str) {
    assert_eq!(error(sql), expected);
}

// [spec:pgorm:def:codegen.ddl+4/test]    the whole pipeline runs from DDL text:
// one entity file per CREATE TABLE, plus index, prelude and active enums
#[test]
fn schema_sql_generates_one_file_per_table() {
    let generated = from_sql(SCHEMA);

    assert_eq!(
        generated.names(),
        [
            "label.rs",
            "owner.rs",
            "task.rs",
            "task_label.rs",
            "mod.rs",
            "prelude.rs",
            "pgorm_active_enums.rs",
        ]
    );
}

// [spec:pgorm:sem:codegen.ddl.types+9/test]    the type spellings map onto the
// ColumnType vocabulary, serial included
#[test]
fn column_types_map_through_the_vocabulary() {
    let generated = from_sql(SCHEMA);
    let task = generated.file("task.rs");

    assert_contains(task, "#[pgorm(primary_key)] pub id: i64,");
    assert_contains(task, "pub owner_id: i32,");
    assert_contains(
        task,
        r#"#[pgorm(column_type = ColumnType::Text)] pub title: String,"#,
    );
    assert_contains(task, "pub state: TaskState,");
    assert_contains(
        task,
        r#"#[pgorm(column_type = ColumnType::Double, nullable)] pub weight: Option<f64>,"#,
    );
    assert_contains(task, "pub tags: Option<Vec<String>>,");
    assert_contains(task, "pub due: Option<DateTimeWithTimeZone>,");
    assert_contains(
        task,
        r#"#[pgorm(column_type = ColumnType::JsonBinary, nullable)] pub body: Option<Json>,"#,
    );
    assert_contains(task, "pub r#ref: Option<Uuid>,");
    assert_contains(generated.file("owner.rs"), "pub name: String,");
}

// [spec:pgorm:sem:codegen.ddl.objects+10/test]    a CREATE TYPE ... AS ENUM
// reaches the generated active enum through the columns that name it
#[test]
fn enum_type_reaches_the_generated_active_enum() {
    let generated = from_sql(SCHEMA);
    let enums = generated.file("pgorm_active_enums.rs");

    assert_contains(
        enums,
        r#"#[pgorm(rs_type = String, enum_name = "task_state")]"#,
    );
    assert_contains(enums, "pub enum TaskState");
    assert_contains(enums, r#"#[pgorm(string_value = "open")] Open,"#);
    assert_contains(enums, r#"#[pgorm(string_value = "closed")] Closed,"#);
    assert_contains(
        generated.file("task.rs"),
        "use super::pgorm_active_enums::TaskState;",
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    a foreign key keeps its columns
// and its declared actions
#[test]
fn foreign_keys_keep_their_columns_and_actions() {
    let generated = from_sql(SCHEMA);

    assert_contains(
        generated.file("task.rs"),
        r#"#[pgorm(belongs_to = super::owner::Entity, from = Column::OwnerId, to = super::owner::Column::Id, on_update = ForeignKeyAction::Restrict, on_delete = ForeignKeyAction::Cascade,)]"#,
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    a table-level composite primary
// key plus two foreign keys is read as a junction table
#[test]
fn composite_key_junction_becomes_conjunct_relations() {
    let generated = from_sql(SCHEMA);

    assert_contains(
        generated.file("task_label.rs"),
        "#[pgorm(primary_key, auto_increment = false)] pub task_id: i64,",
    );
    assert_contains(
        generated.file("task.rs"),
        "impl Related<super::label::Entity> for Entity",
    );
    assert_contains(
        generated.file("label.rs"),
        "impl Related<super::task::Entity> for Entity",
    );
}

// [spec:pgorm:sem:codegen.ddl.objects+10/test]    a single-column unique index
// marks its column unique; a plain index states no entity fact
#[test]
fn unique_index_marks_its_column_unique() {
    let generated = from_sql(SCHEMA);

    assert_contains(
        generated.file("owner.rs"),
        r#"#[pgorm(column_type = ColumnType::Text, nullable, unique)] pub email: Option<String>,"#,
    );
    assert_contains(
        generated.file("task.rs"),
        "pub due: Option<DateTimeWithTimeZone>,",
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    a column-level UNIQUE becomes the
// table-level unique constraint Postgres creates for it, which is where the entity model
// reads unique
#[test]
fn column_unique_constraint_marks_the_column() {
    let generated = from_sql("CREATE TABLE t (id serial PRIMARY KEY, email text UNIQUE);");

    assert_contains(
        generated.file("t.rs"),
        r#"#[pgorm(column_type = ColumnType::Text, nullable, unique)] pub email: Option<String>,"#,
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    a schema-qualified name is kept
// as the schema-qualified table name the statement targets
#[test]
fn schema_qualified_table_names_are_kept() {
    const SQL: &str = "CREATE TABLE app.task (id serial PRIMARY KEY);";
    let tables = parse_schema(SQL).expect("schema should parse");
    let table = tables.first().expect("the task table");

    let TableName::SchemaTable(schema, name) = table.get_table_name() else {
        panic!("the task table should carry its schema");
    };
    assert_eq!(schema.to_string(), "app");
    assert_eq!(name.to_string(), "task");
    assert!(
        from_sql(SQL).has("task.rs"),
        "the entity is keyed by the table name alone"
    );
}

// [spec:pgorm:sem:codegen.ddl.objects+10/test]    COMMENT ON statements are folded
// into the table and column they describe
#[test]
fn comments_are_folded_into_their_table() {
    let tables = parse_schema(SCHEMA).expect("schema should parse");
    let task = tables.get(1).expect("the task table");

    let TableName::Table(name) = task.get_table_name() else {
        panic!("the task table should be a plain table name");
    };
    assert_eq!(name.to_string(), "task");
    assert_eq!(
        task.get_comment().map(String::as_str),
        Some("work to be done")
    );

    let title = task
        .get_columns()
        .iter()
        .find(|column| column.get_column_name() == "title")
        .expect("the title column");
    assert!(
        title
            .get_column_spec()
            .iter()
            .any(|spec| matches!(spec, ColumnSpec::Comment(text) if text == "short summary")),
        "the column comment should ride on the column definition"
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    a column's COLLATE clause becomes its
// collation, bare or qualified, and the entity generated from it is the one the
// uncollated table generates
#[test]
fn a_column_collation_rides_on_the_statement() {
    let collated = r#"CREATE TABLE note (
        id int PRIMARY KEY,
        title text COLLATE "C" NOT NULL,
        body text COLLATE pg_catalog."default"
    );"#;
    let collations = |sql: &str| {
        let tables = parse_schema(sql).expect("schema should parse");
        tables[0]
            .get_columns()
            .iter()
            .map(|column| {
                column.get_collation().map(|collation| {
                    (
                        collation.schema().map(|schema| schema.to_string()),
                        collation.name().to_string(),
                    )
                })
            })
            .collect::<Vec<_>>()
    };
    let expected = vec![
        None,
        Some((None, "C".to_owned())),
        Some((Some("pg_catalog".to_owned()), "default".to_owned())),
    ];
    assert_eq!(collations(collated), expected);

    let plain = r#"CREATE TABLE note (id int PRIMARY KEY, title text NOT NULL, body text);"#;
    assert_eq!(from_sql(collated).files, from_sql(plain).files);

    // The statement renders the clause back, and reads back as the same
    // collations.
    let tables = parse_schema(collated).expect("schema should parse");
    let rendered = format!("{};", tables[0]);
    assert!(
        rendered.contains(r#""title" text COLLATE "C" NOT NULL"#),
        "{rendered}"
    );
    assert_eq!(collations(&rendered), expected);
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    a statement the bridge does
// not read is named, never skipped
#[test]
fn unsupported_statements_are_named() {
    assert_error(
        "CREATE TABLE t (id int); ALTER TABLE t ADD COLUMN b int;",
        "unsupported DDL: ALTER TABLE at statement 2",
    );
    assert_error(
        "CREATE TABLE t (id int); CREATE TRIGGER x BEFORE INSERT ON t EXECUTE FUNCTION f();",
        "unsupported DDL: CREATE TRIGGER at statement 2",
    );
    assert_error(
        "CREATE VIEW v AS SELECT 1;",
        "unsupported DDL: CREATE VIEW at statement 1",
    );
    assert_error(
        "CREATE OR REPLACE RECURSIVE VIEW v (n) AS SELECT 1;",
        "unsupported DDL: CREATE VIEW at statement 1",
    );
    assert_error(
        "CREATE TABLE t (id int); CREATE MATERIALIZED VIEW m AS SELECT id FROM t;",
        "unsupported DDL: CREATE MATERIALIZED VIEW at statement 2",
    );
    assert_error(
        "REFRESH MATERIALIZED VIEW CONCURRENTLY m;",
        "unsupported DDL: REFRESH MATERIALIZED VIEW at statement 1",
    );
    assert_error(
        "CREATE TABLE t AS SELECT 1 AS id WITH NO DATA;",
        "unsupported DDL: CREATE TABLE AS at statement 1",
    );
    assert_error(
        "CREATE TABLE t (id int); INSERT INTO t VALUES (1);",
        "unsupported DDL: INSERT at statement 2",
    );
    assert_error(
        "CREATE SEQUENCE s;",
        "unsupported DDL: CREATE SEQUENCE at statement 1",
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    a CREATE TABLE clause with
// no entity meaning is named rather than dropped
#[test]
fn unsupported_table_clauses_are_named() {
    assert_error(
        "CREATE TABLE t (id int) PARTITION BY RANGE (id);",
        "unsupported DDL: a PARTITION BY clause on table `t` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (id int) INHERITS (u);",
        "unsupported DDL: an INHERITS clause on table `t` at statement 1",
    );
    assert_error(
        "CREATE TEMP TABLE t (id int);",
        "unsupported DDL: a temporary or unlogged table on table `t` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (id int) WITH (fillfactor = 70);",
        "unsupported DDL: a WITH storage option on table `t` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (LIKE u);",
        "unsupported DDL: a LIKE clause on table `t` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (id int, CHECK (id > 0));",
        "unsupported DDL: a CHECK constraint on table `t` at statement 1",
    );
    assert_error(
        "CREATE TABLE other.app.t (id int);",
        "unsupported DDL: a cross-database table name on table `t` at statement 1",
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    the same holds for column
// clauses the entity model has no room for
#[test]
fn unsupported_column_clauses_are_named() {
    assert_error(
        "CREATE TABLE t (id int CHECK (id > 0));",
        "unsupported DDL: a CHECK constraint on column `t`.`id` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (id int GENERATED ALWAYS AS IDENTITY (START WITH 10));",
        "unsupported DDL: sequence options on the identity of column `t`.`id` at statement 1",
    );
    assert_error(
        r#"CREATE TABLE t (name text COLLATE db.pg_catalog."C");"#,
        "unsupported DDL: a cross-database collation name on column `t`.`name` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (id int REFERENCES u);",
        "unsupported DDL: REFERENCES without a column list on column `t`.`id` at statement 1",
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    what PostgreSQL 18's grammar
// added and the entity model cannot hold yet is named, not read as the older
// shape each one resembles
#[test]
fn postgres_18_constraints_are_named() {
    // An entity's unique columns are single columns, and a temporal unique
    // key has two at least.
    assert_error(
        "CREATE TABLE t (id int, during tstzrange, UNIQUE (id, during WITHOUT OVERLAPS));",
        "unsupported DDL: a WITHOUT OVERLAPS unique key on table `t` at statement 1",
    );
    // PostgreSQL refuses PERIOD on one side alone (42830) and a temporal key
    // over its period alone (42601).
    assert_error(
        "CREATE TABLE t (id int, during tstzrange,
            FOREIGN KEY (id, PERIOD during) REFERENCES u (id, during));",
        "unsupported DDL: a PERIOD on one side of a foreign key on table `t` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (during tstzrange, PRIMARY KEY (during WITHOUT OVERLAPS));",
        "unsupported DDL: a WITHOUT OVERLAPS key over its period alone on table `t` at statement 1",
    );
    // A NOT ENFORCED anywhere but after a foreign key is named, where
    // PostgreSQL refuses it too (0A000 on a key, 42601 on a column's NOT NULL).
    assert_error(
        "CREATE TABLE t (id int NOT NULL NOT ENFORCED);",
        "unsupported DDL: a NOT ENFORCED constraint on column `t`.`id` at statement 1",
    );
    // An enforced foreign key is what it always was.
    assert!(
        parse_schema("CREATE TABLE t (u_id int, FOREIGN KEY (u_id) REFERENCES u (id) ENFORCED);")
            .is_ok()
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    an explicit ENFORCED on a column's
// REFERENCES rides on the statement, and the entity is the plain key's
// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    an ENFORCED anywhere else is
// named, as PostgreSQL refuses it there
#[test]
fn an_explicit_enforced_rides_on_the_statement() {
    let parent = "CREATE TABLE u (id int PRIMARY KEY);";
    let enforced = format!(
        "{parent} CREATE TABLE t (id int PRIMARY KEY, u_id int REFERENCES u (id) ENFORCED);"
    );
    let tables = parse_schema(&enforced).expect("schema should parse");
    let foreign_keys = tables[1].get_foreign_key_create_stmts();
    assert_eq!(
        foreign_keys
            .iter()
            .map(|key| key.get_foreign_key().get_enforcement())
            .collect::<Vec<_>>(),
        [Some(Enforcement::Enforced)]
    );
    let rendered = format!("{};", tables[1]);
    assert!(
        rendered.contains(r#"REFERENCES "u" ("id") ENFORCED"#),
        "{rendered}"
    );

    let plain =
        format!("{parent} CREATE TABLE t (id int PRIMARY KEY, u_id int REFERENCES u (id));");
    assert_eq!(from_sql(&enforced).files, from_sql(&plain).files);

    assert_error(
        "CREATE TABLE t (id int NOT NULL ENFORCED);",
        "unsupported DDL: an ENFORCED clause on column `t`.`id` at statement 1",
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    a NOT NULL constraint's name and
// NO INHERIT ride on the statement, from the column or the table, and the
// entity generated from it is the one the plain NOT NULL generates
#[test]
fn a_not_null_constraint_rides_on_the_statement() {
    let declared = r#"CREATE TABLE note (
        id int CONSTRAINT "id present" NOT NULL PRIMARY KEY,
        title text NOT NULL NO INHERIT,
        body text,
        tag text NULL,
        stamp int,
        CONSTRAINT body_present NOT NULL body NO INHERIT,
        NOT NULL tag NOT VALID,
        NOT NULL stamp,
        CONSTRAINT stamp_present NOT NULL stamp
    );"#;
    let tables = parse_schema(declared).expect("schema should parse");
    let not_nulls = tables[0]
        .get_columns()
        .iter()
        .map(|column| {
            let specs = column.get_column_spec();
            let not_null = specs.iter().find_map(|spec| match spec {
                ColumnSpec::NotNull { name, no_inherit } => {
                    Some((name.as_ref().map(|name| name.to_string()), *no_inherit))
                }
                _ => None,
            });
            let nullable = specs.iter().any(|spec| matches!(spec, ColumnSpec::Null));
            (column.get_column_name(), not_null, nullable)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        not_nulls,
        [
            (
                "id".to_owned(),
                Some((Some("id present".to_owned()), false)),
                false
            ),
            ("title".to_owned(), Some((None, true)), false),
            (
                "body".to_owned(),
                Some((Some("body_present".to_owned()), true)),
                false
            ),
            ("tag".to_owned(), Some((None, false)), false),
            (
                "stamp".to_owned(),
                Some((Some("stamp_present".to_owned()), false)),
                false
            ),
        ]
    );
    let rendered = format!("{};", tables[0]);
    for clause in [
        r#""id" integer CONSTRAINT "id present" NOT NULL"#,
        r#""title" text NOT NULL NO INHERIT"#,
        r#""body" text CONSTRAINT "body_present" NOT NULL NO INHERIT"#,
    ] {
        assert!(rendered.contains(clause), "{rendered}");
    }

    let plain = r#"CREATE TABLE note (
        id int NOT NULL PRIMARY KEY,
        title text NOT NULL,
        body text NOT NULL,
        tag text NOT NULL,
        stamp int NOT NULL
    );"#;
    assert_eq!(from_sql(declared).files, from_sql(plain).files);
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    two NOT NULL clauses on one
// column that PostgreSQL would refuse to make one constraint of are refused
#[test]
fn conflicting_not_null_constraints_are_refused() {
    for (columns, problem) in [
        (
            "id int CONSTRAINT one NOT NULL CONSTRAINT two NOT NULL",
            "names its NOT NULL constraint both `one` and `two`",
        ),
        (
            "id int CONSTRAINT one NOT NULL, CONSTRAINT two NOT NULL id",
            "names its NOT NULL constraint both `one` and `two`",
        ),
        (
            "id int NOT NULL, NOT NULL id NO INHERIT",
            "declares NOT NULL both with and without NO INHERIT",
        ),
    ] {
        assert_error(
            &format!("CREATE TABLE t ({columns});"),
            &format!("statement 1: column `t`.`id` {problem}"),
        );
    }
    assert_error(
        "CREATE TABLE t (id int, NOT NULL missing);",
        "statement 1: table `t` has no column `missing`",
    );
}

// [spec:pgorm:sem:codegen.ddl.types+9/test]    a type spelling outside the
// vocabulary is named, and so is a modifier the vocabulary cannot hold
#[test]
fn unsupported_types_are_named() {
    assert_error(
        "CREATE TABLE t (data hstore);",
        "unsupported DDL: type `hstore` on column `t`.`data` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (at timestamp(3));",
        "unsupported DDL: `timestamp` with a type modifier on column `t`.`at` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (bits varbit);",
        "unsupported DDL: `varbit` without a length on column `t`.`bits` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (tags text[3]);",
        "unsupported DDL: a sized array on column `t`.`tags` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (grid text[][]);",
        "unsupported DDL: a multi-dimensional array on column `t`.`grid` at statement 1",
    );
}

// [spec:pgorm:def:codegen.ddl+4/test]    a type the builder can spell but codegen
// cannot render passes the bridge and is refused by the transform gate
#[test]
fn types_codegen_cannot_render_reach_the_gate() {
    assert!(parse_schema("CREATE TABLE t (net inet);").is_ok());
    assert_error(
        "CREATE TABLE t (net inet);",
        "table `t` column `net`: column type Inet is not supported by codegen",
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    an index clause the builder
// cannot express is named
#[test]
fn unsupported_index_clauses_are_named() {
    let table = "CREATE TABLE t (id int, name text);";
    assert_error(
        &format!("{table} CREATE INDEX i ON t (name) WHERE id > 0;"),
        "unsupported DDL: a WHERE clause on index `i` at statement 2",
    );
    assert_error(
        &format!("{table} CREATE INDEX i ON t (lower(name));"),
        "unsupported DDL: an expression column on index `i` at statement 2",
    );
    assert_error(
        &format!("{table} CREATE INDEX i ON t (name) INCLUDE (id);"),
        "unsupported DDL: an INCLUDE clause on index `i` at statement 2",
    );
    assert_error(
        &format!("{table} CREATE INDEX CONCURRENTLY i ON t (name);"),
        "unsupported DDL: CONCURRENTLY on index `i` at statement 2",
    );
    assert_error(
        &format!("{table} CREATE INDEX i ON t (name NULLS FIRST);"),
        "unsupported DDL: a NULLS FIRST or NULLS LAST clause on index `i` at statement 2",
    );
    // A unique index is carried as the table constraint enforcing it, whose
    // key has no ordering and no access method to hold either.
    assert_error(
        &format!("{table} CREATE UNIQUE INDEX i ON t (name DESC);"),
        "unsupported DDL: a DESC column on unique index `i` at statement 2",
    );
    assert_error(
        &format!("{table} CREATE UNIQUE INDEX i ON t USING hash (name);"),
        "unsupported DDL: an access method other than btree on unique index `i` at statement 2",
    );
}

// [spec:pgorm:sem:codegen.ddl.objects+10/test]    a unique index folds into the unique
// constraint that enforces it, keeping its name, columns and NULLS NOT DISTINCT; an explicit
// ASC, `USING btree` and IF NOT EXISTS fold away with nothing lost
#[test]
fn a_unique_index_folds_into_its_constraint() {
    let table = "CREATE TABLE t (id int, name text, code text);";
    let folded = |index: &str| {
        let tables = parse_schema(&format!("{table} {index}")).expect("schema should parse");
        tables[0].to_string()
    };
    let columns = r#"CREATE TABLE "t" ( "id" integer, "name" text, "code" text"#;

    for index in [
        "CREATE UNIQUE INDEX t_name ON t (name, code) NULLS NOT DISTINCT;",
        "CREATE UNIQUE INDEX IF NOT EXISTS t_name ON t USING btree (name ASC, code) \
         NULLS NOT DISTINCT;",
    ] {
        assert_eq!(
            folded(index),
            format!(
                r#"{columns}, CONSTRAINT "t_name" UNIQUE NULLS NOT DISTINCT ("name", "code") )"#
            ),
            "{index}"
        );
    }

    // A plain index states no entity fact and folds into nothing, ordered or
    // not, under any access method.
    assert_eq!(
        folded("CREATE INDEX t_code ON t USING hash (code); CREATE INDEX t_id ON t (id DESC);"),
        format!("{columns} )")
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    a COMMENT the bridge cannot
// attach is named
#[test]
fn unsupported_comment_targets_are_named() {
    assert_error(
        "COMMENT ON SCHEMA public IS 'x';",
        "unsupported DDL: COMMENT ON an object other than a table or column at statement 1",
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    a statement that names an
// object the file does not declare is named too
#[test]
fn unresolved_references_are_named() {
    assert_error(
        "CREATE INDEX i ON missing (id);",
        "statement 1: no CREATE TABLE for table `missing`",
    );
    assert_error(
        "CREATE TABLE t (id int); COMMENT ON COLUMN t.missing IS 'x';",
        "statement 2: table `t` has no column `missing`",
    );
    assert_error(
        "CREATE TABLE t (id int); CREATE TABLE t (id int);",
        "statement 2: table `t` is declared twice",
    );
    assert_error(
        "CREATE TYPE s AS ENUM ('a'); CREATE TYPE s AS ENUM ('b');",
        "statement 2: type `s` is declared twice",
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    a table declaring a second
// primary key is named in every spelling PostgreSQL refuses (42P16), rather
// than read as the composite key one `PRIMARY KEY (a, b)` declares
#[test]
fn a_second_primary_key_is_named() {
    let refused = "statement 2: table `t` declares more than one primary key";
    for table in [
        "CREATE TABLE t (a int PRIMARY KEY, b int PRIMARY KEY);",
        "CREATE TABLE t (a int PRIMARY KEY, b int, PRIMARY KEY (a, b));",
        "CREATE TABLE t (a int, b int, PRIMARY KEY (a, b), PRIMARY KEY (b));",
        "CREATE TABLE t (a int, b int PRIMARY KEY, PRIMARY KEY (a));",
        "CREATE TABLE t (a int PRIMARY KEY PRIMARY KEY);",
    ] {
        assert_error(&format!("CREATE TABLE u (id int); {table}"), refused);
    }
    assert!(parse_schema("CREATE TABLE t (a int, b int, PRIMARY KEY (a, b));").is_ok());
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    a key naming one column
// twice, which PostgreSQL refuses (42701), is named rather than read with or
// without its repeat; a unique index doing so is named as a DESC column on one is
#[test]
fn a_key_naming_a_column_twice_is_named() {
    for (table, column) in [
        ("CREATE TABLE t (a int, b int, PRIMARY KEY (a, a));", "a"),
        ("CREATE TABLE t (a int, b int, UNIQUE (a, b, b));", "b"),
        (
            r#"CREATE TABLE t ("A" int, b int, CONSTRAINT k UNIQUE (b, "A", b, "A"));"#,
            "b",
        ),
    ] {
        assert_error(
            &format!("CREATE TABLE u (id int); {table}"),
            &format!("statement 2: table `t` names column `{column}` twice in one key"),
        );
    }
    assert_error(
        "CREATE TABLE t (a int, b int); CREATE UNIQUE INDEX i ON t (a, b, a);",
        "unsupported DDL: column `a` named twice on unique index `i` at statement 2",
    );
    // Case is the server's: "A" and a are two columns, so neither key repeats.
    assert!(parse_schema(r#"CREATE TABLE t ("A" int, a int, PRIMARY KEY ("A", a));"#).is_ok());
    assert!(
        parse_schema(r#"CREATE TABLE t ("A" int, a int); CREATE UNIQUE INDEX i ON t ("A", a);"#)
            .is_ok()
    );
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    a foreign key onto a table
// or a column the file never declares is named too — by the transform gate the
// whole pipeline runs, which is where every table is in hand at once
#[test]
fn unresolved_foreign_keys_are_named() {
    assert_error(
        "CREATE TABLE orders (id serial PRIMARY KEY, customer_id integer REFERENCES customers (id));",
        "table `orders`: relation to `customers` names a table the schema does not define",
    );
    assert_error(
        "CREATE TABLE customers (id serial PRIMARY KEY);
         CREATE TABLE orders (id serial PRIMARY KEY, customer_id integer REFERENCES customers (code));",
        "table `orders`: relation to `customers` references column `code`, which `customers` does \
         not have",
    );
}

// [spec:pgorm:def:codegen.ddl+4/test]    text the PostgreSQL grammar rejects
// comes back as the parser's own message
#[test]
fn invalid_sql_reports_the_parser_message() {
    let message = error("CREATE TABLE t (id int;");
    assert!(
        message.starts_with("schema SQL did not parse: "),
        "unexpected message: {message}"
    );
    assert!(
        message.contains("syntax error"),
        "unexpected message: {message}"
    );
}

// [spec:pgorm:def:codegen.ddl+4/test]    the bridge is the inverse of the DDL
// builder: statements rendered to text and parsed back generate the same
// entities as the statements themselves
#[test]
fn rendered_ddl_round_trips_through_the_bridge() {
    let statements = cake_schema();
    let text = statements
        .iter()
        .map(|statement| format!("{statement};"))
        .collect::<Vec<_>>()
        .join("\n");

    let direct = generate(cake_schema(), Opts::default());
    let round_tripped = from_sql(&text);

    assert_eq!(round_tripped.files, direct.files);
}

// [spec:pgorm:sem:codegen.ddl.types+9/test]    the types that once shared a
// spelling with another variant now each recover themselves
#[test]
fn one_spelling_one_variant_round_trips() {
    let statements = || {
        vec![keyed_with(
            "ledger",
            &["id"],
            vec![
                serial("id"),
                typed("payload", ColumnType::Bytea),
                typed("seen", ColumnType::Timestamp),
                typed("amount", ColumnType::Money),
                typed("width", ColumnType::SmallInteger),
            ],
        )]
    };
    let text = statements()
        .iter()
        .map(|statement| format!("{statement};"))
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        from_sql(&text).files,
        generate(statements(), Opts::default()).files
    );
}

// [spec:pgorm:sem:codegen.ddl.objects+10/test]    the round trip holds for the
// statements outside the table too: an enum type and a unique index
#[test]
fn enum_and_unique_index_round_trip() {
    let statements = || {
        let mut task = keyed_with(
            "task",
            &["id"],
            vec![
                serial("id"),
                enum_col("state", "task_state", &["open", "done"]),
                col("code").string().not_null().to_owned(),
            ],
        );
        task.unique(unique_key("task", "code"));
        vec![task.take()]
    };
    let enum_type = Type::create(runtime_name("task_state"))
        .values(["open", "done"])
        .to_string();
    let text = statements()
        .iter()
        .map(|statement| format!("{statement};"))
        .fold(format!("{enum_type};\n"), |mut text, statement| {
            text.push_str(&statement);
            text
        });

    assert_eq!(
        from_sql(&text).files,
        generate(statements(), Opts::default()).files
    );
}

// [spec:pgorm:sem:codegen.ddl.types+9/test]    each built-in range and multirange type
// reads back as the `Range` or `Multirange` of its subtype
#[test]
fn range_types_map_through_the_vocabulary() {
    let generated = from_sql(
        "CREATE TABLE span (\
             id integer PRIMARY KEY, \
             a int4range NOT NULL, b int8range NOT NULL, c numrange NOT NULL, \
             d daterange NOT NULL, e tsrange NOT NULL, f tstzrange, \
             g int4multirange NOT NULL, h int8multirange NOT NULL, i nummultirange NOT NULL, \
             j datemultirange NOT NULL, k tsmultirange NOT NULL, l tstzmultirange\
         );",
    );
    let span = generated.file("span.rs");
    for field in [
        "pub a: Range<i32>,",
        "pub b: Range<i64>,",
        "pub c: Range<Decimal>,",
        "pub d: Range<Date>,",
        "pub e: Range<DateTime>,",
        "pub f: Option<Range<DateTimeWithTimeZone> >,",
        "pub g: Multirange<i32>,",
        "pub h: Multirange<i64>,",
        "pub i: Multirange<Decimal>,",
        "pub j: Multirange<Date>,",
        "pub k: Multirange<DateTime>,",
        "pub l: Option<Multirange<DateTimeWithTimeZone> >,",
    ] {
        assert_contains(span, field);
    }
    // The derive takes the column type from the field's own `ValueType`, so
    // the compact form states none.
    assert_not_contains(span, "column_type");
}

// [spec:pgorm:req:codegen.entity.types.unsupported+11/test]    an array of ranges or
// multiranges is a `Vec` of them, the derive taking its column type from the
// field's own `ValueType` as it does for a range
// [spec:pgorm:sem:codegen.ddl.types+9/test]    and a range type takes no modifier
#[test]
fn array_of_ranges_generates_a_vec() {
    let generated = from_sql(
        "CREATE TABLE span_list (\
             id integer PRIMARY KEY, \
             a int4range[] NOT NULL, b datemultirange[] NOT NULL, c tstzrange[]\
         );",
    );
    let span_list = generated.file("span_list.rs");
    for field in [
        "pub a: Vec<Range<i32> >,",
        "pub b: Vec<Multirange<Date> >,",
        "pub c: Option<Vec<Range<DateTimeWithTimeZone> > >,",
    ] {
        assert_contains(span_list, field);
    }
    assert_not_contains(span_list, "column_type");
    assert_error(
        "CREATE TABLE span (id integer PRIMARY KEY, a int4range(3));",
        "unsupported DDL: `int4range` with a type modifier on column `span`.`a` at statement 1",
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    an identity inside a composite
// key is carried, not refused, and the generated entity declares it on its
// column and compiles: the key is not generated whole, the column is
// [spec:pgorm:sem:codegen.entity.compact.attrs+12/test]    `identity` follows
// `primary_key`, and the identity column carries no `auto_increment = false`
#[test]
fn composite_key_identity_generates_a_compiling_entity() {
    let generated = from_sql(TENANT_TICKET);
    assert_contains(
        generated.file("tenant_ticket.rs"),
        include_str!("sql/tenant_ticket.rs"),
    );

    assert!(!tenant_ticket::PrimaryKey::auto_increment());
    assert_eq!(
        tenant_ticket::Column::Id.def(),
        ColumnType::Integer.def().identity()
    );
    assert_eq!(
        tenant_ticket::Column::TenantId.def(),
        ColumnType::Integer.def()
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    `BY DEFAULT` keeps its form, an
// identity column is NOT NULL unasked, and each key column declares its own form
// [spec:pgorm:sem:codegen.entity.pk+4/test]    a key every column of which is
// an identity is generated whole
#[test]
fn identity_forms_reach_their_columns() {
    let by_default = from_sql(
        "CREATE TABLE seq_row (id int GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, n int);",
    );
    assert_contains(
        by_default.file("seq_row.rs"),
        "#[pgorm(primary_key, identity_by_default)] pub id: i32,",
    );

    let pair = from_sql(
        "CREATE TABLE pair (a int GENERATED ALWAYS AS IDENTITY, \
         b bigint GENERATED BY DEFAULT AS IDENTITY, PRIMARY KEY (a, b));",
    );
    assert_contains(
        pair.file("pair.rs"),
        "#[pgorm(primary_key, identity)] pub a: i32, \
         #[pgorm(primary_key, identity_by_default)] pub b: i64,",
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    a table-level FOREIGN KEY over two
// columns is bridged as one key pairing them in order, and the entities
// generated from it compile, the owning side's relation and the target's
// inverse each joining on both pairs
#[test]
fn a_composite_foreign_key_joins_on_both_pairs() {
    let statements = parse_schema(TENANT_TASKS).expect("the schema parses");
    let foreign_keys = statements[1].get_foreign_key_create_stmts();
    assert_eq!(foreign_keys.len(), 1);
    let key = foreign_keys[0].get_foreign_key();
    assert_eq!(key.get_columns(), ["tenant_id", "task_id"]);
    assert_eq!(key.get_ref_columns(), ["tenant_id", "id"]);

    // The writer closes an attribute list with a trailing comma, which rustfmt
    // drops from the compiled copy, so the comparison sets it aside.
    let generated = from_sql(TENANT_TASKS);
    for (file, fixture) in [
        ("tenant_task.rs", include_str!("sql/tenant_task.rs")),
        (
            "tenant_task_note.rs",
            include_str!("sql/tenant_task_note.rs"),
        ),
    ] {
        let written = norm(generated.file(file)).replace(", )", ")");
        assert!(written.contains(&norm(fixture)), "{file}: {written}");
    }

    let (joined, _) = tenant_task_note::Entity::find()
        .join(
            JoinType::InnerJoin,
            tenant_task_note::Relation::TenantTask.def(),
        )
        .build();
    assert!(
        joined.ends_with(
            r#"INNER JOIN "tenant_task" ON "tenant_task_note"."tenant_id" = "tenant_task"."tenant_id" AND "tenant_task_note"."task_id" = "tenant_task"."id""#
        ),
        "{joined}"
    );
    let task = tenant_task::Model {
        tenant_id: 7,
        id: 3,
        title: "triage".to_owned(),
    };
    let (notes, _) = task.find_related(tenant_task_note::Entity).build();
    assert!(
        notes.ends_with(
            r#"INNER JOIN "tenant_task" ON "tenant_task"."tenant_id" = "tenant_task_note"."tenant_id" AND "tenant_task"."id" = "tenant_task_note"."task_id" WHERE "tenant_task"."tenant_id" = $1 AND "tenant_task"."id" = $2"#
        ),
        "{notes}"
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    a foreign key's deferrability and
// NOT ENFORCED, declared on its column or at table level, ride on the statement
// [spec:pgorm:sem:codegen.entity.relations+5/test]    and reach the relation the
// entity declares, which generates the same key back
// [spec:pgorm:sem:codegen.entity.transform+16/test]
#[test]
fn a_foreign_keys_check_reaches_the_relation() {
    let generated = from_sql(KEY_STAY);
    // The fixtures are compiled as well as compared, so rustfmt has dropped
    // the trailing comma the writer leaves inside an attribute.
    for (file, fixture) in [
        ("key_room.rs", include_str!("sql/key_room.rs")),
        ("key_stay.rs", include_str!("sql/key_stay.rs")),
    ] {
        let written = norm(generated.file(file)).replace(", )", ")");
        assert!(written.contains(&norm(fixture)), "{file}: {written}");
    }

    let def = key_stay::Relation::KeyRoom.def();
    assert_eq!(def.enforcement, Some(Enforcement::NotEnforced));
    assert_eq!(
        def.deferrability,
        Some(Deferrability::DeferrableInitiallyDeferred)
    );
    let regenerated = pgorm::Schema::new()
        .create_table_from_entity(key_stay::Entity)
        .expect("the entity's table is generated")
        .to_string();
    assert!(
        regenerated
            .contains(r#"REFERENCES "key_room" ("id") DEFERRABLE INITIALLY DEFERRED NOT ENFORCED"#),
        "{regenerated}"
    );

    let table_level = "CREATE TABLE key_room (id int PRIMARY KEY);
        CREATE TABLE key_stay (
            id int PRIMARY KEY,
            room_id int NOT NULL,
            FOREIGN KEY (room_id) REFERENCES key_room (id) INITIALLY DEFERRED NOT ENFORCED
        );";
    assert_eq!(from_sql(table_level).files, generated.files);

    // ENFORCED and NOT DEFERRABLE state the defaults: the statement carries
    // them as said, and the relation is the plain key's.
    let defaults = "CREATE TABLE key_room (id int PRIMARY KEY);
        CREATE TABLE key_stay (
            id int PRIMARY KEY,
            room_id int NOT NULL REFERENCES key_room (id) NOT DEFERRABLE INITIALLY IMMEDIATE ENFORCED
        );";
    let tables = parse_schema(defaults).expect("schema should parse");
    let key = tables[1].get_foreign_key_create_stmts()[0].get_foreign_key();
    assert_eq!(key.get_deferrability(), Some(Deferrability::NotDeferrable));
    assert_eq!(key.get_enforcement(), Some(Enforcement::Enforced));
    let plain = "CREATE TABLE key_room (id int PRIMARY KEY);
        CREATE TABLE key_stay (id int PRIMARY KEY, room_id int NOT NULL REFERENCES key_room (id));";
    assert_eq!(from_sql(defaults).files, from_sql(plain).files);
}

// [spec:pgorm:req:codegen.ddl.unsupported+18/test]    a foreign key's attribute clause
// said twice, or INITIALLY DEFERRED on a NOT DEFERRABLE key, is named, as
// PostgreSQL refuses each (42601); a deferrable key is still named
#[test]
fn a_refused_foreign_key_check_is_named() {
    let parent = "CREATE TABLE u (id int PRIMARY KEY);";
    for (clauses, said) in [
        ("DEFERRABLE DEFERRABLE", "DEFERRABLE"),
        ("NOT DEFERRABLE DEFERRABLE", "DEFERRABLE"),
        ("INITIALLY DEFERRED INITIALLY IMMEDIATE", "INITIALLY"),
        ("NOT ENFORCED ENFORCED", "ENFORCED"),
    ] {
        assert_error(
            &format!("{parent} CREATE TABLE t (u_id int REFERENCES u (id) {clauses});"),
            &format!("statement 2: column `t`.`u_id` gives its foreign key {said} twice"),
        );
    }
    assert_error(
        &format!(
            "{parent} CREATE TABLE t (u_id int REFERENCES u (id) NOT DEFERRABLE INITIALLY DEFERRED);"
        ),
        "statement 2: INITIALLY DEFERRED on a NOT DEFERRABLE key on column `t`.`u_id`",
    );
    assert_error(
        "CREATE TABLE t (id int, UNIQUE (id) DEFERRABLE);",
        "unsupported DDL: a deferrable constraint on table `t` at statement 1",
    );
    assert_error(
        "CREATE TABLE t (id int UNIQUE DEFERRABLE);",
        "unsupported DDL: a deferrable constraint on column `t`.`id` at statement 1",
    );
}

// [spec:pgorm:sem:codegen.ddl.tables+11/test]    a temporal primary key and a PERIOD foreign
// key ride on the statement with their periods held apart
// [spec:pgorm:sem:codegen.entity.relations+5/test]    and reach the entity, whose key ends
// `without_overlaps` and whose relation carries `from_period` / `to_period`, generating the
// same keys back
// [spec:pgorm:sem:codegen.entity.pk+4/test]
#[test]
fn temporal_key_and_period_relation_round_trip() {
    let tables = parse_schema(TEMPORAL).expect("schema should parse");
    let key = tables[0].get_primary_key().expect("a key");
    assert_eq!(
        key.get_without_overlaps().map(|name| name.to_string()),
        Some("valid_at".to_owned())
    );
    let foreign_key = tables[1].get_foreign_key_create_stmts()[0].get_foreign_key();
    assert_eq!(foreign_key.get_columns(), ["room_id"]);
    assert_eq!(
        foreign_key
            .get_period()
            .map(|(column, ref_column)| (column.to_string(), ref_column.to_string())),
        Some(("during".to_owned(), "valid_at".to_owned()))
    );

    let generated = from_sql(TEMPORAL);
    for (file, fixture) in [
        ("temporal_room.rs", include_str!("sql/temporal_room.rs")),
        ("temporal_stay.rs", include_str!("sql/temporal_stay.rs")),
    ] {
        let written = norm(generated.file(file)).replace(", )", ")");
        assert!(written.contains(&norm(fixture)), "{file}: {written}");
    }
    assert!(temporal_room::PrimaryKey::without_overlaps());
    let room = pgorm::Schema::new()
        .create_table_from_entity(temporal_room::Entity)
        .expect("the entity's table is generated")
        .to_string();
    assert!(
        room.contains(r#"PRIMARY KEY ("id", "valid_at" WITHOUT OVERLAPS)"#),
        "{room}"
    );
    let stay = pgorm::Schema::new()
        .create_table_from_entity(temporal_stay::Entity)
        .expect("the entity's table is generated")
        .to_string();
    assert!(
        stay.contains(
            r#"FOREIGN KEY ("room_id", PERIOD "during") REFERENCES "temporal_room" ("id", PERIOD "valid_at")"#
        ),
        "{stay}"
    );
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    a temporal key whose period the table
// declares before another key column is refused, its entity's key following its fields' order
#[test]
fn a_period_declared_before_the_key_is_refused() {
    assert_error(
        "CREATE TABLE t (valid_at daterange, id int, PRIMARY KEY (id, valid_at WITHOUT OVERLAPS));",
        "table `t`: the period `valid_at` of a WITHOUT OVERLAPS key is declared before another \
         key column; an entity's key follows its fields' order, its period last",
    );
}
