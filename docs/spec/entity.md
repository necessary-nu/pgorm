# Entity system

The entity layer (`src/entity/`) defines the trait family that maps Rust types onto
PostgreSQL tables: entities, models, active models, columns, primary keys, relations,
links, and active enums. This spec captures what the code does today, including its
explicit limitations.

## Entity traits

> [spec:pgorm:def:entity.traits+4]
> The entity trait family is defined in `src/entity/` and re-exported from
> `pgorm::entity` (`src/entity/mod.rs`). Its base identifier contract is
> `pgorm_query::StaticName` (`[spec:pgorm:def:sql.types]`), re-exported from
> `base_entity.rs`: `SqlName + Copy + Debug + 'static` plus
> `as_str(&self) -> &str`. There MUST be exactly one such trait in the
> workspace. The entity layer used to declare a second of its own under a
> different name because the query layer's returned `&'static str`, which an
> entity's name — borrowed from `self` through `EntityName::table_name` —
> cannot be; the two are now one trait at the looser return, so a glob of both
> modules is unambiguous and implementing the wrong one is not a state.
> `EntityName: StaticName + Default` maps an entity to a table. `EntityTrait: EntityName`
> is the abstract entity, carrying five associated types: `Model`
> (`ModelTrait<Entity = Self> + FromQueryResult`), `ActiveModel`
> (`ActiveModelBehavior<Entity = Self>`), `Column` (`ColumnTrait`), `Relation`
> (`RelationTrait`), and `PrimaryKey`
> (`PrimaryKeyTrait + PrimaryKeyToColumn<Column = Self::Column>`).
>
> The trait family is produced by the derive macros exported from
> `src/entity/prelude.rs` (`Entity`, `Relation`, and friends), an
> entity having one written form (`[spec:pgorm:req:macros.derive.one-form]`). The
> traits are not sealed, so what is written by hand — the `EntityName` of an entity
> whose names are computed, or a key the arity guard of `entity.traits.crud` refuses —
> meets the same bounds.

> [spec:pgorm:req:entity.traits.entity-name+5]
> `EntityName::table_name` is the only required method and MUST return the table's SQL
> name. `schema_name` and `comment` default to `None`; `module_name` defaults to
> `table_name()`; `unique_keys` — the table's unique keys over two columns or
> more, static data as the table's name is: each a `UniqueKey` of the
> constraint's name or `None`, its columns' SQL names in key order and whether
> nulls are equal under it, which `create_table_from_entity` writes — defaults
> to none, and is what the composite derive's `unique(..)` keys declare
> (`macros.derive.entity-model.attrs`). `table_ref` MUST produce a
> schema-qualified `TableName`
> (`(Name::runtime(schema), entity)`) when `schema_name` returns `Some`, and a bare
> `TableName::Table` otherwise (`src/entity/base_entity.rs`). It is the DDL-position
> type, so schema projection targets take it directly; query positions widen it to a
> `FromItem` through `IntoFromItem`. All generated SQL that names the table goes
> through `table_ref`, so a `Some` schema name qualifies every statement.
> An entity handed straight to a builder's table position — `from`, a join, an
> `INSERT`, `UPDATE` or `DELETE` target, a DDL target — MUST be qualified the same
> way, so a raw statement reads and writes the table `find()` does and never a
> same-named one `search_path` finds first: the `SqlName` the `EntityTrait` derive writes
> answers `schema_name` as its `table_schema`, which `IntoTableName` reads
> (`[spec:pgorm:def:sql.types.table-ref]`). An entity whose `SqlName` is written
> by hand qualifies there only if it answers `table_schema` too.

> [spec:pgorm:req:entity.traits.crud+4]
> `EntityTrait` provides the static *read* surface (`src/entity/base_entity.rs`):
> `find()` returns a fresh `Select<Self>`; `find_by_id(values)` builds on `find()` by
> adding an equality filter per primary-key column, consuming the value tuple in
> primary-key iteration order. `delete_by_id(values)` returns a `DeleteMany`
> filtered per primary-key column like `find_by_id`; it stays because the
> per-column filter loop is real work, not a forward.
>
> `EntityTrait` MUST NOT carry `insert`, `insert_many`, `update`, `update_many`,
> `delete`, or `delete_many`. Each was a one-line forward to a builder
> constructor — `Insert::one`, `Insert::many`, `Update::one`, `Update::many`,
> `Delete::one`, `Delete::many` — and a second spelling of a constructor is a
> second thing to learn, a second place for docs to drift, and a second name
> in every reader's search. This follows the precedent of
> `entity.active-model.save`: where two spellings mean one thing, the surface
> keeps the one that says what it does. `Insert`, `TryInsert`, `Update`, and
> `Delete` are exported from `entity::prelude` so the surviving spelling is
> reachable wherever the deleted one was.
>
> `find_by_id` and `delete_by_id` MUST panic with `primary key arity mismatch` when the
> number of supplied values differs from the primary key's arity, in either direction.
> A value of another arity than the key's `ValueType` does not compile, so the panic is
> reached only through a hand-written `PrimaryKeyTrait` whose `ValueType` disagrees with
> its own variants.
> Values are accepted via `IntoPrimaryKey<<Self::PrimaryKey as PrimaryKeyTrait>::ValueType>`
> (`entity.traits.primary-key`), so composite keys are passed as tuples whose parts
> convert one by one: `find_by_id((1, "x"))` against an `(i32, String)` key, with no
> `.to_owned()`. These two are the only APIs that take a key value; a cursor boundary
> takes a value tuple over its order columns, not a key.

> [spec:pgorm:def:entity.traits.column+7]
> `ColumnTrait: StaticName + Iterable + FromStr` (`src/entity/column.rs`) describes one
> column of an entity. `def()` returns the column's `ColumnDef`; `entity_name()` and
> `as_column_ref()` qualify the column with its `EntityName`. The trait exposes an
> expression-building surface wrapping `pgorm_query::Expr`: comparison operators `eq`,
> `ne`, `gt`, `gte`, `lt`, `lte`; range `between` / `not_between`; pattern matching
> `like`, `not_like`, and the sugar `starts_with`, `ends_with` and
> `contains`, whose search text is LITERAL: `%`, `_` and `\` in it are
> escaped before the pattern is built (`s%`, `%s`, `%s%`), so
> `contains("%")` matches a percent sign rather than every row — a pattern
> is `like`'s job and `like`/`not_like` take theirs verbatim; aggregates `max`, `min`, `sum`, `count`; null checks `is_null`,
> `is_not_null`, `if_null`; set membership `is_in` / `is_not_in`, its
> array-parameter counterpart `eq_any` / `ne_all`
> (`[spec:pgorm:req:sql.ast.expr.eq-any]`), and subqueries
> `in_subquery` / `not_in_subquery`; plus `into_expr` and `into_returning_expr`.
> The comparison and set-membership operators pass their values through `save_as`, so
> enum-typed columns compare against properly cast values; `eq_any` / `ne_all` pass
> their single array value through `save_array_as` instead, the whole array being one
> operand.
>
> The comparison operators above take a *value*, bound by `Into<Value>`. Comparing
> against another column, or against a computed expression, is a separate named
> family: `eq_col`, `ne_col`, `gt_col`, `gte_col`, `lt_col` and `lte_col` take any
> `ColumnTrait` and render `"a"."x" <op> "b"."y"`, each side qualified by its own
> entity; `eq_expr` takes any `Into<SimpleExpr>` for the computed case. `eq` and its
> siblings MUST NOT be widened to admit an expression operand: their operand goes
> through `save_as`, which is what casts an enum value, and a widened bound would drop
> that cast silently. The `_col` and `_expr` forms deliberately do not apply `save_as` —
> a column or an expression is already typed on the server side, and an enum column
> compared against another column of the same enum type needs no cast.
>
> `json_key()` names the key the column occupies in a JSON object, which is a
> different namespace from the SQL name `StaticName::as_str` gives: the `with-json`
> conversions of `[spec:pgorm:req:entity.active-model.json]` deserialize through the
> entity's `Model`, so the key is the model field's name as `serde` spells it.
> The `Entity` derive MUST emit it from the field the column was derived from and the
> `serde` renames declared on that field and its struct; the trait's default falls back
> to the SQL name, which is what a hand-written `Column` — having no field to read —
> can offer and what the two namespaces agree on when nothing is renamed.
>
> `ColumnType` is a re-export of `pgorm_query::ColumnType`; the crate's own `ColumnType`
> enum was dropped and `ColumnTypeTrait` (`def()`, `get_enum_name()`) bridges a
> `ColumnType` or existing `ColumnDef` into a `ColumnDef`.

> [spec:pgorm:req:entity.traits.column-def+2]
> `ColumnDef` (`src/entity/column.rs`) carries a column's definition attributes:
> `col_type: ColumnType`, `null`, `unique`, `indexed`, `default`, and
> `comment: Option<String>`. `ColumnTypeTrait::def()` MUST initialise a definition
> as non-null, non-unique, non-indexed, with no default and no comment. Builder methods
> flip individual attributes: `unique()`, `indexed()`, `null()` / `nullable()` (aliases),
> `comment(v)`, `default_value(T: Into<Value>)`, `default(T: Into<SimpleExpr>)`
> (the latter accepting arbitrary expressions), `identity()` /
> `identity_by_default()` (`GENERATED ALWAYS` / `BY DEFAULT AS IDENTITY`), and
> `generated(expr, kind)` (`GENERATED ALWAYS AS (<expr>) { STORED | VIRTUAL }`, the
> kind a `GeneratedKind` that is always stated, `[spec:pgorm:req:sql.ddl.column-def]`).
> `get_column_type()` and `is_null()` expose the type and nullability for introspection.
>
> `default` is one slot, `Option<ColumnDefault>`, holding a default expression, an
> identity form or a generation expression and its kind, never two: PostgreSQL refuses
> a column declaring any pair (42601, "both default and identity specified", "both
> default and generation expression specified", "both identity and generation
> expression specified"), so a definition holding two would describe no table. The
> builders writing it replace whatever it held. There is no sequence-option form: an
> entity declares which form generates a column, not how its sequence counts.
>
> A generated column is read like any other — a model holds its computed value,
> `RETURNING` included — and written by nobody. Nothing in the write paths treats it
> specially: a model writes it by leaving it `NotSet`, and an insert carrying a value
> for it (`Set` or `Unchanged`, both of which an insert writes) or an update that
> `Set`s it is refused by the server (`428C9`), exactly as a value for a
> `GENERATED ALWAYS` identity column is. The two kinds
> behave alike there; they differ in what the schema can say about them, since
> PostgreSQL refuses a virtual column that is `unique` or `indexed` (`0A000`) — a
> refusal the derive makes at compile time
> (`[spec:pgorm:sem:macros.derive.entity-model.column-def]`) and a hand-written
> definition meets when its schema is created.

> [spec:pgorm:sem:entity.traits.column.enum-cast+14]
> Enum-typed columns are transparently cast at the SQL boundary
> (`src/entity/column.rs`). On read, `select_as` / `select_enum_as` casts an enum
> column to `text` — or `text[]` when the column type is `Array` of an enum — and
> leaves non-enum columns untouched. On write, `save_as` / `save_enum_as` casts the
> value to the enum's database type name, carried as the structured
> `TypeName` (`sql.types.type-name`): schema qualification and the array
> flag ride the structure, and the parts render bare only when they are
> lowercase identifiers that are no restricted keyword — never as raw SQL
> text. As a special
> case under the `with-json` + `postgres-array` features, saving into a `Json` /
> `JsonBinary` column flattens a `Value::Array` of JSON values into a single
> `Value::Json` array value instead of applying an enum cast.
>
> A column of a range type a schema created (`ColumnType::CreatedRange`) takes
> the same write path through `Expr::as_range`
> (`[spec:pgorm:def:sql.value.created-range]`): the value — the text a
> `CreatedRange` newtype converts into, or a built-in range value, which
> is written as its text first — is cast to the type by its structured
> `TypeName`, there being no cast to it from a built-in range type. On read
> it is left untouched: the range's binary form decodes as it is. A column of
> the multirange created beside such a range is written the same way and read
> as an enum column is, `select_as` casting it to `text`: tokio-postgres
> reports that multirange as a simple type whose binary form it cannot read.
>
> A `money` column (`ColumnType::Money`) is read and written as `numeric`,
> which a `Decimal` field binds and decodes: on the wire a `money` is an
> `int8` count of the currency's minor units, whose scale only the server's
> `lc_monetary` knows and which `rust_decimal` does not read. `select_as`
> casts the column to `numeric`, `numeric[]` for an array of `money`, and
> `save_as` casts a value to `money`, `save_array_as` an array operand to
> `money[]`; a value's placeholder is pinned to its own type inside the cast
> (`sql.render.cast-param-type`), so a `Decimal` arrives as `numeric` and an
> integer as the whole currency units PostgreSQL's integer cast reads it as.
> The server converts at the session's scale both ways, a `Decimal` with more
> places than the currency rounded to its places. A value is cast to `money`
> rather than left `numeric`, which a column assignment would take but no
> comparison resolves (`42883`, *operator does not exist: money =
> numeric*). The live round trip of a generated entity's `money` column and
> array of `money`, inserted, read back whole and by column, found by each
> comparison form and updated through `RETURNING`, is held at the workspace
> root (`tests/codegen_column_types_tests.rs`).
>
> A column whose type is a domain (`ColumnType::Domain`) is read and written
> as its base is, and cast, where its base needs a cast, to the domain by its
> own name. A domain over an enum — or over an array of one, or over another
> such domain — is read as `text`, or `text[]` when what it holds is an
> array, and a value written to it is cast to the domain, its array to the
> domain's array; a domain over a created range or multirange is written
> through `Expr::as_range` to the domain and its multirange read as `text`;
> a domain over `money`, or an array of one, is read as `numeric` and a value
> written to it cast to the domain, its array to the domain's array, so the
> domain's checks hold the value as its column does.
> Every other domain needs no cast at all: its values travel as its base
> type's (`exec.cursor.binding-accepts`), and a comparison with one resolves
> its base type's operator.
>
> A column whose domain is over an enum, directly or through further
> domains, is the exception in comparisons: PostgreSQL resolves no operator
> for it, not even between two values of the domain (`42883`, *operator does
> not exist: d = d*), so every comparison `ColumnTrait` builds casts the
> column to the enum by its structured `TypeName`, and casts each value
> operand to the enum in place of `save_as` — an array operand (`eq_any`,
> `ne_all`) to the enum's array in place of `save_array_as`. The value is
> compared, not written, so it is not cast to the domain and the domain's
> constraints do not apply to it, as they apply to no comparison with any
> other domain's column, whose value travels as its base type's. The
> column-taking forms (`eq_col` and its siblings) cast each column so typed,
> either side, and `eq_expr` casts its own. A domain over an array of an enum
> compares as the domain, PostgreSQL's array comparisons resolving for it, and
> an array of such a domain compares as that array, its value cast to it.
>
> The same holds wherever pgorm writes a comparison with such a column.
> `in_subquery` / `not_in_subquery` cast the column, and the subquery's one
> projected expression under the alias it had: the enum resolves no operator
> with the domain either (`42883`, *operator does not exist: e = d*), so a
> subquery projecting the same domain needs the cast as much as the column
> does. A projection the cast cannot reach in place — `*` or a table's `*`,
> whose cast is not SQL, or a window function's, which pgorm-query pairs with
> its `OVER` — is compared through the subquery's array instead:
> `= ANY(CAST(ARRAY(..) AS e[]))` for `in_subquery`, `<> ALL(..)` for
> `not_in_subquery`, which answer as `IN` and `NOT IN` do, a null member
> included (`sql.ast.expr`). The server refuses `ARRAY(..)` of more than one
> column (`42601`) as it refuses `IN` of one, so a `*` wider than a column is
> refused, never narrowed to its first column, as wrapping it in a derived
> table whose alias list named one column would narrow it. The array is the
> subquery's whole result, built once rather than planned as a semi-join,
> which is the cost of these shapes alone. Several projected expressions are
> left as written: the membership is of one column, so the server refuses
> them (`42601`, *subquery has too many columns*) whatever their types, and no
> cast could make them a comparison. `max` and `min` aggregate the column cast to the enum and
> yield the enum, PostgreSQL resolving neither for the domain (`42883`,
> *function max(d) does not exist*); `count` resolves for the domain and keeps
> the bare column, and `sum` resolves for no enum at all. A relation's join
> condition casts each side whose column is so typed (`query.build.join`): a
> `ColumnPairs` named by entity columns, as `RelationBuilder::columns` and
> `and_columns` name them, records at construction the enum each side
> compares as, and `rev()` swaps the records with the pairs; one named by
> bare names (`ColumnPairs::new`, `and`, `push`) has no column type to read,
> records none, and joins bare. The loader's batch predicate casts such a key
> column and its values (`query.loader.batching`), and
> `belongs_to_tbl_alias`, which scopes `find_linked` to one model, compares
> each key column as `belongs_to` does. PostgreSQL implements no foreign key
> with a column of such a domain on either side (`42804`, *incompatible
> types*), not even with the enum it is over, so `create_table_from_entity`
> refuses a relation over one rather than write its key
> (`schema.from-entity`): such a relation is a join with no key behind it.
>
> A pipeline's entity column so typed carries the enum it compares as
> (`pipeline.qualify`), and each `ExprOps` operator applied to it — the
> comparisons, `in_array`, its operand and each member, `coalesce` and
> `is_null` among them — casts every side that carries one, as the `min` and
> `max` aggregates cast their argument; sorting, grouping, projecting and
> `count` keep the bare column. The type comes from the entity wherever one
> is known: a relation read under a name of its own keeps it, its columns
> written `Named::col(column)`, qualified by the name and carrying what the
> entity column carries. Where none is — `col(table, column)` over a table no
> entity describes, an alias token naming a projection — the column compares
> as the type the server gives it, the domain, which the server refuses
> (`42883`), until `compares_as(enum)` names the enum it compares as, a
> structured `TypeName` schema-qualified where the enum is; the operators then
> cast it as they cast an entity column so typed. The language bindings'
> pipelines take the same two routes: a registered entity's column read
> under a qualifier (`col(table, entity.col(name))`) lowers through the
> registration's own `Named::col`, and `compares_as` names the enum where no
> registration types the column (`bindings.pipeline-sources`,
> `bindings.pipeline-expressions`). A binding's registered column is a
> member — of a list of values, each converted to its declared kind, or of a
> subquery's rows — through its `ColumnTrait`'s `is_in`, `is_not_in`,
> `in_subquery` and `not_in_subquery`, so it is cast as Rust's is
> (`bindings.entity-reads`); a statement builder's own expression is SQL as
> written, and is cast by the caller where it needs one.
>
> Every value-position operand a `ColumnTrait` predicate takes passes through
> `save_as`, but under a column whose domain is over an enum, where it is cast
> to the enum: the scalar comparators, `between` / `not_between`, `if_null`,
> and `is_in` / `is_not_in` per element. A value predicate that binds an
> operand bare is a defect — the column-taking (`eq_col`) and expression-taking
> (`eq_expr`) forms are the deliberate exceptions, since their operands are not
> values. The keyset cursor's boundary values and order columns follow the
> same discipline (`exec.cursor.keyset`).
>
> `save_array_as` / `save_enum_array_as` are the array counterpart of `save_as` /
> `save_enum_as`, for the operands that are one array value rather than one value
> per element (`eq_any`, `ne_all`): the default casts to `{enum_name}[]` when the
> column's type is an enum or an array of one, and leaves every other column
> untouched. It does not take the JSON-flattening path — flattening an array into
> a scalar would destroy the operand `= ANY` needs. A column that overrides
> `save_as` with a cast of its own also overrides `save_array_as` with the array
> spelling of the same type: `#[pgorm(save_as = "…")]` generates both, so the
> scalar and array comparisons of one column cannot disagree about its cast.

> [spec:pgorm:def:entity.traits.primary-key+7]
> `PrimaryKeyTrait: StaticName + Iterable` (`src/entity/primary_key.rs`) defines an
> entity's primary key as an iterable enum of key columns. Its `ValueType` associated
> type is the Rust value form of the whole key and is bound by
> `Sized + Send + Debug + PartialEq + IntoValueTuple + TryFromValueTuple
> + TryGetableMany + TryFromU64 + PrimaryKeyArity + IntoPrimaryKey<Self::ValueType>`;
> `auto_increment()` reports whether the
> database generates the *whole* key, so an insert naming no key column still writes a row
> with one: a one-column key the serial family (or the column's own default, identity or
> generation expression) fills, or a composite key every column of which is an identity or
> a stored generated column. A composite key with one
> generated column — `(tenant_id, id)`, `id` an identity — reports false, because the
> caller still supplies `tenant_id`; the generated part is that column's fact, carried by
> its `ColumnDef` (`entity.traits.column-def`). `without_overlaps()`, `false`
> unless a key overrides it, says the key is PostgreSQL 18's temporal key,
> its last column the period compared for overlap where the rest are compared
> for equality (`[spec:pgorm:req:sql.ddl.create-table]`). The key value is
> still the whole key, period included, and a lookup by it is by equality:
> equal periods overlap and an empty one is refused (`23514`), so at most one
> row holds a value of the key, and `find_by_id`, an update and a delete by
> key name one version of a row; which version covers a moment is a filter on
> the period, not a key lookup. The live suite holds the key generated
> `WITHOUT OVERLAPS`, refusing an overlapping version (`23P01`) and admitting
> one that only touches, a lookup by an overlapping but unequal period naming
> no row, and an update and a delete by key reaching one version.
> `PrimaryKeyToColumn` maps key variants to columns (`into_column`)
> and back (`from_column -> Option<Self>`). `PrimaryKeyArity` exposes a
> `const ARITY: usize`: any single `TryGetable` scalar has arity 1, and tuple impls
> cover composite keys of 1 through 12 components.
>
> `IntoPrimaryKey<V>` is the conversion a key value is taken through, part by part:
> anything `Into<V>` for a one-column key (`V: TryGetable`), and for a composite key a
> tuple of the same arity, 1 through 12, whose part `i` is `Into` the key's part `i`.
> `Into` itself cannot carry this — `(i32, &str)` has no `Into<(i32, String)>`, and
> std would need an impl per pair of tuple types — so a lookup against an
> `(i32, String)` key used to need `"x".to_owned()` where the one-column key took
> `"x"`. The two impl families cannot overlap: no tuple is `TryGetable`. A tuple of the
> wrong arity has no impl, so it is a compile error rather than the runtime arity
> panic. `ValueType` carries the bound itself, which every value type satisfies by
> these impls, so generic code holding a `ValueType` can pass it where a key is taken.
> The impls are one per arity, with no per-type expansion: `cargo check` of the
> integration-test crate after touching the library took 53.3 s of user time before
> them and 52.7 s after.

> [spec:pgorm:def:entity.traits.model+3]
> `ModelTrait: Clone + Send + Debug` (`src/entity/model.rs`) is the read-side row
> representation. `get(column)` returns the column's `Value`;
> `set(column, value) -> Result<(), Error>` writes it, reporting a column this
> model does not carry, or a value whose type does not match the field, as
> `Error::Type`. `find_related(R)` returns a `Select<R>` scoped to this instance via
> `Related::find_related().belongs_to(self)`; `find_linked(L)` scopes a multi-hop
> `Linked` join to this instance using the last hop's `r{n}` table alias.
> `into_active(self)` converts the model into
> `<Self::Entity as EntityTrait>::ActiveModel` by delegating to `IntoActiveModel`;
> because the destination is fixed by the entity rather than by a type parameter, the
> call needs no annotation where `into_active_model()` does.
> `delete(self, db)` converts the model through `IntoActiveModel` and delegates to
> `ActiveModelTrait::delete`, so behavior hooks run. `TryIntoModel<M>` is the fallible
> reverse conversion with a blanket identity impl for any model.

> [spec:pgorm:def:entity.traits.from-query-result+5]
> `FromQueryResult` (`src/entity/model.rs`) instantiates a type from a `QueryResult`
> row given a column-name prefix: `from_query_result(res, pre)`.
> `from_query_result_optional` reads a row that may not carry the type at all —
> the related side of an outer join — answering `Ok(None)` for an absent row and
> propagating every other decode failure, against the witness `exec.decode.absent`
> defines. `find_by_statement(stmt, values)` builds a
> `SelectorRaw<SelectModel<Self>>` for running raw SQL into typed rows.
> `expected_columns` reports the columns `from_query_result` reads, so a statement can
> be checked against the type before a row exists (`exec.verify`) and so the optional
> decode knows which columns witness an absent row; it defaults to `None`, the answer
> of an implementation that does not report them.
> `PartialModelTrait: FromQueryResult` (`src/entity/partial_model.rs`) adds
> `select_cols<S: QuerySelect>(S) -> S::Projected`, letting a partial model declare
> exactly the columns it needs on a select. The return type is the *projected* state,
> not `S`, so an implementation that selects no column cannot typecheck: a field-less
> `PartialModel` struct is a compile error rather than a query with an empty
> projection (`query.build.modifiers`).

> [spec:pgorm:def:entity.traits.active-enum+2]
> `ActiveEnum: Sized + Iterable` (`src/entity/active_enum.rs`) maps a Rust enum onto a
> database value. `Value` is the backing Rust type and must implement `ActiveEnumValue`
> (`Into<Value> + ValueType + Nullable + TryGetable`). `name()` returns the database
> enum's identifier as a `Name`; `to_value` / `into_value` convert a variant to its
> database value; `try_from_value` performs the fallible reverse mapping, returning
> `Error` for unknown values; `db_type()` returns the column definition used for the
> enum column. `as_enum()` wraps a value expression in a cast to the enum's type name,
> and `values()` enumerates every variant's database value in iterator order. The
> `ValueVec` associated type has no purpose and is documented for removal.

> [spec:pgorm:req:entity.traits.active-enum.limits+2]
> `ActiveEnumValue` is implemented for `String`, `i8`, `i16`, `i32`, `i64` and `u32`
> only (`src/entity/active_enum.rs`). `try_get_vec_by` — reading an array of enum
> values, a Postgres-only capability — MUST NOT panic when it cannot decode: it
> MUST return `TryGetError::Db(Error::Type(_))` for `u32` (not supported by
> `postgres-array`), and likewise for the other types when the `postgres-array`
> feature is disabled. The blanket `TryFromU64` impl for every `ActiveEnum` MUST
> return `Error::ConvertFromU64`, so a primary key containing an active-enum field
> MUST declare `auto_increment = false` to be usable.

## Active model

> [spec:pgorm:req:entity.active-model+4]
> `ActiveModelTrait: Clone + Debug` (`src/entity/active_model.rs`) is the write-side
> row representation whose fields are `ActiveValue`s. Implementations MUST provide
> per-column state access: `get` (a non-consuming read), `take` (removes and returns,
> leaving `NotSet`), `set` (stores a `Value` as `Set`), `not_set` (clears to
> `NotSet`), `is_not_set`, `reset` (per-column `Unchanged` → `Set`), and `default()`
> (all columns `NotSet`). `reset_all` applies `reset` to every column.
>
> The three accessors that name a column and a value — `get`, `take` and `set` —
> are fallible in the same way and for the same reason: a column the model does
> not carry MUST come back as `Error::Type` rather than as a value. `get` and
> `take` therefore return `Result<ActiveValue<Value>, Error>` and `set` returns
> `Result<(), Error>` (which also reports a value of the wrong type for the
> field). Answering an unknown column with `NotSet` is specifically forbidden:
> "this model has no such column" and "this column holds no value" are different
> facts about different things, and a caller that cannot tell them apart cannot
> detect the first at all.
>
> The error is unreachable through a derived ActiveModel, whose fields are its
> entity's columns by construction (`[spec:pgorm:sem:macros.derive.active-model]`);
> it exists for hand-written implementations, which can disagree with their
> `Entity`. pgorm's own callers, all of which iterate the entity's own columns,
> therefore either propagate it (`Update::one`'s filter and value preparation,
> `DeleteOne`'s filters, `set_from_json`, `from_json`) or treat the column as
> carrying nothing, exactly as `NotSet` does (`Insert::add`, `UpdateMany::set`,
> `is_changed`, `get_primary_key_value`) — the latter where the method's own
> signature admits no error, and never where the caller could act on one.
>
> `get_primary_key_value`
> reads `PrimaryKeyArity::ARITY` values, in primary-key iteration order, into one
> `ValueTuple` (`[spec:pgorm:def:sql.value.tuple]`) whatever the arity, and
> MUST return `None` if any key component is `NotSet`.
> `is_changed` returns `true` when any attribute is in the `Set` state.

> [spec:pgorm:def:entity.active-model.active-value+1]
> `ActiveValue<V: Into<Value>>` (`src/entity/active_model.rs`) is a three-state
> machine over a column value: `Set(V)` (a value actively being written),
> `Unchanged(V)` (a value loaded from the database and not modified), and `NotSet`
> (no value). `Default::default()` is `NotSet`, and the `NotSet` variant is re-exported
> at crate root so `ActiveModel { field: NotSet, .. }` reads naturally. Only `Set`
> values participate in generated `INSERT`/`UPDATE` column lists; `Unchanged` primary
> keys still drive `WHERE` clauses. `PartialEq` compares equal only for identical
> variants with equal payloads.
>
> The `Set` state has three spellings, and documentation and examples MUST use the
> free `set(value)` function (`[spec:pgorm:req:entity.active-model.from-sugar]`) for
> construction: it is the only one that converts into the column type. The `Set(v)`
> variant and the `ActiveValue::set(v)` associated constructor both pin `v` to `V`
> exactly, and remain the spelling where the variant itself is the subject — pattern
> matching, where no function call can appear, and the cases where `V` is not
> inferable from context.

> [spec:pgorm:sem:entity.active-model.active-value.ops]
> `ActiveValue` accessors (`src/entity/active_model.rs`): the constructors `set`,
> `unchanged`, `not_set` and the predicates `is_set`, `is_unchanged`, `is_not_set`
> mirror the three variants. `take(&mut self)` removes the value — returning
> `Some(value)` for `Set` or `Unchanged`, `None` for `NotSet` — and leaves
> `NotSet` behind; its documentation MUST say so rather than describe it as a
> mutable read, which is what it is not. `unwrap(self)` and `as_ref(&self)` return
> the inner value and panic on `NotSet`; `try_as_ref` is the non-panicking form
> returning `Option<&V>`. `into_value` consumes the `ActiveValue` and yields the
> `Value` it held as `Option<Value>`; `into_wrapped_value`
> converts to `ActiveValue<Value>` preserving the variant. `reset` promotes
> `Unchanged` to `Set` and leaves `NotSet` untouched. `set_if_not_equals(value)`
> assigns `Set(value)` unless the current state is `Unchanged` with an equal payload,
> in which case it does nothing — making `is_changed` reflect actual differences.

> [spec:pgorm:req:entity.active-model.from-sugar+1]
> pgorm provides a blanket `impl<V: Into<Value>> From<V> for ActiveValue<V>`
> (`src/entity/active_model.rs`) so any column value converts into `ActiveValue` with
> plain `.into()`, without writing `ActiveValue::Set(...)` — an ergonomic divergence
> from upstream SeaORM. The conversion MUST produce the `Set` state. Additionally,
> `From<ActiveValue<V>> for ActiveValue<Option<V>>` MUST lift a value into a nullable
> column position while preserving the variant (`Set(v)` → `Set(Some(v))`,
> `Unchanged(v)` → `Unchanged(Some(v))`, `NotSet` → `NotSet`).
>
> The blanket pins the source type to the field type exactly, so a borrowed value has
> no route into an owning column: `&str` reaches only `ActiveValue<&str>`, never
> `ActiveValue<String>`. Three targeted conversions close that gap —
> `From<&str> for ActiveValue<String>`, `From<&str> for ActiveValue<Option<String>>`,
> and `From<&[u8]> for ActiveValue<Vec<u8>>`. Each names a target the blanket does not
> produce for the same source, so they cohere with it, and each MUST produce the `Set`
> state.
>
> The free function `pub fn set<V: Into<Value>, T: Into<V>>(value: T) -> ActiveValue<V>`
> is the `Set`-shaped counterpart of those conversions: it applies `Into<V>` at the
> call site, so `set("Apple")` reaches an `ActiveValue<String>` and no ActiveModel
> field needs a `.to_owned()`. Neither the `Set` variant nor the `ActiveValue::set`
> associated constructor can offer this — a variant admits no conversion, and the
> associated form is bound by the `V` of the impl block. `set` is exported at the crate
> root and is the spelling documentation uses
> (`[spec:pgorm:def:entity.active-model.active-value]`).

> [spec:pgorm:req:entity.active-model.persistence+2]
> `ActiveModelTrait::insert` MUST execute via `Insert::exec_returning_model`, so on
> PostgreSQL the insert and the returned `Model` are a single `INSERT ... RETURNING`
> round trip. `ActiveModelTrait::update` executes
> `Update::one(am).exec_returning_model(db)`
> (an `UPDATE ... RETURNING` statement keyed on the primary key) and likewise returns
> the fresh `Model`. `ActiveModelTrait::delete` deletes by the model's primary key and
> returns the rows-affected count as `u64`. All three are async and generic over any
> `ConnectionTrait` (`src/entity/active_model.rs`). `insert` and `update` are the
> trait's only write entry points, and which of them runs is the caller's stated
> choice rather than a property of the model
> (`[spec:pgorm:req:entity.active-model.save]`). Both accept a primary key in
> either the `Set` or the `Unchanged` state.

> [spec:pgorm:req:entity.active-model.save+1]
> `ActiveModelTrait` MUST NOT carry a `save` operation that infers insert-versus-update
> from the primary-key state. The inherited inference — `insert` when at least one key
> column is `NotSet`, `update` otherwise — reads a distinction the input does not carry:
> `Set` and `Unchanged` are both "holds a value", so an entity with a manually assigned
> key, whose caller must populate that key to name the row at all, could never reach
> `insert` through it, and creating such a row was possible only by bypassing the
> ActiveModel API. Intent that is unrepresentable in the input is stated by the caller
> instead: `insert` and `update` (`[spec:pgorm:req:entity.active-model.persistence]`)
> are separate entry points, and insert-or-update in a single statement is expressed
> explicitly through `Insert::on_conflict`. The removal is deliberate and `save` MUST
> NOT be reintroduced under this or another name (`src/entity/active_model.rs`).

> [spec:pgorm:req:entity.active-model.hooks+1]
> `ActiveModelBehavior: ActiveModelTrait` (`src/entity/active_model.rs`) defines
> lifecycle hooks with pass-through defaults returning `Ok`. Ordering is fixed:
> `insert` MUST call `before_save(self, db, insert: true)` before executing and
> `after_save(model, db, true)` on the returned model; `update` does the same with
> `insert: false`; `delete` MUST call `before_delete(self, db)` before executing and
> `after_delete` (on a clone of the pre-delete active model) after. `new()` defaults to
> `ActiveModelTrait::default()` and is the hook for constructing an active model with
> default values.
>
> An `Err` from a *before* hook aborts the operation: it returns before the statement
> is built, so nothing reaches the database. An `Err` from an *after* hook is returned
> to the caller but MUST NOT be read as an abort — the statement has already executed
> on the connection, and the row stays written or stays deleted. The three entry points
> run their hooks and their statement on the `&C` they were handed and MUST NOT open a
> transaction of their own: a write the caller asked for on a plain connection is not
> silently widened into one, and the caller who wants the hook and the write to stand
> or fall together passes a `DatabaseTransaction` as that `&C` and lets the `Err`
> reach the rollback. The Rust docs on `ActiveModelBehavior`, `insert`, `update` and
> `delete` MUST state this asymmetry rather than promise an abort the code does not
> perform.

> [spec:pgorm:req:entity.active-model.into+3]
> `IntoActiveModel<A>` converts a type into an active model and has a blanket identity
> impl for any `ActiveModelTrait`; derived models convert `Model` → `ActiveModel` with
> every field `Unchanged` (used by `Model::delete`, `set_from_json`, and by callers
> turning a returned `Model` back into an active model to update it).
> `IntoActiveValue<V>` governs how an `IntoActiveModel` struct's fields become states:
> `Option<V>` MUST map `Some(v)` → `Set(Some(v))` and `None` → `NotSet`;
> `Option<Option<V>>` MUST map `Some(inner)` → `Set(inner)` (allowing an explicit
> `Set(None)` to null a column) and `None` → `NotSet`; the plain scalar impls
> (`bool`, integer and float primitives, `&'static str`, `String`, `Vec<u8>`, the
> feature-gated `Json`/date-time/`Decimal`/`Uuid` types, and the `Range` and
> `Multirange` of each built-in subtype, those over jiff types under
> `with-jiff`) MUST produce `Set` (`src/entity/active_model.rs`).

> [spec:pgorm:req:entity.active-model.json+4]
> Under the `with-json` feature, `ActiveModelTrait::from_json` builds an active model
> by deserializing the JSON object into the entity's `Model` (errors surface as
> `Error`), converting it with `IntoActiveModel`, then normalizing states per column:
> attributes whose key exists in the JSON object become `Set`, and all others MUST be
> `NotSet` (`src/entity/active_model.rs`).
>
> Presence detection and deserialization MUST read the same key namespace, and that
> namespace is `serde`'s: the key a column occupies is the model field's name as
> `serde` spells it, read from `ColumnTrait::json_key`
> (`[spec:pgorm:def:entity.traits.column]`) — never the SQL column name, which the
> deserializer never sees. The `Entity` derive computes each column's key from the
> field it derived that column from, applying `#[serde(rename = "..")]` and
> `#[serde(rename_all = "..")]` (the deserialize half where the split form is used);
> `#[pgorm(column_name)]` and `#[pgorm(rename_all)]` name the SQL column and MUST NOT
> take part. A model `serde` serializes therefore round-trips back through `from_json`
> however either side is renamed, where reading SQL names would have silently dropped
> every renamed column to `NotSet`. A hand-written `Column` has no field to read and
> keeps the SQL name as its key — the name the two namespaces agree on when nothing is
> renamed.
>
> `set_from_json` applies the same conversion in place, and MUST leave the active
> model untouched when it fails: the JSON is converted into a whole new active model
> first, and a conversion that returned `Err` MUST NOT have written anything to
> `self` — in particular MUST NOT have cleared the primary key, which would leave a
> caller holding a model that can no longer name its row. On success it MUST NOT alter
> the primary-key values either: they are read off `self` and put onto the replacement
> via `set()`, so a `Set` or `Unchanged` key keeps its value but comes back in the
> `Set` state (an `Unchanged` key is upgraded), while `NotSet` keys stay `NotSet`.

## Relations

> [spec:pgorm:req:entity.relation+1]
> Relations are declared per entity through `RelationTrait: Iterable + Debug`, whose
> `def(&self) -> RelationDef` maps each variant of the entity's `Relation` enum to a
> definition (`src/entity/relation.rs`). `RelationType` has exactly two variants:
> `HasOne` and `HasMany` — belongs-to is expressed as ownership direction, not a third
> type. `EntityTrait::belongs_to(related)` MUST start a builder with `HasOne` and
> `is_owner = false`; `EntityTrait::has_one(R)` and `has_many(R)` require
> `R: Related<Self>` and derive their builder from the reversed related entity's
> definition (`R::to().rev()`) with `is_owner = true` — but the builder keeps only
> the tables and columns of that reversed definition: its `on_delete`/`on_update`
> actions, condition hooks and `fk_name` are discarded (`RelationBuilder::from_rel`),
> so FK actions declared on the belongs-to side do not surface on the reversed
> relation. The `Related<R>` trait exposes
> `to()`, `via()` (defaulting to `None`; `Some` denotes a junction-table hop), and
> `find_related()`, which MUST inner-join `to()` (and `via()` when present, joined in
> reverse) onto a fresh `Select<R>`.

> [spec:pgorm:def:entity.relation.def+12]
> `RelationDef` (`src/entity/relation.rs`) is the concrete relation record:
> `rel_type`, `from_tbl` / `to_tbl` (`FromItem`, since a relation is joined into a
> query and may be re-aliased), `columns` (`ColumnPairs`),
> `is_owner`, optional `on_delete` /
> `on_update` foreign-key actions (`pgorm_query::ForeignKeyAction`), an optional
> boxed `on_condition` closure receiving the left and right join names, an optional
> `fk_name`, an optional `enforcement` (`pgorm_query::Enforcement`) and
> `deferrability` (`pgorm_query::Deferrability`) its foreign key is declared
> with, and a `condition_type` (`All` = AND, `Any` = OR). `rev()` swaps the
> from/to tables and columns, negates `is_owner`, clears `fk_name`, and keeps the
> remaining attributes; an attached `on_condition` keeps its authored roles, its
> arguments re-swapped along with the tables, so reversing a def MUST NOT change
> what a predicate written for `(source, target)` receives. A closure attached
> after reversing is authored against the reversed roles. `from_alias(alias)` re-points `from_tbl` at a table alias for
> self-join disambiguation; `on_condition(f)` replaces any existing custom condition;
> `condition_type(t)` sets how the ON clauses combine.
>
> `ColumnPairs` (`src/entity/key.rs`) is the column set a relation joins on,
> held as a first `(from, to)` pair plus any further pairs. The two sides of a
> relation are therefore one value, not two: the only constructor,
> `ColumnPairs::new(from, to)`, takes a pair, `and(from, to)` / `push(from, to)`
> extend by a pair, and `rev()` swaps within each pair. A set of join columns is
> consequently non-empty and equal-sided by construction — the arities cannot
> disagree, so no consumer has to reconcile them and none can silently drop a
> column. `arity()` reports the number of pairs; `from_key()` /
> `to_key()` project one side as a `Key` for consumers that key on a
> single side.
>
> `Key` (`pgorm-query/src/key.rs`) is a newtype over `Vec<Name>`: the
> columns a lookup keys on, in declared order, at every arity. It MUST NOT tag
> its arity in its representation, so a column set of a given width has one
> spelling and every consumer walks it rather than dispatching on how many
> columns it holds — `iter()`, `IntoIterator`, `arity()`, and `single()`, which
> answers with the one column of a unary set and declines for a wider one. `From<Name>`, `From<Vec<Name>>` and `FromIterator<Name>` build
> one. `IntoKey` converts a `Name`, any `StaticName`, and tuples of up to 12
> names; `KeyOf<E>` (`src/entity/key.rs`), a subtrait of `IntoKey`, restricts
> conversions to columns of entity `E`. `Key`, `IntoKey` and `IntoBoundary`
> live in pgorm-query and are re-exported: `IntoKey`'s blanket impl is over
> `StaticName`, and Rust admits no concrete sibling impl — not `Name`, not a
> tuple — beside a blanket over a trait foreign to the crate, so the trait and
> its impls MUST share a crate.
>
> Each `IntoKey` impl also names a `ValueType`: the tuple of `Value` of the
> same length as the columns it produces, or `ValueTuple` for `Key` itself,
> whose arity is only known at runtime. `IntoBoundary<K>` is the matching
> relation on the value side, implemented exactly for the tuples whose length `K`
> describes — plus, for `K = ValueTuple`, every `IntoValueTuple`. A consumer that
> pairs a column set with values (`[spec:pgorm:sem:exec.cursor.keyset]`)
> therefore gets the arity agreement from the type system rather than by
> checking it, and the `Key` case is the only one left to check.
>
> The `on_condition` closure receives the two identifiers *so that its
> predicate can follow the tables under whatever names the join binds them
> to*: the right identifier is the alias whenever the joined side is
> re-aliased — `join_as`, the graph's `join_maybe_as` / `join_one_as`
> (`[spec:pgorm:req:query.graph.aliases]`), a `Linked` hop's `r{i}`. A
> closure that ignores its parameters and qualifies by a hardcoded table
> name renders that name verbatim, so under an alias the predicate
> constrains the un-aliased name — a table absent from the query, or
> another join of the same table — and nothing errors client-side: the
> wrong rows match silently. This is a documented sharp edge, not a checked
> one; the closure is opaque, and no consumer can detect the miswiring. A
> closure MUST derive every table qualification from its parameters to be
> sound under aliasing; one that hardcodes is sound only where its relation
> is never joined re-aliased. The `Relation` derive's `on_condition` attribute
> written as an expression is where the mistake is easiest to write — it has
> no parameters in scope, and a `ColumnTrait` operator
> (`Column::Name.like(..)`) qualifies with the entity's canonical table —
> and the `tests_cfg` `TropicalFruit` relation is the standing in-tree
> example; written as a closure, `"|left, right| .."`
> (`[spec:pgorm:syn:macros.derive.relation]`), it takes the two identifiers
> as any other closure does.
>
> A relation's foreign key may be PostgreSQL 18's `NOT ENFORCED` — recorded,
> never checked, with no referential triggers — or deferrable, and the
> relation says so: `RelationBuilder::enforcement(e)` and `deferrability(d)`
> set the two fields, which `rev()` keeps and the conversion into a foreign
> key carries (`[spec:pgorm:req:entity.relation.fk]`), so the schema
> generated from an entity creates the key the entity describes rather than
> one enforced and checked at once. A `NOT ENFORCED` key admits a row whose
> columns reference nothing, and the relation's readers take such a row as
> having no related row, the answer they give a key that matches nothing:
> `load_one` yields `None` for it, `load_many` and `find_related` leave it
> out of every bucket, a graph's optional slot pairs it with `None`, and a
> required slot, joined `INNER`, leaves it out. The live suite holds each,
> beside the plain key that refuses the orphan (`23503`), and holds a
> deferred key letting a transaction write a row before the row it
> references, refusing at `COMMIT` one that never came (`23503`).
>
> A relation may match a `PERIOD`, PostgreSQL 18's temporal foreign key
> `FOREIGN KEY (.., PERIOD p) REFERENCES t (.., PERIOD q)`. `ColumnPairs` holds
> the period pair as a slot of its own beside the pairs matched for equality,
> set by `RelationBuilder::period(from, to)` on a builder that has its columns
> (a period alone is refused, `42601`) and read back by `period()`; `iter()`
> and the pairs stay the equality pairs, `rev()` swaps the period with them,
> and `from_key()` / `to_key()` end with the period column, since a row's
> period is part of what identifies the rows a join reached. The relation
> joins a row to each row whose period overlaps its own
> (`[spec:pgorm:sem:query.build.join]`), of which there may be several: the
> versions of a temporal key that together cover the row's period, which the
> server holds the foreign key to. So the relation's readers reach each such
> version: `find_related` and a graph slot return each, a required slot
> pairing the row with each; `load_one` reads the one version a period falls
> in and reports a period across two as more than one related row, as it
> reports any `HasOne` that matches two (`query.loader.regroup`); and the
> versions' `load_many` gives each version the rows it overlaps. Rows of one
> key in different periods keep apart, the loaders filing a row under its
> period too. The live suite holds each.

> [spec:pgorm:req:entity.relation.builder+1]
> `RelationBuilder<E, R, C>` (`src/entity/relation.rs`) accumulates a
> `RelationDef`, with `C` tracking whether the join columns have been supplied.
> The `belongs_to` path starts at `C = NoColumns` and MUST be given a column pair
> through `.columns(from, to)`, which takes one `E::Column` and one `R::Column`
> and moves the builder to `C = ColumnPairs`; a composite key adds each further
> pair with `.and_columns(from, to)`. Columns are therefore always supplied in
> pairs, and the two sides of a relation can never be given different numbers of
> columns. The `has_one` / `has_many` path starts at `C = ColumnPairs`, pre-filled
> from the reversed related definition, where `.columns(from, to)` instead
> replaces the pre-filled set with the given pair. Optional
> attributes are set by `on_delete(action)`, `on_update(action)`, `on_condition(f)`,
> `fk_name(name)`, and `condition_type(t)` in either state; `condition_type`
> defaults to `ConditionType::All`. The finished definition is obtained via
> `From<RelationBuilder<E, R, ColumnPairs>> for RelationDef`; there is no such
> conversion from `NoColumns`, so a relation missing its columns is a compile
> error rather than a panic.

> [spec:pgorm:req:entity.relation.linked+4]
> `Linked` (`src/entity/link.rs`) expresses a multi-hop join: `link()` returns the
> ordered `Vec<RelationDef>` chain from `FromEntity` to `ToEntity`. `find_linked()`
> MUST build the join by iterating the chain in reverse, aliasing each hop's source
> table as `r0`, `r1`, ... and inner-joining it to the previous alias (the innermost
> hop joins the target table the statement already selects from).
> `ModelTrait::find_linked` scopes the result to a model instance by filtering on the
> final alias `r{len - 1}` (`src/entity/model.rs`).
>
> A hop is not a bespoke join, and `find_linked` MUST NOT carry join machinery of
> its own. Walking a chain is only the binding of each hop's two ends to the names
> the walk gives them — the joined side re-aliased `r{i}`, the far side re-pointed
> at `r{i-1}`, which the innermost hop leaves at whatever identifier its relation
> already qualifies by — and the join itself is the crate's reverse edge,
> `QuerySelect::join_as_rev` (`[spec:pgorm:sem:query.build.join]`). The ON
> clause is therefore the one `join_condition` every other join in the crate
> emits, so a hop honours everything its relation declares: every `(from, to)`
> column pair, the `condition_type` that combines them, and the `on_condition`
> closure, which receives the two bound names in the roles the relation was
> written with (`[spec:pgorm:def:entity.relation.def]`). There is no second
> walker for the first to drift from.
>
> Those aliases are a type, `LinkedAlias`, whose `hop(i)` renders `r{i}` — not a
> `format!` repeated at each site that needs one. The last rung is derived by the
> provided method `Linked::last_hop_alias`, which every builder walking a chain
> MUST use and which is public precisely so that callers do too: the name is
> otherwise an internal that a caller can only reproduce by hardcoding `r4`, and
> a chain that later gains a hop rebinds that string to a different table without
> any diagnostic. The method is named for the mechanism rather than for one of
> its readings: `find_linked` walks the ladder backwards from `ToEntity`, so
> its last rung is the *source* table — which is exactly why it is what scopes
> `ModelTrait::find_linked`, and what a caller ordering or filtering on a
> `FromEntity` column names. A chain whose `link()` is empty yields
> `hop(0)`; the derivation saturates rather than underflowing, and the resulting
> query names a table the (join-less) statement does not have, which the server
> reports.
>
> A chain a `Related` implementation already describes MUST NOT have to be
> restated as a hand-written `Linked`: `RelatedLink<E, R>` is that chain, its
> `link()` returning `[E::to()]` for a direct relation and `[via, E::to()]` for
> a junction-mediated one — the order `Linked` expects. It is a zero-sized
> `Copy` witness carrying no data, written `RelatedLink::to(target_entity)`,
> which names only the target: the source entity is inferred from the position
> the witness is used in. Hand-written `Linked` impls remain the way to express
> what a `Related` impl does not — chains of more than one relation, and hops
> carrying a bespoke `on_condition`.
>
> The linked form aliases the joined table, so `RelatedLink::to(Entity)` on an
> entity related to itself is well-formed where an unaliased join of the
> `Related` path would name one table twice.

> [spec:pgorm:req:entity.relation.fk+6]
> A `RelationDef` converts into DDL foreign-key forms via
> `From<RelationDef> for ForeignKeyCreateStatement` and `for TableForeignKey`
> (`src/entity/relation.rs`). The conversion maps every pair in `columns` to a
> constrained column and its referenced column,
> writes the period pair after them as the key's `PERIOD` pair when the
> relation has one, applies `on_delete` and `on_update` actions, the
> `deferrability` and the `enforcement` when present, and names the
> constraint from `fk_name` when set; otherwise the name MUST be derived as
> `fk-{from_table}-{from_cols joined with '-'}`, the from columns being
> `from_key()`'s, the period's last, and bounded to PostgreSQL's 63 bytes
> as every name schema generation derives is
> (`[spec:pgorm:sem:schema.from-entity]`). Both conversions unpack the table
> references to bare tables (the schema of a `FromItem`'s `TableName`, and any bound
> alias, are reduced away by `unpack_table_ref`).
>
> Both conversions are total and MUST stay so: a `ColumnPairs` is non-empty and
> balanced by construction and a foreign key is built from exactly that, so the
> first pair goes to the constructor and the rest are appended
> (`[spec:pgorm:req:sql.ddl.foreign-key]`). There is no unpaired or empty
> column set on either side of the conversion for it to fail on.

## Prelude

> [spec:pgorm:def:entity.prelude+11]
> `pgorm::entity::prelude` (`src/entity/prelude.rs`) is the glob a file that
> talks to the database imports instead of naming what it needs one item at a
> time. Membership is chosen from what code actually writes, and is public API:
> removing a name from it breaks callers that never mentioned it.
>
> It carries the entity trait family and the derives that produce it
> (`entity.traits`), each by the name `macros.derive.names` gives it, so a derive
> named for a trait arrives with the trait; the query-builder traits whose methods are otherwise
> unreachable — `QueryFilter`, `QuerySelect`, `QueryOrder`, `QueryTrait`,
> `PaginatorTrait`, `CursorTrait`, `LoaderTrait`; the active-model vocabulary
> `ActiveValue` with its `Set` / `Unchanged` / `NotSet` variants,
> `IntoActiveModel`, `IntoActiveValue`, `TryIntoModel`, and the free `set`
> constructor (`entity.active-model.from-sugar`); the CRUD entry points
> `Select`, `Insert`, `Update`, `Delete`; the decode surface `FromQueryResult`,
> `QueryResult`, `DecodeSelect`, `DecodeRaw` (`exec.crud.selector-entry`); the
> connection types `DatabasePool`, `DatabaseConnection`, `DatabaseTransaction`,
> `ConnectionTrait`, `TransactionTrait`; `Iterable`, `Condition`, `JoinType`,
> `Value`, the `error` module's contents, and the handful of `pgorm_query`
> names an entity definition needs (`Expr`, `Name`, `StringLen`,
> `ForeignKeyAction`, `Func`, `Arc`, and `ConditionType`, `Deferrability`,
> `Enforcement` and `ArrayType`, which with `ForeignKeyAction` and `ColumnType`
> a derive's attributes name as written, with no prefix of the derive's
> (`[spec:pgorm:req:macros.derive.code-values]`)), with the range vocabulary a field of a range
> column is written in — `Range`, `Multirange` and `RangeType`
> (`[spec:pgorm:def:sql.value.range]`) — so a generated entity's
> `Range<i32>` field and `ColumnType::Range(RangeType::Int4)` resolve through
> the glob, and `CreatedRange` with its derive of the same name
> (`[spec:pgorm:def:sql.value.created-range]`), so a generated
> `pgorm_range_types.rs` resolves through it too. A file that also imports `std::ops::Range` by name keeps its
> own: an explicit import shadows a glob. The interval value `Interval`
> (`[spec:pgorm:def:sql.value.interval]`) is a member, with the qualifier
> vocabulary an interval column's type is written in — `IntervalSpec`,
> `PgInterval` and `IntervalPrecision` (`[spec:pgorm:def:sql.types.column-type]`)
> — so a generated entity's
> `column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth))`
> resolves through the glob as a range column's does.
>
> `Order` — `pgorm_query`'s `ASC`/`DESC` enum — is deliberately NOT a member.
> `order` is an ordinary table name, so an entity aliased `Order` is ordinary
> too, and a module globbing both it and this prelude makes every mention of
> `Order` an E0659 ambiguity; `order_by_asc` / `order_by_desc` cover the common
> case and the enum is one import away. The same hazard is live for members
> that stay — `ActiveEnum` collides with an entity of that name — so a file
> hitting it disambiguates with an explicit import, which is why the hazard is
> stated here rather than resolved by shrinking the prelude to names no schema
> could reuse.
>
> It also carries the relation vocabulary an entity definition and its call
> sites write: `Related`, `Linked`, `RelationDef`, `RelationTrait`, and the
> `RelatedLink` witness that spares a `Related` chain from being restated as a
> hand-written `Linked` (`[spec:pgorm:req:entity.relation.linked]`).
>
> The alias vocabulary `alias` and `AliasName` are members
> (`[spec:pgorm:sem:query.build.alias]`) on the same grounds as `Expr`: a name
> the query introduces is written where the query is written, and a token is
> only cheaper than `Name::runtime` when it is already in scope. `Func` is a
> member on the same grounds: a field's `default_expr` is Rust written in the
> entity's own module (`[spec:pgorm:sem:macros.derive.entity-model.column-def]`),
> and `default_expr = Func::uuidv7()` — a key PostgreSQL fills time-ordered
> (`[spec:pgorm:def:sql.ast.func]`) — resolves through the glob. `LinkedAlias` is
> NOT a member — it is reached as the return of a `Linked` method, so a caller
> never has to name the type.
>
> `StaticName` is a member — pgorm_query's, re-exported
> (`[spec:pgorm:def:entity.traits]`). The entity layer once declared a
> second trait of its own with the same method and an incompatible signature,
> and a prelude that globs one of them into every file made that collision
> reachable from anywhere; there is now one trait, so the hazard is retired
> rather than something a reader has to remember.
