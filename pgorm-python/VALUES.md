# Native values

`pgorm.Value` owns a `pgorm::pgorm_query::Value` in the extension. Values are
immutable and reusable. `value.value` returns an independent Python value;
mutating a returned list or dictionary does not change the native value.

```python
from decimal import Decimal
from pgorm import TypeName, Value

small = Value(42, "i16")
money = Value(Decimal("19.9900"))
missing_text = Value.null("text")
json_null = Value.json(None)           # a JSON value, not SQL NULL
missing_json = Value.null("json")     # SQL NULL with the JSON tag
empty = Value.array("i32", [])
missing_array = Value.array("i32", None)
mood = TypeName("mood", schema="application")
label = Value("calm", mood)
labels = Value.array(mood, [label, None])
```

`Value(data, kind)` explicitly chooses a Rust variant. `Value(data)` infers
`bool`, `i64`, `f64`, `text`, `bytes`, `decimal`, `uuid`, `date`, `time`,
naive `datetime` or `interval` from exact Python types, `json` from a `dict`,
and an array from a `list`: an array of the one kind its items other than
`None` infer as, each `None` an SQL NULL element. Inference rejects `None`, an
empty or all-`None` list, a list of several kinds or of lists, every `tuple`,
which reads as a row, aware datetimes and custom objects; `Value.array(kind,
items)` declares what it cannot. It never chooses an integer type from a
boolean, calls an arbitrary object's string conversion, or promotes an
overflowing integer to another width. Passing an existing `Value` clones it;
supplying a different kind with that value raises `ConstructionError`. A kind
that is no kind name, `TypeName`, `CreatedRange` or `CreatedMultirange` raises
`TypeError`, and a name pgorm has no kind for `ConstructionError`.

| Kind | Rust variant | Python input/output and limits |
| --- | --- | --- |
| `bool` | `Bool` | `bool` |
| `i8`, `i16`, `i32`, `i64` | `TinyInt`, `SmallInt`, `Int`, `BigInt` | `int` within the chosen signed range |
| `u32`, `u64` | `Unsigned`, `BigUnsigned` | `int` in 0 through 2^width − 1 |
| `f32`, `f64` | `Float`, `Double` | `float`; signed zero, infinities and representable NaN payloads retained |
| `text`, `char` | `String`, `Char` | `str`; `char` is exactly one Unicode scalar |
| `bytes` | `Bytes` | `bytes`, including zero and non-UTF-8 bytes |
| `decimal` | `Decimal` | `decimal.Decimal`, exact 96-bit coefficient and scale 0–28 |
| `uuid` | `Uuid` | `uuid.UUID` |
| `json` | `Json` | JSON-compatible values; see below |
| `date` | `Date` | `datetime.date`, years 1–9999 |
| `time` | `Time` | `datetime.time` with no timezone and `fold=0` |
| `datetime` | `DateTime` | naive `datetime.datetime` with `fold=0` |
| `datetime_utc` | `DateTimeWithTimeZone` | aware `datetime.datetime` with zero UTC offset |
| `ipnetwork` | `IpNetwork` | IP/prefix string; native formatting on output, host bits retained |
| `mac_address` | `MacAddress` | exactly six bytes |
| `vector` | `Vector` | list/tuple of exact `f32` values; returns a list |
| `interval` | `Interval` | `pgorm.Interval(months, days, microseconds)`; see [Intervals](#intervals) |
| `array` | `Array` | `Value.array(element_kind, list_or_tuple_or_none)` |
| `enum` | `String` plus `TypeName` | `Value(label_or_none, type_name)` |
| `int4range` … `tstzmultirange` | `Range`, `Multirange` | `pgorm.Range`, `pgorm.Multirange`; see [Ranges](#ranges) |
| `CreatedRange(..)`, `CreatedMultirange(..)` | `String` (the text form) | `pgorm.Range`, `pgorm.Multirange`; see [Range types a schema created](#range-types-a-schema-created) |

Every scalar supports `Value.null(kind)` or `Value(None, kind)`. These are
typed SQL NULL values. `Value.json(None)` creates JSON null; it has
`is_null == False`. Array type identity survives empty and NULL arrays.
Array items may be compatible native values, Python values or SQL NULLs.
Use `Value.json(None)` as an array item to distinguish it from SQL NULL.
`array.items()` returns tagged values; `array.value` returns plain values.
Nested and heterogeneous arrays are rejected.

Enum labels retain their schema and type name separately from their Rust
string payload. `type_name` and `element_type` expose that identity. Type-name
parts are 1–63 UTF-8 bytes without NUL; case, punctuation and Unicode remain
unchanged. Lowering uses Rust's identifier-only `TypeName` API. Neither type
declaration nor value construction performs DDL.

`f32` conversion rejects a Python float if narrowing and widening changes its
bits. For example, `1.5` is exact but `0.1` is rejected. Applications may
explicitly round with `struct` before constructing a value when that is their
intention. Rust-to-Python conversion also rejects a NaN payload that cannot
survive widening; `snapshot()` remains available to inspect those native bits.
Vector elements follow the same policy.

Decimal conversion never passes through float or the active Decimal context.
Non-finite values, more than 29 coefficient digits and exponents outside
−28 through 28 are rejected; the remaining coefficient must fit Rust's
96-bit limit exactly. Positive exponents expand to scale zero. Representable
negative zero and trailing fractional zeros retain their sign and scale.

JSON accepts `None`, exact `bool`, `int`, finite `float`, `str`, lists, tuples
and dictionaries with string keys. Integers must fit Rust's signed 64-bit or
unsigned 64-bit JSON number representation. Tuples become JSON arrays. Cyclic
objects and nesting deeper than 64 levels are rejected. Decimal, UUID and
date/time values require application-selected JSON encodings. No implicit
string conversion or non-finite JSON number is used.

Temporal values preserve microseconds. Finer Rust precision and subsecond
timezone offsets raise errors; the Rust temporal types cannot represent a leap
second at all. A timezone-aware input must select a temporal kind, and
`datetime_utc` is the only aware kind. It requires a zero offset, because the
Rust value is an instant with nowhere to keep a timezone: the offset is
validated and discarded, and output is an aware `datetime` with
`datetime.timezone.utc` and `fold=0`. An input at another offset is converted in
Python first, with `value.astimezone(datetime.timezone.utc)`; a `zoneinfo`
daylight-saving fold resolves during that conversion, so the two instants an
ambiguous local time denotes stay distinct.

These are Rust value representation limits. PostgreSQL may impose additional
limits when binding a value to a particular database type; the execution API
reports a database/conversion error. A native `u64`, for example, is not a
claim that PostgreSQL has an unsigned 64-bit integer column type.

## Ranges

`pgorm.Range` and `pgorm.Multirange` are immutable Python values for
PostgreSQL's six built-in range types and their multiranges. A range is
`Range(lower, upper, bounds="[)")` or `Range.empty()`; `None` on a side is
no bound, and a side with no bound includes nothing, so its bracket is always
`(` or `)`. `Range()` is every value and differs from `Range.empty()`. A
multirange is `Multirange(iterable_of_ranges)`, a sequence.

```python
from decimal import Decimal
from pgorm import Multirange, Range, Value

span = Value(Range(1, 5), "int4range")
open_ended = Value(Range(Decimal("0.50"), None, "(]"), "numrange")
nothing = Value(Range.empty(), "daterange")
sets = Value(Multirange([Range(1, 3), Range(5, 8)]), "int8multirange")
missing = Value.null("tstzrange")
spans = Value.array("int4range", [Range(1, 3), None])
```

| Kind | Bounds |
| --- | --- |
| `int4range`, `int4multirange` | `i32` |
| `int8range`, `int8multirange` | `i64` |
| `numrange`, `nummultirange` | `decimal` |
| `daterange`, `datemultirange` | `date` |
| `tsrange`, `tsmultirange` | `datetime` |
| `tstzrange`, `tstzmultirange` | `datetime_utc` |

The kind is always explicit: a range's element type is not inferred. Each bound
converts exactly as a scalar of its element kind does, with the same limits.
Equality is structural, as in Rust; the server canonicalises a discrete range,
so `Range(1, 5, "[]")` written to an `int4range` reads back as `Range(1, 6)`,
and stores a multirange sorted and merged. Result columns of these types
decode to the same values. A snapshot's `data` for a range is `{"empty": true}`
or `{"lower": ..., "upper": ..., "bounds": "[)"}`, each bound in its element
kind's own encoding and `null` for no bound; a multirange's is a list of them.

### Range types a schema created

A range type made with `CREATE TYPE ... AS RANGE` has a name only its schema
knows. Its kind names it, with the value kind of its subtype:
`CreatedRange(name, subtype, schema=None)`, and `CreatedMultirange(name,
subtype, schema=None)` for the multirange PostgreSQL creates beside it, named
by its own name (`floatmultirange`, or `slot_multirange` beside `slot`).

```python
from pgorm import CreatedMultirange, CreatedRange, Multirange, Range, Value, bind

floatrange = CreatedRange("floatrange", "f64", schema="measure")
span = Value(Range(1.5, 2.5), floatrange)
assert span.snapshot()["data"] == "[1.5,2.5)"
assert bind(span).inspect().sql == "SELECT CAST($1::text AS measure.floatrange)"
spans = Value(Multirange([Range(1.0, 3.0)]), CreatedMultirange("floatmultirange", "f64"))
```

The subtype is one of the twelve a created range can range over, Rust's
`RangeSubtype`: `i16`, `i32`, `i64`, `f32`, `f64`, `decimal`, `text`, `date`,
`time`, `datetime`, `datetime_utc` and `uuid`. Each bound converts as a scalar
of that kind does, with its limits. The value holds the range's text form, the
`Value::String` a Rust `CreatedRange` newtype converts into, written by
Rust's `Display` with each bound quoted where the range parser needs it.
`bind` writes it as `CAST($1::text AS name)` and `literal` as an escaped string
cast to the name, Rust's `Expr::as_range`, because there is no cast between two
range types. The name is a quoted, possibly schema-qualified type name. A `str`
in the type's text form is accepted as well and read with the subtype's own
parsing, the way PostgreSQL's range input reads it, so whitespace inside the
brackets is part of a bound. `value` reads the text back as a `Range` or
`Multirange`, `created_type` returns the kind, `type_name` the name, and the
snapshot's `type` is `{"kind": "created_range", "name": ..., "schema": ...,
"subtype": ...}` with the text as its `data`.

A result column of a created range type decodes, from its binary form through
the subtype's own codec, to that kind, named as the column's type is, with the
schema the server reports. This holds for one over `int4` too: it reads as
`CreatedRange("slot", "i32", schema=...)`, not as `int4range`, so the value
writes back to its column (an `int4range` literal there is `42804`). A created
range is continuous whatever its subtype, so `[1,5]` reads back as written. The
driver reports a created multirange as a simple type with no subtype, so a
column of one raises `DecodeError`; select it cast to `text` and pass the text
to `Value(text, CreatedMultirange(...))`. `Value.array(CreatedRange(...), items)`
is an array of the kind, each item converted as a value of it is and held as its
text, and is cast to the type's array where it is written; a result column of
an array of a created range decodes element by element to it, and a query's
array of a created multirange raises `DecodeError` as a column of one does. The pipeline and SQL/JSON's `JsonDefault` refuse a created range's
value, as they refuse a qualified enum's, having no place for its cast.

## Intervals

`pgorm.Interval(months=0, days=0, microseconds=0)` is PostgreSQL's
`interval`: three fields, each an exact `int` with its own sign, `months` and
`days` within 32 bits and `microseconds` within 64. Nothing normalises one
field into another, so `Interval(months=1)` and `Interval(days=30)` are
different values, though PostgreSQL's `=` calls them equal, and `1 mon -2 days`
is a value of its own. `datetime.timedelta` is not accepted: it has no months,
and it keeps only its days signed, so `-1 hour` would arrive as
`-1 day +23 hours`. A `pgorm.Interval` infers as `interval`, binds in binary
and decodes back field for field, the two triples with every field at its
limit being PostgreSQL's `infinity` and `-infinity`. `str()` gives the text
PostgreSQL reads back as the same value; a snapshot carries the three fields
as integer strings.

```python
from pgorm import Interval, TypeName, Value, bind, select

span = Interval(months=1, days=-2, microseconds=3_000_004)
query = select(bind(span).cast(TypeName("text")).as_("text"))  # '1 mon -2 days +00:00:03.000004'
spans = Value.array("interval", [span, None])
```

## Inspection and equality

`kind`, `is_null`, `type_name` and `element_type` expose the native tags.
Equality compares Rust values and enum identities, including the Rust
bitwise float semantics: equal NaN payloads compare equal and positive and
negative zero compare unequal. Native values are not hashable.

`snapshot()` produces a detached JSON-compatible dictionary with `version: 1`,
`type`, `sql_null` and `data`. Arrays recursively retain each element's tag.
Integers and decimals use strings; floats and vector elements use hexadecimal
IEEE bit patterns, preserving infinities, signed zero and NaN payloads without
nonstandard JSON numbers. Bytes use byte lists, UUIDs/IP networks use native
text, and temporal text retains native precision and offsets. This is an
inspection format; it is not SQL or a public deserialization API.

Input errors raise `ConstructionError`; unrepresentable native outputs raise
`DecodeError`. Neither path silently substitutes `None` or truncates values.

Run the installed-wheel conversion suite without PostgreSQL:

```sh
target/python-check/bin/python -m unittest discover \
  -s pgorm-python/tests -p 'test_values.py' -v
```
