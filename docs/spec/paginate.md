# Pagination

pgorm offers two pagination mechanisms: keyset ("cursor") pagination over
ordered columns (`src/executor/cursor.rs`) and classic LIMIT/OFFSET page
pagination (`src/executor/paginator.rs`). The cursor module also defines
`ValueHolder`, the `ToSql` adapter used by every executor path to bind
`pgorm_query::Value` parameters, and with it the wire-type contract every
bound parameter is held to.

## Cursor pagination (`exec.cursor`)

> [spec:pgorm:def:exec.cursor+5]
> `Cursor<S, K>` wraps a `SelectStatement` plus the target table, an
> `Key` of one or more order columns, an optional `Window` row
> limit, optional `before`/`after` boundary `ValueTuple`s, a `sort_asc`
> flag (default ascending), and a list of secondary order columns. `K` is
> the boundary shape the order columns fix — the `IntoKey::ValueType`
> of `[spec:pgorm:def:entity.relation.def]` — and defaults to
> `ValueTuple`. The boundaries are set by `before`/`after`, whose arity `K`
> fixes, or by `before_with`/`after_with`, which take the cursor's whole
> sort key including its secondary order columns and so cannot be typed by
> `K` (`[spec:pgorm:sem:exec.cursor.keyset]`). Cursors are created via
> `Select::cursor_by` (order columns on the entity's table) and, for joined
> reads, `SelectGraph::cursor_by` / `cursor_by_on`
> (`[spec:pgorm:sem:query.graph.cursor]`: order columns on the root or on a
> positionally-named slot), each returning a cursor keyed by its argument's
> `ValueType`. `CursorTrait` names the `SelectorTrait` used
> to decode rows; `into_model` and `into_partial_model` re-target the
> decoded type and carry `K` across unchanged. `Cursor` also implements
> `QuerySelect` and `QueryOrder` for further query modification.
>
> `S` is unconstrained on the struct: only `Cursor::all` requires
> `S: SelectorTrait`, because only fetching needs to decode. `SelectUndecoded`
> exploits that — it is the `S` the `select_only` typestates' `cursor_by`
> returns (`query.build.modifiers`), and being no `SelectorTrait` it makes a
> cursor over a caller's projection unfetchable until `into_model` or
> `into_partial_model` names the row type.

> [spec:pgorm:sem:exec.cursor.keyset+8]
> A cursor's *keyset* is the column list its rows are totally ordered by:
> the order columns, qualified with the cursor's table, followed by each
> unary secondary order entry qualified with its own table
> (`[spec:pgorm:sem:exec.cursor.order]`). `ORDER BY` and the boundary
> comparison MUST both be built from that one list, so the row order and
> the predicate that resumes it cannot disagree about where a page ends.
>
> `after(values)` filters to rows strictly beyond the boundary in the
> logical sort direction: column `>` value when ascending, `<` when
> descending. `before(values)` is the mirror image (`<` ascending, `>`
> descending). For a key of n columns the boundary is the row-value
> comparison `(c1, ..., cn) ⋈ (v1, ..., vn)` written out as
> `(c1 = v1 AND ... AND cn ⋈ vn) OR (c1 = v1 AND ... AND c(n-1) ⋈ v(n-1)) OR ... OR (c1 ⋈ v1)`
> where `⋈` is the direction comparison — one generic fold over the keyset,
> at every arity. Conditions are added to the composed query's `WHERE` via
> `cond_where`; both `before` and `after` may be set simultaneously.
>
> Each boundary value binds under its keyset column's `save_as` cast
> (`entity.traits.column.enum-cast`), captured while the typed column is in
> hand — an order column's from the typed column set at `cursor_by` time, a
> graph's tiebreak's from the source's primary-key column as the graph
> installs it (`[spec:pgorm:sem:query.graph.cursor]`), and a caller's
> secondary entry's from the column `add_secondary_order_by(table, column)`
> takes — the keyset itself being type-erased identifiers, so the cast
> cannot be recovered later. A keyset column whose domain is over an enum
> compares as every predicate on it does: the column is cast to the enum in
> each of the boundary's comparisons, and its value to the enum in place of
> `save_as`, PostgreSQL resolving no operator for the domain (`42883`).
> `ORDER BY` keeps the bare column, which sorts as its enum does, so the row
> order and the boundary agree. An identifier no column of the capture set
> claims binds bare, as a plain column's value always did, and so does an
> entry installed through `set_secondary_order_by`, whose `(table, Key)`
> pairs name no typed column: it is the form for an entry no entity column
> describes, and a column whose domain is over an enum installed through it
> is refused by the server (`42883`) once a boundary compares it.
> `add_secondary_order_by` appends one entry, qualified with the table — or
> alias — it is given; `set_secondary_order_by` replaces every entry,
> appended ones included.
>
> A boundary may be given at either of two arities, and its arity selects
> how much of the keyset it compares. At the arity of the order columns it
> compares those alone — the only shape available when there are no
> secondary entries, and the fallback for a joined cursor: it can say no
> more than "past every row sharing this order-column value", so resuming a
> page that ended part-way through such a run drops the rest of that run.
> At the arity of the whole keyset it compares the whole keyset, which
> names a single row and resumes from it exactly. Any other length MUST be
> reported when the filters are composed, as
> `Error::Query(RuntimeError::Internal)` reading "cursor boundary of arity
> {n} does not match {m} order column(s)" — with {m} written "{primary} or
> {keyset}" when the cursor has secondary entries. No arity mismatch
> panics.
>
> The order-column arity is a type error rather than a runtime one.
> `before` and `after` accept any `V: IntoBoundary<K>`, and for the `K` a
> `cursor_by` argument fixes, the only tuples implementing
> `IntoBoundary<K>` are those of the same length — so
> `cursor_by((A, B)).after(1)` does not compile, and neither does
> `cursor_by(A).after((1, 2))`. The one exception is a runtime-built
> `Key`, whose `ValueType` is `ValueTuple`: that `K` admits any
> `IntoValueTuple`, keeping the arity check for execution. The extended
> arity has no such `K` to be checked against — it is the order columns'
> length plus a secondary count fixed at run time — so `before_with` and
> `after_with` take any `IntoValueTuple` and are checked when the filters
> are composed. They accept either arity, so widening a call site is
> adding `_with` and the trailing values.
>
> One limitation follows from comparing a joined table's column: under an
> outer join a secondary key value may be `NULL`, and a `NULL` operand
> makes the comparison `NULL` rather than true, so a row whose tiebreak
> column is null is excluded from the page that `ORDER BY` would place it
> in. Rows with a null keyset value are reachable through the order-column
> boundary, not through an extended one.
>
> The comparison direction is read when the filters are composed, from the
> cursor's final `sort_asc` — not at the call to `before`/`after`. So
> `after(x).desc()` and `desc().after(x)` build the same query, and both
> mean "the rows following `x` in descending order", i.e. those less than
> `x`, rather than the ascending sense the `after(x)` call site suggests.

> [spec:pgorm:sem:exec.cursor.window+1]
> The row limit is a single `Option<Window>`, where `Window` is
> `First(u64)` or `Last(u64)` and `Window::rows` projects out the count.
> A "first" and a "last" limit therefore cannot both be set: `first(N)`
> and `last(N)` each replace the whole window, so the most recent call
> wins. A set window applies `LIMIT rows`. `Last` fetches the window from
> the far end by flipping the emitted SQL sort order: the SQL order is
> ascending iff `sort_asc` XNOR `Last` (i.e. `asc` + `First` or `desc` +
> `Last` emit `ASC`; the other two combinations emit `DESC`). When the
> window is `Last`, the fetched buffer is reversed in memory after
> decoding, so `all` always returns rows in the cursor's logical
> (`asc`/`desc`) order regardless of windowing direction.

> [spec:pgorm:sem:exec.cursor.order+4]
> `Cursor::all` composes each execution onto a *copy* of the stored query:
> the limit, then the order clause, then the boundary filters are applied
> to the clone, which is then built and executed via `query_all` and
> decoded row by row with the selector. The stored query stays the one the
> caller handed the cursor, so a cursor MUST be re-executable — the clauses
> a `SelectStatement` replaces per call (`LIMIT`, `ORDER BY`) and the
> `WHERE` it only ever conjoins to are alike here, and a moved boundary or
> a flipped direction replaces the previous execution's rather than
> intersecting with it. `after(5)` then `after(2)` therefore pages from 2,
> not from `id > 5 AND id > 2`, and toggling `asc`/`desc` with a boundary
> set does not compose a page that is empty by construction.
>
> Ordering first clears any pre-existing `ORDER BY` on the copy, then
> orders by the cursor's keyset — its order columns qualified with its
> table, in declared order, then its secondary order entries qualified with
> theirs — all using the single resolved direction of
> `exec.cursor.window`. This is the same list the boundary comparison of
> `[spec:pgorm:sem:exec.cursor.keyset]` is built from. Only
> unary secondary entries take part — those whose `Key` has arity 1
> (`[spec:pgorm:def:entity.relation.def]`), a length now read rather than
> matched on; composite secondary keys are silently ignored, in the
> ordering and in the boundary alike. A joined read installs those entries from its declaration rather
> than from a call site (`[spec:pgorm:sem:query.graph.cursor]`), so a joined
> cursor is totally ordered and can be resumed mid-tie through `after_with`
> / `before_with`.

> [spec:pgorm:def:exec.cursor.binding+6]
> `ValueHolder` (cursor.rs) is a public newtype over `pgorm_query::Value`
> implementing `tokio_postgres::types::ToSql`; every executor path
> (select, insert, update, delete, cursor, paginator) wraps built
> statement values in it for parameter binding. It delegates per variant:
> `Bool` to the primitive `bool` impl; the integer variants
> (`TinyInt`/`SmallInt`/`Int`/`BigInt`/`Unsigned`/`BigUnsigned`) and the
> float variants (`Float`/`Double`) through the numeric coercion of
> `[spec:pgorm:req:exec.cursor.binding-coerce]`, which also covers the
> `oid` and `"char"` targets the corresponding primitive impls define —
> `BigUnsigned` (u64) reaching all of them through a checked conversion to
> `i64`; `Char` is stringified; `String`, `Bytes`, `Json`, `Date`, `Time`,
> `DateTimeWithTimeZone` and `Uuid` bind their payload, with `None` payloads
> emitted as SQL `NULL` (`IsNull::Yes`); `Array` recursively wraps its
> elements in `ValueHolder` (a `None` array is `NULL`), so the numeric
> coercion also applies element-wise against the array's member type;
> `Range` and `Multirange` write their bounds through `ValueHolder` the same
> way, against the range's subtype, per
> `[spec:pgorm:req:exec.cursor.binding-range]`.
>
> Two variants are written by hand rather than delegated, each because the
> library impl refuses or discards a value PostgreSQL accepts.
>
> `DateTime` (the naive `timestamp`) is encoded as the microsecond count of
> `duration_since` the PostgreSQL epoch, through
> `postgres_protocol::types::timestamp_to_sql`. `postgres-types` derives the
> same payload from `since`, whose span's largest unit is *days*; balancing
> days down to microseconds obliges jiff to say how long a day is, and given
> a relative civil datetime it answers by projecting onto the absolute
> `Timestamp` timeline, which is strictly narrower than `civil::DateTime` by
> its largest UTC offset at each end. So roughly 26 hours at each end of the
> calendar are refused with `"value too large to transmit"` despite being
> representable, storable and decodable. A `SignedDuration` difference needs
> no calendar context and has no such edge. Inside the range both encodings
> MUST agree byte for byte.
>
> `Decimal` binds its payload except when the value is zero and carries a
> non-zero scale, where the 8-byte header is written directly with the
> display scale the value holds. A zero `numeric` has no digit groups, so
> that header is its entire encoding and the display scale its only
> informative field; `rust_decimal`'s zero shortcut hardcodes the scale to 0,
> which sends `0.00` as `0`. Scale is a property of the value — PostgreSQL
> reports `scale(0.00::numeric)` as 2 and renders the two differently — and
> the decode side already refuses to lose it, so the binding side MUST NOT.
>
> `Vector` delegates to `pgvector`'s own `ToSql` impl, which `pgorm-query`
> enables through pgvector's `postgres` feature. `IpNetwork` and
> `MacAddress` are encoded directly with
> `postgres_protocol::types::inet_to_sql` (address, prefix length, and
> `is_cidr` = 0 — so an `IpNetwork` keeps its prefix rather than being
> flattened to a host address) and `postgres_protocol::types::macaddr_to_sql`
> (the six raw bytes). Neither `ipnetwork` nor `mac_address` ships a `ToSql`
> impl and the orphan rule forbids adding one here, so these two wire
> formats are written by hand using the same helpers `postgres-types` uses
> for its own `cidr`/`eui48` support.
>
> `accepts` returns `true` for every Postgres type — not because every type
> is bindable, but because `accepts` is a static method with no access to
> the `Value` whose representation is the question. The acceptance decision
> is per-variant, so it lives where the variant is in hand: `to_sql`, which
> `to_sql_checked!` reaches and which runs client-side while the bind
> message is being encoded, before anything is sent. What it accepts is
> `[spec:pgorm:req:exec.cursor.binding-accepts]`.

> [spec:pgorm:req:exec.cursor.binding-coerce+2]
> When the Postgres type inferred for a placeholder is in the numeric
> family (`int2`, `int4`, `int8`, `float4`, `float8`, `numeric`),
> `ValueHolder::to_sql` MUST bind an integer- or float-valued `Value` in
> *that* type's wire format rather than in its own:
>
> - the integer variants (`TinyInt`, `SmallInt`, `Int`, `BigInt`,
>   `Unsigned`, `BigUnsigned`) convert to `int2`/`int4`/`int8` through
>   `i16`/`i32`/`i64::try_from`, and MUST return a `ToSql` error — never
>   panic — when the value does not fit; to `float4`/`float8` by `as`
>   conversion, rounding the way Postgres' own integer-to-float cast
>   does; and to `numeric` through an exact `Decimal`.
> - `BigUnsigned` (u64) reaches all of those through `i64::try_from`,
>   Postgres having no unsigned 64-bit type to write it in. The
>   conversion is checked, so a value above `i64::MAX` is the same
>   `ToSql` error as any other out-of-range integer, whatever the
>   inferred type — it MUST NOT wrap to a negative `int8`, which would
>   both store the wrong number and silently match the wrong rows. The
>   bound form therefore agrees with the inline literal rendering of
>   `[spec:pgorm:sem:sql.value.render]`, which prints the full `u64`:
>   the two spellings of a `Value` never mean different numbers.
> - `Float` and `Double` convert to `float8` by widening and to `float4`
>   by narrowing, erroring when a finite value narrows to an infinity, and
>   to `numeric` through `Decimal::try_from`, erroring when the value is
>   not representable. Binding a float against an integer type is an
>   error rather than a silent truncation.
>
> Two further integer targets are converted the same checked way rather
> than fallen through to a primitive impl: `oid` through `u32::try_from`
> and `"char"` through `i8::try_from`, so a `Unsigned` bound against `oid`
> and a `TinyInt` bound against `"char"` keep the encodings those impls
> define while a value that does not fit is an error instead of a
> reinterpretation. Every *other* inferred type is refused by
> `[spec:pgorm:req:exec.cursor.binding-accepts]`: an integer or a float has
> no representation `bytea`, `text`, `bool` or `uuid` could receive, and
> writing one anyway is what let a mismatch through. Because `Value::Array`
> binds through `Vec<ValueHolder>`, which hands each element the array's
> member type, both the conversions and the refusal apply element-wise.
>
> This is what makes an integer operand work against a floating-point
> column: `Expr::col(c).gt(2)` renders `"c" > $1`, for which Postgres
> infers `$1 :: float8`, and the `Int` value is written as a `float8`
> instead of producing an `08P01` protocol error. (An arithmetic operand
> carries its own type instead, `"c" * $1::int4`, per
> `[spec:pgorm:sem:sql.render.placeholder-typing]`, and PostgreSQL widens
> it.)

> [spec:pgorm:req:exec.cursor.binding-gaps+3]
> `ValueHolder`'s `ToSql` implementation MUST reach every `Value` variant:
> no arm may `panic!`, `unimplemented!` or `todo!`, and no arm may reach a
> panic in a delegate. The former panicking arms are gone.
> `Value::TinyUnsigned` (u8) and `Value::SmallUnsigned` (u16) no longer
> exist as variants at all (see `[spec:pgorm:def:sql.value]`), so passing
> a `u8` or `u16` is a compile error rather than a runtime panic;
> `Value::Vector`, `Value::IpNetwork` and `Value::MacAddress` bind per
> `[spec:pgorm:def:exec.cursor.binding]`.
>
> The `Array` arm is where the delegate clause bites: `Vec<T>`'s own
> `to_sql` reads its member type out of `Kind::Array` and `panic!`s on
> anything else, so the arm MUST establish that the target is an array
> itself and raise the refusal of
> `[spec:pgorm:req:exec.cursor.binding-accepts]` when it is not. A
> `Value::Array` against a scalar placeholder is reachable — an active
> model whose column type is a scalar `jsonb` but whose Rust field is a
> `Vec` produces exactly that when the JSON-flattening branch of
> `cast_enum_as` is compiled out — so this is a live path, not a
> defensive one.
>
> One vestige remains, which was never reachable: the commented-out
> time-crate arms (`TimeDate`, `TimeTime`, `TimeDateTime`,
> `TimeDateTimeWithTimeZone`). This fork's `Value` has no such variants, so
> there is nothing to bind.
>
> An inferred type outside the numeric family is no longer ignored. It was:
> every non-numeric variant used to be written in its own binary format
> whatever Postgres inferred, and the claim that the server would report
> the difference was false — `"1234"` bound to an `int4` placeholder was
> accepted by the server and read as 825373492, the integer those four
> ASCII bytes spell. The same path serves predicates and writes, so a
> mismatch selected and stored wrong values rather than erroring.
> `[spec:pgorm:req:exec.cursor.binding-accepts]` closes it.
>
> `bits_tests` used to be the standing example of the gap, where saving an
> integer into a `BIT(n)` column made Postgres infer `bit` for a parameter
> the driver wrote as an `int8` (`22P03`). It is no longer one:
> `[spec:pgorm:req:sql.render.cast-param-type]` pins a cast operand's
> placeholder to the type the value is actually written as, and the test
> runs unignored.

> [spec:pgorm:req:exec.cursor.binding-accepts+4]
> A `Value` MUST NOT be written into a placeholder whose inferred Postgres
> type its binary representation is not the wire format of.
> `ValueHolder::to_sql` decides this per variant and raises a `ToSql` error
> — `` cannot bind a `<Variant>` value to Postgres type `<type>` `` — which
> tokio-postgres surfaces while encoding the bind message, so the statement
> never reaches the server. The failure arrives as an `Error::Postgres`
> carrying no `DbError`, the same shape as any other encoding failure (see
> `[spec:pgorm:sem:error.model.sql-class]`).
>
> The question the check asks is about *representation*, not about meaning.
> A type is accepted when the bytes this variant writes are the bytes that
> type is sent in; whether they mean what the caller intended is the
> caller's business. That is why `timestamp` and `timestamptz` are
> interchangeable below and why the numeric family is a family.
>
> Two things are transparent to the decision:
>
> - **Domains.** A domain's values are sent in the representation of the
>   type it is built over, so both the acceptance check and the encoding
>   are made against the base type, unwrapped transitively. Only the
>   diagnostic names the type as the schema declares it, so a refusal on a
>   `positive_int` says `positive_int`, not `int4`.
> - **`NULL`.** A `None` payload is sent as a length of -1 with no bytes at
>   all. Having no representation, it has none to mismatch, and binds
>   against whatever type Postgres inferred.
>
> The accepted targets, per variant:
>
> | `Value` variant | accepted Postgres types |
> | --- | --- |
> | `Bool` | `bool` |
> | `TinyInt`, `SmallInt`, `Int`, `BigInt`, `Unsigned`, `BigUnsigned` | `int2`, `int4`, `int8`, `oid`, `"char"`, `float4`, `float8`, `numeric` — converted per `[spec:pgorm:req:exec.cursor.binding-coerce]` |
> | `Float`, `Double` | `float4`, `float8`, `numeric`; `int2`/`int4`/`int8` are a distinct "without loss" refusal rather than a representation mismatch |
> | `String`, `Char` | `text`, `varchar`, `bpchar`, `name`, `xml`, `unknown`; every enum (a label is sent as its own text); and `citext`, `ltree`, `lquery`, `ltxtquery` — the text-backed extension types `postgres-types` itself names. A `String` also binds against a range type a schema created, and against a multirange type a schema created whose kind a COPY's description completed, as the range or multirange its text spells (`[spec:pgorm:req:exec.cursor.binding-range]`) |
> | `Bytes` | `bytea` |
> | `Json` | `json`, `jsonb` |
> | `ChronoDate` | `date` |
> | `ChronoTime` | `time` |
> | `ChronoDateTime`, `ChronoDateTimeUtc`, `ChronoDateTimeLocal`, `ChronoDateTimeWithTimeZone` | `timestamp`, `timestamptz` |
> | `Uuid` | `uuid` |
> | `Decimal` | `numeric` |
> | `Array` | any array type; each element is then held to the member type by the same rule |
> | `Vector` | a type named `vector` |
> | `IpNetwork` | `inet`, `cidr` |
> | `MacAddress` | `macaddr` |
> | `Range` | any range type, built-in or created; each bound is then held to the subtype by the same rule (`[spec:pgorm:req:exec.cursor.binding-range]`) |
> | `Multirange` | a built-in multirange type, or a multirange type a schema created whose kind a COPY's description completed (`conn.copy.rows`); each range's bounds are then held to the subtype |
>
> All four chrono datetime variants take `timestamp` and `timestamptz`
> alike because the two share one representation — microseconds since
> 2000-01-01 — and differ only in whether that instant is read as a wall
> clock or as UTC. `postgres-types` draws the line in the same place for
> its own `SystemTime` impl. A `Value`-level check cannot tell the two
> apart and MUST NOT pretend to.
>
> Two consequences are deliberate. Postgres `money` is an `int8` on the
> wire, not a `numeric`, so a `Decimal` bound against it is refused rather
> than writing `numeric` bytes into a parameter that would read them as a
> 64-bit integer, a silent corruption turned into an error at the bind. An
> entity's `money` column, a `Decimal` field wherever codegen generates one
> (`codegen.entity.types`), never meets that refusal: its value is written
> cast to `money` from a placeholder pinned to `numeric`, and its column read
> cast to `numeric` (`entity.traits.column.enum-cast`). And a column whose `save_as` cast does not
> reach its operand — `eq_any` over a `save_as` column, or a cursor keyset
> filter, both of which build the predicate on a raw `Expr` — leaves the
> placeholder typed by the column rather than by the cast, and a variant
> that does not match it is refused where it previously produced a server
> error or wrong bytes.


> [spec:pgorm:req:exec.cursor.binding-interval]
> `ValueHolder` binds a `Value::Interval` against a placeholder whose type is
> `interval`, or a domain over it, unwrapped as every binding decision is
> (`[spec:pgorm:req:exec.cursor.binding-accepts]`), writing the sixteen
> bytes of `Interval::to_binary` (`[spec:pgorm:def:sql.value.interval]`). Any
> other type is refused before the statement is sent, with ``cannot bind a
> `Interval` value to Postgres type `<type>` ``, and a `None` payload is
> `NULL` against any type. Nothing is coerced: no other type's wire format
> holds three fields, and writing the microseconds alone into a `time` or a
> `bigint` would drop the months and the days. An array of intervals binds
> through the `Array` arm, each element against the member type.

> [spec:pgorm:req:exec.cursor.binding-range+3]
> A `Value::Range` binds against a placeholder whose type is a range — the
> built-in six, or a range type a schema created, which tokio-postgres
> reports as a range over its subtype all the same — and a
> `Value::Multirange` against a multirange whose kind names its subtype: a
> built-in one, or one a schema created whose kind a COPY's description
> completed (`conn.copy.rows`); anything else is refused before the
> statement is sent (`[spec:pgorm:req:exec.cursor.binding-accepts]`). A
> `None` payload is `NULL`. The range is written through
> `postgres_protocol::types::range_to_sql` — the empty range as its flag byte
> alone, otherwise a flag byte saying which sides are inclusive or unbounded
> and each present bound length-prefixed — and each bound is written by
> `ValueHolder` itself against the subtype, so a bound gets every coercion
> and every encoding fix a scalar of its variant gets: an `int8` value
> narrowed to an `int4range`'s subtype, a naive datetime at the ends of the
> calendar, a zero `numeric` keeping its scale
> (`[spec:pgorm:def:exec.cursor.binding]`), and a bound of the wrong variant
> refused by that bound's own rule. A `NULL` bound is written as no bound,
> because that is what PostgreSQL's constructors read it as and what the
> literal rendering writes (`[spec:pgorm:def:sql.value.range]`); sent as a
> `NULL`, its length of -1 would be read by the range's receive function as
> a length.
>
> A `Value::String` binds against a range type a schema created as the range
> its text spells. Such a range's `Value` is its text form
> (`[spec:pgorm:def:sql.value.created-range]`): the `Value` the newtype
> naming it converts into and the one a binary COPY reads its column as
> (`conn.copy.rows`). The text is read by `FromStr` on `Range<T>`, `T` the
> `RangeSubtype` the subtype reads as, and the range is then written as a
> `Value::Range` is, each bound against the subtype. Against a multirange a
> schema created whose kind a COPY's description completed, the text is read
> by `FromStr` on `Multirange<T>` and written as a `Value::Multirange` is.
> Text that spells no range or multirange of that subtype, and a subtype no
> `RangeSubtype` reads, are refused before the statement is sent, as a
> mismatch naming the type. Against a built-in range a `String` is a
> mismatch: those six have `Value::Range`. The
> parse is pgorm's, reading the spellings the Rust type reads, where the
> server's input function reads every spelling the type takes, which is why
> the ORM writes a created range as its text cast to the type.
>
> A multirange is written as a count and then each range length-prefixed in
> a range's own encoding, against the subtype the multirange's kind names. A
> statement's placeholder of a multirange a schema created cannot be bound in
> binary: tokio-postgres learns a type's kind from `pg_range` joined on the
> *range* type, so the multirange arrives as a simple type with no subtype to
> write the bounds in, and guessing one would send bytes the server could
> read as another type. A value of one is written as its text cast to its
> name instead (`[spec:pgorm:def:sql.value.created-range]`), and a
> `Value::String` or a `Value::Multirange` against such a placeholder is
> refused here as a mismatch. Only a COPY's columns, which pgorm describes
> before the COPY starts, carry the subtype the catalogue completes.
>
> PostgreSQL types a placeholder beside a range column by the column: in
> `r @> $1`, `$1 <@ r` and `r && $1` it infers the range type, so an element
> written as `col.contains(3)` is refused as an `Int` against `int4range`.
> The element test pins its operand — `Expr::val(3).cast_as(alias("int4"))`
> — and then reaches the element form of the operator.

## Offset pagination (`exec.paginator`)

> [spec:pgorm:def:exec.paginator+3]
> `Paginator<'db, C, S>` holds the `SelectStatement` to page over, a
> zero-based current `page`, a `page_size`, a borrowed connection, and a
> phantom selector. It MUST NOT hold a failure in place of that
> statement: a paginator that exists is one that pages, and every reason
> a source could not be turned into a statement is settled before one is
> constructed.
> `ItemsAndPagesNumber` carries `number_of_items` and `number_of_pages`.
> `PaginatorTrait::paginate(db, page_size)` constructs a paginator
> infallibly and is implemented for exactly the sources whose statement
> the builder produced and which therefore cannot fail to page:
> `Selector<S>`, `Select<E>` (via `into_model`), and `SelectGraph<E, S>`
> (via its own selector, `[spec:pgorm:sem:query.graph.terminals]`). The
> trait also provides `count`, defined as `paginate(db, 1).num_items()`.
>
> `SelectorRaw<S>` — whose statement is the caller's own text — MUST NOT
> implement the trait, and paginates instead through its own inherent
> `paginate(db, page_size) -> Result<Paginator, Error>` and
> `count(db) -> Result<u64, Error>`
> (`[spec:pgorm:sem:exec.paginator.raw]`). The distinction is carried by
> the type rather than by a `Result` on every call site: a builder-backed
> caller has no failure to handle and is not made to write one, and a raw
> caller cannot reach a page without having handled theirs.
> `PinBoxStream` is the pinned boxed stream alias returned by
> `into_stream`.

> [spec:pgorm:req:exec.paginator.page-size+2]
> `page_size` is a `NonZeroU64` — in the `paginate` signature and in the
> `Paginator` field it is stored in — so a zero page size is not a value
> a caller can supply and not a state the paginator can hold. There is
> no assertion to trip, no panic to catch and no `Error` to recover:
> zero is rejected by the compiler at the call site. The page-count
> division of `[spec:pgorm:sem:exec.paginator.count]` is therefore total
> by construction, and `PaginatorTrait::count` names its page size of
> one as `NonZeroU64::MIN` rather than a literal the type would refuse.

> [spec:pgorm:sem:exec.paginator.fetch+3]
> `fetch_page(page)` executes a clone of the query with
> `LIMIT page_size OFFSET page_size * page`; pages are zero-indexed and
> the paginator's own cursor is not consulted or advanced. The offset is
> computed with checked multiplication: a `page` whose offset would not
> fit a `u64` is an `Error::Query`, not a debug-build panic and not a
> release-build wrap to a small offset that would silently serve the
> wrong rows. Rows are decoded through the selector, aborting on the
> first decode error. `fetch()` is `fetch_page(cur_page())`; `cur_page()`
> reports the counter, starting at 0.
>
> The step that moves the counter without fetching MUST be called
> `advance()`, not `next()`. It performs no I/O, returns nothing, and is
> not `Iterator::next`; sitting beside `fetch_and_next`
> (`[spec:pgorm:sem:exec.paginator.iterate]`), which is the call that
> does read, a `next()` reads at a call site as the fetch it is not.

> [spec:pgorm:sem:exec.paginator.count]
> `num_items` counts by wrapping the paginator's query — with limit,
> offset, and `ORDER BY` stripped — as a subquery aliased `sub_query`
> under `SELECT COUNT(*) AS num_items`, decoding the count as `i64` and
> casting to `u64`; if the count query yields no row the result is 0.
> `num_pages` and `num_items_and_pages` derive the page count as
> `items / page_size + (items % page_size > 0)` — ceiling division, so a
> partial trailing page counts as a page and zero items yield zero
> pages. Each call re-runs the count query; item and page numbers are
> not cached.

> [spec:pgorm:sem:exec.paginator.iterate]
> `fetch_and_next` fetches the current page, then increments the page
> counter, returning `Some(items)` when the page is non-empty and `None`
> when it is empty. Termination is therefore detected only by an empty
> fetch: a result set that is an exact multiple of `page_size` costs one
> extra query returning zero rows. `into_stream` wraps this loop as an
> async stream yielding `Ok(Vec<S::Item>)` per non-empty page; the
> stream ends at the first empty page and yields the error (then ends)
> if any fetch fails.

> [spec:pgorm:sem:exec.paginator.raw+7]
> Paginating a `SelectorRaw` MUST decide what the raw statement is by
> parsing it with libpg_query — the PostgreSQL server's own parser, the
> same `pg_query` revision, on PostgreSQL 18's grammar, that the render
> oracle and `sql!` use (`[spec:pgorm:req:sql.render.oracle]`,
> `[spec:pgorm:def:macros.sql]`) — and MUST NOT decide it by inspecting
> the statement's text. The
> statement is accepted only when it parses, holds exactly one statement,
> and that statement is a `SelectStmt` carrying no `INTO` clause. A
> `WITH ... SELECT` therefore qualifies: PostgreSQL hangs the `WITH`
> clause off the `SelectStmt` itself rather than making it a statement of
> its own, so a CTE pages like any other `SELECT`. `VALUES (…), (…)` and
> set operations qualify for the same reason — both parse as a
> `SelectStmt`. This check stays in the ORM because libpg_query is the
> ORM's dependency and deliberately not the query builder's.
>
> An accepted statement is taken at the extent the parser reports for it,
> which excludes any terminating `;` that a subquery position would
> refuse. PostgreSQL 18's parser starts that extent at the statement's
> first token, so whitespace and comments leading the statement are not
> carried into the wrapper: they lie outside the statement, as the `;`
> does. A comment trailing the statement lies inside its extent and is
> carried. The extent is then paired
> with its bind values as a `SqlTemplate`
> (`SqlTemplate::from_sql`, `[spec:pgorm:req:sql.render.custom-expr]`)
> and placed in relation position as
> `FromItem::Template(fragment, "sub_statement")`
> (`[spec:pgorm:def:sql.types.table-ref]`). What the paginator holds is
> then ONE shape — an ordinary `SelectStatement` — for a built source and
> a raw one alike: the page clauses of `exec.paginator.fetch` and the
> count wrapper of `exec.paginator.count` are the builder's, and the
> paginator MUST NOT format SQL text of its own. The rendering, including
> the newline before the closing parenthesis that keeps a trailing `--`
> comment from swallowing it, belongs to
> `[spec:pgorm:req:sql.render.subquery]`. Wrapping rather than splicing
> means `LIMIT` and `OFFSET` land outside the caller's own clauses instead
> of colliding with them, so a raw statement that already carries
> `ORDER BY` or `LIMIT` still pages correctly; PostgreSQL will not reorder
> rows a subquery sorted, so the caller's `ORDER BY` still governs page
> boundaries.
>
> The statement's non-marker text MUST be reproduced verbatim; its `$N`
> markers MUST NOT be. Each marker is paired with its value at
> construction and re-emitted as a parameter of the enclosing statement,
> so the fragment renumbers into the builder's `$N` space in
> first-reference order, one parameter per reference: a statement reading
> `$2` before `$1` comes back reading `$1` before `$2` with the values
> permuted to match, and one reading `$1` twice comes back reading two
> parameters that both hold it. Renumbering rather than preserving the
> caller's numbers is what makes the fragment COMPOSE — it no longer has
> to be the only thing in the statement that binds anything — and it is
> what lets the page clauses be appended by the builder rather than
> counted out by hand.
>
> Which `$N` are markers is settled by the `sql.token` tokenizer, which
> reads `--` and nested `/* */` comments as one `Space` token and
> dollar-quoted bodies (tagged and untagged) and `E'…'` strings as one
> `Quoted` token (`[spec:pgorm:sem:sql.token.limits]`) — the same
> opacity PostgreSQL's own scanner has, and the reason a second scan of
> the text is no longer needed. Comment bodies, dollar-quoted strings,
> single-quoted and E-string literals, and bracketed subscripts therefore
> read the same paginated as they do executed directly, and a `$99`
> written inside any of them stays text.
>
> The marker census MUST be exact and MUST be settled at `paginate`
> rather than by the server. A marker the caller supplied no value for is
> refused, naming the marker and how many values were supplied; a
> supplied value nothing in the statement reads is refused too, naming
> the value — the server would refuse that bind anyway, and refusing here
> reports it once instead of once per page.
>
> Everything else — text the grammar rejects, a `;`-separated script, an
> `INSERT`/`UPDATE`/`DELETE`/DDL statement, a `SELECT ... INTO` — is
> neither a panic nor mangled SQL sent to the server.
>
> Every one of these refusals MUST be returned by `paginate` itself, as an
> `Error::Query` naming what the statement actually parsed as — using
> PostgreSQL's own node name for anything it has no SQL keyword for — and
> MUST NOT be recorded in the constructed paginator for a later reader to
> replay. `SelectorRaw`'s `paginate` therefore returns
> `Result<Paginator, Error>` and its `count` returns `Result<u64, Error>`,
> which is why `SelectorRaw` does not implement `PaginatorTrait`
> (`[spec:pgorm:def:exec.paginator]`). The failure belongs to the caller
> who supplied the statement, at the call that supplied it: a paginator
> handed back to that caller has already been proven to page, so
> `fetch_page`, `fetch`, `num_items`, `num_pages` and `into_stream`
> report only what the *database* answered, and a statement that was never
> pageable cannot reach any of them. Deferring it instead would make a
> `paginate` that returns no `Result` a claim the type could not keep,
> and would report one authoring mistake once per reader rather than once.
>
> The wrapping is public as `raw_select(sql, values)`, which gives the
> `SelectStatement` a `SelectorRaw` pages and counts, or the same refusal,
> for a caller that sends the page or the count itself: `page_statement` and
> `count_statement` take it, as a language binding's raw statement does
> (`[spec:pgorm:req:bindings.raw-pages]`).
