# Query execution

The executor layer (`src/executor/`) turns built statements into database
round-trips and decodes `tokio_postgres` rows back into Rust values. It is
split into a decoding surface (`QueryResult` and the `TryGetable` family,
`src/executor/query.rs`) and a CRUD execution surface (`Selector`,
`src/executor/{select,insert,update,delete,execute}.rs`).
These rules capture what the code does today, including known gaps.

## Decoding (`exec.decode`)

> [spec:pgorm:def:exec.decode+3]
> `QueryResult` is a `#[repr(transparent)]` wrapper around a single
> `tokio_postgres::Row`. Values are extracted through the `TryGetable`
> trait, which has three entry points: `try_get_by` (any
> `tokio_postgres::row::RowIndex`, i.e. a column name or ordinal),
> `try_get` (a prefix plus column name, composed as `{pre}{col}` with no
> separator through the bounded spelling of
> `[spec:pgorm:sem:query.graph.writer]`, so a composition past
> PostgreSQL's 63-byte identifier bound looks up the same bounded name the
> projection writers mint; an empty prefix uses the bare column name), and
> `try_get_by_index` (ordinal position in the select list). Its fourth
> method extracts nothing: `accepts` asks which PostgreSQL types the
> implementing type can decode, for use before a row exists
> (`exec.verify.accepts`).
>
> `QueryResult` re-exposes these as `try_get_by`, `try_get`,
> `try_get_by_index`, `try_get_many`, and `try_get_many_by_index`, each
> converting the internal `TryGetError` into `Error`. `column_names`
> returns the result set's column names in order.

> [spec:pgorm:sem:exec.decode.null+1]
> Decoding delegates to `Row::try_get`. A resulting
> `tokio_postgres::Error` is classified by `TryGetError::postgres`: when
> the error's `source()` downcasts to `tokio_postgres::types::WasNull`
> it becomes `TryGetError::Null(...)`; any other error (including one with
> no source at all) becomes `TryGetError::Db(Error::Postgres(...))`.
>
> The blanket `TryGetable for Option<T>` impl converts
> `TryGetError::Null` into `Ok(None)` and propagates every other error,
> so SQL `NULL` is only an error when decoding into a non-`Option` type.
> `From<TryGetError> for Error` renders the null case as
> `Error::Type("A null value was encountered while decoding {s}")`.

> [spec:pgorm:sem:exec.decode.null-context+1]
> Null-decode errors do not carry structured column context. The payload
> of `TryGetError::Null` is the `Display` output of the underlying
> `tokio_postgres::Error` (e.g. "error deserializing column 0"), not the
> requested column name or prefix. The internal helper
> `err_null_idx_col` (query.rs:494) that would format the index is dead
> code (`#[allow(dead_code)]`) and returns the literal string `"TODO"`;
> the index formatting is commented out. This is a known limitation of
> the current behavior, not a contract to preserve. The absent-row test of
> `exec.decode.absent` therefore reads nullness off the row itself rather
> than off this payload.

> [spec:pgorm:req:exec.decode.absent]
> A `LEFT JOIN` that matched nothing still produces a row, one whose
> right-hand columns are all `NULL`. The joined decode reads its related
> side through `FromQueryResult::from_query_result_optional`, which MUST
> tell that unmatched row apart from a related row that is present and
> fails to decode: a decode failure MUST NOT be reported as a missing row.
>
> The witness is the set of columns the target reads under the prefix
> `pre`:
>
> - When `expected_columns` reports them
>   (`entity.traits.from-query-result`), the row is absent only if every
>   reported column — looked up as `{pre}{name}`, exactly as
>   `TryGetable::try_get` looks it up — is a column of the result set
>   *and* holds SQL `NULL`. A reported column the statement does not
>   return leaves the witness incomplete; that is a projection mistake
>   rather than an absent row, so the decode error MUST propagate.
> - A target that reports no columns (a hand-written implementation) is
>   judged against the result set instead: absent only if the row carries
>   at least one column whose name starts with `pre` *and* every such
>   column is `NULL`. An empty `pre` names every column of the row.
>
> Only that shape is `Ok(None)`. Every other decode failure propagates —
> a column whose PostgreSQL type the field cannot decode, an enum label
> with no Rust variant, a malformed JSON payload, or a `NULL` read into a
> non-`Option` field of a row whose remaining witness columns are not
> `NULL`.
>
> The nullness test is `QueryResult::all_null(pre, cols)` and
> `QueryResult::all_null_under(pre)`. Both read each column as
> `Option<AnyValue>`, where `AnyValue` is a private `FromSql` target that
> accepts every PostgreSQL type and reads none of the bytes, so a column
> holding a value the caller could not decode still counts as present.
> `all_null` answers `false` for an empty column list and for a column the
> result set does not carry; `all_null_under` answers `false` when no
> column carries the prefix.
>
> Two shapes are indistinguishable in the result set and are both read as
> absent: an unmatched outer join, and a matched row whose every witness
> column is genuinely `NULL`. An entity's primary key is `NOT NULL` and
> every entity model reads it, so the second arises only for a projection
> that leaves the key out.

> [spec:pgorm:def:exec.decode.types+3]
> The scalar Rust types implementing `TryGetable` by direct delegation to
> `Row::try_get` (and therefore accepting exactly the Postgres types
> tokio-postgres's `FromSql` accepts for them) are: `bool`, `i8`, `i16`,
> `i32`, `i64`, `f32`, `f64`, `String`, `Vec<u8>`, and
> `rust_decimal::Decimal`.
>
> `Decimal` is not feature-gated, and `rust_decimal` is a required
> dependency of `pgorm` rather than an optional one, because
> `[spec:pgorm:def:exec.cursor.binding]`'s bind path is always compiled
> and needs `Decimal`'s `ToSql` in every configuration — both for
> `Value::Decimal`, an unconditional variant of the value model
> (`[spec:pgorm:def:sql.value]`), and for the `numeric` arm of
> `[spec:pgorm:req:exec.cursor.binding-coerce]`, which is how *every*
> integer and float reaches a `numeric` placeholder. `rust_decimal` is
> compiled in any case, `pgorm-query` depending on it unconditionally; only
> this edge turns on the `db-tokio-postgres` feature that carries the impl.
> A `with-rust_decimal` feature would therefore gate nothing it claimed to,
> and turning it off did not drop a dependency — it only failed to build.
>
> Feature-gated additions: `serde_json::Value` (`with-json`);
> `jiff::civil::Date`, `jiff::civil::Time`, `jiff::civil::DateTime` and
> `jiff::Timestamp` (`with-jiff`); `uuid::Uuid` and its format wrappers
> (`with-uuid`). These three features are weaker than they look for the
> same reason — the underlying types are unconditional in `pgorm-query` and
> their `FromSql`/`ToSql` impls come from `postgres-types` through
> tokio-postgres's own always-on features — so what they gate is the
> conversion surface (`TryGetable`, `IntoActiveValue`, the prelude names),
> not the dependency. They still build with the feature off, which is why
> they remain.
>
> `jiff` is, like `rust_decimal`, a required dependency rather than an
> optional one. The bind path writes a naive datetime itself rather than
> through `postgres-types`
> (`[spec:pgorm:req:exec.cursor.binding-gaps]`), so it names jiff's civil
> types in every configuration; an optional edge left a build without
> `with-jiff` unable to compile. The crate is compiled in any case,
> `pgorm-query` and tokio-postgres's `with-jiff-0_2` both depending on it, so
> `with-jiff` turning off removes the conversion surface and nothing else.
>
> A workspace build cannot show any of this, because feature unification
> hands `pgorm` every feature any member's dev-dependency asks for. The
> `rust-features` checks in `.config/nplan/config.styx` therefore build the
> library alone — with no default features, with `macros` only, and with
> the defaults — and a path named outside its feature gate fails there.
>
> There is no time-crate, chrono or bigdecimal support: jiff is the only
> datetime path and `Decimal` the only arbitrary-precision numeric.
> With `with-uuid`, `uuid::Uuid` and its format wrappers
> (`uuid::fmt::Braced`, `Hyphenated`, `Simple`, `Urn`) decode by first
> reading a `uuid::Uuid` and then converting.
>
> The three Postgres-specific payload types behind `Value::Vector`,
> `Value::IpNetwork` and `Value::MacAddress` also implement `TryGetable`,
> unconditionally (they are not feature-gated). `pgvector::Vector`
> delegates straight to `Row::try_get`, using the `FromSql` impl pgvector
> provides under its `postgres` feature (enabled by `pgorm-query`), which
> accepts any type named `vector`. `ipnetwork::IpNetwork` and
> `mac_address::MacAddress` ship no `FromSql` impl and the orphan rule
> forbids writing one for them, so each decodes through a private local
> newtype — `InetSql` and `MacAddrSql` — that implements `FromSql` over
> `postgres_protocol::types::inet_from_sql` and
> `postgres_protocol::types::macaddr_from_sql` respectively and is
> unwrapped by the `TryGetable` impl. `InetSql` accepts `INET` and `CIDR`;
> `MacAddrSql` accepts `MACADDR` only, so the 8-byte `MACADDR8` is
> rejected. All three are re-exported from `pgorm_query` (and hence
> `pgorm::entity::prelude`) as `Vector`, `IpNetwork` and `MacAddress`, so
> callers need not depend on the payload crates directly.
>
> None of the three has a `Vec<T>` array impl: `exec.decode.array` does
> not cover them.

> [spec:pgorm:sem:exec.decode.u32-oid]
> `u32` implements `TryGetable` by decoding a `tokio_postgres::types::Oid`,
> so it reads Postgres `OID` columns, not `INT4`. Likewise `Vec<u32>`
> (under `postgres-array`) decodes as `Vec<Oid>`. No other unsigned
> integer widths implement `TryGetable`; `u8`, `u16`, and `u64` cannot be
> decoded from a row.

> [spec:pgorm:def:exec.decode.array+2]
> Under the `postgres-array` feature, `Vec<T>` implements `TryGetable` by
> direct array delegation for a subset of the `exec.decode.types` scalars:
> `bool`, `i8`, `i16`, `i32`, `i64`, `f32`, `f64`, `String`, plus the
> feature-gated json/chrono/decimal types and the uuid format
> wrappers, which decode as `Vec<uuid::Uuid>` then convert element-wise.
> `Vec<u32>` decodes as `Vec<Oid>` per `exec.decode.u32-oid`. The subset
> is proper: `Vec<u8>` is bytes rather than an array, and
> `pgvector::Vector`, `ipnetwork::IpNetwork` and
> `mac_address::MacAddress` have no `Vec<T>` impl at all — the array
> macros delegate to `Row::try_get` for `Vec<$type>`, which the two
> newtype-decoded types cannot satisfy without a separate unwrapping
> macro.
>
> Each array decode reads an array of a domain as an array of the type the
> domain is over, through any domains over domains. PostgreSQL reports a
> domain column by its base type, so a scalar needs nothing, but an array of
> a domain by the domain's own array type, whose element `postgres-types`
> checks by identity and refuses; the element's representation is its base
> type's, as it is when a value is bound (`exec.cursor.binding-accepts`), so
> the decode and its acceptance answer for the array type with every domain
> under it replaced by its base.

> [spec:pgorm:req:exec.decode.interval+1]
> `Interval` implements `TryGetable` through its own `FromSql`
> (`[spec:pgorm:def:sql.value.interval]`), and `Vec<Interval>` does under
> `postgres-array`, as the arrays of `[spec:pgorm:def:exec.decode.array]`
> do, each reading a column's months, days and microseconds as the server
> sent them. The prelude names `Interval`, and it implements
> `IntoActiveValue`, so an entity field can be an `Interval`, an
> `Option<Interval>` or a `Vec<Interval>`: `Schema::create_table_from_entity`
> writes its column as `interval` (`ValueType::column_type`), an
> `ActiveValue::Set` writes it bound in binary, a predicate on it binds the
> interval it compares with, and a row decodes back field for field. Codegen
> maps an `interval` column, and an array of them, to the type whatever the
> column's field qualifier, carrying the qualifier in the column's
> definition (`[spec:pgorm:sem:codegen.entity.types]`).

> [spec:pgorm:def:exec.decode.range+4]
> `Range<T>` and `Multirange<T>` (`[spec:pgorm:def:sql.value.range]`)
> implement `TryGetable` for each built-in subtype: `i32`, `i64` and
> `Decimal` unconditionally, and `jiff::civil::Date`,
> `jiff::civil::DateTime` and `jiff::Timestamp` under `with-jiff`, the gate
> their scalars have (`[spec:pgorm:def:exec.decode.types]`). `Range<T>`
> alone implements it for the subtypes only a range type a schema created
> ranges over (`[spec:pgorm:def:sql.value.created-range]`): `i16`, `f32`,
> `f64` and `String` unconditionally, `jiff::civil::Time` under `with-jiff`
> and `Uuid` under `with-uuid`. Their multiranges are reported by
> tokio-postgres as simple types, and an array of such a range has no
> conversion to bind it, so neither decodes. pgorm-query
> owns the two types but not the wire crates, so they have no `FromSql`, and
> the orphan rule forbids writing one here; each decodes through a local
> newtype — `RangeSql` and `MultirangeSql` — as an address decodes through
> `InetSql`. Giving pgorm-query `postgres-protocol` instead would
> change the dependency set every detached workspace locks, for an impl only
> pgorm calls.
>
> `RangeSql<T>` accepts a type whose kind is a range over a type `T`
> accepts, so it reads a range type a schema created over that subtype as
> it reads the built-in. It reads the flag byte through
> `postgres_protocol::types::range_from_sql`: the empty flag is
> `Range::Empty` and nothing else, never two unbounded sides; an unbounded
> side is `Bound::Unbounded`; each present bound is decoded by `T`'s own
> `FromSql` against the subtype, so a bound fails exactly as that value
> outside a range fails — a subtype's `infinity` among them, which is a bound
> value and not an absent bound. A NULL bound is a malformed message, since
> the server never sends one, and is refused rather than read as either.
> `MultirangeSql<T>` accepts a built-in multirange over a type `T` accepts
> and reads its count of length-prefixed ranges, refusing a count, a length
> or a trailing byte the message does not hold.
>
> That reading is one implementation. `range_from_sql_with(raw, bound)` and
> `multirange_from_sql_with(raw, bound)` read the format, each present
> bound's bytes handed to `bound`; `range_from_sql::<T>(subtype, raw)` and
> `multirange_from_sql::<T>(subtype, raw)` pass `T`'s `FromSql` for the
> subtype, and are what the two newtypes read through. All six are public:
> a binary COPY reads a built-in range's bounds into `Value`s through them
> (`conn.copy.rows`), and the language bindings read a built-in range
> through `RangeSql`, `MultirangeSql` or the `_with` readers, keeping no
> reader of their own. A multirange a schema
> created is reported by tokio-postgres as a simple type, which neither
> accepts: it is read as its text instead, by the newtype that names it
> (`[spec:pgorm:def:sql.value.created-range]`). `TryGetable::accepts` answers through the newtype, so
> `VerifyStatement` reports a range column of another subtype or a
> multirange read as a range.
>
> An array of ranges is `Vec<Range<T>>`, and of multiranges
> `Vec<Multirange<T>>`, for the same subtypes under the same gates and,
> as every `Vec<T>` decode is (`exec.decode.array`), under `postgres-array`.
> Each reads `Vec<RangeSql<T>>` or `Vec<MultirangeSql<T>>` through the
> driver's array decoding, so it accepts an array whose member the newtype
> accepts — `int4range[]` and an array of a range type a schema created over
> `int4`, but not `int8range[]`, `int4range` or `int4multirange[]` — and an
> element the newtype refuses fails the whole array. A NULL element fails as
> it does in every other `Vec<T>`. Their `ValueType` is the generic one
> (`[spec:pgorm:def:sql.value.array]`).

> [spec:pgorm:def:exec.decode.many]
> `TryGetableMany` extracts tuples from a row: `try_get_many` takes a
> prefix and a slice of column names, `try_get_many_by_index` reads
> ordinals starting at 0. It is implemented for any single
> `T: TryGetable` (using the first column), for `(T,)`, and for tuples of
> arity 2 through 12 of `TryGetable` elements. `find_by_statement`
> constructs a `SelectorRaw<SelectGetableValue<Self, C>>` from a raw SQL
> string and values, with column names supplied by iterating the `C`
> identifier enum.

> [spec:pgorm:req:exec.decode.many-arity]
> Callers of `try_get_many` MUST supply at least as many column names as
> the tuple arity: a shorter slice yields a type error of the form
> "Expect {N} column names supplied but got slice of length {len}".
> Surplus column names are ignored. The index-based path performs no such
> check; it simply reads ordinals `0..N`.

> [spec:pgorm:def:exec.decode.json+1]
> Under `with-json`, `TryGetableFromJson` decodes any
> `serde::Deserialize` type from a JSON/JSONB column via the
> `tokio_postgres::types::Json<T>` wrapper. A blanket impl provides
> `TryGetable` for every `TryGetableFromJson` type; consequently a type
> can implement `ActiveEnum` or `TryGetableFromJson` but not both.
>
> `TryGetableArray` (blanket-implemented for `TryGetableFromJson` types)
> provides `Vec<T>` decoding by reading the column as a
> `serde_json::Value` and running `from_json_vec`, which deserializes
> each element of a JSON array and fails with
> `Error::Json("Value is not an Array")` for any non-array value.

> [spec:pgorm:def:exec.decode.from-u64+3]
> `TryFromU64` converts a `u64` (e.g. a rows-affected-derived id) into a
> primary-key value type. Numeric impls (`i8`, `i16`, `i32`, `i64`, `u8`,
> `u16`, `u32`, `u64`) use checked `TryInto`, failing with
> `Error::Conversion` on overflow. `String` converts via `to_string`.
> Every other implementor — `bool`, `f32`, `f64`, `Vec<u8>`,
> `serde_json::Value`, the jiff types, `Decimal`,
> `uuid::Uuid`, `Range<T>` and `Multirange<T>` over any subtype, which a
> temporal key holds as its period (`entity.traits.primary-key`), and tuples
> of arity 2 through 12 — unconditionally returns `Error::ConvertFromU64`.

## Statement verification (`exec.verify`)

> [spec:pgorm:def:exec.verify+1]
> `VerifyStatement::verify::<M>(sql)` (`src/executor/verify.rs`) prepares
> `sql` on the connection and checks the result columns PostgreSQL
> describes against the columns `M` reports. It is implemented for
> `DatabaseConnection` and `DatabaseTransaction`, and by a blanket impl for
> every reference to an implementor; `M` is any `FromQueryResult + 'static`
> type, the `'static` bound being what naming the target in an error costs.
> The metric wrappers (`metric.layer`) do not implement it — verification is
> not a statement they have anything to report about — so an instrumented
> caller verifies on the `inner()` connection.
> The statement is prepared through `prepare`, not `prepare_cached`, and
> dropped, so verifying neither reuses nor seeds a cached statement.
>
> `FromQueryResult::expected_columns` answers `Option<Vec<ExpectedColumn>>`
> — one entry per column the type reads, in read order. An
> `ExpectedColumn` carries the column `name`, the `rust_type` spelling of
> the field decoding it, and that type's acceptance function
> (`exec.verify.accepts`). A column read through a cast also carries its
> read cast: `ExpectedColumn::selected_as(&column)` takes it off the entity
> column's own `select_as` answer, by the probe the graph's writer uses
> (`query.graph.writer`), and `read_cast()` reports it, `None` for a column
> read as the server sends it. Verification ignores it — a raw statement
> states its own casts — and a reader that writes the projection itself
> applies it (`pipeline.terminal`). Every reported name MUST appear among the
> statement's result columns, and each matched column's PostgreSQL type
> MUST satisfy `accepts`. The first failure of either test is returned and
> the remaining columns are not examined. Names are matched bare — the
> empty prefix `find_by_statement` decodes with — against the first result
> column of that name, which is the one `Row::try_get` would read.
>
> Nothing is executed, so the check costs one prepare round trip and no
> rows: it answers for a statement whose result set is empty exactly as it
> does for one with data, which is the point. A statement the server
> refuses to prepare at all fails as `Error::Postgres` before any column is
> compared.

> [spec:pgorm:sem:exec.verify.accepts+1]
> `TryGetable::accepts(ty)` answers whether a column of PostgreSQL type
> `ty` can be decoded into the implementing Rust type. Every built-in impl
> delegates to the `FromSql::accepts` of the type its decode actually
> reads, so acceptance and decoding cannot disagree: the
> `exec.decode.types` scalars and the `exec.decode.array` vectors delegate
> to themselves, the uuid format wrappers to `uuid::Uuid`, `u32` to `Oid`
> (`exec.decode.u32-oid`), `IpNetwork` and `MacAddress` to the private
> `InetSql` and `MacAddrSql` newtypes, and the `TryGetableFromJson` blanket
> impl accepts `JSON` and `JSONB`. `Option<T>` delegates to `T`: a nullable
> field accepts exactly what its payload accepts, and nullability plays no
> part (`exec.verify.limits`).
>
> The trait's default answers `true`. An implementation that does not
> override it — an `ActiveEnum` mapping, a `ValueType` newtype,
> the `TryGetableArray`-backed `Vec<T>`, or any hand-written impl —
> therefore accepts every column type, and verification reports nothing
> about that field rather than guessing at a decode path it cannot see.

> [spec:pgorm:req:exec.verify.errors]
> A failed verification MUST name what to change. `Error::Verify` wraps
> `VerifyError`, whose variants are distinct rather than one message
> shape: `ColumnMissing` carries the target, the column it reads, and the
> columns the statement does return; `ColumnType` carries the target, the
> column, the Rust type it decodes into, and the PostgreSQL type the
> statement returns for it; `Unreflected` carries the target
> (`exec.verify.manual`). The target is `std::any::type_name::<M>()`.
> `VerifyError` is `#[non_exhaustive]`, and verification returns these
> rather than panicking, per `[dec:pgorm:no-panic]`.

> [spec:pgorm:req:exec.verify.manual+1]
> `FromQueryResult::expected_columns` defaults to `None`, so a hand-written
> impl reports no columns. Verifying such a target MUST answer
> `VerifyError::Unreflected` rather than `Ok(())`: a decode pgorm cannot
> see into is unverifiable, and answering `Ok` would claim more than the
> evidence supports. A hand-written impl opts back in by overriding
> `expected_columns`. Both derives generate it
> (`macros.derive.from-query-result`, `macros.derive.model`), so derived
> targets — plain structs, entity `Model`s, and `PartialModel`
> structs, which derive `FromQueryResult` alongside it — are verifiable
> without further work.

> [spec:pgorm:req:exec.verify.limits+1]
> Verification checks column names and type acceptance, and MUST NOT be
> read as checking more.
>
> Nullability is out. PostgreSQL's describe answer carries no NOT NULL
> flag for a result column, so a `T` field is not reported for a column
> that can be NULL, nor an `Option<T>` field for one that cannot; a NULL
> arriving in a non-`Option` field remains `exec.decode.null`'s error at
> decode time. Parameters are out: `verify` binds no values, and a
> parameter-count mismatch already fails at Bind on every execution rather
> than only once rows exist. Row counts and query semantics are out.
>
> Columns the statement returns but the target does not read are not an
> error: reading a subset is a legitimate projection. Prefixed decoding is
> out — only the empty prefix is checked, so the `s{i}_` prefixes a source
> graph decodes under (`[spec:pgorm:sem:query.graph.decode]`) and the tuple
> targets of `TryGetableMany` (`exec.decode.many`), which are not
> `FromQueryResult` types at all, have no verification path. A pass is
> therefore not a proof
> that decoding will succeed; it closes the specific hole where an empty
> result set hides a target that names a column the statement does not
> return, or reads one into a type that cannot decode it.

## CRUD execution (`exec.crud`)

> [spec:pgorm:def:exec.crud+1]
> Statement execution is mediated by selector types: `Selector<S>` holds
> a `SelectStatement` plus a `SelectorTrait` implementor, and
> `SelectorRaw<S>` holds a raw SQL string plus `Values`. `SelectorTrait`
> has a single method, `from_raw_query_result`, turning one `QueryResult`
> into an item. Implementors: `SelectModel<M>` (one `FromQueryResult`
> model, empty prefix), `SelectGetableValue<T, C>` (tuple by named columns
> from the `C` enum), `SelectGetableTuple<T>` (tuple by ordinal), and
> `GraphRow<E, S>` (a source graph's declared tuple, one prefix per decoded
> source — `[spec:pgorm:sem:query.graph.decode]`).
>
> Conversions from a builder: `Select::into_model`, `into_partial_model`,
> `into_values`, `into_tuple`, `from_raw_sql`; the constructors that take a
> statement or a string directly are enumerated by
> `exec.crud.selector-entry`. Executing a `Selector` first builds SQL
> with the Postgres `QueryBuilder`, then binds each `Value` through the
> `ValueHolder` `ToSql` adapter (see `exec.cursor.binding`).

> [spec:pgorm:sem:exec.crud.selector-entry+2]
> A caller who has built a `SelectStatement` outside the entity builders —
> a CTE used as the driving table has no entity behind it at all — MUST
> still be able to name a decode target without collapsing the statement to
> a string. `Selector` therefore has one constructor per decode shape:
> `from_select::<M>(SelectStatement)` for a `FromQueryResult` type,
> `into_tuple::<T>(SelectStatement)` for an ordinal tuple, and
> `with_columns::<T, C>(SelectStatement)` for a tuple named by an `SqlName`
> enum. `SelectorRaw` mirrors the last two over `(String, Values)`:
> `from_statement::<M>`, `into_tuple::<T>` and `with_columns::<T, C>`.
>
> Those constructors name the row type twice — `Selector<S>`'s `S` is not
> constrained by the call, so the target appears once to pick the selector
> type and once as the constructor's own parameter, before the statement is
> mentioned at all. The statement-first spelling is therefore the trait pair
> `DecodeSelect` and `DecodeRaw`, whose `into_model::<M>`, `into_tuple::<T>`
> and `into_values::<T, C>` name it once and read in the order the rest of
> the builder does. `DecodeSelect` is implemented for `SelectStatement`;
> `DecodeRaw` for `(S, Values)` where `S: Into<String>`, which is what
> `SelectStatement::build`, `Pipeline::into_sql` and the `prql!` macro all
> hand back, so a compiled query flows into a decode without being taken
> apart first. They delegate to the constructors and add no behaviour of
> their own; both spellings remain, and both are in `entity::prelude`.
>
> The `Selector` constructors yield an ordinary `Selector`, so what they
> build is not a lesser statement: `one` still injects `LIMIT 1`
> (`exec.crud.select`), the empty-projection guard still runs before
> anything reaches the server (`query.build.modifiers`), and `all`, `stream`
> and the paginator are the same code as for an entity query. The
> `SelectorRaw` constructors carry the raw statement's semantics unchanged —
> no injected limit, and no projection guard, since a caller-supplied string
> has no projection list to inspect.

> [spec:pgorm:sem:exec.crud.select+4]
> `Selector::one` and `one_opt` narrow the query's row limit to one row with
> `narrow_limit_to_one` (`sql.ast.select.tie-limit`), then execute through
> the connection's `query_opt`. The narrowing sets `LIMIT 1` and never widens
> a limit: a query limited to no row, by `limit(0)` or a tie limit of zero,
> keeps `LIMIT 0` and reads none, which `one` and `one_opt` report as they do
> an empty result. The limit is set on the statement
> the selector holds, which for a CTE query is the carrying select and never
> a CTE body (`query.build.with.attach`). `SelectorRaw::one` and `one_opt`
> execute the raw statement as-is (no limit is injected), so a raw statement
> that returns more than one row is a `query_opt` protocol error rather than
> a first row.
>
> `one` returns the decoded item and fails with `Error::RecordNotFound`
> when zero rows are returned; `one_opt` returns `Ok(None)` in that
> case. This is a deliberate pgorm difference from SeaORM, where `one`
> returns an `Option`. `all` executes via `query_all` and decodes every
> row through `from_raw_query_result`, aborting on the first decode
> error. `Select::one`/`one_opt`/`all` delegate through `into_model`, and a
> source graph's terminals through its own selector
> (`[spec:pgorm:sem:query.graph.terminals]`).

> [spec:pgorm:req:exec.tie-limit+1]
> Every ORM path that sends a SELECT built from a `SelectStatement` MUST
> call `check_tie_limit` before it renders (`sql.ast.select.tie-limit`),
> beside the empty-projection guard those paths share, so a tie limit with no
> ORDER BY or beside `SKIP LOCKED` comes back as an `Error::QueryBuilder`
> naming it rather than as SQL the server refuses: a `Selector`'s and a
> `Select`'s terminals and streams, a graph's, a paginator's pages and count,
> and a cursor's executions. A tie limit reaches an ORM builder through
> `QueryTrait::query`, which hands out the statement.
>
> Where the ORM writes a row limit of its own, it MUST NOT replace a tie
> limit without a word. A paginator's `fetch_page` and `num_items` and a
> cursor's executions refuse a statement carrying one, with an
> `Error::Query` saying so, because pages and windows are counted in rows and
> replacing the tie limit with them would drop its ties. `one` and `one_opt`
> narrow any row limit to one row (`exec.crud.select`), a tie limit included:
> one row is asked for, and the first row of the order is the one a tie limit
> returns first, while a tie limit of zero, which returns no row, narrows to
> `LIMIT 0`.

> [spec:pgorm:req:exec.crud.exec-vocabulary+2]
> A CRUD terminal's name MUST determine the shape of what it returns, so
> that a reader of the call site needs no knowledge of which constructor
> produced the builder. Exactly five terminal names exist, each but `exec` in a
> singular and a plural spelling, and each MUST mean the same thing on every
> builder that offers it:
>
> - `exec` MUST emit no `RETURNING` clause and MUST return the rows-affected
>   count as a bare `u64`.
> - `exec_returning_pk` MUST return the inserted row's primary key, typed as
>   the entity's `PrimaryKey::ValueType`; `exec_returning_pks` MUST return
>   `Vec<ValueType>`, one key per row written.
> - `exec_returning_model` MUST return exactly one `Model`;
>   `exec_returning_models` MUST return `Vec<Model>`.
> - `exec_returning_change` MUST return one `Change<Model>`, the row before
>   the write and after it; `exec_returning_changes` MUST return
>   `Vec<Change<Model>>`, one per row changed (`exec.crud.versions`).
> - `exec_returning_upsert` MUST return `Option<Upserted<Model>>`, `None` for a
>   row the statement did not write; `exec_returning_upserts` MUST return
>   `Vec<Upserted<Model>>`, one per row written (`exec.crud.versions`).
>
> A builder offers the singular spelling only where it has exactly one row to
> answer for, and the plural only where it may have any number: the name says
> how many answers come back, and a batch cannot be asked for one. `Insert`
> and `TryInsert` carry that count as their row type, `OneRow` or `ManyRows`
> (`query.build.insert`). The mapping is therefore total:
>
> | Builder | `exec` | key-returning | model-returning | version-returning |
> | --- | --- | --- | --- | --- |
> | `Insert<A, OneRow>` | `u64` | `exec_returning_pk` → `ValueType` | `exec_returning_model` → `Model` | `exec_returning_upsert` → `Option<Upserted<Model>>` |
> | `Insert<A, ManyRows>` | `u64` | `exec_returning_pks` → `Vec<ValueType>` | `exec_returning_models` → `Vec<Model>` | `exec_returning_upserts` → `Vec<Upserted<Model>>` |
> | `TryInsert<A, R>` | `TryInsertResult<u64>` | the `Insert<A, R>` shape in `TryInsertResult` | the `Insert<A, R>` shape in `TryInsertResult` | *absent* |
> | `UpdateOne<A>` | *absent* | *absent* | `exec_returning_model` → `Model` | `exec_returning_change` → `Change<Model>` |
> | `UpdateMany<E>` | `u64` | *absent* | `exec_returning_models` → `Vec<Model>` | `exec_returning_changes` → `Vec<Change<Model>>` |
> | `DeleteOne<A>` | `u64` | *absent* | *absent* | *absent* |
> | `DeleteMany<E>` | `u64` | *absent* | *absent* | *absent* |
>
> `TryInsert`'s wrapper is the receiver type's contract, not a per-method
> variation: every `TryInsert` terminal MUST wrap the corresponding `Insert`
> terminal's shape in `TryInsertResult`. `TryInsert` offers no upsert
> terminal: its `Empty` and `Conflicted` say that nothing was written, which
> the upsert's own answer already says, `None` for the one row and nothing for
> a row of a batch.
>
> The batch spelling exists because the singular one, offered on every
> insert, answered a batch with whichever row came back: `exec_returning_pk`
> returned the last row's key — the only one read of the `RETURNING` rows it
> had asked for — and `exec_returning_model` wrote every row and then failed
> on the second one it was handed, since its one-row decode refused more.
>
> `UpdateOne` MUST NOT offer a bare `exec`. Updating one model by primary key
> always reads the row back, so there is no count-shaped answer to give; a
> caller wanting a count uses `Update::many` filtered to the key. The
> absence is the point: no `exec` on `UpdateOne` can be misread as a count.
>
> The retired spellings MUST NOT exist. `exec_without_returning` is `exec`;
> `exec_with_returning` is `exec_returning_model` (or `exec_returning_models`
> on `UpdateMany`); the old `Insert::exec`, which returned a primary key
> under a name that promised nothing, is `exec_returning_pk`.

> [spec:pgorm:sem:exec.crud.versions]
> PostgreSQL 18's `RETURNING` reads a written row as it was before the
> statement wrote it and as the statement left it (`sql.ast.returning`), and
> four write terminals read both, answering in two types the crate root
> exports, `Change` and `Upserted`. `UpdateOne::exec_returning_change` returns a
> `Change<Model>` — `old`, the row before the update, and `new`, the row after
> it — and `UpdateMany::exec_returning_changes` one per row the update
> changed, in the order the server returned them. `Insert::exec_returning_upsert`
> (`OneRow`) and `exec_returning_upserts` (`ManyRows`) return an
> `Upserted<Model>` per row the insert wrote: `Inserted(model)` for a row it
> inserted, and `Updated(Change)` for a row whose `ON CONFLICT DO UPDATE`
> updated the row it conflicted with, before and after. A row the statement
> did not write — one `DO NOTHING` skipped, or one the `DO UPDATE`'s `WHERE`
> left as it was — returns nothing, so the singular terminal answers `None`
> for it and the plural leaves it out; an insert without a conflict clause
> answers `Inserted` for every row. `Upserted::into_model` is the row as the
> statement left it, whichever it did.
>
> Which an upsert did is read off the old version, the answer PostgreSQL
> documents: a row the insert wrote has no old row, which reads `NULL` in
> every column, and a row `DO UPDATE` updated has the row as it stood. The
> old model decodes through `FromQueryResult::from_query_result_optional`
> (`exec.decode.absent`), whose all-`NULL` witness is exactly that: an
> entity's primary key is `NOT NULL`, so an existing row never reads `NULL`
> in every column. The live suite holds the answer under the race `ON
> CONFLICT` arbitrates: an upsert waiting on another transaction's
> uncommitted insert of its key updates the committed row, and says so.
>
> The list returns every column of the entity from both versions, each read
> through the column's `select_as` (`entity.traits.column.enum-cast`), the
> old one named under the prefix `o_` and the new under `n_` by the bounded
> composition every prefixed decode reads (`query.graph.writer`), so a
> column name long enough to reach PostgreSQL's 63-byte identifier bound
> still decodes. The two versions are always renamed, `RETURNING WITH (OLD AS
> "pgorm_old", NEW AS "pgorm_new")`: `old` and `new` are scoped like any
> relation name, and a statement with a relation called either — an entity
> whose table is called `old`, an `UpdateMany::from` item called `new` — would
> take the keyword, the server answering `old."col"` from that relation
> without complaint, so an entity whose table is `old` would report its new
> row as its old. A relation called `pgorm_old` or `pgorm_new` is refused
> instead (`42712`), which says so rather than answering wrong.
>
> The terminals keep their builders' terms: `exec_returning_change` with
> nothing to set sends nothing and returns the row its `WHERE` reads as both
> versions, as `exec_returning_model` returns it; `exec_returning_changes`
> with nothing to set is `Error::NothingToSet`, as the other many-row
> terminals are (`exec.crud.update`); and `exec_returning_upserts` holds a
> batch to uniform columns and answers an insert of no models with nothing,
> as `exec_returning_models` does (`exec.crud.insert-returning`). An update by
> key matching no row is `Error::RecordNotFound`. `TryInsert` offers neither
> upsert terminal: its `Empty` and `Conflicted` are what the upsert's own
> answer already says, nothing for nothing written.

> [spec:pgorm:sem:exec.crud.insert+6]
> `Insert::exec_returning_pk` and `exec_returning_pks` append a `RETURNING`
> clause of the entity's primary-key columns and resolve the keys (typed as
> the entity's `PrimaryKey::ValueType`) from that clause and from nothing
> else. The statement runs through `query_all`, each returned row's
> primary-key columns are read by name, and a decode failure of the key
> columns fails with `Error::UnpackInsertId`. This holds whether or not the
> caller supplied the key: an entity whose key is not auto-increment MUST be
> answered from the row the database wrote, exactly as an auto-increment one
> is, and a key part the database generated — an identity inside a composite
> key (`macros.derive.entity-model.primary-key`) — comes back in the tuple
> like any other part.
>
> `exec_returning_pk` exists on `Insert<A, OneRow>` alone, which holds one
> model, so the statement writes at most one row: its key is the answer, and
> no row — the conflict clause skipped it — fails with
> `Error::RecordNotInserted`. `exec_returning_pks` exists on
> `Insert<A, ManyRows>` and returns every row's key, in the order the rows
> came back. For `INSERT ... VALUES` that is the order the models were added:
> PostgreSQL's executor writes the `VALUES` rows in list order and emits each
> `RETURNING` row as it writes it, which a live test pins with keys added out
> of key order, though PostgreSQL's documentation makes no promise about the
> order of `RETURNING` rows. A row the conflict clause skipped has no key, so
> under `ON CONFLICT DO NOTHING` the keys are those of the rows written, still
> in order, and no longer one per model; a batch every row of which was
> skipped, and an insert no model was ever added to, return no key, which is
> `Ok` — a batch asked for every key written and got them all.
>
> The client-supplied-key mode is deleted and MUST NOT return. It ran the
> statement through `execute`, discarded the `RETURNING` rows it had already
> asked for, and rebuilt the key from a `ValueTuple` the builder had cached —
> so the answer was the key the caller *asked* to write rather than the key of
> the row that got written. Under `ON CONFLICT DO UPDATE` those differ: an
> insert of `id = 42` that conflicts on some other unique column updates the
> existing row and reports `42`, a primary key naming no row. Reading
> `RETURNING` reports the conflict row's own key. With the cache gone, a
> mistyped `PrimaryKey::ValueType` on an insert surfaces as the
> `Error::UnpackInsertId` of a failed column decode rather than as the
> `Error::Type` the tuple reconstruction raised. No CRUD path rebuilds a
> key from a cached tuple any more: `exec.crud.update`'s no-op read reuses
> its statement's `WHERE` rather than reconstructing one.
>
> The key is returned bare. There is no `InsertResult` wrapper and no
> `last_insert_id` field: a one-field struct whose field repeated the
> method's promise carried no information the name does not, and
> "last insert id" named a MySQL affordance rather than the `RETURNING`ed
> primary key this actually is.

> [spec:pgorm:sem:exec.crud.insert-returning+3]
> `Insert::exec_returning_model` and `exec_returning_models` append a
> `RETURNING` clause of **all** entity columns and decode every returned row
> through `SelectorRaw::<SelectModel<Model>>::all`, in the order the rows came
> back (`exec.crud.insert`). `exec_returning_model`, on `Insert<A, OneRow>`,
> answers with its one row and fails with `Error::RecordNotFound` when none
> came back; `exec_returning_models`, on `Insert<A, ManyRows>`, answers with
> every row, none included. `Insert::exec` appends no `RETURNING` clause and
> returns the rows-affected count as `u64`.

> [spec:pgorm:sem:exec.crud.try-insert+4]
> `TryInsert::exec` and the returning terminals of its row type
> (`exec_returning_pk` and `exec_returning_model` on `OneRow`,
> `exec_returning_pks` and `exec_returning_models` on `ManyRows`)
> wrap the corresponding `Insert` executions in `TryInsertResult`. When
> the underlying insert statement has no columns (e.g. `Insert::many`
> over an empty iterator), they return `TryInsertResult::Empty` without
> touching the database — the failsafe for empty batch inserts.
>
> Each otherwise reports an insert that the conflict clause skipped
> as `TryInsertResult::Conflicted`, each reading the signal its own
> execution yields for "no row was written". `exec_returning_pk` maps a
> `Error::RecordNotInserted`, which `exec.crud.insert` raises only when
> the primary-key `RETURNING` came back empty. `exec`
> maps a zero rows-affected count and `exec_returning_model` maps a
> missing `RETURNING` row — both only when the statement carries an
> `ON CONFLICT` clause, since neither signal can otherwise be attributed
> to a conflict. Without such a clause those two keep the plain `Insert`
> outcome, `TryInsertResult::Inserted(0)` and `Error::RecordNotFound`
> respectively. The batch terminals read an empty `Vec` the same way: with an
> `ON CONFLICT` clause it is `Conflicted`, the clause having skipped every
> row, and without one it is `Inserted` of the empty `Vec`; a batch the clause
> skipped only in part is `Inserted` of the rows written. Success becomes
> `TryInsertResult::Inserted(..)`; every other error propagates.

> [spec:pgorm:sem:exec.crud.update+7]
> `UpdateMany::exec` executes the statement and returns the rows-affected
> count as `u64`.
>
> An update whose `WHERE` matches nothing is `Ok(0)`, never an error. There
> is no `Updater` and no `check_record_exists`: the count is the whole
> answer and the caller decides what zero means. `Error::RecordNotUpdated`
> is deleted with them — it had no other producer, and an error variant no
> code path can raise is a promise the error model cannot keep.
>
> That answer is what an update carrying no SET values MUST NOT borrow. Such
> a statement is never sent, so no `WHERE` was ever evaluated and there is no
> matched-row count to report; returning `0` would spend the one signal
> `Ok(0)` already carries on a second, incompatible meaning. Both many-row
> terminals therefore refuse it with `Error::NothingToSet`
> (`[spec:pgorm:def:error.model]`) — `exec` and `exec_returning_models`
> alike, so the choice of terminal cannot change whether the statement was
> sent. The input is ordinary rather than exotic: `UpdateMany::set` skips
> `Unchanged` and `NotSet` fields (`[spec:pgorm:sem:query.build.update]`),
> so a model read back from the database and handed straight to `set`
> produces exactly it.
>
> The singular terminal is not held to that refusal, and the asymmetry is the
> point: `UpdateOne::exec_returning_model` promises the model, names one row
> by its primary key, and so has a row to read when there is nothing to
> write. `UpdateMany` has no such row, and a `SELECT` standing in for the
> refusal would change what the method costs without changing what it is
> called.
>
> A `TryUpdate` escape valve — the `TryInsertResult::Empty`
> (`[spec:pgorm:sem:exec.crud.try-insert]`) of updates, letting a caller
> take "nothing to set" as an outcome rather than an error — is deliberately
> NOT provided. `TryInsert` exists because a conflict clause makes "no row
> written" a routine result of a statement that was sent; nothing to set is
> a property of the statement the caller built, testable before executing it
> through `QueryTrait::as_query().get_values()`, so the valve would wrap a
> question the caller can already ask.
>
> `UpdateOne::exec_returning_model` returns the updated model: it appends a
> `RETURNING` clause of all entity columns and decodes through
> `SelectorRaw::one`, so an update matching zero rows surfaces the
> `Error::RecordNotFound` of `exec.crud.select`. On the no-op path (nothing
> to set) it instead re-reads the current model with a plain `SELECT` under
> the update statement's complete `WHERE` — the primary-key predicates
> `query.build.update` installed plus every filter the caller added — so a
> guard predicate (a version or tenant check) excludes an unchanged model
> exactly as it would exclude a changed one, and a non-matching guard
> surfaces the same `Error::RecordNotFound`. The changed and unchanged
> paths thereby agree on row eligibility, and since no `UPDATE` is issued
> on the no-op path, it can fire no triggers. A statement carrying no
> `WHERE` at all is refused with `Error::PrimaryKeyNotSet`, but the guard
> is defensive only: `query.build.update` rejects an unset primary key when
> the `UpdateOne` is built, so no caller can reach the terminal with one.
> There is no typed-key rebuild: the read reuses the statement's own
> predicates rather than reconstructing a key through `sql.value.tuple`, so
> a mistyped `PrimaryKey::ValueType` is never consulted here.
> `UpdateMany::exec_returning_models` appends the
> same full-column `RETURNING` and returns `Vec<Model>` via `all`.

> [spec:pgorm:sem:exec.crud.delete+1]
> `DeleteOne::exec` and `DeleteMany::exec` both build
> the `DeleteStatement`, bind values through `ValueHolder`, execute, and
> return the rows-affected count as `u64`. There is no existence check:
> deleting zero rows is `Ok(0)`, never an error.
>
> There is no `Deleter` and no `DeleteResult`. The intermediate builder had
> no caller and no capability the two entry points lack, and the result
> struct was a one-field wrapper over the count that `exec` now returns
> directly — the same `u64` every other `exec` yields, per
> `exec.crud.exec-vocabulary`.

> [spec:pgorm:def:exec.crud.exec-result+1]
> `ExecResult` is a `#[repr(transparent)]` wrapper over a `u64`
> rows-affected count (`ExecResultHolder`), exposed via
> `rows_affected()`. It carries no primary key; inserted keys are
> obtained through `RETURNING` per `exec.crud.insert`.

## Streaming (`exec.stream`)

> [spec:pgorm:def:exec.stream+2]
> Row-level streaming is exposed through `ConnectionTrait::query_raw`, which
> takes the same `BorrowToSql` `ExactSizeIterator` params as `execute_raw`
> and returns a `tokio_postgres::RowStream`: rows are read from the
> connection as they arrive instead of being buffered into a `Vec` the way
> `query_all` does. It is implemented by `DatabaseConnection`,
> `&DatabaseConnection`, `DatabaseTransaction`, `InstrumentedConnection`,
> and `InstrumentedTransaction`.
>
> On top of it the executor exposes `stream` on `SelectorRaw`, `Selector`,
> `Select`, and `SelectGraph`, plus `stream_partial_model` on `Select`,
> each returning `PinBoxSendStream<'db, Result<Item, Error>>` — an alias for
> `Pin<Box<dyn Stream<Item = ..> + Send + 'db>>`. Unlike `PinBoxStream`
> (used by `Paginator::into_stream`) it is `Send`, so a streamed select can
> be consumed from a spawned task. The graph's grouped read has no streamed
> form: its output requires all rows of a root before any entry is complete
> (`[spec:pgorm:sem:query.graph.grouped]`). Page-batched and keyset-windowed
> consumption remain available through `exec.paginator` and `exec.cursor`.

> [spec:pgorm:sem:exec.stream.decode+1]
> `SelectorRaw::stream` binds `Values` through the `ValueHolder` `ToSql`
> adapter exactly as `all` does, then maps the `RowStream` item-wise: each
> `Ok(row)` is decoded by `S::from_raw_query_result`, and each transport
> error becomes `Error::Postgres`. Decoding is lazy and per-item — no row is
> decoded until it is polled, and a decode failure is yielded as one `Err`
> item rather than discarding the rest of the result set, which is the
> deliberate difference from `all` (which aborts at the first bad row).
>
> The adapter is a stateless `map`: pgorm neither fuses the stream after an
> error nor cancels the in-flight query. Dropping the stream before it is
> exhausted is safe — the tokio-postgres connection task keeps paging
> through and discarding the remaining messages, so the connection stays
> usable — but the query still runs to completion server-side. A stream MUST
> NOT be polled after the connection or transaction it came from is dropped:
> `RowStream` is `'static` and does not borrow the connection, so doing so
> compiles, and the stream then yields `Error::Postgres` for a closed
> connection.
>
> The metric layer records `query_raw` at stream *creation*, timing only the
> round-trip that produced the stream and reporting `rows: None` — the row
> count is not knowable up front, and `MetricsCollector`'s hooks are `async`
> so they cannot be invoked from the stream's `Drop`.
