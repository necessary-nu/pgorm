# pgorm derive macros

This spec covers the procedural macro suite: the `pgorm-macros` crate (entity/model
derives, active enums, relations, partial models, the iteration those derives write
for their enums, and the `#[pgorm_macros::test]` harness attribute), the two pgorm-query proc-macro crates
(`pgorm-query-derive` for `SqlName`/`StaticName`, `pgorm-query-attr` for `#[enum_def]`)
with `pgorm-query-derive-internals`, the library the name derives share with
pgorm-macros', and `pgorm-sql-macro` (the compile-time-checked `sql!` and `prql!` literals). Rules are
maintenance-scope: they describe what the macros generate and reject today, including
known limitations.

## The macro suite

> [spec:pgorm:def:macros.derive+7]
> The `pgorm-macros` crate exposes the ORM's derive macros, all gated behind the crate's
> `derive` feature: `Entity`, `EntityTrait`, `Model`, `Column`, `Relation`,
> `ActiveEnum`, `Disjoint`, `IntoActiveModel`, `PartialModel`, `FromQueryResult`,
> `ValueType`, `JsonValueType`, `CreatedRange`, `MigrationName`, `SqlName` and
> `StaticName`, named as `macros.derive.names` says, plus the
> `#[pgorm_macros::test]` attribute macro.
> There is no iteration derive: the derives that need their enums iterable write it
> (`macros.derive.enum-iter`). Every entity-side derive reads its configuration from
> the shared `#[pgorm(...)]` helper attribute (`SqlName` and `StaticName` read
> pgorm-query's `#[iden]` and `#[method]`).
>
> There is no Seaography support: the `seaography` cargo feature, the
> `DeriveRelatedEntity` derive it gated, and the codegen option that emitted a
> `RelatedEntity` enum are all deleted rather than kept as a shim. The gated expansion
> named `seaography::RelationBuilder` and `async_graphql` paths no crate in the
> dependency graph provided, and the real Seaography targets sea-orm's `EntityTrait`, so
> no crate could have satisfied it; with the feature off the derive expanded to nothing,
> which made it a no-op that still broke the workspace feature matrix when enabled.
>
> One auxiliary derive is defined in the same crate and re-exported by pgorm-migration
> beside the trait it implements: `MigrationName` implements
> `pgorm_migration::MigrationName` by taking the file stem of `file!()`. pgorm does not
> re-export it.
>
> Every expansion emits call-site-relative `pgorm::...` paths, never `::pgorm::...`, and
> there is no `#[pgorm(crate = ...)]` override or `proc-macro-crate` lookup. The name
> `pgorm` MUST therefore resolve where the derive is written: renaming the dependency in
> `Cargo.toml` is a loud `E0433`, fixed by aliasing the name back
> (`use my_orm as pgorm;`) in the module holding the entities or at the crate root. The
> same relative spelling is why pgorm's own fixtures can write `use crate as pgorm;`.
> Renaming `pgorm-query` needs no alias, since the expansions reach it through the
> `pgorm::pgorm_query` re-export; `pgorm-migration`, which the `MigrationName` derive
> names directly, follows the same rule as `pgorm`.
>
> Separately, `pgorm-query-derive` provides `SqlName` and `StaticName` derives using the
> `iden` and `method` helper attributes, whose expansion names `pgorm_query::...`; its
> derives and pgorm-macros' of the same names, which name `pgorm::pgorm_query::...`, are
> one implementation, the ordinary library `pgorm-query-derive-internals`, each crate
> passing it the path it reaches the traits by (`macros.derive.iden`). A proc-macro
> crate exports only its macros, which is why the shared code is a library of its own.
> `pgorm-query-attr` provides the `#[enum_def]` attribute macro.

## Form, names and attribute values

> [spec:pgorm:req:macros.derive.one-form]
> An entity MUST have one written form: the composite derive on its `Model`
> struct and the relation derive on its `Relation` enum, configured by
> attributes. There is no hand-assembled form, so the derives that existed
> only to build one — the primary-key, custom-column and active-model
> derives — are deleted, and codegen writes only this form. What a
> hand-written trait method could say and no attribute did, an attribute
> MUST now say: `column_type` takes any expression of type `ColumnType`, a
> helper's call included; `select_with` and `save_with` name a function
> from the column's expression to the one read or written, beside the
> `select_as` and `save_as` casts; and `on_condition` takes a closure of the
> two tables as the join binds them, as well as an expression that ignores
> them. One case keeps a hand-written part: an entity whose table or schema
> name is computed at run time declares its own `Entity` struct, implements
> `EntityName` on it, and puts the entity-trait derive on it beside a
> composite derive given no `table_name`.

> [spec:pgorm:req:macros.derive.names+2]
> Every derive MUST be named for what it makes or implements, as Rust's own
> derives and serde's are, with no `Derive` prefix; a derive and a trait
> live in different namespaces, so the two can share a name. The derives
> are `Entity` (the composite derive, on a `Model` struct), `EntityTrait`
> (on a hand-written `Entity` struct whose names are computed), `Model` (a
> further model of an entity), `Column` (a column enum for a custom
> select), `Relation`, `ActiveEnum`, `Disjoint` (an enum stored as a base
> table and its kinds' tables, `[spec:pgorm:def:disjoint.declare]`), `OneOf`
> (a reference to one of several entities,
> `[spec:pgorm:def:disjoint.one-of]`), `IntoActiveModel`, `PartialModel`, `FromQueryResult`, `ValueType`,
> `JsonValueType` (a value stored as a JSON column), `CreatedRange`,
> `MigrationName` (in pgorm-migration), and
> pgorm-query's `SqlName` and `StaticName`, reachable from pgorm with no
> dependency on pgorm-query. The `Derive`-prefixed names, and
> `FromJsonQueryResult`, MUST NOT exist, as aliases or otherwise; the Coming
> from SeaORM guide maps each to its replacement.
>
> No two macros a pgorm prelude exports may share a name, and none may take
> a name a widely used derive crate exports: the display derive is folded
> into `ActiveEnum` as its opt-in `display` key rather than named `Display`
> beside derive_more's and strum's, and pgorm-macros' own name derive is
> folded into pgorm-query's `SqlName`, which already does more.

> [spec:pgorm:req:macros.derive.code-values+4]
> An attribute value that is Rust code MUST be written as Rust code — a path
> as a path, a type as a type, an expression as an expression — and is
> taken as written, with no implicit prefix: `belongs_to =
> super::cake::Entity`, `from = Column::CakeId`, `rs_type = String`,
> `db_type = ColumnType::Integer`, `on_delete = ForeignKeyAction::Cascade`,
> `deferrability = Deferrability::DeferrableInitiallyImmediate`,
> `condition_type = ConditionType::Any`, `default_value = -1`. That covers
> the relation targets and column pairs, the periods, `on_condition`,
> `column_type`, `db_type`, `array_type`, `rs_type`, `default_expr`,
> `default_value`, the generated expressions, `from_expr`, `from_col`, the
> foreign-key actions, `enforcement`, `deferrability`, `condition_type`,
> the `columns` of an entity's `unique(..)` (`columns = (Column::A, Column::B)`),
> `PartialModel`'s `entity` and `IntoActiveModel`'s `active_model`. A value that is SQL or a name stays a string —
> `table_name`, `schema_name`, `column_name`, `fk_name`, a `unique` key's
> `name`, an `ActiveEnum`'s
> `enum_name`, `string_value`, `display_value`, `range_name`,
> `multirange_name`, `iden`, `comment`, and the `select_as` and `save_as`
> cast types — as do the case styles of `rename` and `rename_all`, which
> are not Rust. A string where code is expected MUST be a compile error
> spanned on the string, saying to remove the quotes. `default_value` is an
> expression of the column's value, which a string literal is, so
> `default_value = "untitled"` is the string it spells. On a model field,
> the key naming the field's `Column` variant is `variant`, so `enum_name`
> names a SQL type wherever it appears. The names the values take for
> granted — `ColumnType`, `ForeignKeyAction`, `Deferrability`,
> `Enforcement`, `ConditionType` and `ArrayType` — are members of the entity
> prelude (`[spec:pgorm:def:entity.prelude]`).

> [spec:pgorm:req:macros.derive.hygiene]
> A derive MUST expand on a declaration a `macro_rules!` macro writes as it
> does on one written out, whichever of its tokens the macro's caller passes
> in and whichever the macro writes itself. Such an input carries spans of
> two hygiene contexts, and a local is resolved only within the context of
> its own span, so a parameter or local a derive introduces MUST carry the
> same span at its binding and at every use: the derive's call site
> wherever the two would otherwise differ, as they do between a binding the
> derive writes and a use inside an expression it spans at an input token
> (a variant's payload, a target entity, a column path) so that a type
> error points at what the program wrote. A disjoint enum whose variants the
> caller passes, and a one-of enum whose targets the caller passes and whose
> column paths the macro completes, compile and declare what their written
> out forms declare.

> [spec:pgorm:sem:macros.derive.boilerplate]
> A derive MUST write what follows from what it was told, so an entity's
> module holds only what the program decides:
>
> - `Entity` implements `ActiveModelBehavior` for the `ActiveModel`, with no
>   hooks, unless the model says `#[pgorm(hooks)]`, in which case the
>   program implements it (`[spec:pgorm:req:entity.active-model.hooks]`).
>   The derive that wrote only that empty implementation is deleted.
> - `Relation` implements `Related<F>` for each entity `F` exactly one of
>   its variants reaches, as that variant's definition, unless the variant
>   says `related = false`. An entity two variants reach gets none, since
>   only the program knows which it means, and neither does a path through
>   a junction (`Related::via`), which the program writes.
> - `Relation`, `Column` and `ActiveEnum` make their types iterable
>   themselves, so no program writes `EnumIter`, and no prelude exports it.
> - `ActiveEnum` with neither `rs_type` nor `db_type` is a PostgreSQL enum
>   of strings: each value is its variant's `string_value`, or else the
>   variant's name in the `rename_all` style, snake_case by default, and the
>   type is named by `enum_name`, or else the enum's name in snake_case. An
>   enum stored any other way names both keys, and a variant with no value
>   then falls back to its discriminant.

## The composite derive

> [spec:pgorm:sem:macros.derive.entity-model+9]
> `Entity` is the composite derive: applied to a `Model` struct, it expands
> the entity-model generation, re-runs the `Model` derive on the same input and writes the
> `ActiveModel` (`macros.derive.active-model`), so one derive yields the full entity
> module. The entity-model portion
> generates: (1) a `Column` enum deriving `Copy, Clone, Debug` with one variant per
> non-ignored field, beside the impls the `Column` derive writes for an enum
> (`macros.derive.column`), each variant named for its field's `column_name` or else the
> snake_case of the variant, and iterable over every variant but `table_iden`'s
> `Table`; (2) an `impl ColumnTrait for Column` with
> `type EntityName = Entity`, a `def()` match arm per column, a `json_key()` match arm
> per column, and `select_as`/`save_as`/`save_array_as`
> overrides for columns carrying `select_as`/`save_as` attributes, or naming the
> functions `select_with`/`save_with`/`save_array_with`. What the `_as`
> attributes carry is a type *expression* the entity's own author wrote — `BIT(8)`,
> `numeric(10,2)` — and not an identifier, so the override MUST cast through
> `cast_as_raw`, which renders the text as written. Routing it through the
> identifier-quoting `cast_as` instead would spell `CAST(_ AS "BIT(8)")`, a name
> PostgreSQL has no type for. The verbatim rendering is sound exactly because the
> text is a compile-time literal in the caller's source, unreachable from data;
> `[spec:pgorm:def:sql.types.type-name]` is where that text rides — the
> `verbatim` flag of the ordinary cast node, not a shape of its own
> (`[spec:pgorm:req:sql.ast.cast-shape]`) — and it keeps every
> *identifier*-borne type name quoted
> — a `save_as = "T"` column's `save_array_as` casts to `T[]`, so scalar and array
> comparisons carry the same cast — and otherwise fall back to
> `ColumnTrait::select_enum_as`/`save_enum_as`/`save_enum_array_as`; (3) only when
> `table_name` is given, a `pub struct Entity;` deriving
> `Copy, Clone, Default, Debug, EntityTrait` together with a hand-rolled `EntityName`
> impl returning the `table_name`, optional `schema_name`, and optional `comment` —
> without one, the entity's names are computed at run time and its `Entity` is written
> by hand (`macros.derive.entity`); and (4) a `PrimaryKey` enum deriving
> `Copy, Clone, Debug`, iterable, with its name, column mapping and `PrimaryKeyTrait`
> impls (see `[spec:pgorm:sem:macros.derive.entity-model.primary-key]`).
>
> A `select_with`, `save_with` or `save_array_with` names a function of the hook's own
> signature, `fn(Expr) -> SimpleExpr`, which the override calls with the column's
> expression or the value written — the conversion a fixed cast cannot say: a type
> named at run time, or one whose name needs quoting. A column is read one way and
> written one way, so `select_as` beside `select_with`, `save_as` beside `save_with`,
> or one of them twice is a compile error spanned at the second. `save_with` needs
> `save_array_with` beside it, as `save_as` makes `save_array_as` cast to its `T[]`:
> the array `= ANY` and `<> ALL` compare a column with is not a value of the column's
> type, so the function writing one value cannot write it. `save_array_with` without
> `save_with` is refused.
>
> The `json_key()` arm is the one place the derive reads an attribute outside the
> `#[pgorm(...)]` namespace: the key is the field's own name, put through
> `#[serde(rename_all = "..")]` on the struct and replaced outright by
> `#[serde(rename = "..")]` on the field, taking the deserialize half where either is
> written in the split `(serialize = .., deserialize = ..)` form. Every other `serde`
> parameter is stepped over unread. Column naming does not take part in either
> direction — `#[pgorm(column_name)]` and `#[pgorm(variant)]` move the SQL name and
> the variant, not the key — which is the point of the two namespaces being separate
> (`[spec:pgorm:def:entity.traits.column]`).

> [spec:pgorm:req:macros.derive.entity-model.reject+4]
> The `Entity` derive's input MUST be a struct named exactly `Model`; any other identifier
> is the compile error "`Entity` is derived on the struct named `Model`",
> spanned at the offending identifier rather than raised as a macro panic. The
> entity MUST have at least one `#[pgorm(primary_key)]` field: with none, the derive
> emits the compile error "Entity must have a primary key column. See SeaORM's issue
> 485 <https://github.com/SeaQL/sea-orm/issues/485> for details." spanned at the
> struct's identifier, and no `PrimaryKey` enum.

> [spec:pgorm:syn:macros.derive.entity-model.attrs+10]
> Struct-level `#[pgorm(...)]` keys recognised by the `Entity` derive: `table_name = Lit`,
> `schema_name = Lit`, `comment = Lit`, bare `table_iden`, bare `hooks`
> (`macros.derive.boilerplate`), and `rename_all = "style"`
> (case styles per the strum-derived list: `camelCase`, `PascalCase`, `kebab-case`,
> `snake_case`, `SCREAMING_SNAKE_CASE`, `SCREAMING-KEBAB-CASE`, `lowercase`, `UPPERCASE`,
> `title_case`, `mixed_case`; unknown styles are a compile error). `table_iden` (only
> effective together with `table_name`) adds a `Table` variant to `Column`, which
> iterating the columns skips, whose `def()` arm panics with "Table
> cannot be used as a column".
>
> `unique(..)`, given once per key, declares a unique key over two columns or
> more, which `EntityName::unique_keys` returns in declaration order
> (`[spec:pgorm:req:entity.traits.entity-name]`) and
> `create_table_from_entity` writes (`[spec:pgorm:sem:schema.from-entity]`):
> `columns = (..)`, a tuple of `Column` paths written bare in key order, as a
> relation's `from` is, each `Column::<Variant>` of the model's own columns,
> which the derive writes as that column's SQL name since the key is data
> (`EntityName::unique_keys`), beside an optional `name = "..."`, the constraint's
> name, which PostgreSQL derives when it is absent, and an optional bare
> `nulls_not_distinct`, under which nulls are equal: `unique(name =
> "uq-roster-team-shirt", columns = (Column::Team, Column::Shirt))`. The
> columns are one value rather than words of the list because two keys
> sharing a column would repeat a word of one attribute, which
> `clippy::duplicated_attributes` refuses in the program deriving it. The
> derive MUST refuse, spanned where the mistake is written: no `columns`;
> fewer than two, a one-column key being its field's `unique`; a column
> named twice, which PostgreSQL refuses (`42701`); a path naming none of the
> model's columns; a column declared `generated_virtual`, which PostgreSQL
> cannot key (`0A000`) as it cannot the field-level `unique`
> (`[spec:pgorm:sem:macros.derive.entity-model.column-def]`); a quoted column; a key
> given twice; a name that is empty or past 63 bytes, which PostgreSQL would
> refuse or truncate; any other key; and `unique` on a `Model` given no
> `table_name`, whose `EntityName` the program writes and whose
> `unique_keys` it writes there.
>
> Field-level keys: bare `primary_key`, `without_overlaps`, `nullable`, `indexed`,
> `unique`, `ignore`, `identity`, `identity_by_default`;
> `auto_increment = bool`; `column_type = <expr>`, any expression of type `ColumnType`;
> `column_name = "string"`; `variant = Ident`, the field's `Column` variant;
> `default_value = <expr>`; `default_expr = <expr>`; `generated_stored = <expr>`;
> `generated_virtual = <expr>`; `comment = Lit`; `select_as = "sql type"`;
> `save_as = "sql type"`; `select_with = path`, `save_with = path` and
> `save_array_with = path`, each a function. Every value that is Rust code is
> written bare (`macros.derive.code-values`). String-typed keys reject non-string
> literals with an `Invalid <key> ...` error, and a string where code is expected —
> `column_type = "Text"`, `default_expr = "Func::uuidv7()"`, `select_with = "f"` — is
> the compile error "`<key>` is Rust code, written bare: remove the quotes", spanned
> at the string. `enum_name` on a field is refused at the key, saying the key naming a
> field's `Column` variant is `variant`.
>
> A key outside the recognised set, at either level, MUST be the compile error
> "unknown pgorm attribute `<key>`" spanned at the key itself, so a typo — a
> `auto_incremnt = false` that would otherwise leave `auto_increment` at its `true`
> default and render `bigserial` where the author asked for `bigint` — is diagnosed
> rather than dropped. The same refusal governs what reads a subset of this vocabulary,
> and depends on which derive is reading. The composite runs `Model` and writes the
> `ActiveModel` over its own input after refusing what the vocabulary does not hold,
> so those two read a field's `variant` (and `ignore`) and step over every other
> field-level key above, consuming its `= value`, and leave the container to the
> composite. Derived by the program on a further model, `Model` reads the container's
> `entity` and a field's `variant` and `ignore`, and MUST refuse every other key at
> the key: one of the composite's container or field keys saying it is the
> composite's, any other as unknown. The `Column` derive reads a variant's
> `column_name` and refuses any other key, on a variant or on the enum; the `Table`
> variant `table_iden` adds carries none.

> [spec:pgorm:sem:macros.derive.entity-model.casing+4]
> Column-variant naming: the field identifier is stripped of a leading `r#`, converted
> to UpperCamelCase, and then keyword-escaped — any of the 49 reserved Rust keywords
> becomes a raw identifier (`r#type`), while `crate`/`Self`/`self` get a trailing
> underscore. A `variant` key replaces the computed variant name entirely with the
> identifier it is written as, a raw identifier included.
>
> Case conversion drops characters that are neither alphanumeric nor word boundaries, so
> the derived name can fail to spell an identifier: the legal field names `__` and `_1`
> derive `""` and `"1"`. Each MUST be a compile error naming the field and the derived
> spelling, spanned at the field identifier, rather than a panic inside `Ident::new`.
> Validity is judged on the keyword-escaped spelling, so `SELF` — which derives `Self`
> and escapes to `Self_` — stays legal. A `variant` value that is not an identifier
> is a compile error at the value. The same rules govern the variant names the `Model`
> derive and the composite's `ActiveModel` compute.
>
> The SQL column name is attached as `#[pgorm(column_name = "...")]` on the generated
> variant only when needed: an explicit `column_name` always wins; with `rename_all`,
> the UpperCamelCase variant name is converted per the case style; otherwise, when the
> field name does not survive a UpperCamelCase-then-snake_case round trip (e.g. `_id`,
> `id_` or names with double underscores), the snake_case of the original name is pinned.
> Fields that are already clean snake_case get no attribute, and their SQL name falls out
> of the `Column` derive's default (snake_case of the variant), so a field given a
> `variant` and no `column_name` names its column after the variant.

> [spec:pgorm:sem:macros.derive.entity-model.column-def+10]
> Each `def()` arm builds `ColumnTypeTrait::def(<column type>)`. The column type is the
> `column_type` expression if present, taken as written with no implicit prefix, so a
> helper's call (`column_type = varchar(40)`) or a type named at run time is one;
> otherwise it is inferred by matching the
> field's Rust type structurally — against the `syn::Type`, never against a
> whitespace-stripped rendering of it:
> `String`/`&str`→`string(None)`,
> `i16`→`SmallInteger`, `i32`→`Integer`, `i64`→`BigInteger`,
> `f32`→`Float`, `f64`→`Double`, `bool`→`Boolean`,
> `Date`/`NaiveDate`→`Date`, `Time`/`NaiveTime`→`Time`, `DateTime`/`NaiveDateTime`→
> `Timestamp`, `DateTimeUtc`/`DateTimeLocal`/`DateTimeWithTimeZone`→
> `TimestampWithTimeZone`, `Uuid`→`Uuid`, `Json`→`Json`, `Decimal`→`Decimal(None)`,
> `Vec<u8>`→`Bytea`. Every row names a `ColumnType` Postgres has, and every row round
> trips: what the type's `Value` binds is what its `TryGetable` reads back.
> A named row matches only a bare,
> single-segment path carrying no generic arguments, so a qualified spelling
> (`std::string::String`) is not in the table; `&str` matches only a shared reference
> with neither lifetime nor `mut`, and `Vec<u8>` only that exact one-argument spelling.
> Any other type is assumed to be an
> `ActiveEnum`-style value and resolves at compile time via
> `<T as ValueType>::column_type()`, reproducing the type exactly as written: a
> lifetime, `dyn`, or `impl` inside it MUST NOT fuse with the token beside it
> (`&'a str` becoming `&'astr`), and the fallback MUST NOT be able to fail to re-parse.
> `u8` and `u16` are deliberately absent from
> the table: they fall through to that `ValueType` path and, since neither
> implements `ValueType` (see `[spec:pgorm:def:sql.value.conversions]`), a `u8`
> or `u16` field is a compile error rather than a column type that panics when
> bound.
>
> `i8`, `u32`, `u64` and `char` are absent for the neighbouring reason, and MUST be a
> spanned compile error naming the type, the reason, and `column_type` as the escape
> hatch — not a silent fall-through and not an inferred type. Each binds a `Value` its
> own decode cannot read back: `i8` binds `int2` but its `FromSql` accepts only the
> Postgres `"char"` type, `u32` binds `int8` but reads `OID`
> (`[spec:pgorm:sem:exec.decode.u32-oid]`), and `u64` and `char` have no `TryGetable` at
> all. Inferring for them generates DDL that inserts fine and then fails
> `WrongType` on the first `find()`, which is exactly the column type this rule exists
> to refuse. An explicit `column_type` is taken as written and never second-guessed.
>
> An `Option<T>` wrapper — a bare, single-segment `Option` with exactly one type
> argument — is unwrapped one level to `T` and forces `nullable`. Modifier calls are
> then chained in order: `.nullable()`, `.indexed()`, `.unique()`,
> `.default_value(expr)` (from `default_value`), `.comment(lit)`, `.default(expr)` (from `default_expr`),
> `.identity()` or `.identity_by_default()` (from the key of the same name), and
> `.generated(expr, GeneratedKind::Stored)` or `.generated(expr, GeneratedKind::Virtual)`
> (from `generated_stored` / `generated_virtual`).
>
> A generated column's expression is a Rust expression written bare, as `default_expr`'s
> is, in the entity's own module, so it names the row's columns as `Column`
> variants: `generated_stored = Expr::col(Column::Price).mul(Expr::col(Column::Quantity))`.
> The kind is in the key's name and has no default, for the reason the builder's
> `generated` takes it as an argument (`[spec:pgorm:req:sql.ddl.column-def]`).
> Each combination PostgreSQL refuses for a generated column MUST be a compile error
> spanned at the field's generated key: both keys, or one twice ("a column is generated
> one way"); a generated key beside `identity` or `identity_by_default` ("both identity
> and generation expression specified", 42601), beside `default_value` or `default_expr`
> ("both default and generation expression specified", 42601), or beside
> `auto_increment` on the same field, since serial is a default (42601) and the key
> already says how the column is filled; and `generated_virtual` on a field that is
> `primary_key`, `unique` or `indexed`, each of which PostgreSQL refuses on a virtual
> column (0A000) and accepts on a stored one. What the server knows and the derive
> does not stays the server's: a virtual column over a user-defined or domain type,
> an enum included (0A000) — a `column_type` override or a `ValueType` decides the
> type — and an expression that is not immutable or reads another generated column
> (42P17).
>
> The parallel `ArrayType` table that the `ValueType` derive reads
> (`[spec:pgorm:sem:macros.derive.value-type]`) is matched the same way and carries the
> same rows mapped to their `ArrayType` counterparts, with two divergences. `Vec<u8>`
> it does not carry at all. And the timezone-aware datetime row splits:
> `DateTimeUtc`→`ChronoDateTimeUtc`, `DateTimeLocal`→`ChronoDateTimeLocal`,
> `DateTimeWithTimeZone`→`ChronoDateTimeWithTimeZone`. The array tag is the element's
> Rust-side decode identity, not its wire type — all three are `timestamptz` on the
> wire, which is why the `ColumnType` row stays merged, but each type's own
> `ValueType::array_type()` names a distinct tag and the inferred tag MUST agree
> with it. The refusal above is on the `ColumnType` path only, so it reaches
> the `ValueType` derive too: a newtype over one of the four refused types is the same
> compile error.

> [spec:pgorm:sem:macros.derive.entity-model.primary-key+7]
> Every `primary_key` field contributes a variant to the generated `PrimaryKey` enum and
> its type to `PrimaryKeyTrait::ValueType` — a bare type for a single key, a tuple for
> composite keys. `auto_increment()` reports whether the database generates the whole
> key (`entity.traits.primary-key`). When no key field declares an identity or a
> generated column it returns
> true only when there is exactly one primary key column and no field set
> `auto_increment = false` (the flag defaults to true and is shared: a `false` on any
> field flips it globally). When some key field does, it returns true exactly when every
> key field does, whatever the flag says: a stored generated column
> (`generated_stored`, `macros.derive.entity-model.column-def`) is computed by the
> database as an identity is drawn by it, so a key made of such columns is generated
> whole, and a key holding one beside a supplied column is not.
>
> A column the database generates inside a composite key — `id` in the multi-tenant
> `(tenant_id, id)` — is declared on its own field: `identity` for
> `GENERATED ALWAYS AS IDENTITY`, `identity_by_default` for
> `GENERATED BY DEFAULT AS IDENTITY`, either of which the column's `def()` carries
> (`macros.derive.entity-model.column-def`) and schema generation renders on that column
> alone (`schema.from-entity`). Either key is allowed on a field outside the key too, an
> identity being a column's fact rather than a key's. The serial family is not offered
> per column: it remains the one-column key's default, PostgreSQL recommends identity
> over it, and a second spelling of a generated column would be a second thing to learn.
> An entity cannot declare the options of an identity's sequence — `START WITH` and the
> rest tune how the sequence counts, not which rows the table holds, and belong to a
> migration's `ColumnDef::identity_with`.
>
> Each combination PostgreSQL refuses for an identity column (42601) MUST be a compile
> error spanned at the field's identity key: both `identity` and `identity_by_default`;
> an identity beside `default_value` or `default_expr` ("both default and identity
> specified"); and an identity on a nullable field, through `nullable` or an `Option`
> type (an identity column is NOT NULL). An identity beside `auto_increment` on the same
> field is refused too, since the identity already says how the column is generated.
> `auto_increment = true` on a field of a composite key MUST be a compile error spanned
> at the key, pointing at `identity`: before this rule it was silently ignored, and the
> author who wrote it got a key column with no generator, whose insert fails `23502`.
> PostgreSQL itself refuses an identity over a non-integer column (22023); the derive
> leaves that to the server, since a `column_type` override decides the type.
>
> A key MUST have at most twelve columns, the widest tuple `ValueType`'s traits are
> implemented for (`entity.traits.primary-key`). A thirteenth `primary_key` field is one
> compile error spanned at its key, naming the limit; before this rule the derive
> generated the 13-tuple and the caller read a cascade of E0277s on traits they never
> wrote. `primary_key` given twice on one field is a compile error spanned at the
> second: a field is one column of the key, and the repeat used to add its type to
> `ValueType` twice beside one `PrimaryKey` variant.
>
> A field declared `primary_key, without_overlaps` is the key's period,
> PostgreSQL 18's temporal key (`entity.traits.primary-key`): the derive
> answers `without_overlaps()` true, and schema generation writes the column
> `WITHOUT OVERLAPS` after the rest. PostgreSQL takes it on the key's last
> column alone and refuses a key it is the only column of (both `42601`), so
> each is a compile error spanned at the key — the period on a field before
> another key field, the period as the only key field — and so is
> `without_overlaps` on a field that is not a key column.
>
> Beside the `PrimaryKey` enum the derive writes `SqlName` delegating to
> `StaticName::as_str`, a `StaticName` impl mapping each variant to its column's SQL
> name — the field's `column_name`, or the name the field derives
> (`macros.derive.entity-model.casing`) — and a `PrimaryKeyToColumn` impl whose
> `into_column`/`from_column` map variants to the same-named variants of `Column`.

> [spec:pgorm:sem:macros.derive.entity+7]
> The `EntityTrait` derive wires up an entity's `Entity` struct: the composite derive
> puts it on the unit struct it writes, and an entity whose names are computed at run
> time puts it on the struct it writes itself (`macros.derive.one-form`). It always
> generates `impl EntityTrait` with associated types `Model`, `ActiveModel`, `Column`,
> `PrimaryKey`, `Relation` — each identifier defaulting to those names but overridable
> via struct-level `#[pgorm(model = Ident, active_model = Ident, column = Ident,
> primary_key = Ident, relation = Ident)]`, every such attribute on the struct read —
> plus `SqlName` and `StaticName` impls that render `EntityName::table_name`, the
> `SqlName` answering `EntityName::schema_name` as its `table_schema`
> (`[spec:pgorm:def:sql.types.table-ref]`). It writes no `EntityName`: the composite
> writes its own, and an entity whose names are computed implements it by hand.
>
> Any other key MUST be a compile error spanned at the key, never dropped unread: one
> of the composite's container keys — `table_name`, `schema_name`, `comment`,
> `table_iden`, `rename_all`, `hooks`, `unique` — naming the composite derive on the `Model` as
> where it goes; `entity`, which names the entity a model or relation belongs to, saying the
> derive is written on the entity itself; and any other as unknown, listing the five
> keys the derive reads.

## Column, Model and ActiveModel derives

> [spec:pgorm:sem:macros.derive.column+7]
> The `Column` derive (enums only; other inputs are a compile error) generates: an inherent
> `default_as_str` returning the snake_case of the variant name or a
> `#[pgorm(column_name = "...")]` override; a `FromStr` impl accepting either the
> snake_case or the lowerCamelCase spelling of each variant and returning
> `ColumnFromStrError(input)` otherwise; an `SqlName` impl writing `StaticName::as_str`;
> an `StaticName` impl whose `as_str` is `default_as_str`; and the enum's iteration over
> its variants in declaration order (`macros.derive.enum-iter`), so no `EnumIter` is
> derived beside it. A column whose name is not
> snake_case says so with `column_name`, the one key the derive reads: any other key,
> on a variant or on the enum, is the compile error "unknown pgorm attribute `<key>`"
> spanned at the key. The composite derive writes the same impls for the `Column` it
> makes, from each field's column name rather than an attribute.

> [spec:pgorm:sem:macros.derive.model+7]
> The `Model` derive (named-field structs only; otherwise a compile error) generates
> `FromQueryResult`, `ModelTrait` and `ColumnField`. `FromQueryResult`: each field is read with
> `row.try_get(pre, Column::<Variant>.as_str())`, where the variant name follows the
> same trim/UpperCamelCase/keyword-escape/`variant` rules as
> `[spec:pgorm:sem:macros.derive.entity-model.casing]`; `#[pgorm(ignore)]` fields are
> filled with `Default::default()` instead. `expected_columns` reports the same
> column names, each paired with its field type's spelling and that type's
> `TryGetable::accepts` and carrying the read cast of its column
> (`ExpectedColumn::selected_as`), so an entity model can be checked against a raw
> statement (`exec.verify`) and a pipeline's model terminals cast what it reads
> (`pipeline.terminal`); ignored fields read no column and are left out. `ModelTrait`: `get` clones the field and
> converts it `.into()` a `Value`; `set` converts through `ValueType::from_value` and
> assigns, answering `Error::Type("value does not match the type of {Model} field
> {field}")` when the value is of another type. Ignored fields have no match arm, so
> `get` on them panics with "field does not exist on {Model}" and `set` returns that
> same text as `Error::Type`. `ColumnField<{ Column::<Variant> as usize }>`, for each
> field that is not ignored, keyed by its column's discriminant: its `Value` is the
> field's type as declared and `field` borrows the field, so code that names a column
> as a value reaches the column's type at compile time, as `OneOf` holds a column to
> its target's key (`[spec:pgorm:def:disjoint.one-of]`). The
> target entity defaults to `Entity`, overridable via `#[pgorm(entity = Ident)]`.
> Derived by the program, the derive reads that `entity` and a field's `variant` and
> `ignore` and nothing else, refusing any other key at the key; run by the composite,
> it reads a field's `variant` and `ignore` and leaves the rest of the input to the
> composite (`macros.derive.entity-model.attrs`).

> [spec:pgorm:sem:macros.derive.active-model+9]
> The composite derive writes, beside its `Model`, a
> `pub struct ActiveModel` — name and entity are hard-coded as `ActiveModel`/`Entity` —
> with one `pub field: ActiveValue<T>` per non-ignored field, plus: `Default`
> delegating to `ActiveModelBehavior::new()`; `From<Model>` mapping every field through
> `ActiveValue::unchanged`; an `IntoActiveModel` impl for `Model`; and
> `ActiveModelTrait` with `take`, `get` and `set`, which share one answer for an
> unmatched column — `Error::Type("This ActiveModel does not have this field")`,
> generated from a single spelling of that message so the three cannot drift
> apart — plus, for `set` alone, `Error::Type("value does not match the type of
> ActiveModel field {field}")` when the value is of another type.
> `not_set` (silently ignores unmatched), `is_not_set` (panics on unmatched),
> `default` (all fields `not_set`), and `reset`. It also generates
> `TryFrom<ActiveModel> for Model` and `TryIntoModel`: each non-ignored field is
> taken out of its `ActiveValue` directly — no round trip through `Value` — and one
> left `NotSet` fails with `Error::AttrNotSet(field)`; ignored fields are rebuilt with
> `Default::default()`.
>
> Beside them it writes `impl ActiveModelBehavior for ActiveModel {}`, the behaviour with
> no hooks, unless the `Model` says `#[pgorm(hooks)]`, when the entity's module
> implements it (`macros.derive.boilerplate`); it writes it whether or not the rest of
> the composite's expansion is refused, so a refusal elsewhere is not echoed as a missing
> implementation. The `IntoActiveModel` impl names the struct as `Model`, not as the
> projection `<Entity as EntityTrait>::Model`: across crates coherence does not
> normalize a projection in another crate's impl header, so a form deriving
> `IntoActiveModel` for this `ActiveModel` in a crate of its own would otherwise be
> refused as conflicting with it (E0119).
>
> The `IntoActiveModel` derive (named-field structs only) generates an
> `IntoActiveModel<A>` impl — `A` defaults to `ActiveModel`, overridable with
> `#[pgorm(active_model = Path)]`, written as the path it is (`active_model =
> customer::ActiveModel`, `macros.derive.code-values`), the one key it reads on every
> `#[pgorm(...)]` of the struct, any other key there or on a field being a compile error
> at the key — that converts each field via
> `IntoActiveValue::into_active_value(..).into()` and fills the remainder with
> `..Default::default()`, enabling partial "form" structs, in the entity's crate or any
> other. A field's type converts through `IntoActiveValue`: pgorm implements it for the
> built-in column types, and each derive that makes a column value type — `ActiveEnum`,
> `ValueType`, `JsonValueType` and `CreatedRange` — writes `IntoActiveValue<Self>` for
> it, so a form's field of such a type sets its column as a built-in one does, and
> `Option<T>` of one sets it when `Some`, each of the four writing `Nullable` too.

## Active enums

> [spec:pgorm:syn:macros.derive.active-enum+6]
> The `ActiveEnum` derive applies to enums only ("you can only derive `ActiveEnum` on
> enums").
> Container attributes: `rs_type = Type`, the values' type written as a type
> (`rs_type = String`, `rs_type = i32`), and `db_type = <expr>`, the `ColumnType` the
> values are stored as, written as an expression and taken as written
> (`db_type = ColumnType::Integer`), come together or not at all. An enum naming neither
> is a PostgreSQL enum of strings (`macros.derive.boilerplate`): its `rs_type` is
> `String` and its column type
> `ColumnType::Enum { name: Self::name(), schema, variants: Self::name_values() }`, where
> `schema` is the `schema_name` attribute as a name when given and `None` otherwise. A
> `db_type` without `rs_type` is the compile error "Missing macro attribute `rs_type`",
> and an `rs_type` without `db_type` is refused at the key, saying an enum stored as
> `rs_type` names its column type too and a PostgreSQL enum of strings names neither.
> No spelling of `db_type` stands for the enum type: a `db_type` is always the expression
> written. `enum_name = "string"` (defaulting to the enum's name in snake_case),
> `schema_name = "string"`, `rename_all = "style"` and the bare `display` are optional,
> and attribute order does not matter. Variant attributes:
> `string_value = "s"`, `num_value = int`, `rename = "style"`, and
> `display_value = "s"`, which only `display` reads, so without `display` it is a
> compile error spanned at its value rather than text dropped unread. An unknown key at
> either level is the compile error "unknown pgorm attribute `<key>`" spanned at the
> key. String-flavoured markers (`string_value`, `rename`, `rename_all`) and
> `num_value` are mutually exclusive across the whole enum; mixing them is the compile
> error "All enum variants should specify the same `*_value` macro attribute...".
>
> In a PostgreSQL enum a variant with no `string_value` is its name in the `rename` or
> `rename_all` style, snake_case by default, and a value that is not a string is
> refused where it is written: a `num_value`, and a discriminant on a variant that
> takes the snake_case default, each saying an enum stored as numbers names `rs_type`
> and `db_type`. In an enum that names both keys, a variant with no attribute falls back to
> its integer discriminant, including negative literals written with unary minus (other
> unary operators are rejected), and a variant with neither attribute nor usable
> discriminant is a compile error.
>
> A PostgreSQL enum's value, however it is given, MUST be at most 63 bytes, the most
> PostgreSQL holds in an enum's value (`NAMEDATALEN - 1`; `CREATE TYPE` refuses a longer
> one with `42602`): a longer one is refused at expansion, spanned at its `string_value`
> or else at its variant, naming the value and its length in bytes. An empty value is one
> PostgreSQL holds, and is not refused. The limit bounds no value of an enum that names
> `db_type`, whose column type decides what it holds. A derive is given its enum as
> configured, so a variant that `#[cfg]` removes is no value of the enum, as it is no
> variant of the type, and a key that `cfg_attr` gives is read as written.

> [spec:pgorm:sem:macros.derive.active-enum.expansion+6]
> The expansion generates: a unit struct `{Enum}Enum` with an `SqlName` impl writing the
> `enum_name`; when any variant has a string value (given, through `rename` or
> `rename_all`, or the snake_case name a PostgreSQL enum's variant defaults to), an enum
> `{Enum}Variant` with an `SqlName` impl writing the raw string values
> and an inherent `name_values()` returning them as `Name`s, in declaration order; an
> `ActiveEnum` impl
> with `Value = rs_type`, `ValueVec = Vec<rs_type>`, `name()` returning the
> `{Enum}Enum` iden, `to_value()` matching variants to their values, `try_from_value()`
> matching back (comparing `v.as_ref()` for strings) and failing with
> `Error::Type("unexpected value for {Enum} enum: ...")`, and `db_type()` building a
> `ColumnDef` from the `db_type` expression, or from the PostgreSQL enum type an omitted
> one stands for; the enum's iteration over its variants in declaration order
> (`macros.derive.enum-iter`), which `ActiveEnum: Iterable` requires; plus `TryGetable`,
> `TryGetableArray`, `Into<Value>`, `ValueType`, `Nullable`, and the `NotU8` marker impl.
>
> `{Enum}Variant` identifiers are produced by an escaping PascalCase conversion:
> characters outside Unicode UAX#31 identifier classes, plus `_` and space, are replaced
> by their `{:#X}` hex notation (`_`→`0x5F`, space→`0x20`); the empty string becomes
> `__Empty`; a leading digit gets an `_` prefix. Known limitation: the conversion is
> case-preserving but collision-blind — string values differing only in case map to the
> same identifier and fail to compile. The enum carries
> `#[allow(clippy::enum_variant_names)]`, since its variants are spelt by the escaping and
> not by the program: values sharing a last word, `web_shop` and `pop_up_shop`, give
> variants sharing a postfix, which the lint refuses.
>
> With `display`, the expansion also implements `std::fmt::Display`, writing each
> variant's `display_value`, or else its identifier text. It is a key rather than a
> derive of its own so that no pgorm macro takes the name `Display` derive_more and
> strum export (`macros.derive.names`), and opt-in so that it never collides with a
> `Display` the program implements itself.

## Relations

> [spec:pgorm:syn:macros.derive.relation+7]
> The `Relation` derive applies to enums only. Each variant requires exactly one of
> `belongs_to = path`, `has_one = path` or `has_many = path`, the entity it reaches
> written as a path (`belongs_to = super::cake::Entity`); none is the error "Missing
> one of 'has_one', 'has_many' or 'belongs_to'", and a second is "a relation is one of
> `belongs_to`, `has_one` or `has_many`, once". `belongs_to` variants additionally
> require `from = Column::X` and `to = Column::Y` (errors "Missing attribute
> 'from'"/"Missing attribute 'to'"); on `has_one`/`has_many` both are optional, but the
> two MUST be given together or not at all — one without the other raises the same
> missing-attribute error, since a column can only be added to a relation paired with
> its counterpart. Both values are expressions and a tuple on either side is taken
> apart per column, so a composite key expands to
> `.columns(from0, to0).and_columns(from1, to1)...`. Sides of unequal length are
> rejected at expansion, where the arity is still known, with "'from' names N column(s)
> and 'to' names M; a relation joins its columns in pairs". Further optional keys chain
> builder calls, each value an expression taken as written: `on_update` / `on_delete`
> (`ForeignKeyAction::Cascade`), `on_condition`, `condition_type`
> (`ConditionType::Any`), and `enforcement` / `deferrability`
> (`Enforcement::NotEnforced`, `Deferrability::DeferrableInitiallyDeferred`). The
> builder methods take the enums by type, so a name none of their variants has is the
> compiler's refusal at the value. `fk_name`, the constraint's name, is a string.
> `from_period = Column::P` and `to_period = Column::Q` give a temporal relation's
> period pair (`entity.relation.def`), chained as `.period(P, Q)` after the column
> pairs: the two come together ("Missing attribute 'to_period'" / "'from_period'"),
> after a `from` / `to` pair, since a period is matched after the columns compared for
> equality, and beside no `on_update` / `on_delete` but a path ending in `NoAction`
> (`ForeignKeyAction::NoAction`), PostgreSQL 18 refusing every other action on a
> temporal foreign key (`0A000`) — each refusal a compile error, the last spanned at the
> action. `on_condition` is a closure of the two tables as the join binds them,
> `|left, right| ..`, whose parameters are typed as the `Name`s they are where it leaves
> them untyped and whose result goes through `IntoCondition`, or an expression that
> ignores them, which is wrapped in a closure that does; a closure of another arity is
> refused, "an `on_condition` closure takes the two tables the join binds". A variant's
> keys accumulate across its `#[pgorm(...)]` attributes, and a key given twice is the
> compile error "duplicate `<key>`" at the second. `related = false` leaves the entity
> the variant reaches to a `Related` impl the program writes (`macros.derive.boilerplate`);
> `related` is written only so, `related = true` refused at its value as the default,
> and `related = false` on a variant whose entity another variant also reaches is refused
> at its value, since the derive writes no `Related` for that entity anyway. The
> expansion is an
> `impl RelationTrait` whose `def()` matches each variant to
> `Entity::belongs_to/has_one/has_many(target)` plus the paired column calls, the
> chained modifiers and a catch-all arm panicking "No RelationDef for {Relation}"; the
> enum's iteration over its variants in declaration order (`macros.derive.enum-iter`);
> and, for each target path exactly one variant names and that variant does not say
> `related = false`, `impl Related<target> for Entity` whose `to()` is that variant's
> `def()`. An entity is known by the path that names it, compared as written: two
> spellings of one entity are two targets, and the two impls written for them conflict.
> The entity identifier defaults to `Entity`, overridable via container
> `#[pgorm(entity = Ident)]`, the one key the derive reads on the enum. Any other key
> there is refused at the key: one of the composite's container keys saying it goes on
> the `Model` under `Entity`, any other as unknown.

## Projection and value derives

> [spec:pgorm:sem:macros.derive.partial-model+6]
> The `PartialModel` derive accepts only non-generic named-field structs (generics and other
> shapes are compile errors). The container attribute `entity = Type` names the source
> entity, written as a type; it is required unless every field carries `from_col` or
> `from_expr`, and its absence otherwise produces "you need specific which entity you
> are using". Per field: no attribute selects
> `<entity as EntityTrait>::Column::{UpperCamelCase(field)}`; `from_col = path` selects
> the column the path names, taken as written (`from_col = cake::Column::Name`), under
> the column's own `select_as` and aliased to the field name; `from_expr = <expr>`
> selects the expression aliased to the field name. The expansion implements `PartialModelTrait::select_cols` by chaining
> `QuerySelect::column` / `column_as` calls; the chain's type is `S::Projected`, so a
> struct with no fields — whose chain is empty and whose body is therefore the
> unprojected `S` — does not compile. It also implements `FromQueryResult` for the
> struct as that derive does (`[spec:pgorm:sem:macros.derive.from-query-result]`),
> reading `skip` and stepping over the projection's own keys, so the struct derives
> `PartialModel` alone and a `FromQueryResult` derived beside it is a conflicting
> implementation. Each field that reads an entity column, by default or through
> `from_col`, reports that column's read cast (`ExpectedColumn::selected_as`), so a
> pipeline's model terminals cast it (`pipeline.terminal`).
>
> Every key accumulates across all meta items of all `#[pgorm(...)]` attributes on the
> item, so a later key cannot erase an earlier one and the derive can never generate a
> projection the source does not state. Consequently: a repeated `entity`, `from_col` or
> `from_expr` is the compile error "duplicate `<key>`"; supplying both `from_col` and
> `from_expr` on one field reaches the guard and is the compile error "you can only use
> one of `from_col` or `from_expr`"; and any key outside `entity` at the container level,
> or outside `from_col`/`from_expr`/`skip` at the field level, is
> "unknown pgorm attribute `<key>`" spanned at the key.

> [spec:pgorm:sem:macros.derive.from-query-result+4]
> `FromQueryResult` (named-field structs only; generics supported) implements
> `from_query_result(row, pre)` by `row.try_get(pre, "<field>")` for each field, using
> the un-rawed field identifier as the column name; fields marked `#[pgorm(skip)]` are
> filled with `Default::default()`. `skip` accumulates across every meta item of every
> `#[pgorm(...)]` attribute on the field, so once asked for it cannot be un-asked by a
> later key or a second attribute. It reads no other key: any other on a field or on
> the struct is the compile error "unknown pgorm attribute `<key>`" spanned at the key,
> except that `PartialModel`'s `entity`, `from_col` and `from_expr` are refused saying
> `PartialModel` reads them and implements `FromQueryResult` itself
> (`[spec:pgorm:sem:macros.derive.partial-model]`). The `FromQueryResult` that
> `PartialModel` writes steps over a field's `from_col` and `from_expr`, which
> `PartialModel` reads.
>
> It also implements `expected_columns`, reporting one `ExpectedColumn` per read field
> in declaration order: the same un-rawed identifier, the field type as written (with
> the spaces tokenisation leaves between its parts removed, so `Option<i32>` rather
> than `Option < i32 >`), and `<FieldType as TryGetable>::accepts`. Skipped fields read
> no column and are left out, so a skipped field is not required of the statement
> (`exec.verify`). The acceptance function is taken from the field type, so a generic
> field reports the accepts of whatever `TryGetable` the caller's bound resolves to.
>
> The `JsonValueType` derive performs no input validation at all (only the identifier is
> used). It generates the `TryGetableFromJson` marker; `From<T> for Value` serialising
> through `serde_json::to_value`; a `ValueType` impl accepting only `Value::Json(Some(_))`
> (column type and array type are `Json`); `Nullable` returning `Value::Json(None)`; and
> the `NotU8` marker.
>
> A `serde_json::to_value` failure — a map with keys JSON cannot spell, or a `Serialize`
> impl that errors — MUST panic, with a message naming the type and carrying serde's own
> error. `From` has no channel to report failure, and the alternative is binding SQL NULL
> for a value that was never null: a confusing constraint violation against a `NOT NULL`
> column, and silent data loss against a nullable one. The panic is documented on the
> derive.

> [spec:pgorm:sem:macros.derive.value-type+7]
> The `ValueType` derive targets newtype tuple structs; it reads only the first unnamed
> field. Any other input shape is a compile error spanned at the struct identifier —
> "`ValueType` can only be derived on a tuple struct", or "`ValueType` requires
> the tuple struct to hold a value field" for an empty one — not a macro panic. Optional
> attributes `column_type = <expr>`, any expression of type `ColumnType` taken as the
> entity derive takes one (`macros.derive.entity-model.column-def`), and
> `array_type = <expr>`, an `ArrayType` expression taken as written
> (`array_type = ArrayType::Int`), override the inferred `ColumnType`/`ArrayType`, which otherwise
> use the same Rust-type tables (and `Option<T>` unwrapping) as
> `[spec:pgorm:sem:macros.derive.entity-model.column-def]`, falling back to
> `<T as ValueType>::column_type()`/`array_type()` — and inheriting that rule's refusal
> of `i8`, `u32`, `u64` and `char` on the `ColumnType` side. Attribute errors propagate: a
> string `column_type` or `array_type`, and any other key in the
> `#[pgorm(...)]` list, is a spanned compile error rather than being silently ignored. The expansion implements
> `From<T> for Value` (through `self.0`), `TryGetable`, `ValueType` delegating to
> the inner type with `type_name()` = the struct name, `IntoActiveValue<Self>`
> (`macros.derive.active-model`), and `Nullable`, whose `null()` is the inner type's —
> for a field `Option<T>` the payload `T`'s, `Option` itself having no `Nullable`, the
> same unwrapping the column-type inference does. So `Option<Newtype>` is a nullable
> column and a form's optional field, as `Option<T>` of a built-in type is: `None`
> binds as the inner type's typed NULL. An inner type with no `Nullable` of its own is
> a compile error at the derive.

> [spec:pgorm:sem:macros.derive.created-range+3]
> The `CreatedRange` derive names a range type a schema created, or the
> multirange created beside one (`[spec:pgorm:def:sql.value.created-range]`),
> on a tuple struct holding one field: written `Range<T>` under `range_name =
> "..."`, `Multirange<T>` under `multirange_name = "..."` — the last path
> segment, however qualified, with one type argument, `T` the subtype. Exactly
> one of the two keys is required; both, neither, a field the key does not
> name, or any other shape is a compile error spanned at the struct, the key
> or the field. `schema_name = "..."` qualifies the type. Each becomes
> `Name::runtime`, so it is a name and quoted where it needs to be, and any
> other key is a spanned compile error. The expansion implements
> `pgorm::CreatedRange` (`name()`, the `TypeName`), `From<Range<T>>` for the
> newtype, `From<newtype> for Value` as `Value::String` of the text form,
> `Nullable` as the `NULL` string, `TryGetable` — delegating to `Range<T>`'s
> for a range, and for a multirange reading the column's text (which its
> `select_as` casts it to) and parsing it with `Multirange<T>`'s `FromStr`, a
> failure being `TryGetError::Db` naming the newtype — with `accepts`
> answering as what it reads does, and `ValueType`: `from_value` parses a
> `Value::String` with the held type's `FromStr` and refuses anything else,
> `type_name()` is the struct name, `array_type()` is `String`, and
> `column_type()` is `ColumnType::CreatedRange` or `CreatedMultirange`
> carrying the name, the schema and `<T as ValueType>::column_type()`. A field of the newtype therefore
> needs no `column_type` attribute: the `Entity` derive takes the created
> range column type from it. It implements `NotU8` too, so `Vec` of the
> newtype is a field, an array of the type, converting into the `Value::Array`
> of the texts, and `TryGetableArray`, through which such a field decodes:
> for a range each element through its subtype's binary codec, for a
> multirange the column's `text[]` (which its `select_as` casts it to), each
> element parsed as the newtype's own column is, a failure the same
> `TryGetError::Db`. Both read through functions pgorm exports hidden, which
> answer with a `TryGetError` naming the feature in a build without
> `postgres-array`, so the expansion compiles without it.

## Name derives and query helpers

> [spec:pgorm:sem:macros.derive.iden+3]
> The `SqlName` and `StaticName` derives pgorm exports — at its root, in
> `pgorm::entity` and in the entity prelude, beside the traits of the same names — are
> pgorm-macros', and are pgorm-query's derives (`macros.derive.iden.query`) in all but
> the path their expansion names the traits by, `pgorm::pgorm_query::...`, so a crate
> depending only on pgorm can derive them; pgorm-macros has no name derive of its own.
> The traits arrive in pgorm through a glob that carries pgorm-query's derives with
> them, and pgorm's explicit import of pgorm-macros' shadows those in the macro
> namespace alone. pgorm-migration's prelude names pgorm's, which shadow the derives
> its `pgorm_query::*` glob carries. A name written in a string is `#[iden = "..."]`,
> as pgorm-query spells it, not `#[pgorm(iden = "...")]`.

> [spec:pgorm:sem:macros.derive.iden.query+1]
> `pgorm-query-derive` defines `SqlName` and `StaticName` derives over enums and unit
> structs (helper attributes `iden` and `method`; empty enums expand to nothing; other
> shapes are a compile error). The container name defaults to the snake_case type name
> and may only be renamed with `#[iden = "name"]` — list forms at container level are
> rejected. Variant forms: `#[iden = "name"]` and `#[iden(rename = "name")]` substitute
> a literal; `#[method = "name"]` and `#[iden(method = "name")]` render via
> `self.name()`; `#[iden(flatten)]` delegates to the single field of a one-field
> variant (multi-field or unit variants are "Must have a single field is supported for
> flattenning"); the variant `Table` renders the container name; everything else
> snake_cases the variant. Each name is written as a format argument, never as the
> format string, so a name holding `{` or `}` is written as it is. `prepare` is
> emitted only when all names are statically valid idens — first char `_` or ASCII
> alphabetic, rest `_` or ASCII alphanumeric (`method`/`flatten` disqualify) —
> wrapping the name in quote characters; otherwise the trait default quotes it.
> `StaticName` additionally generates `as_str(&self) -> &str` and `AsRef<str>` from
> the same naming rules. A malformed attribute is a spanned compile error, never a
> panic. The derive keeps the attribute word `iden` — the ratified rename covers the
> types and the derives, not the attribute key, which sits beside
> `column_name` / `enum_name` / `table_name` keys that mean other things. The
> expansion lives in the library `pgorm-query-derive-internals`, shared with
> pgorm-macros' derives of the same names (`macros.derive.iden`).

> [spec:pgorm:sem:macros.derive.enum-def+1]
> `#[enum_def]` (in `pgorm-query-attr`) applies to named-field structs — anything else
> panics with "#[enum_def] can only be used on structs" — and re-emits the input
> unchanged followed by a generated
> `pub enum {prefix}{Struct}{suffix}` (defaults: empty prefix, suffix `Name`, so
> `StructName`) deriving `Debug, Clone, Copy, PartialEq, Eq, Hash`, with a `Table`
> variant plus one PascalCase variant per field. Its `SqlName::unquoted` writes
> `stringify!` of the table name identifier for `Table` — defaulting to the snake_case
> struct name, overridable with `table_name = "..."` (which must itself be a valid
> identifier, since it is re-parsed as one) — and `stringify!` of the original field
> identifier for each field variant. The `crate_name = "..."` argument (default
> `pgorm_query`) rewrites the `SqlName` trait path, and there is nothing else in
> the expansion for it to rewrite: the sink argument is spelled
> `&mut dyn ::std::fmt::Write` at its own crate's path, as the two `SqlName`
> derives already spelled it, so the generated method names pgorm-query exactly
> once and a re-export of `std::fmt::Write` is not part of pgorm-query's
> surface.

## Iteration and test harness

> [spec:pgorm:sem:macros.derive.enum-iter+1]
> `Relation`, `Column` and `ActiveEnum` make the enum they are given iterable, and the
> composite derive the `Column` and `PrimaryKey` it makes: for an enum `E` the expansion
> writes an `EIter` struct of the enum's visibility and implements `pgorm::Iterable`
> (strum's `IntoEnumIterator`) yielding it, with `Iterator`, `DoubleEndedIterator`,
> `ExactSizeIterator`, `FusedIterator`, `Clone`, and `Debug` on the iterator, over the
> variants in declaration order. The composite's `Column` leaves out the `Table` variant
> `table_iden` adds. The generation is adapted from strum's `EnumIter`, and no derive
> of that name exists: nothing outside these expansions needs one, and `Iterable` is
> the trait a program names.

> [spec:pgorm:sem:macros.derive.test-attr+1]
> `#[pgorm_macros::test]` is a hidden test-harness attribute: it rewrites an async test
> body into a `#[test]` fn that initialises a `tracing_subscriber` at DEBUG level with a
> test writer and drives the body through `crate::block_on!`. Caller attributes are
> passed through to the generated fn verbatim. The expansion adds no cfg gating of its
> own, so wrapped tests compile unconditionally.

## Compile-time SQL validation

> [spec:pgorm:def:macros.sql+6]
> `pgorm-sql-macro` is a proc-macro crate exporting two function-like macros for
> callers: `sql!`, specified here, and `prql!` (`[spec:pgorm:def:macros.prql]`). A
> third, hidden from its documentation, is the compile-time half of pgorm-migration's
> `sql_migration!` (`[spec:pgorm:def:migration.sql-file]`), which calls it with its own
> crate path and holds a migration's `.sql` file to the grammar as `sql!` holds a
> literal; nothing else is meant to call it. `sql!` takes one
> string literal and nothing else. While the calling crate is being compiled, the
> literal's text is handed to `pg_query::parse` — the Rust binding to libpg_query, the
> PostgreSQL server's own parser, at the same revision the render oracle uses and for
> the same reason (`[spec:pgorm:req:sql.render.oracle]`), so a literal is held to
> PostgreSQL 18's grammar: a statement only 18 parses, such as a `VIRTUAL` generated
> column, compiles. A literal the grammar
> accepts expands to itself, byte for byte, as a `&'static str` usable in const
> position; there is no wrapper type, no runtime check and no allocation, so the macro
> is invisible in the generated code.
>
> The macros live in their own crate rather than in `pgorm-macros` because a proc macro
> for entity derives is the wrong place for the oracle and the PRQL compiler. The root
> crate is a plain dependent and re-exports both as `pgorm::sql` and `pgorm::prql`,
> unconditionally: the retired `sql-macro` feature only decided whether the name was in
> scope — never whether anything was compiled, since `pg_query` is a plain dependency
> of `pgorm` itself (`[spec:pgorm:sem:exec.paginator.raw]` parses raw statements at
> runtime) and prqlc is a plain dependency by the same permanence posture
> (`[spec:pgorm:def:pipeline.adapter]`) — and a gate that guards nothing is surface
> without a state.
>
> The call sites are the escape hatches that take SQL as text, and documentation
> of them MUST be a rule that finds them rather than a list that goes stale — the
> list this rule used to carry named four of them while ten existed, which is
> worse than no list, because an auditor who trusts it audits a quarter of the
> surface. The rule is the `SqlText` bound (`[spec:pgorm:def:conn.sql-text]`):
> every function taking its SQL as `&T where T: SqlText`. On a connection that is
> each `ConnectionTrait` method — `execute`, `execute_raw`, `query_one`,
> `query_opt`, `query_all`, `query_raw` — plus `batch_execute`, which takes its
> `&str` directly because it runs a script through the simple-query protocol. Away
> from a connection a raw statement is SQL text paired with its values, and every
> entry point takes the pair in the same shape, the text under that bound beside
> its `Values` (`[spec:pgorm:sem:exec.crud.selector-entry]`), with the three
> `DecodeRaw` methods taking the pair as their receiver, its text dereferencing to
> `SqlText`. Migration bodies are the last of them.

> [spec:pgorm:req:macros.sql.reject]
> `sql!` MUST refuse, at compile time, both an input that is not a single string literal
> and a literal the PostgreSQL grammar rejects. It MUST NOT panic on any input: a
> proc-macro panic surfaces as an unattributed compiler abort rather than a diagnostic
> the caller can act on, which is the same objection `[dec:pgorm:no-panic]` raises
> against panicking on a caller's mistake at runtime.
>
> Non-literal input is refused by `syn`'s own parse: bare tokens and non-string literals
> give `expected string literal` spanned at the offending token, and a literal followed by
> anything else gives `unexpected token` spanned at the trailing argument. The macro
> deliberately takes no bound parameters; `$1` placeholders are written in the SQL and
> their values supplied at the call site.
>
> A grammar rejection becomes `syn::Error::new(literal.span(), ..)` whose message is
> `PostgreSQL rejected this SQL: ` followed by the parser's own diagnostic, unwrapped from
> the `pg_query::Error` variant that would otherwise prefix it with a stage name. Input
> the parser cannot even be handed — a literal carrying an interior NUL byte, which fails
> the `CString` conversion — takes the same path and reports the conversion failure, so
> no input reaches an abort.

> [spec:pgorm:sem:macros.sql.script]
> A literal holding a `;`-separated script is validated whole: a single `pg_query::parse`
> call walks every statement in the string and reports the first one the grammar refuses,
> so a well-formed opening statement does not excuse a malformed later one. No separate
> split pass is involved — `pg_query::split_with_parser` rejects exactly what `parse`
> rejects, so a split-then-parse pipeline would carry a stage that can never fire.
>
> A literal that yields no statements at all — empty, whitespace-only, comment-only, or a
> bare `;` — is accepted, because PostgreSQL accepts it: the server answers an empty query
> string with an empty-query response. `sql!` reports on the grammar and nothing else, so
> it does not add a "must contain a statement" rule of its own.

> [spec:pgorm:req:macros.sql.ceiling]
> Two limits are part of the contract and MUST be stated on the macro rather than implied
> away.
>
> The check is syntax only. libpg_query carries no catalog, so unknown tables, unknown
> columns and misapplied type modifiers (`money(12, 2)`) all pass — the same ceiling the
> render oracle declares. `sql!` rules out typos in SQL, not mistakes about the schema: a
> literal it accepts can still fail against a live server, and the integration suite
> remains the semantic oracle.
>
> The error spans the whole literal, not the offending byte inside it. Narrowing to a
> sub-literal span needs `proc_macro::Literal::subspan`, which is nightly-only —
> `proc-macro2` compiles its call away and returns `None` on stable — so the position
> rides in the message instead: the offending line is reprinted with a caret under the
> column the parser named. libpg_query surfaces only the message and not a cursor offset,
> so that column is recovered by locating the token the message quotes, using the same
> last-occurrence heuristic as the render oracle; a message naming no token, such as
> `syntax error at end of input`, gets no caret.

## Compile-time PRQL compilation

`prql!` is the text sibling of the typed pipeline (`docs/spec/pipeline.md`): the same
prqlc compiler and the same oracle over what it emits, but for queries known whole at
compile time, where the pipeline's composability buys nothing and a literal buys
build-time checking of the finished statement.

> [spec:pgorm:def:macros.prql+1]
> `prql!` takes one PRQL string literal followed by zero or more argument expressions
> (`prql!("from invoice | filter total > $1 | take 5", min_total)`). While the calling
> crate compiles, the literal's text is compiled by prqlc — the text path: parse,
> resolve, lower, `rq_to_sql` with `Dialect::Postgres`, `no_format` / `no_signature`,
> the same staged compiler the runtime pipeline drives through its adapter — and the
> macro expands to the tuple `(&'static str, Values)`: the emitted SQL as a string
> literal spanned on the PRQL literal it came from, and a `pgorm::Values` (re-exported
> at the root for exactly this expansion) holding each argument converted through
> `Into<Value>`, in placeholder order. The expansion is a pure tuple expression — no
> runtime helper, no new public types — so `let (sql, values) = prql!(...)` lands
> directly on the raw-SQL entry points (`SelectorRaw::from_statement`,
> `ConnectionTrait::query_raw`), and a failed conversion is an ordinary type error
> spanned on the argument. The expansion names `::pgorm` absolutely, so the calling
> crate must depend on `pgorm` under that name. A placeholder may be reused: `$1`
> twice in the query is one argument, bound once, with the SQL carrying `$1` twice.
>
> The macro lives in `pgorm-sql-macro` beside `sql!` rather than in a third proc-macro
> crate: both macros end at the libpg_query oracle and share its diagnostic plumbing,
> and a crate split would buy a second build unit and nothing else. The compiling,
> the oracle and the census live in `pgorm-sql-check`, the ordinary library the
> run-time checks share (`[spec:pgorm:req:macros.runtime]`), which carries its own
> `prqlc` dependency on the fork, `default-features = false` — the same pinned
> revision as the root crate's runtime pipeline, the one sanctioned exception to the
> adapter's confinement rule (`[spec:pgorm:def:pipeline.adapter]`) — and the two
> pins MUST move in lockstep so the text macro and the typed pipeline emit through one
> compiler.

> [spec:pgorm:req:macros.prql.reject]
> Five mistakes MUST be refused at compile time, and the macro MUST NOT panic on any
> input — the same objection `[dec:pgorm:no-panic]` raises at runtime applies to an
> unattributed proc-macro abort:
>
> 1. PRQL the compiler rejects is refused with prqlc's own rendered diagnostic
>    (`DisplayOptions::Plain`, so no ANSI escapes reach the build log), spanned on the
>    literal.
> 2. Emitted SQL the PostgreSQL grammar rejects is refused with the parser's message
>    and the emitted SQL itself — the caller never wrote that text, so the message
>    always shows it, with a caret under the offending column when the parser names a
>    token (the `sql!` heuristic) and the bare statement when it does not.
> 3. Arity: exactly one argument per distinct `$N`. Too few is an error on the literal
>    naming every unbound placeholder (`nothing binds $2, $3`); too many is one error
>    per surplus argument, spanned on that argument, naming the `$N` the query does
>    not have.
> 4. Contiguity: the distinct placeholders MUST be exactly `$1..=$max`. A gap is
>    refused naming both ends (`the query uses $3 but never $2`), and `$0` — which the
>    grammar tolerates but no bind protocol can satisfy — is refused outright.
>    Contiguity is checked before arity, so a gap is reported as a gap rather than as
>    a miscount.
> 5. PRQL's own refusal of a parameterized limit — `take $N` does not compile —
>    surfaces at build time through channel (1), carrying prqlc's message
>    (`` `take` expected int or range, but found $1 ``).
>
> Input that does not start with a string literal is `syn`'s own refusal, as with
> `sql!`. When several refusals apply at once, every error is emitted: the expansion
> wraps them in a block so multiple `compile_error!` invocations stay legal in the
> expression position the tuple would have occupied.

> [spec:pgorm:sem:macros.prql.census]
> The placeholder census reads the *emitted SQL's* parse tree, not the PRQL AST: after
> the oracle accepts the statement, the `pg_query` tree is serialized and walked whole
> for `ParamRef` nodes, and the set of their numbers is the census. This is the only
> complete account of what the server will demand at prepare time — an s-string can
> carry a raw `$N` into the SQL with no `Param` node ever existing on the PRQL side,
> and it still costs an argument here, while `'$5'` inside a string literal is data
> and does not. The tree is walked generically (as serialized JSON) rather than
> through `pg_query`'s visitor, which by its own documentation covers only the node
> types its table-extraction cares about; a placeholder must not be able to hide in a
> clause the visitor skips.

> [spec:pgorm:sem:macros.prql.sstring]
> S-strings pass through: they are PRQL's raw-SQL interpolation hole, deliberately
> excluded from the typed pipeline's vocabulary but available to the macro's text as
> the caller's escape hatch. prqlc splices their contents into the emitted SQL
> verbatim, and the macro adds no screening of its own — the boundary is the oracle,
> which validates the *finished* statement, so an s-string that lands in well-formed
> SQL executes as written and one that breaks the statement is refused at build time
> by channel (2) of `[spec:pgorm:req:macros.prql.reject]`.

> [spec:pgorm:req:macros.prql.ceiling]
> The check ends where the catalog begins, and the limits MUST be stated on the macro.
> Neither prqlc nor libpg_query knows the schema: whether `total` is a column, whether
> `$1`'s slot accepts the value bound to it, and whether the output shape matches the
> model that decodes it are the server's questions, answered at prepare time and by
> `VerifyTrait::verify::<M>` (`[spec:pgorm:def:exec.verify]`) at runtime. And as with
> `sql!`, every refusal spans the whole literal (`subspan` is nightly-only); the
> position inside the text rides in the message — prqlc's own source frame for PRQL
> errors, the caret line for oracle rejections.

## Run-time checks

> [spec:pgorm:req:macros.runtime]
> The checks `sql!` and `prql!` make MUST also be offered at run time, for text a
> program has only once it runs and for the language bindings, which have no compile
> step: `pgorm::sql_check`, a re-export of `pgorm-sql-check`, an ordinary library the
> proc-macro crate calls too, so a check made either way runs the same code and
> refuses with the same text. `check_sql(text)` holds the text to the grammar as
> `sql!` holds a literal, a script statement by statement
> (`[spec:pgorm:sem:macros.sql.script]`), and refuses with a `Rejection`: the parser's
> message, the token it quotes when it quotes one, that token's byte offset, and the
> caret block; its `Display` is the message `sql!` reports. `compile_prql(text)`
> compiles PRQL as `prql!` does, holds the emitted SQL to the grammar, refuses a gap and
> `$0` (`[spec:pgorm:req:macros.prql.reject]`), and gives the SQL with the number of
> distinct placeholders it takes; `check_arity(given)` holds a count of values to them,
> refusing too few by the placeholders nothing binds and too many by the first
> placeholder the query lacks. Each refusal is a value, never a panic. The ceilings
> are the macros' (`[spec:pgorm:req:macros.sql.ceiling]`,
> `[spec:pgorm:req:macros.prql.ceiling]`): no catalog is consulted.
