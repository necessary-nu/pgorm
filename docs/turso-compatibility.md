# Experimental native Turso compatibility

The `turso` feature uses Turso's embedded API at
`adf3b41f99ebb6b1cdd1b60ae08b483bc45686fa` (`0.9.0-pre.6`). PostgreSQL remains
the reference implementation. This matrix distinguishes the driver's codecs
from type identities that the native compiler can preserve.

## Scalar codecs

| PostgreSQL parameter type | Native value | Result representation |
| --- | --- | --- |
| BOOL | Integer 0 or 1 | BOOL when column or compiled predicate metadata establishes it |
| INT2 | Signed integer | INT2 for declared `smallint` columns |
| INT4 | Signed integer | INT4 for native INTEGER columns and casts |
| INT8 | Signed integer | INT8 for declared `bigint` columns and counts |
| TEXT, VARCHAR, BPCHAR | UTF-8 text | TEXT, or VARCHAR for declared `varchar(n)` columns |
| BYTEA | Blob | BYTEA for declared `bytea`, native BLOB, and compiled BLOB casts |
| Finite FLOAT4, FLOAT8 | 64-bit floating value | FLOAT8 for native REAL columns and casts |
| A null with one of the types above | Null | Null with the prepared column's type |

An unmodified parameter projection can retain its explicitly supplied type.
Other results are described from native column and compiled expression metadata
before execution. Integer literals use the width established by their compiled
literal: INT4 when it fits i32, otherwise INT8 within i64. `COUNT(*)` and
`COUNT(column)` use INT8. Comparisons, logical predicates
and `EXISTS` use BOOL even where native affinity alone reports INTEGER.

The driver uses the shared binary `ToSql` and `FromSql` representations. Custom
codecs work when they accept a supported type and supply its binary format.
Text-format codecs, malformed payloads, out-of-range integers, nonfinite floats,
and conversions that would lose precision are refused. Nulls do not bypass type
validation, and empty text or bytes remain distinct from null.

NUMERIC, temporal values, UUID, JSON/JSONB, arrays, intervals, ranges,
multiranges, domains and other user-defined type identities have no native
driver codec in this subset. An unresolved expression is refused with its
column name and position; arbitrary arithmetic and aggregates other than count
are not inferred from returned cells.

## Metadata differences

The native compiler can erase PostgreSQL identities before exposing its
compiled representation. REAL identifies the engine's 64-bit value, including
columns originally declared with a floating type of another width. SMALLINT
casts and serial aliases in table declarations become INTEGER; bare
VARCHAR/BPCHAR declarations and
VARCHAR/BPCHAR, UUID, JSON and JSONB casts become TEXT. Unparameterized NUMERIC
declarations and NUMERIC casts become REAL/FLOAT8. A declared `numeric(p,s)`
retains its unsupported custom identity and is refused by the driver.

These canonical types do not establish fidelity to the original PostgreSQL
type. A TEXT result from an erased UUID cast is not UUID codec support, and a
Rust UUID reader must reject it. Likewise a NUMERIC expression lowered by the
engine to REAL does not preserve decimal precision. Explicit NUMERIC, UUID and
JSON parameter types remain unsupported regardless of nullness.

Table OIDs, column attribute numbers and PostgreSQL type modifiers are not
invented: native columns report `None`, `None` and `-1` respectively. Native
SQLSTATE and diagnostic fields are preserved where the engine supplies them.
Driver conversion and unsupported-operation errors carry no fabricated
SQLSTATE.

Rows own their binary payload and retain their prepared metadata. They remain
readable after the next native step, statement release or connection shutdown.
Native results do not fabricate pgwire messages or select a backend on each
cell read.

Prepared metadata is tied to the native compiler context. A schema or context
change invalidates the handle; `Error::is_statement_invalidated()` identifies
this refusal without a fabricated SQLSTATE. Discard that cached statement and
prepare it again for a later call. Invalidation can be detected after an engine
step, so this error does not prove that the operation had no effects and does
not authorize automatic retry.

At this engine revision, adopting another connection's schema can repeatedly
advance the native compilation context because of an internal schema-copy
identity mismatch. Before a fresh prepare in autocommit mode, the driver
synchronizes differing local/shared schema handles with a native `SELECT 1`.
This uses the ordinary worker cancellation and cleanup path, runs only for
that schema mismatch, and does not add a query or reparse on cache hits.

## Parameters and execution

Native parameters require explicit types through `prepare_typed` or the typed
driver query methods. Raw SQL with arbitrary `dyn ToSql` parameters cannot use
PostgreSQL's context inference. ORM-built queries retain supported `Value`
variants until native preparation, including typed nulls, and use the existing
connection-local cache keyed by SQL and ordered parameter types. PostgreSQL
does not resolve these hints or allocate their type vector.

ORM integer hints are INT2 for TinyInt/SmallInt, INT4 for Int, and INT8 for
BigInt/Unsigned/BigUnsigned. Values outside the selected representation still
fail the ordinary checked codec; a u64 above i64::MAX is refused. String/Char
use TEXT, Bytes uses BYTEA, and Float/Double retain FLOAT4/FLOAT8 parameter
types. Unsupported variants fail with the parameter position before execution.

Prepared calls accept one command. `batch_execute` runs a trusted script and
stops at failure; outside an explicit transaction, preceding completed
statements may remain committed. It does not provide PostgreSQL's implicit
script transaction. At the pinned revision, an empty script succeeds but a
comment-only nonempty script produces the native syntax error, SQLSTATE 42601.
`execute` reports completed command counts, including the returned row count
for SELECT.

The pool cache discards a known-invalid native handle before using it again.
An invalidation delivered while polling a stream marks the shared statement,
so its next cache lookup prepares afresh. The failed operation is not replayed
automatically. Native SQLSTATE 0A000 is a server refusal, not evidence that
PostgreSQL's cached-plan retry is safe. A conversion or cardinality error after
stepping likewise does not prove that a write had no effects.
