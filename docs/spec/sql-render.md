# SQL rendering (pgorm-query backend)

This spec covers the PostgreSQL SQL rendering engine in `pgorm-query/src/backend/`
(`query_builder.rs`, plus the `Oper` helper in `backend/mod.rs`) and the statement
preparation layer in `pgorm-query/src/prepare.rs`. It is a maintenance-scope spec:
rules capture what the renderer emits today, including known limitations, not what
an ideal Postgres renderer would emit.

## The renderer

> [spec:pgorm:def:sql.render+1]
> The SQL renderer is the unit struct `QueryBuilder` in
> `pgorm-query/src/backend/query_builder.rs`. It is the only backend: pgorm-query
> renders exclusively PostgreSQL-dialect SQL. `QueryBuilder` walks the statement
> AST (`SelectStatement`, `InsertStatement`, `UpdateStatement`, `DeleteStatement`
> and the DDL statement types) and emits SQL text into a `SqlWriter` sink through
> `prepare_*` methods. Rendering is infallible by construction: all
> writes `unwrap()`, and unsupported AST shapes abort via `panic!` /
> `unimplemented!` rather than returning errors.

> [spec:pgorm:def:sql.render.writer+2]
> `SqlWriter` (`prepare.rs`) is the output sink trait for rendering:
> `fmt::Write + ToString` plus `push_param(value)` and
> `push_param_source_typed(value)`, whose default body is
> `push_param` and which only sinks that emit placeholders override (see
> `sql.render.cast-param-type`). Neither takes a `QueryBuilder` argument: the
> builder is a stateless unit struct, so a sink that needs it names it
> directly. Two sinks exist:
>
> `String` implements `SqlWriter` by rendering each parameter inline —
> `push_param` appends `QueryBuilder::value_to_string(&value)` — producing a
> self-contained SQL string with no bind parameters (the `to_string()` build
> path). It takes the default `push_param_source_typed`: an inline literal has
> no wire format to disagree about.
>
> `SqlWriterValues` implements `SqlWriter` by emitting a placeholder and
> collecting the `Value` into an internal `Vec<Value>`. It is constructed with a
> placeholder string and a `numbered` flag; `into_parts()` returns the final
> `(String, Values)` pair (the `build()` path).

## Render conformance

> [spec:pgorm:req:sql.render.oracle+4]
> Every string `QueryBuilder` renders — through the `to_string()` path with
> literals inlined, or through the `build()` path with `$N` placeholders — MUST
> parse under the PostgreSQL grammar. The contract is enforced inside the test
> suite by an oracle: pgorm-query carries a dev-dependency on `pg_query`, the
> Rust binding to libpg_query (the PostgreSQL server's own parser), at the one
> revision of the necessary-nu fork every pgorm crate names (the root manifest
> says what the fork carries, and `tests/fork_pin_tests.rs` holds the
> declarations and lockfiles to it). That revision is upstream's unreleased
> 18.0.0 on libpg_query 18.1.0, so the oracle parses with PostgreSQL 18.6's
> grammar — the release the live test server runs. Oracle and server read one
> grammar: a render the server would refuse is not passed by a newer parser,
> and a render only 18 accepts (a `VIRTUAL` generated column, a
> `WITHOUT OVERLAPS` key) is not refused by an older one. crates.io's
> `pg_query` carries PostgreSQL 17's grammar; upstream's own 18 release
> replaces the fork when it ships.
>
> The oracle is `pgorm-query/tests/postgres/oracle.rs`. `assert_parses` feeds a
> string to the parser and fails with the parser's message and a caret at the
> token it names; `assert_query_eq` pairs that check with the expected spelling;
> and an `assert_eq!` shim — imported by a test module in place of
> `pretty_assertions::assert_eq` — applies the oracle to any left-hand `String`
> or `&str` that opens with a statement keyword — a query keyword (`SELECT`,
> `VALUES`, `TABLE`, `WITH`) or the verb of a write, DDL or utility statement
> (`INSERT`, `CREATE`, `COPY` and the rest) — so the existing render
> assertions are held to the grammar without being rewritten one by one. A
> query keyword opens a statement whether it begins the render bare or
> follows its opening parentheses: the grammar lets a statement begin with
> `(` only as a parenthesised query, which is how a grouped set operation's
> render begins (`[spec:pgorm:sem:sql.ast.select.union]`), and a bare
> `VALUES` or `TABLE` query, which no builder renders today, is a statement
> the shim parses the day one does.
> Rendered fragments that are not whole statements — a parenthesised operand,
> a collation, a tuple among them — are skipped by that keyword test.
> `oracle_sweep.rs` additionally composes a matrix of builder outputs —
> select/join/CTE/window/union/locking shapes, INSERT/UPDATE/DELETE with
> RETURNING and ON CONFLICT, the DDL statements, the `ColumnType` and `BinOper`
> vocabularies, and the `build()` placeholder path — and requires every
> rendering to parse.
>
> The oracle is syntax-only: it has no catalog, so unknown tables and columns,
> misapplied type modifiers (`money(12, 2)`), an empty select list
> (`SELECT  FROM "t"`, which is valid PostgreSQL), and cross-database references
> all pass. The live-Postgres integration suite remains the semantic oracle.
>
> One shape renders text the grammar refuses by design rather than by defect:
> a tie limit with no ORDER BY or beside `SKIP LOCKED`, which no type keeps
> out and which `check_tie_limit` refuses before every pgorm, Python and
> JavaScript path renders (`sql.ast.select.tie-limit`). Its render is held
> rejected, with the parser's message, beside the check that refuses it.
>
> Known exceptions MUST be pinned, not skipped. Each render the grammar rejects
> today has an `oracle_pins_*` test in
> `pgorm-query/tests/postgres/oracle_pins.rs` asserting the rejection and naming,
> in a comment, the plan node that fixes it; the assertion site that produces
> such a render is marked `assert_eq_unparsed!` rather than left on the
> oracle-checked `assert_eq!`. A pin fails as soon as its render becomes valid,
> so a fix cannot land without the pin being retired.

## Placeholders and parameters

> [spec:pgorm:req:sql.render.placeholders+1]
> Parameters MUST be rendered as numbered placeholders `$1`, `$2`, … — the
> only spelling PostgreSQL has, so the mark and the numbering are constants of
> the renderer rather than a setting anything can read or choose. When rendering into a
> `SqlWriterValues`, each `push_param` call MUST increment a counter that starts
> at 0 and write `${counter}` after incrementing, so the first parameter emitted
> is `$1`. Placeholder numbers therefore follow textual emission order exactly,
> and the collected `Values` vector is index-aligned with them (`$N` binds
> `values[N-1]`). Each parameter occurrence gets a fresh number; the renderer
> never deduplicates equal values into a shared placeholder.

> [spec:pgorm:req:sql.render.param-vs-inline+1]
> `SimpleExpr::Value` MUST be rendered through `SqlWriter::push_param` (a `$N`
> placeholder in the `build()` path), while `SimpleExpr::Constant` MUST always
> be rendered inline via `value_to_string`, regardless of sink. `LIMIT` and
> `OFFSET` amounts on SELECT — the only statement that carries either — are
> stored as `Value`s and MUST be parameterized (` LIMIT $N`, ` OFFSET $N`). The
> values in an `Order::Field` ordering (see `sql.render.select-order`) are
> inlined via `value_to_string` even in parameterized mode.

> [spec:pgorm:req:sql.render.cast-param-type+4]
> A `SimpleExpr::Value` cast operand MUST be rendered through
> `push_param_source_typed` rather than `push_param`. There is one place this
> can arise, because a cast has one shape
> (`[spec:pgorm:req:sql.ast.cast-shape]`): the operand of a
> `SimpleExpr::AsEnum`, which every `cast_as` / `as_enum` / `save_as` /
> `cast_as_raw` spelling builds and which renders as
> `CAST(operand AS <TypeName>)`.
> Postgres infers a placeholder's type from the cast target, but the driver
> writes the value in the format of the type it *is*, so an unpinned cast
> operand binds the wrong bytes: `CAST($1 AS BIT(8))` makes the server expect
> `bit` for what the driver sends as an `int8`, and it answers `22P03`.
>
> `SqlWriterValues` therefore appends `::{type}` to the placeholder, taking the
> name from `Value::source_type_name`: `bool`; `int2` for `TinyInt` and
> `SmallInt`, `int4` for `Int`, `int8` for `BigInt`, `Unsigned` and
> `BigUnsigned`; `float4` / `float8`; `text` for `String` and `Char`; `bytea`;
> `date`, `time`, `timestamp`, and `timestamptz`; `uuid`; `numeric`; `inet`; `macaddr`;
> for `Range` and `Multirange`, the built-in range or multirange type their
> tag names (`int4range`, `tstzmultirange`); and, for `Array`, the element
> name from `ArrayType::source_type_name` suffixed `[]`, a range element's
> name being its range type's (`int8range[]`). `Json` and `Vector`
> are `None` and stay unpinned — a JSON payload binds as either `json` or
> `jsonb` depending on which the server asked for, and the pgvector type name
> is not guaranteed to resolve in the search path.
>
> A pin names a type the value can be *bound* as, not necessarily the type its
> own `ToSql` impl would name: `TinyInt` pins to `int2` — rather than to the
> one-byte `"char"` an `i8` binds as by default — because the numeric coercion
> of `[spec:pgorm:req:exec.cursor.binding-coerce]` widens it, and because
> `CAST($1::int2 AS text)` yields the digits of the number where
> `CAST($1::"char" AS text)` would yield the character with that code point.
> `Unsigned` pins to `int8` rather than `oid` for the same reason. The one
> shape this costs is a cast from a `TinyInt` to `"char"` itself, for which
> `int2` has no cast: that (never-exercised) target now fails when the
> statement is parsed instead of binding.
>
> Pinning only annotates the placeholder — the outer cast still runs — so
> `CAST($1::text AS tea)` reaches an enum through Postgres' text I/O
> conversion, exactly as the unpinned form did.

> [spec:pgorm:sem:sql.render.placeholder-typing+1]
> Every `SimpleExpr::Value` renders as a `$n` placeholder
> (`sql.render.param-vs-inline`), and PostgreSQL types a placeholder from the
> context it appears in. A placeholder in a position that supplies *no* context
> resolves to `text`, and the value the driver then binds is neither the type the
> server inferred nor convertible to it. The two positions that supply no context
> are a bare projection — `SELECT $1`, whether standalone, in a LATERAL marker
> subquery, or as the anchor arm of a recursive CTE, whose column types the whole
> recursion is resolved from — and any operand whose siblings are themselves
> untyped.
>
> The failures are the server's, at parse or bind time, not the renderer's: a
> recursive CTE whose anchor projects a bare `$n` settles that column as `text`
> and the recursive arm's `"col" + $n` fails `42883`
> (`operator does not exist: text + …`), while a bare `SELECT $1` bound with an
> integer fails `22021` (`invalid byte sequence for encoding "UTF8"`) because the
> driver wrote `int4` bytes where the server asked for `text`.
>
> pgorm-query annotates a placeholder in exactly two places — the operand of a
> cast, per `sql.render.cast-param-type`, and a value in an SQL/JSON position
> that gives it no type, per `sql.render.sql-json` — and MUST NOT guess a type
> anywhere else. Neither is a guess: the type written is the value's own, in a
> position that accepts it as it is. Anywhere else the renderer has no catalog
> and no expression typing, so an annotation would be a guess that silently
> changes what the statement means.
> Supplying the context is therefore the caller's obligation, and it has two
> spellings: annotate the value with `Expr::val(v).cast_as("<type>")`, which
> renders `CAST($n::<pin> AS <type>)` and gives the position a type; or, where the
> value is a fixed literal rather than caller data, use `SimpleExpr::Constant`,
> which is inlined and so is never a placeholder at all
> (`join_lateral_on_true`'s `TRUE` is this case, per `query.build.lateral`).

## Identifiers and literals

> [spec:pgorm:req:sql.render.ident-quoting+7]
> The quote is the double quote, and it is the only one: PostgreSQL has a
> single identifier quote, so it is written at the render sites rather than
> carried in a parameter that could hold another character. Every identifier
> rendered through `SqlName::prepare` MUST be wrapped in double quotes with any
> embedded `"` doubled (`SqlName::quoted` replaces `"` with `""`: `he"llo` →
> `"he""llo"`). Quoting is unconditional — there is
> no reserved-word or safe-character check. This applies to column names, table
> names, schema/database qualifiers, aliases, CTE names, window names, and
> index/constraint/foreign-key names. Multi-part references join the quoted
> parts with `.` (e.g. `"schema"."table"."column"`).
>
> No caller-supplied identifier reaches output unquoted. The bound is what
> says so: a position that renders a *name* takes an identifier type or a
> `TypeName`, and every such position renders through `SqlName::prepare` or
> through `TypeName`'s part policy (`sql.types.type-name`), which quotes
> anything that is not a lowercase identifier and every keyword the
> position's grammar would not read bare as that name, save the type
> spellings and call forms that rule lists per keyword. In particular
> `ColumnType::Named` carries a `TypeName` and renders through
> `to_sql_string`, `IndexType::Named`'s access method renders through
> `TypeName::prepare_part`, `Function::Named` function names render under
> the same part policy, and `ExtensionCreateStatement`'s schema is a
> `Name` like every other schema qualifier. Each of those three was once
> a bare `SqlName::to_string`, which emitted a hostile catalogue name as SQL;
> the fix is the one shared policy rather than a per-site escape, so a new
> render site inherits it.
>
> Only two things in the crate render caller text verbatim, and neither is a
> name: `TypeName::raw` (bound to `&'static str`, the only way to set the
> private field it fills, and verbatim for that text alone — a schema on it
> is a name) and `SimpleExpr::Raw` (bound to `&'static str` by
> `Expr::raw`). Both carry program text rather than data, and their bounds
> are what keeps that true.

> [spec:pgorm:req:sql.render.string-escape+1]
> `QueryBuilder::escape_string` MUST apply exactly these replacements, in
> order: `\` → `\\`, `'` → `\'`, NUL (`\0`) → `\0`, backspace
> (0x08) → `\b`, tab (0x09) → `\t`, LF → `\n`, CR → `\r` — and nothing
> else. Every mapping is an escape PostgreSQL's `E''` language defines;
> `\z` and `\"` are NOT in that language (the server drops the backslash,
> so the old `0x1A → \z` mapping silently turned 0x1A into the letter
> `z`), and 0x1A and the double quote are legal string characters that
> MUST pass through raw and round-trip byte-identically with the bound
> path. When rendering a string literal (`write_string_quoted`), the
> escaped text is wrapped in single quotes; if the escaped text contains
> any backslash the literal MUST instead be an E-string, `E'...'`, so
> Postgres interprets the backslash escapes. `unescape_string` is the
> inverse mapping (a backslash followed by `0 b t n r` maps back to the
> control character; any other escaped character maps to itself).

> [spec:pgorm:def:sql.render.value-literals+5]
> `value_to_string` defines the inline literal syntax per `Value` variant:
>
> Every `None` variant of every `Value` type renders as the bare keyword
> `NULL`. `Bool` renders `TRUE` / `FALSE`. All integer, unsigned, float,
> double, and `Decimal` values render via their plain `Display` output,
> unquoted. `String` and `Char` render via `write_string_quoted` (see
> `sql.render.string-escape`); a `Char` is encoded as the full UTF-8 text of its
> scalar value and then escaped and quoted exactly as the equivalent
> one-character `String`, so `é` renders as `'é'`, `—` as `'—'`, and `'` as
> `E'\''`. The char is never narrowed to a single byte: rendering a char is
> total, and the rendered text always denotes the char that was given. `Json`
> renders its compact serialization as a quoted string. `Bytes` renders as a
> Postgres hex bytea literal `'\xAB01…'` with uppercase two-digit hex per byte.
> Temporal values render single-quoted with fixed formats: date `%Y-%m-%d`,
> time `%H:%M:%S%.f`, naive datetime `%Y-%m-%d %H:%M:%S%.f`, and the zoned
> datetime `%Y-%m-%d %H:%M:%S%.f %:z`, always at `+00:00` because the value
> carries an instant rather than an offset. Sub-second digits are kept, to the
> microsecond PostgreSQL stores and the parameter path binds
> (`sql.value.render`); `%.f` omits the fraction and its dot when it is zero,
> so a whole second renders as it would without the directive. A format
> string MUST NOT name a field its value lacks: an unfillable directive is
> written through into the rendered text rather than refused. `Uuid`,
> `IpNetwork`, and `MacAddress` render their `Display` form in single quotes.
>
> `Array(_, Some(v))` renders `ARRAY [e1,e2,…]` — the keyword `ARRAY`, a
> space, then square brackets containing each element recursively rendered by
> `value_to_string`, joined by `,` with no spaces. An *empty* array MUST carry a
> cast naming its element type — `ARRAY []::int4[]`, the name taken from
> `ArrayType::source_type_name` — because there is no element for PostgreSQL to
> infer a type from and it rejects the bare `ARRAY []` with "cannot determine
> type of empty array". The grammar accepts that bare form, so no render oracle
> can catch it (`sql.render.oracle` is syntax-only) and the live-server suite is
> the only witness. The `Json` and `Vector` element tags have no single type
> name to pin (see `sql.render.cast-param-type`), so their empty arrays keep the
> untypeable spelling. `Vector(Some(v))` renders as a quoted pgvector literal
> `'[f1,f2,…]'`.
>
> `Range(ty, Some(r))` renders as `ty`'s constructor called with each bound's
> own literal, `NULL` for no bound and a bracket pair for inclusivity —
> `int4range(1, 5, '[)')` — and the empty range as `'empty'::int4range`;
> `Multirange(ty, Some(m))` as `ty`'s multirange constructor over those,
> `int4multirange()` when empty (`sql.value.render`).
>
> `Keyword` expressions render as bare `NULL`, `CURRENT_DATE`, `CURRENT_TIME`,
> or `CURRENT_TIMESTAMP` — the whole lexicon, with no caller-supplied arm.

## Operators, precedence, and parentheses

> [spec:pgorm:def:sql.render.operators+8]
> `prepare_bin_oper` defines the operator lexicon. Logical/predicate:
> `AND`, `OR`, `LIKE`, `NOT LIKE`, `ILIKE`, `NOT ILIKE`, `SIMILAR TO`,
> `NOT SIMILAR TO`, `IS`, `IS NOT`,
> `IS DISTINCT FROM`, `IS NOT DISTINCT FROM`, `IN`,
> `NOT IN`, `BETWEEN`, `NOT BETWEEN`, `BETWEEN SYMMETRIC`,
> `NOT BETWEEN SYMMETRIC`, `AS`. Comparison: `=`, `<>`,
> `<`, `>`, `<=`, `>=`. Arithmetic: `+`, `-`, `*`, `/`, `%`. Shift: `<<`,
> `>>`. PostgreSQL-specific: `@@` (Matches), `@>` (Contains), `<@`
> (Contained), `||` (Concatenate), `&&` (Overlap), `&<` (NotExtendRightOf),
> `&>` (NotExtendLeftOf), `-|-` (AdjacentTo), `%` (Similarity), `<%`
> (WordSimilarity), `<<%` (StrictWordSimilarity), `<->`
> (SimilarityDistance), `<<->` (WordSimilarityDistance), `<<<->`
> (StrictWordSimilarityDistance), `->` (GetJsonField), `->>` (CastJsonField),
> `#>` (GetJsonPath), `#>>` (CastJsonPath), `?` (HasJsonKey), `?|`
> (HasAnyJsonKeys), `?&` (HasAllJsonKeys), `@?` (JsonPathExists), `~`
> (Regex), `~*` (RegexCaseInsensitive), `!~` (NotRegex), `!~*`
> (NotRegexCaseInsensitive), `AT TIME ZONE` (AtTimeZone), and pgvector's `<->`
> (EuclideanDistance), `<#>`
> (NegativeInnerProduct), `<=>` (CosineDistance).
> `BinOper::Raw(op)` emits its operator text verbatim. Note the deliberate
> lexeme collisions: `%` serves both Mod and Similarity, `<->` both
> SimilarityDistance and EuclideanDistance, and `<<` and `>>` are the shifts
> and the range tests strictly left of and strictly right of alike. The only
> unary operator is `UnOper::Not` → `NOT`.
>
> The `?` family's lexemes are operator text like any other and carry no
> placeholder meaning: parameters are numbered `$N`
> (`sql.render.placeholders`), so a rendered `?` reaches the server as the
> JSON operator it is and nothing in the client rewrites it.
>
> `ESCAPE` is not an operator: it is grammatical only as the tail of a `LIKE`
> / `ILIKE` / `SIMILAR TO` pattern, so it renders from
> `SimpleExpr::LikePattern` — the pattern as a value, then ` ESCAPE ` and the
> escape character as an inline constant, or ` ESCAPE ''` when escaping is
> off — and there is no `BinOper` that could place it anywhere else.

> [spec:pgorm:def:sql.render.precedence+14]
> Parenthesis elision is driven by
> `inner_expr_well_known_greater_precedence(inner, outer)`, which returns true
> (safe to drop parens around `inner`) when: the inner expression is an atom —
> `Column`, `Tuple`, `Constant`, `FunctionCall`, `Value`, `Keyword`, `Case`,
> `SimpleCase`, `Subscript`, `Collate`, `Grouping`, `LikePattern`, `AsEnum`,
> `SqlJson` (`sql.render.sql-json`) or `SubQuery` (all but the first two are
> already self-wrapping — a
> `Subscript` either binds tighter than every operator or wraps its own base,
> per `sql.render.subscript`; a `Collate` wraps itself, per
> `sql.render.collate`, because `COLLATE` binds tighter than every binary
> operator but is not an operand in the grammar's two `b_expr` positions —
> `AsEnum` is
> the cast of `[spec:pgorm:req:sql.ast.cast-shape]` and spells its own
> `CAST(…)` parentheses, which is why folding the second cast shape into it
> could not cost a `BETWEEN` operand its bare rendering); the inner expression
> is an arithmetic (`* / % + -`) or shift (`<< >>`) binary and the outer
> operator is a comparison, BETWEEN, IN, LIKE, or logical operator; or the
> inner expression is a comparison, BETWEEN, IN, LIKE, or IS binary and the
> outer operator is logical (`AND`/`OR`/`NOT`). Each of those families is the
> whole family and not a pair: BETWEEN is `BETWEEN`, `NOT BETWEEN` and both
> `SYMMETRIC` forms, and IS is `IS`, `IS NOT`, `IS DISTINCT FROM` and
> `IS NOT DISTINCT FROM`, and LIKE is `LIKE`, `NOT LIKE`, `ILIKE`,
> `NOT ILIKE`, `SIMILAR TO` and `NOT SIMILAR TO`, which the grammar ranks
> together at one level — a new operator added to
> one of these lexemes but not to its `Oper` predicate is the failure mode
> this clause exists to name.
> A BETWEEN is bare under the logical operators for the reason a comparison
> is: the grammar ranks BETWEEN above `NOT`, `AND` and `OR`, so in
> `a BETWEEN 1 AND 2 AND b` the first `AND` is BETWEEN's own and the second
> the logical operator, the one reading the grammar admits. Its own `AND` is
> no reason to keep the parentheses, and no rule kept them on purpose; its
> bounds are operands of the BETWEEN rather than of that `AND`
> (`sql.render.parens`).
> The Postgres-specific extension also
> treats an inner `@>`, `<@`, `&&`, `&<`, `&>`, `-|-`, `%` (similarity), `<%`,
> `<<%`, `@@`, `?`, `?|`, `?&`, `@?`, `~`, `~*`, `!~` or `!~*` comparison as
> higher precedence than a logical outer
> operator — the membership of that set is "returns boolean", which is why the
> JSON existence operators join it and the JSON *accessors* (`->`, `->>`,
> `#>`, `#>>`, which return JSON or text) do not. Every member is a generic
> operator, which the grammar ranks above the comparisons (the similarity `%`
> ranks with `*`), so its bare form under `AND`, `OR` and `NOT` groups as the
> expression was built; a boolean-valued variant missing from the set keeps
> parentheses its peers drop. The same ranking puts every member above the
> comparisons, IS, BETWEEN, IN and LIKE, so the set also renders bare as
> either operand of any of those: the grammar reads `"a" @> $1 = TRUE` as
> `("a" @> $1) = TRUE` and `TRUE IS DISTINCT FROM "a" ~ $1` with the match as
> the right operand, and a BETWEEN's lower bound, a `b_expr`, admits a
> generic operator, so `x BETWEEN "a" ~ $1 AND "b" ~ $2` keeps both matches
> as its bounds. A comparison, IS, BETWEEN, IN or LIKE inner expression is
> not a member and keeps its parentheses under any of them. The range tests
> strictly left of and strictly right of are the shifts' `<<` and `>>`,
> already bare there as shifts. An
> operator with no `BinOper` variant reaches the renderer only as
> `BinOper::Raw`, whose result the renderer cannot know, and keeps its
> parentheses. All other combinations are considered unknown and keep their
> parentheses.

> [spec:pgorm:req:sql.render.parens+5]
> `binary_expr` renders `left op right` and MUST parenthesize each operand by
> default, dropping parentheses only in these cases. Left operand: dropped when
> `sql.render.precedence` says the left is higher-precedence, or when the left
> is a binary expression with the *same* operator and that operator is
> well-known left-associative (`AND`, `OR`, `+`, `-`, `*`, `%`, plus `||` for
> Postgres) — so `a AND b AND c` and `a || b || c` render flat. Right operand:
> dropped when higher-precedence, or under the one structural hack left for a
> ternary construct encoded as nested binaries: the outer operator is in the
> BETWEEN family — `BETWEEN`, `NOT BETWEEN`, and the two `SYMMETRIC` forms —
> and the right is an `AND` binary (`x BETWEEN a AND b`). That `AND` is
> BETWEEN's own keyword, not a logical operator, so it renders bare and each
> bound is placed as an operand of the BETWEEN: bare when
> `sql.render.precedence` ranks it above BETWEEN (an atom, an arithmetic or
> shift binary, a boolean generic operator), parenthesised otherwise. Judged as operands of a logical
> `AND`, a comparison bound would render bare and regroup — the grammar reads
> `x BETWEEN a AND c = d` as `(x BETWEEN a AND c) = d` — and a LIKE lower
> bound would not parse, the lower bound being a `b_expr`. The hack keys on
> `Oper::is_between`, so membership of that predicate, not the lexeme, is what
> decides: an operator whose text begins `BETWEEN` but which the predicate
> does not name renders `x BETWEEN … (a AND b)`, which parses as a comparison
> against a boolean rather than as a range test.
> A unary `NOT` likewise wraps its operand
> in parentheses unless the operand is higher-precedence per
> `sql.render.precedence`.
>
> The other two hacks this once carried are gone with the shapes that needed
> them. `LIKE`'s `ESCAPE` tail is a `SimpleExpr::LikePattern` rather than a
> nested binary, and a cast is a `SimpleExpr::AsEnum` rather than an `AS`
> binary over a `SimpleExpr::Raw`
> (`[spec:pgorm:req:sql.ast.cast-shape]`) — both are atoms in
> `sql.render.precedence`, so ordinary elision covers them and no operator
> pair needs a special case.

> [spec:pgorm:sem:sql.render.empty-in+1]
> A binary `IN` or `NOT IN` whose right side is an empty tuple is rewritten to a
> comparison of two string values, rendering as `$N = $M` with those values as
> parameters (or with the literals inline under `to_string`). The rewrite is
> asymmetric, so each side keeps the truth value its predicate has over an empty
> set: empty `IN` becomes `'a' = 'b'` and matches no rows, empty `NOT IN`
> becomes `'a' = 'a'` and matches every row. Both operands are non-NULL literals
> in either form, so neither can evaluate to UNKNOWN.

## Conditions

> [spec:pgorm:req:sql.render.condition-chain+1]
> `prepare_condition(holder, keyword, …)` MUST emit nothing at all when the
> condition holder carries no condition; otherwise it emits ` {keyword} ` (with
> surrounding spaces; keyword is `WHERE`, `HAVING`, or `ON`) followed by the
> condition. There is exactly one rendering path: the `all`/`any` `Condition`
> tree is lowered by its `to_simple_expr()` conversion and written by the
> ordinary expression renderer, so parenthesisation and operator precedence
> follow `sql.render.precedence` alone. A holder carrying an empty `Condition`
> still emits the keyword, followed by that set's constant (`TRUE` for `all`,
> `FALSE` for `any`) per `sql.ast.condition.flattening`.

## SELECT

> [spec:pgorm:req:sql.render.select-order+7]
> `prepare_select_statement` MUST emit clauses in exactly this order: the
> statement's carried WITH clause when it has one (`query.build.with`), rendered
> through the same `prepare_with_clause` the three write statements use and
> therefore already ending in a separating space; one `(` for each set
> operation arm that regroups the chain before it (`sql.ast.select.union`);
> `SELECT`; optional distinct (`ALL`, `DISTINCT`, or `DISTINCT ON (col, …)`);
> the comma-separated select expressions; ` FROM ` with comma-separated table
> references (omitted entirely when no from-table); one space-separated join
> expression per join; the WHERE condition; ` GROUP BY `, its `DISTINCT`
> when set, and its grouping elements (`sql.render.grouping`); the
> HAVING condition; the WINDOW clause when the statement declares a window,
> ` WINDOW "name" AS ( … )` with each further entry after a comma
> (`sql.render.window`); any union clauses; ` ORDER BY ` expressions;
> the row limit, ` LIMIT $N` or a tie limit in its place
> (`sql.render.tie-limit`); ` OFFSET $N`; and the row-locking clause. The WINDOW
> clause belongs to the same query level as HAVING, so it MUST precede
> the set operations and the ORDER BY/LIMIT/OFFSET/locking tail that apply to
> the combined result — that is the position PostgreSQL's grammar requires,
> and emitting it later makes a named window unusable with any of them.
>
> Each order expression renders the expression, then ` ASC` / ` DESC`, then
> optionally ` NULLS FIRST` / ` NULLS LAST`. An `Order::Field(values)`
> ordering instead renders a `CASE WHEN expr=v0 THEN 0 WHEN expr=v1 THEN 1 …
> ELSE n END` expression with the values inlined, emulating MySQL
> `FIELD()`-style ordering. Unions render as ` UNION (…)`, ` UNION ALL (…)`,
> ` INTERSECT (…)`, ` INTERSECT ALL (…)`, ` EXCEPT (…)` or ` EXCEPT ALL (…)`
> with the sub-select parenthesized. An `INTERSECT` or `INTERSECT ALL` arm
> that follows a `UNION` or `EXCEPT` arm of either form is preceded by a `)`
> closing one of the parentheses opened before `SELECT`, so
> `a.union(b).intersect(c)` renders `(SELECT … UNION (…)) INTERSECT (…)`:
> PostgreSQL's grammar takes a parenthesised set operation as an operand, and
> the WITH clause stays outside it, ahead of the whole statement. The ORDER
> BY, row limit, OFFSET and locking tail follow the last arm, outside every
> parenthesis, and so apply to the combined result as before.

> [spec:pgorm:req:sql.render.joins+2]
> Join types MUST render as `JOIN`, `CROSS JOIN`, `INNER JOIN`, `LEFT JOIN`,
> `RIGHT JOIN`, `FULL OUTER JOIN`, followed by the joined table reference
> (prefixed `LATERAL ` when the join is marked lateral). A `JoinKind::Qualified`
> join then renders its constraint as an ` ON …` condition via
> `sql.render.condition-chain`; a `JoinKind::Cross` join renders nothing after
> the table, because PostgreSQL admits no `ON` after `CROSS JOIN` and the AST
> carries no condition for one (`sql.ast.select.join`).
> `JoinOn` has exactly one form, `Condition`, so `prepare_join_on` is total.
> There is no `USING (…)` output: the `JoinOn::Columns` variant that stood for
> it was never constructed by any builder and rendered `unimplemented!()`, and
> it MUST NOT return without a renderer.

> [spec:pgorm:sem:sql.render.locking+1]
> A lock clause renders as `FOR ` plus `UPDATE`, `NO KEY UPDATE`, `SHARE`, or
> `KEY SHARE`; then ` OF ` with its names, comma-separated and each a quoted
> unqualified identifier, when it names any; then optionally ` NOWAIT` or
> ` SKIP LOCKED`.

> [spec:pgorm:req:sql.render.tie-limit]
> A tie limit MUST render ` FETCH FIRST $N ROWS WITH TIES` in the place
> ` LIMIT $N` takes, its count a parameter as LIMIT's is, and an OFFSET
> follows it as it follows `LIMIT` — the grammar admits OFFSET on either side
> of a `FETCH FIRST`. Rendering does not consult `check_tie_limit`: a tie
> limit with no ORDER BY, or beside `SKIP LOCKED`, renders the same text,
> which the grammar refuses (`sql.ast.select.tie-limit`,
> `sql.render.oracle`).

> [spec:pgorm:req:sql.render.table-function+2]
> A `FromItem::TableFunction` MUST render each call as a function call does
> in FROM — its name, then its arguments parenthesised — comma-separated
> inside `ROWS FROM (` … `)` when it is a `ROWS FROM`; then
> ` WITH ORDINALITY` when it numbers its rows; then ` AS "alias"`, and the
> column names parenthesised directly after the alias, `AS "t"("a", "b")`,
> when it names them. The calls' arguments are bound as any argument is. A
> call's column definition list renders as `sql.render.column-definitions`
> says.

> [spec:pgorm:req:sql.render.column-definitions]
> A column definition list MUST render parenthesised, each column its quoted
> name, a space and its type as a column definition writes it
> (`sql.render.ddl.types`), comma-separated: `("a" integer, "b" text[])`. A
> `TableFunction` of one call whose list stands in the alias — no
> `ROWS FROM` asked for, no ordinal column and no column names — renders it
> directly after the alias, `JSON_TO_RECORD($1) AS "t"("a" integer)`; any
> other renders as a `ROWS FROM`, each call with a list followed by ` AS `
> and its list inside the `ROWS FROM (` … `)`,
> `ROWS FROM (JSON_TO_RECORD($1) AS ("a" integer), GENERATE_SERIES(1, 2))`.

> [spec:pgorm:req:sql.render.table-sample]
> A `FromItem::TableSample` MUST render the sampled table as a named table
> renders — its name, then ` AS "alias"` when one is bound, since the grammar
> puts the alias before the clause — then ` TABLESAMPLE BERNOULLI (` or
> ` TABLESAMPLE SYSTEM (`, the percentage as a parameter, and `)`; then, when
> seeded, ` REPEATABLE (`, the seed as a parameter, and `)`.

> [spec:pgorm:req:sql.render.window+7]
> A window specification is never emitted bare: `prepare_window_spec` wraps a
> `WindowStatement` in `( ` … ` )` (note the spaces inside the parentheses),
> `prepare_extended_window` wraps an `ExtendedWindow` the same way, and they
> are the only ways a specification reaches the sink. Both spelling sites
> therefore agree, for either kind of window — an inline windowed projection
> renders ` OVER ( … )`, ` OVER ( "base" ORDER BY … )` when it is built on a
> declared window, and a WINDOW clause entry renders `"name" AS ( … )` — and an empty specification
> renders as the legal `(  )`. `prepare_window_clause` writes the clause:
> ` WINDOW ` and its entries in the order the statement holds them, separated
> by `, `, or nothing when it declares no window. An extended window writes
> its base's quoted name where a window of its own writes its PARTITION BY,
> then the same ORDER BY and frame: `( "base" ORDER BY … ROWS … )`, or
> `( "base" )` alone. A projection that references a declared window by name
> instead renders ` OVER "name"`, with any projection alias following the
> window in either form (` OVER … AS "alias"`).
>
> `OVER` reaches the sink only after a projection whose expression is a
> `SimpleExpr::FunctionCall`, because that is the only production PostgreSQL's
> grammar allows it after; a windowed column reference, arithmetic expression,
> `CASE` or `CAST` is rejected by the grammar no matter how it is spelled. The
> renderer carries no guard for this and MUST NOT grow one — the projection it
> would reject is not constructible (`sql.ast.window-statement`), so a guard
> would be dead code standing in for a type.
>
> Within the parentheses a specification renders `PARTITION BY expr, …`, then
> ` ORDER BY ` order-exprs, then the frame clause: ` RANGE `, ` ROWS ` or
> ` GROUPS `,
> followed by either `BETWEEN start AND end` when an end bound exists or the
> start bound alone, then ` EXCLUDE ` and `CURRENT ROW`, `GROUP`, `TIES` or
> `NO OTHERS` when an exclusion is set. Frame bounds render `UNBOUNDED
> PRECEDING`, `CURRENT ROW`, `UNBOUNDED FOLLOWING`; an offset bound renders its
> expression through `prepare_simple_expr` — a value as a `$N` parameter in the
> `build()` path and as the literal inline, a cast or arithmetic as itself —
> then a space and the keyword (`$1 PRECEDING`, `CAST('1 day' AS interval)
> PRECEDING`, `1 + 1 FOLLOWING`). The offset takes no parentheses of its own:
> the keyword after it ends the expression, as `WHEN` and `THEN` do in
> `sql.render.case`.
>
> The WINDOW clause, both specifications and the frame render in a child
> module of the query builder, `query_builder_window.rs`. A frame whose bound,
> `BETWEEN` or exclusion went missing still parses — as a different frame —
> so each rendering is held to libpg_query's `frame_options` bitmask and its
> offset nodes, not only to the grammar, and a WINDOW clause to the entries,
> names and bases libpg_query reads from it.

> [spec:pgorm:req:sql.render.func-mods]
> `prepare_function_arguments` MUST render a call's parenthesized argument
> list — comma-separated, each argument prefixed `DISTINCT ` when its
> `FuncArgMod` says so — and then, in this order, the two aggregate modifier
> clauses of `sql.ast.func`: ` WITHIN GROUP (ORDER BY ` order-exprs `)` when
> the ordering is non-empty, then ` FILTER (WHERE ` condition `)` when a
> filter is set. Each renders nothing at all when absent, so a call with
> neither is byte-identical to what it rendered before the clauses existed.
>
> The order is PostgreSQL's and is not a choice: its grammar puts WITHIN GROUP
> before FILTER, and both before `OVER`. `OVER` is rendered by
> `prepare_select_expr` (`sql.render.window`) after the whole function
> expression, so `agg(x) FILTER (WHERE c) OVER w` falls out of the two rules
> composing rather than needing either to know about the other — which is the
> reason the modifiers render here, inside the arguments function, rather than
> at the two call sites that reach it.
>
> Both clauses render through the paths their contents already have — order
> expressions through `prepare_order_expr`, the condition through
> `prepare_condition_where` — so a `$N` parameter inside a filter is numbered
> in the enclosing statement's sequence exactly as one in a WHERE is, and a
> filter is subject to the same escaping and quoting as any other condition.
>
> The renderer emits the clauses on whatever function carries them, including
> a `Function::Named` one and a `FromItem::FunctionCall`. It has nothing to
> refuse: per `sql.ast.func` whether a clause is meaningful on a given
> function is PostgreSQL's determination, and a renderer that second-guessed
> it would reject valid calls on functions this enum does not enumerate.

> [spec:pgorm:req:sql.render.sql-json+1]
> A `SimpleExpr::SqlJson` MUST render as an atom: each function form in its
> own call parentheses, and `IS JSON` inside parentheses of its own —
> `("data" IS JSON OBJECT WITH UNIQUE KEYS)` — as `CASE` takes, so
> `sql.render.precedence` never places one. Inside them, the operand of
> `IS [NOT] JSON` is parenthesised when it is an operator expression, because
> `NOT`, `AND` and `OR` bind looser than `IS`.
>
> The query functions render `JSON_EXISTS(` / `JSON_VALUE(` / `JSON_QUERY(`,
> the context item, `, `, the path, then ` PASSING v AS "name"` (further
> pairs comma-separated), ` RETURNING <type>`, `JSON_QUERY`'s one shaping
> clause (` WITH UNCONDITIONAL WRAPPER`, ` WITH CONDITIONAL WRAPPER` or
> ` OMIT QUOTES`), then ` <behaviour> ON EMPTY` and ` <behaviour> ON ERROR`,
> each clause only when set and always in this order, the grammar's. The path
> MUST render as `CAST(<path> AS jsonpath)` with the path a `text` value
> pushed through `push_param_source_typed`: under `build()` that is
> `CAST($n::text AS jsonpath)`, a bound parameter whose type is the one the
> driver writes, and under `to_string()` the escaped literal. A behaviour
> renders `NULL`, `ERROR`, `TRUE`, `FALSE`, `UNKNOWN`, `EMPTY ARRAY`,
> `EMPTY OBJECT` or `DEFAULT <literal>`. The `DEFAULT` value MUST render
> inline under both paths, through `value_to_string` — the literal escaping
> of `sql.render.string-escape` and `sql.render.value-literals` — because the
> server refuses a parameter there (`42804`). It is the one value of an
> SQL/JSON expression that is never bound, and since it is a `Value` rather
> than an expression, nothing but an escaped literal can reach that position.
>
> The constructors render `JSON_OBJECT(k : v, …[ ABSENT ON NULL][ WITH UNIQUE
> KEYS][ RETURNING t])`, `JSON_ARRAY(v, …[ NULL ON NULL][ RETURNING t])`,
> `JSON_ARRAY(<select>[ RETURNING t])`, `JSON_OBJECTAGG(k : v[ ABSENT ON
> NULL][ WITH UNIQUE KEYS][ RETURNING t])`, `JSON_ARRAYAGG(v[ ORDER BY …][
> NULL ON NULL][ RETURNING t])`, `JSON(v[ WITH UNIQUE KEYS])`,
> `JSON_SCALAR(v)` and `JSON_SERIALIZE(JSON(v)[ RETURNING t])`; an aggregate's
> ` FILTER (WHERE …)` follows its parentheses as a `FunctionCall`'s does
> (`sql.render.func-mods`). `JSON_SERIALIZE`'s operand MUST be read through
> `JSON(..)`, with its ` FORMAT JSON` inside: PostgreSQL 18.6's
> `JSON_SERIALIZE` writes a `jsonb` operand as bytes of its binary header, and
> `JSON(..)` gives it the `json` value it makes of every other operand
> (`sql.ast.expr.sql-json`). A clause with nothing before it between the
> parentheses has no leading space, so `JSON_OBJECT(RETURNING jsonb)` and the
> empty `JSON_OBJECT()` and `JSON_ARRAY()` render as PostgreSQL writes them.
> An object member MUST render `key : value`, not `key VALUE value`: the
> `VALUE` form takes only a `c_expr` as its key, which a typed placeholder
> `$1::text` is not (`42601`). A JSON input's ` FORMAT JSON` follows its
> expression — before `JSON_ARRAYAGG`'s `ORDER BY`, the other order being
> `42601`.
>
> A value in a position that gives it no type — a context item, a `PASSING`
> value, an object key or value, an array element, an aggregate's operands, the
> operand of `JSON()`, `JSON_SCALAR()`, `JSON_SERIALIZE()` and `IS JSON` —
> MUST render followed by `::<its type>` under both paths: `$1::int4` bound
> and `5::int4` inline, the type being `Value::source_type_name`'s, and
> `jsonb` for a `Value::Json`, which has none of its own but is JSON in every
> such position. PostgreSQL otherwise types these parameters `text` or cannot
> type them at all (`42P18` in `JSON_OBJECT` and `JSON_OBJECTAGG`), and the
> type is what the JSON means: `JSON_SCALAR($1::int4)` is the number `5` where
> a `text` parameter is the string `"5"`, and a `jsonb` member nests where a
> `text` one is a string. Annotating the inline literal too is what keeps
> `to_string()` building the JSON `build()` does — an untyped `'{"b":1}'`
> would embed as a string. This is the second place a placeholder carries a
> type, beside a cast's operand (`sql.render.cast-param-type`), and it is no
> guess: the type is the value's own, in a position that accepts any type
> (`sql.render.placeholder-typing`). An expression other than a value is
> written as it is, and carries its own type.

> [spec:pgorm:req:sql.render.json-table]
> A `FromItem::JsonTable` MUST render `JSON_TABLE(` the context item, `, ` the
> root path, ` AS "name"` when the root path is named, the `PASSING` list as
> `sql.render.sql-json` writes it, ` COLUMNS (` the columns comma-separated
> `)`, ` ERROR ON ERROR` or ` EMPTY ON ERROR` when set, then `) AS "alias"`.
> A column renders `"name" FOR ORDINALITY`; `"name" <type>[ PATH <p>][
> <behaviour> ON EMPTY][ <behaviour> ON ERROR]`; `"name" <type> FORMAT JSON[
> PATH <p>][ <shaping>][ <behaviour> ON EMPTY][ <behaviour> ON ERROR]`;
> `"name" <type> EXISTS[ PATH <p>][ <behaviour> ON ERROR]`; or `NESTED PATH <p>[
> AS "name"] COLUMNS (..)`, recursively. The type renders as a column's does
> (`sql.render.ddl.types`), and the behaviours and shaping as the query
> functions' do.
>
> Every path — the root, a column's, a nested one — MUST render as a string
> literal under both render paths, through `write_string_quoted`, the escaping
> `sql.render.string-escape` defines for an inlined string: a quote or a
> backslash escaped by a backslash inside `E'..'`, any other path a plain
> `'..'`. It is the literal PostgreSQL requires there, and the same escaping
> is what keeps a path's text from ending the literal: a path is data, so it
> is never interpolated. A
> context item, a `PASSING` value and a `DEFAULT` render as they do in the
> query functions — typed and bound, typed and bound, and an escaped literal.

> [spec:pgorm:req:sql.render.subquery+5]
> A `SimpleExpr::SubQuery` MUST render its optional operator prefix (`EXISTS`,
> `ANY`, `SOME`, `ALL`, `ARRAY`) directly followed by the parenthesized
> sub-statement.
> A `SimpleExpr::Tuple` renders `(e1, e2, …)`; `SimpleExpr::Values` renders
> `(v1, v2, …)` with each element parameterized. As a FROM item,
> `FromItem::SubQuery` renders `(SELECT …) AS "alias"`, `FromItem::ValuesList`
> renders `(VALUES (…), (…)) AS "alias"`, `FromItem::FunctionCall` renders
> `func(args) AS "alias"`, `FromItem::JsonTable` renders per
> `sql.render.json-table` and `FromItem::TableFunction` per
> `sql.render.table-function`; all such forms carry mandatory aliases, and
> `FromItem::Table` renders its `TableName` followed by ` AS "alias"` when an
> alias is bound, as `FromItem::TableSample` does before its sampling clause
> (`sql.render.table-sample`). Contexts that require a plain identifier reference — DDL
> statements, index and foreign-key targets — take a `TableName` instead, so
> a value-bearing reference never reaches them
> (`[spec:pgorm:sem:sql.ddl.panics]`).
>
> `FromItem::Template` renders `(<fragment>\n) AS "alias"`. The fragment is
> the `SqlTemplate`'s resolved segments written in order: each text segment
> verbatim, each resolved value through the ordinary expression path, so the
> fragment's `$N` renumber into the enclosing statement's parameter space
> (`[spec:pgorm:req:sql.render.custom-expr]`). The newline before the
> closing parenthesis is unconditional and load-bearing: the fragment is
> foreign text that may end inside a `--` comment, which would otherwise
> swallow the parenthesis. It is written here rather than required of
> whoever built the fragment, so no caller can forget it.

## CTEs

> [spec:pgorm:req:sql.render.cte+5]
> A `WithClause` renders `WITH ` followed by its comma-separated common table
> expressions; a `RecursiveWithClause` renders `WITH RECURSIVE ` followed by the
> single one it holds. Each CTE renders as: quoted table name; optional
> ` ("col", …)` column list; ` AS `; optional materialization hint
> (`MATERIALIZED ` or `NOT MATERIALIZED `); then the parenthesized
> sub-statement followed by a trailing space — so the separator between two CTEs
> reads `) , ` rather than `), `. For the recursive form the options render
> after the CTE: `SEARCH BREADTH FIRST BY ` / `SEARCH DEPTH FIRST BY ` expr
> ` SET "alias" `, and `CYCLE ` expr ` SET "col" USING "col" `. The renderer has
> nothing to refuse — the empty clause and the multi-CTE recursive clause are
> unrepresentable per `sql.ast.with` and `sql.ast.with.recursive` — so it
> carries no assertion and MUST NOT panic on a caller-built clause.
>
> One `prepare_with_clause` serves every statement that can carry a clause: it
> is called from `prepare_select_statement` (`sql.render.select-order`),
> `prepare_insert_statement` (`sql.render.insert`) and the UPDATE and DELETE
> halves of `sql.render.update-delete`, each for that statement's own carried
> clause and always as the first thing written. A MERGE carries only the plain
> form, so `prepare_merge_statement` calls the plain half,
> `prepare_plain_with_clause`, which `prepare_with_clause` writes a plain
> clause through (`sql.render.merge`). There is no second rendering
> path — no wrapper statement to prefix (`query.build.with.single`) — so a CTE
> query reads identically whichever statement it prefixes and at whatever
> nesting level `SubQueryStatement` places it.

## INSERT / UPDATE / DELETE

> [spec:pgorm:req:sql.render.insert+2]
> `prepare_insert_statement` MUST render: the statement's carried WITH clause
> when it has one (`sql.render.cte`), then `INSERT` (or `REPLACE` when the
> statement's replace flag is set — kept from the MySQL-era API even though
> PostgreSQL has no such statement), ` INTO ` and the table; then either the
> default-values form or the explicit form. The default-values form (used when
> `default_values` is set with no columns and no source) renders the RETURNING
> hook then `VALUES ` and `(DEFAULT)` repeated `num_rows` times,
> comma-separated. The explicit form renders ` ("col1", "col2", …)` then
> either `VALUES ` with one parenthesized comma-separated expression row per
> value row, or, for insert-from-select, the rendered SELECT statement. ON
> CONFLICT (`sql.render.on-conflict`) and RETURNING (`sql.render.returning`)
> follow.
>
> An `Overriding` renders `OVERRIDING SYSTEM VALUE` or `OVERRIDING USER VALUE`
> immediately before the source, in *both* forms — PostgreSQL's grammar admits
> the clause ahead of `DEFAULT VALUES` as readily as ahead of `VALUES` or a
> query. The keyword MUST therefore be derived once, above the branch that
> picks the form, rather than emitted inside one of them: the failure the
> single derivation exists to prevent is the clause being silently dropped on
> whichever path forgot it.

> [spec:pgorm:req:sql.render.on-conflict+2]
> When present, the conflict clause MUST render ` ON CONFLICT`, then its shape.
> `OnConflict::AnyDoNothing` MUST render ` DO NOTHING` and nothing else.
> `OnConflict::Targeted` MUST render its arbiter, then its action. A
> `ConflictArbiter::Inference` renders ` (` the comma-separated target
> elements `)` — each a quoted conflict column or a rendered conflict
> expression — then, if the target carries one, ` WHERE ` and its condition;
> a `ConflictArbiter::Constraint` renders ` ON CONSTRAINT ` and the
> constraint's name, quoted by `SqlName::prepare` like every other
> identifier. `ConflictAction::DoNothing` renders ` DO NOTHING`;
> `ConflictAction::Update` renders ` DO UPDATE SET ` with comma-separated
> assignments, where `ConflictAssignment::Column` renders
> `"col" = "excluded"."col"` (the `excluded` pseudo-table is double-quoted) and
> `ConflictAssignment::Expr` renders `"col" = expr`, followed by ` WHERE ` and
> its condition when the update carries a filter.
>
> Because the target list is non-empty and the assignment list is non-empty by
> construction, and a named arbiter carries no list or predicate to misplace,
> the renderer has no empty case to guard: every clause it can be handed is
> one the PostgreSQL grammar accepts. The one shape the grammar accepts but
> parse analysis does not — `ON CONFLICT DO UPDATE` with no arbiter — is
> unrepresentable per `sql.ast.on-conflict`, which is the only guard available
> since `sql.render.oracle` cannot see it.

> [spec:pgorm:req:sql.render.returning+4]
> A returning clause on INSERT, UPDATE, DELETE or MERGE MUST render as the
> final clause ` RETURNING `, then, when the clause renames either version of
> the written row, `WITH (`, the renames comma-separated with `OLD AS "o"`
> before `NEW AS "n"` whichever was named first, and `) `; then, on a MERGE
> that returns its action, `merge_action()`, followed by `, ` when a list
> follows; then `*`, a comma-separated list of column refs, a
> comma-separated list of expressions, or a comma-separated list of
> expressions each followed by ` AS ` and its quoted name (`"v" AS "w"`). A
> MERGE that returns its action and
> has no list renders ` RETURNING merge_action()`. A clause renaming neither writes no `WITH`. A version's column
> (`ColumnRef::RowColumn`) renders the version's keyword bare, `.`, and the
> quoted column (`old."col"`), and every column of a version
> (`ColumnRef::RowAsterisk`) the keyword and `.*`. The keyword is written
> bare because it is the relation's name rather than a caller's: quoting it
> would parse the same (`sql.ast.returning`). There is no pre-source
> emission point: PostgreSQL spells the returned rows in exactly one
> position, so the renderer has exactly one.

> [spec:pgorm:req:sql.render.update-delete+3]
> Each half MUST open with the statement's carried WITH clause when it has one
> (`sql.render.cte`). `UPDATE ` then renders the table, ` SET ` with
> comma-separated `"col" = expr` assignments, then the FROM clause, then WHERE
> and RETURNING. `DELETE ` renders `FROM ` and the table, then the USING
> clause, then WHERE and RETURNING.
>
> The two relation clauses render through the same path a SELECT's FROM does:
> when the list is non-empty, ` FROM ` (UPDATE) or ` USING ` (DELETE) followed
> by the items comma-separated, each written by `prepare_from_item`, so a
> subquery, a values list, a function call and a `FromItem::Template` fragment
> render in a write statement exactly as they do in a SELECT — parameters of a
> fragment included, which renumber into the enclosing statement's space. An
> empty list renders nothing at all, not a bare keyword: the clause is absent
> from the AST's default, so the overwhelmingly common single-table write is
> byte-identical to what it rendered before the clause existed. Position is
> fixed by the grammar and not negotiable — PostgreSQL requires FROM after
> SET and USING after the target table, both before WHERE, since the WHERE is
> what reads the relations they introduce.
>
> Neither renders ORDER BY or LIMIT:
> PostgreSQL accepts neither on a write statement, and per `sql.ast.update`
> and `sql.ast.delete` the two statements hold nothing to render them from, so
> the renderer has no invalid clause to guard against. Row selection that needs
> an order or a limit is expressed by the caller as a subquery filter over a
> SELECT, which carries both.

## MERGE

> [spec:pgorm:req:sql.render.merge+1]
> A `MergeStatement` MUST open with its carried WITH clause when it has one,
> written by the plain-clause renderer that `prepare_with_clause` also uses
> (`sql.render.cte`). Then come `MERGE INTO `, `ONLY ` when set, and the
> target as a named table (`"schema"."t" AS "a"`). ` USING ` follows with the
> source written by `prepare_from_item`, so every FROM item renders as it does
> in a SELECT (`sql.render.subquery`), and then ` ON ` and the join
> condition. The arms follow: every matched arm, then every not-matched arm,
> then every not-matched-by-source arm, each kind's conditional arms in call
> order and its unconditional arm after them (`sql.ast.merge`). The
> RETURNING clause comes last (`sql.render.returning`).
>
> An arm renders ` WHEN MATCHED`, ` WHEN NOT MATCHED` or ` WHEN NOT MATCHED
> BY SOURCE`, then ` AND ` and its condition when it has one, then ` THEN `
> and its action. The not-matched arm keeps the spelling without `BY TARGET`,
> which PostgreSQL 15 and 16 also read, and the parse is the same
> (`MERGE_WHEN_NOT_MATCHED_BY_TARGET`). The condition
> takes no parentheses. Here `AND` is the clause's keyword rather than an
> operator, and the condition runs to `THEN`, so a top-level `OR` is read
> whole, as the parse tree shows. A not-matched-by-source arm takes a matched
> arm's actions and renders them the same way. The actions render as
> `UPDATE SET "col" =
> <expr>, ..`, `DELETE`, `DO NOTHING`, `INSERT DEFAULT VALUES`, and
> `INSERT ("col", ..)` followed by an optional ` OVERRIDING SYSTEM VALUE` or
> ` OVERRIDING USER VALUE`, then ` VALUES (<expr>, ..)`. The insert's column
> list and its row are written from the same pairs. On the `build()` path
> every value is a `$N` placeholder, numbered in text order.

## COPY

> [spec:pgorm:req:sql.render.copy]
> A COPY statement (`sql.ast.copy`) MUST render as `COPY `, its table or
> `(`its query`)`, for COPY FROM its column list in parentheses when it has
> one, then ` FROM STDIN` or ` TO STDOUT`, then ` WITH (FORMAT <format>` and
> its options, each `, <OPTION> <value>`, and `)`. COPY FROM's `WHERE` comes
> last. `FORMAT` is always written, so the statement names its format rather
> than leaving it to the server's default, and every option is written with
> its value: `FREEZE true`, `HEADER true` or `HEADER match`, `ON_ERROR
> ignore`, `REJECT_LIMIT <n>`, `LOG_VERBOSITY verbose` or `silent`, a forced
> column list as `("a", "b")` or `*`. The options follow the order of
> PostgreSQL's own documentation, `FORMAT`, `FREEZE`, `DELIMITER`, `NULL`,
> `DEFAULT`, `HEADER`, `QUOTE`, `ESCAPE`, `FORCE_QUOTE`, `FORCE_NOT_NULL`,
> `FORCE_NULL`, `ON_ERROR`, `REJECT_LIMIT`, `ENCODING`, `LOG_VERBOSITY`.
>
> Every name is a quoted identifier (`sql.render.ident-quoting`). The
> grammar takes an option's string argument only as a string constant, so
> `DELIMITER`, `NULL`, `DEFAULT`, `QUOTE`, `ESCAPE` and `ENCODING` are written
> as an inlined string value is (`sql.render.string-escape`), an `E'..'`
> string when the escaping writes a backslash.
>
> COPY takes no bind parameters. PostgreSQL prepares it without analysing
> the query inside, so the statement declares none, and a value bound for a
> `$1` in that query is refused (`08P01`, *bind message supplies 1
> parameters, but prepared statement "" requires 0*). `Display` is therefore
> the only rendering. A query source and a `WHERE` are written with
> their values inlined, by the `String` sink (`sql.render.writer`), and there
> is no `build()` whose placeholders nothing could bind.

## CASE

> [spec:pgorm:req:sql.render.case]
> Both forms of `sql.ast.case` MUST render self-parenthesised, `(CASE … END)`,
> which is what places them among the atoms of `sql.render.precedence`. The
> searched form renders each arm as ` WHEN (` condition `) THEN ` result, the
> condition through `prepare_condition_where`. The simple form renders its
> operand once, directly after `CASE `, then each arm as ` WHEN ` value
> ` THEN ` result with the value bare: `WHEN` and `THEN` delimit every
> expression the grammar admits in those positions, so neither the operand nor
> a value needs parentheses of its own, and a compound one (`"a" + 1`,
> `x > 1 AND y < 9`) is read whole. Both forms then render ` ELSE ` and the
> default result when one is set, and ` END)`; the two share that tail.
>
> Parameters number in textual order — the operand, then each arm's value and
> result in turn, then the `ELSE` — per `sql.render.placeholders`, and the
> renderer pins none of them (`sql.render.placeholder-typing`). The server
> types a simple-form arm value from its `=` comparison with the operand, but
> a result is typed only by its sibling results, so a CASE whose results are
> all bound values leaves the caller to supply their type.
>
> The parser tells the two forms apart by one field — libpg_query's
> `CaseExpr.arg`, set for the simple form and empty for the searched one — so
> each rendering is held to that node and not only to the grammar: a simple
> CASE whose operand went missing still parses, as a searched CASE over
> non-boolean arms.

## Grouping sets

> [spec:pgorm:req:sql.render.grouping+1]
> The GROUP BY list MUST render ` GROUP BY ` and its elements
> comma-separated, and nothing at all when it is empty — its `DISTINCT`
> quantifier included, which renders `DISTINCT ` between the keyword and the
> first element (`sql.ast.select.group-distinct`). A set of one
> expression renders that expression bare, so a list built only from the
> plain group-by methods is byte-identical to the flat expression list it
> replaced; a set of several renders them parenthesised, `("a", "b")`, and the
> empty set renders `()`. `ROLLUP` and `CUBE` render the keyword, a space and
> the parenthesised expression list — `ROLLUP ("a", "b")` — except over no
> expressions, where they render `()`: `ROLLUP ()` is not in the grammar, and
> the empty set is exactly what an empty `ROLLUP` or `CUBE` stands for.
> `GROUPING SETS` renders `GROUPING SETS (`, its elements comma-separated by
> this same rule, and `)`. A `SimpleExpr::Grouping` renders `GROUPING` and its
> arguments parenthesised.
>
> A tuple item renders as a parenthesised row, `ROLLUP (("a", "b"), "c")`,
> which is what makes it one unit of the list. The raw grammar does not say
> so: it reads `("a", "b")` as a row expression — here and as a set of
> several alike — and only parse analysis turns a parenthesised row in a
> grouping position into a set. libpg_query, the render oracle, therefore
> shows a grouping-set node only for `ROLLUP`, `CUBE`, `GROUPING SETS` and
> `()`, and the goldens are held to exactly those nodes and kinds; that a row
> groups as one unit is held by the live suite, which counts the rows each
> element yields and reads `GROUPING()`'s bitmask for each.

## Array subscripts

> [spec:pgorm:req:sql.render.subscript]
> A `SimpleExpr::Subscript` MUST render its base, then `[` and the subscript,
> then `]`: an `Index` as its expression, a `Slice` as its lower bound, `:`,
> and its upper bound, an omitted bound rendering as nothing on its side of
> the colon (`[2:]`, `[:3]`, `[:]`). Parameters number in textual order — the
> base's, then the index or each bound.
>
> PostgreSQL's grammar admits a subscript directly after a column reference, a
> parameter, or a parenthesised expression or subquery, and nowhere else:
> `f(x)[1]`, `CAST(x AS int[])[1]` and `ARRAY[1,2][1]` are syntax errors. The
> base is therefore written bare in exactly two cases — a `Column`, and a base
> that is itself a `Subscript`, so that a chain renders as one
> multi-dimensional access (`"m"[2][3]`) rather than as a subscript of a
> subscript's result (`("m"[2])[3]`), which the server refuses per
> `sql.ast.expr.subscript` — and parenthesised in every other case. A `Value`
> base is parenthesised too even though `$1[1]` would parse, because the same
> statement's `to_string` rendering inlines the value as a literal, which
> would not; so is a `Case` or an operator-less `SubQuery` base, whose own
> parentheses would already suffice, because one rule for every non-column
> base is cheaper to hold than a list of the bases that happen to wrap
> themselves.
>
> A bound array base needs its type supplied by the caller
> (`sql.render.placeholder-typing`): `($1)[1]` asks the server to subscript a
> placeholder of unknown type, and `cast_as_type` with an array `TypeName`
> is the spelling that gives it one.

## Collations

> [spec:pgorm:req:sql.render.collate]
> A `SimpleExpr::Collate` MUST render inside its own parentheses: `(`, the
> operand, ` COLLATE `, the collation's name, `)`. The name renders through
> `SqlName::prepare` part by part — the schema, `.`, then the name when it is
> qualified — so each part is double-quoted with any embedded `"` doubled,
> whatever it spells (`"C"`, `"und-x-icu"`, `"pg_catalog"."default"`).
>
> The parentheses are PostgreSQL's own: its deparser writes a `CollateExpr`
> the same way. They are needed because `COLLATE` is an `a_expr` production
> and not a `c_expr`. It binds tighter than every binary operator — only a
> subscript, `::`, `.` and unary minus bind tighter, and none of those is
> written around a `Collate` bare — so it would be an atom under any operator
> the AST places around it, except that two positions take the narrower
> `b_expr`, which has no `COLLATE` at all. A `BETWEEN` lower bound written
> `'a' COLLATE "C"` is a syntax error; a column `DEFAULT 'a' COLLATE "C"`
> parses, and is read as the *column's* collation clause with a default of
> plain `'a'`, a different statement from the one built. Self-wrapped, the
> clause is an atom in every position, which is what places it among the
> atoms of `sql.render.precedence`.
>
> Inside the parentheses the operand is written bare when it already binds at
> least as tightly — a column, a value, a constant, a keyword, a call, a
> cast, a subscript, a tuple, a subquery, a `CASE` or another `Collate`, each
> of which is a `c_expr` or wraps itself — and parenthesised when it is a
> `Unary` or `Binary` expression, a `Raw` fragment, a `Template` or a
> `LikePattern`: `"a" || "b" COLLATE "C"` collates only `"b"`, so a compound
> operand is wrapped to be collated whole, and verbatim text is wrapped
> because nothing about its precedence is known. A `Value` operand is an
> ordinary placeholder, `($1 COLLATE "C")`: the renderer pins no type on it
> (`sql.render.placeholder-typing`), and none is needed, because the server
> types a collated placeholder `text` from the clause itself.
>
> The goldens are held to libpg_query's `CollateClause`: its `collname` parts
> are the name's, and its `arg` is the whole operand — a compound one's
> operator node, not the operator's right-hand side.

## Custom expressions

> [spec:pgorm:req:sql.render.custom-expr+3]
> `SimpleExpr::Raw(s)` MUST be written verbatim, unescaped; its payload is
> `&'static str`, so only program text can reach it.
> `SimpleExpr::Template` carries a `SqlTemplate`, whose template and
> substitutions are resolved against each other at CONSTRUCTION —
> `SqlTemplate::new`, reached through `Expr::template`,
> `Expr::template_with_expr` and `Expr::template_with_exprs`, all of which
> return `Result` — and never at render.
>
> Construction tokenizes the template with the SQL tokenizer (`sql.token`) and
> reads each `$` punctuation token: `$` followed by another `$` contributes one
> literal `$` (consuming both); `$` followed by an unquoted token parsing as a
> non-zero integer `N` denotes substitution `N`, counting from 1, and MAY be
> written any number of times. Every other `$` — one trailing the template, one
> standing before a space or punctuation, `$0`, or `$abc` — is a malformed
> placeholder and MUST be refused with
> `Error::Template { reason: MalformedPlaceholder | ZeroIndex }`; positional
> substitution of "the next value" is not a form this template language has.
> All other tokens pass through as literal text, and because the template is
> tokenized, placeholder-like text inside quoted tokens is neither substituted
> nor counted.
>
> `SqlTemplate::from_sql(sql, values)` is the second constructor, for text
> that is real SQL rather than a template authored for this machinery — a
> caller's own statement, which is where relation-position fragments come
> from (`[spec:pgorm:def:sql.types.table-ref]`). It differs in the `$`
> grammar and in nothing else: only `$N` is a substitution, and every other
> `$` is reproduced verbatim — `$$` included, because in real SQL `$$` opens
> a dollar-quoted body rather than standing in for a literal `$`. This is
> the reading `[spec:pgorm:sem:sql.render.inject]` already does, and it is
> PostgreSQL's own, since the tokenizer reads comments as space and
> dollar-quoted and `E'…'` bodies as quoted
> (`[spec:pgorm:sem:sql.token.limits]`). `MalformedPlaceholder` is
> therefore unreachable from this constructor. Its values are `Value`s, not
> arbitrary expressions: a `$N` in real SQL is a bind marker, and admitting
> an expression there would turn a parameterised statement into a spliced
> one. The grammar is a reading and not a property of the value, so what
> either constructor hands back is the same resolved sequence and no
> consumer of a `SqlTemplate` can tell them apart.
>
> The census MUST come out exact: the set of distinct `N` referenced MUST equal
> `1..=values.len()`. A reference past the end is
> `Error::Template { reason: IndexOutOfRange }`; a supplied value the template
> never names — including the value stranded by an arity hole such as `$1, $3`
> over three values — is `Error::Template { reason: UnreferencedValue }`.
> Rendering a placeholder as literal text, or dropping an unused value, is
> silent wrongness and is not permitted.
>
> What survives construction is a flat sequence of literal-text and
> resolved-expression segments carrying no indices, so rendering writes each
> text segment verbatim, renders each expression in place, and cannot fail or
> index out of range. This closes the render-time index panic that the previous
> version of this rule described, per `[dec:pgorm:no-panic]` and
> `[dec:pgorm:invalid-states-unrepresentable]`.
>
> Because each reference was paired with its value rather than with an index,
> rendering emits ONE parameter per reference, numbered by the enclosing
> statement's own counter. A template therefore renumbers into whatever
> statement it lands in — a value read twice becomes two parameters holding
> it, and values read out of order come out in reference order with the
> values permuted to match — which is what lets a template compose with a
> statement that binds values of its own instead of requiring its own
> numbering to be free.
>
> A `SimpleExpr::AsEnum(type, expr)` at the top level is rewritten to a cast and
> renders as `CAST(expr AS type)`, with the type name written raw (unquoted) as
> a verbatim expression.

## Parameter injection

> [spec:pgorm:sem:sql.render.inject+3]
> `inject_parameters(sql, params)` (`prepare.rs`) converts a parameterized SQL
> string back into inline SQL and returns `Result`. It tokenizes the input; a
> `$` punctuation token immediately followed by an unquoted token that parses
> as a non-zero `usize` `N` references parameter `N`, counting from 1, and MAY
> be referenced any number of times. Every other token is reproduced verbatim,
> a `$` that is not followed by such an integer included: unlike
> `sql.render.custom-expr`, `$$` is NOT an escape here, because what arrives is
> real SQL, where `$$` opens a dollar-quoted body rather than standing in for a
> literal `$`. Because quoted tokens are opaque to the tokenizer — and the
> tokenizer reads comments as space and dollar-quoted bodies and `E'…'`
> strings as quoted (`sql.token.limits`) — a `$N` spelling inside a string
> literal, a quoted identifier, a comment or a dollar-quoted body is
> neither substituted nor counted, so a value can never escape such a
> context by ending it.
>
> The census is settled before anything is written, and MUST come out exact:
> the distinct `N` referenced MUST equal `1..=params.len()`. `$0`, a reference
> past the end, and a parameter the SQL never names each return
> `Error::Template` — respectively `ZeroIndex`, `IndexOutOfRange` and
> `UnreferencedValue` — where the previous version of this rule panicked on the
> vector index. Each reference is paired with its parameter up front, so the
> writing walk holds no indices at all. String values are re-escaped on
> injection (e.g. `B'C` becomes `E'B\'C'`).

## DDL

> [spec:pgorm:def:sql.render.ddl.types+8]
> `prepare_column_type` defines the Rust-side `ColumnType` → PostgreSQL type
> name mapping (all lowercase): Char(n) → `char(n)`/`bpchar`, the latter
> the type with no length limit, since a bare `char` is `char(1)`
> (`sql.ddl.column-types`); String →
> `varchar(n)`/`varchar`; Text → `text`; SmallInteger → `smallint`; Integer →
> `integer`; BigInteger → `bigint`; Float → `real`; Double →
> `double precision`; Decimal → `decimal(p, s)`/`decimal`; Timestamp →
> `timestamp`; TimestampWithTimeZone → `timestamp with time zone`; Time →
> `time`; Date → `date`; Interval → `interval`, `interval(p)` or
> `interval <fields>` per `IntervalSpec`, the fractional-seconds precision
> spelled by the second-bearing field itself (`interval HOUR TO SECOND(3)`);
> Bytea →
> `bytea`; Bit → `bit(n)`/`"bit"`, the latter the bit type with no length
> limit, since a bare `bit` is `bit(1)` (`sql.ddl.column-types`); VarBit →
> `varbit(n)`; Boolean → `bool`;
> Money → `money`; Json → `json`; JsonBinary → `jsonb`; Uuid →
> `uuid`; Array(t) → recursive element type plus `[]`; Vector →
> `vector(n)`/`vector`; Cidr → `cidr`; Inet → `inet`; MacAddr → `macaddr`;
> LTree → `ltree`; Range(t) → `t`'s range type, `int4range` through
> `tstzrange`; Multirange(t) → `t`'s multirange type, `int4multirange`
> through `tstzmultirange`; Named/Enum → the type name through `TypeName`'s part
> policy (`sql.types.type-name`), a lowercase name that is no restricted
> keyword bare and anything else quoted, never the identifier's raw
> string. The mapping is
> total — no variant is unsupported and none panics. An auto-increment column
> instead renders `smallserial`, `serial`, or `bigserial` by integer width;
> auto-increment on any other type renders that type's own spelling. Table
> DDL (`CREATE TABLE … ( … )`, `ALTER TABLE`
> add/modify/rename/drop column and add/drop foreign key, `DROP TABLE`,
> `TRUNCATE TABLE`, `ALTER TABLE … RENAME TO`), index DDL
> (`CREATE [UNIQUE ]INDEX … ON … [USING BTREE|GIN|HASH|<method>] (cols)` with
> optional ` NULLS NOT DISTINCT`, where a named access method renders under
> the same part policy), and foreign-key DDL (`FOREIGN KEY (…)
> REFERENCES … (…) [ON DELETE action] [ON UPDATE action]` with actions
> `RESTRICT`, `CASCADE`, `SET NULL`, `NO ACTION`, `SET DEFAULT`) are rendered
> by the same builder with identifiers quoted per `sql.render.ident-quoting`.

> [spec:pgorm:req:sql.render.ddl.enum-type+5]
> `CREATE TYPE` renders `CREATE TYPE name AS ENUM (…)` — the name via
> `TypeRef`'s quoted, dot-joined parts, so a schema-qualified type renders
> `"schema"."name"`. In cast and column-type position an
> enum type renders through `TypeName`'s part policy
> (`sql.types.type-name`): a lowercase part that is no restricted keyword
> bare, anything else a quoted identifier — so an enum name is a name
> there, never SQL, and
> `enumeration(Name::runtime("text, injected integer"))` yields a type
> PostgreSQL refuses rather than an extra column.
>
> A LABEL is not a name. Each is carried as a `String` and emitted through
> `push_param` — a `$N` parameter in the `build()` path and a quoted string
> literal inline in the `to_string()` path — so a label is never an
> identifier at either end: not in the type that holds it, and not in the
> SQL it renders to.
> `ALTER TYPE name` supports ` ADD VALUE v [BEFORE w | AFTER w]`,
> ` RENAME TO v`, and ` RENAME VALUE v TO w`; every label operand is likewise
> parameterized, but the `RENAME TO` target is a type name rather than a
> label and MUST render as a quoted identifier. `DROP TYPE [IF EXISTS ]name, … [CASCADE|RESTRICT]` renders
> type names via `TypeRef` with quoted, dot-joined parts. Callers executing
> these statements against PostgreSQL MUST use a rendering path that inlines
> the labels, since Postgres does not accept bind parameters in DDL.

> [spec:pgorm:sem:sql.render.ddl.extension+3]
> `CREATE EXTENSION [IF NOT EXISTS ]name [WITH SCHEMA s] [VERSION v]
> [CASCADE]` and `DROP EXTENSION [IF EXISTS ]name [CASCADE|RESTRICT]` MUST
> render the extension name and schema as quoted identifiers — both are
> `Name`s and go through `SqlName::prepare` like any other identifier, with
> no re-wrapping at the render site — and the version as a single-quoted string
> literal through `sql.render.string-escape` — the grammar takes a word or a
> string there, and a version like `v0.1.0` is not a word. The one string a
> DDL render still interpolates verbatim is `ColumnDef::extra`, which exists
> to carry SQL text the vocabulary cannot spell and is documented as caller
> responsibility by `sql.ddl.column-def`.
