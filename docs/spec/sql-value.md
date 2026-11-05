# SQL Value and Base Type Vocabulary

This section specifies the value container (`pgorm-query/src/value.rs`) and the
base type vocabulary (`pgorm-query/src/types.rs`, plus `ColumnType`/`StringLen`
from `pgorm-query/src/table/column.rs`) that the query builder, DDL builders and
codegen all share. Rules capture current behaviour of the PostgreSQL-only fork,
including panic semantics and quirks inherited from sea-query.

## The Value container

> [spec:pgorm:def:sql.value+3]
> `Value` is the single enum container for all SQL values. Every variant wraps
> an `Option` of its payload; `None` encodes SQL NULL while preserving the type
> tag. Payloads larger than one pointer are boxed so the enum stays small:
> `Bool(Option<bool>)`, `TinyInt(i8)`, `SmallInt(i16)`, `Int(i32)`,
> `BigInt(i64)`, `Unsigned(u32)`, `BigUnsigned(u64)`, `Float(f32)`,
> `Double(f64)`, `String(Box<String>)`,
> `Char(char)`, `Bytes(Box<Vec<u8>>)`, `Json(Box<serde_json::Value>)`,
> `Date(Box<civil::Date>)`, `Time(Box<civil::Time>)`,
> `DateTime(Box<civil::DateTime>)`,
> `DateTimeWithTimeZone(Box<jiff::Timestamp>)`, `Uuid(Box<Uuid>)`,
> `Decimal(Box<Decimal>)`, `Array(ArrayType, Option<Box<Vec<Value>>>)`,
> `Vector(Box<pgvector::Vector>)`, `IpNetwork(Box<IpNetwork>)`,
> `MacAddress(Box<MacAddress>)`, and the two range carriers of
> `[spec:pgorm:def:sql.value.range]`,
> `Range(RangeType, Option<Box<Range<Value>>>)` and
> `Multirange(RangeType, Option<Box<Multirange<Value>>>)`, which carry their
> range type beside the payload as `Array` carries its element type.
>
> There are no `u8` or `u16` variants. Postgres has no unsigned integer
> types, so the MySQL-era `TinyUnsigned`/`SmallUnsigned` spellings — whose
> binding path panicked at runtime — were removed outright; a `u8` or `u16`
> is now a compile error wherever a `Value` is required. The two surviving
> unsigned variants both have a Postgres meaning: `Unsigned` (u32) is the
> `OID` carrier (see `[spec:pgorm:sem:exec.decode.u32-oid]`) and
> `BigUnsigned` (u64) is how `LIMIT`/`OFFSET` counts reach the builder.
>
> Unlike upstream sea-query, none of these variants are feature-gated in this
> fork: jiff, serde_json, rust_decimal, uuid, ipnetwork, mac_address and
> pgvector are unconditional dependencies of `pgorm-query`, so every variant is
> always compiled in.
>
> There is no `Value` → `serde_json::Value` conversion. The inherited
> `sea_value_to_json_value` MUST NOT exist, under that name or any other: it
> had no in-tree caller, panicked on non-UTF-8 `Bytes`, turned a temporal `None`
> into the JSON string `"NULL"` rather than JSON null, and rendered a temporal
> payload as its quoted SQL literal — answers wrong often enough that reviving
> it would be a defect rather than a restoration.
>
> `Value` implements `PartialEq`, `Eq` and `Hash`, and the three MUST agree:
> a `Value` is a map key, so `a == b` has to imply `hash(a) == hash(b)` and
> every value has to equal itself. The float-carrying variants therefore
> compare *bitwise*, by the same `to_bits()` the hash uses — `Float` and
> `Double` on their payloads, `Vector` element-wise over its `f32` slice, with
> a `None` payload equal only to another `None` (and hashed as a zero byte).
> `Array` compares its element tag and then its elements through this same
> impl, so a float nested in an array is compared bitwise too, and `Range` and
> `Multirange` compare their range type and then their bounds the same way.
> Every other
> variant compares structurally, and values of two different variants are
> never equal. `Display` renders the value as a Postgres SQL literal by
> delegating to `QueryBuilder.value_to_string`.
>
> Bitwise comparison is a deliberate divergence from both IEEE 754 and
> PostgreSQL float equality, in the two places they disagree with each other:
> `NaN` equals itself (IEEE says no; PostgreSQL's `=` operator also says yes),
> and `0.0` and `-0.0` are *distinct* (both IEEE and PostgreSQL say they are
> equal). The alternative — IEEE equality with a bitwise hash, as the derived
> `PartialEq` gave — breaks both `Eq`'s reflexivity and the hash contract, and
> a `HashMap` keyed on such a value has undefined lookup behaviour rather than
> merely surprising results.
>
> The divergence is observable where a `Value` is a map key over live column
> data — `LoaderTrait`'s `HashMap<ValueTuple, _>`
> (`[spec:pgorm:sem:query.loader.regroup]`) — because the server and the map
> then disagree about `-0.0`: a row PostgreSQL matched on `a.k = b.k` can fail
> to find its bucket, surfacing as the unmatched-key error rather than as a
> wrong answer. A float is a poor relation key for that reason, and this rule
> makes the failure loud instead of leaving it to a hash contract violation.

> [spec:pgorm:def:sql.value.conversions+1]
> `From<T> for Value` covers the Rust primitives one-to-one: `bool`→`Bool`,
> `i8`→`TinyInt`, `i16`→`SmallInt`, `i32`→`Int`, `i64`→`BigInt`,
> `u32`→`Unsigned`, `u64`→`BigUnsigned`, `f32`→`Float`, `f64`→`Double`,
> `char`→`Char`. `u8` and `u16` have no conversion, no `Nullable` and no
> `ValueType` impl — they are not part of the value vocabulary at all.
> `&str`, `&String`, `String` and `Cow<'_, str>` all convert to `String`
> (owned, boxed); `&[u8]` and `Vec<u8>` convert to `Bytes`. `serde_json::Value`,
> `NaiveDate`, `NaiveTime`, `NaiveDateTime`, `Decimal`, `Uuid`, `IpNetwork`,
> `MacAddress` and `pgvector::Vector` convert to their same-named variants.
>
> `jiff::Timestamp` maps to `DateTimeWithTimeZone`. It carries an absolute
> instant and no offset, which is what `timestamptz` stores, so there is no
> offset-bearing spelling to normalize. The `uuid::fmt` wrapper types
> (`Braced`, `Hyphenated`, `Simple`, `Urn`) convert into the plain `Uuid`
> variant via `into_uuid()`; converting back out re-applies the corresponding
> format accessor.
>
> `From<Option<T>>` exists for any `T: Into<Value> + Nullable`: `Some(v)`
> converts `v`, `None` produces the typed null `T::null()`. There is no
> `From<Vec<u8>> for Value::Array` — `u8` vectors always become `Bytes` (see
> `[spec:pgorm:def:sql.value.array]`).

> [spec:pgorm:def:sql.value.value-type+5]
> `ValueType` is the extraction/reflection trait implemented by every Rust type
> that maps into `Value`. `from_value(v: Value) -> Result<Self, ValueTypeError>`
> succeeds only when `v` is the matching variant with a `Some` payload
> (`ValueTypeError` displays as "Value type mismatch") and is the trait's only
> extraction entry point: there is no panicking `unwrap`/`expect` pair, on the
> trait or as an inherent convenience on `Value`. Its name is its own: a
> method named `try_from` made `Decimal::try_from(value)` ambiguous with the
> language prelude's `TryFrom::try_from` (`E0034`) wherever the trait was
> imported, and one named `try_from_value` would be ambiguous on every
> `ActiveEnum` beside that trait's `try_from_value(&Self::Value)`, the two
> traits often imported together. `Decimal::from_value(value)` therefore
> resolves as written.
> `type_name()` returns the Rust type name, `array_type()` the matching
> `ArrayType` tag, and `column_type()` the default `ColumnType` for schema
> generation (e.g. `String`→`String(StringLen::None)`, `Vec<u8>`→`Bytea`,
> `char`→`Char(Some(1))` — one character, where `Char(None)` is the unbounded
> `bpchar` — `Decimal`→`Decimal(None)`, `i8`/`i16`→`SmallInteger`,
> `u32`/`u64`→`BigInteger` — the `int8` those values bind as —
> `civil::DateTime`→`Timestamp`, `jiff::Timestamp`→`TimestampWithTimeZone`,
> `IpNetwork`→`Inet`, `MacAddress`→`MacAddr`, `Vector`→`Vector(None)`). The
> `ColumnType` a Rust type maps to MUST be one Postgres actually has: no
> mapping may name a width or signedness the server will not honour.
>
> `Nullable` provides `null() -> Value`, the typed-NULL constructor used by the
> `Option` conversions. `ValueType for Option<T>` returns `Ok(None)` when the
> input equals `T::null()` and otherwise delegates to `T::from_value`, so a
> `None` payload of the right variant round-trips to `Option::None` while a
> wrong-variant value still errors.

> [spec:pgorm:sem:sql.value.accessor-panics+2]
> The inherent accessors on `Value` MUST NOT panic. (The rule keeps the id it
> was given when they did: every `as_ref_*` panicked with a message like
> `not Value::Json` on a variant other than its own.)
>
> The `is_*`/`as_ref_*` pairs are `is_json`/`as_ref_json`,
> `is_date`/`as_ref_date` and the other three temporal accessors,
> `is_decimal`/`as_ref_decimal`, `is_uuid`/`as_ref_uuid`,
> `is_array`/`as_ref_array`, `is_ipnetwork`/`as_ref_ipnetwork` and
> `is_mac_address`/`as_ref_mac_address`. Each `as_ref_*` returns `Option<&T>`
> borrowed from the payload, and its `None` is ambiguous by design: it means
> either SQL NULL of the accessor's own variant or a value of some other
> variant entirely, and the return value cannot distinguish them. The `is_*`
> predicate of the pair is the discriminator for a caller that needs the
> distinction — `is_array` is true for an `Array` of any element tag — and
> `ValueType::from_value` is the typed extraction that reports a mismatch as
> `ValueTypeError`.
>
> `as_naive_utc_in_string` stringifies any non-NULL temporal variant, in
> the UTC-naive form for the three zoned ones, and returns `None` for both a
> NULL temporal variant and a non-temporal one. `as_ipaddr` returns the network
> address of a non-NULL `IpNetwork` and `None` otherwise. `decimal_to_f64`
> returns the payload of a non-NULL `Decimal` through `to_f64`, and `None` if
> the value is not a `Decimal`, is NULL, or has no `f64` representation.

## Arrays

> [spec:pgorm:def:sql.value.array+6]
> `ArrayType` is the element-type tag carried by `Value::Array`; its variants
> mirror the scalar `Value` variants (`Bool` through `Bytes`, `Json`, the six
> temporal tags, `Uuid`, `Decimal`, `IpNetwork`, `MacAddress`, `Vector`), plus
> `Range(RangeType)` and `Multirange(RangeType)`, the tags of an array of
> ranges (`int4range[]`) and of multiranges. There is no nested-array tag.
> `ValueType::array_type()` is total — `pgvector::Vector` answers
> `ArrayType::Vector` rather than panicking — but `Vector` does not implement
> `NotU8`, so `Vec<Vector>` has no `From`/`ValueType` impl and that tag reaches
> a `Value` only through `Value::array`, which names its element type and needs
> no marker. `Range<T>` and `Multirange<T>` do implement it, for every
> `T: RangeElement`, so `Vec<Range<i32>>` is `int4range[]` and
> `Vec<Multirange<Date>>` is `datemultirange[]`, tagged
> `ArrayType::Range(RangeType::Int4)` and `ArrayType::Multirange(..)`
> (`sql.value.range`).
>
> `Vec<T>` converts to `Value::Array(T::array_type(), ...)` only for `T`
> implementing the `NotU8` marker trait (all supported element types except
> `u8`), because `Vec<u8>` is claimed by the `Bytes` conversion. `Nullable for
> Vec<T>` produces `Array(tag, None)`. `ValueType for Vec<T>` requires the
> stored tag to equal `T::array_type()` and then converts every element through
> `T::from_value`, so a mismatched element inside the vector surfaces as
> `ValueTypeError` rather than panicking.
> `ValueType::column_type()` for `Vec<T>` is
> `ColumnType::Array(Arc::new(T::column_type()))`.
>
> `Value::array(values)` gathers any `IntoIterator` of `V: Into<Value> +
> ValueType` into one `Value::Array`, taking the element tag from
> `V::array_type()` rather than from the elements. Reading the tag from the Rust
> type is what makes an empty collection representable: an untagged empty array
> has no element to infer a type from, and so no inline spelling PostgreSQL can
> resolve. It is the constructor behind the array-parameter predicates
> (`[spec:pgorm:req:sql.ast.expr.eq-any]`) and, unlike `From<Vec<T>>`, needs no
> `NotU8` bound, the caller having already named the element type.

## Value kinds

> [spec:pgorm:def:sql.value.kinds]
> Each `ArrayType` is a kind of value, and each kind has one name, the one
> pgorm's language bindings name it by (`[spec:pgorm:req:bindings.values]`)
> and the generative campaign's wire format carries. `ArrayType::kind_name`
> answers `bool`, `i8`, `i16`, `i32`, `i64`, `u32`, `u64`, `f32`, `f64`,
> `text`, `char`, `bytes`, `json`, `decimal`, `uuid`, `date`, `time`,
> `datetime`, `datetime_utc`, `ipnetwork`, `mac_address`, `vector` and
> `interval` for the scalar tags, and a range's or a multirange's catalogue
> name for `Range` and `Multirange` (`int4range`, `tstzmultirange`).
> `ArrayType::from_kind_name` reads a name back exactly, `None` for any other
> text, and `ArrayType::kinds` yields every kind once: the scalar tags, then
> each built-in range type's range followed by its multirange.
>
> `ArrayType::null` is SQL NULL of a kind, `Value::array_type` the kind a
> value is, an array's being the tag its elements carry (`Value::is_array`
> tells the two apart), and `Value::is_null` whether a value is SQL NULL:
> JSON's `null` and an empty array are present values.
> `RangeType::element_type` is the kind a built-in range's bounds are, `Int`
> for `int4range`.
>
> Every one of these is total. Inside pgorm-query `ArrayType`, `Value` and
> `RangeType` are matched without a wildcard arm, so a variant one of them
> gains is named here before the crate builds, and a binding reading a kind
> through these never meets one it cannot name. The bindings and the replay
> harness take their kinds from here rather than from tables of their own.

## Ranges

> [spec:pgorm:def:sql.value.range+5]
> A range value is `Range<T>`, an enum of two shapes: `Empty`, and
> `Bounds { lower, upper }` with each bound a `std::ops::Bound<T>` —
> `Included`, `Excluded` or `Unbounded`. The empty range is a value of its
> own and MUST NOT be spelled as a pair of bounds, because no pair of bounds
> is equal to it: the server writes `[5,5)` back as `empty`, `(,)` is the
> range of every value, and a decode that read the empty flag as two
> unbounded sides would turn a range containing nothing into one containing
> everything. An unbounded side is not a side bounded by a subtype's own
> `infinity` either: `[2024-01-01,infinity)` keeps `infinity` as a bound
> value (`upper_inf` is false), and decoding it fails exactly as decoding
> that `infinity` outside a range does, rather than reading it as
> `Unbounded`.
>
> `Range::new(lower, upper)` builds the bounds shape, `is_empty`, `lower` and
> `upper` read it (`None` for the empty range), and `map` converts each
> bound's value keeping its inclusivity. The standard library's ranges
> convert to the bounds they spell: `a..b` is `[a,b)`, `a..=b` is `[a,b]`,
> `a..` is `[a,)`, `..b` is `(,b)`, `..=b` is `(,b]` and `..` is `(,)`.
> Nothing on the Rust side evaluates the bounds: equality is structural, and
> canonicalisation and ordering are the server's. A discrete range comes back
> in its canonical form — `int4range` `[1,5]` reads back as `[1,6)`, and `(1,5]`
> as `[2,6)` — while a continuous one (`numrange`, `tsrange`, `tstzrange`)
> comes back as written; inverted bounds are refused (`22000`), as is a
> discrete bound whose canonical form overflows (`int4range` `[1,2147483647]`,
> `22003`).
>
> A multirange is `Multirange<T>`, a list of ranges built from a `Vec`, an
> iterator or `Default` (the empty multirange), read by `iter` and
> `IntoIterator`. It is its own type rather than a `Vec<Range<T>>` because
> PostgreSQL has both and they are different types: a `Vec` of ranges is an
> array, `int4range[]`, and converts, binds and decodes as one
> (`sql.value.array`, `exec.decode.range`) — the server keeps an array's
> elements in the order written, an empty range among them, and canonicalises
> each as it would a range alone. The server stores a multirange sorted, with
> overlapping and adjacent ranges merged and empty ones dropped, so
> `{[5,8), [1,3), [2,4), empty}` reads back as `{[1,4),[5,8)}`.
>
> `RangeType` names which of PostgreSQL's six built-in range types a range
> is, by the subtype it ranges over: `Int4`, `Int8`, `Numeric`, `Date`,
> `Timestamp` and `TimestampTz`, whose `range_type_name` and
> `multirange_type_name` are `int4range`/`int4multirange` through
> `tstzrange`/`tstzmultirange`. The set is closed because PostgreSQL's is.
> `RangeElement` is the sealed trait of the Rust types those six range over —
> `i32`, `i64`, `Decimal`, `civil::Date`, `civil::DateTime` and
> `jiff::Timestamp` — and answers the `RangeType`; it is sealed because the
> answer is one of the six, and a type with no built-in range would have to
> name one it is not. Each is a `RangeSubtype`
> (`[spec:pgorm:def:sql.value.created-range]`), the wider sealed set a range
> type a schema created can range over.
>
> For `T: RangeElement`, `Range<T>` and `Multirange<T>` convert into
> `Value::Range` / `Value::Multirange` tagged with `T::range_type()`, each
> bound becoming the `Value` of `T`'s own variant; `Nullable` gives the
> tagged `NULL`, and `ValueType` inverts the conversion, refusing another
> range type's tag — the tag is what refuses an empty range of another
> subtype, which has no bound to tell it by. `column_type()` is
> `ColumnType::Range(T::range_type())` or `ColumnType::Multirange(..)`, and
> `array_type()` the matching `ArrayType` tag.
>
> A `Range<Value>` built by hand can carry a `NULL` bound, which no typed
> range can. It MUST mean no bound, everywhere: that is how PostgreSQL's
> range constructors read a `NULL` argument (`int4range(NULL, 5)` is
> `(,5)`), so the literal rendering
> (`[spec:pgorm:sem:sql.value.render]`), the binding
> (`[spec:pgorm:req:exec.cursor.binding-range]`) and `ValueType`'s
> extraction all read it so and the two renderings of one value agree.
>
> A range type created by `CREATE TYPE ... AS RANGE`
> (`[spec:pgorm:req:sql.ddl.type-range]`) has a name only its schema knows,
> so `RangeType` has no variant for it: a value of one is its text form cast
> to the type by name, and a column of one is `ColumnType::CreatedRange`
> (`[spec:pgorm:def:sql.value.created-range]`). A `Value::Range` of a
> built-in still binds against a placeholder the server types as such a
> range over its subtype, because the wire format of a range is its
> subtype's, and a row of one decodes through `Range<T>`.
>
> Under pgorm-query's `serde` feature, which pgorm's `with-json` turns on,
> `Range<T>` and `Multirange<T>` implement `Serialize` and `Deserialize`
> wherever `T` does, in the form serde gives the types' own shapes, so every
> serde format can carry them. The empty range is the unit variant `Empty`
> (`"Empty"` in JSON), and a range of bounds the struct variant `Bounds` with
> its `lower` and `upper` fields, each written as serde writes any
> `std::ops::Bound`: `"Unbounded"`, `{"Included":v}` or `{"Excluded":v}`. So
> `[1,5)` is `{"Bounds":{"lower":{"Included":1},"upper":{"Excluded":5}}}`,
> and `(,)` is `{"Bounds":{"lower":"Unbounded","upper":"Unbounded"}}`,
> distinct from `"Empty"` and from the `null` of an absent `Option<Range<T>>`.
> A multirange is its list of ranges, a JSON array, in the order written and
> with an empty range kept. A bound's value is `T`'s own serde form — a
> `Decimal` as rust_decimal writes it, exact text under `with-json`, and a
> jiff value as its ISO text. Deserializing refuses a missing or an unknown
> field of `Bounds`. A present bound is always an object holding its value,
> so no value of `T` reads as no bound. The Python binding's snapshot writes
> no bound as `null` beside a bracket string, which is unambiguous there only
> because no snapshot payload is `null`; serde_json writes a non-finite float
> as `null`, so that form would read `[0,Infinity)` back as `[0,)`, where
> this one fails to deserialize it. The newtype codegen generates for a range
> type a schema created derives the same serde traits under `with_serde`
> (`[spec:pgorm:def:codegen.entity.serde]`), a newtype struct, which JSON
> writes as the range it holds.
>
> Both language bindings carry the six and their multiranges, and the range
> types a schema created, as their `Range` and `Multirange`, each bound
> converted with its subtype kind's limits and the kind never inferred
> (`bindings.values`, `bindings.created-ranges`); Python's classes are
> `python.values`'s, and its snapshot of one `python.value-tags`'s.

## Range types a schema creates

> [spec:pgorm:def:sql.value.created-range+9]
> A range type a schema creates with `CREATE TYPE ... AS RANGE`
> (`[spec:pgorm:req:sql.ddl.type-range]`) is named by that schema, so no
> `RangeType` variant can carry it, and `Value` does not: it stays one pointer
> of payload wide. A value of one travels as its *text form* cast to the type
> by name. The Rust side names the type once, in a newtype over `Range<T>`
> deriving `CreatedRange` (`[spec:pgorm:sem:macros.derive.created-range]`)
> with `range_name` and an optional `schema_name`, as an enum type is named
> once by its `ActiveEnum`. A bare `Range<T>` field naming the type in an
> attribute is not the shape, because `ValueType::column_type()` of a
> `Range<f64>` would have no correct answer without a name, and the entity
> derives reach a field only through its `ValueType`. The newtype implements
> `pgorm::CreatedRange`, whose `name()` is the type's `TypeName` and whose
> `into_expr()` is the value cast to it.
>
> `RangeSubtype` is the sealed set of Rust types such a range ranges over:
> `i16`, `i32`, `i64`, `f32`, `f64`, `Decimal`, `String`, `civil::Date`,
> `civil::Time`, `civil::DateTime`, `jiff::Timestamp` and `Uuid` — the
> scalars PostgreSQL can range over (each type has a default b-tree operator
> class) whose text form the type's input function reads back as the same
> value and whose binary form a `FromSql` of the Rust type decodes. The rest
> are left out: `bool` has two values to range over, `money` has no `Value` of
> its own, `interval` has no text parsing in pgorm — the server writes one in
> the session's `IntervalStyle`, and a created multirange is read back as its
> text — and `inet` and `macaddr` decode through pgorm's own newtypes rather
> than a `FromSql` the range reader can use.
>
> The text form is `Display` and `FromStr` on `Range<T>` and `Multirange<T>`
> for `T: RangeSubtype`. `Display` writes `empty`, or a bracket, each bound
> and a bracket, as PostgreSQL's range output does: an unbounded side, and a
> `NULL` bound — no bound here as everywhere — is nothing, and a bound is its
> subtype's text, written as it is unless it is empty or holds a quote, a
> backslash, a bracket, a parenthesis, a comma or whitespace, when it is
> double-quoted with each `"` and `\` doubled; the range parser would
> otherwise read those as syntax or keep them around the value. A float is
> Rust's shortest round-trip spelling (`inf`, `NaN` and `-0` included), a
> temporal bound the literal rendering's text truncated to the microsecond,
> an instant with its offset. `FromStr` reads what PostgreSQL's range input
> reads — surrounding whitespace, a case-insensitive `empty`, an empty side as
> no bound, a quoted bound with backslash escapes and doubled quotes — and
> each bound with the subtype's own text parsing, anything else being a
> `ValueTypeError`. For a continuous range whose bounds print alike the text
> is the server's own; where a bound prints differently (`1e300` as `1e+300`,
> `inf` as `Infinity`) the server's text reads back as the same value.
>
> `Expr::as_range(type_name)` is the cast. It writes a `Value::Range` or
> `Value::Multirange` operand as its text — a `NULL` staying `NULL` — and an
> array of them as the array of their texts, each `NULL` element staying
> `NULL`, and casts the operand to the `TypeName`, quoted and schema-qualifiable as every
> type name is (`[spec:pgorm:def:sql.types.type-name]`); any other operand,
> such as the text the newtype converts into, is cast as it is. There is no
> cast between two range types (`CAST(int4range(1, 5) AS slot)` is `42846`),
> nor between their arrays (`cannot cast type int4range[] to slot[]`), so
> text is the one form every range type reads: `CAST($1::text[] AS slot[])`
> reads an array of texts as one text casts to the type. A text operand is a bound
> parameter pinned to `text`, `CAST($1::text AS floatrange)`, so the server's
> input function reads it, every spelling the type takes; a bare `$1` there is
> typed as the range, and a `String` bound to it is read by pgorm instead,
> through the subtype's Rust parsing
> (`[spec:pgorm:req:exec.cursor.binding-range]`). Inline it is an escaped
> string literal, and the two renderings are one value. A bound holding the
> literal's syntax, a quote or a placeholder is read back as the text it is.
>
> A column of the type is `ColumnType::CreatedRange { name, schema, subtype }`:
> DDL writes the name, schema-qualified and quoted where needed, and the
> subtype is the `ColumnType` of what the bounds are, which no name says and
> codegen reads. `ColumnTrait::save_as` on such a column is `as_range` to its
> type (`[spec:pgorm:sem:entity.traits.column.enum-cast]`), so an insert, an
> update and every value predicate write the text cast to the type, and
> `save_array_as` is `as_range` to the type's array, so a quantified
> comparison (`eq_any`, `ne_all`, `compare_any`, `compare_all`) writes the
> array of the ranges' texts cast to it. That is
> what turns the `to_string()` of a built-in `Range<i32>` written to a column
> of a range type over `int4` from a constructor call PostgreSQL refuses —
> `42804` in an insert, `42883` against the column — into a literal it reads.
> A pipeline compares a bound built-in `Range` or `Multirange` with such a
> column — an entity column, or one `compares_as` names the type of
> (`[spec:pgorm:req:pipeline.surface]`) — through its text as well, but has
> the server write it: the placeholder is minted before the comparison that
> casts it, and may be compared elsewhere in its stage as the range it is, so
> the value stays the built-in range bound in binary, pinned to its type and
> cast to `text` before the column's type, `CAST($1::int4range::text AS
> slot)`. The server's text of a built-in range reads back as the same
> range.
> A column of an array of the type — `Vec` of its newtype, a field the
> newtype's `NotU8` admits, or an array of a domain over the type — is
> `ColumnType::Array` of it, and its `save_as` is `as_range` to the type's
> array, `slot[]` (a domain's array for a domain's): an insert, an update and
> every value predicate write the array of the values' texts cast to it,
> `CAST($1::text[] AS slot[])`, the empty array and a `NULL` element read by
> the server as written, and a built-in range array compared with it is
> written as the array of its ranges' texts. A pipeline binds a built-in range
> or multirange array compared with it in binary, pinned and cast through
> `text`, `CAST($1::int4range[]::text AS slot[])`, which the server reads as
> an array of the texts. No quantified comparison exists with an array
> column: `ANY` and `ALL` read their array's elements, and PostgreSQL's
> arrays carry no dimension in their type, so the elements of `slot[]` are
> each a `slot` (`operator does not exist: slot[] = slot`).
> A row decodes from the range's binary form: tokio-postgres reports a created
> range as a range over its subtype, so `Range<T>` decodes for every
> `RangeSubtype` and refuses one over another subtype
> (`[spec:pgorm:def:exec.decode.range]`), and the newtype decodes through it.
> An array of one decodes element by element the same way, for every
> `RangeSubtype`, a `NULL` element refused as every array's is; an array of a
> created multirange, which the driver reports with no subtype, is read as
> `text[]`, its column's `select_as` casting it so, and each element parsed as
> a scalar one is.
> Where a column is read into `Value`s, as a binary COPY reads one, a range
> type a schema created over a `RangeSubtype` reads as the `String` of its
> text form, the value its newtype converts into, and so does the multirange
> beside it (`conn.copy.rows`).
>
> That format has one implementation, `pgorm::CreatedRangeCodec`, named by
> the value kind of the subtype and whether it is the multirange: a value's
> binary form read through the subtype's codec into its text form
> (`CreatedRangeText`, the `FromSql` of such a column), and the text form
> written from ranges whose bounds are `Value`s of the subtype's kind and
> read back into them. A `numeric` bound is read as `ExactDecimal` reads a
> `numeric`, held to what a `Decimal` keeps exactly and refused rather than
> rounded past it. pgorm's binary COPY and cursor bindings read and write a
> created range through it, and so do both language bindings, which keep no
> codec of their own (`bindings.created-ranges`).
>
> pgorm cannot author the C function a canonical function is
> (`[spec:pgorm:req:sql.ddl.type-range]`), so a range type a schema creates
> through it is continuous whatever its subtype: one over `int4` keeps `[1,5]`
> and `(1,5]` as written where `int4range` moves them, and `[5,5)` is still
> `empty`. The server refuses, with its own code on both renderings, inverted
> bounds (`22000`), a bound the subtype does not read and malformed text
> (`22P02`), and a bound outside the subtype (`22003`).
>
> The multirange PostgreSQL creates beside such a range travels the same
> way, and is read the same way too. It is named by its own name, which is
> not the range's: a newtype over `Multirange<T>` deriving
> `CreatedRange` with `multirange_name`, whose `Value` is the
> multirange's text form (`{[1,3),[5,8)}`) and whose column is
> `ColumnType::CreatedMultirange { name, schema, subtype }`. A value is
> written through `as_range` exactly as a range is — `CAST($1::text AS
> floatmultirange)` — and a built-in `Multirange<i32>` reaches a multirange
> created over `int4` the same way, there being no cast between multirange
> types either (`42846`). Its variadic constructor over an array of its
> ranges (`floatmultirange(VARIADIC $1)`) was the other route and is not
> taken: it needs the range's name as well as the multirange's, and an array
> of the range bound as `floatrange[]`, one more statement shape for a value
> the text cast already writes.
>
> It is read as its text, because tokio-postgres learns a type's kind from
> `pg_range` joined on the *range* type, so a created multirange arrives as a
> simple type with no subtype to read its binary form by. `ColumnTrait::select_as`
> casts a column of one to `text`, as it casts an enum column, and the
> newtype parses the text with `Multirange<T>`'s `FromStr`, refusing text
> that reads as no multirange of its subtype. The cast rides the read path an
> enum already has, so `find`, the graph and the pipeline's `select_sources`
> all read it, a `NULL` stays `NULL`, and the server's sorted, merged form is
> what reads back. A binary COPY casts nothing, so it reads such a column
> through the catalogue instead: its description completes the multirange's
> kind with the range's subtype, and its field reads as the `String` of the
> text form `Multirange<T>`'s `Display` writes (`conn.copy.rows`), which the
> server reads back as the same value, as for a range.
> `ARRAY(SELECT unnest(col))`, which the driver types as an
> array of the range and could read in binary, was the other route and is
> not taken: it reads `NULL` as `{}`, and it is not a cast, so the pipeline,
> which reads only a cast off `select_as`, would project the bare column. The
> text is the server's own output, so it reads back exactly under the
> server's defaults — `DateStyle` `ISO`, `extra_float_digits` `1` — and a
> session that changes them changes the text: another date style is
> refused, and a lowered `extra_float_digits` rounds a float bound. Nothing
> checks the subtype on the way in: a newtype naming another subtype than
> the column's refuses text it cannot read, and reads numerals of another
> numeric type as its own.

## Intervals

> [spec:pgorm:def:sql.value.interval+2]
> `Interval` is PostgreSQL's `interval` as a value: the public fields
> `months: i32`, `days: i32` and `microseconds: i64`, each with its own sign,
> built by `Interval::new(months, days, microseconds)` or `Default`, which is
> zero. The three fields MUST stay apart: a month has no fixed number of days
> and a day no fixed number of microseconds across a daylight-saving change,
> so `1 mon` is not `30 days`, and `1 mon -2 days` is a value no single-signed
> duration spells. Nothing in pgorm normalises one field into another or
> carries the time as float seconds. Every triple is a value the server
> stores; the two whose every field is at its maximum or at its minimum are
> PostgreSQL's `infinity` and `-infinity` (`is_infinity`, `is_neg_infinity`).
> Equality and hashing are field by field, so `1 mon` and `30 days` are two
> values here where PostgreSQL's `=` on `interval` calls them equal.
>
> `Value::Interval(Option<Box<Interval>>)` carries one, beside the variants
> `[spec:pgorm:def:sql.value]` lists, and `ArrayType::Interval` tags an
> `interval[]`, beside the tags of `[spec:pgorm:def:sql.value.array]`. The
> payload is boxed though it holds no pointer: inline, its sixteen bytes and
> the `Option` around them would widen every `Value` from 24 bytes to 32.
> `From<Interval>`, `Nullable` and `ValueType` give the variant, its typed
> `NULL` and its extraction — `type_name` `Interval`, `array_type`
> `ArrayType::Interval`, `column_type` `ColumnType::Interval(IntervalSpec::Any(None))`,
> the unqualified `interval` — and `Interval` is `NotU8`, so `Vec<Interval>` is
> an `interval[]`. `is_interval` and `as_ref_interval` are its accessor pair,
> and neither panics (`[spec:pgorm:sem:sql.value.accessor-panics]`).
> `source_type_name` is `interval`, so a cast operand's placeholder is pinned
> `$1::interval` and an array's `$1::interval[]`
> (`[spec:pgorm:req:sql.render.cast-param-type]`), and an empty array's
> literal is `ARRAY []::interval[]`.
>
> The binary form is `interval_send`'s: the microseconds, the days and then
> the months, each big-endian, sixteen bytes in all. `Interval::to_binary`
> writes it, and `Interval` implements `postgres_types::FromSql`, which reads
> it from `interval` or a domain over it and refuses any other length. The
> codec sits on the type, unlike a range's
> (`[spec:pgorm:def:exec.decode.range]`), because pgorm, pgorm-python and
> pgorm-napi all decode an interval through it, and `postgres-types` is
> already a dependency of pgorm-query, so no detached workspace locks a new
> crate. There is no `ToSql`: its signature names `bytes`' `BytesMut`, which
> pgorm-query does not depend on, and its one writer, `ValueHolder`, writes
> `to_binary` (`[spec:pgorm:req:exec.cursor.binding-interval]`).
>
> Under pgorm-query's `serde` feature, which pgorm's `with-json` turns on,
> `Interval` implements `Serialize` and `Deserialize` as a struct of its three
> fields by name: `{"months":1,"days":-2,"microseconds":3000004}` in JSON.
> Each field is the integer it holds, never a duration string or float
> seconds, so every triple serializes losslessly and the infinities as the
> extreme triples they are. Deserializing reads exactly those three fields: a
> missing one, an unknown one (`hours`) or a value outside its field's width
> is an error, never a zero, a dropped unit or a wrapped number. The keys are
> the ones the Python binding's snapshot writes (`[spec:pgorm:req:python.value-tags]`),
> each value the number it is rather than the decimal text a snapshot writes
> for every integer. A Model deriving serde over an `Interval` field —
> `ActiveModelTrait::from_json`'s (`[spec:pgorm:req:entity.active-model.json]`),
> or one codegen generates (`[spec:pgorm:def:codegen.entity.serde]`) — so
> compiles and round-trips.
>
> Both language bindings carry it as their `Interval` (`bindings.values`),
> JavaScript converting a `Temporal.Duration` to one (`napi.temporal`), and
> both bind it in built statements as this variant. The generative campaign carries it as the portable kind
> `interval`, whose payload is the three fields as exact decimal text, the
> Python binding's snapshot of one (`[spec:pgorm:req:generative.format]`):
> its programs bind, inline and decode intervals through both bindings and
> compare them field by field, and its replay encodes the same triple.

> [spec:pgorm:sem:sql.value.interval-text+1]
> `Display` for `Interval` writes the text PostgreSQL's interval input reads
> back as the same triple: `infinity` and `-infinity` for the two infinite
> triples, `+0 days` when every field is zero, and otherwise each nonzero
> field, in the order months, days, microseconds, as an explicitly signed
> integer and its unit — `+1 months -2 days +3000004 microseconds`. Every
> field MUST carry its sign: under `IntervalStyle` `sql_standard` the server
> applies a leading `-` to every field after it unless another field is
> signed too, so `-1 days 2 hours` would be read as `-1 days -2 hours`. The
> microseconds are an integer, so no field passes through a fraction the
> server would round, and the infinities are the words PostgreSQL's own
> output uses for them. On 18 the all-maximum and all-minimum triples spelled
> as fields also read back as the infinities, but the words do not depend on
> that.
>
> The literal rendering (`[spec:pgorm:sem:sql.value.render]`) writes that
> text single-quoted through `write_string_quoted` and cast,
> `'+1 months -2 days +3000004 microseconds'::interval`, and a `None` payload
> as `NULL`. Unlike the temporal literals it carries its cast: an untyped
> string is `text` wherever no column types it, and an array of them is a
> `text[]`, which an `interval[]` column refuses (`42804`). The literal and
> the bound value are one triple, under every `IntervalStyle`.

## Domains on the wire

> [spec:pgorm:def:sql.value.base-type]
> A domain's values travel in the representation of the type it is over, so
> whatever reads, writes or accepts a value against a type the server
> described looks through that type's domains first. `base_type(ty)`, a free
> function over `postgres_types::Type`, is that one step, PostgreSQL's own
> `getBaseType`: a domain, a domain over a domain included however deep, is
> the type at the bottom, and any other type is `ty` itself, borrowed rather
> than copied. It looks through `ty` and nothing inside it: an array, range or
> multirange of a domain is its own type with its own representation, so it
> is returned as it is, and a caller that reads one asks `base_type` of the
> element or subtype it reaches. pgorm's parameter binding
> (`exec.cursor.binding-accepts`), its array decode (`exec.decode.array`),
> its binary COPY rows (`conn.copy.rows`), its range types a schema created
> (`sql.value.created-range`), `Interval`'s acceptance check
> (`sql.value.interval`) and the Python and JavaScript bindings' codecs all
> ask it, reaching it as `pgorm::pgorm_query::base_type`, so none of them
> spells the step alone.

## Value tuples

> [spec:pgorm:def:sql.value.tuple+3]
> `ValueTuple` is a newtype over `Vec<Value>`: one ordered tuple of values for
> composite keys and VALUES lists, at every arity. It MUST NOT tag its arity in
> its representation. A tuple of a given width therefore has exactly one
> spelling, which is what makes the derived `PartialEq`, `Eq` and `Hash` lawful
> by construction: a key built from a Rust pair and one gathered from an
> iterator of the same two values ARE equal and hash alike, so a
> `HashMap<ValueTuple, _>` (`[spec:pgorm:sem:query.loader.regroup]`) cannot
> split one logical key across two buckets. Under an arity-tagged
> representation that agreement rested on every producer picking the same tag;
> here the divergent spelling cannot be constructed.
>
> `IntoValueTuple` is implemented for any single `Into<Value>` and for 2-
> through 12-tuples, each producing its values in field order. `IntoIterator
> for ValueTuple` yields them in that same positional order, `iter()` borrows
> them, and `arity()` is their count. `From<Vec<Value>>` and
> `FromIterator<Value>` build a tuple from values already gathered.
>
> `TryFromValueTuple` inverts the mapping, is arity-strict, and is fallible:
> it returns `Result<Self, ValueTupleError>` and never panics. Every impl makes
> the same single length check — 1 for the scalar impl, 2..=12 for the tuple
> impls — and any other length is `ValueTupleError::Arity { expected, actual }`,
> both fields plain lengths rather than named shapes. Element extraction goes
> through `ValueType::from_value`, so a type mismatch at any position is
> `ValueTupleError::Element { position, expected }`, naming the zero-based
> position and the `ValueType::type_name` required there. The conversion is
> short-circuiting: the leftmost failing position is the one reported.
> `ValueTupleError` implements `std::error::Error`; `Arity` displays as
> `expected a tuple of arity {expected}, received {actual}` and `Element` as
> `value at position {position} is not a valid {expected}`, the type name
> backquoted.
>
> `Values` is a `Vec<Value>` newtype with `iter()` and `IntoIterator`, used to
> carry a statement's collected parameters.

## Literal rendering

> [spec:pgorm:sem:sql.value.render+2]
> `QueryBuilder.value_to_string` renders a `Value` as an inline Postgres
> literal (also used by `Display for Value` and by `SqlWriter for String` when
> a statement is built without parameter binding). `None` payloads render as
> `NULL`; booleans as `TRUE`/`FALSE`; numerics and `Decimal` in plain decimal
> form. Strings and chars are single-quoted after `escape_string`, switching
> to the `E'...'` form when the escaped text contains a backslash. `Bytes`
> renders as `'\xHEX...'`. Temporal values render quoted with formats
> `%Y-%m-%d`, `%H:%M:%S%.f`, `%Y-%m-%d %H:%M:%S%.f` and, for the zoned
> variants, `%Y-%m-%d %H:%M:%S%.f %:z`.
>
> A literal MUST carry the sub-second digits its value holds. `%.f` writes the
> fraction and omits both it and the dot when it is zero, so a whole second
> renders exactly as it would without the directive — which is why adding it
> moved no rendered output anywhere in the tree.
>
> The value MUST first be truncated to the microsecond, because that is what
> PostgreSQL stores and what the parameter path does with the same value. Left
> alone, a ninth digit would be rounded by the server on the literal path and
> discarded by the encoder on the bound one, so the same value would mean two
> different things depending on how it travelled
> (`[spec:pgorm:def:exec.cursor.binding]`).
>
> `Uuid`, `IpNetwork` and `MacAddress` render as
> quoted display strings. `Array` renders as `ARRAY [elem,...]` recursively
> and `Vector` as a quoted bracket literal `'[v1,v2,...]'`.
>
> A range renders as a call of its range type's constructor,
> `int4range(1, 5, '[)')`: each bound is written by this same rendering of
> its own variant, an unbounded side as `NULL` — which is how the constructor
> reads one, so a `NULL` bound means no bound here as it does on the bound
> path — and the third argument is the two brackets, `[` or `(` for the lower
> side and `]` or `)` for the upper, an unbounded side always open. The empty
> range has no bounds to pass and is the one textual literal,
> `'empty'::int4range`, cast so that it names its type the way an empty
> array does. A multirange is its multirange type's constructor over its
> ranges, `int4multirange(int4range(1, 3, '[)'), 'empty'::int4range)`, and
> the empty multirange `int4multirange()`. A constructor call needs no
> quoting rules of its own: the range text form would have to quote and
> escape each bound's text inside its own brackets, a second escaper for
> values the literal rendering already writes.

## Identifier machinery

> [spec:pgorm:def:sql.types+9]
> `SqlName` is the name trait (bounded `Any + Send + Sync`): implementors provide
> `unquoted`, and the trait derives `to_string` (unquoted), `quoted()` —
> which doubles any embedded double quote — and `prepare`, which writes the
> identifier wrapped in double quotes. Neither takes a quote character:
> PostgreSQL has one identifier quote, so there is no `Quote` type and no
> parameter that could carry a different one (which is also how the panic on a
> non-ASCII quote byte stopped existing). It is spelled `SqlName` beside the
> crate's `SqlText`, so the two things a string can be — a name, or trusted
> SQL — are told apart by the trait a value satisfies.
> `StaticName` adds `as_str(&self) -> &str` for `Copy + Debug + 'static` names
> whose text the program fixes. It is ONE trait: the entity layer MUST NOT
> declare a second with the same shape, which is what made a glob of both
> modules ambiguous.
>
> `Name` is the erased, shared form: a non-generic wrapper over
> `std::sync::Arc<dyn SqlName>` (the inherited `SeaRc<T>` and the `RcOrArc`
> re-export are gone; shared ownership of anything else is spelled `Arc`
> directly). It is the only exported spelling — the `DynIden`/`SharedIden`
> pair that named one type twice is gone. `Name` equality compares the erased
> name's `TypeId` and the unquoted string, so two names are equal exactly when
> they are the same concrete type rendering the same text. Equality MUST NOT
> be decided by the trait object's vtable address: Rust guarantees a vtable
> neither unique per type nor stable across codegen units, so an address
> comparison can report two names of one type and one text unequal — and
> `Name` underpins `PartialEq` for `ColumnRef`, `TableName` and the rest of
> the AST, where that is a wrong answer about SQL. The `TypeId` is asked of
> the value rather than stored beside it — the `Any` bound on `SqlName` is
> there for this, and `&dyn SqlName` upcasts to `&dyn Any` to ask — so a
> `Name` stays one pointer pair wide, which it must: one sits in nearly every
> AST node, and a field would have doubled them all. The crate is
> `#![forbid(unsafe_code)]`, and the transmute the vtable comparison needed is
> what that forbids.
>
> `IntoName` converts any `SqlName + 'static` (or an existing `Name`) into a
> `Name`. It MUST NOT accept `&str` or `String`: text reaches identifier
> position through `Name::runtime(impl Into<String>)` and nowhere else, so
> every point at which a value becomes a name is one grep away rather than an
> inference away. The inner type `Name::runtime` wraps is private, so there is
> no second door. `IntoName` is the whole conversion surface: there is no
> separate one-or-many identifier trait, so a call site that wants several
> names takes an iterator of `IntoName`.
>
> There is no empty-name identifier type: PostgreSQL rejects a zero-length
> delimited identifier, so the `NullAlias` that rendered one — and only ever
> served as a placeholder inside `ColumnDef::take` — is gone
> (`[dec:pgorm:invalid-states-unrepresentable]`).
>
> `AliasName` is the token form of a name the query text spells
> literally, minted by the free function `alias(name)` over a `&'static str`.
> It is `Copy`, `SqlName` and `StaticName`, so `IntoName`, `IntoColumnRef` and
> every other name position accepts it with no conversion, and one
> `let rn = alias("rn")` binding serves as both the declaration and every
> reference — the name is spelled once instead of once per site.
>
> The motivating case is a name the query itself introduces, where the token
> is what makes a reference unable to miss its declaration; but the remit is
> the whole statically-known set, so a cast target, a `CREATE TYPE` name or a
> named column type is written the same way rather than in a second spelling
> that a reader would have to tell apart. The split that matters is against
> `Name::runtime`, which is for names computed at run time — a `format!`-built
> alias, a `String` arriving from a caller — and which is where the
> `&'static str` bound draws the line: a name known only at run time cannot
> be a token, and no amount of interning would make it one. The two are no
> longer near-homographs: the type that took runtime text was called `Alias`
> next to a free function called `alias` that took static text, and only the
> latter survives under that word.
>
> The token carries no evidence that its name was ever attached to anything:
> a token no projection declares still compiles, and the server reports the
> unknown column exactly as it would for a mistyped string.

> [spec:pgorm:def:sql.types.column-ref+1]
> `ColumnRef` has seven forms: `Column(Name)`, `TableColumn(Name,
> Name)`, `SchemaTableColumn(Name, Name, Name)`, `Asterisk`,
> `TableAsterisk(Name)`, and the two that read a version of a written row in
> a RETURNING list, `RowColumn(ReturningRow, Name)` and
> `RowAsterisk(ReturningRow)` (`sql.ast.returning`). `IntoColumnRef` maps a
> bare name to `Column`, a 2-tuple to `TableColumn`, a 3-tuple to
> `SchemaTableColumn`, the `Asterisk` unit type to `Asterisk`, `(name,
> Asterisk)` to `TableAsterisk`, `(ReturningRow, name)` to `RowColumn`, and
> `(ReturningRow, Asterisk)` to `RowAsterisk`.

> [spec:pgorm:def:sql.types.table-ref+7]
> Table references are split by position, so that a reference which names no
> table cannot reach a statement that needs one. There are three positions,
> and each takes the widest type its position admits: DDL targets a name,
> DML targets a name with an optional alias, and a `FROM` clause or join
> additionally admits the value-producing forms.
>
> `TableName` is the DDL-position reference and has exactly two forms:
> `Table(Name)` and `SchemaTable(Name, Name)`. There is no
> database-qualified form — Postgres rejects a cross-database reference at
> execution, so the shape is not offered. `IntoTableName` maps a 2-tuple to
> `SchemaTable` and accepts a `TableName` unchanged. It maps a bare name to
> `Table`, unless the name knows its table's schema: `SqlName::table_schema`,
> `None` unless an implementor answers it, names the schema that qualifies a
> name wherever it names a table, and a bare name answering `Some` maps to
> `SchemaTable` of that schema (a `Name::runtime`) and the name, erased to a
> `Name` first or not. An entity is such a name
> (`[spec:pgorm:req:entity.traits.entity-name]`). The schema qualifies the
> table and nothing else: a column of it is still qualified by the table
> name alone, which the qualified table exposes. `TableName::table()`
> returns the table name; `TableName::schema()` returns the schema name when
> the name carries one.
>
> `NamedTable` is the DML-position reference: a struct of a `name: TableName`
> and an `alias: Option<Name>`, which is exactly what PostgreSQL's write
> statements target. `IntoNamedTable` accepts a `NamedTable` unchanged and
> widens anything `IntoTableName` accepts — a bare name, a 2-tuple, a
> `TableName` — to the unaliased form, so a DML target spelled as a bare name
> needs no ceremony; `From<TableName> for NamedTable` is the same widening as
> a value conversion. `NamedTable::alias(a)` binds or replaces the alias, and
> `NamedTable::qualifier()` returns the identifier a column of the table is
> qualified by — the bound alias when there is one, otherwise the table name.
>
> `FromItem` is the query-position reference: `Table(NamedTable)` — the DML
> reference reused, so aliasing is expressed in one place — plus the six
> value-producing forms `SubQuery(SelectStatement, alias)`,
> `ValuesList(Vec<ValueTuple>, alias)`, `FunctionCall(FunctionCall, alias)`,
> `Template(SqlTemplate, alias)`, `JsonTable(Box<JsonTable>, alias)`
> (`sql.ast.json-table`) and `TableFunction(Box<TableFunction>, alias)`
> (`sql.ast.select.table-function`), each carrying a mandatory alias, and
> `TableSample(Box<TableSample>)` (`sql.ast.select.table-sample`), a named
> table read through `TABLESAMPLE`, which carries its alias, when it has one,
> on the table it samples.
> `IntoFromItem` accepts a `FromItem` unchanged and widens anything
> `IntoNamedTable` accepts; `From<NamedTable> for FromItem` and
> `From<TableName> for FromItem` are the same widening as value conversions.
> `FromItem::alias(a)` binds or replaces the alias on any form.
> `FromItem::table_name()` returns the name for the named form and the
> sampled table's for a sample, and `None` for the value-producing forms;
> `FromItem::qualifier()` returns the identifier a column of the item is
> qualified by, delegating to `NamedTable::qualifier()` for the named form and
> a sample.
>
> `Template` is how SQL this crate did not build stands where a relation
> stands, and it is template-class rather than raw: its payload is a
> `SqlTemplate` (`[spec:pgorm:req:sql.render.custom-expr]`), so the
> fragment arrives already paired with the values its `$N` markers number
> and a fragment whose markers and values disagree is unconstructible. There
> is no `&'static str` sibling in this position: relation-position fragments
> come from callers' runtime statements, which is exactly the case the
> template class exists for, and a verbatim form would have to re-open the
> render-time census the template class closed. It reaches a query through
> the ordinary `from`/`join` entry points, which take any `IntoFromItem`, so
> the crate gains no `Result`-returning builder method for it — the `Result`
> is `SqlTemplate`'s, one step earlier, where it can be handled without
> interrupting a builder chain.
>
> Because every DDL target takes `TableName`, every DML target takes
> `NamedTable` and every query position takes `FromItem`, a subquery, values
> list or function call in a DDL or DML position is a type error rather than a
> render-time panic or a statement PostgreSQL rejects; an alias in a DDL
> position is a type error for the same reason
> (`[spec:pgorm:sem:sql.ddl.panics]`).

> [spec:pgorm:def:sql.types.opers+6]
> `UnOper` has the single variant `Not`. `BinOper` enumerates the binary
> operator vocabulary: logical `And`/`Or`; pattern `Like`/`NotLike` plus
> Postgres `ILike`/`NotILike` and the regular-expression `SimilarTo`/
> `NotSimilarTo` (`sql.ast.expr.similar-to`); the IS family `Is`/`IsNot` and the null-safe
> `IsDistinctFrom`/`IsNotDistinctFrom`; `In`/`NotIn`;
> `Between`/`NotBetween` and their bound-sorting
> `BetweenSymmetric`/`NotBetweenSymmetric`; comparisons `Equal`, `NotEqual`,
> `SmallerThan`,
> `GreaterThan`, `SmallerThanOrEqual`, `GreaterThanOrEqual`; arithmetic
> `Add`/`Sub`/`Mul`/`Div`/`Mod`; shifts `LShift`/`RShift`; `As`;
> full-text/containment `Matches`, `Contains`, `Contained`; `Concatenate`,
> `Overlap`; the range position tests `NotExtendRightOf` (`&<`),
> `NotExtendLeftOf` (`&>`) and `AdjacentTo` (`-|-`); pg_trgm similarity
> operators (`Similarity`, `WordSimilarity`,
> `StrictWordSimilarity` and their `*Distance` forms); JSON access
> `GetJsonField` (`->`), `CastJsonField` (`->>`), `GetJsonPath` (`#>`) and
> `CastJsonPath` (`#>>`); JSON key existence `HasJsonKey` (`?`),
> `HasAnyJsonKeys` (`?|`) and `HasAllJsonKeys` (`?&`), and JSON path
> existence `JsonPathExists` (`@?`); regex `Regex` (`~`) and
> `RegexCaseInsensitive` (`~*`) and their complements `NotRegex` (`!~`) and
> `NotRegexCaseInsensitive` (`!~*`); temporal `AtTimeZone`; pgvector distances
> `EuclideanDistance`,
> `NegativeInnerProduct`, `CosineDistance`; and an escape hatch
> `Raw(&'static str)`. There is no `Escape` operator: `ESCAPE` is
> grammatical only as the tail of a `LIKE` or `SIMILAR TO` pattern, so it
> belongs to
> `SimpleExpr::LikePattern` and cannot be applied to two arbitrary operands
> (`[dec:pgorm:invalid-states-unrepresentable]`).
>
> JSON containment and merge get no variants of their own: `Contains`,
> `Contained` and `Concatenate` already render `@>`, `<@` and `||`, which are
> the operators PostgreSQL defines for `jsonb` as well as for arrays and
> ranges (`sql.ast.expr.operators`). Nor do the range tests strictly left of
> and strictly right of: they are the operators `<<` and `>>`, which
> `LShift` and `RShift` render and which bind as the shifts do, and the
> jsonpath match `@@` is `Matches`, the full-text operator's lexeme.
>
> `BetweenSymmetric` and `NotBetweenSymmetric` are variants rather than
> `Raw("BETWEEN SYMMETRIC")` because the renderer classifies the *variant*,
> not the lexeme: `Oper::is_between` is what unwraps the `AND` binary a
> BETWEEN's two bounds are encoded as (`sql.render.parens`), so a symmetric
> form written through `Raw` renders `x BETWEEN SYMMETRIC (a AND b)` — a
> statement the grammar accepts and reads as a boolean comparison. The same
> argument applies to `IsDistinctFrom`/`IsNotDistinctFrom` and `Oper::is_is`,
> though there the cost of missing the family is only parentheses. It is the
> general rule for this enum: an operator whose parenthesisation differs from
> the default MUST have a variant, and `Raw` is for operators the renderer has
> no reason to look at. Every operator that answers a boolean is such an
> operator, since the renderer drops parentheses around a boolean operand
> (`sql.render.precedence`) and cannot know a `Raw` operator's result: that is
> why `!~`, `!~*`, `@?`, `&<`, `&>` and `-|-` are variants, where through
> `Raw` they kept parentheses `~`, `?` and `&&` drop.

## Column type vocabulary

> [spec:pgorm:def:sql.types.type-name+8]
> `TypeName` (`pgorm-query/src/types.rs`) is the structured spelling of a
> type in cast or column-type position: `schema: Option<Name>`,
> `name: Name`, `array: bool`, and a private `verbatim`. Rendering
> (`to_sql_string`) joins the parts with `.` and appends a structural `[]`
> for arrays. Each part is written bare or quoted by one policy
> (`pgorm-query/src/keywords.rs`), which knows where the part stands.
>
> A part renders bare when it is a lowercase identifier
> (`^[a-z_][a-z0-9_]*$`) and not a keyword PostgreSQL restricts. Unquoted
> names fold to lowercase, so bare and quoted are the same name there.
> Every other part renders as a quoted identifier, case preserved. The
> restricted keywords are PostgreSQL's `COL_NAME`, `TYPE_FUNC_NAME` and
> `RESERVED` categories — the set PostgreSQL's own `quote_ident()` quotes —
> embedded from the `kwlist.h` of the release the linked libpg_query
> carries, 18.6. A test pins the embedded list against that scanner in both
> directions, and fails when the linked parser moves; the restricted set is
> unchanged from PostgreSQL 17 and in 19, and PostgreSQL 16 differs only in
> leaving `json` unrestricted. A restricted keyword written bare is not a
> name. In a type position `select` is a syntax error, and in a function
> position the shape of the statement changes: `not(1)` is a boolean NOT,
> `row(1)` a row constructor, `distinct(1)` first in a projection a
> `SELECT DISTINCT`, and `only` before a `FROM` function an inheritance
> marker. Quoted, each is an identifier naming what the bare word would
> have named had it not been a keyword.
>
> The policy writes bare, where `quote_ident()` would quote, exactly two
> kinds of keyword. Each is a keyword the caller means as the grammar's own
> form rather than as a name, and each is listed per keyword:
>
> - **Type spellings**, in an unqualified type position — a cast target, a
>   column type or an enum type, array or not. `bigint`, `bit`, `boolean`,
>   `char`, `character`, `dec`, `decimal`, `float`, `int`, `integer`,
>   `interval`, `json`, `nchar`, `numeric`, `real`, `smallint`, `time`,
>   `timestamp` and `varchar` render bare, and each resolves to its
>   catalogue type (`integer` → `int4`). Quoted, most would name no type at
>   all, and `"char"` names a different one, the single-byte catalogue
>   type, which this spelling cannot reach. The words `national` and
>   `precision` are halves of two-word spellings and are quoted. A
>   two-word spelling (`double precision`, `character varying`,
>   `timestamp with time zone`) is a type expression rather than a name:
>   write it with `TypeName::raw`. No part of a schema-qualified type is a
>   spelling — `integer.ty` is a syntax error, and `s.integer` names a type
>   called `integer` in `s` — so such a part is quoted like any other
>   keyword.
> - **Call forms**, in a function position — `Func::named`, in an
>   expression or a `FROM` list. `coalesce`, `greatest`, `least` and
>   `nullif` render bare, as the expressions the parser builds for them.
>   They are not functions in `pg_proc`, so quoting them would name a
>   function that does not exist.
>
> Every other restricted keyword is quoted at every position, including a
> type's schema, the name after it, and an index access method
> (`IndexType::Named`). Where the grammar would have read a
> `TYPE_FUNC_NAME` keyword bare as the same name anyway (`left` as a
> function), the quoted spelling names the same object. One `UNRESERVED`
> keyword is quoted too, at a function position only: `operator(` in an
> expression opens the grammar's qualified-operator syntax
> (`OPERATOR(schema.op)`), so a bare `operator(1, 2)` is a syntax error. The
> identifier oracle holds every keyword the linked scanner knows at every
> position under this policy (`[spec:pgorm:req:security.ident-oracle]`),
> which is how that one was found.
>
> A name is therefore a name: text that is not an identifier, and an
> identifier that is a keyword, becomes a quoted identifier PostgreSQL
> refuses or resolves as a name, never SQL it executes. `raw_text` gives
> the unquoted dotted spelling, which is a description and not SQL:
> codegen respells a named type from it, and no writer puts it into a
> statement. The
> pipeline's `select_sources` read cast renders through `to_sql_string`
> like every other cast (`[spec:pgorm:sem:pipeline.select-sources]`).
> `Function::Named` names render under the same policy at their own
> position, and so does `IndexType::Named`'s access method. Both reach
> `prepare_part` directly because each is a single name rather than a
> `TypeName`: one policy, not a per-site escape.
>
> Type EXPRESSIONS — `BIT(8)`, `numeric(12, 2)` — are not names, and they
> are the one exception: `TypeName::raw` stores its argument in
> `verbatim`, and `to_sql_string` writes that text in place of the name,
> unquoted and unescaped. `verbatim` is private and holds the
> `&'static str` itself, and `raw` is the only way to set it: a quoted
> `TypeName` cannot be turned verbatim by assignment, and reassigning
> `name` on a raw type does not change what renders. `is_verbatim()` is how
> a reader asks. Only that text is verbatim: a schema qualifying a raw type
> is a name and renders under the part policy, and `[]` stays structural.
> The argument is bound `&'static str`, so only a literal written in the
> calling source can reach it and no runtime `String` — nothing a value
> could have reached — can become SQL there; the bound is the enforcement,
> not the doc. It is reached from `Expr::cast_as_raw` and the derive's
> `select_as` / `save_as` attributes, which carry the same bound, so the set
> of expressions that can emit an unescaped type name is one a reader can
> enumerate. A type that arrives as a *name*, including at
> runtime, takes `TypeName::new` or `ColumnType::named` and is quoted.
> `Func::cast_as` is deleted, `cast_as` quoting by default instead.
>
> Carrying the answer here rather than in a second node shape is what lets a
> cast have exactly one shape (`[spec:pgorm:req:sql.ast.cast-shape]`).

> [spec:pgorm:def:sql.types.column-type+11]
> `ColumnType` (in `pgorm-query/src/table/column.rs`, `#[non_exhaustive]`) is
> the type vocabulary shared by DDL generation, `ValueType::column_type()` and
> codegen, and every variant MUST name a type Postgres has: `Char(Option<u32>)`,
> `String(StringLen)`, `Text`, `Bytea`, `SmallInteger`, `Integer`,
> `BigInteger`, `Float`, `Double`, `Decimal(Option<(u32, u32)>)`, `Timestamp`,
> `TimestampWithTimeZone`, `Time`, `Date`, `Interval(IntervalSpec)`,
> `Bit(Option<u32>)`, `VarBit(u32)`, `Boolean`, `Money`,
> `Json`, `JsonBinary`, `Uuid`, `Named(TypeName)` (a type this vocabulary
> has no variant for, named rather than spelled: the payload is a `TypeName`
> so it renders under the one part policy — `ColumnType::named(text)` and
> `ColumnDef::named(name)` both build one, and the name may be
> schema-qualified or an array),
> `Enum { name, schema, variants }` (`schema: Option<Name>` — a qualified
> enum type carries its schema in the type itself, so every rendering that
> names the type can qualify),
> `Array(Arc<ColumnType>)`, `Vector(Option<u32>)`, `Cidr`, `Inet`, `MacAddr`,
> `LTree`, `Range(RangeType)` and `Multirange(RangeType)`, PostgreSQL's
> built-in range and multirange types (`[spec:pgorm:def:sql.value.range]`),
> `CreatedRange { name, schema, subtype }`, a range type a schema created,
> named in full as an enum type is and carrying the `ColumnType` of its
> subtype, `CreatedMultirange { name, schema, subtype }`, the multirange
> created beside one (`[spec:pgorm:def:sql.value.created-range]`), and
> `Domain { name, schema, base }`, a domain (`sql.ddl.domain`), named in full
> and carrying the `ColumnType` of its base. `Domain` is the one spelling of a
> column whose type is a domain: `Named` renders the same DDL but says nothing
> of what the column holds, where the base is what a value is read and written
> as and what codegen generates the field from.
> `ColumnType::serial_spelling` reports the serial form of the
> integer trio and `None` for everything else
> (`[spec:pgorm:req:sql.ddl.column-def]`).
>
> The vocabulary carries no MySQL-era spelling and no variant that renders
> something other than what it names, and MUST NOT reacquire one. `Year` had
> no Postgres spelling at all. `TinyInteger`, like the already-removed
> `TinyUnsigned`/`SmallUnsigned`, was a second name for `smallint`;
> `Unsigned`/`BigUnsigned` rendered plain `integer`/`bigint`, claiming a
> signedness Postgres does not have; `Blob`, `Binary(u32)` and
> `VarBinary(StringLen)` were three names for `bytea` with their lengths
> silently discarded, collapsed into the single `Bytea`; `DateTime` and
> `Timestamp` were two names for the same `timestamp`, collapsed onto the
> Postgres spelling; and `Money`'s precision/scale pair produced a
> `money(p, s)` type modifier Postgres rejects when it resolves it. The
> `ColumnDef` builders went with them — there is no `tiny_integer`,
> `unsigned`, `big_unsigned`, `year`, `blob`, `binary`, `binary_len`,
> `var_binary`, `date_time` or `money_len`, and no `ColumnType::var_binary`
> constructor; `bytea()`, `timestamp()` and `money()` are the surviving
> spellings.
>
> `StringLen` parameterises varchar length: `N(u32)`, `Max`, or the
> default `None`. `IntervalSpec` is the interval tail: `Any(Option<
> IntervalPrecision>)` for the unqualified `interval`/`interval(p)`, and
> `Fields(PgInterval)` for the qualified forms. `PgInterval` enumerates the
> thirteen interval field qualifiers (`Year` through `MinuteToSecond`); the
> four second-bearing ones (`Second`, `DayToSecond`, `HourToSecond`,
> `MinuteToSecond`) carry an `Option<IntervalPrecision>` because PostgreSQL
> takes a precision only where the trailing field is `SECOND`, so
> `interval HOUR(3)` does not construct
> (`[dec:pgorm:invalid-states-unrepresentable]`). `IntervalPrecision` is the
> closed set PostgreSQL accepts, `P0` through `P6`, with `new(digits)`
> returning `None` outside it and `digits()`/`Display` spelling it back.
> `PgInterval` implements `Display` as the SQL keywords with the precision
> appended (`YEAR TO MONTH`, `SECOND(3)`, ...) and a case-insensitive
> `TryFrom<&str>` inverse over the bare keywords.
>
> `ColumnType` equality compares parameters for the parameterised variants,
> compares `Named` and `Enum` by rendered identifier strings (and variant
> lists), compares `Array` element types recursively, compares `Range` and
> `Multirange` by the range type they name, compares `CreatedRange` and
> `CreatedMultirange` by rendered name and schema and subtype, compares
> `Domain` by rendered name and schema and base, and otherwise compares enum
> discriminants. Convenience constructors: `ColumnType::named(str)`,
> `ColumnType::string(Option<u32>)` and `ColumnType::var_binary(u32)`.
