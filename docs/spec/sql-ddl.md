# DDL Statement Builders

This section specifies the schema (DDL) statement builders in `pgorm-query`:
table create/alter/drop/rename/truncate (`pgorm-query/src/table/`), index and
foreign key statements (`pgorm-query/src/index/`,
`pgorm-query/src/foreign_key/`), `CREATE TYPE ... AS ENUM` / `AS (...)` / `AS RANGE` and extension
statements (`pgorm-query/src/extension.rs`), sequence statements
(`pgorm-query/src/sequence/`), schema statements (`pgorm-query/src/schema.rs`),
domain statements (`pgorm-query/src/domain/`),
view and materialized view statements
(`pgorm-query/src/view/`) and `CREATE TABLE AS`
(`pgorm-query/src/table/create_as.rs`), `COMMENT ON` statements
(`pgorm-query/src/comment.rs`), and the rendering contract
implemented by the Postgres `QueryBuilder`
(`pgorm-query/src/backend/query_builder.rs`). All rules describe current
behaviour, including the leftovers from the multi-backend ancestry.

> [spec:pgorm:req:sql.ddl+12]
> The DDL surface MUST be reachable through the entry-point helpers: `Table`
> (`create`/`create_as`/`alter`/`drop`/`rename`/`rename_column`/`truncate`), `Index`
> (`create`/`drop`),
> `ForeignKey` (`create`), `Type` (`create`/`alter`/`drop`),
> `Domain` (`create`/`alter`/`drop`),
> `Schema` (`create`/`drop`/`rename`),
> `Sequence` (`create`/`alter`/`drop`/`rename`),
> `View` (`create`/`create_recursive`/`drop`),
> `MaterializedView` (`create`/`refresh`/`drop`),
> `Extension` (`create`/`drop`) and `Comment` (`on_table`/`on_column`).
>
> The render surface a statement exposes MUST follow from what it binds, not
> from which family it belongs to. Every statement has the value-inlined
> rendering — its `Display`, reached as `to_string()` — and that rendering MUST
> carry, on the `impl` itself rather than only in module prose, the note that
> it inlines rather than binds, pointing at `build` where the statement has
> one.
>
> `TypeCreateStatement` and `TypeAlterStatement` push their enum labels through
> `push_param` (`[spec:pgorm:req:sql.render.ddl.enum-type]`), so they have a
> second rendering and MUST expose it as `build() -> (String, Values)` beside
> the `build_collect(sink)` they already had — the capability named, rather
> than reachable only by constructing a `SqlWriterValues` and calling the
> undocumented `into_parts`. PostgreSQL accepts no bind parameter in DDL, so
> that pair is for inspection and the inlined rendering is what a caller
> executes; both the `build` doc and the `Display` doc MUST say so.
>
> Every other DDL statement — schema, domain, table, index, foreign-key,
> sequence, view, materialized view, `CREATE TABLE AS`, comment, extension,
> `DROP TYPE` — has
> no placeholder-emitting entry point at all: its renderer is
> `pub(crate)` and its only public route is `Display` over a `String` sink, so
> a value it carries (a column `DEFAULT`, a `CHECK` expression, a value in
> the query a view wraps) is always
> inlined as an escaped literal and there is nothing left to bind. These expose
> `to_string()` and nothing else, delegating to the corresponding `prepare_*`
> method on the single Postgres `QueryBuilder`. The claim they "carry no bind
> parameters" is the one to avoid restating: they carry values, and inline
> them.
>
> The `SchemaStatementBuilder` trait and its `build`/`build_any`/`to_string`
> triplication are gone, as are the `build_ref`/`build_collect_ref` inherent
> methods on type and extension statements. No
> rendering method takes a `QueryBuilder` argument — the builder is a stateless
> unit struct, so passing one carried no information.
>
> Every identifier a
> DDL statement renders — table, column and type names, and index, constraint
> and foreign-key names alike — MUST go through `SqlName::prepare` and so render
> double-quoted (quote character `"`, embedded quotes doubled); no identifier
> is interpolated raw between quote characters. Index and constraint names are
> held as `Name` and accepted as `IntoName`, so a runtime name minted by
> `Name::runtime` escapes like any other.
>
> No enum stands over a family's statements to dispatch among them: each
> statement is built by its entry-point helper and rendered as itself, so
> `Schema` names only a database schema (`sql.ddl.schema`). The
> `TableStatement`, `IndexStatement`, `ForeignKeyStatement`, `SchemaStatement`
> and `QueryStatement` wrappers, which nothing built or read, are gone and
> MUST NOT return (`sql.surface`).

## Tables

> [spec:pgorm:req:sql.ddl.create-table+16]
> `TableCreateStatement` composes a table name, ordered `ColumnDef`s (`col()`,
> which stamps the table ref onto each column), the table's keys — one primary
> key (`primary_key()`) and any number of unique keys (`unique()`), each a
> `TableKey` — foreign keys (`foreign_key()`), `CHECK` constraints (`check()`),
> an `if_not_exists` flag, a `comment` and a trailing `extra` string.
>
> Every embedder MUST take what it embeds by value. `primary_key()` and
> `unique()` take any `IntoTableKey` of their kind, and `foreign_key()` is
> bounded `Into<ForeignKeyCreateStatement>`; each consumes its argument, as
> `col()`'s `IntoColumnDef` consumes a column: a caller reusing the value
> writes `.to_owned()` or `.clone()` and sees the copy in the source, rather
> than the embedder cloning or draining a `&mut` behind their back. Reuse after
> the call therefore has one outcome across all of them
> (`[spec:pgorm:req:sql.ast]`).
>
> Rendering MUST emit `CREATE TABLE [IF NOT EXISTS ]<table> ( ... )` with the
> body in this fixed order: column definitions, then the primary key, then the
> unique keys in the order they were added, then foreign-key clauses (in
> `Mode::Creation`, i.e. without `ALTER TABLE`/`ADD`), then the `CHECK`
> constraints in the order they were added, all comma-separated. A key renders as `[CONSTRAINT "name"
> ]PRIMARY KEY (cols[, "period" WITHOUT OVERLAPS])` or `[CONSTRAINT "name"
> ]UNIQUE [NULLS NOT DISTINCT ](cols[, "period" WITHOUT OVERLAPS])`, then `[
> INCLUDE (names)][ <deferrability>]`, the columns and the included names each
> quoted like every other identifier.
> `get_primary_key()` reads the primary key back, and `get_unique_keys()` the
> unique keys in the order they were added.
>
> A table's keys follow PostgreSQL's own model, where a primary key is a
> unique key over columns that are never null, the one the table calls *the*
> key, and either is a tuple of one or more columns. So there is one key type,
> `TableKey<K>`, whose kind `K` is `Primary` or `Unique`: a non-empty ordered
> column list started at its first column (`TableKey::new(c)`) and extended by
> `col(c)` or, for a computed list, `cols(iter)`, with `name`, `include` and
> `deferrability` (`[spec:pgorm:req:sql.ddl.deferrability]`). `IntoTableKey<K>`
> converts any `IntoKeyColumns` — one column (any `IntoName`) or a tuple of
> one to twelve columns, the widest a primary key's value is
> (`entity.traits.primary-key`) — or a key already built. `IntoKeyColumns`
> hands back the first column apart from the rest and has no impl for an
> empty tuple, a slice or a `Vec`, which could be empty: `PRIMARY KEY ()` is a
> syntax error, and the key is non-empty by construction. `NULLS NOT DISTINCT` is the unique kind's alone,
> `nulls_not_distinct()` on `TableKey<Unique>` read back by
> `is_nulls_not_distinct()`: PostgreSQL refuses it on a primary key (`42601`),
> so the primary form has no method to carry it, which a `compile_fail` doctest
> holds. The table-constraint grammar takes a key of plain column names,
> `INCLUDE`, `NULLS NOT DISTINCT` on a unique key, deferrability, and, from
> PostgreSQL 18, `WITHOUT OVERLAPS` on the last column, and nothing else: the
> live suite shows a non-unique kind, a `DESC`/`ASC` entry,
> an operator class, a `COLLATE`, an expression entry, a `WHERE` predicate and
> a `USING` access method each refused as a syntax error, and `PRIMARY KEY
> NULLS NOT DISTINCT` likewise. `TableKey` has no method for any of them, and
> an `IndexCreateStatement`, which carries all of them for the standalone
> `CREATE INDEX`, does not convert into one. A key names no table — it is
> written inside the one it constrains — and has no rendering of its own, no
> `Display` and no build path, because PostgreSQL spells it only inside a
> table statement; `ALTER TABLE` adds the same type
> (`[spec:pgorm:req:sql.ddl.alter-table]`). Its readers are `get_name()`,
> `get_columns()`, `get_include()`, `get_deferrability()` and
> `get_without_overlaps()`. Its columns,
> not the key, are what an `ON CONFLICT` target naming the key takes:
> `OnConflict::columns` accepts the same `IntoKeyColumns`, and no `TableKey`,
> whose name and options an inference target cannot carry
> (`[spec:pgorm:req:sql.ast.on-conflict]`).
>
> Either kind may be PostgreSQL 18's temporal key: `without_overlaps(c)` ends
> it `"c" WITHOUT OVERLAPS`, a later call replacing the column, and
> `get_without_overlaps()` reads it back. The key's other columns are
> compared for equality and that one, a range or multirange, for overlap, so
> two rows may share the other columns only while their periods do not
> overlap. The live suite holds what it means: an overlapping period for one
> key is refused as the exclusion the key is enforced as (`23P01`, by a GiST
> index), periods that only touch, `[2020-01-01,2020-02-01)` and
> `[2020-02-01,2020-03-01)`, are both admitted, and an empty period is refused
> (`23514`), where the plain key over the same columns, the control, admits
> the overlap. PostgreSQL takes `WITHOUT OVERLAPS` on the last column alone
> and refuses a key it is the only column of (both `42601`), so the column is
> not a flag on an entry of the list, which could stand anywhere in it, but a
> slot of its own written after every other column whatever order the calls
> come in, and the key's first column, which its constructor takes, is never
> it (`[dec:pgorm:invalid-states-unrepresentable]`). `get_columns()` is the
> equality columns alone. Everything else a key carries combines with it, and
> the live suite shows each meaning what it means on a plain key: a name,
> `INCLUDE`, deferrability — a deferred temporal key holds an overlap between
> statements and refuses it at `COMMIT` (`23P01`) — and the unique kind's
> `NULLS NOT DISTINCT`, under which two rows with no value and one period
> conflict where the plain temporal key admits both. `NOT ENFORCED` it
> refuses as on any key (`0A000`), and a key has none to set
> (`[spec:pgorm:req:sql.ddl.enforcement]`). Two requirements are the
> server's knowledge, which a key naming columns cannot see: the column must
> be a range or multirange (`42804`), and a scalar column beside it needs a
> GiST operator class, which the `btree_gist` extension gives
> (`[spec:pgorm:req:sql.ddl.extension]`; `42704` without it). A temporal key
> is no unique index, which is what an `ON CONFLICT` arbiter by inference
> needs (`[spec:pgorm:req:sql.ast.on-conflict]`).
>
> A key names each of its columns once, its `WITHOUT OVERLAPS` column among
> them: PostgreSQL refuses a key naming a column twice (`42701`, *column "a"
> appears twice in primary key constraint*), as a primary or a unique key, in
> `CREATE TABLE` and after `ALTER TABLE`'s `ADD`, and the live suite holds
> each. A repeat MUST be written as given and left to the server, and MUST
> NOT be dropped: a key with the repeat removed is a key other than the one
> written, which the caller would never see declared. The repeat is not ruled
> out by type, because two columns of one type are two values no type tells
> apart (`[dec:pgorm:invalid-states-unrepresentable]`), and it is not refused
> by the key either: `col`, `cols`, the tuple conversions and every embedder
> are infallible, and making each return a `Result` would put an error path
> on every key a program declares for the one mistake the statement's
> execution reports by name, which is the fallible boundary a DDL statement
> has (`[dec:pgorm:no-panic]`). Codegen, which reads keys at a fallible
> boundary of its own, refuses one by name (`codegen.ddl.unsupported`,
> `codegen.entity.transform`). The included columns are no part of the key:
> `INCLUDE` may name a key column or one column twice, and the server
> accepts both. An `ON CONFLICT` target is not a key and takes a repeat too
> (`[spec:pgorm:req:sql.ast.on-conflict]`).
>
> A table has one primary key, and PostgreSQL refuses a second in every
> spelling (`42P16`, *multiple primary keys for table are not allowed*): on two
> columns, on a column beside a table constraint, as two table constraints, or
> twice on one column. So the statement holds its key in one slot, and a later
> `primary_key()` call replaces the key an earlier one declared, name and
> options with it, as a second `raw_suffix()` replaces the first; the unique
> keys are a list `unique()` appends to. A column has no key clause of its own
> — `ColumnDef` has no `primary_key()` or `unique_key()` and `ColumnSpec` no
> key arm (`[spec:pgorm:req:sql.ddl.column-def]`) — so a key is declared on
> the table and only there, and keys always render after the columns, one
> form per concept. Two primary keys therefore have no representation
> (`[dec:pgorm:invalid-states-unrepresentable]`); the live suite holds a table
> given two keys by the builder as created with the second, where SQL
> declaring two is refused. The column spellings, the `index()` embedder and
> its `IndexConstraint` are gone with the second key they made representable
> and MUST NOT return. A one-column key the column spelling wrote as
> `"id" integer PRIMARY KEY` is `PRIMARY KEY ("id")` after the columns, which
> PostgreSQL creates as the same constraint under the same derived name
> (`<table>_pkey`), as a column's `UNIQUE` and a one-column `UNIQUE (…)` are
> both `<table>_<column>_key`.
>
> A `CHECK` constraint is a `Check`: the expression its constructor takes
> (`Check::new(expr)`), with by-value `name(n)`, `enforcement(e)`
> (`[spec:pgorm:req:sql.ddl.enforcement]`) and `no_inherit()`, and the readers
> `get_expr()`, `get_name()`, `get_enforcement()` and `is_no_inherit()`. It
> renders `[CONSTRAINT "name" ]CHECK (<expr>)[ NO INHERIT][ ENFORCED | NOT
> ENFORCED]` wherever it stands — here, on a
> column (`[spec:pgorm:req:sql.ddl.column-def]`) and after `ALTER TABLE`'s
> `ADD` (`[spec:pgorm:req:sql.ddl.alter-table]`) — and has no
> deferrability, which PostgreSQL never gives a `CHECK`
> (`[spec:pgorm:req:sql.ddl.deferrability]`). Each position takes any
> `IntoCheck`: an expression, which is the unnamed, enforced constraint, or a
> `Check` already built, as a key position takes a column or a `TableKey`.
> `NO INHERIT` keeps the constraint from a table created `INHERITS` this
> one, and the live suite holds it: a parent's `NO INHERIT` `CHECK`, on a
> column, at table level or added after, is not copied to the child, which
> admits the row the parent refuses, where a plain one is copied. It is
> written before the enforcement, because a column's `CHECK` takes it there
> alone (`NOT ENFORCED NO INHERIT` on a column is `42601`). A partitioned
> table's constraints always reach its partitions, so the server refuses it
> there (`42P16`), in `CREATE TABLE` and after `ADD` alike. A `CHECK` is
> never `NOT VALID` here: that is an `ALTER TABLE`'s
> (`[spec:pgorm:req:sql.ddl.alter-table]`).
>
> After the closing parenthesis only the `extra` string follows (e.g.
> `USING columnar`). There are no table options: the MySQL-era `TableOpt`
> (`Engine`, `Collate`, `CharacterSet`) and its
> `engine`/`collate`/`character_set` builders rendered `ENGINE=`,
> `COLLATE=` and `DEFAULT CHARSET=` trailers Postgres rejects, and the
> uninhabited `TablePartition` had no renderer at all; both are gone with the
> statement's `options` and `partitions` fields, and MUST NOT return. The
> table-level
> `comment` is stored and exposed via `get_comment()` but is not rendered
> here: on Postgres a table comment is a statement of its own, built through
> `[spec:pgorm:req:sql.ddl.comment]`.
>
> The table name is structural rather than checked: `Table::create(table)` and
> `TableCreateStatement::new(table)` take any `IntoTableName` and there is no
> `table()` setter, so the `CREATE TABLE  ( ... )` PostgreSQL rejects at the
> parenthesis has no constructor
> (`[dec:pgorm:invalid-states-unrepresentable]`). `take()` moves every
> accumulated part out and copies only that name, for the same reason
> (`[spec:pgorm:req:sql.ast]`).
>
> A statement with no columns renders `CREATE TABLE <table> (  )`, and that is
> deliberately left buildable: PostgreSQL accepts a table with no columns, so
> the empty body is odd rather than invalid and gets documented rather than
> forbidden. Unlike an empty alter or a missing target, there is no unparseable
> render here for a type to prevent.

> [spec:pgorm:req:sql.ddl.column-def+13]
> `ColumnDef` holds a name, an optional `ColumnType`, an optional `Collation`
> and an ordered list of `ColumnSpec`s (`Null`, `NotNull { name, no_inherit }`, `Default(SimpleExpr)`, `AutoIncrement`,
> `Check(Check)`, `Generated { expr, kind }`,
> `Identity(IdentityGeneration, Option<SequenceOptions>)`, `RawSuffix(&'static str)`,
> `Comment(String)`),
> populated by the fluent typed setters
> (`integer()`, `string_len(n)`, `timestamp_with_time_zone()`, `interval()`,
> `vector()`, `enumeration()`, `array(elem)`, `cidr()`, `ltree()`, ...,
> `not_null()`, `not_null_named(name)`, `not_null_no_inherit()`, `default(v)`,
> `check(c)`, `generated(expr, kind)`, `identity()`,
> `identity_by_default()`, `identity_with(generation, options)`, `raw_suffix(s)`, etc.).
>
> A column carries no key. A primary or unique key is the table's, a tuple of
> one or more columns declared on the table
> (`[spec:pgorm:req:sql.ddl.create-table]`), so `ColumnSpec` has no
> `UniqueKey` or `PrimaryKey` arm and `ColumnDef` no `unique_key()`,
> `primary_key()` or their deferrability variants: a key on a column was the
> spelling that let one table hold two primary keys, and adding a key to a
> table that exists is `ALTER TABLE`'s `add_primary_key` / `add_unique`
> (`[spec:pgorm:req:sql.ddl.alter-table]`), not a column's.
>
> A column MUST render as the quoted name, one space, the type spelling, then
> ` COLLATE ` and the collation's quoted name when it has one
> (`[spec:pgorm:req:sql.render.collate]`), then each spec in insertion order: `NULL`,
> `[CONSTRAINT "name" ]NOT NULL[ NO INHERIT]`, `DEFAULT <expr>`,
> `[CONSTRAINT "name" ]CHECK (<expr>)[ ENFORCED | NOT ENFORCED]` — `check(c)`
> takes the `IntoCheck` a table's `check()` does
> (`[spec:pgorm:req:sql.ddl.create-table]`) —
> `GENERATED ALWAYS AS (<expr>) { STORED | VIRTUAL }`,
> `GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY[ (<options>)]`, and
> `RawSuffix` verbatim.
>
> A column definition's `DEFAULT` sits where PostgreSQL's grammar reads a
> `b_expr`, which has no logical operator, `IS`, `BETWEEN`, `IN` or `LIKE`
> (`42601` at the operator), in `CREATE TABLE` and `ADD COLUMN` alike. So a
> `Unary`, `Binary`, `Raw`, `Template` or `LikePattern` default MUST render
> parenthesised, `DEFAULT (TRUE AND FALSE)`, and any other expression bare,
> as `CREATE DOMAIN`'s default does (`sql.ddl.domain`). `ALTER COLUMN ... SET
> DEFAULT` reads the full expression grammar and renders its expression bare.
>
> A column's `NOT NULL` is a catalog constraint in PostgreSQL 18 — a
> `pg_constraint` row of kind `n`, recorded under the name the column gives it
> or else under `<table>_<column>_not_null` — and a column has exactly one: the
> server makes one constraint of repeated `NOT NULL` clauses and refuses two
> that name it differently (`XX000` between a column's own clauses, `42601`
> between a column's and a table-level one) or disagree on `NO INHERIT`
> (`42601`). So `NotNull` carries the constraint's `name` and `no_inherit`,
> and a column holds at most one `NotNull` spec: `not_null()`,
> `not_null_named(name)` and `not_null_no_inherit()` each set that one spec,
> the first of them putting it in the list where it is called and the rest
> changing it where it stands. `.not_null().default(1).not_null_named(n)`
> therefore renders `CONSTRAINT "n" NOT NULL DEFAULT 1`, a later name replaces
> an earlier, and `not_null()` on a column that already refuses nulls changes
> nothing, so neither refusal has a render to come from
> (`[dec:pgorm:invalid-states-unrepresentable]`). The plain `not_null()` stays
> the spelling of the unnamed constraint, which is the common case, and the
> named form always carries a name, so each state has one spelling. `NO
> INHERIT` keeps the constraint from a table created `INHERITS` this one; a
> partitioned table's constraint always reaches its partitions, so the server
> refuses `NO INHERIT` there (`0A000`) and accepts it on a partition. `NOT
> VALID` has no column-level spelling — the grammar refuses it in `CREATE
> TABLE` and `ADD COLUMN` alike (`42601`) — and so no setter: a not-null
> constraint is added `NOT VALID` by `ALTER TABLE`'s `add_not_null`
> (`[spec:pgorm:req:sql.ddl.alter-table]`). A `NOT NULL` is never deferrable
> (`42601` on a column, `0A000` at table level) nor `[NOT] ENFORCED` (likewise).
> The constraint is dropped by a modified column's `Null` spec, `ALTER COLUMN
> "c" DROP NOT NULL`, whatever its name, and by `DROP CONSTRAINT "name"`;
> either is refused for a primary-key column and for a child's inherited copy
> (`42P16`).
>
> `CREATE TABLE` has no table-level `NOT NULL "c"`, PostgreSQL 18's other
> spelling of the same constraint. It creates the constraint the column's
> clause creates, under the same derived name, and the column's clause carries
> the name and `NO INHERIT` it would. Its one addition, `NOT VALID`, describes
> nothing there: the live suite's server creates a table-level `NOT NULL c NOT
> VALID` valid (`convalidated`), a new table having no rows to leave
> unchecked. So the concept has one form in `CREATE TABLE`, as a key does
> (`[spec:pgorm:req:sql.ddl.create-table]`).
>
> A generated column is one of PostgreSQL's two kinds, `GeneratedKind::Stored`
> (computed when the row is written, kept on disk) or `GeneratedKind::Virtual`
> (computed when the row is read, PostgreSQL 18's addition), and
> `GeneratedKind::keyword` gives the `STORED` / `VIRTUAL` word. The kind MUST
> always be written: PostgreSQL 17 refuses a generated column that names
> neither keyword, and 18 reads one that names neither as `VIRTUAL`, so a
> render that left it to the server would change kind with the release. That
> is why `generated(expr, kind)` takes the kind as an argument with no
> default — a one-argument `generated(expr)` does not compile — and why the
> kind is a closed pair rather than a `stored: bool`, whose `false` read
> backwards and once stood for a render no release before 18 parsed
> (`[dec:pgorm:invalid-states-unrepresentable]`); that flag MUST NOT return.
> pgorm targets PostgreSQL 18, the release the oracle parses with and the
> live server runs (`[spec:pgorm:req:sql.render.oracle]`), so a virtual
> column is no longer a render the builder has to keep from 17.
>
> What the server refuses around a generated column is the server's
> knowledge, not the column's, and the live suite holds each refusal by
> SQLSTATE: for either kind, an expression that is not immutable (`42P17`),
> one that reads another generated column (`42P17`) or a whole row
> (`42P17`), one holding a subquery (`0A000`), a `DEFAULT` or an identity
> beside it (`42601`), and a generated column as a partition key (`42P17`);
> for a virtual column, an index on it or on an expression reading it, a
> primary key, a unique key and a foreign key (each `0A000`), a user-defined
> or domain type — an enum or composite included (`0A000`) — and a
> user-defined function in its expression (`0A000`). A stored column takes
> an index and a key, and a `NOT NULL` or `CHECK` on either kind is enforced
> (`23502`, `23514`). A row writes neither kind: an insert or update that
> supplies a value other than `DEFAULT` is refused (`428C9`).
>
> `AutoIncrement` produces no keyword; instead it replaces the type spelling
> with the serial family, which `ColumnType::serial_spelling` defines over the
> integer trio alone — `SmallInteger`→`smallserial`, `Integer`→`serial`,
> `BigInteger`→`bigserial`. Every other type has no serial form, so the spec
> contributes nothing and the column renders its declared type; the
> substitution MUST NOT panic, and MUST NOT invent a serial spelling for a
> type Postgres has none for. `Comment` specs are skipped entirely — a column
> comment is a statement of its own (`[spec:pgorm:req:sql.ddl.comment]`).
> `IntoColumnDef` accepts both `ColumnDef` and `&mut ColumnDef` (via
> `take()`), enabling the builder-by-reference doctest style; `take()` clones
> the name rather than swapping in a placeholder identifier, so no empty
> identifier exists to leak into a rendered column.
>
> `Identity` is the standard-SQL form PostgreSQL has recommended since 10, and
> the one `auto_increment` is the legacy spelling of: `identity()` sets
> `IdentityGeneration::Always`, `identity_by_default()` sets `ByDefault`, and
> `IdentityGeneration::keyword` gives the `ALWAYS` / `BY DEFAULT` words —
> the same two `information_schema.columns.identity_generation` reports. Which
> form a column uses MUST be that closed pair rather than a flag: there is no
> third state and no both-at-once, so the choice is typed rather than checked
> (`[dec:pgorm:invalid-states-unrepresentable]`). `auto_increment` stays,
> because it is what the entity derive's `auto_increment` attribute means and
> retiring it is a separate migration; its documentation MUST point at
> `identity` as the recommended form. In `ALTER TABLE`, `Identity` is the one
> spec that spells an action rather than a clause: on `ADD COLUMN` it renders as
> the column clause above, on `MODIFY` it renders
> `ALTER COLUMN "c" ADD GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY[ (<options>)]`.
>
> An identity column owns a sequence, and that sequence's options are the ones
> a standalone sequence takes (`[spec:pgorm:req:sql.ddl.sequence]`), from the
> same `SequenceOption` vocabulary: `identity_with(generation, options)` takes
> the form and any `Into<SequenceOptions>`, and the column writes ` (<options>)`
> after `AS IDENTITY`, in `CREATE TABLE`, `ADD COLUMN` and `MODIFY` alike.
> `identity()` and `identity_by_default()` carry `None` and write no
> parentheses, the only spelling of no options there is: `AS IDENTITY ()` is a
> syntax error, and `SequenceOptions` has no empty value to render one from.
> The two clauses of a standalone sequence that are not `SequenceOption`s stay
> out of it: the sequence counts in the column's own type, and PostgreSQL
> refuses an `AS` there as conflicting with it (`42601`); the column owns it,
> and an `OWNED BY` there is accepted and changes nothing. `SEQUENCE NAME`,
> which would name the sequence rather than let PostgreSQL derive
> `<table>_<column>_seq`, is not built, and neither is `ALTER COLUMN ... SET
> <option>`: `pg_get_serial_sequence` reports the derived name, and
> `Sequence::alter` under it reaches every option the column form does — only
> `OWNED BY` is refused there, as moving an identity sequence's ownership
> (`0A000`). These options used to have no spelling and rode `raw_suffix`.
>
> One boundary is deliberate: identity's exclusivity with `Default`,
> `Generated` and `AutoIncrement` is documented rather than typed. Those are four *separate explicit calls* a
> caller has to write, not a flag the API offers — nothing here presents the
> invalid combination as a choice — and collapsing them into one slot would move
> `DEFAULT` out of the insertion-ordered spec list this same rule fixes, which
> is a contract every interleaved render depends on. The combinations therefore
> render, the grammar accepts them, and the server refuses each by name; the
> live suite holds those refusals so the boundary is checked rather than
> asserted. Revisiting it means reshaping the spec list, and that reshaping is
> the work, not this clause.
>
> The collation is a slot of the column and not a spec, set by
> `collate(c)` — any `IntoCollation`, as `Expr::collate` takes
> (`[spec:pgorm:req:sql.ast.expr.collate]`) — and read back by
> `get_collation()`. PostgreSQL's grammar lets `COLLATE` stand anywhere among
> a column's clauses but refuses a second one ("multiple COLLATE clauses not
> allowed"), so a spec pushed in insertion order could build that refusal and
> a slot cannot: a second call replaces the first. Where it is written
> therefore follows the type rather than the call order. It is distinct from
> a `COLLATE` inside a `DEFAULT` expression, which is the expression's and is
> self-parenthesised so the grammar cannot read it as the column's.
>
> `RawSuffix` is deliberately verbatim and is the one DDL render that
> interpolates a caller string unquoted. It exists as the escape hatch for
> column SQL the `ColumnType`/`ColumnSpec` vocabulary cannot spell, so quoting
> or escaping it would defeat its only purpose: whatever a caller puts there is
> emitted as written, and the caller owns its trustworthiness — including
> whether it parses at all. Its `&'static str` bound is the contract that only
> program text reaches that position. Anything expressible through the typed
> setters MUST use them instead.

> [spec:pgorm:req:sql.ddl.column-types+8]
> `prepare_column_type` defines the `ColumnType` → Postgres type-name
> contract, and it is total: every variant has exactly one Postgres spelling
> and none can fail. It MUST spell: `Char(Some(n))`→`char(n)`,
> `Char(None)`→`bpchar`; `String(N(n))`→`varchar(n)`,
> `String(Max|None)`→`varchar`; `Text`→`text`; `SmallInteger`→`smallint`;
> `Integer`→`integer`; `BigInteger`→`bigint`; `Float`→`real`;
> `Double`→`double precision`; `Decimal(Some((p,s)))`→`decimal(p, s)`,
> `Decimal(None)`→`decimal`; `Timestamp`→`timestamp`;
> `TimestampWithTimeZone`→`timestamp with time zone`; `Time`→`time`;
> `Date`→`date`; `Interval(Any(None))`→`interval`,
> `Interval(Any(Some(p)))`→`interval(p)`,
> `Interval(Fields(f))`→`interval FIELDS`, where a second-bearing field
> spells its own precision (`SECOND(3)`, `HOUR TO SECOND(3)`);
> `Bytea`→`bytea`; `Bit(Some(n))`→`bit(n)`, `Bit(None)`→`"bit"`;
> `VarBit(n)`→`varbit(n)`; `Boolean`→`bool`; `Money`→`money`; `Json`→`json`;
> `JsonBinary`→`jsonb`; `Uuid`→`uuid`; `Array(t)`→ recursive element spelling
> plus `[]`; `Vector(Some(n))`→`vector(n)`, `Vector(None)`→`vector`;
> `Named(type_name)`, and `Enum`, `CreatedRange`, `CreatedMultirange` and
> `Domain` by their `name` and `schema`→ the type name through
> `TypeName`'s part policy (`sql.types.type-name`), a lowercase name that
> is no restricted keyword bare and anything else quoted; `Cidr`→`cidr`;
> `Inet`→`inet`; `MacAddr`→`macaddr`;
> `LTree`→`ltree`; `Range(t)`→ the range type `t` names (`int4range`,
> `int8range`, `numrange`, `daterange`, `tsrange`, `tstzrange`) and
> `Multirange(t)`→ its multirange (`int4multirange` through
> `tstzmultirange`).
>
> `Char(None)` is the blank-padded type with no length limit, which
> `format_type` prints as `bpchar`, and it is written by that name: PostgreSQL
> reads a bare `char` or `character` as `char(1)` (live on 18.6), so spelling
> `Char(None)` as `char` created a column that refused every value longer than
> one character, and a `bpchar` column read back through codegen
> (`codegen.ddl.types`) came back as `char(1)`. `ColumnDef::char()` is SQL's
> bare `char` and builds `Char(Some(1))`, as `ValueType::column_type()` does
> for a Rust `char` (`sql.value.value-type`).
>
> `Bit(None)` is the bit type with no length limit (type modifier `-1`), whose
> columns hold a bit string of any length, each value keeping its own. It has
> no keyword spelling: PostgreSQL reads a bare `bit` as `bit(1)` (live on
> 18.6), so it is written as the quoted type name `"bit"`, which is how
> `format_type` prints it and what codegen reads back as `Bit(None)`
> (`codegen.ddl.types`). `ColumnDef::bit(length)` builds `Bit(length)`, SQL's
> bare `bit` being `bit(Some(1))`, and the Python and JavaScript `DataType`
> `"bit"` with no length is `bit(1)`. `VarBit(n)` is `varbit(n)`.

> [spec:pgorm:req:sql.ddl.alter-table+13]
> `TableAlterStatement` names one table and collects `TableAlterOption`s:
> `AddColumn` (with an `if_not_exists` flag), `ModifyColumn`, `DropColumn`,
> `AddForeignKey`, `DropConstraint`, `AddPrimaryKey`, `AddUnique`,
> `SetExpression`, `DropExpression` (with an `if_exists` flag), `AddNotNull`,
> `AddCheck`, `ValidateConstraint` and `AlterConstraint`. Both the
> table and a first option are structural rather than checked:
> `Table::alter(table)` yields a `PendingTableAlter`, which is a named table and
> nothing more — it implements no build path and cannot render — and each of
> its fifteen action methods consumes it
> and returns the statement, whose own methods append the rest. PostgreSQL parses
> neither `ALTER TABLE "font"` nor `ALTER TABLE ADD COLUMN ...`, and neither MUST
> be constructible (`[dec:pgorm:invalid-states-unrepresentable]`); the
> `No alter option found` panic that stood in for the first of those is gone, and
> MUST NOT return. The statement has no `take()` for the same reason: moving the
> options out would leave the action-less statement this type exists to rule
> out, and a `take()` that copied instead would be a promise the name does not
> keep (`[spec:pgorm:req:sql.ast]`) — a second copy is `.to_owned()`.
>
> `add_foreign_key` — on both `PendingTableAlter` and `TableAlterStatement` —
> takes any `IntoAddedConstraint<TableForeignKey>` by value, as `add_column`
> takes `IntoColumnDef`: an embedder consumes what it embeds, and a borrow
> that silently cloned would be the third reuse-outcome
> `[spec:pgorm:req:sql.ddl.create-table]` rules out.
>
> A foreign key or a `CHECK` may be added `NOT VALID`, leaving the rows
> already there unchecked while every row written from then on is held to it.
> `TableForeignKey::not_valid()` and `Check::not_valid()` wrap the constraint
> in a `NotValid<C>` (read back by `get_constraint()`), and the only positions
> that take one are the two `ADD` actions, `add_foreign_key` and `add_check`,
> through `IntoAddedConstraint<C>`: anything that converts into the
> constraint, which is added checked, or the `NotValid` wrapper, which is
> added `NOT VALID`. The clause renders last, after the deferrability and the
> enforcement it may carry, which is where PostgreSQL takes it. Nothing that
> creates a table or a column takes a `NotValid` — `foreign_key()`, `check()`
> and `ColumnDef::check` — so it does not compile there
> (`[dec:pgorm:invalid-states-unrepresentable]`, `compile_fail` doctests on
> `NotValid`): a `CREATE TABLE` creates every constraint valid, its table
> having no rows to leave unchecked, so a table-level `NOT VALID` there is
> taken and ignored (`convalidated` true), and a column's `CHECK`,
> `REFERENCES` or an added column's `CHECK` has no place for it (`42601`). A
> `NOT NULL` is added `NOT VALID` by its own type
> (`NotNullConstraint::not_valid`), for the same reason. The live suite holds
> what it means: over rows that break it, the plain action is refused
> (`23503`, `23514`) and the `NOT VALID` one taken, `convalidated` false; a new
> breaking row is refused at once; `validate_constraint` refuses the old rows
> the same way, and once they are mended makes the constraint valid. A `NOT
> ENFORCED` constraint is never valid whatever it says, takes `NOT VALID`
> beside it, and `VALIDATE CONSTRAINT` refuses it (`55000`). PostgreSQL
> refuses `NOT VALID` on any other constraint (`0A000` on a unique key), which
> no type here carries.
>
> `add_primary_key` and `add_unique`, on both types, take the key a table
> declares when it is created — any `IntoTableKey` of their kind, a column, a
> tuple or a built `TableKey` — by value, and render `ADD [CONSTRAINT "name"
> ]PRIMARY KEY (cols)…` / `ADD [CONSTRAINT "name" ]UNIQUE [NULLS NOT DISTINCT
> ](cols)…` with the key's `INCLUDE` and deferrability, the table-level
> spelling of `[spec:pgorm:req:sql.ddl.create-table]` after `ADD`. They are
> how a key is added to a table that exists, now that a column carries none
> (`[spec:pgorm:req:sql.ddl.column-def]`): the `ADD COLUMN … UNIQUE` and
> `ADD UNIQUE ("c")` a column's key spec used to render are this, one key at a
> time. Whether the table already has a primary key is the server's
> knowledge, not the builder's; a second is refused there (`42P16`).
>
> Rendering MUST emit a single `ALTER TABLE <table> ` prefix
> with the options comma-separated: `ADD COLUMN [IF NOT EXISTS ]<column-def>`
> (same column rendering as create, including the serial substitution for
> auto-increment); `DROP COLUMN "c"`; `ADD CONSTRAINT ... FOREIGN KEY ...`
> (foreign-key clauses in `Mode::TableAlter`, i.e. without a nested `ALTER
> TABLE`); and `DROP CONSTRAINT [IF EXISTS ]"name"[ RESTRICT | CASCADE]`.
>
> A constraint of any kind is dropped by name, by the one action
> `drop_constraint`, on both types: a primary or unique key, a foreign key, a
> `CHECK` and PostgreSQL 18's `NOT NULL` alike, which kind the name holds
> being the server's knowledge. It takes any `IntoConstraintDrop`: a name,
> which is the plain drop, or a `ConstraintDrop` — the name its constructor
> takes, with by-value `if_exists()` and `cascade()` / `restrict()`, the last
> of those two winning, and the readers `get_name()`, `is_if_exists()` and
> `get_behavior()`. The behavior is a `DropBehavior`, `Restrict` or `Cascade`,
> unset rendering nothing, which is `RESTRICT`. The live suite holds each kind
> dropped this way, the row it refused admitted once it is gone, and what the
> server refuses: a name the table has no constraint under (`42704`), which
> `IF EXISTS` passes over with a notice; a key another table's foreign key
> depends on (`2BP01`), said `RESTRICT` or not, which `CASCADE` drops with the
> foreign key; the `NOT NULL` of a primary-key column (`42P16`); and a
> constraint a table inherited, which only its parent drops (`42P16`). The
> action replaces `drop_foreign_key`, which rendered this same `DROP
> CONSTRAINT` under a name for one kind, and the `DropForeignKey` option
> behind it; neither MUST return.
>
> A column rename is NOT one of those options. PostgreSQL admits `RENAME` only
> as the sole action of an `ALTER TABLE`, so it is a statement of its own:
> `Table::rename_column(table, from, to)` builds a `ColumnRenameStatement`
> rendering `ALTER TABLE <table> RENAME COLUMN "a" TO "b"`. A rename listed
> beside an `ADD COLUMN` therefore does not construct
> (`[dec:pgorm:invalid-states-unrepresentable]`). All three names are
> constructor arguments and none has a setter: PostgreSQL rejects the render
> that omits any of them, so the partly-named rename does not construct
> either.
>
> `ModifyColumn` decomposes into per-aspect Postgres actions: when a type is
> present, `ALTER COLUMN "c" TYPE <type>[ COLLATE <collation>]` — the retype
> is the only place PostgreSQL changes a column's collation, so a modified
> column that carries a collation and no type writes no collation, as it
> writes no `Generated` spec; then per spec `ALTER COLUMN "c"
> DROP NOT NULL` (for `Null`), `SET NOT NULL`, `SET DEFAULT <expr>`,
> `ADD <check>` or the
> `Extra` string, comma-separated. `AutoIncrement`, `Generated` and `Comment`
> specs are ignored in modify. A `Check` is a constraint the table gains, so it
> is written as the action that adds one, as `add_check` writes it; the bare
> `CHECK (<expr>)` it was once written as is no `ALTER TABLE` action, and the
> server refuses it (`42601`). `SET NOT NULL` is the plain `NotNull`'s: it
> has no place for a name or `NO INHERIT`, so a modified column whose
> `NOT NULL` carries either writes the table-level action that does,
> `ADD [CONSTRAINT "name" ]NOT NULL "c"[ NO INHERIT]`, as `add_not_null`
> would. The two differ over a constraint that is still `NOT VALID`: `SET NOT
> NULL` validates the one it finds, keeping its name, where the `ADD` is
> refused (`55000`).
>
> `add_not_null(constraint)`, on both types, takes a `NotNullConstraint`:
> PostgreSQL 18's table-level `NOT NULL`, over one column the constructor
> takes (`NotNullConstraint::new(c)`; the grammar refuses `NOT NULL a, b` and
> `NOT NULL (a)`, `42601`), with by-value `name(n)`, `no_inherit()` and
> `not_valid()` and the readers `get_column()`, `get_name()`, `is_no_inherit()`
> and `is_not_valid()`. It renders `ADD [CONSTRAINT "name" ]NOT NULL "c"[ NO
> INHERIT][ NOT VALID]` and is the one spelling with a place for `NOT VALID`,
> which leaves the rows already there unchecked while holding new rows at once
> (`23502`); `convalidated` stays false until `validate_constraint` checks
> them. Without `NOT VALID` a null already there refuses the action (`23502`).
> A column has one not-null constraint (`[spec:pgorm:req:sql.ddl.column-def]`),
> so the action against a column that has one does nothing where the one there
> already says as much — the same name or none, the same `NO INHERIT`, and
> valid where this one is — and is refused (`55000`) where it does not: a
> different name, a different `NO INHERIT`, or a valid constraint over one
> still `NOT VALID`. What the column has is the server's knowledge, as is
> whether the column exists (`42703`) and whether the table is partitioned,
> where `NO INHERIT` is refused (`0A000`); `add_primary_key` over a column
> whose constraint is still `NOT VALID` is refused as well (`55000`).
>
> `add_check(c)`, on both types, takes any `IntoAddedConstraint<Check>` — an
> `IntoCheck` (`[spec:pgorm:req:sql.ddl.create-table]`) or a `NotValid<Check>`
> — and renders `ADD [CONSTRAINT "name" ]CHECK (<expr>)[ NO INHERIT][ ENFORCED
> | NOT ENFORCED][ NOT VALID]`: the rows already there are checked and one
> that breaks the condition refuses the action (`23514`), unless the
> constraint is `NOT ENFORCED`, which holds no row
> (`[spec:pgorm:req:sql.ddl.enforcement]`), or `NOT VALID`, which leaves them
> for `validate_constraint`.
>
> `validate_constraint(name)` renders `VALIDATE CONSTRAINT "name"`: the rows a
> `NOT VALID` foreign key, `CHECK` or `NOT NULL` constraint skipped, checked
> now and refused as the insert would have been (`23502` for a null). It is
> the server's knowledge which kind a name holds: any other kind is refused
> (`42809`), a name the table has no constraint under too (`42704`), and a
> constraint already valid is left as it is. `alter_constraint(name, change)`
> renders `ALTER CONSTRAINT "name" <change>` for a `ConstraintChange`:
> `Inherit` (`INHERIT`), under which a `NOT NULL` passes to inheriting tables
> again and each child lacking it takes it, and `NoInherit` (`NO INHERIT`),
> under which it is kept from them from then on, each child keeping its copy
> as its own (`conislocal`). Both are PostgreSQL 18's and apply to a `NOT
> NULL` alone: a `CHECK`, key or foreign key named there is refused (`42809`),
> as is `NO INHERIT` on a partitioned table's (`0A000`). `Enforced`
> (`ENFORCED`) and `NotEnforced` (`NOT ENFORCED`), also 18's, apply to a
> foreign key and, from PostgreSQL 19, a `CHECK`
> (`[spec:pgorm:req:sql.ddl.enforcement]`): a key's enforcement is refused
> there (`42809`), as 18 refuses a `CHECK`'s. Each change is a
> closed choice rather than a flag, so an `ALTER CONSTRAINT` that changes
> nothing has no value to be built from
> (`[dec:pgorm:invalid-states-unrepresentable]`).
>
> A constraint rename is a statement of its own for the reason a column's is:
> `Table::rename_constraint(table, from, to)` builds a
> `ConstraintRenameStatement` rendering `ALTER TABLE <table> RENAME CONSTRAINT
> "a" TO "b"`, with all three names constructor arguments and no setter. PostgreSQL refuses `RENAME
> CONSTRAINT` beside another action (`42601`), so one listed beside `ADD`
> does not construct. It renames a constraint of any kind, a `NOT NULL`
> included, and the live suite holds what it does: a primary or unique key's
> index takes the new name with it, and a parent's rename reaches each
> child's inherited copy. The server refuses a name the table has no
> constraint under (`42704`), a new name another of its constraints holds
> (`42710`) or, renaming a key, a name another relation holds, its index
> being renamed too (`42P07`), and the rename of an inherited copy on the
> child (`42P16`).
>
> A generated column's expression is changed by an action of its own, not by
> a modified column's `Generated` spec, which would carry a kind PostgreSQL
> cannot change (`[spec:pgorm:req:sql.ddl.column-def]`) and so describe an
> alteration no statement makes. `set_expression(column, expr)`, on both
> `PendingTableAlter` and `TableAlterStatement`, renders `ALTER COLUMN "c" SET
> EXPRESSION AS (<expr>)` (PostgreSQL 17): the column keeps its kind and the
> rows already written take the new value — a stored column's table is
> rewritten to hold it (its `relfilenode` changes) and a virtual column's is
> not, since it computes on read. The `AS` is always written, `SET EXPRESSION
> (<expr>)` being a syntax error (`42601`), and the column and expression are
> both arguments, so neither half is missing from a built action.
> `drop_expression(column)` renders `ALTER COLUMN "c" DROP EXPRESSION` and
> `drop_expression_if_exists(column)` the same with ` IF EXISTS`: a stored
> column becomes a plain one that keeps each row's last computed value and is
> written like any other from then on.
>
> Whether the column is generated, and which kind, is the server's knowledge,
> and the live suite holds each refusal by SQLSTATE: either action on a column
> that is not generated, an identity included, is refused (`55000`), except
> that `DROP EXPRESSION IF EXISTS` passes over it with a notice; `DROP
> EXPRESSION` on a virtual column is refused, `IF EXISTS` or not (`0A000`),
> having no stored values to keep; `SET EXPRESSION` on a virtual column is
> refused while its table belongs to a publication (`0A000`), where a stored
> column's goes through; a `CHECK` constraint on the table refuses it too on
> PostgreSQL 18 (`0A000`), while from 19 the rows are checked against the
> table's `CHECK` constraints instead, a row that breaks one refusing the
> action (`23514`); and a new
> expression is held to what a generated column's is at creation —
> immutable, reading no other generated column (`42P17`), holding no
> subquery (`0A000`), of the column's type (`42804`). Both actions combine
> with the statement's others in one `ALTER TABLE`.

> [spec:pgorm:req:sql.ddl.drop-rename-truncate+4]
> `TableDropStatement` accumulates multiple `TableName`s and MUST render
> `DROP TABLE [IF EXISTS ]"t1", "t2"[ RESTRICT][ CASCADE]` (`restrict()` and
> `cascade()` append `TableDropOpt`s in call order). `TableRenameStatement`
> MUST render `ALTER TABLE <from> RENAME TO <to>`, where the source is a
> `TableName` and the target is a bare `Name`: `RENAME TO` cannot move a
> table between schemas, so a qualified target does not construct
> (`[dec:pgorm:invalid-states-unrepresentable]`). `TableTruncateStatement`
> MUST render `TRUNCATE TABLE <table>`; no `CASCADE`/`RESTART IDENTITY`
> options are exposed.
>
> All three take their targets in the constructor, because PostgreSQL rejects
> every one of these statements with the name left out: `Table::drop(table)`
> seeds the list and `table()` appends the rest, in the pattern
> `[spec:pgorm:req:sql.ddl.index-create]` uses for index columns, so the
> empty `DROP TABLE ` cannot be built; `Table::rename(from, to)` and
> `Table::truncate(table)` take theirs whole and expose no setter. `take()` on
> a drop copies the target list rather than moving it, so no target-less husk
> is left behind.

## Comments

> [spec:pgorm:req:sql.ddl.comment+5]
> A comment is a statement of its own on Postgres, not a clause of `CREATE
> TABLE`, so `CommentStatement` is built separately from the DDL creating the
> object it describes. `Comment::on_table(table, text)` and
> `Comment::on_column(table, column, text)` are the only constructors and both
> take target and text up front, so every `CommentStatement` denotes a
> complete statement and no build path can fail or panic. The target table is
> a `TableName` (`[spec:pgorm:def:sql.types.table-ref]`) — the same type
> every other DDL statement targets, reached through `IntoTableName` from an
> iden or a `(schema, table)` tuple — so a comment can only name a table the
> DDL beside it could also name, and there is no conversion to fail.
>
> Rendering MUST emit `COMMENT ON TABLE <table> IS <text>` or
> `COMMENT ON COLUMN <table>.<column> IS <text>`, where the table, schema
> and column names render through `SqlName::prepare` (double-quoted,
> embedded quotes doubled) and the text renders as a string literal: every
> embedded single quote doubled, and nothing else altered *unless the text
> holds a backslash*, in which case every backslash is doubled too and the
> literal is written `E'<text>'`.
>
> The text is never a bind parameter (a DDL statement yields SQL alone), so
> this quoting is the whole injection boundary for comment text — and a
> boundary MUST NOT depend on a session setting. Doubling alone does.
> `standard_conforming_strings` decides whether a backslash escapes the
> character after it, so under `off` the text `\'` renders as `\''` and reads
> as an escaped quote followed by a *closing* one, with everything after it
> read as SQL. The setting is not hypothetical: `pgorm_pool::Config::options`
> hands a caller's `-c` flags to the connection, so the session that executes
> this DDL is reachable configuration rather than an assumption about the
> server. Writing the backslashes out under an `E''` says what they are under
> either reading; text with no backslash has nothing to disambiguate and MUST
> keep the plain form, so the common rendering is unchanged.
>
> This is NOT `[spec:pgorm:req:sql.render.string-escape]`, which additionally
> maps control characters to their `E''` spellings. Comment text is prose and a
> newline in it is a newline; only what the grammar forces is rewritten.
>
> The comment text a `TableCreateStatement` carries — its own
> (`[spec:pgorm:req:sql.ddl.create-table]`) and each `ColumnSpec::Comment`
> (`[spec:pgorm:req:sql.ddl.column-def]`) — MUST be reachable as those
> statements: `TableCreateStatement::comments()` returns one
> `CommentStatement` per carried comment, the table's first and then one per
> commented column in column order, each targeting the statement's own table.
> The setters therefore describe a table completely and no carried text is
> unrenderable. They MUST NOT instead be appended to the create statement's
> own SQL: that string is executed as a single prepared statement
> (`[spec:pgorm:sem:conn.pool.statement-cache]`), and PostgreSQL refuses to
> prepare a string holding more than one command, so appending would trade a
> comment silently dropped for a `CREATE TABLE` that no longer runs.

## Indexes

> [spec:pgorm:req:sql.ddl.index-create+11]
> `IndexCreateStatement` carries a target table, a `TableIndex` (name plus
> ordered `IndexColumn`s), an `IndexKind`, an `include` list of non-key column
> names, a `where` predicate, and `nulls_not_distinct`,
> `index_type` and `if_not_exists` flags. Its target table and its column list
> MUST both be non-empty by construction: `Index::create(table, col)` and
> `IndexCreateStatement::new(table, col)` take the table and the first column
> and `col()` appends the rest, in the pattern
> `[spec:pgorm:def:sql.ast.with]` uses for CTEs, and there is no `table()`
> setter. It has no `take()` at all: moving the table or the columns out would
> leave exactly the target-less, column-less husk the constructor rules out, and
> a `take()` that copied instead would be a promise the name does not keep
> (`[spec:pgorm:req:sql.ast]`) — a second copy is `.to_owned()`. PostgreSQL rejects an empty
> column list (`CREATE INDEX ... ()`) and rejects `CREATE INDEX "n" ON  (...)`
> at the parenthesis, so both states are unreachable rather than checked
> (`[dec:pgorm:invalid-states-unrepresentable]`). The index *name* is the one
> part that stays optional: PostgreSQL derives a name when `CREATE INDEX`
> omits it, so `CREATE INDEX  ON "t" ("c")` parses and is left buildable.
> `IndexKind` is the closed pair `Plain | Unique`: `unique()` sets it, and
> `is_unique_key()` and `kind()` read it back. There is no primary-key kind,
> because PostgreSQL spells `PRIMARY KEY` only as a table constraint and never
> as `CREATE INDEX`; the primary key is the table's `TableKey`
> (`[spec:pgorm:req:sql.ddl.create-table]`), and so the standalone
> renderer has no primary key to be handed. `IntoIndexColumn` accepts an
> iden, an `(iden, IndexOrder)` pair, or an `IndexColumn` built outright, and
> nothing else: the MySQL prefix-length forms
> `(iden, u32)` and `(iden, u32, IndexOrder)` are gone with the
> `IndexColumn::prefix` field they fed, and MUST NOT return.
>
> An `IndexColumn` is one entry of the index: an `IndexColumnTarget`, an
> optional operator class, and an optional order. The target is the closed pair
> `Name(Name) | Expr(SimpleExpr)`, because PostgreSQL renders the two
> differently — a column bare, an expression parenthesised — and which one an
> entry holds MUST be a state of the type rather than a shape the renderer
> infers from an expression
> (`[dec:pgorm:invalid-states-unrepresentable]`). `IndexColumn::name(n)` and
> `IndexColumn::expr(e)` construct them, and `operator_class(c)` and `order(o)`
> each set their slot outright, replacing whatever was there; the
> `(iden, IndexOrder)` tuple stays as the shorthand for the common case. The
> operator class is an identifier and MUST render quoted like every other name,
> never as SQL.
>
> The standalone form MUST render `CREATE [UNIQUE ]INDEX [IF NOT EXISTS
> ]"name" ON <table>[ USING <type>] (cols)[ INCLUDE (names)][ NULLS NOT
> DISTINCT][ WHERE <predicate>]` — the grammar's own order for those clauses —
> where
> `<type>` is `BTREE`, `GIN` (the `IndexType::Gin` variant, also set by the
> `gin()` shorthand — the access method is named for what PostgreSQL calls it,
> not for the full-text use it serves), `HASH`, or a custom identifier, and
> each entry renders as `"name"|(<expr>)[ "opclass"][ ASC|DESC]`. There is no
> prefix length: `"name" (128)` is MySQL's
> syntax for indexing a leading substring, PostgreSQL rejects it outright, and
> an index the server cannot accept MUST NOT be constructible — the expression
> index is the legitimate occupant of that syntactic position. Postgres defines
> `NULLS NOT DISTINCT` for unique indexes alone, so the flag MUST render only
> when the kind is `Unique`; on any other kind it is carried but not spelled.
>
> The predicate is a `ConditionHolder` reached through `ConditionalStatement`,
> so `and_where`/`cond_where` conjoin here exactly as they do on a query
> (`[spec:pgorm:req:sql.render.condition-chain]`) and an absent predicate spells
> no keyword. `include(cols)` appends rather than replaces, matching every other
> accumulating builder on this statement.
>
> An `IndexCreateStatement` renders only as `CREATE INDEX`: it does not
> convert into the `TableKey` a table declares
> (`[spec:pgorm:req:sql.ddl.create-table]`), which is a type of its own
> because nearly everything this statement carries — the `Plain` kind, an
> expression or ordered entry, an operator class, the predicate, the access
> method — is a syntax error in a table constraint. The split runs both
> ways: deferrability, which a key takes and `CREATE INDEX` does not, is a
> field of `TableKey` and not of this statement, whose standalone rendering
> would have to drop it or emit a syntax error
> (`[spec:pgorm:req:sql.ddl.deferrability]`).
>
> `CONCURRENTLY` MUST NOT be offered. It is not a property of the index but of
> how the statement runs: PostgreSQL refuses it inside a transaction block, and
> pgorm executes DDL through connections whose migrations and test fixtures are
> transactional, so a builder that could spell it would produce a statement the
> runtime cannot run. A caller who needs it runs the SQL themselves, outside a
> transaction, where the constraint is visible. The index target is a
> `TableName`, so both of its forms render and no other shape is
> constructible.

> [spec:pgorm:req:sql.ddl.index-drop+3]
> `IndexDropStatement` MUST render `DROP INDEX [IF EXISTS ]["schema".]"name"`.
> The index name is a `Name` taken by `Index::drop(name)`, being the whole
> of what the statement names: PostgreSQL rejects `DROP INDEX ` at end of
> input, so the nameless drop does not construct
> (`[dec:pgorm:invalid-states-unrepresentable]`). The table is the part that
> stays optional, and only its schema portion is used (indexes are
> schema-scoped in Postgres); a plain `Table` name contributes nothing, and
> `DROP INDEX "name"` with no table at all is valid PostgreSQL.

## Foreign keys

> [spec:pgorm:req:sql.ddl.foreign-key+10]
> `TableForeignKey` holds the owning and referenced table names, a non-empty
> list of `(column, referenced column)` pairs, an optional constraint name, and
> optional `on_delete`/`on_update` `ForeignKeyAction`s (`Restrict`→`RESTRICT`,
> `Cascade`→`CASCADE`, `SetNull`→`SET NULL`, `NoAction`→`NO ACTION`,
> `SetDefault`→`SET DEFAULT`), an optional `Deferrability` saying when the
> check runs (read back by `get_deferrability()`), and an optional
> `Enforcement` saying whether it runs at all
> (`enforcement(e)`, read back by `get_enforcement()`;
> `[spec:pgorm:req:sql.ddl.enforcement]`). `TableForeignKey::new(table, column, ref_table,
> ref_column)` — reached from a statement as `ForeignKey::create(..)` — takes
> both tables and the first pair, and `col(column, ref_column)` appends further
> pairs; there is no setter for either table and no constructor taking a column
> list, because PostgreSQL rejects `ALTER TABLE  ADD FOREIGN KEY`,
> `FOREIGN KEY ()`, `REFERENCES  ()` and `REFERENCES "t" ()` alike
> (`[dec:pgorm:invalid-states-unrepresentable]`). Holding the two sides as one
> list of pairs also makes the arity mismatch unrepresentable — a render the
> grammar accepts and only parse analysis rejects, so no oracle could have
> caught it. Neither `TableForeignKey` nor the `ForeignKeyCreateStatement`
> wrapping it has a `take()`: moving the tables and the first pair out would
> leave exactly the husk the constructor rules out, and a `take()` that copied
> them would be a promise the name does not keep
> (`[spec:pgorm:req:sql.ast]`). A second copy is `.to_owned()`.
>
> A foreign key may be PostgreSQL 18's temporal one: `period(column,
> ref_column)`, on `TableForeignKey` and `ForeignKeyCreateStatement`, closes
> both column lists with `PERIOD`, a later call replacing the pair, and
> `get_period()` reads it back; `columns()`, `get_columns()` and
> `get_ref_columns()` are the pairs matched for equality alone. Those still
> match by equality, and the referencing row's period, a range or multirange,
> must be covered by the union of the periods of the referenced rows that
> match them. The live suite holds what it means: a child period spanning two
> adjacent parent rows is admitted, one running past them or naming no parent
> is refused (`23503`), and so is deleting a parent row a child's period
> needs; the plain foreign key over the same pairs, the control, is refused
> outright (`42830`), `PERIOD` being the only way to reference a key ending
> `WITHOUT OVERLAPS` (`[spec:pgorm:req:sql.ddl.create-table]`). PostgreSQL
> takes `PERIOD` on the last column of each list alone and refuses a list it
> is the only column of (both `42601`), and refuses it on one side without
> the other (`42830`). So the pair is a slot of its own, written last on both
> sides whatever order the calls come in, one call writes both sides, and the
> constructor's pair always precedes it
> (`[dec:pgorm:invalid-states-unrepresentable]`). That the referenced key is
> temporal (`42830`) and the two periods share a type (`42804`) is the
> server's knowledge.
>
> PostgreSQL 18 runs a temporal foreign key's referential actions as `NO
> ACTION` alone: `CASCADE`, `SET NULL`, `SET DEFAULT` and `RESTRICT` are each
> refused, on delete and on update (`0A000`, *unsupported ON DELETE action for
> foreign key constraint using PERIOD*), where `NO ACTION` said outright is
> taken, as the live suite holds. The builder can write the refused pair, by
> choice. Ruling it out by type takes a second foreign-key type, one with no
> action to set, through every embedder, reader and `ALTER TABLE` option
> that takes the one type now; and a setter chain over `&mut` cannot move a
> key from one type to the other partway, so that type would need a
> constructor taking all six names, or a by-value change of type that could
> carry an action set before it. The server refuses the pair in the statement
> that creates the key, and the refusal is a release's limit rather than the
> grammar's: the SQL standard defines the actions on a temporal key, and
> PostgreSQL leaves them to a later release, as they need the `UPDATE` /
> `DELETE ... FOR PORTION OF` that was withdrawn from 19 before its release.
> Deferrability and enforcement a temporal key takes as a plain one does.
>
> `Deferrability` is the one enum of `[spec:pgorm:req:sql.ddl.deferrability]`,
> which unique and primary keys share. The foreign key is the case with a use
> an ORM meets — rows that reference each other, which only a check deferred
> to `COMMIT` lets a transaction insert.
>
> The standalone statement MUST render `ALTER TABLE <from> ADD [CONSTRAINT
> "name" ]FOREIGN KEY (cols[, PERIOD "period"]) REFERENCES <to> (ref-cols[,
> PERIOD "ref-period"])[ ON DELETE <action>][ ON UPDATE <action>][
> <deferrability>][ <enforcement>]`; inside `CREATE TABLE` the same clause renders
> without the `ALTER TABLE`/`ADD` prefix, and inside `ALTER TABLE` options
> only the `ALTER TABLE` prefix is dropped. On the `CREATE TABLE` path the key
> is restamped onto the owning table by `TableCreateStatement::foreign_key`:
> an embedded key constrains the table it sits inside and MUST NOT name
> another. That embedder, and the
> `add_foreign_key` of `[spec:pgorm:req:sql.ddl.alter-table]`, take the key by
> value (`Into<ForeignKeyCreateStatement>` and `Into<TableForeignKey>`
> respectively) rather than by reference: an embedder consumes what it embeds,
> so a caller who reuses the key writes the copy
> (`[spec:pgorm:req:sql.ddl.create-table]`). A foreign key is dropped by
> name as a constraint of any kind is, by `drop_constraint`
> (`[spec:pgorm:req:sql.ddl.alter-table]`): `ForeignKey::drop` and the
> `ForeignKeyDropStatement` it built rendered that same `ALTER TABLE <table>
> DROP CONSTRAINT "name"` under a name for one kind, and are gone with the
> `Drop` arm of `ForeignKeyStatement`; they MUST NOT return. Foreign-key table
> targets are `TableName`s, so both forms render and no other shape is
> constructible.

## Deferrability

> [spec:pgorm:req:sql.ddl.deferrability+4]
> `Deferrability` says when a constraint's check runs. It is `NotDeferrable`,
> `DeferrableInitiallyImmediate` or `DeferrableInitiallyDeferred` —
> PostgreSQL's three reachable states as one closed choice rather than two
> independent flags, because `INITIALLY DEFERRED` is grammatical only on a
> `DEFERRABLE` constraint and the pair naming a constraint both undeferrable
> and initially deferred MUST NOT construct
> (`[dec:pgorm:invalid-states-unrepresentable]`). It renders ` NOT DEFERRABLE`,
> ` DEFERRABLE INITIALLY IMMEDIATE` or ` DEFERRABLE INITIALLY DEFERRED`
> directly after the constraint it qualifies. Unset, the clause renders
> nothing: `NOT DEFERRABLE` is the server's default, so a builder that emitted
> it would be asserting a choice the caller did not make.
>
> The one enum qualifies every constraint the builder spells that PostgreSQL
> lets defer. A foreign key carries it as a field
> (`[spec:pgorm:req:sql.ddl.foreign-key]`). A primary or unique key carries
> it on its `TableKey`, whose `deferrability(d)` sets it
> (`[spec:pgorm:req:sql.ddl.create-table]`): it follows the key's column
> list and any `INCLUDE`, in `CREATE TABLE` and after `ALTER TABLE`'s `ADD`
> alike (`[spec:pgorm:req:sql.ddl.alter-table]`). A column carries no key
> and so no key's deferrability: the column spellings that did —
> `unique_key_deferrability(d)` and `primary_key_deferrability(d)` — are gone
> with the column keys (`[spec:pgorm:req:sql.ddl.column-def]`), and the
> clause has one position it can be written in, the one PostgreSQL's grammar
> puts it.
>
> Two positions take no deferrability, and the builder has no way to hand them
> one. A `CHECK` constraint is never deferrable: PostgreSQL refuses a
> table-level `CHECK (…) DEFERRABLE` in its grammar (`0A000`, "CHECK
> constraints cannot be marked DEFERRABLE") and a column-level one as a
> misplaced clause (`42601`), so `ColumnSpec::Check` and
> `TableCreateStatement::check` carry none, and neither does `NOT NULL`. And
> `CREATE UNIQUE INDEX` has no `DEFERRABLE` in its grammar (`42601`): only a
> constraint is deferred, never an index. `IndexCreateStatement`, which
> renders standalone as that statement, therefore holds no deferrability, and
> `TableKey`, which does, has no rendering of its own — no `Display` and no
> build path — so it reaches SQL only inside a table statement.
>
> The three states differ only in *when* the check runs, which no rendered
> text shows, so the distinction belongs to the live suite. `NOT DEFERRABLE`
> checks a unique or primary key row by row, so `UPDATE … SET n = n + 1` over
> consecutive values collides with the next row before that row has moved.
> `DEFERRABLE INITIALLY IMMEDIATE` checks at the end of each statement, where
> the same update succeeds, and `SET CONSTRAINTS … DEFERRED` can postpone it.
> `DEFERRABLE INITIALLY DEFERRED` checks at `COMMIT`, so a transaction may hold
> a duplicate between statements and is refused (`23505`) only if one is
> still there when it commits — or at `SET CONSTRAINTS … IMMEDIATE`, which runs
> the check where it is said.
>
> The clause has a cost the builder documents rather than refuses: a
> deferrable unique or primary key, `INITIALLY IMMEDIATE` included, cannot
> arbitrate an `ON CONFLICT` (`55000`), because the server cannot say whether a
> row conflicts while the check that would say so may still be pending. That
> holds whether the arbiter is inferred from columns or named outright with
> `ON CONSTRAINT` (`sql.ast.on-conflict`): which constraint it reaches, and
> whether that one is deferrable, is the server's knowledge, not the
> builder's, so a table that upserts on a key keeps that key undeferrable.

## Enforcement

> [spec:pgorm:req:sql.ddl.enforcement]
> `Enforcement` says whether the server holds rows to a constraint, PostgreSQL
> 18's `ENFORCED` / `NOT ENFORCED`: `Enforced` or `NotEnforced`, a closed pair
> rendering ` ENFORCED` or ` NOT ENFORCED` directly after the constraint it
> qualifies, and after its deferrability where it has one — the order
> `pg_get_constraintdef` writes. Unset, the clause renders nothing: `ENFORCED`
> is the server's default, so a builder emitting it would assert a choice the
> caller did not make, and `Enforced` renders only because a caller said it.
>
> A foreign key carries it as a field (`[spec:pgorm:req:sql.ddl.foreign-key]`)
> and a `CHECK` constraint on its `Check` (`[spec:pgorm:req:sql.ddl.create-table]`),
> and nothing else can, because nothing else takes it: PostgreSQL refuses
> `[NOT] ENFORCED` on a primary key, a unique key, a `NOT NULL` and an
> `EXCLUDE` at table level (`0A000`, "... constraints cannot be marked NOT
> ENFORCED"), and after a column's key, `NOT NULL` or `DEFAULT` as a misplaced
> clause (`42601`). `TableKey<K>`, `NotNullConstraint` and a column's
> `NotNull` therefore have no enforcement to set
> (`[dec:pgorm:invalid-states-unrepresentable]`), and as no unique key, primary
> key or exclusion constraint can be `NOT ENFORCED`, no `ON CONFLICT` arbiter
> is one (`sql.ast.on-conflict`).
>
> What the clause means belongs to the live suite, since no rendered text shows
> it. A `NOT ENFORCED` constraint is recorded (`pg_constraint.conenforced`
> false) and never checked: rows that break it are inserted, it is never
> valid (`convalidated` false), a foreign key has no referential triggers so
> its `ON DELETE` / `ON UPDATE` actions never fire, and it still needs the
> referenced columns' unique key to be created (`42830`). It may be
> `DEFERRABLE` (the server records both, the deferral meaning nothing while
> nothing is checked); a `CHECK` stays undeferrable either way (`0A000`).
> `VALIDATE CONSTRAINT` refuses it (`55000`). `ALTER CONSTRAINT "name"
> ENFORCED` / `NOT ENFORCED` (`ConstraintChange`, `[spec:pgorm:req:sql.ddl.alter-table]`)
> moves a foreign key between the two, `ENFORCED` checking every row as it
> goes (`23503`) and `NOT ENFORCED` leaving it not valid. From PostgreSQL 19
> it moves a `CHECK` the same way, `ENFORCED` refusing a row that breaks the
> condition (`23514`); 18 cannot alter a `CHECK`'s (`42809`), so there one is
> dropped and added again.

## Enum types

> [spec:pgorm:req:sql.ddl.type-enum+7]
> An enum type reference declares an optional schema — `Type::create` and its
> siblings take any `IntoTypeRef`, so `(schema, name)` names a qualified type
> and a bare name an unqualified one — and every DDL rendering MUST qualify
> when a schema is present, via `TypeRef`'s quoted, dot-joined parts.
> `TypeCreateStatement` takes its type name in `Type::create(name)`, because
> `CREATE TYPE ` is rejected at end of input and a nameless statement
> therefore MUST NOT construct
> (`[dec:pgorm:invalid-states-unrepresentable]`). The name alone MUST render
> `CREATE TYPE <name>`, which PostgreSQL accepts as a shell type. `as_enum()`
> makes it an enumeration and `values(iter)` appends labels, implying
> `as_enum()` when it has not been called — the marker and the labels are one
> field (`TypeAs::Enum(Vec<String>)`), so no label list survives without the
> `AS ENUM` that renders it. A label is DATA, not a name: it renders as a
> string literal, so `values` is bound `Into<String>` and MUST NOT take an
> identifier type. The two are not interchangeable — an identifier bound
> would invite a caller to pass an `SqlName` whose text is then emitted as a
> literal, spelling a contract the render does not keep. An enumeration MUST render `CREATE TYPE <name> AS
> ENUM (<labels>)` with the parentheses always present, empty list included:
> `CREATE TYPE "t" AS ENUM ()` is an accepted spelling of the empty enum, and
> it was the missing parentheses — `CREATE TYPE "t" AS ENUM` — that PostgreSQL
> rejected. The type name is a quoted identifier (`TypeRef`
> supports `Type`, `SchemaType` and `DatabaseSchemaType` dotted forms) while
> the labels pass through the value pipeline, i.e. single-quoted string
> literals in `to_string` builds and bind parameters in parameterised builds.
> `TypeAs` has three variants, `Enum`, `Composite`
> (`[spec:pgorm:req:sql.ddl.type-composite]`) and `Range`
> (`[spec:pgorm:req:sql.ddl.type-range]`), and what a type is, is that
> one slot: `as_enum()` and `values()` on a composite or a range replace it
> with a label list, as `as_composite()` and `attribute()` and `as_range()`
> replace what was there, so a statement never carries two kinds. A base
> type, which names C input and output functions, belongs with
> `CREATE FUNCTION` outside the builder.

> [spec:pgorm:req:sql.ddl.type-alter-drop+7]
> `TypeAlterStatement` MUST render `ALTER TYPE <name>` followed by exactly one
> option: `ADD VALUE 'v'`, `ADD VALUE 'v' BEFORE 'w'` / `AFTER 'w'`
> (`before()`/`after()` only upgrade an existing `Add` option and are no-ops
> otherwise), `RENAME TO "new"`, or `RENAME VALUE 'old' TO 'new'`. The enum
> labels go through the value pipeline and render as single-quoted string
> literals; the `RENAME TO` target is a type name, not a label, and MUST
> render as a quoted identifier. The bounds MUST say which is which:
> `add_value`, `rename_value`, `before` and `after` take `Into<String>` and
> carry `String` in `TypeAlterOpt::Add` / `RenameValue` and
> `TypeAlterAddOpt::Before` / `After`, while `rename_to` keeps `IntoName`
> and `TypeAlterOpt::Rename` keeps `Name`. Unlike the other type builders,
> `TypeAlterStatement` methods take `self` by value.
>
> `Type::alter(name)` yields a `PendingTypeAlter` rather than a statement, and
> each option method consumes it into a `TypeAlterStatement` carrying the name
> and that one option: PostgreSQL rejects both `ALTER TYPE ` and `ALTER TYPE
> "t"` with no option, so neither the nameless nor the option-less form MUST be
> constructible (`[dec:pgorm:invalid-states-unrepresentable]`), the same
> `PendingTableAlter` shape `Table::alter` uses. Its attribute methods consume
> it into the composite's own statements instead, `CompositeAlterStatement`
> and `AttributeRenameStatement` (`[spec:pgorm:req:sql.ddl.type-composite]`):
> a composite's changes chain and take a behavior, which no enumeration
> option does, so they are not `TypeAlterOpt` variants that a label method
> could be applied to.
>
> `TypeDropStatement` MUST render `DROP TYPE [IF EXISTS ]<name1>, <name2>
> [ CASCADE|RESTRICT]` with names as quoted (possibly schema-qualified)
> identifiers; `cascade()` and `restrict()` overwrite the same option slot,
> so the last call wins. `Type::drop(name)` takes the first name and `name()` /
> `names()` append further ones, so the list is non-empty by construction and
> the `DROP TYPE ` PostgreSQL rejects at end of input does not build.
>
> None of this is particular to an enumeration. `DROP TYPE` and `RENAME TO`
> name a type of any kind and serve a composite unchanged
> (`[spec:pgorm:req:sql.ddl.type-composite]`); the label options are an
> enumeration's, and the server refuses them on any other type (`42809`).

## Composite types

> [spec:pgorm:req:sql.ddl.type-composite+2]
> `TypeCreateStatement` defines a composite type — a row type — beside the
> enumeration of `[spec:pgorm:req:sql.ddl.type-enum]` and the range of
> `[spec:pgorm:req:sql.ddl.type-range]`. `as_composite()`
> makes it one, and `attribute(name, type)` and `attribute_collated(name,
> type, collation)` append an attribute, implying `as_composite()` when it has
> not been called. The marker and the attributes are one field
> (`TypeAs::Composite(Vec<CompositeAttribute>)`), so no attribute list
> survives without the `AS (...)` that renders it.
>
> A composite MUST render `CREATE TYPE <name> AS (<attribute>, ...)`, each
> attribute its name as a quoted identifier, one space, its `ColumnType` as a
> column writes it (`[spec:pgorm:def:sql.render.ddl.types]`), and then
> ` COLLATE <collation>` when it has one, quoted as
> `[spec:pgorm:req:sql.render.collate]` writes a column's. The parentheses
> MUST be present for an empty list too: `CREATE TYPE "t" AS ()` is the empty
> composite PostgreSQL accepts.
>
> An attribute is a `CompositeAttribute`, not a `ColumnDef`. The grammar gives
> an attribute a name, a type and a collation and nothing else — `NOT NULL`,
> `DEFAULT`, a constraint or `GENERATED` after it is a syntax error (`42601`)
> — so the spec list that could spell those is absent rather than ignored.
> The rest is the server's to refuse: a repeated attribute name (`42701`), a
> collation on a type that has none (`42804`), and a name the schema already
> uses — a composite is a relation itself, so a sequence's or an index's name
> is taken (`42P07`), and a table's or a view's is the name of the row type it
> already has (`42710`).
>
> What a type is, is one slot. Choosing a kind — `as_enum` or `values`,
> `as_composite` or `attribute`, `as_range` — replaces what the other kind
> held, as the last of `cascade()` and `restrict()` wins, so no statement
> carries labels, attributes or a range definition at once.
>
> `DROP TYPE` and `ALTER TYPE ... RENAME TO` need nothing new for a
> composite, and nothing was added: `TypeDropStatement` and `rename_to` name a
> type of any kind, and the live suite drops and renames a composite through
> them — `RESTRICT` refusing while a column still has the type (`2BP01`),
> `CASCADE` dropping that column with it. The label alterations are an
> enumeration's, and the server refuses them on a composite (`42809`, *is not
> an enum*).
>
> A composite's attributes are altered by `ALTER TYPE`, and a
> `PendingTypeAlter` (`[spec:pgorm:req:sql.ddl.type-alter-drop]`) starts
> either of two statements. `add_attribute(name, type)`,
> `add_attribute_collated(name, type, collation)`, `drop_attribute(name)`,
> `drop_attribute_if_exists(name)`, `alter_attribute(name, type)` and
> `alter_attribute_collated(name, type, collation)` each consume it into a
> `CompositeAlterStatement` holding that first `AttributeChange` — `Add` or
> `Retype`, each a `CompositeAttribute` written as the create writes one, or
> `Drop`, a name and an `IF EXISTS` flag — and the same six methods on the
> statement append further changes, which it renders comma-separated in call
> order, `ADD ATTRIBUTE "a" <type>[ COLLATE c]`, `DROP ATTRIBUTE [IF EXISTS
> ]"a"` and `ALTER ATTRIBUTE "a" TYPE <type>[ COLLATE c]`; `changes()` reads
> them back, never empty, so the change-less alteration has no constructor. A
> change's behavior is a slot of the statement: `cascade()` and `restrict()`
> set a `DropBehavior` (`[spec:pgorm:req:sql.ddl.alter-table]`), the last
> winning, read back by `get_behavior()`, and it is written after every
> change, since the grammar gives each change one; an unsaid behavior is
> `RESTRICT`. `rename_attribute(from, to)` consumes the pending alter into an
> `AttributeRenameStatement` rendering `RENAME ATTRIBUTE "a" TO "b"[
> <behavior>]`, with the same two behavior methods: PostgreSQL takes `RENAME`
> as an `ALTER TYPE`'s sole action (`42601` beside another), so the rename is
> a statement with no method to chain a change onto, which a `compile_fail`
> doctest holds (`[dec:pgorm:invalid-states-unrepresentable]`). An alteration
> names its type as the other type statements do, and every attribute and
> collation name is a quoted identifier registered with the identifier oracle.
>
> The live suite holds what the changes make and what the server refuses
> around them. Chained changes apply in order, and the catalogue holds the
> attributes they leave, each with its type and collation. A composite that
> is a typed table's type (`CREATE TABLE ... OF`) is altered only `CASCADE`,
> which carries each change into the table's columns: without it every
> change is refused (`2BP01`), said `RESTRICT` or not. A column of the type in
> another table refuses a retype even `CASCADE` (`0A000`). An attribute added
> twice is refused (`42701`), and one dropped that the type lacks too
> (`42703`), which `drop_attribute_if_exists` passes over with a notice. `NOT
> NULL`, `DEFAULT` or a `USING` conversion after a change is a syntax error
> (`42601`), and no change has a place for one.
>
> Nothing in pgorm reads a composite value. A column can be declared with the
> type (`ColumnType::named`), but no `Value` variant carries a composite, no
> `TryGetable` decodes one, and tokio-postgres decodes one only into a Rust
> type with a derived `FromSql`, which pgorm neither derives nor re-exports —
> so a model field of a composite type fails to decode, and the live suite
> holds a `String` refused. The read path is to expand the value in the
> projection, `(<column>).<attribute>` through `Expr::raw` or an
> `SqlTemplate`, or to cast it to `text` and parse that. The type this builds
> is therefore one raw SQL and server-side code can use and the ORM cannot yet
> name. A decode is what a consumer would add. The attribute alterations,
> which `sql.scope` deferred until a reader would keep in step with them, are
> built as DDL nonetheless: a migration changes a type's shape whether or not
> the ORM reads it, and the raw reads above follow the new shape.

## Range types

> [spec:pgorm:req:sql.ddl.type-range+2]
> `TypeCreateStatement::as_range(definition)` defines a range type, beside the
> enumeration of `[spec:pgorm:req:sql.ddl.type-enum]` and the composite of
> `[spec:pgorm:req:sql.ddl.type-composite]`, in the one slot what a type is
> occupies (`TypeAs::Range`), so it replaces a label or attribute list and is
> replaced by one. The definition is an `extension::RangeDefinition`,
> constructed over its subtype, `RangeDefinition::new(subtype)`, because
> `SUBTYPE` is the one option PostgreSQL requires (`42601` without it): a
> range type without one does not construct
> (`[dec:pgorm:invalid-states-unrepresentable]`). The subtype is a
> `ColumnType`, written as a column's type is
> (`[spec:pgorm:def:sql.render.ddl.types]`), so a type created elsewhere is
> named by `ColumnType::named`; PostgreSQL discards a type modifier there, so
> a range over `varchar(10)` is a range over `varchar`.
>
> The other options are set on the definition and written only when set, each
> a name and so quoted:
>
> - `subtype_opclass(name)`, `SUBTYPE_OPCLASS`: the b-tree operator class
>   that orders the subtype, when its default is not the ordering wanted;
> - `collation(collation)`, `COLLATION`: the collation a collatable subtype
>   is ordered under, any `IntoCollation` and so schema-qualifiable;
> - `subtype_diff(name)`, `SUBTYPE_DIFF`: the function measuring the
>   distance between two subtype values, which a GiST index over the range is
>   much faster for having;
> - `multirange_type_name(name)`, `MULTIRANGE_TYPE_NAME`: what the multirange
>   PostgreSQL creates beside the range is called, any `IntoTypeRef` and so
>   schema-qualified as the range's own name can be. Unset, PostgreSQL
>   derives it from the range's name, replacing `range` with `multirange` or
>   appending `_multirange` (`floatrange` → `floatmultirange`).
>
> The operator class and the difference function are unqualified, as a
> function name is everywhere in this builder (`Func::named`), and resolve on
> the search path. The statement MUST render `CREATE TYPE <name> AS RANGE
> (SUBTYPE = <type>[, SUBTYPE_OPCLASS = <name>][, COLLATION = <collation>][,
> SUBTYPE_DIFF = <name>][, MULTIRANGE_TYPE_NAME = <name>])`, in the order
> PostgreSQL documents them. It binds nothing, so its two renderings are one.
>
> `CANONICAL` is not built, and MUST NOT be until this builder can author
> the function it names. A canonical function takes and returns the range
> type itself, so it has to exist before the range does, against a shell type
> created first; PostgreSQL refuses the option on a type with no shell
> (`42P17`), refuses a shell type as an SQL function's argument (`42P13`) and
> as a PL/pgSQL function's result (`0A000`), which leaves a C function — a
> function body, outside the builder as `CREATE FUNCTION` is
> (`[spec:pgorm:req:sql.scope]`). A range without one is continuous, as
> `numrange` is: its bounds are stored as written.
>
> The rest is the server's to refuse, with its own codes: a collation on a
> subtype that has none (`42809`), a difference function that does not take
> two subtype values (`42883`), an operator class for another type (`42804`)
> or none of that name (`42704`), and a subtype that does not exist
> (`42704`). `DROP TYPE` and `ALTER TYPE ... RENAME TO` reach a range as they
> reach the other kinds, and the live suite drops one through them —
> `RESTRICT` refusing while a column has the type (`2BP01`), `CASCADE`
> taking the multirange with it.
>
> A value of a range type created this way is its text form cast to the
> type by name, and a column of one is `ColumnType::CreatedRange`, named and
> carrying its subtype (`[spec:pgorm:def:sql.value.created-range]`); a built-in
> `Range<T>` value reaches such a column through that cast, there being no
> cast between two range types. Its multirange travels as its text too, and
> is read as its text, because tokio-postgres reports it as a simple type
> (`[spec:pgorm:req:exec.cursor.binding-range]`); a column of one is
> `ColumnType::CreatedMultirange`.

## Sequences

> [spec:pgorm:req:sql.ddl.sequence]
> `Sequence` builds the statements a sequence takes: `create(name)`,
> `alter(name)`, `drop(name)` and `rename(from, to)`. A sequence is a
> relation, so each name is an `IntoTableName` — a bare name or a
> `(schema, name)` pair — rendered as quoted, dot-joined identifiers, and the
> new name of a rename is a bare `Name`, because `RENAME TO` leaves a sequence
> in its schema and the grammar refuses a qualified one (`42601`).
>
> - `SequenceCreateStatement` MUST render `CREATE SEQUENCE [IF NOT EXISTS
>   ]<name>[ AS <type>][ <options>][ OWNED BY <owner>]`.
> - `SequenceAlterStatement` MUST render `ALTER SEQUENCE [IF EXISTS ]<name>[ AS
>   <type>][ <options>][ RESTART[ WITH <n>]][ OWNED BY <owner>]` with at least
>   one clause after the name. `Sequence::alter` yields a `PendingSequenceAlter`
>   and only choosing a clause on it — `as_type`, `options`, `restart`,
>   `restart_with`, `owned_by`, `owned_by_none` — produces a statement, on which
>   the same clauses chain: `ALTER SEQUENCE "s"` alone is a syntax error, so it
>   MUST NOT construct (`[dec:pgorm:invalid-states-unrepresentable]`), as
>   `PendingTableAlter` and `PendingTypeAlter` do for theirs.
> - `SequenceDropStatement` MUST render `DROP SEQUENCE [IF EXISTS ]<name>[,
>   <name>...][ CASCADE| RESTRICT]`. `Sequence::drop` takes the first name and
>   `name()` appends, so the list is non-empty; `cascade()` and `restrict()`
>   share one slot, the last call winning.
> - `SequenceRenameStatement` MUST render `ALTER SEQUENCE <name> RENAME TO
>   <new>`.
>
> Every statement renders through `Display` alone: a sequence carries names
> and integers and nothing to bind (`[spec:pgorm:req:sql.ddl]`).
>
> The options are one vocabulary for every position that takes them, a
> standalone sequence's and an identity column's
> (`[spec:pgorm:req:sql.ddl.column-def]`): `SequenceOption` is `IncrementBy`,
> `MinValue` / `NoMinValue`, `MaxValue` / `NoMaxValue`, `StartWith`, `Cache`
> and `Cycle` / `NoCycle`, each number an `i64` written as an integer literal,
> `i64::MIN` and `i64::MAX` included. `SequenceOptions` holds one or more of
> them, at most one per clause — a bound and its `NO` form share a clause, as
> `CYCLE` and `NO CYCLE` do — because PostgreSQL refuses a clause given twice,
> and those pairs too (`42601`, *conflicting or redundant options*); an option
> for a clause already filled replaces the one there, and a second
> `options(..)` call merges in the same way. It has no empty value: it is built
> `From` its first option and grows by `and`, so an alter that takes it as its
> only clause still writes one, and an identity column writes its parentheses
> only when it has a set. Its clauses MUST render space-separated in one fixed
> order — increment, minimum, maximum, start, cache, cycle — whatever order
> they were given in.
>
> The clauses only a standalone sequence takes are methods of its statements,
> not options. `AS` is a `SequenceType` — `smallint`, `integer`, `bigint` —
> the three types PostgreSQL accepts (`22023` for any other), so the choice is
> a closed set rather than a `ColumnType`; it sets the default bounds, and an
> alter moves a bound that sat at the old type's limit to the new one's.
> `RESTART [WITH n]` is the alter's. `OWNED BY` takes a table and a column, so
> the bare column PostgreSQL refuses (`42601`, *invalid OWNED BY option*) does
> not construct; dropping the column or its table then drops the sequence, and
> `owned_by_none()` writes `OWNED BY NONE`, which releases it. A table in
> another schema than the sequence's is the server's refusal (`55000`).
>
> The numbers are judged by the server, not by the type: a zero step, a cache
> below one, bounds that cross, a start outside them and a bound outside the
> counting type are each refused with `22023`, and each is checked against the
> rest of the definition — including what an alter leaves unchanged — which no
> single option can see. A `NO CYCLE` sequence past its bound fails at
> `nextval` (`2200H`). The live suite holds those refusals, and that a
> sequence the builder creates or alters hands out the values its options say.
>
> Four spellings are not built, each a whole statement through `execute`:
> `TEMPORARY` / `UNLOGGED` and `SET { LOGGED | UNLOGGED }` are persistence
> choices of the kind `[spec:pgorm:req:sql.scope]` rules out for tables;
> `OWNER TO` is a role change, ruled out with privileges; and `SET SCHEMA` has
> no table counterpart in the builder either.

## Views

> [spec:pgorm:req:sql.ddl.view+1]
> `View` builds the statements a view takes: `create(name, body)`,
> `create_recursive(name, column, body)` and `drop(name)`. A view is a
> relation, so it is named as a table is — any `IntoTableName`, a bare name or
> a `(schema, name)` pair, rendered as quoted, dot-joined identifiers — and its
> body is a `SelectStatement` the constructor takes by value, as every
> embedder takes what it embeds (`[spec:pgorm:req:sql.ddl.create-table]`),
> and checks as it takes it.
> Name and body are constructor arguments with no setter, because `CREATE
> VIEW  AS` and a view without a query are syntax errors, so neither husk
> constructs (`[dec:pgorm:invalid-states-unrepresentable]`). A view of an
> entity's query takes the statement `QueryTrait::into_query` gives.
>
> - `ViewCreateStatement` MUST render `CREATE [OR REPLACE ]VIEW <name>[
>   (<columns>)][ WITH (<options>)] AS <query>[ WITH { LOCAL | CASCADED }
>   CHECK OPTION]`.
> - `RecursiveViewCreateStatement` MUST render `CREATE [OR REPLACE ]RECURSIVE
>   VIEW <name> (<columns>)[ WITH (<options>)] AS <query>`.
> - `ViewDropStatement` MUST render `DROP VIEW [IF EXISTS ]<name>[,
>   <name>...][ CASCADE | RESTRICT]`. `View::drop` takes the first name and
>   `name()` appends, so the list is non-empty, and `cascade()` and
>   `restrict()` share one `DropBehavior` slot
>   (`[spec:pgorm:req:sql.ddl.alter-table]`), the last call winning.
>   `names()` reads the list back.
>
> The body is the caller's query with its values inlined. PostgreSQL stores a
> view's definition as SQL text, so a definition takes no bind parameter, and
> the body renders through the value-inlining sink every DDL statement
> renders through (`[spec:pgorm:req:sql.ddl]`): each value an escaped
> literal, a string with a quote or a backslash in its `E''` form
> (`[spec:pgorm:req:sql.render.string-escape]`). The live suite reads the
> stored definition back with the value in it.
>
> `create` and `create_recursive` MUST refuse a body whose tie limit
> PostgreSQL's grammar refuses — one with no ORDER BY, or beside `SKIP
> LOCKED` (`sql.ast.select.tie-limit`) — and a body with a WINDOW clause, or a
> reference to one, that it refuses (`sql.ast.window-statement`), with
> pgorm-query's error for it, so each returns a `Result` and a view over such
> a body never constructs. The constructor is the one boundary every body
> passes on its way to the server. Rendering is infallible, a DDL statement
> is sent as the text it renders, through `execute` or `batch_execute`, with
> no executor that could run the checks first as the `SELECT` executors and
> `copy_out` run them, and the body has no setter once taken, so the query
> checked is the one that renders: the refused state cannot be designed out,
> and the failure is a typed error at the nearest fallible boundary
> (`[dec:pgorm:invalid-states-unrepresentable]`, `[dec:pgorm:no-panic]`). The
> Python and JavaScript builders refuse the same bodies as they read them. The
> checks read the body's own clauses, as they do on their own: a `SELECT`
> nested in the body is the server's to refuse. The oracle rejects an
> unordered tie limit inside `CREATE VIEW` as it does in a bare `SELECT`
> (`42601`), and the live suite holds each refusal coming back from a
> statement built and sent in a transaction as `Error::QueryBuilder`, the
> transaction left unaborted because nothing reached the server, the server
> refusing the same body written into a view, a materialized view and a
> table by hand (`42601`, `42704`, `42P20`), and the corrected body
> constructing and being taken.
>
> Column names are `Name`s appended by `column()` and `columns()` and read
> back by `get_columns()`. With none, no parentheses are written, because the
> grammar has no empty column list (`42601`). They rename the query's leading
> columns. More names than the query has columns is refused by the server
> (`42601`), because the query's arity is not something a builder value
> records once a `*` or a subquery is in it.
>
> A recursive view is PostgreSQL's shorthand for the view of a `WITH
> RECURSIVE` query. Its body is the non-recursive term `UNION [ALL]` the
> recursive one, which reads the view by name, and the grammar rewrites the
> statement into that query, so the oracle reads the column names back as the
> common table expression's. The column list is what that query's rows are
> called, and the grammar refuses a recursive view without one (`42601`), so
> `create_recursive` takes the first column and the list is never empty. The
> grammar also refuses a check option on a recursive view (`0A000`, *WITH
> CHECK OPTION not supported on recursive views*). The two forms are
> therefore separate types, and `RecursiveViewCreateStatement` has no
> `check_option`, which a `compile_fail` doctest holds
> (`[dec:pgorm:invalid-states-unrepresentable]`). Neither has `IF NOT
> EXISTS`, a clause PostgreSQL does not have for a view (`42601`); `OR
> REPLACE` is how a view that may already exist is defined.
>
> `or_replace()` writes `OR REPLACE`. The server holds the replacement to the
> view it replaces: the new query keeps every column the view has, in place,
> with its name and type, and may add columns after them, while a column
> moved, dropped, renamed or retyped is refused (`42P16`). That turns on the
> view that exists, which is the server's knowledge. A replacement also
> replaces the view's options and check option with its own, so one it
> leaves out is reset, as the live suite holds through `pg_class.reloptions`.
>
> The options are two flags, `security_barrier()` and `security_invoker()`,
> each written only when set, in that order, inside one `WITH (...)`. An
> unset flag is the server's default, false, which therefore has no
> spelling.
>
> - `security_barrier` evaluates the view's own conditions before any a
>   query over it supplies, so a cheap function the query passes never sees
>   a row the view hides. The live suite holds a function that fails on the
>   hidden row: it fails through the plain view and passes through the
>   barrier one.
> - `security_invoker` checks the relations the view reads against the role
>   querying it rather than the view's owner. The live suite holds a role
>   granted the view alone reading through the plain view and refused through
>   the invoker one (`42501`).
>
> `check_option(ViewCheckOption)` sets the check option, one slot the last
> call fills, read back by `get_check_option()`: `Local` writes `WITH LOCAL
> CHECK OPTION` and `Cascaded` writes `WITH CASCADED CHECK OPTION`. A row that
> an insert or update writes through the view must be one the view can see,
> or the server refuses it (`44000`). `LOCAL` holds the row to this view's
> condition alone, and `CASCADED` to the conditions of the views beneath it
> as well, as the live suite holds over a view of a view. The bare `WITH
> CHECK OPTION` means `CASCADED` and has no spelling of its own. Only an
> automatically updatable view takes a check option — a query over one table
> or updatable view with no aggregate, `DISTINCT`, `GROUP BY`, `HAVING`,
> `LIMIT`, `OFFSET`, set operation or `WITH` — and which views those are is
> the server's to judge (`0A000` otherwise).
>
> An entity's table may be a view. The entity is declared as any other is,
> naming the view as its table, and reads the rows the view shows. Through
> an automatically updatable view, its inserts, updates and deletes reach the
> table beneath: the table's defaults and identity fill what an insert
> leaves out, `RETURNING` reads the written row back, and a write the view's
> condition does not reach, such as a delete of a row the view hides,
> affects nothing. The live suite holds each of these, and a check option
> refusing an entity's insert and update (`44000`). A write through a view
> the server cannot update is refused there (`55000`). Nothing in an entity
> says its table is a view, so `Schema::create_table_from_entity` over one
> builds a table, and the view itself is created through `View::create`.
>
> Two spellings are not built, each a whole statement through `execute`:
>
> - `TEMPORARY` views, a persistence choice of the kind
>   `[spec:pgorm:req:sql.scope]` rules out for tables.
> - `ALTER VIEW`. A view's definition and options change through `OR
>   REPLACE`. Its name, its columns' names and the default an insert through
>   it takes change through `Table::rename`, `Table::rename_column` and
>   `Table::alter`'s `modify_column`, whose `ALTER TABLE` PostgreSQL accepts
>   on a view, as the live suite holds. Its owner is a role's concern, and
>   `SET SCHEMA` has no table counterpart in the builder either.

## Materialized views

> [spec:pgorm:req:sql.ddl.materialized-view+1]
> `MaterializedView` builds the statements a materialized view takes:
> `create(name, body)`, `refresh(name)` and `drop(name)`. Each names the view
> as a table is named, and the create takes its body by value, refuses the
> bodies a view's constructor refuses, returning a `Result`, and writes it
> with its values inlined, as a view does (`[spec:pgorm:req:sql.ddl.view]`).
>
> - `MaterializedViewCreateStatement` MUST render `CREATE MATERIALIZED VIEW
>   [IF NOT EXISTS ]<name>[ (<columns>)] AS <query>[ WITH NO DATA]`.
> - `MaterializedViewRefreshStatement` MUST render `REFRESH MATERIALIZED VIEW
>   [CONCURRENTLY ]<name>[ WITH NO DATA]`.
> - `MaterializedViewDropStatement` MUST render `DROP MATERIALIZED VIEW [IF
>   EXISTS ]<name>[, <name>...][ CASCADE | RESTRICT]`, its list and behavior
>   held as a view drop holds them.
>
> A materialized view holds the rows its query returned when it was last
> populated. The create runs the query and stores its rows; `with_no_data()`
> leaves the view unpopulated instead, and the server refuses to read it
> (`55000`) until a refresh populates it. `WITH DATA` is the default and has
> no spelling of its own, as an unset clause writes nothing elsewhere in the
> builder. Column names are appended by `column()` and `columns()` and read
> back by `get_columns()`, renaming the query's leading columns, with more
> than the query has refused (`42601`). `if_not_exists()` leaves an existing
> relation of the name alone, its rows with it, with a notice (`42P07`).
> PostgreSQL 18 has no `OR REPLACE` for a materialized view (`42601`), and
> the statement has none, which a `compile_fail` doctest holds: a
> materialized view that changes is dropped and created again.
>
> A refresh runs the query again and replaces the view's rows. Unqualified,
> it holds a lock that blocks reads of the view until it commits.
> `concurrently()` replaces the rows without blocking reads, by comparing
> the old rows with the new, and `with_no_data()` empties the view and leaves
> it unpopulated. PostgreSQL refuses `CONCURRENTLY` beside `WITH NO DATA`
> (`42601`), but at execution, after the grammar has accepted the pair, so
> the oracle cannot keep it out. The two are therefore one slot the last call
> fills, read back by `is_concurrently()` and `is_with_no_data()`, and the
> pair has no render (`[dec:pgorm:invalid-states-unrepresentable]`).
>
> What a concurrent refresh needs besides is the server's knowledge, and the
> live suite holds each refusal: a unique index on the view over plain
> columns with no `WHERE` (`55000` without one), which `Index::create`
> builds over the view's name, and a view already populated (`0A000`).
> Unlike `CREATE INDEX CONCURRENTLY` (`[spec:pgorm:req:sql.ddl.index-create]`),
> a concurrent refresh runs inside a transaction block, and the live suite
> runs one there.
>
> An entity's table may be a materialized view, as it may be a view, and the
> entity reads the rows the view holds. The live suite holds an entity
> reading a refreshed view, not seeing a row added to the table until the
> next refresh, and refused a write, a materialized view not being
> updatable (`42809`). This drop refuses a view and `DROP VIEW` refuses a
> materialized view (`42809`).
>
> Two spellings are not built:
>
> - `USING <access method>`, `WITH (<storage options>)` and `TABLESPACE`,
>   storage choices `[spec:pgorm:req:sql.scope]` rules out for tables.
> - `ALTER MATERIALIZED VIEW`, whose rename and column rename
>   `Table::rename` and `Table::rename_column` reach, as for a view.

## Tables from a query

> [spec:pgorm:req:sql.ddl.create-table-as+1]
> `Table::create_as(name, body)` builds a `TableCreateAsStatement`, which
> MUST render `CREATE TABLE [IF NOT EXISTS ]<name>[ (<columns>)] AS <query>[
> WITH NO DATA]`. It creates a table whose columns are the query's, with
> their names and types, and whose rows are the query's rows when the
> statement runs, or none under `with_no_data()`. The names `column()` and
> `columns()` give rename the leading columns, and `get_columns()` reads them
> back; more than the query has are refused (`42601`). The name and the body
> are constructor arguments, and the body is taken by value, refused when a
> view's constructor refuses it, so `create_as` returns a `Result`
> (`[spec:pgorm:req:sql.ddl.view]`), and written with its values inlined,
> as every DDL statement writes a value (`[spec:pgorm:req:sql.ddl]`).
> `if_not_exists()` leaves an existing relation alone, its rows with it.
>
> From then on it is a table like any other, which the live suite holds: its
> kind is `r`, a change to the table the query read does not reach it, and
> rows inserted into the empty one are kept. Nothing ties it to the query
> again, which is what separates it from a materialized view
> (`[spec:pgorm:req:sql.ddl.materialized-view]`).
>
> Three spellings are not built:
>
> - `TEMPORARY` and `UNLOGGED`, `ON COMMIT`, `USING`, `WITH (<storage
>   options>)` and `TABLESPACE`, ruled out for tables by
>   `[spec:pgorm:req:sql.scope]`.
> - `SELECT ... INTO`, another spelling of this same statement.
> - `CREATE TABLE ... AS EXECUTE`, which names a prepared statement that no
>   builder makes.

## Schemas

> [spec:pgorm:req:sql.ddl.schema]
> `Schema` builds the statements a schema takes: `create(name)`, `drop(name)`
> and `rename(from, to)`. A schema is named by one identifier, so every name
> is an `IntoName` rendered as a quoted identifier, never a `(schema, name)`
> pair: schemas do not nest, and the grammar refuses a qualified schema name.
>
> - `SchemaCreateStatement` MUST render `CREATE SCHEMA [IF NOT EXISTS ]<name>`.
> - `SchemaDropStatement` MUST render `DROP SCHEMA [IF EXISTS ]<name>[,
>   <name>...][ CASCADE| RESTRICT]`. `Schema::drop` takes the first name and
>   `name()` appends, so the list is non-empty; `cascade()` and `restrict()`
>   share one `DropBehavior` slot, the last call winning.
> - `SchemaRenameStatement` MUST render `ALTER SCHEMA <name> RENAME TO <new>`.
>   PostgreSQL's grammar has no `IF EXISTS` for it (`42601`), so the statement
>   MUST NOT offer one (`[dec:pgorm:invalid-states-unrepresentable]`).
>
> Every statement renders through `Display` alone: a schema statement carries
> names and nothing to bind (`[spec:pgorm:req:sql.ddl]`). A schema is
> created inside a transaction and undone with it, as the rest of the DDL is.
>
> What the server refuses, no builder value can see, and the live suite holds
> each refusal by SQLSTATE: a create of a schema that exists (`42P06`, a
> notice under `IF NOT EXISTS`); a name starting `pg_`, which PostgreSQL
> reserves for its own schemas, as a created schema or a rename's new name
> (`42939`); a rename onto a name already taken (`42P06`); a drop or rename
> of a schema that does not exist (`3F000`, the drop's a notice under `IF
> EXISTS`); and a drop of a schema that holds anything, under `RESTRICT`
> said or unsaid (`2BP01`), where `CASCADE` drops what it holds. The suite
> builds a table, an enum type and a sequence inside a schema the builder
> created, each qualified by the schema's name, and an entity whose
> `schema_name` names that schema reads and writes its table there.
>
> Three spellings are not built. `AUTHORIZATION` and `ALTER SCHEMA … OWNER
> TO` name a role, which `[spec:pgorm:req:sql.scope]` rules out with
> privileges. The schema-element list — `CREATE SCHEMA s CREATE TABLE …` —
> creates objects every builder here already creates qualified by the
> schema's name, and PostgreSQL refuses it beside `IF NOT EXISTS` (`0A000`),
> the clause a migration reaches for.

## Domains

> [spec:pgorm:req:sql.ddl.domain+3]
> `Domain` builds the statements a domain takes: `create(name, base)`,
> `alter(name)` and `drop(name)`. A domain is a type, so each name is an
> `IntoTypeRef` — a bare name or a `(schema, name)` pair — rendered as
> `TypeRef`'s quoted, dot-joined parts, as an enum type's is
> (`sql.ddl.type-enum`). The base is a `ColumnType`, spelled as a column's
> type is (`sql.ddl.column-types`), another domain or an array included.
>
> - `DomainCreateStatement` MUST render `CREATE DOMAIN <name> AS <base>[
>   COLLATE <collation>][ DEFAULT <expr>][ NOT NULL| NULL][ <check>...]`, the
>   clauses in that order whatever order they were given in and the checks in
>   the order added. A default and a nullability are one slot each, the last
>   call winning, because PostgreSQL refuses two defaults and a `NULL` beside a
>   `NOT NULL` (`42601`). The default sits where the grammar reads a `b_expr`,
>   which has no logical operator, `IS`, `BETWEEN`, `IN` or `LIKE`, so a
>   `Unary`, `Binary`, `Raw`, `Template` or `LikePattern` default MUST render
>   parenthesised and any other expression bare.
> - `Domain::alter` yields a `PendingDomainAlter`, and only choosing an action
>   on it produces a `DomainAlterStatement`, which carries exactly that
>   action: PostgreSQL has no `ALTER DOMAIN` without one and no list of them
>   (`42601`), so neither MUST construct
>   (`[dec:pgorm:invalid-states-unrepresentable]`). It MUST render `ALTER
>   DOMAIN <name> ` followed by one of `SET DEFAULT <expr>`, `DROP DEFAULT`,
>   `SET NOT NULL`, `DROP NOT NULL`, `ADD <check>[ NOT VALID]`, `DROP
>   CONSTRAINT [IF EXISTS ]<name>[ CASCADE| RESTRICT]` (a `ConstraintDrop`,
>   `sql.ddl.alter-table`), `RENAME CONSTRAINT <name> TO <new>`, `VALIDATE
>   CONSTRAINT <name>` or `RENAME TO <new>`, the new name a bare `Name`
>   because a rename leaves a domain in its schema (`42601` for a qualified
>   one). `SET DEFAULT` reads the full expression grammar and renders its
>   expression bare. `ALTER DOMAIN` has no `IF EXISTS` (`42601`), so neither
>   type offers one.
> - `DomainDropStatement` MUST render `DROP DOMAIN [IF EXISTS ]<name>[,
>   <name>...][ CASCADE| RESTRICT]`. `Domain::drop` takes the first name and
>   `name()` appends, so the list is non-empty; `cascade()` and `restrict()`
>   share one `DropBehavior` slot, the last call winning.
>
> A check renders `[CONSTRAINT <name> ]CHECK (<expr>)`. It is a `DomainCheck`
> — its expression, which the constructor takes, and an optional name — or
> an expression, the unnamed check over it (`IntoDomainCheck`); unnamed,
> PostgreSQL names it `<domain>_check`. It is not the table's `Check`, which
> carries a `NO INHERIT` and an enforcement that a domain's check refuses
> (`42P17` at `CREATE DOMAIN`, `0A000` at `ALTER DOMAIN`). `DomainCheck::
> not_valid` wraps it in the `NotValid` that `add_check` takes beside a
> plain check (`sql.ddl.alter-table`), leaving the values columns of the
> domain already hold unchecked until `VALIDATE CONSTRAINT`;
> `DomainCreateStatement::check` takes no `NotValid`, the grammar having no
> `NOT VALID` at `CREATE DOMAIN` (`42601`). The value a check tests is
> `Expr::domain_value()`, `Keyword::Value`, which renders `VALUE`
> (`sql.ast.keywords`): PostgreSQL reads that name inside a domain's check as
> the value tested and anywhere else as a column named `value`.
>
> Every statement renders through `Display` alone, a default and the values a
> check compares with inlined (`[spec:pgorm:req:sql.ddl]`), and is undone
> with the transaction it runs in. A column whose type is a domain is
> `ColumnType::Domain` (`sql.types.column-type`), which renders the domain's
> name through `TypeName`'s part policy alone, as an array's element and as
> another domain's base.
>
> What the server refuses, no builder value can see, and the live suite holds
> each refusal by SQLSTATE: a domain name a type already has, created or
> renamed onto (`42710`); a second constraint of one name, added or renamed
> onto (`42710`); a missing domain or constraint (`42704`, a notice under a
> drop's `IF EXISTS`); a value outside a check (`23514`) and a null under `NOT
> NULL` (`23502`), whether written to a column of the domain or met among the
> values already there by `ADD CHECK`, `SET NOT NULL` or `VALIDATE
> CONSTRAINT`; a subquery in a default or a check (`0A000`); a `COLLATE` on a
> base type with no collation (`42804`); and a drop under `RESTRICT`, said or
> unsaid, of a domain a column is typed with (`2BP01`), where `CASCADE` drops
> the column too. A `NOT VALID` check holds every value written after it and
> leaves the rest, which its `VALIDATE CONSTRAINT` then refuses with `23514`.
> The suite also has an entity whose columns are domains read and write
> through them, a check's refusal reaching the caller as pgorm's typed error.
>
> PostgreSQL's plan cache does not see a domain's default change. An insert
> that leaves a column of the domain out is planned with the domain's default
> in it, and `ALTER DOMAIN … SET DEFAULT` or `DROP DEFAULT` invalidates no
> plan, so a statement a connection prepared before the change keeps writing
> the default it was planned with — and pgorm prepares each statement once per
> pooled connection and reuses it — while the same statement prepared after
> it, or sent unprepared, writes the new one. Only a re-plan for another
> reason ends it, and none can be counted on: a reset of the session's caches
> re-plans every statement the session keeps prepared, under its old name,
> and comes with `DISCARD PLANS`, with a schema created in its database, or
> with the session falling behind the server's shared invalidation queue,
> which catalogue churn in any database on the server causes. The stale
> default is therefore intermittent, and no proof that a pool kept its
> statements, nor the new default proof that it cleared them: the suite tells
> a clear by the statement a session holds, its name and preparation time in
> `pg_prepared_statements`, which a clear closes and a re-plan keeps. A check
> or a `NOT NULL` added at the same point holds at once, being tested as each
> value is written; the suite holds both on 18.6, the old default whenever
> the session is shown not to have re-planned, by an immutable function the
> planner folded into a plan of its own.
> `DatabasePool::clear_statement_caches`
> (`conn.pool.statement-cache.clear`) is the remedy: after it, a connection
> that already ran the insert prepares it again once it is next lent and
> writes the new default, which the suite holds too. pgorm-migration's `up`
> calls it once a batch that applied a migration commits (`migration.up`), so
> a default a migration changed reaches the pool's connections with nothing
> further asked of the application; DDL run any other way is followed by the
> call.
>
> Three spellings are not built. `ALTER DOMAIN … SET SCHEMA` the builder has
> for no table, sequence or type either, and `OWNER TO` is a role change,
> which `[spec:pgorm:req:sql.scope]` rules out with privileges. A named
> `NOT NULL` — `CONSTRAINT <name> NOT NULL`, or one added with `ADD
> CONSTRAINT` — adds nothing a caller needs: `not_null()` and
> `set_not_null()` write the same constraint, which PostgreSQL names
> `<domain>_not_null`, and `drop_not_null()` drops it by no name.

## Extensions

> [spec:pgorm:req:sql.ddl.extension+5]
> `ExtensionCreateStatement` MUST render `CREATE EXTENSION [IF NOT EXISTS ]
> <name>[ WITH SCHEMA <schema>][ VERSION <version>][ CASCADE]`, and
> `ExtensionDropStatement` MUST render `DROP EXTENSION [IF EXISTS ]<name>
> [ CASCADE| RESTRICT]`. The name is a `Name` taken by
> `Extension::create(name)` / `Extension::drop(name)` and has no setter: it used
> to default to the empty `String`, which renders as the zero-length delimited
> identifier `""` PostgreSQL rejects, so a statement that never names an
> extension MUST NOT construct
> (`[dec:pgorm:invalid-states-unrepresentable]`). An explicitly empty
> identifier remains the caller's own to avoid, as `Name::runtime("")` is
> everywhere else in the crate. The schema is a `Name` set by
> `schema(impl IntoName)` — a schema qualifier is a name, and it takes the
> identifier type every other schema position in the crate takes — while the
> version stays a `String`, because the grammar puts a string literal there
> and text is what it is. None of the three is written verbatim: name and
> schema render as quoted identifiers and version as a quoted string literal
> (`[spec:pgorm:sem:sql.render.ddl.extension]`). On drop, `CASCADE` and
> `RESTRICT` share one `ExtensionDropOpt` slot that `cascade()`/`restrict()`
> overwrite, so the pair PostgreSQL rejects does not construct; a drop carries
> no schema or version, because it renders neither.
> `PgLTree` is a ready-made `SqlName` rendering `ltree` (usable directly as an
> extension name); the ltree column type itself is `ColumnType::LTree`.

## Panics and unsupported forms

> [spec:pgorm:sem:sql.ddl.panics+4]
> DDL building does not panic. Every `prepare_*` path over a constructible
> statement runs to a rendered string, so the absence of a `Result`-returning
> DDL build path costs a caller nothing: there is no failure for one to carry.
> The last edge was the empty `TableAlterStatement`, which panicked with
> `No alter option found`; it is closed by construction rather than converted to
> an error, because a statement with no action is not a statement
> (`[spec:pgorm:req:sql.ddl.alter-table]`). It MUST NOT come back, in that
> form or as a `Result`.
>
> Column type and auto-increment shape were panics of their own, and MUST NOT
> come back either. `ColumnType` carries no variant without a Postgres spelling
> — `Year` is gone with the enum entry that produced the `Year is not
> available in Postgres.` panic — so `prepare_column_type` is total. The
> serial substitution is likewise total: `auto_increment()` on a type outside
> the integer trio renders the declared type rather than panicking with
> `... doesn't support auto increment`
> (`[spec:pgorm:req:sql.ddl.column-def]`). Neither guard is a `Result`; both
> are closed by making the renderer's match exhaustive over spellings that
> exist.
>
> Table reference shape was the third, and MUST NOT come back
> either. Table statements (`create`/`alter`/`rename`/`drop`/`truncate`), index
> and foreign-key targets and comment targets take a `TableName`
> (`[spec:pgorm:def:sql.types.table-ref]`), which has no form the renderer
> could refuse. The five `Not supported` panics and the `TableRef with values
> is not support` panic that guarded these positions are gone, and a caller
> cannot reintroduce them: binding an alias makes a reference a `NamedTable`,
> and a subquery, values-list or function-call reference is a `FromItem`;
> neither typechecks as a DDL target.
