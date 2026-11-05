# PRQL-shaped Pipeline API

`src/pipeline/` provides a typed, relation-to-relation query builder in the
shape of a [PRQL](https://prql-lang.org) pipeline, compiled to PostgreSQL SQL
through prqlc's intermediate representations. The pipeline is a permanent part
of the crate, compiled in every build. Rules are grouped under
`[spec:pgorm:req:pipeline]`.

## Architecture

> [spec:pgorm:def:pipeline.adapter+2]
> The pipeline lowers typed Rust construction directly into prqlc's PL AST —
> no PRQL text round-trip — and compiles it with `pl_to_rq` followed by
> `rq_to_sql` targeting `Dialect::Postgres`, producing a single SQL string
> with `$N` placeholders. Clause placement is the compiler's: a filter after
> an aggregation becomes `HAVING`, a filter after a window wraps the pipeline
> so far in a CTE, and stages nest as subqueries exactly where the relational
> semantics require.
>
> The PL AST is not a stable API, so the boundary is confined: every `prqlc`
> import in the root crate lives in the private `src/pipeline/adapter.rs`,
> the dependency is pinned to one revision of the necessary-nu fork (`prqlc =
> { git = ..., rev = ... }`, `default-features = false`), and no prqlc type
> appears in the public API. A compiler bump is
> absorbed by rewriting the adapter alone. prqlc is a plain dependency of the
> root crate by operator decision — the pipeline is part of the permanent
> story, and every build pays its compile, the same posture as `pg_query` —
> and it MUST NOT appear in any other workspace crate's dependencies, with
> exactly one exception: `pgorm-sql-macro` carries the same pinned revision for the
> `prql!` macro (`[spec:pgorm:def:macros.prql]`), which runs the compiler at
> build time over PRQL text. The two pins MUST stay identical, so the typed
> pipeline and the text macro emit through one compiler version and a bump
> rewrites both consumers in one motion.

## Surface

> [spec:pgorm:req:pipeline.surface+4]
> A `Pipeline` is constructed by `from(impl IntoSource)` or
> `from_schema(schema, table)` — the only entry points, so a sourceless
> pipeline is unrepresentable — and grown one whole transform at a time:
> `filter`, `derive`, `select`, `group(keys)` followed by `aggregate(aggs)`,
> `window(columns, over)`, `sort`, `take(i64)`,
> `take_range(RangeInclusive<i64>)` (1-based, inclusive),
> `join(JoinSide, relation, condition)` with an explicit condition only, the
> set operations `append` / `intersect` / `remove` over another relation,
> and `distinct` ([spec:pgorm:req:pipeline.compose]). Every relation
> position — the source, the join operand, a set-operation operand — takes
> `impl IntoSource`, and a whole `Pipeline` is an `IntoSource`, so pipelines
> compose with each other by the same spellings that name tables; any of
> them may be read under a name of its own, which is how a relation meets
> itself ([spec:pgorm:sem:pipeline.self-join]), and an entity read under one
> writes its columns as `Named::col(column)`, qualified by the name and typed
> by the entity.
>
> Each transform has two forms. The plain one takes its expressions by value
> — the whole query reads as one chained expression, with constants written
> as Rust literals. The `_with` one takes a closure and hands it the `Binder`,
> and exists only where a runtime value has to enter
> (`[spec:pgorm:req:pipeline.params]`). The `_with` closures of the
> list-taking transforms return a fixed-size `[Expr; N]`: an array is the one
> list shape whose element type can be written under the higher-ranked brand.
>
> Grouping is two steps that read as PRQL's do: `group(keys)` yields a
> `Grouped`, whose only method back to a `Pipeline` is `aggregate(aggs)` (or
> `aggregate_with`), so a grouping with nothing aggregated is unrepresentable
> rather than merely wrong. The pair lowers to the single PRQL
> `group {keys} (aggregate {aggs})` transform, so it stays one stage and a
> filter that follows still lands in `HAVING`.
>
> `window(columns, over)` names what to compute and what to compute it over.
> The `Over` is built by `by(keys)` (partition), `sort_by(keys)` (ordering)
> or `over()` (neither), chained (`by(..).sort_by(..)`) and narrowed by
> `rows(start, end)` or `range(start, end)` — integer bounds relative to the
> current row, `None` meaning unbounded on that side. With a partition the
> stage compiles to `PARTITION BY` under a `group`; without one the window
> spans the relation.
>
> Every scalar position takes `impl Into<Expr>` and every list position takes
> `impl ExprList`. `Into<Expr>` is implemented for `Expr` itself, for any
> `ColumnTrait` column (qualified by its own entity,
> `[spec:pgorm:sem:pipeline.qualify]`), for an `AliasName` token, and for
> the Rust literals `i32`, `i64`, `f64`, `bool` and `&str`. `ExprList` is
> implemented for a single expression, for `[T; N]` and `Vec<T>` of one
> convertible type, and for tuples of up to twelve mixed ones — the mixed
> case being the ordinary one for a projection, and a tuple being how PRQL
> spells a projection too.
>
> The operators live on the `ExprOps` trait, blanket-implemented for `Expr`,
> for `AliasName` and for every `ColumnTrait` column, so a column or a token
> is as much an expression as an `Expr` is: comparisons (`eq`, `ne`, `gt`,
> `gte`, `lt`, `lte`), `and` / `or` (and `Not` on `Expr`), arithmetic as
> methods (`add`, `sub`, `mul`, `div`, `rem`) and as the std `ops` traits on
> `Expr` (`+ - * / %`, `Neg`), `coalesce`, `is_null` / `is_not_null`
> (rendered `IS [NOT] NULL`), `in_array` over any `IntoIterator` (rendered
> `IN (...)`, members may be literals or bound placeholders), `cast` over the
> closed `CastType` enum (rendered `CAST(x AS t)`; closed because the type
> name reaches the SQL text verbatim), `compares_as(type)`, naming the type
> a column with no entity behind it compares as — the enum a domain is over,
> `money` — so that each comparison, `in_array` and `coalesce` casts the
> column and every literal or bound value beside it to that type, as an
> entity column of the type is cast
> (`[spec:pgorm:sem:entity.traits.column.enum-cast]`): a `money` column read
> by name compares with a `Decimal` under `compares_as(money)`, which the
> server would otherwise refuse (`42883`, *money > numeric*), `as_(name)`, and
> `desc()` / `asc()` for sort keys. Free functions supply
> `case(arms, otherwise)` with a
> mandatory fallback, `null()`, the aggregates (`sum` — which PRQL wraps as
> `COALESCE(SUM(x), 0)` —, `min`, `max`, `average`, `stddev`, `count`,
> `count_rows`, `count_distinct`) and the window functions (`row_number`,
> `rank` / `rank_dense` — which take the ranked column, per PRQL's own
> signatures —, `lag`, `lead`, `first`, `last`).
>
> `as_(name)` names a projected expression. The name is an `AliasName` token
> (`[spec:pgorm:def:sql.types]`) when anything refers back to it, and a
> bare `&'static str` when nothing does — `as_` takes `impl Into<AliasName>`
> and both spell the same thing. A token declared once by `let rn =
> alias("rn")` is also its own reference: it converts to an unqualified
> expression, so `filter(rn.lte(2))` refers to what `row_number().as_(rn)`
> introduced, and the name exists in the program exactly once. The token
> carries no evidence that it was ever attached: a reference to a name no
> stage introduced compiles, and the server answers for it
> (`[spec:pgorm:req:pipeline.errors]`).
>
> `ExprOps`'s comparison names are also `ColumnTrait`'s. Both traits can be
> in scope: the pipeline's methods take `self` by value and `ColumnTrait`'s
> take `&self`, so a column resolves to the pipeline's method where both are
> imported, and the ORM spelling is then `ColumnTrait::gt(&col, v)`. The
> pipeline is in no prelude, so this is per-module and opt-in, and the
> failure mode of getting it wrong is a type error at the ORM call site.
>
> Because every transform maps relation to relation, any `fn(Pipeline) ->
> Pipeline` is a composable query scope, and scopes may bind their own
> parameters; a finished pipeline composes with other pipelines whole, as a
> relation ([spec:pgorm:req:pipeline.compose]).
>
> Deliberately cut vocabulary (not quality): `loop`, s-strings and f-strings
> (raw SQL interpolation holes), the `text` / `date` / `math` std modules,
> range membership (`between`), `group`-scoped bodies other than `aggregate`
> and `window`, and iterator adapters as list arguments (an `ExprList` is an
> array, a `Vec`, a tuple or a single expression; a computed sequence is
> collected into a `Vec` first).

## Parameters

> [spec:pgorm:req:pipeline.params+4]
> Values reach the SQL by exactly two routes, and the spelling says which.
> A Rust literal — `1`, `1.5`, `true`, `"text"` — converts into an `Expr` and
> is inlined into the SQL text, exactly as a literal written in PRQL text
> would be; it is a constant of the query, and the pipeline renders it: a
> string literal is written as PostgreSQL text by the adapter and handed to
> the compiler as pass-through source, so nothing escapes it a second time.
> Every `'` in the value is doubled, and a value carrying a backslash is
> written as an `E''` literal with its backslashes doubled — the same trigger
> `pgorm-query` renders on (`[spec:pgorm:req:sql.render.string-escape]`) —
> so the literal denotes the value whichever way `standard_conforming_strings`
> is set. prqlc's own string rendering MUST NOT be used: it delegates to a
> dialect-agnostic formatter that reads a quote following a backslash, or one
> already doubled, as pre-escaped and emits it unchanged, and in an ordinary
> PostgreSQL literal — where a backslash means nothing — that lets `\'` close
> the string so the rest of the value parses as SQL. The invariant is that an
> inlined literal parses as exactly one statement and denotes exactly the
> value it was given. A runtime value goes through the `Binder`: the `_with` form of
> every expression-taking transform passes one to its closure, and
> `bind(value)` pushes the `pgorm_query::Value` and mints its `$N`
> placeholder in a single step, numbering in bind order across the whole
> pipeline, so a placeholder without its value cannot be constructed. The
> converse can arise without being written: prqlc carries `ExprKind::Param`
> through lowering verbatim — including inside `HAVING` and aggregate
> arguments — but its optimizer MAY prune an expression nothing reads (a
> derived column no later stage keeps), and the placeholder vanishes with it
> while the bound value remains. `into_sql` therefore takes a census of the
> emitted SQL: `pg_query::scan`, the PostgreSQL lexer pgorm already links,
> whose `PARAM` tokens cannot be confused with a `$N` inside a string
> literal. Values whose placeholders were optimized away are discarded and
> the surviving placeholders renumber contiguously `$1..$K`, the SQL text
> rewritten and the `Values` compacted in one pass; a repeated placeholder
> keeps its value exactly once. The invariant the terminals rely on —
> position `N` in the emitted SQL is position `N` in the returned `Values` —
> is thus enforced at the boundary rather than assumed of the optimizer. The
> lexer census is complete here where it has to be argued for in `prql!`
> ([spec:pgorm:sem:macros.prql.census]): the pipeline offers no raw-SQL hole
> (s-strings are cut from the surface, `[spec:pgorm:req:pipeline.surface]`),
> and the one text it hands the compiler to pass through untouched is a string
> literal it rendered itself, which the lexer reads as a single constant token
> — so every placeholder in the emitted text is a
> `Param` node the binder minted, and the census can only ever find a subset
> of the minted numbers — never a foreign `$N` with no value behind it. A
> census that cannot be trusted (unscannable text, an out-of-range number;
> neither reachable from the builder's own output) passes the statement
> through unchanged for the server to judge, keeping `into_sql` panic-free
> (`[spec:pgorm:req:pipeline.errors]`).
>
> The binder and the expressions it returns are branded with a
> higher-ranked, invariant closure lifetime; an expression containing a
> bound placeholder is thereby pinned to the pipeline that minted it, and
> carrying it into another pipeline is a compile error, not a runtime check.
> Param-free expressions (columns, tokens, literals, `col`) are
> brand-polymorphic and reusable anywhere. The by-value transforms take
> `Expr<'static>`, a brand a bound expression can never satisfy, so the plain
> form of a transform cannot smuggle a placeholder in either. The same
> reasoning binds `Over`: a window spec holds lowered nodes and therefore
> erases the brand it was built from, so its keys are `ExprList<'static>` and
> a placeholder cannot enter a partition or a window ordering — where it
> would mean nothing anyway.
>
> The one sanctioned crossing between pipelines is embedding a whole
> pipeline ([spec:pgorm:req:pipeline.compose]), which is sound where the
> expression crossing is not because the values move with their
> placeholders: the embedded pipeline's values append to the consumer's and
> its placeholders renumber by the same offset, so the alignment invariant —
> position `N` in the SQL is position `N` in the `Values` — is preserved
> across the merge.
>
> `take` and `take_range` accept integers by value, not expressions: PRQL
> refuses a parameterized `LIMIT`, so the signature mirrors the one form
> that compiles.

## Composition

> [spec:pgorm:req:pipeline.compose]
> A whole `Pipeline` is a relation: `IntoSource` is implemented for
> `Pipeline` by value, so another pipeline can read it (`from`), join it
> (`join`), or combine with it (`append` / `intersect` / `remove`). This is
> PRQL's own composition mechanism, `let` bindings: each embedded pipeline
> becomes a `let table_N = (...)` statement ahead of `main`, and prqlc
> lowers each referenced binding to a CTE — or inlines it where SQL allows,
> as it may for an `append` operand. `sort` and `take` inside an embedded
> pipeline stay inside its binding (rendered in the CTE), and PRQL's sticky
> `sort` carries outward to the reading pipeline, as it does in PRQL text.
>
> Embedding consumes the pipeline by value, and the consumption is the
> re-branding: the embedded pipeline's values append to the consumer's, and
> a rebase walk over its PL nodes shifts every `$N` placeholder up by the
> count the consumer had already bound — prqlc passes `Param` nodes through
> verbatim, so renumbering at PL-construction time is this module's job —
> and shifts its own binding references past the bindings the consumer
> already holds. Values and placeholders therefore cross together and stay
> aligned; an unattached `Expr` still cannot cross
> (`[spec:pgorm:req:pipeline.params]`). Nesting is unbounded: an embedded
> pipeline's own embeddings ride along, renumbered the same way.
>
> The binding names are minted by the module — `table_N`, by position — and
> are internal: no API takes or returns one, and callers refer to an
> embedded pipeline's columns by their own names instead. prqlc mints its
> wrapping CTEs in the same `table_N` namespace and steps around taken
> names, so the two sequences coexist; the namespace is reserved, and a real
> table sharing a minted name is shadowed by the binding. A name the
> embedded pipeline introduced with `as_` is referred to unqualified
> (the alias token); a name known to exactly one side of a join resolves to
> that side even unqualified; a name both sides export is an ambiguity prqlc
> refuses at `into_sql`, naming the candidates. In the join condition —
> where neither an embedded relation nor a mid-pipeline consumer has a name
> to qualify by — `this(column)` and `that(column)` qualify by role, PRQL's
> own `this` / `that`, and are scoped to that condition. `AliasName` tokens
> declared in two composed pipelines never collide: each lives in its own
> binding's scope, and the same name may be introduced in both.
>
> The set operations correspond by column position and emit the `ALL` forms:
> `append` is `UNION ALL`, `intersect` is `INTERSECT ALL`, `remove` is
> `EXCEPT ALL` (each row of the operand cancels one matching row, not all).
> `distinct` is PRQL's `group this (take 1)`, rendered `SELECT DISTINCT`,
> and folded by prqlc into `UNION DISTINCT` when it directly follows
> `append`. When both projections are visible to prqlc a column-count
> mismatch is refused at `into_sql`; relations with wildcard projections
> are the server's to check. After `intersect` or `remove` the combined
> relation is renamed, so later stages refer to its columns by bare name —
> an entity-qualified reference no longer resolves — while after `append`
> the left side's naming survives.
>
> `this` expands over a relation whose columns no projection has replaced
> to the wildcard that relation still carries, so the deduplication key
> holds one too. prqlc renders the deduplication as plain
> `SELECT DISTINCT *` only while that key equals the query's own frame, and
> falls through to `DISTINCT ON (<key>)` as soon as a later stage lands in
> the same query and the two differ. The wildcard is spellable in that
> position only qualified — `cake.*` is a whole-row reference PostgreSQL
> reads as a composite value, a bare `*` is not an expression its grammar
> has at all — and prqlc qualifies by table exactly when the query reads
> more than one. A deduplication that reads a single relation and keys on a
> wildcard MUST therefore be the last stage of its own query: the next
> stage hoists it into a binding, where the frame settles and plain
> `DISTINCT` stands. Nothing following means nothing owed, so a
> deduplication that ends the pipeline renders as the one query it always
> was. The hoist is what keeps the two readings apart — deduplicating whole
> rows and then projecting is a different relation from projecting and then
> deduplicating, and stage order is which one was asked for.
>
> A chain of set operations associates left: each one combines everything
> written before it with its own operand, so `append(b)` then `intersect(c)`
> means `(a UNION ALL b) INTERSECT ALL c`. Rendered flat that is not what
> PostgreSQL reads. `INTERSECT` binds tighter than `UNION` and `EXCEPT`,
> which share one level and associate left, so the server would evaluate
> `a UNION ALL (b INTERSECT ALL c)` — a different relation, and under the
> `ALL` forms every row of a self-combining source twice. An operation whose
> operator binds tighter than one already pending therefore takes its left
> operand as a binding: prqlc's rendering offers no parentheses, and a CTE
> boundary is the same bracket by another spelling. The obligation is read
> off the whole pending run rather than the stage that happens to be last,
> because a stage between two set operations need not break the chain —
> prqlc folds a projection the relation already carries back into the set
> operation's own arms. Where precedence cannot move the reading — `append`
> or `remove` after any operation, any operation after `intersect` — nothing
> is minted.

## Self-joins

> [spec:pgorm:sem:pipeline.self-join+1]
> A relation meets itself by being read twice under two names.
> `IntoSource::named(name)` is a provided method on the trait every relation
> position already takes, so `join(side, employee::Entity.named(manager),
> condition)` needs no second spelling of `join`, no aliased table type and
> no new expression form: the name is an `AliasName` token, and
> `col(manager, ID)` — the disambiguating reference that already existed —
> is how the far side's columns are written. The `Named` an entity's
> `named` returns keeps the entity's type, so `Named::col(column)` writes
> its column under the name and compared as the entity types it
> (`[spec:pgorm:sem:entity.traits.column.enum-cast]`), where `col` only
> qualifies. It lowers to PRQL's own operand
> alias (`join m=employee`) and renders as SQL's, `employee AS manager`,
> with no CTE: the employee-manager query is one join stage, selecting
> `Column::Name` beside `col(manager, NAME)`.
>
> Naming a relation replaces the name it had, exactly as SQL's `AS` does:
> after `employee::Entity.named(manager)` the reference `employee.name` no
> longer resolves, and prqlc refuses it by name
> ([spec:pgorm:req:pipeline.errors]). The ordinary shape therefore names
> only the second occurrence and leaves the reading pipeline
> entity-qualified. The name then reaches every stage after the join —
> `filter`, `sort`, `select` — which is what distinguishes it from `this` /
> `that`, scoped to the condition alone
> ([spec:pgorm:req:pipeline.compose]); and names chain, so a third and
> fourth occurrence of one table are the same motion.
>
> A whole `Pipeline` is a relation, so an embedded pipeline takes a name the
> same way: the binding still lowers to a CTE and the name attaches to the
> reference (`table_0 AS manager`). Renaming the far side's columns with
> `as_` inside the embedded pipeline, and reading them unqualified
> afterwards, remains a working spelling and is the one composition already
> gave; it costs a rename per column crossed, where a name costs one per
> relation, and it changes the embedded projection to do it. A name is
> screened against the reserved set like any `as_` name, and shares the
> `table_N` namespace the module mints its bindings in — reserved for that
> reason ([spec:pgorm:req:pipeline.compose]).

## Window frames

> [spec:pgorm:sem:pipeline.window-frame]
> A frame authored with `rows(start, end)` or `range(start, end)` reaches the
> emitted SQL for every window function, not only the ones the compiler
> happens to carry it for. The bounds are rows (or values) relative to the
> current row, and each maps to exactly one SQL bound: `Some(0)` is
> `CURRENT ROW`, a negative offset is `n PRECEDING`, a positive one is
> `n FOLLOWING`, and `None` is `UNBOUNDED PRECEDING` as a start and
> `UNBOUNDED FOLLOWING` as an end. `rows` renders `ROWS BETWEEN start AND
> end` and `range` renders `RANGE BETWEEN start AND end`; the two bounds are
> independent, so every direction — both preceding, spanning the current
> row, both following, and unbounded on either side — is expressible and is
> rendered as written.
>
> prqlc emits a frame only for the calls its own standard library annotates
> as frame-aware, which is the aggregates: `first`, `last`, `lag`, `lead`,
> `rank`, `rank_dense` and `row_number` are handed the frame and drop it.
> For `FIRST_VALUE` and `LAST_VALUE` that is a different query — the frame is
> what those functions read — so pgorm writes the whole `OVER (...)` clause
> for these seven itself, as a `derive` of expressions spelling their own
> partition keys, ordering keys and frame, rather than as prqlc's `window`
> transform. The ranking and offset functions are written the same way even
> though PostgreSQL ignores a frame on them: one rule covers the vocabulary,
> rather than the subset whose answer happens to move.
>
> A written clause states what prqlc's would have: the window's own
> partition and ordering keys, and — for an unpartitioned window that states
> no ordering of its own — the ordering the relation already carried, which
> is the ordering prqlc reads into such a window. A window mixing frame-aware
> and frame-blind columns therefore renders one frame twice, identically, on
> the two paths. Without an authored frame none of these seven is rewritten
> and the stage is prqlc's `window` exactly as before; the one call pgorm
> writes whether or not a frame was authored is `count`, for the separate
> reason given in `[spec:pgorm:sem:pipeline.count-argument]`.

## Counting an expression

> [spec:pgorm:sem:pipeline.count-argument]
> `count(expr)` MUST emit `COUNT(expr)` and `count_rows()` MUST emit
> `COUNT(*)`, in every position either can be written. The two are different
> questions wherever the counted expression is nullable — PostgreSQL's
> `COUNT(expr)` counts non-null values and `COUNT(*)` counts rows — and the
> surface separates them, so the translation MUST NOT collapse one into the
> other.
>
> PRQL's own `count` is the row count: the language documents it as counting
> the items of the relation it is given, including nulls, and prqlc's SQL
> standard library accordingly discards the argument and renders `COUNT(*)`.
> That is the right reading of PRQL and is left alone — `count_rows()` is
> PRQL's `count this` and stays prqlc's to render, frame and implicit window
> included. `count(expr)` is pgorm's separate spelling and is written out
> instead, by the same mechanism as
> `[spec:pgorm:sem:pipeline.window-frame]`: an expression spelling its own
> SQL, with the interpolated argument left for prqlc to resolve.
>
> A written call carries the clause its position gives it. Inside an
> `aggregate` it carries none, because the grouping says what the aggregate
> ranges over. Inside a `window` it carries that window's own `OVER (...)`,
> partition, ordering and frame included — which is why `count` is written
> even when no frame was authored, where the frame-blind seven are not. In
> any other stage it carries `OVER ()`, the implicit window over the whole
> relation that prqlc gives an aggregate used outside a grouping, so the
> reading of `derive` and `select` is the one they already had.
>
> `count_distinct(expr)` needs no rewrite: prqlc's standard library renders
> it with its column.

## Failure model

> [spec:pgorm:req:pipeline.errors+4]
> Pipeline construction is infallible; `into_sql()` is the fallible
> boundary, returning `PipelineError` and never panicking, per
> `[dec:pgorm:no-panic]`. Four variants: `UnquotableIdentifier(name)` — an
> identifier carries a `"` or a NUL byte, begins with `$`, or is `*`,
> refused rather than escaped (see below); `ReservedAlias(name)` — a name
> given to `as_` collides with the closed reserved set (the top-level
> bindings of prqlc 0.13's `std` module, its submodule names `math` /
> `text` / `date`, and the PRQL keywords), screened before compilation so
> the collision is named instead of surfacing as an opaque resolution
> failure; `Compile(diagnostics)`, carrying prqlc's own rendered
> diagnostics for everything its resolver rejects (a `std` name used as a
> value, ill-typed stages); and `ReshapedSources(stage)` — the
> `select_sources` terminal (`[spec:pgorm:sem:pipeline.select-sources]`)
> was asked to project entity models out of a pipeline whose sources'
> column namespaces a `select`, `group().aggregate()`, `intersect` or
> `remove` stage had already replaced, refused before prqlc compiles and
> naming the stage. `From<PipelineError> for Error` lifts into
> `Error::Query` so the terminals can fail through the ordinary channel.
>
> Identifier screening is the first thing `into_sql()` does, over every
> name the pipeline would hand prqlc — each alias and each segment of each
> identifier reference, so a schema, a table, a binding, a column and an
> alias are all covered, whichever of `col`, `from_schema`, `named_runtime`
> or `as_runtime` minted it. The refused set is closed and decided by pgorm
> alone, never by asking the compiler:
>
> - a `"` anywhere — the delimiter itself;
> - a NUL anywhere, which no PostgreSQL identifier can carry under any
>   quoting;
> - a leading `$`. prqlc writes a name bare whenever it matches
>   `[a-z_$][a-z0-9_$]*` and is not a keyword — a pattern older than PRQL's
>   own `$name` parameter token, which reaches the SQL by a different path —
>   and PostgreSQL's lexer reads a bare `$1` as a bound parameter and a bare
>   `$$` or `$tag$` as a dollar-quote delimiter, so two such names would
>   swallow everything between them, a filter included, as one string
>   constant. The refusal is also what keeps the parameter census sound
>   (`[spec:pgorm:req:pipeline.params]`): with no name able to lex as
>   `PARAM`, every `$N` in the emitted text is one the binder minted. A `$`
>   after the first character is not refused: `a$b` is one identifier to
>   PostgreSQL, bare or quoted;
> - the name `*`, which prqlc resolves as its wildcard before any quoting is
>   decided, so no spelling of it reaches the SQL as a column.
>
> Every other name renders: a space, a capital or a keyword is an ordinary
> name, which the compiler quotes where the grammar would not read it bare
> as that name. Length is not screened here — an identifier
> past the 63-byte limit is a separate question, about what it collides
> with after the server truncates it. The refusal costs names PostgreSQL
> itself would accept quoted (`"$1"`, `"*"`); a table or column so named is
> still reachable through the ORM's builders, which quote every name
> themselves (`[spec:pgorm:req:sql.render.ident-quoting]`).
>
> Refusing rather than escaping is the point, and it is a property of the
> dependency graph rather than of taste. pgorm hands identifiers to prqlc
> as plain text and prqlc quotes them, so what an embedded `"` becomes is
> decided by whichever prqlc the build resolved: the pinned fork doubles
> it, while registry `prqlc 0.13.14` escapes through sqlparser, which
> leaves a `"` preceded by a backslash alone — closing the identifier early
> and handing the remainder to the server as SQL. A leading `$` is left
> bare by both, and teaching the fork to quote it would still leave it bare
> under the registry crate. pgorm depends on the fork
> directly, but that is not a choice it can hold for a consumer: the
> consumer's own `patch` table can redirect the dependency, and a crates.io
> release, which cannot carry a git dependency, would have to name the
> registry crate. So which renderer stands between a caller's string and
> the emitted identifier is not pgorm's to decide. Pre-doubling in pgorm
> would not settle it either — the fork would double it a second time. So
> the screen runs in pgorm's own code and returns before
> `adapter::compile`, which is what makes the answer the same under either
> compiler.
>
> Compilation has no catalog, and this is the honest ceiling of the alias
> token: a token whose name no stage introduced still compiles, resolving as
> a column of the source relation, and whether that column exists is the
> server's question, answered at execution like any other raw SQL. Binding a
> token to the projection that declares it is deliberately not modelled in
> the type system — no brand ties a reference to its declaration — so the
> guarantee the token buys is that the name is written once, not that it was
> attached. Compile-time name checking covers only what prqlc can see: its
> own std namespace and pipeline-introduced names.

## Qualification

> [spec:pgorm:sem:pipeline.qualify+4]
> Column references are table-qualified by construction. An entity column
> already carries its entity, so `Into<Expr>` for `ColumnTrait` recovers the
> qualification from `entity_name()` rather than making the caller restate
> it, and a bare `order::Column::Total` mints the two-part identifier. The
> unqualified form — ambiguous the moment a join appears — is never
> constructed from a column. `col(table, column)` remains for the tables an
> entity does not describe and for disambiguation, taking an `SqlName` pair.
>
> `IntoSource` is any relation a pipeline can read — the source, the join
> operand, a set-operation operand — and there are four: an `EntityTrait`
> entity (which contributes its `table_name` and, when it has one, its
> `EntityName::schema_name`, so an entity source is schema-correct without a
> second spelling; the instance's, so an entity whose names are computed at run time
> reads the table of the instance handed to it, under the default's table name where
> the instance's differs, since its columns qualify by that name —
> `[spec:pgorm:req:entity.traits.entity-name]`), an `AliasName` token, a `Name`, and a whole `Pipeline`
> ([spec:pgorm:req:pipeline.compose]). `into_source` yields the opaque
> `Source` carrier, whose contents only the pipeline module can construct,
> so the set of relation shapes is closed.
> `from_schema(schema, table)` schema-qualifies a table no entity describes.
> A schema rides the identifier path through prqlc's `default_db` namespace
> and renders as `schema.table` — probed against the grammar oracle, with no
> known limitation. Identifier quoting is prqlc's: names needing quotes
> (spaces, case, reserved words such as a table named `order`) render
> double-quoted. An `AliasName` token used as an expression is the one
> bare-identifier form, reserved for names the pipeline itself introduced.

## Termination

> [spec:pgorm:sem:pipeline.terminal+1]
> `into_sql()` yields `(String, Values)`, the raw-SQL currency of
> `exec.crud.selector-entry`: `into_model::<M>()` and `into_tuple::<T>()`
> stage exactly those entry points (`SelectorRaw::from_statement`,
> `SelectorRaw::into_tuple`), and the convenience terminals `all::<M>(db)`,
> `one::<M>(db)` and `one_opt::<M>(db)` go straight to `FromQueryResult`
> models on any `ConnectionTrait`. `one` / `one_opt` append a `take 1`
> stage before compiling — the pipeline is still a pipeline at that point,
> so the limit belongs to it, in contrast to `SelectorRaw::one`, which
> executes its text as written.
>
> The model terminals (`into_model`, `all`, `one`, `one_opt`) compile the
> pipeline as `M` reads it. Where `M::expected_columns()` reports a column
> carrying a read cast (`exec.verify`) — an entity model's enum column, read
> as its `text`, a `money` column read as `numeric`, a
> `#[pgorm(select_as = "…")]` override — the wildcard a pipeline otherwise
> selects would deliver the column's own type, which the field does not
> decode, so the pipeline ends in one more projection: every column `M`
> reports, in its order, each referred to as `this.<name>` and a cast column
> written `CAST(this.<name> AS type)` under its own name, the type through
> the `TypeName` part policy `select_sources` writes its casts by. `this`
> keeps a column named like one of PRQL's own (`date`, `rank`) a column.
> The aliases end the pipeline, so they pass the identifier screen and not
> the reserved-alias one: no later stage exists to resolve one to the PRQL
> name it shares. A type that reports no columns, or none carrying a cast,
> is compiled as written — `into_sql` and the model terminals then emit the
> same text. The projection names `M`'s columns bare, so it needs a relation
> in which each name means one column; after a join of relations sharing one
> prqlc refuses it as `Compile` ("Ambiguous name"), which is the case
> `select_sources` exists for.

> [spec:pgorm:sem:pipeline.select-sources+4]
> `select_sources(sources)` is the model-decode terminal: where
> `into_model::<M>` asks the caller for a row type whose projection the
> caller must have arranged, `select_sources` takes the relations
> themselves and arranges the projection. It is the pipeline's complement
> to the source graph — row-shaped reads that need what the graph excludes
> (right and full joins, aggregates beside models in a later `derive`,
> arbitrary composed relations) are written here; reads the graph's slot
> typing can carry belong there (`[spec:pgorm:def:query.graph]`).
>
> The argument is a single source or a tuple of up to six — each an
> `IntoSource` some stage of the pipeline read (the `from` source or a
> join operand), restated: an entity for a relation read bare, the same
> `named(..)` spelling for a relation read under a name
> (`[spec:pgorm:sem:pipeline.self-join]`), so two occurrences of one table
> are told apart exactly as the join told them apart. Before compilation
> the terminal appends one final projection stage through the same writer
> as the graph's (`[spec:pgorm:sem:query.graph.writer]`): for the i-th
> listed source (zero-based), every column of its entity in iteration
> order, projected under the read cast derived from the column's own
> `select_as` answer — the enum default's `text` / `text[]` and a
> `#[pgorm(select_as = "…")]` override's cast alike, probed off the
> method's returned shape so this writer and the `SelectStatement` one
> cannot disagree about a column's read cast. The cast's type is written by
> pgorm, as `CAST(column AS type)` around the column prqlc renders, through
> the `TypeName` part policy both writers share
> (`[spec:pgorm:def:sql.types.type-name]`); it is never handed to prqlc,
> whose `std.as` writes the identifier in its type slot verbatim. A name
> in that position is therefore quoted exactly as the `SelectStatement`
> writer quotes it, and only a `TypeName::raw` type expression — program
> text — is written as it stands. Each column is qualified by the source's
> name — the `named` token, or the entity's own qualification
> (`[spec:pgorm:sem:pipeline.qualify]`) — and aliased `s{i}_{col}` under
> the writer's 63-byte-bounded spelling. An explicitly
> aliased projection is what dissolves prqlc's `_expr_N` renaming: two
> sources sharing a column name land under different prefixes by
> construction, so the compiler never has to invent names the decode
> cannot predict.
>
> Every listed source decodes through the absence witness
> (`[spec:pgorm:req:exec.decode.absent]`) as `Option<Model>` — every
> position, the first included. The pipeline's joins carry no missability
> in their types, and under a right or full join the *left* side is the
> absent one, so the terminal claims nothing a join could falsify: the
> row type is `(Option<E1::Model>, …, Option<En::Model>)`, and a source
> listed alone still decodes as `Option<E1::Model>`. Callers who can
> prove a side present unwrap it; the graph is the surface whose types
> state it.
>
> `select_sources` yields a `SelectedSources`, a terminal-only value on
> the pattern of `Grouped`: its only methods are the terminals —
> `into_sql()`, `all(db)`, `one(db)`, `one_opt(db)`, with the `take 1`
> semantics of `[spec:pgorm:sem:pipeline.terminal]` — so no transform can
> follow the selection and reshaping *after* it is unrepresentable.
> Reshaping *before* it is refused at `into_sql`: a pipeline that already
> passed through `select`, `group(..).aggregate(..)`, `intersect` or
> `remove` no longer carries the sources' own column namespaces (a
> projection replaced them, an aggregation collapsed them, a set-op
> rename dissolved the entity qualification,
> `[spec:pgorm:req:pipeline.compose]`), and the terminal MUST fail with a
> typed `PipelineError` variant naming the offending stage — before prqlc
> compiles, so the caller reads "select_sources after <stage>" rather
> than an opaque unresolved-name diagnostic. `filter`, `derive`, `sort`,
> `take` / `take_range`, `join`, `window` and `append` (whose left-side
> naming survives) leave every source addressable and compose freely ahead
> of the terminal; construction itself stays infallible per
> `[spec:pgorm:req:pipeline.errors]`.
>
> `distinct` composes ahead of the terminal except where it settles the
> pipeline into a binding, which is the one thing a per-source projection
> cannot survive: behind a binding the relation answers under that
> binding's name alone, and this projection is written in the sources' own.
> The refusal is the same gate naming `distinct`, and it MUST fire whether
> the settle has already happened or is still owed — a deduplication over a
> single relation whose columns are still a wildcard owes one
> (`[spec:pgorm:req:pipeline.compose]`), and this projection would be the
> stage that discharges it. Where the deduplication reads more than one
> relation nothing is owed and it composes as the other stages do. Listing
> the sources before the deduplication, or decoding with `into_model`,
> reaches the same rows.
>
> The catalog-less ceiling stands: a listed source the pipeline never
> read compiles up to prqlc, which refuses the unresolvable columns as
> `Compile(diagnostics)` — the terminal checks stage shape, not
> membership. And as everywhere the witness reads, a matched row whose
> every projected column is NULL is indistinguishable from an unmatched
> one only when the projection omits the primary key, which this
> projection never does.
