# Entity code generation (pgorm-codegen)

`pgorm-codegen` turns a discovered PostgreSQL schema into Rust entity source
files. The pipeline has two stages: `EntityTransformer::transform` converts a
list of `TableCreateStatement`s into an in-memory `Entity` model, and
`EntityWriter::generate` renders those entities into output files. Everything
below describes what the code emits today; text-level shapes are pinned by the
golden fixtures under `pgorm-codegen/tests/`. A schema reaches the pipeline as
DDL text, through `entities_from_sql` and `parse_schema` at the crate root,
specified under [Schema from DDL text](#schema-from-ddl-text).

> [spec:pgorm:def:codegen.entity+6]
> The entity generator is the pipeline
> `EntityTransformer::transform(Vec<TableCreateStatement>) -> EntityWriter`
> followed by `EntityWriter::generate(&EntityWriterContext) -> WriterOutput`.
> An `Entity` carries `table_name`, `schema_name` (the source table's own
> qualifier, `None` when it had none — `codegen.entity.transform`), `columns`
> (name, `ColumnType`, `auto_increment`, `identity`, `not_null`, `unique`), `relations`,
> `conjunct_relations`,
> and `primary_keys`. A `WriterOutput` is a list of `OutputFile { name, content }`
> — the writer never touches the filesystem; its caller writes the files and
> runs `rustfmt` over them.
>
> `EntityWriterContext` selects all generation options: `with_serde`, `with_copy_enums`, `schema_name`, `lib`,
> `serde_skip_deserializing_primary_key`, `serde_skip_hidden_column`,
> `model_extra_derives`, `model_extra_attributes`, `enum_extra_derives`,
> and `enum_extra_attributes`. Extra derives are appended
> to the generated derive lists; extra attributes are emitted as additional
> `#[...]` lines on the Model struct or enum.
>
> Errors are the `#[non_exhaustive]` `Error` enum, whose two variants are
> `StdIoError(io::Error)` and `TransformError(String)` (`sql.surface`).
>
> Where generated source spells a member of one of pgorm-query's growing
> enums — a built-in `RangeType`, a `ForeignKeyAction` — the writer reads it
> from a table naming each member codegen writes, and validation refuses a
> column or relation holding one the table lacks with a `TransformError`, so a
> member pgorm-query gains is reported rather than written as another. A
> column expression's keywords are codegen's own closed set, the three
> `CURRENT_*` values its reader accepts.

## Schema discovery → Entity model

> [spec:pgorm:sem:codegen.entity.transform+16]
> `EntityTransformer::transform` builds one `Entity` per input
> `TableCreateStatement`. A table's identity is the `TableIdent` its
> `TableName` spells: the bare name, and the schema qualifying it when the
> form is `SchemaTable`. The schema is kept, not discarded — it becomes the
> entity's `schema_name` and, through it, the `schema_name` the generated
> entity declares (`codegen.entity.context`). Discarding it made every
> generated statement depend on the session's `search_path`: an entity read
> from `tenant_a.item` named only `item`, so its CRUD could reach a different
> schema's `item`, and the schema the DDL bridge had faithfully preserved
> (`codegen.ddl.tables`) was thrown away one stage later. Every statement has
> a name, so the read cannot fail and the
> `TransformError("Table name should not be empty")` that stood in for the
> nameless case is gone with the state it guarded
> (`[spec:pgorm:req:sql.ddl.create-table]`), and MUST NOT come back.
>
> Identity is what every lookup keys on: the entity map, the per-target
> counters behind `num_suffix`, the `self_referencing` test, and the inverse
> and conjunct relations synthesised below — `tenant_a.item` and
> `tenant_b.item` are two tables, and a key onto one is not a key onto the
> other. A `Relation` carries its target's identity as `ref_schema` beside
> `ref_table`; only the bare name reaches the generated text, since a module
> path names one table and the collision gate is what makes that so
> (`codegen.entity.collisions`).
>
> A reference — a foreign key's target here, an index's or comment's table in
> the DDL bridge (`codegen.ddl.objects`) — resolves by one rule: a
> schema-qualified reference names exactly that table, and an unqualified one
> names the table written without a qualifier, failing that the one table
> with that bare name. The second clause is `search_path`'s reading of an
> unqualified name in the only case where it has one answer, and it is total
> wherever generation is possible at all: two tables sharing a bare name are
> refused before any reference is resolved (`codegen.entity.collisions`), so
> "the one table with that bare name" cannot be several. A reference that
> resolves to nothing keeps the name it was written with, and is reported by
> the closure check below.
>
> Every failure this gate reports names a table by its identity —
> `` table `tenant_a.item` `` for a qualified table, `` table `item` `` for an
> unqualified one — so a message about one of two same-named tables says
> which. `<table>` below is that identity.
>
> Per column: `auto_increment` and `not_null` come from the presence of the
> matching `ColumnSpec` on the column definition, and
> `identity` from a `ColumnSpec::Identity`'s form, which also makes the column
> `not_null` — PostgreSQL's own rule for an identity column, and the one the
> derive holds the generated field to. An identity carrying sequence options
> is refused with
> ``TransformError("table `<table>` column `<column>`: an entity declares an
> identity's form, not its sequence options")``: an entity has no place for
> them (`macros.derive.entity-model.primary-key`), and dropping them would
> generate an entity whose schema counts differently from the one it was read
> from. A column's `ColumnSpec::Default` and `ColumnSpec::Generated` become
> its `default` and its generation expression and kind, read back into the
> subset `codegen.entity.expressions` holds; a part outside it — a typed
> function such as `Func::uuidv7()`, an operator such as `LIKE`, a qualified
> column — is ``TransformError("table `<table>` column `<column>`: a DEFAULT
> codegen cannot read: <part>")`` (or ``a generation expression codegen cannot
> read``), since the writer has no Rust to print for it and dropping it would
> generate an entity whose schema fills the column otherwise. So is a column
> carrying two of a `DEFAULT`, a generation expression, an identity and the
> serial family, which a statement holds as a list of specs and PostgreSQL
> refuses (`42601`): ``TransformError("table `<table>` column `<column>`: a
> column takes one of a DEFAULT, a generation expression, an identity and the
> serial family; PostgreSQL refuses two")``. Once the table's columns are
> read, a `DEFAULT` reading a column is refused, as PostgreSQL refuses it
> (`0A000`): ``TransformError("table `<table>` column `<column>`: its DEFAULT
> reads column `<x>`; a DEFAULT reads no column")``; so is a generation
> expression reading a column the table does not have, which the generated
> `Column::<X>` would not compile against: ``TransformError("table `<table>`
> column `<column>`: its generation expression reads column `<x>`, which the
> table does not have")``; and so is a `VIRTUAL` generated column in the
> primary key or a one-column unique key, which PostgreSQL does not support
> (`0A000`) and the derive refuses: ``TransformError("table `<table>` column
> `<column>`: a VIRTUAL generated column cannot be keyed, which PostgreSQL
> does not support")``. A column
> with no `ColumnType` yields
> ``TransformError("table `<table>` column `<column>`: column type should
> not be empty")``. A key is the table's (`sql.ddl.create-table`), so `unique`
> comes from the table: a column is unique when some unique key of the table
> covers that one column and nothing else. The primary key's columns are the
> table's primary key's (`get_primary_key()`), one column or a tuple, in key
> order; a table has one, so there is no second source to merge and no
> duplicate `PrimaryKey` variant to generate. Every column whose (possibly
> array-inner) type is `ColumnType::Enum` registers an `ActiveEnum` in a
> `BTreeMap` keyed by enum name, deduplicating across tables.
>
> `transform` is the pipeline's validation gate: every failure a caller's
> schema can cause MUST come back from this one call as a `TransformError`,
> leaving the writer that follows infallible on its output. Besides the
> table name and the column types (`codegen.entity.types.unsupported`), the
> gate checks that every identifier the writer will derive from a DB name —
> table, column, primary key, relation target, conjunct relation, enum name
> and enum value, in each of the snake_case, camel-case and keyword-escaped
> forms the generators use — has a legal Rust form
> (`codegen.entity.keywords`) and is that one name's alone
> (`codegen.entity.collisions`); that every primary-key name is the name of a
> column of its own table, else
> ``TransformError("table `<table>`: primary key column `<column>` is not a
> column of the table")``; and that a key has at most twelve columns, else
> ``TransformError("table `<table>`: a primary key of <n> columns; an entity's
> key has at most 12, the widest tuple its value can be")``. A wider key would
> generate a `ValueType` tuple the key traits have no impl for
> (`entity.traits.primary-key`), which the derive refuses by name
> (`macros.derive.entity-model.primary-key`) — a compile error in source the
> caller did not write, where this names the table described. A statement the caller built may
> also hold what the DDL bridge refuses before any statement is built, and two
> of those the reads above would drop without a word: a primary or unique key
> ending `WITHOUT OVERLAPS`, whose `get_columns()` is its equality columns
> alone, and a foreign key matching on a `PERIOD`, whose pairs are its
> equality pairs alone (`sql.ddl.create-table`, `sql.ddl.foreign-key`). Read
> so, the first would make its other columns the table's key or a unique
> column, and the second a relation that joins a row to every period of the
> row it references, for the reasons `codegen.ddl.unsupported` gives; so each
> is read: a temporal primary key becomes the entity's key with its period
> last and `without_overlaps` set, and a `PERIOD` foreign key a relation
> carrying the period pair. A unique key ending `WITHOUT OVERLAPS`, whose
> `get_columns()` is its equality columns alone, is refused, an entity's
> unique columns being single columns: ``TransformError("table `<table>`: an
> entity cannot hold a WITHOUT OVERLAPS unique key")``. So is a temporal key
> whose period the table declares before another key column, since an
> entity's key follows its fields' order and the derive takes
> `without_overlaps` on the last key field alone: ``TransformError("table
> `<table>`: the period `<column>` of a WITHOUT OVERLAPS key is declared before
> another key column; an entity's key follows its fields' order, its period
> last")``. So is a primary or unique key naming
> one column twice, which the builder writes as given and PostgreSQL refuses
> (`42701`, `sql.ddl.create-table`): read, it would generate a `PrimaryKey`
> enum with one variant twice, which does not compile, or make the one column
> of a unique key over `(a, a)` unique on its own, so it is
> ``TransformError("table `<table>`: a key names column `<column>` twice")``,
> the columns compared as the text the server reads.
>
> Once every table has been read, the gate also checks that the schema is
> closed under its own foreign keys: each relation's referenced table is a
> table of this schema, each referenced column a column of that table, and
> each constrained column a column of the table that owns the key. A
> generated file names its target's module and columns, so an unresolved
> reference would otherwise reach the caller as Rust that does not compile —
> a `belongs_to` onto a module nobody generated — while the absent target
> takes no inverse and no conjunct relation with it, silently. The three
> failures are
> ``TransformError("table `<table>`: relation to `<ref table>` names a table
> the schema does not define")``,
> ``TransformError("table `<table>`: relation to `<ref table>` references
> column `<column>`, which `<ref table>` does not have")`` and
> ``TransformError("table `<table>`: relation to `<ref table>` constrains
> column `<column>`, which the table does not have")``. Every foreign key
> names a referenced table, so that read cannot fail and the
> ``TransformError("... referenced table should not be empty")`` that stood in
> for the tableless case is gone with the state it guarded
> (`[spec:pgorm:req:sql.ddl.foreign-key]`), and MUST NOT come back: the
> conversion is `From<&TableForeignKey> for Relation`, not a `TryFrom`.
>
> A foreign key PostgreSQL cannot implement is refused with them: one
> matching, for equality, a column typed as a domain over an enum — directly
> or through further domains — on either side, which the server answers
> `42804` whatever the other side is (`schema.from-entity`). The bridge reads
> DDL that is never run, so a `REFERENCES` the server would refuse reaches
> the gate as written, and a statement the caller built may hold one too;
> generated, it would be a relation `Schema::create_table_from_entity`
> refuses in turn. It is ``TransformError("table `<table>`: relation to `<ref
> table>` matches column `<column>` with `<ref column>`, and PostgreSQL
> implements no foreign key with a column typed as a domain over an enum on
> either side (42804)")``, naming the first such pair. Every pairing the
> server keys generates its relation: an enum with itself, a domain over any
> other base with that base or itself, and a domain over an array of an enum
> with itself. The pair's types are read from the columns themselves, so
> every key is checked; a relation `ColumnPairs` names by bare names, which
> schema generation cannot check, is not one codegen writes.
>
> The reference and collision checks run over the tables as read, before
> inverse and conjunct relations are synthesised: those are the gate's own
> work, and these checks are what make the lookups that synthesise them
> total. Identifier validation runs over the final entities, after that
> synthesis, so the derived relations are covered too. The `format_ident!`
> and column-lookup sites downstream of the gate are therefore internal
> invariants, not caller-reachable failures.
>
> Foreign keys become `BelongsTo` relations on the owning table, keeping the
> FK's columns, referenced columns, `on_update`, and `on_delete` actions, and
> its `NOT ENFORCED`, deferrability and `PERIOD` pair: an explicit `ENFORCED` or `NOT
> DEFERRABLE` states the default and is kept as nothing said, so the entity
> is the plain key's. A
> relation whose referenced table equals its own table is flagged
> `self_referencing`. When several FKs of one table reference the same
> target table, each such relation receives a distinct 1-based `num_suffix`
> — but the numbering runs in reverse declaration order: the per-target
> counter is seeded with the number of FKs to that target and decremented as
> suffixes are handed out, so for N FKs to one target the first-declared
> receives suffix N and the last-declared receives 1 (`fruit_id1` becomes
> `Fruit2` and `fruit_id2` becomes `Fruit1`). A single FK to a target keeps
> suffix 0.
>
> Entities are held in a `BTreeMap` keyed by identity, ordered by bare name
> before schema, so all outputs that iterate entities (entity files,
> `mod.rs`, `prelude.rs`) are ordered alphabetically by table name whatever
> the schemas sort like. Before writing, each entity's `relations` are sorted
> by referenced table name and its `conjunct_relations` by target name.

> [spec:pgorm:sem:codegen.entity.transform.inverse+1]
> For every non-self-referencing relation with `num_suffix == 0`, the
> transformer adds an inverse relation to the referenced entity, pointing
> back at the FK-owning table with empty column lists. The inverse type is
> `HasOne` when the FK is unique and `HasMany` otherwise, and a key is unique
> in any of three ways: every one of its columns is a unique column of the
> owning table; some unique index of the owning table covers exactly the
> key's columns, compared as sets; or the key's column set is exactly the
> owning table's full primary-key set. The middle case is the one a
> column-at-a-time reading cannot see — a composite `UNIQUE` over the key's
> columns constrains the key as a whole while leaving every column of it free
> on its own — and it is exact in both directions: a unique index over a
> superset of the key constrains the key not at all, and is not read as
> uniqueness. Self-referencing relations and
> suffixed relations produce no inverse (the suffixed case would emit a
> `Relation` variant with no usable `Related` impl). An inverse relation is
> dropped when the target entity already has any relation to that table.

> [spec:pgorm:sem:codegen.entity.transform.conjunct+1]
> A table is treated as a many-to-many junction when it is nothing but the
> join: exactly two of its relations are `BelongsTo` relations carrying
> columns of their own, and those two column sets together are exactly the
> table's primary key. Each of the two referenced entities then receives a
> `ConjunctRelation { via: junction_table, to: other_ref_table }`. Both
> conditions are load-bearing. Only the table's own foreign keys are legs:
> the inverse relations synthesised above belong to other tables' keys, so a
> table that is referenced is no more a junction for it, and a junction that
> is referenced is no less one — the classification does not depend on the
> order the transformer works in. And a table keyed by something of its own,
> with two foreign keys beside that key, joins nothing: its primary key is
> not the two keys' columns, so the pair is two references and not a join.
> When an entity accumulates more than one conjunct relation to the same
> target (duplicated many-to-many paths), all conjunct relations to that
> target are removed — ambiguity is resolved by generating nothing. When a
> conjunct relation targets the same table as an ordinary relation, that
> relation's `impl_related` flag is cleared so only the conjunct
> (`via`-based) `Related` impl is generated.

## Output files

> [spec:pgorm:req:codegen.entity.files+1]
> `EntityWriter::generate` MUST produce: one `<table_name_snake_case>.rs`
> per entity; an index file named `lib.rs` when `lib` is true and `mod.rs`
> otherwise; `prelude.rs`; `pgorm_active_enums.rs` if and only if at
> least one enum was discovered; and `pgorm_range_types.rs`, one newtype per
> range type a schema created (`codegen.entity.types`), if and only if a
> column names one. Every generated file MUST start with the
> header line
> ``//! `pgorm` Entity, @generated by pgorm-codegen <version>``
> where `<version>` is the pgorm-codegen crate version.
>
> The index file MUST contain, in order: `pub mod prelude;`, one
> `pub mod <table_name_snake_case>;` per entity (keyword-escaped, in
> alphabetical order), `pub mod pgorm_active_enums;` when enums
> exist, and `pub mod pgorm_range_types;` last when range types do.
> `prelude.rs` MUST contain one
> `pub use super::<table_name_snake_case>::Entity as <TableNameCamelCase>;`
> per entity. Entity-file code blocks are joined with blank lines; content
> is unformatted `TokenStream` text (callers are expected to run rustfmt).

> [spec:pgorm:sem:codegen.entity.context+4]
> `EntityWriterContext::new` takes one `EntityWriterOptions` struct — every
> generation option is a named field, and its `Default` is the no-flags
> shape (no serde, `mod.rs`) — and returns
> `Result<EntityWriterContext, Error>`.
>
> The `schema_name` option is a **default, not an override**: an entity's
> schema name is the source table's own qualifier when it has one
> (`codegen.entity.transform`), and the option only when it has none. This is
> the direction that cannot lie. The qualifier is a fact the schema states
> about where the table is, and an option that overrode it would generate an
> entity naming a schema the table is demonstrably not in — the very failure
> discarding the qualifier used to produce. The option's job is the
> complementary one: naming the schema that discovery ran against, which is
> precisely the schema an unqualified name belongs to. So the two never
> disagree, and generating a mixed-schema DDL with `schema_name` set gives
> each table the schema it is actually in. `EntityWriter::gen_schema_name`
> takes the entity and the option and applies that precedence, so no
> generator can be written that forgets it; "a schema name is configured", in
> `codegen.entity.compact` and `codegen.entity.compact.model`, means the
> result of it.
>
> `new` normalizes the option lists at construction:
> `model_extra_derives` / `enum_extra_derives` pass through `bonus_derive`,
> which parses each string as a `TokenStream` and folds them into a single
> leading-comma fragment (`, A, B`); `model_extra_attributes` /
> `enum_extra_attributes` pass through `bonus_attributes`, which wraps each
> parsed string in its own `#[...]` attribute line. An extra derive or
> attribute that is not valid Rust token text is returned as
> ``TransformError("`<option>` entry `<string>` is not valid Rust token
> text")`` — naming the option field it came from — before any file is
> generated.

## The entity file

> [spec:pgorm:def:codegen.entity.compact+3]
> The writer emits one form of entity, the composite derive's
> (`[spec:pgorm:req:macros.derive.one-form]`), and no option selects another.
> Per entity it emits, in order: the imports (`use pgorm::entity::prelude::*;`,
> serde imports, and one `use super::pgorm_active_enums::<EnumName>;` per
> distinct enum used by the entity's columns); a single `Model` struct
> deriving `Clone, Debug, PartialEq, Entity` (plus `Eq`, serde
> derives, and extra derives) annotated with
> `#[pgorm(schema_name = "...", table_name = "...")]` (`schema_name` only
> when the entity has one — its source table's qualifier, else the configured
> default, per `codegen.entity.context`); a `Relation` enum deriving
> `Copy, Clone, Debug, EnumIter, Relation` whose variants carry
> `#[pgorm(...)]` relation attributes (an entity with no relations emits the
> empty `pub enum Relation {}`); the `Related` impls; and
> `impl ActiveModelBehavior for ActiveModel {}`. The `Entity` derive writes
> the `Entity`, `Column`, `PrimaryKey` and `ActiveModel` from the `Model`.
>
> `gen_compact_code_blocks` assembles the file as an ordered
> `Vec<TokenStream>`, one block per section: the import block (`gen_import`
> extended with `gen_import_active_enum` and `gen_import_range_types`),
> `gen_compact_model_struct`, `gen_compact_relation_enum`, zero or more
> `gen_impl_related` blocks, zero or more `gen_impl_conjunct_related` blocks,
> then `gen_impl_active_model_behavior`. `write_entities` prepends the
> generated-file header and joins the stringified blocks with blank lines, so
> each section is one contiguous block in the output (rendered shapes pinned
> by `tests/compact/`).

> [spec:pgorm:sem:codegen.entity.compact.attrs+12]
> In the compact Model, each field's `#[pgorm(...)]` attribute assembles
> parts in this fixed order: `column_name = "..."` when the DB column name
> is not already snake_case; `primary_key` when the column is in the primary
> key, followed by `without_overlaps` when it is a temporal key's period, its
> last column, and otherwise by `auto_increment = false` when that PK column is
> neither auto-increment, an identity nor a generated column; `identity` or
> `identity_by_default` when
> the column is one, key or not — the derive refuses an identity or a
> generation expression beside `auto_increment`, so an identity key column
> carries `primary_key, identity` and a generated one `primary_key,
> generated_stored = ..`; `default_expr = ..`, `generated_stored = ..` or
> `generated_virtual = ..` when the column has a `DEFAULT` or is generated,
> the Rust that builds the expression written bare
> (`codegen.entity.expressions`); `column_type = <expression>` wherever the type the
> derive infers from the field's Rust type
> (`macros.derive.entity-model.column-def`) is not the column's own, and on a
> `Float`, `Double` or `Bytea` column besides; `nullable` (only alongside a
> `column_type`) when the column is nullable; and `unique` when the column is
> unique. Fields needing none of these carry no `#[pgorm]` attribute.
>
> A compact entity carries each column's full type, so that the table it
> creates (`Schema::create_table_from_entity`) is the table described, every
> length, precision, scale and qualifier included, and a value written
> through it is held to them. The `column_type` value is the type's whole
> `ColumnType` expression as `codegen.entity.types` spells it, its integers
> carrying a `u32` suffix. `String` infers an unbounded `varchar`, so
> `varchar(3)` states `ColumnType::String(StringLen::N(3u32))`
> (`StringLen::Max` stating itself too), every `Char` — `char(2)` states
> `ColumnType::Char(Some(2u32))`, and `char` alone, which is `char(1)`,
> `ColumnType::Char(Some(1u32))` — states its length or
> `ColumnType::Char(None)`, `text` states `ColumnType::Text` and a named type
> `ColumnType::named("...")`; `Decimal` infers an unconstrained `numeric`, so
> `numeric(10, 2)` states `ColumnType::Decimal(Some((10u32, 2u32)))` and
> `money` `ColumnType::Money`; `Json` infers `json`, so `jsonb` states
> `ColumnType::JsonBinary`; and `Interval` infers an unqualified `interval`,
> so an interval carrying a field qualifier or a precision states its
> `IntervalSpec`:
> `ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth))`,
> `ColumnType::Interval(IntervalSpec::Any(Some(IntervalPrecision::P3)))`. Without it
> `varchar(3)` would be created as a `varchar` holding any length, and
> `interval YEAR TO MONTH` as an `interval` keeping the days and seconds the
> column discards. A domain column always states one, `ColumnType::Domain {
> name: Name::runtime("..."), schema: .., base: Arc::new(..) }`, because no Rust
> type infers a domain. An array is inferred as the array of what its
> element's Rust type infers, so it states
> `ColumnType::Array(Arc::new(ColumnType::<element>))` exactly when its element would
> other than for a `Float`, `Double` or `Bytea` element, which `f32`, `f64`
> and `Vec<u8>` infer as they are: `text[]`, `varchar(3)[]` and
> `numeric(8, 3)[]` state one, `real[]` and `integer[]` none. A created
> range's newtype is inferred as the range over what its `Range<E>`'s `E`
> infers (`codegen.entity.types`), so a column of one, or of the multirange
> beside it, states `ColumnType::CreatedRange { name: Name::runtime("..."),
> schema: .., subtype: Arc::new(..) }` or `ColumnType::CreatedMultirange { .. }`
> exactly when its
> subtype would: a range over `text` does, which `String` would infer over
> `varchar`. Every other type — the integers, `boolean`, `json`, `uuid`, the
> dates and times, an unqualified `interval`, a bare `numeric`, an unbounded
> `varchar`, an enum, a built-in range or multirange, and arrays of these —
> is what its field infers, and states none. The live round trip of a
> generated entity with a column of every kind of type codegen reads is held
> at the workspace root (`tests/codegen_column_types_tests.rs`), over the
> module `tests/codegen_column_types/` it compiles, whose files
> pgorm-codegen's `tests/column_types_tests.rs` compares the writer's output
> with.
>
> The `column_type` part is an expression the `Entity` derive takes as
> written (`macros.derive.entity-model.column-def`), so it is generated
> *program text*, and the names it uses — `Arc`, `ColumnType`, `Name`, `StringLen`,
> `IntervalSpec`, `PgInterval`, `IntervalPrecision` — resolve through the
> prelude glob the entity file imports (`[spec:pgorm:def:entity.prelude]`),
> as an enum a domain is over resolves through the import of it. Every part
> of it but the names of a named type, a domain and a created range is fixed
> by this rule — a length and `Decimal`'s precision and scale are integers,
> and an interval's qualifier is one of a closed set of variants — and the name, which comes
> from the described schema, MUST be written by rendering it as a string
> literal rather than by placing it between two quote characters. A type name
> is data: Postgres accepts names that Rust source does not, and a name
> holding a `"` pasted into the attribute either fails to parse in the
> generated file or, given a comment opener, names a *different* type with
> nothing reporting the difference. Rendered as a literal it round-trips
> whole, as a domain's name and schema are each spelled inside
> `Name::runtime("...")`. The
> shapes that spelling cannot carry are refused before the writer runs
> (`codegen.entity.types.unsupported`).
>
> An expression part's string is program text the same way, the derive
> parsing it as a Rust expression, and is held to the same discipline: it is
> printed from the expression's tree, never from the schema's text, and
> everything the schema contributes to it reaches it as a token of its own —
> a column as the `Column` variant the gate has already validated
> (`codegen.entity.keywords`), and a function name, a type name and every
> string literal as a Rust string literal — so a name or a literal holding a
> quote, a comment opener or Rust syntax is data inside the program, never
> program.

> [spec:pgorm:sem:codegen.entity.compact.model+2]
> `gen_compact_model_struct` emits the compact `Model` as one block: the
> derive attribute `Clone, Debug, PartialEq, Entity` followed by
> the `Eq` slot, the serde fragment (`codegen.entity.serde.derives`), and
> `model_extra_derives`; the struct attribute
> `#[pgorm(schema_name = "...", table_name = "...")]` (the `schema_name =`
> part present only when the entity has a schema name, and carrying the
> entity's, not the option's, when the two differ —
> `codegen.entity.context`); then
> `model_extra_attributes` as further attribute lines. Fields follow the
> entity's column order; each field is the keyword-escaped snake_case
> column name (`codegen.entity.keywords`) typed per `codegen.entity.types`,
> preceded first by its assembled `#[pgorm(...)]` attribute
> (`codegen.entity.compact.attrs`, omitted when no parts apply) and then by
> its serde attribute (`codegen.entity.serde.skip`). Primary-key membership
> is decided by matching the raw DB column name against the entity's
> `primary_keys` list.

> [spec:pgorm:sem:codegen.entity.imports+3]
> `gen_import` builds the per-file import block: always
> `use pgorm::entity::prelude::*;`, followed by the serde import selected
> by `WithSerde` — `use serde::Serialize;` for `Serialize`,
> `use serde::Deserialize;` for `Deserialize`,
> `use serde::{Deserialize, Serialize};` for `Both`, nothing for `None`.
> For entity files, `gen_import_active_enum` extends the block with one
> `use super::pgorm_active_enums::<EnumName>;` per distinct enum among the
> entity's column types (looked through `Array` via `get_inner_col_type`),
> deduplicated in first-use column order, and `gen_import_range_types` with
> one `use super::pgorm_range_types::<RangeName>;` per distinct range or
> multirange type a schema created, the same way. The same `gen_import` block —
> without enum or range imports — heads `pgorm_active_enums.rs`
> (`codegen.entity.enums`) and `pgorm_range_types.rs`, whose newtypes carry
> the serde derives (`codegen.entity.serde`); in every file the imports sit directly below
> the `write_doc_comment` header required by `codegen.entity.files`.

## Type mapping

> [spec:pgorm:sem:codegen.entity.types+12]
> Model field types come from `Column::get_rs_type`: a non-null column maps
> to `T`, a nullable column to `Option<T>`, where `T` is:
>
> | ColumnType | Rust type |
> |---|---|
> | `Char(_)`, `String(_)`, `Text`, `Named(_)` | `String` |
> | `SmallInteger` / `Integer` / `BigInteger` | `i16` / `i32` / `i64` |
> | `Float` / `Double` | `f32` / `f64` |
> | `Json`, `JsonBinary` | `Json` |
> | `Decimal(_)`, `Money` | `Decimal` |
> | `Uuid` | `Uuid` |
> | `Bytea` | `Vec<u8>` |
> | `Boolean` | `bool` |
> | `Interval(_)` | `Interval`, whatever the `IntervalSpec` |
> | `Enum { name, .. }` | UpperCamelCase of `name` |
> | `Array(inner)` | `Vec<T(inner)>` (recursive) |
> | `Range(t)` / `Multirange(t)` | `Range<E>` / `Multirange<E>`, `E` being `i32`, `i64`, `Decimal`, `Date`, `DateTime` or `DateTimeWithTimeZone` for `Int4` through `TimestampTz` |
> | `CreatedRange { name, .. }` / `CreatedMultirange { name, .. }` | UpperCamelCase of `name`, the newtype over `Range<E>` / `Multirange<E>` that `pgorm_range_types.rs` declares |
> | `Domain { base, .. }` | `T(base)` (recursive): a domain's values are its base type's |
> | `Date`, `Time`, `Timestamp`, `TimestampWithTimeZone` | per `codegen.entity.types.datetime` |
>
> No row produces an unsigned Rust integer: Postgres has no unsigned integer
> type, so a generated field MUST NOT claim one.
>
> The named types resolve through `pgorm::entity::prelude::*`, as the names
> in a stated `column_type` do (`codegen.entity.compact.attrs`). The writer
> states no `column_type` for a range column, the derive taking its type from
> the field's own `ValueType`; inside a type it does state, a built-in range
> is `ColumnType::Range(RangeType::Int4)`, `RangeType` resolving through the
> prelude too.
>
> An interval column is an `Interval` field whatever its field qualifier and
> precision: the qualifier constrains what the column stores, not the type a
> row decodes as (`[spec:pgorm:def:sql.value.interval]`). The qualifier is
> carried in the column's definition instead, so that the table the entity
> creates (`Schema::create_table_from_entity`) is the table described,
> qualifier for qualifier, and a value written through the entity is stored
> as that column stores it — the fields the qualifier leaves out dropped, the
> seconds rounded to its precision. The writer states the `IntervalSpec` in
> `column_type`, through the prelude as `RangeType` is, wherever the field's
> `Interval` would infer an unqualified `interval`
> (`codegen.entity.compact.attrs`):
> `ColumnType::Interval(IntervalSpec::Any(Some(IntervalPrecision::P3)))`,
> `ColumnType::Interval(IntervalSpec::Fields(PgInterval::DayToSecond(Some(IntervalPrecision::P3))))`.
> The live round trip of a generated
> entity with a column of each kind of qualifier is held at the workspace
> root (`tests/codegen_interval_tests.rs`), over the files
> `pgorm-codegen/tests/sql/interval/` compares the writer's output with.
>
> A range type a schema created generates one newtype, as an enum type
> generates one `ActiveEnum`: `pub struct <RangeName>(pub Range<E>);`
> deriving `Clone`, `Debug`, `PartialEq`, `Eq` unless `E` is a float, and
> `CreatedRange` under `#[pgorm(range_name = "<name>")]`, with
> `schema_name` when the type is qualified; a column of its multirange
> generates `pub struct <MultirangeName>(pub Multirange<E>);` under
> `multirange_name` the same way
> (`[spec:pgorm:def:sql.value.created-range]`). `E` is the subtype's Rust
> type as this table maps it — `i16`, `i32`, `i64`, `f32`, `f64`, `Decimal`,
> `String` (for `Char`, `String` and `Text`), `Date`, `Time`, `DateTime`,
> `DateTimeWithTimeZone` or `Uuid`. The derive, given no `column_type`,
> takes a field's column type from the newtype's `ValueType`, whose subtype
> is what `E` infers, so the writer states none where that is the column's
> subtype, and where it is not — a range over `text`, which `String` infers
> as `varchar` (`codegen.entity.compact.attrs`) — spells
> `ColumnType::CreatedRange` or `CreatedMultirange { name, schema, subtype }`
> out, the subtype as the schema describes it. The live round trip of a
> generated entity is held at the workspace root (`tests/created_range_tests.rs`),
> over the same files `pgorm-codegen/tests/sql/created_range/` compares the
> writer's output with.
>
> A domain column is a field of its base's Rust type — an integer, an enum's
> `ActiveEnum`, a created range's newtype, a `Vec` for a domain over an array
> — and an array of one a `Vec` of that. The domain's name rides in the
> column's definition, as an interval's qualifier does, so that the table the
> entity creates types the column with the domain and a value the entity
> writes is held to the domain's default, `NOT NULL` and checks. The writer
> spells `ColumnType::Domain { name: Name::runtime("<name>"), schema:
> <Some(Name::runtime("<schema>")) or None>, base: Arc::new(<base>) }` out in
> `column_type`, the base spelled as a type inside another is, an enum base
> included. A
> domain over an enum or a created range imports and generates that type as a
> column of it does. The live round trip of a generated entity whose columns
> are domains over an integer, another domain, a sized string, an enum, an
> array and a created range, a schema-qualified one and arrays of two of
> them, is held at the workspace root (`tests/codegen_domain_tests.rs`), over
> the files `pgorm-codegen/tests/sql/domain/` compares the writer's output
> with.
>
> An enum column states no `column_type`: the derive takes its type from the
> field's `ActiveEnum`, and an enum array's from the `Vec` of it. An enum
> inside a type the writer does state — a domain's base, or the element of an
> array a domain is over — is the `ActiveEnum`'s column type,
> `<Enum>::db_type().get_column_type().clone()`, since the `Arc` holding it
> holds a `ColumnType`: a domain over an enum array is
> `ColumnType::Domain { .., base: Arc::new(ColumnType::Array(Arc::new(<Enum>::db_type().get_column_type().clone()))) }`,
> the enum's schema, when it has one, carried by the `ActiveEnum`. The live
> round trip of a generated entity whose columns are arrays of an enum and of
> a schema-qualified one, nullable and not, and a domain over an enum array is
> held at the workspace root (`tests/codegen_enum_array_tests.rs`), over the
> files `pgorm-codegen/tests/sql/enum_array/` compares the writer's output
> with.
>
> The `Eq` derive is added to the Model derive list only when no column's
> type is `Float` or `Double`, checked recursively through `Array` element
> types, a created range's or multirange's subtype and a domain's base; a
> single float column suppresses `Eq` for the whole Model.

> [spec:pgorm:sem:codegen.entity.types.datetime+2]
> Date/time columns map to one fixed set of field types, the prelude's
> temporal aliases:
>
> | ColumnType | generated type |
> |---|---|
> | `Date` | `Date` |
> | `Time` | `Time` |
> | `Timestamp` | `DateTime` |
> | `TimestampWithTimeZone` | `DateTimeWithTimeZone` |
>
> `Timestamp` MUST map to the time-zone-naive type (`jiff::civil::DateTime`,
> re-exported from the prelude as `DateTime`), because that is what Postgres
> `timestamp` is; mapping it to the instant type would claim a time zone the
> column does not carry, and would disagree with the inference table's
> `DateTime`→`Timestamp` direction
> (`[spec:pgorm:sem:macros.derive.entity-model.column-def]`).
>
> There is no option selecting between date/time crates, and codegen MUST NOT
> offer one. jiff is the single temporal crate pgorm models PostgreSQL with,
> so a selector would have exactly one legal value. The previous `DateTimeCrate`
> option is the case against reintroducing it: its second variant emitted a
> `TimeDate` family of aliases that existed in no crate in the workspace, so
> the one setting a caller could change was the one that produced source that
> could not compile. Because the generated identifiers come from the prelude
> glob rather than from any named crate, repointing the aliases is how the
> temporal backing changes, and generated text does not move when it does.

> [spec:pgorm:req:codegen.entity.types.unsupported+10]
> Column types outside the mapping table are not supported, and support is
> decided when a `Column` is built rather than when it is rendered. Both
> `TryFrom<&ColumnDef> for Column` and — through it —
> `EntityTransformer::transform` MUST reject an unsupported type with
> ``TransformError("table `<table>` column `<column>`: column type <type> is
> not supported by codegen")``, where `<type>` is the `ColumnType`'s `Debug`
> form; outside `transform` the message names the column alone. `Array`
> element types are checked recursively, so an array of an unsupported
> element type is itself unsupported. An array of ranges or multiranges is
> supported as its element is — `int4range[]` is `Vec<Range<i32>>`
> (`[spec:pgorm:def:sql.value.array]`) — the compact writer stating no
> `column_type` for it, as for a range. So is an array of intervals, its
> element's qualifier with it: `interval YEAR TO MONTH[]` is `Vec<Interval>`,
> and no `IntervalSpec` is refused. A range type a schema created, and
> the multirange beside it, is supported over the subtypes
> `codegen.entity.types` maps for it, and refused
> over any other with ``TransformError("... range type `<name>` over <subtype>
> is not supported by codegen; a created range's subtype is one pgorm's
> `RangeSubtype` covers")``, `<subtype>` in `Debug` form; an array of one is
> refused as ``TransformError("... an array of range type `<name>` is not
> supported by codegen; the newtype a created range generates has no array
> conversion")``. An array of an array is refused as ``TransformError("... an
> array of an array is not supported by codegen; an entity field holds one
> dimension")``, whatever its innermost element: a field is one `Vec` of its
> element's Rust type, and `Vec<Vec<T>>` has no `ValueType`, so the entity
> would not compile. The DDL bridge refuses the `[][]` spelling that would
> describe one (`codegen.ddl.types`), but a schema described as
> `TableCreateStatement`s can hold one, so the gate holds it to the same
> refusal. A domain is supported as its base is, a domain over a
> domain recursively, and an array of one as an array of its base would be —
> an array of a domain over a created range refused as that array is — but
> for an array of a domain over an array, refused as ``TransformError("... an
> array of a domain over an array is not supported by codegen; an entity
> field holds one dimension")``; a domain over an array of an array is refused
> as that array is.
>
> A type inside the table can still be beyond the writer, in which case the
> same gate refuses it with its own message. `Enum { name, .. }` is refused
> when `name` has no Rust identifier form, because the writer emits it as one
> (`codegen.entity.keywords`). `Named(type_name)` is refused when `type_name`
> is schema-qualified, is an array, or is a verbatim type expression, with
> ``TransformError("... named column type `<name>` is not supported by
> codegen; only a bare type name survives the generated
> `ColumnType::named(\"..\")`")`` where `<name>` is the type name's SQL
> spelling: the writer respells a named type as a single-argument
> `ColumnType::named("...")`, which rebuilds one unqualified, non-array,
> quoted name, so a `TypeName` carrying more than that would be generated as
> a type other than the one described. The *characters* of a bare name are
> not constrained — it becomes a string literal, never an identifier, and
> `citext`, `CamelType` and `my type` are all names Postgres accepts — so it
> is the name's shape that is checked here and its spelling that
> `codegen.entity.compact.attrs` makes safe.
>
> Codegen MUST NOT be expected to degrade gracefully on such a type: no
> placeholder code is emitted and no file is generated, the whole run fails.
> It MUST NOT abort by panic either — the failure is a value the caller
> handles. Because no `Column` can hold an unsupported type, the wildcard
> arms of `Column::get_rs_type` and `write_col_type` are unreachable and
> stand as internal invariants.

## Serde

> [spec:pgorm:def:codegen.entity.serde+1]
> `WithSerde` has four variants parsed from the strings `none`,
> `serialize`, `deserialize`, `both` (anything else is a
> `TransformError("Unsupported enum variant '...'")`). It contributes both
> an import and extra Model/enum derives: `Serialize` adds
> `use serde::Serialize;` and the `Serialize` derive; `Deserialize` the
> mirror image; `Both` adds `use serde::{Deserialize, Serialize};` and both
> derives; `None` adds nothing. The same derives are appended to generated
> active enums and to the newtype of each range type a schema created, and
> the same import is emitted at the top of `pgorm_active_enums.rs` and of
> `pgorm_range_types.rs`: a Model serializing a field of one of those types
> needs the type to serialize too.
>
> The output compiles under every variant whatever the columns, because
> every type a generated field can hold serializes under pgorm's `with-json`
> feature: `Interval`, `Range` and `Multirange` through pgorm-query's `serde`
> feature (`[spec:pgorm:def:sql.value.interval]`,
> `[spec:pgorm:def:sql.value.range]`), and `Decimal` and the jiff and uuid
> types through their own crates'. `tests/serde_values_tests.rs` holds this
> with an entity of interval, built-in range, multirange and created range
> columns — nullable, qualified and as arrays — generated and compiled under
> each variant, its rows written and read back through JSON by the variants
> that derive each half.

> [spec:pgorm:sem:codegen.entity.serde.skip]
> Two flags add field-level serde attributes, and both are effective only
> when serde output is enabled: `serde_skip_hidden_column` (any of
> `Serialize`/`Deserialize`/`Both`) puts `#[serde(skip)]` on every column
> whose DB name starts with `_`; `serde_skip_deserializing_primary_key`
> (`Deserialize` or `Both` only) puts `#[serde(skip_deserializing)]` on
> primary-key fields. The hidden-column check wins: a hidden primary key
> gets `#[serde(skip)]`, not `#[serde(skip_deserializing)]`. With
> `WithSerde::None` both flags are inert.

> [spec:pgorm:sem:codegen.entity.serde.derives+3]
> `WithSerde::extra_derive` is the single producer of the serde derive
> fragment: an empty stream for `None`, otherwise a leading-comma fragment
> (`, Serialize`, `, Deserialize`, or `, Serialize, Deserialize` for
> `Both`). The Model writer splices it at a fixed slot — after
> the base derives and the conditional `Eq`, before `model_extra_derives` —
> so a serde-enabled compact Model derives exactly
> `Clone, Debug, PartialEq, Entity, Eq, Serialize, Deserialize`
> (pinned by `tests/compact_with_serde/cake_both.rs`). The same fragment is
> appended to generated active enums after their base derives and optional
> `Copy` (`codegen.entity.enums`), and to each range newtype after
> `CreatedRange` (`codegen.entity.types`), so a newtype over a
> `Range<i32>` derives exactly
> `Clone, Debug, PartialEq, Eq, CreatedRange, Serialize, Deserialize`
> under `Both`. The fragment and the serde import
> (`codegen.entity.imports`) always travel together, both derived from the
> one `WithSerde` value in the context.

## Primary keys

> [spec:pgorm:sem:codegen.entity.pk+4]
> A key's facts surface as field attributes (`codegen.entity.compact.attrs`):
> `primary_key` on each key column, followed by `without_overlaps` on a
> temporal key's period, by `auto_increment = false` on a column that is
> neither auto-increment, an identity nor a generated column, and by
> `identity` / `identity_by_default` on an identity. The derive reads them
> into `PrimaryKeyTrait` (`macros.derive.entity-model.primary-key`): its
> `ValueType` the key column's Rust type, or the tuple of them for a
> composite key, and its `auto_increment()` whether the database generates
> the whole key (`entity.traits.primary-key`), reading the key's own columns
> and no others: true when every key column is an identity or a generated
> column, and otherwise only for a one-column key that is auto-increment. A
> serial column outside the key does not make the key generated, and
> neither does one identity column of a composite key: a table keyed
> `(tenant_id, id)` with `id serial` would otherwise draw both key columns
> from the serial family when schema generation built it from the entity.
>
> A serial column inside a composite key has no attribute to land on — the
> derive offers per-column generation only as an identity — so it generates
> as a plain key column. The entity still works against the table it was read
> from, whose `DEFAULT nextval(..)` fills the column an insert leaves `NotSet`;
> only schema generation from the entity loses the default.

## Column expressions

> [spec:pgorm:sem:codegen.entity.expressions+1]
> A column's `DEFAULT`, and the expression a generated column is computed
> from, are read into one closed subset of PostgreSQL's expressions, which
> the DDL bridge reads from the parse tree (`codegen.ddl.tables`), the
> transform reads from a statement's `SimpleExpr` (`codegen.entity.transform`)
> and the writer prints as Rust (`codegen.entity.compact.attrs`). The subset is: a column of the same table by
> its unqualified name; an `integer`, `bigint` or `numeric` literal, the last
> kept as the exact decimal it spells (`1.50` stays `1.50`) within 18 digits;
> a string literal; `TRUE`, `FALSE` and `NULL`; `CURRENT_DATE`,
> `CURRENT_TIME` and `CURRENT_TIMESTAMP`; a call of a function by its
> unqualified name in call syntax, `pg_catalog`'s qualification dropped, and
> `COALESCE`, `GREATEST`, `LEAST` and `NULLIF`, which the grammar builds
> itself and pgorm writes bare (`sql.ast.func`); a cast to a type named by
> its name and schema, with at most one unsized array dimension; the
> arithmetic operators `+ - * / %`, the comparisons `= <> < > <= >=`, `||`,
> `AND`, `OR` and `NOT`; and `IS [NOT] NULL`. Everything else is named where
> it is met, never approximated (`codegen.ddl.unsupported`,
> `codegen.entity.transform`).
>
> The subset is codegen's own type, not `SimpleExpr`. The writer has to print
> the Rust that builds an expression, and a `SimpleExpr` can hold what no
> printed Rust rebuilds — a typed function, a verbatim type, any operator —
> where the subset holds what the reader reads and nothing else, so every
> expression it holds the writer can print, and the gate can tell a
> statement's expression it cannot hold from one it can. Lowered, an
> expression is the `SimpleExpr` the bridged statement carries; printed, it
> is the Rust that builds that same `SimpleExpr` inside the entity's module:
> `Expr::col(Column::<X>)`, `Expr::val(..)` (`Decimal::new(m, s)` for a
> numeric, an `i64`-suffixed literal for a `bigint`),
> `pgorm::pgorm_query::Keyword::Null`, `Expr::current_date()` and its two
> siblings, `Func::named(Name::runtime("f"))` with an `.arg(..)` per
> argument, `.cast_as(Name::runtime("t"))`, or
> `.cast_as_type(pgorm::pgorm_query::TypeName::new(..)..)` for a qualified or
> array type, the `Expr` method of each operator (`AND` and `OR` through
> `.binary(pgorm::pgorm_query::BinOper::And | Or, ..)`), `.not()`,
> `.is_null()` and `.is_not_null()` — every name either one
> `pgorm::entity::prelude` exports or a path from `pgorm`. The schema built
> from the generated entity carries each expression the bridge read, rendered
> alike, and the live suite holds the server to storing it as it stored the
> original's.
>
> A column the gate passes (`codegen.entity.transform`) — its generation
> expression reading only columns of its table, its `DEFAULT` reading none,
> filled one way, and not a keyed `VIRTUAL` generated column — is one the
> derive accepts and PostgreSQL creates. A key whose every column is
> generated is the database's to fill (`codegen.entity.pk`). Schema
> generation from the entity is what the expressions are for: the generated
> entity reads and writes the table it was read from with or without them,
> an insert leaving the column `NotSet` taking the server's default either
> way, so carrying them is what makes the entity a faithful source for the
> schema it was read from.

## Relations

> [spec:pgorm:sem:codegen.entity.relations+5]
> `Relation` enum variants are named by the UpperCamelCase of the referenced
> table, with two adjustments: a self-referencing relation is named
> `SelfRef`, and a nonzero `num_suffix` is appended (`Fruit1`, `Fruit2`,
> `SelfRef1`, ...). Each variant carries a `#[pgorm]`
> attribute, every value but the key's name Rust written bare
> (`macros.derive.code-values`): FK-owning relations emit
> `#[pgorm(belongs_to = <Entity>, from = Column::<Src>, to = <Entity path>Column::<Ref>, on_update = ForeignKeyAction::<Action>, on_delete = ForeignKeyAction::<Action>)]`
> where `<Entity>` is `super::<module>::Entity` (or plain `Entity` when
> self-referencing), multi-column FKs render `from`/`to` as parenthesized
> tuples `(Column::A, Column::B)`, and `on_update`/`on_delete` appear only
> when the FK declared an action (`Restrict`, `Cascade`, `SetNull`,
> `NoAction`, `SetDefault`), with `from_period = Column::<Src>` and
> `to_period = <Entity path>Column::<Ref>` before them for a `PERIOD` foreign
> key, and followed by `deferrability = Deferrability::<Deferrability>` and
> `enforcement = Enforcement::NotEnforced` where the key declared them. Inverse
> relations emit
> `#[pgorm(has_one = <Entity>)]` or `#[pgorm(has_many = <Entity>)]` with no
> `from`/`to`. A relation
> whose constrained and referenced column lists differ in length is rejected by
> `Relation::validate`, so no generated relation can name a column on one side
> without its counterpart on the other.

> [spec:pgorm:sem:codegen.entity.relations.related+1]
> For each relation that is not self-referencing, has `num_suffix == 0`, and
> still has `impl_related` set, the writer emits
> `impl Related<super::<module>::Entity> for Entity` with
> `fn to() -> RelationDef { Relation::<Variant>.def() }`. Self-referencing
> and suffixed relations get `Relation` variants but no `Related` impl
> (`Related` can only be implemented once per target entity), and
> conjunct-shadowed relations are replaced by the `via` impl of
> `codegen.entity.relations.via`.

> [spec:pgorm:sem:codegen.entity.relations.via+1]
> Each conjunct relation emits a many-to-many `Related` impl:
>
> `impl Related<super::<to>::Entity> for Entity` with
> `fn to() -> RelationDef { super::<via>::Relation::<To>.def() }` and
> `fn via() -> Option<RelationDef> { Some(super::<via>::Relation::<Self>.def().rev()) }`
>
> where `<via>` is the junction module, `<To>` the target's variant in the
> junction's `Relation` enum, and `<Self>` the current entity's variant
> there. These impls are emitted after the plain `Related` impls.

## Active enums

> [spec:pgorm:sem:codegen.entity.enums+5]
> All discovered database enums are generated into a single
> `pgorm_active_enums.rs`, in alphabetical order by enum name. Each becomes
> `#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]`
> (plus `Copy` when `with_copy_enums`, then serde derives, then
> `enum_extra_derives`) with
> `#[pgorm(rs_type = String, enum_name = "<db name>")]`, no `db_type` being
> how the derive is told the values are a PostgreSQL enum's
> (`macros.derive.active-enum`) —
> plus `schema_name = "<schema>"` when the described type is
> schema-qualified, so the generated entity round-trips the full type
> identity. Enums are keyed by that identity; two same-named enums under
> different qualifications MUST be refused ("enum `<n>` is used under two
> qualifications; the generated Rust enum can carry only one") rather than
> generated as one colliding Rust enum.
> and `enum_extra_attributes` as further attribute lines. The Rust enum
> name is the UpperCamelCase of the DB enum name; each variant carries
> `#[pgorm(string_value = "<db value>")]`. Variant naming: values starting
> with a digit get an underscore prefix (`3D` → `_3D`); values whose
> UpperCamelCase form is empty (punctuation-only, etc.) fall back to
> per-character encoding — ASCII chars as `U<hex, 4 digits>` and multi-byte
> chars kept verbatim (`/` → `U002F`, `你好` → `你好`) — with a warning
> printed to stdout as `transform` validates the enum; everything else is
> plain UpperCamelCase. A value whose derived variant name is still not a
> legal Rust identifier (a multi-byte character that cannot start one, a
> digit-led value carrying a space) is a `TransformError` per
> `codegen.entity.keywords`, not a panic. Entity files
> using an enum column import it via
> `use super::pgorm_active_enums::<EnumName>;` (deduplicated per file), the
> column a field of the enum's type, from which the derive takes the
> column's (`codegen.entity.types`).

## Disjoint entities

> [spec:pgorm:sem:codegen.entity.disjoint]
> A base table and its kinds' tables, constrained exactly as
> `[spec:pgorm:sem:disjoint.schema]` writes them, MUST generate as one
> disjoint entity, from a live schema and from DDL text alike: the base's
> entity and each kind table's, none with a field for a bookkeeping column,
> and in the base's module a `#[derive(Disjoint)]` enum named for the base
> table in UpperCamelCase. It has one variant per value of the kind type, in
> the type's order, named as `codegen.entity.enums` names an enum's
> variants: a payload variant holding the kind table's `Model` for a value
> that has a table, and a unit variant for one that has none. A variant
> carries `value = "..."` where its value is not its name in snake_case, and
> the enum carries `enum_name` and `schema_name` where the kind type's name
> is not the default the derive would give it. The kind type is not also
> generated into `pgorm_active_enums.rs`, since the derive generates it.
>
> A set missing any part of the shape, or with any part different — a
> foreign key's columns, action or deferrability, a generated column's
> expression or `STORED`, the `UNIQUE` pair — MUST generate as the ordinary
> entities it is, its generated columns as generated columns, with nothing
> dropped, since only the full shape enforces what the enum would claim.

## Identifier hygiene

> [spec:pgorm:sem:codegen.entity.keywords+2]
> Generated identifiers derived from DB names (module names, Model field
> names, table idents) pass through `escape_rust_keyword`: 49 strict/reserved
> Rust keywords are emitted as raw identifiers (`type` → `r#type`,
> `typeof` → `r#typeof`), and the three keywords that cannot be raw
> identifiers — `crate`, `self`, `Self` — get a trailing underscore
> (`crate` → `crate_`, `self` → `self_`). Field names are the snake_case of
> the column name; when that differs from the raw DB name, the DB name is
> preserved via a `column_name` attribute on the Model field.
>
> Keyword escaping is the only rescue on offer. A DB name whose derived form
> is still not a legal Rust identifier — empty after case conversion, all
> digits, or carrying characters an identifier cannot hold (`1`, `-`,
> punctuation-only) — is not mangled into something else: the shared
> `safe_ident` helper rejects it, and every such derivation is put through
> that helper at the `transform` gate, so the failure is
> ``TransformError("<what>: `<derived>` is not a valid Rust identifier")``
> where `<what>` names the table, column, primary key, relation, conjunct
> relation, enum or enum value it came from. The `format_ident!` call sites
> in the writer are downstream of that check and cannot panic on validated
> input.

> [spec:pgorm:req:codegen.entity.collisions+2]
> Every identity the writer derives from a DB name MUST be one name's alone,
> and the `transform` gate refuses a schema in which two names claim one. The
> namespaces checked are: the module name a table derives — and with it the
> file name, which is the same string — and the type name a table derives,
> both across the schema; the field name a column derives, within its table;
> the type name an enum or a range type a schema created derives, across the
> schema and across the two kinds, since both are imported into one entity
> module; and the variant name a value derives, within its enum. Keyword escaping
> (`codegen.entity.keywords`) is applied before the comparison, so the rescue
> cannot itself introduce a collision.
>
> Two tables that share a bare name across schemas — `tenant_a.item` and
> `tenant_b.item`, or `tenant_a.item` and a bare `item`, whose sameness is
> `search_path`'s business and not the generator's — are the case where both
> DB names are right and the collision is the generator's, not the schema's:
> they each need a module, they cannot be told apart by the only name a
> module has, and a run has one output directory to put them in.
> They are refused first, before any table is read, because the schemas that
> tell them apart are exactly what every later lookup by bare name would have
> to guess at (`codegen.entity.transform`) — the refusal is what makes the
> unqualified-reference rule total. The message names both identities and the
> way out:
> ``TransformError("tables `tenant_a.item` and `tenant_b.item` both generate
> the module name `item`: same-named tables in different schemas are different
> tables, and need one generation run each")``. One identity passed twice is a
> different mistake and says so: ``TransformError("table `<table>` is declared
> twice")``, the wording the DDL bridge already uses for it
> (`codegen.ddl.unsupported`).
>
> Every other refusal names both DB names — as identities, so a qualified
> table is named with its schema — and the identifier they share:
> ``TransformError("tables `<first>` and `<second>` both generate the module
> name `<derived>`")``, and in the same shape
> ``tables `<first>` and `<second>` both generate the type name `<derived>` ``,
> ``table `<table>` columns `<first>` and `<second>` both generate the field
> name `<derived>` ``,
> ``enums `<first>` and `<second>` both generate the type name `<derived>` ``
> (``enums and range types `<first>` and `<second>` ...`` when the second is a
> range type), and ``enum `<enum>` values `<first>` and `<second>` both generate the
> variant name `<derived>` ``. `<first>` is whichever name the gate reached
> first, in the order the schema is held in — alphabetical by table name for
> tables and enums, declaration order for columns and enum values. A range
> type a schema created generates one newtype for its bare name, so one bare
> name used under two qualifications is refused before the gate runs:
> ``TransformError("range type `<name>` is used under two qualifications; the
> generated Rust newtype can carry only one")``.
>
> Case conversion is many-to-one — `CakeFilling`, `cake filling` and
> `cake_filling` all derive `cake_filling` — so without this check a schema
> holding two such names generates one file over the other and declares the
> module twice, and the duplicate definition lands in the caller's build
> rather than in ours. This is the same contract as
> `codegen.entity.keywords`, on the other axis: that rule is about a name
> having a Rust form at all, this one about that form being unshared.

## Schema from DDL text

`pgorm_codegen::entities_from_sql` reads a `schema.sql` — DDL text, with no
database to inspect — using libpg_query, the PostgreSQL server's own parser, and bridges the
parse tree into the statements the transformer already consumes. `pg_query` is a
plain dependency of `pgorm-codegen` rather than an optional one: the crate is a
build-time tool nothing links into a running application, so the cost of
compiling the C parser falls on people generating entities and on nobody else.

> [spec:pgorm:def:codegen.ddl+4]
> `parse_schema(&str) -> Result<Vec<TableCreateStatement>, Error>`
> parses DDL text with `pg_query::parse` and returns one statement per
> `CREATE TABLE`, in file order, with every other statement it read folded into
> the table that statement describes.
> `entities_from_sql(&str, EntityWriterOptions) -> Result<WriterOutput, Error>`
> runs the whole pipeline: it builds the `EntityWriterContext` first — so an
> unusable option is reported before the schema is read — then `parse_schema`,
> `EntityTransformer::transform` and `EntityWriter::generate`. Both are
> exported from the crate root, beside the `EntityWriterOptions`,
> `WriterOutput`, `OutputFile` and `Error` they take and return, and the
> crate root is their one path: the bridge's module is private.
>
> Both report failure as `Error::TransformError`, the channel the transform gate
> already uses, and neither panics on any input: text the grammar rejects comes
> back as ``TransformError("schema SQL did not parse: <parser message>")``,
> carrying libpg_query's own message. The bridge validates only what it must to
> build statements; whether the result can be generated at all stays the
> transform gate's decision (`codegen.entity.transform`), so a column type
> `pgorm-query` can spell but `codegen.entity.types.unsupported` cannot render
> is refused there, by name, rather than here.
>
> The bridge is the inverse of the DDL builder: statements built with
> `pgorm-query` and rendered through their `Display`
> (`sql.ddl.create-table`, `sql.ddl.type-enum`) MUST, when parsed back, generate
> the same entity files as the statements themselves — with no asymmetry left
> to document. Both paths hold a key the same way, as the table's
> (`sql.ddl.create-table`), so a key the statement declares and the key the
> bridge reads back from its rendering are one fact.

> [spec:pgorm:req:codegen.ddl.unsupported+17]
> The supported subset is what the entity model can hold: `CREATE TABLE` with
> its columns, `NULL`/`NOT NULL`, primary-key, unique and foreign-key
> constraints, and its `DEFAULT`s and generation expressions within the subset
> `codegen.entity.expressions` reads; `CREATE TYPE ... AS ENUM`; `CREATE TYPE ... AS RANGE`
> and `CREATE DOMAIN` (`codegen.ddl.objects`); `CREATE INDEX`; `CREATE SCHEMA`,
> `IF NOT EXISTS` or not (`codegen.ddl.objects`); and `COMMENT ON TABLE`
> / `COMMENT ON COLUMN` — with a column's `COLLATE` clause and a `NOT NULL`
> constraint's name and `NO INHERIT`, which the bridged statement carries and
> the entity, like a comment, does not (`codegen.ddl.tables`). Everything else in the file MUST be reported — never
> skipped, never quietly reinterpreted. A construct outside the subset is
> ``TransformError("unsupported DDL: <what> at statement <n>")``; a construct
> inside it that this schema cannot resolve is
> ``TransformError("statement <n>: <problem>")``. `<n>` is the statement's
> 1-based position in the text, and `<what>` names the construct the way its
> author wrote it — `ALTER TABLE`, `CREATE TRIGGER`,
> ``a PARTITION BY clause on table `t` ``,
> ``a CHECK constraint on column `t`.`c` `` — with `an unrecognised statement` as
> the fallback for a statement kind the namer does not know.
>
> Named rejections MUST cover at least: every statement other than the seven
> above; a statement written inside a `CREATE SCHEMA` as one of its elements,
> named by its kind (``CREATE TABLE inside CREATE SCHEMA `s` ``); a range type option PostgreSQL does not have, a `SUBTYPE` that is not
> a type name or is a `serial` spelling, and a range type with no `SUBTYPE`
> (unresolved, as PostgreSQL refuses it with `42601`); on a domain, a base
> that is a `serial` spelling or names no type the file declared or the
> vocabulary holds, a catalog-qualified domain name, and every constraint
> PostgreSQL refuses there — ``a UNIQUE constraint on domain `d` ``, `a
> PRIMARY KEY`, `a REFERENCES clause`, `a GENERATED clause`, `an identity
> clause`, `a NO INHERIT constraint`, `a deferrable constraint`, `a NOT
> ENFORCED constraint` and `an ENFORCED clause`, each `on domain <d>`; `INHERITS`, `PARTITION BY`, `PARTITION OF`, `OF <type>`, `LIKE`, `WITH`
> storage options, `TABLESPACE`, `USING <access method>`, `ON COMMIT`,
> catalog-qualified table and collation names and temporary or unlogged
> tables; column
> `CHECK`,
> `STORAGE` and `COMPRESSION` clauses, and an identity's sequence options; table-level `CHECK`
> and `EXCLUDE` constraints, deferrable keys (a foreign key's deferrability
> is read, `codegen.ddl.tables`), `NO INHERIT` on any
> constraint but a `NOT NULL`, `INCLUDE`
> columns, constraint index and storage options, and `MATCH` clauses;
> `REFERENCES` without a referenced column list, which no catalog is present to
> resolve; index `WHERE`, `INCLUDE`, `CONCURRENTLY`, `COLLATE`, operator
> classes, `NULLS FIRST`/`NULLS LAST`, expression columns, tablespaces and
> storage options, and on a unique index a `DESC` column or an access method
> other than `btree`, which the table constraint it folds into cannot spell
> (`codegen.ddl.objects`); and `COMMENT ON` any object other than a table or
> a column.
> Type spellings outside the vocabulary are named the same way
> (`codegen.ddl.types`).
>
> A column's `DEFAULT` and generation expression are read
> (`codegen.ddl.tables`), but only within the subset of PostgreSQL's
> expressions `codegen.entity.expressions` holds; a construct outside it is
> named with the expression it sits in, ``TransformError("unsupported DDL:
> <construct> in the DEFAULT of column `<t>`.`<c>` at statement <n>")`` or
> ``... in the generation expression of column `<t>`.`<c>` ...``, and never read
> as something near it. The constructs named are a subquery, `CASE` and every
> other expression kind outside the subset (`an expression codegen does not
> read`); a cast to a type with a modifier, `SETOF`, `%TYPE` or a sized or
> multi-dimensional array bound (`a cast to a modified type`); a
> schema-qualified function other than `pg_catalog`'s (`a schema-qualified
> function`); an aggregate, window or `VARIADIC` call; a function written in
> the grammar's own syntax — `substring(s from 2)`, `extract(..)`, `trim(..)`
> — (`a function written in SQL syntax`); a function the parse tree names
> `coalesce`, `greatest`, `least` or `nullif`, which only a quoted name
> reaches and which is some other function than the grammar's construct; an
> operator outside the subset, a prefix operator and `IN`, `LIKE`, `BETWEEN`
> and the other operator forms (`an operator codegen does not read`); a float
> with an exponent, an integer past `bigint` or a numeric wider than 18 digits,
> and a bit-string (`a literal`); a qualified column reference; and a SQL value
> function other than `CURRENT_DATE`, `CURRENT_TIME` and `CURRENT_TIMESTAMP`.
>
> The file is read with PostgreSQL 18's grammar (`codegen.ddl`), which
> accepts constraint shapes earlier releases refused. Each resembles a shape
> the bridge does read, so reading it as that shape would be the quiet
> reinterpretation this rule forbids, and each is named instead until the
> entity model can hold it: a
> unique key ending `WITHOUT OVERLAPS` (`a WITHOUT OVERLAPS unique key`), which
> is not the plain key over the same columns and has two columns at least,
> where an entity's unique columns are single columns. The bridged statement
> could carry it (`sql.ddl.create-table`) and the entity cannot. A temporal
> primary key and a `PERIOD` foreign key the entity holds, its key ending
> `without_overlaps` and its relation a period pair (`entity.traits.primary-key`,
> `entity.relation.def`), and the bridge reads both (`codegen.ddl.tables`); a
> `PERIOD` on one side of a foreign key alone (`a PERIOD on one side of a
> foreign key`, `42830`) and a temporal key over its period alone (`a WITHOUT
> OVERLAPS key over its period alone`, `42601`) stay named, as the server
> refuses them. A
> `NOT ENFORCED` foreign key the entity does hold, since its relation says so
> (`entity.relation.def`), and the bridge reads it (`codegen.ddl.tables`); a
> `NOT ENFORCED` anywhere else stays named (`a NOT ENFORCED constraint`), where
> PostgreSQL refuses it too. An explicit `ENFORCED` is
> refused (`an ENFORCED clause`) everywhere PostgreSQL refuses it — after a
> column's `NOT NULL`, key or `DEFAULT` — and read on a column's
> `REFERENCES`, where it states the default (`codegen.ddl.tables`). 18's `NOT
> NULL` constraint is read, not named: its name and `NO INHERIT`, from a
> column's clause or a table-level `NOT NULL c`, are the column's
> (`codegen.ddl.tables`).
>
> Unresolved references are named as well: an index or comment naming a table
> the file never creates, a column comment naming a column its table does not
> have, and a table or a type declared twice — each is keyed by name
> downstream, so a duplicate would otherwise overwrite in silence. So is a
> schema created a second time without `IF NOT EXISTS`, which PostgreSQL
> refuses (`42P06`, *schema already exists*), as ``TransformError("statement
> <n>: schema `<s>` is declared twice")``. So is a table
> declaring more than one primary key, in each spelling PostgreSQL refuses
> (`42P16`, *multiple primary keys for table are not allowed*): `PRIMARY KEY` on
> two columns, on a column beside a table-level `PRIMARY KEY (...)`, as two table
> constraints, or twice on one column. It is
> ``TransformError("statement <n>: table `<t>` declares more than one primary
> key")``. None of these is the composite key one `PRIMARY KEY (a, b)` declares,
> and reading two of them as one — which the transform did, generating either a
> composite key or a `PrimaryKey` enum with a variant twice over that did not
> compile — would be the quiet reinterpretation this rule forbids. So is a
> primary or unique key naming one column twice, which PostgreSQL refuses too
> (`42701`, *column "a" appears twice in primary key constraint*):
> ``TransformError("statement <n>: table `<t>` names column `<c>` twice in one
> key")``. Read as written it would generate that same enum, and a unique key
> over `(a, a)` would make `a` a unique column on its own; read without the
> repeat it would be a key other than the one written. A unique index naming
> a column twice the server does create, but the table constraint it folds
> into cannot (`codegen.ddl.objects`), so it is named as a `DESC` column on one
> is: ``TransformError("unsupported DDL: column `<c>` named twice on unique
> <index> at statement <n>")``. Columns are compared as the text the server
> reads. So are two
> `NOT NULL` clauses on one column that PostgreSQL refuses to make one
> constraint of (`42601`), between which a bridge keeping either would choose
> in silence: two names, ``TransformError("statement <n>: column `<t>`.`<c>`
> names its NOT NULL constraint both `<a>` and `<b>`")``, or a disagreement on
> `NO INHERIT`, ``TransformError("statement <n>: column `<t>`.`<c>` declares
> NOT NULL both with and without NO INHERIT")``; and so is a table-level `NOT
> NULL` naming a column its table does not have, as a column comment naming
> one is. So is a column's `REFERENCES` followed by one attribute clause twice
> — `DEFERRABLE` or `NOT DEFERRABLE`, `INITIALLY`, `ENFORCED` or `NOT ENFORCED`
> — which PostgreSQL refuses (`42601`), ``TransformError("statement <n>:
> column `<t>`.`<c>` gives its foreign key <clause> twice")``, and one followed
> by `INITIALLY DEFERRED` after `NOT DEFERRABLE` (`42601`),
> ``TransformError("statement <n>: INITIALLY DEFERRED on a NOT DEFERRABLE key
> on column `<t>`.`<c>`")``. So is a column filled two ways — two of a serial
> type, an identity, a `DEFAULT` and a generation expression, or one of them
> twice — which PostgreSQL refuses (`42601`, *multiple default values
> specified*, *both default and identity specified*, *both default and
> generation expression specified*, *multiple generation clauses specified*,
> *multiple identity specifications*): ``TransformError("statement <n>: column
> `<t>`.`<c>` is filled by <first> and by <second>; a column takes one")``,
> each named as `a serial type`, `an identity`, `a DEFAULT` or `a generation
> expression`. A bridge keeping either would generate an entity whose schema
> fills the column otherwise than the file; the derive refuses most of the
> pairs besides (`macros.derive.entity-model.column-def`). And so is a
> `DEFAULT` reading a column, which PostgreSQL refuses (`0A000`, *cannot use
> column reference in DEFAULT expression*): ``TransformError("statement <n>:
> the DEFAULT of column `<t>`.`<c>` reads column `<x>`; a DEFAULT reads no
> column")``. A foreign
> key naming a table the file never creates, or a column that table does not
> have, is refused too, but by the transform gate `entities_from_sql` runs
> (`codegen.entity.transform`) rather than here: the bridge resolves one
> statement against the file, and the gate holds every table at once, which is
> where the answer to a foreign key is known.
>
> One construct is accepted without being carried, and only this one: a
> non-unique `CREATE INDEX`. It states no fact the entity model holds —
> `codegen.entity.transform` reads unique and primary-key indexes and nothing
> else — and a `CREATE TABLE` embeds only unique and primary-key constraints
> (`sql.ddl.create-table`), so there is no constraint a non-unique index could
> be carried as. Its table must still exist.

> [spec:pgorm:sem:codegen.ddl.types+9]
> Column types map back through the `ColumnType` → Postgres spelling contract of
> `sql.ddl.column-types`, read over the names the grammar produces: keyword
> spellings arrive qualified as `pg_catalog.<name>`, everything else bare or
> schema-qualified. An enum reference resolves by its exact identity —
> `(schema, name)`, `None` for unqualified — and MUST NOT fall back to a
> same-named enum or range type under a different qualification: the
> mismatch is reported as unresolved, naming the reference as spelled and the
> kind of the type it is not. `bpchar` → `Char`, `varchar` → `String`,
> `text` → `Text`, `int2`/`int4`/`int8` →
> `SmallInteger`/`Integer`/`BigInteger`, `float4`/`float8` → `Float`/`Double`,
> `numeric` → `Decimal`, `timestamp` → `Timestamp`,
> `timestamptz` → `TimestampWithTimeZone`,
> `time` → `Time`, `date` → `Date`, `interval` → `Interval` (below),
> `bool` → `Boolean`, `money` → `Money`, `bytea` → `Bytea`, `bit` → `Bit`,
> `varbit` → `VarBit`, `json` → `Json`, `jsonb` → `JsonBinary`, `uuid` → `Uuid`,
> `inet`/`cidr`/`macaddr`/`ltree` → `Inet`/`Cidr`/`MacAddr`/`LTree`,
> `vector` → `Vector`, the six built-in range types `int4range`,
> `int8range`, `numrange`, `daterange`, `tsrange` and `tstzrange` →
> `Range(Int4)` through `Range(TimestampTz)`, and their multiranges
> `int4multirange` through `tstzmultirange` → `Multirange(..)`. A name the
> file declared as a range type (`codegen.ddl.objects`) resolves to
> `ColumnType::CreatedRange` carrying its identity and its subtype's column
> type, exactly as an enum resolves — by full identity, with no fallback to a
> same-named range type under another qualification — and a name declared
> as the multirange created beside one resolves to
> `ColumnType::CreatedMultirange` the same way. A name the file declared as a
> domain resolves to `ColumnType::Domain` carrying its identity and its base's
> column type, by full identity in the same way, the base read when the domain
> was, so a domain over a domain carries it whole. `serial`, `bigserial` and `smallserial` (and
> `serial4`/`serial8`/`serial2`) are `Integer`/`BigInteger`/`SmallInteger` plus
> the auto-increment fact the renderer spells as the serial family. A name the
> file declared as an enum type resolves to `ColumnType::Enum` carrying that
> type's values, and an unqualified name is looked up as an enum before the
> table above. A type argument the vocabulary can hold is kept
> (`varchar(255)`, `numeric(10, 2)`, `bit(8)`, `vector(3)`), a single-argument
> `numeric(p)` reading as `Decimal(Some((p, 0)))` — its own meaning. An array
> bound wraps the element type in `Array`; only one unsized `[]` is accepted.
>
> An `interval` keeps its field qualifier and precision, which the grammar
> writes as the type's modifiers — a field mask, the `INTERVAL_MASK` bits of
> the fields the qualifier spans or every bit (`0x7FFF`) for none, then the
> precision when one was given — as the `IntervalSpec` that renders them:
> `interval` → `Any(None)`, `interval(3)` → `Any(Some(P3))`,
> `interval YEAR TO MONTH` → `Fields(YearToMonth)`,
> `interval DAY TO SECOND(3)` → `Fields(DayToSecond(Some(P3)))`, through an
> array as on its own. The mask is read as the server's `intervaltypmodin`
> reads it, so the generic spelling `pg_catalog.interval(6)` is
> `YEAR TO MONTH`, as the server takes it to be. A precision past 6, which
> the server would quietly reduce to 6, is named
> (``an `interval` precision of <p>, above the 6 PostgreSQL keeps, on
> column `<t>`.`<c>` ``), and so is a modifier no `IntervalSpec` holds — a
> precision on a field other than `SECOND`, which only the generic spelling
> reaches, or a mask naming no qualifier — as `` `interval` with a type
> modifier``.
>
> The map is close to a bijection because the forward contract
> (`[spec:pgorm:req:sql.ddl.column-types]`) no longer spells one Postgres
> type under several names: `bytea`, `timestamp`, `smallint` and `money` each
> have exactly one `ColumnType` to come back to, so `Bytea`, `Timestamp`,
> `SmallInteger` and `Money` are recovered rather than chosen from a set. Where
> the contract is still many-to-one the reverse takes the faithful branch and
> the collapse is stated rather than hidden: `varchar` reads as
> `String(StringLen::None)`, since `StringLen::Max` renders the same bare
> `varchar` and Postgres has no `varchar(max)` to have written it. Every other
> `ColumnType` in the vocabulary has a reverse. `ColumnType::Named`
> is deliberately not produced — a domain column is `Domain`, which says what
> its values are, where `Named` would say only what DDL writes — and a type
> name outside the table above is an error,
> not a `String` column that quietly means something else. `varbit` without a
> length, a modifier the vocabulary cannot hold (`timestamp(3)`), a sized or
> multi-dimensional array, and a non-integer type modifier are all named
> rejections per `codegen.ddl.unsupported`.

> [spec:pgorm:sem:codegen.ddl.tables+11]
> A `CREATE TABLE` becomes a `TableCreateStatement` carrying the `TableName`
> its name spells — `Table`, or `SchemaTable` when it is schema-qualified;
> a catalog-qualified `db.schema.table` names a cross-database reference
> Postgres does not implement and `TableName` cannot hold, so it is a named
> rejection rather than a name quietly shortened to its last two parts. The
> statement also carries its `IF NOT EXISTS` flag and one `ColumnDef` per
> column definition, in declaration order. Column constraints set the matching
> `ColumnSpec` — `NOT NULL`, `NULL` — and a column-level `REFERENCES` becomes a
> foreign key on that one column. A key is the table's
> (`sql.ddl.create-table`), so a column-level `PRIMARY KEY` becomes the
> table's primary key over that one column, and a column-level `UNIQUE` a
> one-column unique key: the keys Postgres itself creates for them, and so the
> truthful forms to bridge them as. A second primary key in any spelling is
> refused by name (`codegen.ddl.unsupported`) before it reaches the statement,
> whose one primary-key slot would otherwise keep the later key and drop the
> earlier in silence. A primary-key column is `NOT NULL`
> whether or not the DDL spells it, which is Postgres' own rule: the entity
> model reads nullability off the column alone, so an unstated `NOT NULL` would
> otherwise generate an `Option` primary key.
>
> A column's `GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY` becomes the
> column's identity of that form (`ColumnDef::identity` /
> `identity_by_default`), and the column `NOT NULL` with it, which is again
> Postgres' own rule. Its sequence options — `(START WITH 10)` and the rest —
> are a named rejection (`codegen.ddl.unsupported`): the entity a schema
> generates declares the identity's form and nothing more
> (`codegen.entity.transform`).
>
> A column's `DEFAULT` becomes the column's default (`ColumnDef::default`)
> and its `GENERATED ALWAYS AS (..)` the column's generation expression
> (`ColumnDef::generated`), `STORED` or `VIRTUAL` as the grammar reads it — a
> bare `GENERATED ALWAYS AS (..)` is `VIRTUAL` in 18, so the grammar's kind is
> the answer, never the keyword's presence. Each expression is read from the
> parse tree into the subset `codegen.entity.expressions` holds and lowered to
> the `SimpleExpr` the statement carries, which renders an expression the
> server stores as it stores the original: the live suite holds the catalog
> (`pg_get_expr`, `attgenerated`) of a schema run as written equal to that of
> the schema its generated entities create, every construct of the subset and
> every operator nesting whose precedence matters among them. A `DEFAULT NULL`
> is read as the `NULL` it says; the server stores no default for it either
> way. A column takes one of a serial type, an identity, a `DEFAULT` and a
> generation expression, once, and a `DEFAULT` reads no column, as the server
> holds (`codegen.ddl.unsupported`).
>
> A column's `COLLATE` clause becomes the column's collation
> (`ColumnDef::collate`, `[spec:pgorm:req:sql.ddl.column-def]`), bare or
> schema-qualified as written; a catalog-qualified name is a named rejection,
> as a table's is. It rides on the statement and does not reach the generated
> entity, as a column comment does not (`codegen.ddl.objects`): the entity
> model has no collation, and needs none to query the column, because the
> server compares and sorts a column by its own collation without the
> statement naming it. The one consumer that would read it off an entity is
> entity-first schema generation, and a generated entity is the image of a
> schema that already exists. Emitting it would mean an entity attribute and
> a schema-generation arm of its own, and until a caller needs to declare a
> collation from an entity, `parse_schema`'s statements are where it is kept.
>
> A column's `NOT NULL` is PostgreSQL 18's catalog constraint, and its name
> and `NO INHERIT` become the column's (`ColumnDef::not_null_named`,
> `not_null_no_inherit`, `[spec:pgorm:req:sql.ddl.column-def]`), whether
> the column's clause declares them or a table-level `[CONSTRAINT n] NOT NULL
> c [NO INHERIT]` does: the server creates the same constraint from either,
> and `sql.ddl.column-def` gives the column the one spelling. They ride on the
> statement and stop there, as a collation does: the entity reads only that
> the column refuses nulls, which is all a query of it needs, and a generated
> entity is the image of a schema that already exists. The bridge makes one
> constraint of a column's `NOT NULL` clauses as the server does — the name
> any of them gives is kept, and a primary key's or identity's implied `NOT
> NULL` takes the one declared — and refuses the pairs the server refuses
> (`codegen.ddl.unsupported`). A table-level `NOT VALID` is dropped, being a
> clause the server ignores in `CREATE TABLE`: the live suite shows such a
> constraint created valid, there being no rows to leave unchecked. And a
> column declared `NULL` that a table-level `NOT NULL` names refuses nulls,
> as the server creates it: the `NULL` states no constraint, only the
> default.
>
> Table-level `PRIMARY KEY` and `UNIQUE` constraints become the table's
> primary key and unique keys (`TableKey`), keeping the constraint name and,
> on a unique key, `NULLS NOT DISTINCT`; a table-level `FOREIGN KEY` becomes a foreign key with
> its columns, referenced table and referenced columns, and both forms keep the
> constraint name. A foreign key whose two column lists differ in length is a
> named rejection rather than a truncated key — the pairs are what the bridged
> statement is built from (`[spec:pgorm:req:sql.ddl.foreign-key]`), and
> Postgres itself rejects the mismatch at parse analysis. An explicit
> `ENFORCED` on a column's `REFERENCES` becomes the foreign key's
> `Enforcement::Enforced`, riding on the statement as said; the entity is the
> plain key's, which is what it is. At table level the parse tree marks every
> foreign key enforced whether or not the clause is written, so there is
> nothing to tell apart and nothing is carried.
> Referential actions map `RESTRICT`, `CASCADE`, `SET NULL` and
> `SET DEFAULT` onto `ForeignKeyAction`. `NO ACTION` reads as no action
> declared: the grammar fills that same code in when a foreign key declares
> nothing, the two are indistinguishable in the parse tree, and it is
> Postgres' default — so the generated relation carries an `on_update` or
> `on_delete` exactly where the schema chose something other than the default.
>
> A foreign key's deferrability and `NOT ENFORCED` are its relation's
> (`entity.relation.def`), and the bridge reads both, on a column's
> `REFERENCES` or a table-level `FOREIGN KEY`. A table-level key carries them on
> its own node, `INITIALLY DEFERRED` having made it `DEFERRABLE` already; a
> column's arrive as attribute clauses after the `REFERENCES`, which qualify
> it in any order, `INITIALLY DEFERRED` implying `DEFERRABLE` as the server
> reads it: `DEFERRABLE` is `DeferrableInitiallyImmediate`, `INITIALLY
> DEFERRED` `DeferrableInitiallyDeferred`, `NOT DEFERRABLE` `NotDeferrable`
> and `NOT ENFORCED` `Enforcement::NotEnforced`, each riding on the
> statement as said. Attribute clauses after anything else — a key, a `NOT
> NULL` — stay named (`codegen.ddl.unsupported`).
>
> A primary key ending `WITHOUT OVERLAPS` becomes the table's temporal key,
> its period held apart from its other columns (`TableKey::without_overlaps`),
> and a table-level `FOREIGN KEY (.., PERIOD p) REFERENCES t (.., PERIOD q)`
> a foreign key whose last pair is its period (`period(p, q)`), the grammar
> putting `PERIOD` on the last column of each list and nowhere else.

> [spec:pgorm:sem:codegen.ddl.objects+10]
> Statements are resolved against each other rather than in file order: a
> `CREATE TYPE ... AS ENUM` may follow the table whose column names it, and a
> `CREATE INDEX` or `COMMENT ON` may precede its table. An enum type contributes
> its full identity — schema and name — and values to every column typed with
> it (`codegen.ddl.types`), which
> is where `transform` discovers enums; an enum type no column names contributes
> nothing and is returned as no statement of its own.
>
> A `CREATE TYPE ... AS RANGE` contributes its full identity and its
> `SUBTYPE`, read as a column's type is and so against the types declared
> before it — PostgreSQL requires the subtype to exist first, so an enum may
> be a range's subtype. Its other options, `SUBTYPE_OPCLASS`, `COLLATION`,
> `CANONICAL`, `SUBTYPE_DIFF` and `MULTIRANGE_TYPE_NAME`, are read and set
> aside: they say how the server orders, normalises and measures the range,
> none of which changes the value a row holds. It also declares the
> multirange PostgreSQL creates beside the range, by the same subtype:
> `MULTIRANGE_TYPE_NAME` as written — unqualified, the server creates it
> where an unqualified name is created, not beside the range, so its
> identity is unqualified too — or, without it, the name PostgreSQL derives
> in the range's schema: the first `range` in the range's name, compared
> case-sensitively, becomes `multirange`, or, when there is none,
> `_multirange` follows the name cut to 52 bytes, and the result is cut to
> 63 (`floatrange` → `floatmultirange`, `slot` → `slot_multirange`,
> `"FloatRange"` → `"FloatRange_multirange"`; live on 18.6). A range or
> multirange type is where `transform` discovers the newtypes it writes
> (`codegen.entity.types`). Enums, ranges, multiranges and domains share one
> namespace, so `is declared twice` holds across the four kinds.
>
> A `CREATE DOMAIN` contributes its full identity and its base type, read as a
> column's type is and so against the types declared before it, as
> PostgreSQL requires: an enum, a range type or another domain may be a
> domain's base, and an array or a sized type may be. Its `COLLATE`,
> `DEFAULT`, `NOT NULL` or `NULL` and its `CHECK`s, named or not, are read
> and set aside: the server holds every column of the domain to them, none
> changes what a value of the domain is, and the entity names the domain,
> which carries them. A domain's `NOT NULL` does not make a nullable column
> of it a non-null field: the column reads as `NULL` wherever an outer join
> or an absent row makes it one, and an insert leaving it out takes the
> domain's default, so the field's nullability is the column's own.
>
> A `CREATE SCHEMA` contributes no statement and no type: a table or a type in
> the schema carries the schema as its qualifier, which is what the entity
> generated for it names (`codegen.entity.context`). Its `AUTHORIZATION` role
> is read and set aside, ownership being a privilege rather than a fact about
> any value an entity reads or writes. The schema's name is read for one check
> alone: a second `CREATE SCHEMA` of a name, unless it says `IF NOT EXISTS`, is
> refused, as the server refuses it (`codegen.ddl.unsupported`). `CREATE SCHEMA
> AUTHORIZATION <role>`, naming no schema, creates the one named for its role,
> and a role keyword — `CURRENT_USER`, `CURRENT_ROLE`, `SESSION_USER` — is
> answered by the session the file runs in, so such a schema is read with no
> name and takes no part in the check. A qualified name elsewhere in the
> file is not held to the schemas it creates, so a dump that creates its
> schemas and one that does not are read alike: `public` exists without being
> created, and a dump of some schemas names types and tables in others. A
> schema element — `CREATE SCHEMA s CREATE TABLE t (..)` — is named, not read
> (`codegen.ddl.unsupported`): the server qualifies an element's unqualified
> names with the schema as it runs it, which the parse tree does not record,
> so the element read as written would be a table outside its schema.
>
> A unique `CREATE INDEX` is folded into its table as the unique key that
> enforces the same uniqueness — a `TableKey<Unique>`
> (`sql.ddl.create-table`) — keeping its name, its columns and
> `NULLS NOT DISTINCT`; `codegen.entity.transform` then reads a single-column
> unique key as that column's `unique` flag. A table constraint's key is
> plain column names under the default `btree`, so what a unique index can
> say and a constraint cannot is a named rejection per
> `codegen.ddl.unsupported` rather than dropped: a `DESC` key column, and an
> access method other than `btree`. An explicit `ASC` is the default order
> and folds away with nothing lost. `IF NOT EXISTS` describes the statement
> that creates the index rather than the index, and that statement is what
> the folding replaces, so it is read and not carried. `COMMENT ON TABLE`
> becomes the statement's comment and `COMMENT ON COLUMN` a
> `ColumnSpec::Comment` on the named column. Neither
> comment reaches the generated entities, which have no comment surface — they
> ride on the statements so a caller reading `parse_schema`'s output still has
> them.
>
> Tables are held by identity — schema and bare name, the same key the
> transformer holds them under (`codegen.entity.transform`) — and an index or
> comment attaches by resolving its own target against them, by that rule: a
> qualified target names exactly that table, an unqualified one the table
> written without a qualifier, failing that the one table with that bare
> name. So a schema-qualified `CREATE INDEX` or `COMMENT ON` reaches its own
> table rather than whichever table happened to share the bare name, and a
> `CREATE INDEX` a qualifier makes generated for a table the file never
> creates is ``unresolved("no CREATE TABLE for table `tenant_b.item`")``
> rather than a fact quietly folded into `tenant_a.item`. The unqualified
> case is the only one that can be answered by more than one table, and it is
> named rather than guessed at:
> ``TransformError("statement <n>: `item` names more than one table")``. Two
> tables with one bare name cannot be generated at all
> (`codegen.entity.collisions`), so this is reachable only from
> `parse_schema` read on its own. `is declared twice` likewise compares
> identities: two `CREATE TABLE`s for one bare name in different schemas are
> two tables, and the file is read.
