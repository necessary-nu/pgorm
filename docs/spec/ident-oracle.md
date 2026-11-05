# Identifier render oracle

The [sqlmap campaign](sqlmap.md) cannot fire in identifier positions. A level-3
boundary cannot close a faithfully double-quoted identifier and still attach a
payload, so the cases whose input is a name have few or no scheduled
techniques. Yet identifiers are where ORM injection lives. Two holes found by
hand in September 2026 were both names:

- pipeline identifiers under stock prqlc, where `\"` broke out through the
  compiler's escaper (f48c6425);
- pgorm-pool savepoint names, which were interpolated into `SAVEPOINT {name}`
  (276dde57).

This chapter specifies the instrument for those positions. It is structural
rather than behavioural: it judges every position from the statement's parse
tree, so it does not depend on which positions a scanner's payload catalogue
can reach. The WBS node is `identifier-render-oracle`, under
`identifier-injection`. The oracle is `tests/identifier_oracle_tests.rs` and
the modules under `tests/identifier_oracle/`.

## The property

> [spec:pgorm:req:security.ident-oracle+16]
> Every public API that renders a caller-supplied name into SQL text MUST be
> registered with the oracle. A new identifier-bearing API is incomplete until
> it is registered, and an unregistered site is not covered by this rule. The
> registry is:
>
> - `tests/identifier_oracle/registry.rs` for pgorm-query;
> - `registry_orm.rs` for pgorm: the pipeline, the graph, `QuerySelect` and the
>   write helpers, relation and entity names, and schema generation;
> - `registry_disjoint.rs` for the disjoint schema and the one-of `CHECK`:
>   `create_disjoint` and the five changes to a disjoint entity's kinds
>   (`add_disjoint_kind`, `add_disjoint_kind_recreating`,
>   `remove_disjoint_kind`, `add_disjoint_kind_table`,
>   `remove_disjoint_kind_table`), over a shape a binding describes at run
>   time (`DisjointShape::new`), its base, kind table, key column, kind
>   column, kind type, the type's schema and a kind's value each a name, and
>   the value a change removes and the table it drops; and `create_one_of`
>   over an entity whose table and schema are named at run time. A change is
>   a list of statements, so each name is held at each statement it reaches,
>   one site apiece;
> - `live_capture.rs` for sites whose SQL exists only against a live server:
>   pgorm-pool's savepoint name, the migration ledger's custom table name,
>   and the cursor's qualifiers (`Cursor::new`'s table,
>   `set_secondary_order_by`, `add_secondary_order_by`, and a graph cursor's
>   alias tiebreak).
>
> Each entry gives:
>
> - the API;
> - the parse-tree positions its name must occupy;
> - the render policy that governs it;
> - a closure that builds the smallest statement putting a name in that
>   position. For a live-only site, an operation takes the closure's place,
>   and its statements are captured on their way to the server.
>
> A name the API derives from the caller's, such as `pk-{table}` or
> `idx-{table}-{column}`, is one of those positions. There, the hostile
> rendering must carry the same derivation of the hostile name, bounded to
> 63 bytes as pgorm bounds every name it composes. A derived name one of
> whose `-`-separated parts is itself a name pgorm composed and bounded — a
> disjoint base's foreign key `fk-{base}-{value}_{column}`, named for its
> generated key column — carries that part's derivation, bounded, inside its
> own. A disjoint kind's value is an enum label, a value, from which pgorm
> also composes identifiers (the generated key columns, their foreign key,
> the `CHECK` refusing a removed kind): standing alone it must come back as
> the value's bytes, which no identifier limit truncates, and within a name
> as that name's derivation.
>
> Each site is rendered with every name in a hostile corpus
> (`tests/identifier_oracle/corpus.rs`). The corpus contains at least:
>
> - `"`, `""`, `a"b`, `\"`, and the f48c6425 exfiltration payload
>   `x\" , (SELECT 1) AS "leak`;
> - `;`, `--`, `/*`, `*/`, `$1`, `$$`, `$tag$`, `'`, `E'`, and a backslash;
> - whitespace, including newlines and tabs;
> - a combining mark, a right-to-left override, full-width and typographic
>   quotes, and a zero-width joiner;
> - the keywords `select`, `from`, `user`, `not`, `integer`, `left`,
>   `coalesce`, `row`, `distinct` and `only`;
> - `*`, a dotted name, and the pipeline's own binding spelling `table_0`;
> - the empty name;
> - names of exactly 63 and 64 bytes, one of them splitting a two-byte
>   character at the boundary.
>
> Each rendered statement is parsed by libpg_query, the `pg_query` crate that
> the `sql!` macro, the paginator and the metrics fingerprint already use. The
> rendering MUST be exactly one statement. The name MUST come back exactly at
> the declared positions, in the node kind each position should produce, such
> as `RangeVar.relname`, `ColumnRef.fields[i]`, `ResTarget.name`,
> `TypeCast.type_name.names[i]` or `FuncCall.funcname[i]`. At every declared
> position the value MUST equal the name's bytes.
>
> Nothing else in the tree may differ. With every field that records a byte
> offset set aside — `location`, a statement's `stmt_location` and
> `stmt_len`, the offsets PostgreSQL 18's parse tree added beside them
> (`rexpr_list_start` and `rexpr_list_end` on an `IN` list, `list_start` and
> `list_end` on an array constructor, `arg_location`, `payload_location` and
> `conninfo_location`), and a JSON table path's `name_location` — the parse
> tree MUST equal the tree of the same statement rendered with a benign name,
> except at the declared positions. An offset moves with the length of every
> name before it, so it says nothing about structure; an offset field the
> oracle did not set aside would fail every name of a different length.
> The comparison is generic: both trees are the protobuf serialised to JSON,
> walked in parallel, and they must agree on every key, every array length
> and every other leaf. No per-site matcher is written, so a name that
> changes the statement's shape shows up as a difference, whatever the change
> was.
>
> A few outcomes other than a round trip are required instead of a failure,
> and each is tied to a render policy:
>
> - **The empty name.** At every identifier site, the empty name renders as
>   `""`. The grammar refuses that as a zero-length delimited identifier, so
>   no statement exists to run. An enum label is a value, not a name, and an
>   empty label round-trips.
> - **Identifier length.** PostgreSQL truncates an identifier longer than
>   63 bytes (`NAMEDATALEN - 1`) on a character boundary. The truncation
>   happens in the scanner itself, so libpg_query applies it too. The oracle
>   therefore expects a 64-byte name to read back as its first 63 bytes, or
>   as fewer bytes when the cut would split a character. Two long names that
>   share a 63-byte prefix name the same object on the server. That is a
>   collision and a correctness question, not an injection, and this rule
>   does not answer it. The migration ledger shows the consequence: it
>   creates the truncated table, then binds the full name as a `name` value
>   in a catalogue lookup, and the server refuses that value with `42622`
>   (*identifier too long*). The live leg expects that stop.
> - **Type spellings.** A type position under `TypeName`'s part policy
>   writes the grammar's own type spellings bare
>   (`[spec:pgorm:def:sql.types.type-name]`), and the name then resolves to
>   that type (`integer` → `int4`). The oracle accepts this only for the
>   nineteen keywords the rule lists, each named in the oracle, and only
>   when the parse differs from the benign one inside the `TypeName` node
>   that holds the name and nowhere else.
> - **Call forms.** A function position under the same policy writes
>   `coalesce`, `greatest`, `least` and `nullif` bare, as the expressions
>   the grammar builds for them. The oracle accepts this only for those four
>   keywords, each with the node it must become (`CoalesceExpr`,
>   `MinMaxExpr`, `MinMaxExpr`, `AExpr`), and only when the call node
>   holding the name became that node and nothing outside it differs.
> - **Refusals**, for the sites that refuse names
>   (`security.ident-oracle.nul`).
>
> Any other failure is either fixed or pinned. A pin lives in
> `tests/identifier_oracle/pins.rs` and names the plan node that fixes the
> defect. A test asserts on every run that each pinned site × name pair still
> fails, so the defect is reported until its fix lands and the pin cannot
> outlive it. No defect is pinned now. A fixed defect's pairs are held by
> the main property from then on:
>
> - The pipeline's leading-`$` and lone-`*` names
>   (`pipeline-bare-dollar-names`), which prqlc wrote bare as a parameter, a
>   dollar quote or the wildcard, are now refusals
>   (`security.ident-oracle.nul`).
> - `select_sources`'s read cast (`pipeline-read-cast-verbatim`), whose type
>   prqlc wrote verbatim, is now written through `TypeName`'s part policy,
>   and the site is held to that policy.
> - `TypeName`'s part policy wrote keywords bare
>   (`type-part-keyword-names`): `Func::named("not")` rendered `not(1)`, a
>   boolean NOT rather than a call, and reserved words at type, function,
>   schema and access-method positions were syntax errors. It now quotes
>   every keyword PostgreSQL restricts except the listed type spellings and
>   call forms.
>
> Because that policy turns on a keyword list, its sites are also held to
> every keyword the linked scanner knows, not only the corpus's ten. The
> list is read off libpg_query's token table and each word scanned back, so
> it is independent of the list pgorm-query embeds. At every site under the
> policy, each keyword MUST round-trip as a name or be one of the listed
> type spellings or call forms, accepted as above.
>
> The live leg checks the parser against the server, because the structural
> oracle trusts a single parser. For one representative site per parse-node
> kind, it creates real objects in a throwaway database using a handful of the
> nastiest names:
>
> - a quote-closing stacked `DROP TABLE sentinel`;
> - the f48c6425 payload;
> - a comment-and-placeholder name;
> - a string-closing stacked `DROP`;
> - a right-to-left override;
> - a mixed-case name;
> - a dollar-quote.
>
> It then runs the builder's own statement against each object. The kinds
> covered are:
>
> - relation, schema and column;
> - projection, table, subquery and CTE aliases, and the names a RETURNING
>   list gives a written row's old and new versions;
> - function and type names;
> - window names;
> - index, constraint, enum-type, label and comment targets;
> - collation names, in an expression and in a column definition;
> - the constraint an `ON CONFLICT ON CONSTRAINT` names as its arbiter;
> - sequence names, schema-qualified, created, altered, renamed and dropped,
>   and the schema, table and column an `OWNED BY` names;
> - composite type names, and the names and collations of their attributes;
> - range type names, schema-qualified, and the operator class, difference
>   function, collation and multirange name a range type is defined with,
>   and a value cast to the range type and one cast to its multirange by
>   name (`Expr::as_range`), inline and bound;
> - a MERGE's target table, schema-qualified and aliased, and the columns
>   that its `UPDATE SET` and `INSERT` lists name.
>
> For each object, three things MUST hold:
>
> - the catalogue holds it under exactly the name's bytes, compared against
>   a bound parameter;
> - the statement reaches it and returns the expected value or label;
> - the sentinel table survives.
>
> The live-only sites are held to the full corpus. tokio-postgres logs each
> statement it is handed, at debug level, before encoding it:
> `executing statement batch: {sql}` and `preparing query {name}: {sql}`. The
> oracle records those lines for its own thread and compares each run's
> statements with the benign run's, statement by statement.
>
> Extension names and schemas are held to the structural oracle only, because
> the test server can create no arbitrary extension.
>
> The oracle's sensitivity is demonstrated three times over:
>
> - When `SqlName::quoted` stopped doubling `"`, 936 pairs failed: every
>   quote-bearing corpus name at every non-literal pgorm-query site.
> - When the pipeline's screen stopped refusing `"`, every quote-bearing name
>   at every pipeline site was reported as rendered where it must be refused.
> - When pgorm-pool's savepoint quoting stopped doubling, the savepoint site
>   reported two statements where one belongs.
>
> Each failure message names the site, the name, and the differing node or
> the rendered SQL.
>
> The Python bindings reach names only through these Rust APIs, so this
> registry covers them too.

## Refusals and NUL

> [spec:pgorm:req:security.ident-oracle.nul+2]
> Where an API refuses a name, the oracle MUST see the refusal and never
> rendered SQL. The pipeline refuses in three ways, all at `into_sql` and
> never at construction:
>
> - `PipelineError::UnquotableIdentifier`, for any identifier carrying `"` or
>   NUL, beginning with `$`, or equal to `*`. This is pgorm's screen, which
>   runs before prqlc is called (`[spec:pgorm:req:pipeline.errors]`). The
>   corpus hits it with every quote-bearing name, `$1`, `$$`, `$tag$` and
>   `*`.
> - `PipelineError::ReservedAlias`, for an alias PRQL reserves. The corpus
>   hits this with `select`, `from` and `not`.
> - `PipelineError::Compile`, for an unqualified identifier that PRQL's `std`
>   binds, because a PRQL built-in used as a value is a name-resolution
>   failure. A relation or a bare column named `select`, `from` or `not`
>   therefore cannot be read through the pipeline, although a qualified
>   column of that name can.
>
> No pgorm-query or other pgorm API refuses a name on its content.
>
> PostgreSQL has no identifier that can contain NUL, and libpg_query cannot
> parse text containing one. Every registered site MUST declare what it does
> with a NUL-bearing name, and the oracle holds it to that declaration:
>
> | Site class | Behaviour |
> | --- | --- |
> | `SqlName::prepare` (quoted) and `TypeName::prepare_part` sites in pgorm-query and pgorm, and the migration ledger | The NUL byte is rendered into the statement text. postgres-protocol's `write_cstr` then refuses the text in both the extended-protocol `Parse` message and the simple-protocol `Query` message, so no byte reaches the server. The oracle proves this offline against the encoder for every such site. The live leg proves it end to end at `Table::create`: both protocols fail with *error encoding message to server* (*string contains embedded null*), the connection stays usable, and no table exists afterwards. |
> | Enum labels (`TypeCreateStatement::values`, `add_value`, `before`, `after`, `rename_value`) | The inline renderer escapes NUL to `\0` inside `E'…'`, so the text carries no NUL byte. The grammar refuses the escape with *invalid byte sequence for encoding "UTF8": 0x00*. |
> | Pipeline identifiers and aliases | Refused with `UnquotableIdentifier`; nothing is rendered. |
> | pgorm-pool's savepoint name and the cursor's qualifiers | Quoted like `SqlName::prepare`. The captured statement carries the NUL byte, and the encoder refuses it before anything is sent; for the savepoint this is `[spec:pgorm:req:conn.pool.savepoint-name]`. |
>
> This totality holds only for statement text. A NUL in a bound *value* is a
> different path and is not covered here.
