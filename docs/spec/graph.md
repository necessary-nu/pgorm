# The relational source graph

This chapter covers the N-ary relational read: `SelectGraph<E, S>`
(`src/query/graph.rs`), a root entity plus a typed list of joined sources,
one shape at every arity where the inherited surface had a bespoke type per
pair. Joins are computed from the same `RelationDef`s every other join is
(`[spec:pgorm:sem:query.build.join]`), decoding rides the unchanged absence
witness (`[spec:pgorm:req:exec.decode.absent]`), and the terminals land on the
ordinary `Selector` machinery — the graph adds a declaration layer, not a
second execution path. Rules are grouped under `[spec:pgorm:def:query.graph]`.

The two-entity surface this replaces — a bespoke builder, decoder, cursor
and consolidating terminal per pair, with its own `A_`/`B_` column-aliasing
scheme — has been deleted, and the rules that specified it retired with it
(`query.build.combine`, `exec.crud.consolidate`) or rewritten around the
graph (`query.build`, `query.build.modifiers`, `exec.crud`, `exec.stream`,
`exec.cursor`, `exec.paginator`, `entity.relation.linked`). Nothing in this
chapter is stated twice anywhere else.

## Declaration

> [spec:pgorm:def:query.graph+1]
> `SelectGraph<E: EntityTrait, S = (), R = Unsliced>` is a relational read
> declared as a root entity plus a typed tuple `S` of joined, decoded
> sources — *slots* — in join order, and a marker `R` recording whether
> `limit` or `offset` has cut the read to a window of rows
> (`[spec:pgorm:sem:query.graph.slice]`). `EntityTrait::graph()` constructs it, as `find()` constructs
> a `Select<E>`; `SelectGraph::<E>::new()` and `Default` are the same
> construction. Construction initialises the statement with
> `FROM E::default().table_ref()` and immediately projects `E`'s columns
> under the `s0_` prefix through the one writer
> (`[spec:pgorm:sem:query.graph.writer]`), so a graph's select list is
> non-empty from the moment the value exists and the empty-projection guard
> of `[spec:pgorm:sem:query.build.modifiers]` has nothing to catch. There
> is no conversion between `Select<E>` and `SelectGraph<E, S>` in either
> direction: a graph's projection is generated from its declaration, never
> inherited from a builder whose select list a caller may have edited.
>
> A slot is a zero-sized marker implementing the sealed `Slot` trait —
> `Req<F>` or `Opt<F>`, `F: EntityTrait` — whose associated items fix
> everything the slot means in one place: the entity (`type Entity`), the
> output shape (`type Out`: `F::Model` for `Req`, `Option<F::Model>` for
> `Opt`), the `JoinType` the shape implies (`const JOIN`), and the decode
> that reads it. Each edge method appends its slot by returning
> `SelectGraph<E, (…, NewSlot)>`, so the declared tuple, the emitted joins
> and the decoded row type are one fact stated once
> (`[spec:pgorm:sem:query.graph.slots]`).
>
> `via(rel: RelationDef)` joins without declaring a slot: `rel.to_tbl` is
> LEFT JOINed under `join_condition(rel)` exactly as
> `[spec:pgorm:sem:query.build.join]` computes it — `on_condition` and
> `condition_type` included — but the hop contributes nothing to the
> projection or the decode tuple and consumes no prefix index. It is how a
> junction table or a chain hop enters the graph: joined because the path
> runs through it, invisible because nobody asked to read it.
>
> The graph implements `QueryFilter`, `QueryOrder` and `QueryTrait` over its
> `SelectStatement`, so WHERE, ORDER BY, `build()`, `to_string()` and
> `apply_if` work as on every other builder. It deliberately does NOT
> implement `QuerySelect`: the writer owns the projection, and the fluent
> surface offers no way to edit it
> (`[spec:pgorm:sem:query.graph.slots]`). Its row range is set by its own
> `limit` and `offset`, which re-type it
> (`[spec:pgorm:sem:query.graph.slice]`). `QueryTrait::query()` remains the
> raw escape hatch it is everywhere — a caller mutating the statement
> through it stands outside every guarantee this family states, on the same
> terms as handing a hand-built statement to `Selector::from_select`.

> [spec:pgorm:sem:query.graph.slots+2]
> The slot kind is the join type is the decode shape. The three are one
> declaration, so they cannot disagree:
>
> - `join_maybe::<F>(rel)` appends `Opt<F>`: `rel.to_tbl` LEFT JOINed under
>   `join_condition(rel)`, decoded as `Option<F::Model>` through the absence
>   witness (`[spec:pgorm:sem:query.graph.decode]`).
> - `join_one::<F>(rel)` appends `Req<F>`: an INNER JOIN, decoded as a bare
>   `F::Model`. The type states that the join cannot miss; there is no
>   `Option` to unwrap for a row the join guarantees.
> - `join_maybe_as::<F>(rel, alias)` / `join_one_as::<F>(rel, alias)` are
>   the same two with the joined side re-bound to a caller-supplied alias
>   (`[spec:pgorm:req:query.graph.aliases]`).
> - `related_maybe::<F>()` where `E: Related<F>` folds the described path
>   in: when `Related::via()` is `Some`, that junction relation is `via()`ed
>   first, then `Related::to()` is `join_maybe`d — the whole described
>   path in one call, junction hop included.
> - `related_one::<F>()` where `E: Related<F>` folds the same path in as a
>   required source, appending `Req<F>`: when `Related::via()` is `Some`,
>   that junction relation is INNER JOINed under `join_condition` — not
>   `via()`ed, whose join is LEFT — and then `Related::to()` is
>   `join_one`d. Every hop of the path is INNER, so the SQL states the
>   semantics the slot declares: a root row with no related `F` is not in
>   the result, and each row that is decodes `F` as a bare `F::Model`.
>
> "LEFT-joined but decoded as required" and "INNER-joined but decoded
> optional" are unrepresentable: no method constructs either pairing, and
> the decode is the slot's own associated function. So is projection
> editing: the graph has no `select_only`, no `column` / `columns` /
> `column_as`, no `expr` / `exprs` / `expr_as`, no `select` — it does not
> implement `QuerySelect` at all, and the absence MUST be pinned by a
> compile_fail doctest (E0599). So is decode/declaration mismatch: the row
> type is `(E::Model, S1::Out, …, Sn::Out)`, computed from `S`, so ascribing
> a tuple the declaration does not produce is a type error (E0308) — also
> compile_fail-pinned — not a runtime decode surprise. The surface this
> replaces guarded the equivalent mistakes at run time or not at all — a
> projected expression with no name to take was carried through unaliased
> and simply never decoded; here they do not compile.
>
> `via()` joins LEFT so that a missing middle cannot erase root rows by
> itself; a `Req` slot joined through it re-tightens the chain, because the
> INNER join's ON references the middle's columns and NULLs do not satisfy
> it — end-to-end INNER semantics restored. `via` then `join_one` therefore
> reads "must match through the middle", and `via` then `join_maybe` "may be
> absent anywhere along the path".
>
> Slot arity is bounded by the generated impls: the edge methods and the
> `GraphRow` decode (`[spec:pgorm:sem:query.graph.decode]`) are
> macro-generated for slot tuples of arity 1 through 6, so a seventh
> decoded source has no receiver impl and fails to compile. The bound is a
> macro constant — raising it is a mechanical edit, not a design change —
> and `via()` hops do not count against it.

## Projection

> [spec:pgorm:sem:query.graph.writer+4]
> One writer projects every decoded source; nothing else writes the select
> list. At construction (for the root) and at each slot declaration, the
> writer appends, for every variant of the source entity's `Column` in
> iteration order, `col.select_as(Expr::col((qualifier, col)))` aliased as
> `s{i}_{col.as_str()}` — the same enum-cast discipline as `Select<E>`'s
> default list (`[spec:pgorm:sem:query.build.select-defaults]`), so an
> enum-typed column is cast to text on selection here exactly as there, and
> the alias is the plain SQL column name under the prefix, whatever the
> cast wrapped.
>
> The alias MUST respect PostgreSQL's 63-byte identifier bound: the server
> truncates a longer alias silently, so a projection minting 64 bytes and a
> decode requesting all of them would never meet. A prefixed name of 63
> bytes or more is spelled as the longest UTF-8-whole head within 47 bytes,
> padded with `_` back to exactly 47 when a multibyte boundary shortened
> it, followed by the 64-bit FNV-1a hash of the full name as 16 hex
> digits — deterministic, release-stable, and distinct for names sharing a
> head. A name strictly under the bound passes through as the plain
> concatenation. The two namespaces MUST NOT meet at any UTF-8 retreat
> width: every bounded spelling is exactly 63 bytes — the padding is what
> keeps that true for multibyte names — while a composition of exactly 63
> bytes is re-spelled though it would fit, so every plain spelling is
> strictly shorter than every bounded one and a column literally named
> like a bounded spelling composes to its own alias rather than silently
> reading another column's slot. The hash owns a bounded spelling's final
> 16 bytes unconditionally, so equal spellings decompose into equal heads
> and equal hashes; what remains is two distinct long names hashing
> alike — a 64-bit FNV collision this contract accepts and does not check
> for. The
> composition is one function (`result_column_name`) that the writers mint
> through and every prefixed read (`TryGetable::try_get`, the absence
> witness's `all_null`) composes through, so the generated alias and the
> decode witness stay one contract at every length.
>
> `i` counts decoded sources: the root is `0`, each slot takes the next
> index in declaration order, and `via()` hops take none — they are joined,
> never projected, so a junction's columns are transmitted zero times
> rather than decoded into a type the graph cannot name. The qualifier is
> the source's effective identifier — the bound alias for a slot declared
> through `join_maybe_as` / `join_one_as`, otherwise the bare table
> (`FromItem::qualifier()`, the same identifier `join_condition` constrains
> against) — so the projection and the ON clause cannot name one source two
> ways.
>
> The prefixes replace the several independently-maintained aliasing
> schemes the two-entity surface carried — one per builder — with a single
> implementation. The writer is shared with the pipeline's `select_sources`
> terminal (`[spec:pgorm:sem:pipeline.select-sources]`), which emits the
> same per-source blocks over named sources: the prefix scheme and the cast
> discipline are one code path, not a convention two surfaces repeat.
>
> The `s{i}_` names are generated but observable — they are the result
> set's column names, and `[spec:pgorm:sem:query.graph.decode]` reads
> exactly them. They are not, however, part of the fluent surface: no graph
> method takes or returns one, and a caller who needs to reference a
> source in a filter or an ordering qualifies the *source's* identifier
> (table or alias), not the projected alias.

## Decoding

> [spec:pgorm:sem:query.graph.decode+1]
> One row decodes as `(E::Model, S1::Out, …, Sn::Out)`; a slotless graph
> decodes as a bare `E::Model`, not a one-tuple. The selector is
> `GraphRow<E, S>`, a `SelectorTrait` implementor
> (`[spec:pgorm:def:exec.crud]`) at every declarable arity. The root
> decodes via `FromQueryResult::from_query_result` under `"s0_"`, then each
> slot in declaration order under its own prefix: `Req<F>` through
> `from_query_result`, `Opt<F>` through `from_query_result_optional` — the
> absence witness of `[spec:pgorm:req:exec.decode.absent]`, UNCHANGED, of
> which the graph is the N-ary consumer. Everything that rule states holds
> per slot: an unmatched LEFT JOIN reads as `Ok(None)` because every
> witness column under `s{i}_` is present in the result set and NULL; a
> decode failure of a present row propagates rather than being read as
> absence; and the all-NULL-matched-row ambiguity it names cannot arise
> from a graph, because the writer projects every column of the slot's
> entity, primary key included.
>
> Decode errors abort the row: the first failing source's error is the
> row's error, and later sources are not examined. Across rows the
> terminals' own semantics apply — `all` aborts at the first bad row
> (`[spec:pgorm:sem:exec.crud.select]`), `stream` yields the bad row as
> one `Err` item and continues (`[spec:pgorm:sem:exec.stream.decode]`).
> A `Req` slot never consults the witness: its INNER JOIN cannot produce
> the unmatched row, so a NULL arriving in a non-`Option` field of a `Req`
> slot is `[spec:pgorm:sem:exec.decode.null]`'s error, exactly as on a
> single-entity read.

## Terminals

> [spec:pgorm:sem:query.graph.terminals+2]
> The graph terminates by converting into `Selector<GraphRow<E, S>>`;
> everything past that conversion is machinery specified elsewhere, and the
> graph MUST NOT duplicate any of it. `all(db)` and `one_opt(db)` are
> `Selector::all` / `one_opt` (`[spec:pgorm:sem:exec.crud.select]`:
> `one_opt` narrows the row limit to one row and answers `Ok(None)` for
> zero rows). `stream(db)` is `Selector::stream`, yielding
> `PinBoxSendStream<'db, Result<Item, Error>>` with lazy per-item decode
> (`[spec:pgorm:sem:exec.stream.decode]`). These three exist on a sliced
> graph as on an unsliced one, and read the window
> `[spec:pgorm:sem:query.graph.slice]` states. Pagination and `count` reach
> an unsliced graph through `PaginatorTrait` over the same selector
> (`[spec:pgorm:def:exec.paginator]`); page boundaries fall between
> *rows*, not between root models, so a root with several matching slot
> rows spans pages exactly as the underlying SQL does — the grouped read
> (`[spec:pgorm:sem:query.graph.grouped]`) is deliberately not paginable,
> because a page boundary between rows can split one root's children across
> two pages, so no page is a complete entry.
>
> The terminal set is deliberately this small. There is no `one`: a graph
> row is a join product, and the `Error::RecordNotFound` reading of
> "exactly one" is a claim about the product, which multiplies per matched
> slot row — `one_opt` answers the first-row question, `all_grouped` the
> per-root one. There is no `into_model`, `into_tuple`, `into_values` or
> `into_partial_model`: the decode target is fixed by the declaration, and
> a caller-shaped projection is `Select<E>`'s job (or the pipeline's), not
> the graph's.

> [spec:pgorm:sem:query.graph.grouped+2]
> `all_grouped(db) -> Vec<(E::Model, Vec<F::Model>)>` exists on exactly one
> shape: an unsliced graph whose slot tuple is `(Opt<F>,)` — one optional
> slot beside the root, `via()` hops permitted (a junction-mediated has-many
> is this shape). On any other tuple, and on a graph `limit` or `offset` has
> sliced (`[spec:pgorm:sem:query.graph.slice]`), the method does not exist;
> asking for it is a compile error, so a grouped read over two slots — whose
> meaning this rule does not define — cannot be written, and neither can a
> grouped read of a window of rows, whose first or last entry would hold
> only part of its root's children.
>
> Caller ordering dominates. `all_grouped` appends `E`'s primary-key
> columns, qualified with `E`'s table, ascending, as trailing ORDER BY
> keys — after every caller-authored ordering, never before it. A caller
> who ordered by nothing gets pure primary-key order; a caller who ordered
> by anything gets that ordering with the key appended as a deterministic
> tiebreak only. The constructor-injected ORDER BY the two-entity surface
> emitted — which preceded, and therefore silently dominated, every
> ordering the caller wrote — is specified away and MUST NOT be
> reintroduced.
>
> Grouping is keyed, not adjacency-based: rows execute as ordinary
> `GraphRow` tuples and consolidate on the root's primary-key value (read
> from the decoded root model, at whatever arity the key has). Each
> distinct key yields exactly one output entry, positioned at its first
> occurrence in row order; children are pushed in row order, so the
> caller's ordering orders each bucket too; a row whose slot decoded
> `None` contributes the root with an empty `Vec`. An ordering that
> interleaves roots — one on the slot's columns, say — therefore merges a
> torn run into the entry at its first appearance rather than emitting the
> root twice; adjacency is a property of primary-key ordering, not a
> precondition of correctness.

> [spec:pgorm:sem:query.graph.cursor+4]
> `cursor_by<C: KeyOf<E>>(cols)` re-homes the joined keyset cursor
> onto an unsliced graph — the cursor's window is its row limit, so a
> sliced one has neither method (`[spec:pgorm:sem:query.graph.slice]`):
> order columns on the root's table, then the root's own
> primary key, then the primary-key columns of every decoded slot
> installed as unary secondary order entries — qualified with each slot's
> *effective* identifier (its alias when the slot was declared `_as`, per
> `[spec:pgorm:req:query.graph.aliases]`), in slot declaration order.
> `cursor_by_on::<Si>(cols)` is the generalization of the retired
> `cursor_by_other`: the slot is selected by its position at compile time,
> the order columns are typed `KeyOf` that slot's entity and
> qualified with its effective identifier, and the tiebreaks are that
> slot's own primary key first — completing the ordered source's
> continuation key, so two decoded rows sharing an order-column value
> under one root remain distinguishable and a mid-run resume cannot skip
> the sibling — then the root's primary key, then the remaining decoded
> slots' in declaration order. On both paths a tiebreak entry that
> restates one of the order columns is dropped rather than installed
> twice. Each tiebreak carries the casts its key column's boundary
> comparisons take, captured from the typed primary-key column as it is
> installed (`[spec:pgorm:sem:exec.cursor.keyset]`), so a key whose domain
> is over an enum — the root's, a joined slot's, or the ordered slot's own —
> is compared as the enum in a whole-keyset boundary, as an order column on
> it is. Both return `Cursor<GraphRow<E, S>, C::ValueType>`, so the
> boundary arity is typed by the order columns exactly as
> `[spec:pgorm:def:exec.cursor]` states.
>
> The machinery MUST NOT move: the keyset construction, the boundary
> disjuncts, `before` / `after` at order-column arity and `before_with` /
> `after_with` at whole-keyset arity, the direction resolution, the
> arity-mismatch error, and the NULL-tiebreak limitation are
> `[spec:pgorm:sem:exec.cursor.keyset]` and
> `[spec:pgorm:sem:exec.cursor.order]`, unchanged and not restated
> here. The NULL limitation is live on a graph: an unmatched `Opt` slot's
> primary key IS null, so a row whose tiebreak is null is reachable
> through the order-column boundary, not an extended one — resuming with
> `after_with` from inside a matched run and then paging past the
> unmatched roots is the proven pattern. What the graph adds is only
> where the columns come from: the declaration that fixed the joins fixes
> the tiebreak set, so a graph that gains a slot gains its tiebreak with
> no call site naming a column twice.
>
> One seam is inherited knowingly: `Cursor` itself implements
> `QuerySelect` (`[spec:pgorm:def:exec.cursor]`), so a graph's cursor
> can append to the generated projection even though the graph could not.
> The unrepresentability claims of `[spec:pgorm:sem:query.graph.slots]`
> are claims about `SelectGraph`, not about every value derived from it;
> a column appended through the cursor is outside the decode's witness
> set and is simply never read.

## Slicing

> [spec:pgorm:sem:query.graph.slice]
> `limit(n)` and `offset(n)` cut a graph's read to a window of rows. Each
> takes `impl Into<Option<u64>>`, as `QuerySelect`'s do, so `None` removes
> a limit or offset set before; sets its clause on the graph's one
> statement, replacing rather than accumulating, the count bound as a
> parameter; and returns the graph re-typed `SelectGraph<E, S, Sliced>`. The
> third parameter is `Unsliced` from construction, and the two markers are
> uninhabited types: nothing converts a sliced graph back, and the type
> records that the caller has taken charge of the row range whatever value
> was passed. A sliced graph keeps every builder method — the edges,
> `filter`, `order_by`, and `limit` and `offset` again — so a slice may be
> written anywhere in the chain.
>
> The window counts rows of the join product, not roots. `all`, `one_opt`
> and `stream` (`[spec:pgorm:sem:query.graph.terminals]`) answer exactly
> the rows the SQL returns: a root whose many slot matched three rows
> spends three of a limit, and an offset can start part-way through one
> root's rows. `one_opt` narrows the limit as every selector does
> (`[spec:pgorm:sem:exec.crud.select]`) — to one row, never wider, the
> offset kept — so `limit(0)` reads none and `offset(k)` answers the row
> after the first `k`.
>
> Three reads decide the row range themselves, and none of them exists on a
> sliced graph. Asking for one is a compile error (E0599), and each absence
> MUST be pinned by a compile_fail doctest:
>
> - `all_grouped` (`[spec:pgorm:sem:query.graph.grouped]`). A window of
>   rows can start or end part-way through a root's children, and the entry
>   holding the part is indistinguishable from a complete one.
> - `PaginatorTrait`'s `paginate` and `count`
>   (`[spec:pgorm:sem:query.graph.terminals]`). A page's LIMIT and OFFSET
>   replace the slice's (`[spec:pgorm:sem:exec.paginator.fetch]`) and the
>   count strips them (`[spec:pgorm:sem:exec.paginator.count]`), so both
>   would drop it without a word.
> - `cursor_by` and `cursor_by_on` (`[spec:pgorm:sem:query.graph.cursor]`).
>   The cursor's window, `first` or `last`, is its row limit and replaces
>   any other (`[spec:pgorm:sem:exec.cursor.window]`), and an OFFSET
>   would skip rows past the boundary of every execution, rows the keyset
>   places on the page.
>
> Nothing is lost by the refusals: an unsliced graph's page `k` at size `n`
> is the slice `offset(k * n).limit(n)`, and its cursor's `first(n)` is
> `limit(n)` under the cursor's order.
>
> `limit` is not given a per-root meaning on the grouped read. Its meaning
> would then depend on the terminal reached later — rows for `all`, roots
> for `all_grouped` — so the call site of `limit` could not say what it
> counts; and the grouped read keys on each root's first occurrence under
> the caller's ordering, which may order on slot columns and interleave
> roots, so cutting it by roots in SQL means ranking those first
> occurrences around the generated projection, an execution shape of its
> own beside the one selector the terminals share. Nor are row-level
> semantics on the grouped read documented and trusted: that returns a
> value that looks complete and is not, which
> `[dec:pgorm:invalid-states-unrepresentable]` rejects. The first `n` roots
> with all of their related rows is a root read and a load —
> `E::find().limit(n)`, then `load_many` or `load_many_via`
> (`[spec:pgorm:req:query.loader]`).
>
> `QueryTrait::query()` remains the raw escape hatch it is everywhere
> (`[spec:pgorm:def:query.graph]`): a limit written through it leaves the
> graph `Unsliced` in its type, outside this rule's guarantees.

## Aliasing

> [spec:pgorm:req:query.graph.aliases+2]
> The same table enters a graph twice under a caller-bound alias:
> `join_maybe_as::<F>(rel, alias)` / `join_one_as::<F>(rel, alias)` take
> the alias as `impl IntoName` — the `AliasName` token for a static name
> (`[spec:pgorm:sem:query.build.alias]`), `Name::runtime` for a computed one —
> re-bind `rel.to_tbl` to it, and that alias is then the slot's one
> identifier everywhere: the ON condition's right side, the projection
> qualifier, a cursor tiebreak's qualifier. Distinctness is not checked
> client-side: joining a table the graph already names, unaliased,
> renders SQL PostgreSQL refuses ("table name specified more than once"),
> so the mistake errors at execution rather than silently self-joining —
> the same ceiling the alias token carries everywhere.
>
> Another table whose bare name the graph already qualifies columns by is no
> such mistake — `archive.listing` joined beside a root `market.listing` —
> and an edge joining it unaliased MUST join it under an alias of the graph's:
> its schema-qualified name as one identifier, `"archive.listing"`, or, where
> the graph already has that, as it has an unqualified table's bare name, the
> same with `_2`, `_3` and on appended, each bounded to 63 bytes as result
> names are (`[spec:pgorm:sem:query.graph.writer]`). It is then the slot's or
> hop's one identifier, as a caller-bound alias is, and an edge whose
> relation leaves from that table by its own name leaves from it: the table's
> full name names one visit, so the graph re-points the edge, where a
> caller-bound alias of a table joined twice leaves it the relation's to
> point (`RelationDef::from_alias`, below). The tables the graph compares
> are its root's and its edges', and two are one table when their schema and
> bare name are. A call-site predicate or a typed column qualifies by the
> bare name, which means the first table of that name, so a condition on the
> aliased table's columns names them through the alias, which `join_maybe_as`
> binds where the caller would rather choose it.
>
> A hop is re-bound on the same terms. `via_as(rel, alias)` is `via(rel)`
> (`[spec:pgorm:def:query.graph]`) with `rel.to_tbl` re-bound to the
> alias: joined LEFT under `join_condition`, projected nowhere, consuming
> no prefix index. It is how a path that passes through one table twice is
> written — a self-referencing chain through a junction, or any `Linked`
> chain whose hops revisit a table — and the alias is the hop's one
> identifier: the ON condition's right side, and the identifier the next
> edge along the path starts from. The graph MUST NOT re-point that next
> edge itself. A graph's edges need not form one chain — each starts from
> whatever its relation's `from_tbl` names, the root or any earlier source —
> so which visit an edge leaves from is the relation's to state, through
> `RelationDef::from_alias(alias)`; an edge left unbound names the table's
> unaliased visit, or a table the query does not have, and the server
> answers as it does for any such reference. Because the hop is never
> projected, its alias is part of no result-column name: the slots reached
> through it are projected under their own `s{i}_` prefixes, bounded to
> PostgreSQL's 63-byte identifier limit as
> `[spec:pgorm:sem:query.graph.writer]` states, however many times the
> path revisits their table. Slots and hops, aliased or not, are one edge
> walker: the alias binding, the relation's condition and any call-site
> predicate are written once, and a slot is an edge whose joined side is
> then projected.
>
> Call-site ON predicates have a general mechanism and a sugar. The
> general mechanism is the relation's own: `rel.on_condition(f)` before
> handing the def over, the closure receiving the left and right
> identifiers (`[spec:pgorm:sem:query.build.join]`), where the right
> identifier IS the alias when the slot is aliased.
> `join_maybe_filtered(rel, f)` is the sugar: it takes the same closure
> shape and ANDs the produced condition into the join's ON clause *in
> addition to* whatever `on_condition` the relation already carries —
> where `RelationDef::on_condition` replaces
> (`[spec:pgorm:def:entity.relation.def]`), the sugar composes, so a
> call-site narrowing cannot silently drop an authored predicate. ON
> versus WHERE is the point of its existence: under a LEFT JOIN a
> predicate in ON narrows which rows *match* (unmatched roots survive,
> decoding `None`), while the same predicate through `filter` lands in
> WHERE, where an unmatched row's NULLs fail it and the join silently
> tightens to INNER. There is no `join_one_filtered`: under an INNER
> JOIN the two placements select the same rows, and `filter` already
> spells it.
>
> The standing hazard is the authored closure that ignores its
> parameters (`[spec:pgorm:def:entity.relation.def]`): a hardcoded
> table qualification renders verbatim, so under an `_as` slot the
> predicate constrains the un-aliased name — a table not in the query,
> or another join of it — and nothing errors client-side. The graph
> cannot detect this (the closure is opaque to it); the obligation to
> qualify from the parameters lives on the relation rule, and the alias
> methods are where violating it stops being harmless.
