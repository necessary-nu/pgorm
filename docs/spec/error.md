# Error Model

`src/error.rs` defines the crate-wide error taxonomy. Every fallible public
operation returns `Result<_, Error>`; the finer-grained enums below feed into
it.

## Taxonomy

> [spec:pgorm:def:error.model+11]
> `Error` is the crate-wide error enum. Driver and pool failures convert in
> via `From`: `Postgres(tokio_postgres::Error)` (the variant every
> `ConnectionTrait` call and transaction commit produces on database failure;
> its `Display` includes the server `DbError` detail when present) and
> `Pool(pgorm_pool::PoolError)` (produced by `DatabasePool::get`; pool
> exhaustion and acquisition timeouts surface here). `Verify(VerifyError)`
> converts in the same way from the statement-verification failures of
> `exec.verify.errors`, `QueryBuilder(pgorm_query::Error)` from the
> builder layer's own refusals — the template constructors and
> `inject_parameters` — and `Listen(ListenError)` from a listener's
> (`conn.listen`): a channel name too long to listen on, and the overflow
> that ends its notifications, so `?` carries them across the crate boundary
> without a hand-written `map_err` at every ORM-level call site. The
> `From` impl is that variant's construction site. The remaining variants
> are constructed by pgorm itself: `Conversion { from, into, source }`,
> `Query(RuntimeError)`, `ConvertFromU64(&'static str)`, `UnpackInsertId`,
> `PrimaryKeyNotSet`, `AttrNotSet(String)`, `Type(String)`,
> `Json(String)`, `RecordNotFound`, `RecordNotInserted`, `NothingToSet`
> and `Custom(String)`. Every variant MUST have at least one construction
> site: variants that no code can produce are removed rather than kept as
> documentation. `RecordNotUpdated` was removed under that rule when
> `exec.crud.update` deleted `Updater::check_record_exists`, its only
> producer.
>
> `NothingToSet` is the counter-case: it exists because
> `[spec:pgorm:sem:exec.crud.update]` raises it, and it names the statement
> that could not be sent rather than a row that was not found — an update
> with no column to set, which is not the `Ok(0)` of a `WHERE` that matched
> nothing.
>
> `Error` implements `PartialEq`/`Eq` by comparing `Display` strings, so two
> errors with distinct payloads but identical rendered messages compare
> equal. A separate `ColumnFromStrError(String)` covers `FromStr` failures on
> entity columns.
>
> The crate root exports `Error` alongside
> `pub type Result<T, E = Error> = std::result::Result<T, E>`. The default
> type parameter makes `pgorm::Result<T>` the ordinary spelling while still
> admitting a foreign error type, as the transaction closures do.
>
> Every public name in this taxonomy MUST be spelled in full: `Error`,
> `RuntimeError`, `ColumnFromStrError`, `SqlError` and its
> `ConstraintViolation` payload, `VerifyError`, `ListenError`, and
> pgorm-query's `ValueTypeError` / `ValueTupleError`. An
> `Err`-abbreviated spelling MUST NOT
> be reintroduced, and no alias is kept behind for a superseded one. A variant
> name MUST NOT restate its enum: the conversion failure is `Error::Conversion`
> and the wrapped database failure is `TryGetError::Db`.
>
> `Error`, `SqlError`, `VerifyError`, `ListenError` and `TryGetError` are
> `#[non_exhaustive]`: each names kinds of failure, and kinds of failure grow,
> so a caller's `match` ends in an arm for one it does not know
> (`sql.surface`). `RuntimeError` is not: `error.model.runtime` fixes it at its
> one variant, a carrier of a message that a caller takes apart without a
> second arm, and a new kind of failure becomes a variant of `Error`'s rather
> than of its.

> [spec:pgorm:def:error.model.runtime+3]
> `RuntimeError` has exactly one variant, `Internal(String)`, wrapping
> pgorm-internal failure messages, and is the payload of the sole
> `RuntimeError`-carrying variant of `Error`, `Query`. Three crate-private
> helpers build an `Error` from anything `ToString`: `query_err`
> (`Query(Internal(..))`), `type_err` (`Type`) and `json_err` (`Json`). Each
> helper has at least one call site; a helper whose variant or whose call
> sites are gone is removed rather than kept behind `#[allow(dead_code)]`.

## SQLSTATE classification

> [spec:pgorm:sem:error.model.sql-class+4]
> `SqlError` classifies constraint failures by SQLSTATE:
> `UniqueConstraintViolation` for `23505` (`unique_violation`),
> `ForeignKeyConstraintViolation` for `23503` (`foreign_key_violation`),
> `CheckConstraintViolation` for `23514` (`check_violation`),
> `NotNullConstraintViolation` for `23502` (`not_null_violation`) and
> `ExclusionConstraintViolation` for `23P01` (`exclusion_violation`), which
> PostgreSQL 18 raises for an `EXCLUDE` constraint and for a primary or
> unique key ending `WITHOUT OVERLAPS` alike. `Error::sql_error()` is the
> classifier entry point, returning `None` for anything that is not one of
> these. `SqlError` is `#[non_exhaustive]`, so a further class is not a
> breaking change to a caller's `match`.
>
> Every variant carries a `ConstraintViolation`, also `#[non_exhaustive]`,
> holding the server's `message` and the names it reported with it, each an
> `Option<String>`: `schema`, `table`, `column`, `datatype` and
> `constraint`. Which are present depends on what refused the value, and is
> PostgreSQL's report rather than pgorm's: a table's unique, foreign-key,
> check or exclusion constraint names the `schema`, `table` and
> `constraint`; a table's `NOT NULL` column names the `schema`, `table` and
> `column` and no constraint; a domain's `CHECK` names the `schema`, the
> domain as `datatype`, and the `constraint`; a domain's `NOT NULL` names
> only the `schema` and the domain. A variant's `Display` is its class
> followed by the server message (`Check Constraint Violated: <message>`).
>
> The classifier inspects only `Error::Postgres`, taking the driver error's
> `as_db_error()` and matching the resulting `DbError::code()` against those
> five SQLSTATEs; the `ConstraintViolation` fields are that same
> `DbError`'s `message()`, `schema()`, `table()`, `column()`, `datatype()`
> and `constraint()`. An `Error::Postgres` with no server-side `DbError` (a
> transport or protocol failure), any other SQLSTATE, and every
> non-`Postgres` variant all yield `None`.

## Bind refusals

> [spec:pgorm:sem:error.model.bind-refusal]
> `BindRefusal` is a value the driver refused to encode while building a
> statement's bind message, before anything reached the server: `parameter`,
> the one-based number of its `$N` placeholder, and `reason`, the codec's
> refusal of the value where the driver kept one — pgorm's own for a `Value`
> the parameter's type cannot take (`exec.cursor.binding-accepts`), such as
> ``cannot bind a `Decimal` value to Postgres type `money` `` or
> ``value `9223372036854775808` is out of range for Postgres type `int8` ``.
> `BindRefusal::of(&tokio_postgres::Error)` is the classifier, `None` for any
> other failure: tokio-postgres 0.7.18 exposes no public kind for an encode
> failure, so it reads the phase and the zero-based position off the error's
> top-level text (`error serializing parameter N`), never off a cause, whose
> text the value can shape, and takes the reason from the error's source.
> `Error::bind_refusal()` applies it to `Error::Postgres` and yields `None`
> for every other variant. Its `Display` is `parameter $N cannot be bound:`
> followed by the reason, or the first part alone; each language binding
> reports a refusal with exactly that text (`bindings.errors`). The struct is
> `#[non_exhaustive]`.

> [spec:pgorm:sem:error.model.decode-refusal]
> `DecodeRefusal` is a result the driver could not read into what was asked
> for: `reading`, the driver's account of what it was reading — `error
> deserializing column N` (zero-based), ``invalid column `name` ``, `query
> returned an unexpected number of columns`, or `query returned an
> unexpected number of rows` — and `reason`, the decoder's refusal of the
> value where the driver kept one. `DecodeRefusal::of(&tokio_postgres::Error)`
> is the classifier, `None` for any other failure, reading the phase off the
> error's top-level text as `BindRefusal::of` does and the reason off its
> source; `Error::decode_refusal()` applies it to `Error::Postgres` and yields
> `None` for every other variant. Its `Display` is `reading` followed by `: `
> and the reason, or `reading` alone; each language binding reports the
> failure as its `DecodeError` with exactly that text (`bindings.errors`).
> The struct is `#[non_exhaustive]`.
