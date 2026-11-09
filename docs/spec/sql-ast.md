# SQL AST (pgorm-query statement and expression tree)

pgorm-query models SQL statements as plain Rust data structures — an abstract
syntax tree built through fluent, mutating builder methods — which are rendered
to PostgreSQL text by the backend `QueryBuilder` only when a build method is
called. This document specifies the AST layer: the statement types under
`pgorm-query/src/query/`, the expression tree in `pgorm-query/src/expr.rs`, and
function calls in `pgorm-query/src/func.rs`. Rules capture what the code does
today, including panicking edges and deliberate failsafes.

## Overview

> [spec:pgorm:req:sql.ast+4]
> pgorm-query MUST provide a programmatic AST for building SQL statements,
> comprising `SelectStatement`, `InsertStatement`, `UpdateStatement`,
> `DeleteStatement` and `MergeStatement`, plus the `Query` shorthand whose
> associated functions (`Query::select()`, `Query::insert()`, `Query::update()`,
> `Query::delete()`, `Query::merge(..)`, `Query::with()`, `Query::returning()`)
> construct fresh builders. `Query::merge` returns a `PendingMerge`, which
> becomes a statement when its first arm is added (`sql.ast.merge`). Builder
> methods MUST mutate in place and return `&mut Self` so calls chain;
> constructing or mutating a statement MUST NOT touch a database.
>
> The five statement kinds MUST be embeddable as a subquery through
> `IntoSubQueryStatement::into_sub_query_statement`. It wraps the statement
> in the `SubQueryStatement` enum (`SelectStatement`, `InsertStatement`,
> `UpdateStatement`, `DeleteStatement`, `MergeStatement`), which is what a
> common table expression and an expression subquery hold, and which
> displays as the statement it wraps, values inlined as that statement's
> `Display` writes them, so a dependent renders one without matching it.
> Every kind is a CTE body on PostgreSQL 18. A MERGE body yields the rows of its RETURNING
> list, and without one it still runs, as an UPDATE does, while reading the
> CTE is refused (`0A000`); the AST leaves that to the server as it does for
> the other write statements (`sql.ast.with`). Only a SELECT nests as an
> expression subquery: the server refuses the four write statements there
> (`42601`), and every typed constructor of an expression subquery
> (`Expr::exists`, `Expr::any`, `in_subquery` and their kin) takes a
> `SelectStatement`, so only the raw `SimpleExpr::SubQuery` variant can hold
> one. Nesting is a trait of its own rather than a method of
> `QueryStatementBuilder`, because what nests is a narrower question than
> what renders.
> There is no variant for a WITH-prefixed statement: a clause is carried
> by the statement it prefixes (`query.build.with`), so the prefix nests
> wherever the statement does.
>
> Where a builder offers `take()`, the method MUST move the accumulated contents
> out and leave the source in its default (empty) state — it may copy only the
> identity field a constructor demanded, never content a caller added. A type
> whose contents cannot be moved out without leaving an invalid value therefore
> has no `take()` at all rather than a `take()` that copies: `SelectStatement`,
> `WindowStatement`, `TableIndex`, `ColumnDef`, `TableCreateStatement` and
> `TableDropStatement` have one; `IndexCreateStatement`,
> `ForeignKeyCreateStatement`, `TableForeignKey`, `TableAlterStatement` and
> `MergeStatement` do not, and a caller who wants a second copy of one writes
> `.to_owned()`.

> [spec:pgorm:req:sql.surface+40]
> The crate's exports are an explicit list, not a set of module globs.
> `pgorm-query/src/lib.rs` MUST name every exported item in `pub use` statements
> grouped by what the items are for — names, expressions, values, query
> statements, COPY, schema statements, rendering — and the modules those items
> are defined in MUST be private, so `use pgorm_query::*` and that list are the
> same set and nothing becomes API by being declared `pub` inside a module a
> caller can reach. Exactly three modules stay public, each because a path through it
> is the spelling callers write: `error` (`Error`, `Result`, `TemplateError`),
> `extension` (the `CREATE EXTENSION` / `CREATE TYPE` surface, deliberately not
> flattened into the root) and `value` (whose `value::with_array::NotU8` pgorm's
> derives name in generated code). `tests_cfg` is `#[doc(hidden)]` behind its
> own feature and is not surface. No module inside the three is public but
> `value::with_array`, the path that spelling takes, and `value` exports
> nothing the list does not.
>
> What the crate reads but a caller cannot use is `pub(crate)`: the renderer's
> `Mode` and `Oper` helpers, the clause shapes a statement holds internally
> (`JoinExpr`, `JoinKind`, `JoinOn`, `LockClause`, `ConditionHolder`), and
> `prepare`'s re-export of `std::fmt::Write`, which put a `std` trait in
> pgorm-query's namespace and was the only path by which `pgorm_query::Write`
> could be written. An item that narrowing would merely turn into dead code is
> instead exported `#[doc(hidden)]`, so the rendered documentation is the
> curated list while the item stays reachable: the `SelectDistinct` flag,
> whose `All` variant no builder sets, and the `Token`/`Tokenizer` lexer behind
> `inject_parameters`, whose classification accessors only the conformance
> suite calls. Each of those MUST be named
> individually in `lib.rs` rather than left to a glob, so the residue is a list
> a later pass can work through rather than a category.
>
> A public enum is `#[non_exhaustive]` when its variants name members of a
> vocabulary that can grow past what the enum holds: what PostgreSQL defines —
> operators, functions, types and values, statement kinds, the clauses,
> actions, options and behaviours a statement takes, the objects a statement
> names — and the kinds of failure a crate reports. Adding a member to one, as
> PostgreSQL 19 (`sql.target`) or wider coverage of 18 does, is then not a
> breaking change. That is `SimpleExpr`, `Function`, `UnOper`, `BinOper`,
> `SubQueryOper`, `Keyword`, `ColumnRef`, `FromItem`, `SubQueryStatement`,
> `Value`, `ArrayType`, `RangeType`, `ColumnType`, `JsonValueType`, `SqlJson`,
> `JsonExistsBehavior`, `JsonValueBehavior`, `JsonQueryBehavior`,
> `JsonTableBehavior`, `Order`, `UnionType`, `LockType`, `LockBehavior`,
> `FrameType`, `FrameExclusion`, `SampleMethod`, `OnConflict`,
> `ConflictAction`, `ConflictAssignment`, `MatchedAction`, `NotMatchedAction`,
> `TableAlterOption`, `ColumnSpec`, `ConstraintChange`, `ForeignKeyAction`,
> `IndexType`, `SequenceType`, `SequenceOption`, `CommentTarget`,
> `CopyLogVerbosity`, `Token`, `ValueTupleError`, `error`'s `Error` and
> `TemplateError`, and `extension`'s `TypeAs`, `TypeAlterOpt` and
> `AttributeChange`. Three are open for a reason beyond their present
> variants: `Order` lacks `USING`'s operator sort, `SequenceType` lists the
> three types PostgreSQL chooses to count in rather than all a count could be,
> and `OnConflict` pairs a conflict's arbiter with an action whose vocabulary
> grows. An enum stays exhaustive when its variants partition a choice by
> construction, so that another would change what it means rather than extend
> it and a caller is meant to match every arm: the two sides of a binary
> choice (`NullOrdering`, `IndexOrder`, `ReturningRow`, `Overriding`,
> `IdentityGeneration`, `GeneratedKind`, `Enforcement`, `DropBehavior`,
> `TableDropOpt`, `ConditionType`, `IndexKind`, `ViewCheckOption`,
> `SearchOrder`, and `extension`'s `ExtensionDropOpt`, `TypeDropOpt` and
> `TypeAlterAddOpt`), every combination of independent settings PostgreSQL
> acts on (`Deferrability`), the alternatives of a shape (`TableName`,
> `Subscript`, `IndexColumnTarget`, `ConflictElement`, `ConflictArbiter`,
> `ConditionExpression`, `AnyWithClause`, `WindowSelectType`,
> `WindowDefinition`, `SelectDistinct`, `StringLen`, `IntervalSpec`, `Range`,
> and `extension`'s `TypeRef`), a set fixed by what it measures
> (`IntervalPrecision`, the microsecond's six digits; `PgInterval`, the field
> ranges an interval has; `JsonKind`, the values JSON defines and their union),
> `JoinType`, whose variants are the four answers to which sides keep their
> unmatched rows, and the uninhabited frame-side markers `FramePreceding`,
> `FrameCurrentRow` and `FrameFollowing`. A match on an open
> enum outside the crate ends in an arm for a member it does not know, and in
> this workspace that arm refuses it with the matching crate's own typed error
> — pgorm's, codegen's `TransformError`, a binding's error class — rather than
> reading it as a member it knows. pgorm, pgorm-pool, pgorm-migration and
> pgorm-codegen hold their own public enums to the same test, each recorded
> with the rule that specifies it.
>
> The list grows only by a named item that a rule specifies, and so does
> `extension`; each addition to either is recorded here with that rule:
> `Deferrability`, when a constraint's check
> runs (`sql.ddl.deferrability`); `Overriding`, the `OVERRIDING SYSTEM VALUE`
> or `OVERRIDING USER VALUE` an insert carries (`sql.ast.insert`);
> `IdentityGeneration`, whether an identity column is generated `ALWAYS` or
> `BY DEFAULT` (`sql.ddl.column-def`); `IndexColumnTarget`, whether an index
> entry indexes a column or an expression (`sql.ddl.index-create`);
> `Interval`, an interval value as PostgreSQL's months, days and microseconds
> (`sql.value.interval`);
> `CaseOperand` and `SimpleCaseStatement`,
> the simple form of `CASE` (`sql.ast.case`); `Subscript`, what an array
> subscript holds between its brackets (`sql.ast.expr.subscript`);
> `GroupingElement`, `GroupingSets` and `Grouping`, the GROUP BY items beyond
> a plain expression and the `GROUPING()` that reads them
> (`sql.ast.select.grouping`); `FrameStart`, its three side markers
> `FramePreceding`, `FrameCurrentRow` and `FrameFollowing`, and
> `FrameExclusion`, the frame builder and its `EXCLUDE` clause
> (`sql.ast.window-statement`); `ExtendedWindow` and `WindowDefinition`, a
> window built on an earlier entry of a WINDOW clause and what an entry of
> one defines (`sql.ast.window-statement`); `TableKey`,
> `IntoTableKey`, `Primary` and `Unique`, a table's key — one type for the
> primary key and the unique keys, a tuple of one or more columns that one
> column or a tuple converts into, and its two kinds — and the one table-level
> position that takes deferrability (`sql.ddl.create-table`,
> `sql.ddl.deferrability`); `Collation` and
> `IntoCollation`, the name a `COLLATE` clause carries and the conversions
> into it (`sql.ast.expr.collate`); `ConflictArbiter` and `ConflictConstraint`, the
> two ways an `ON CONFLICT` names its arbiter and the typestate of the named
> one (`sql.ast.on-conflict`); `Sequence`, `SequenceCreateStatement`,
> `PendingSequenceAlter`, `SequenceAlterStatement`, `SequenceDropStatement` and
> `SequenceRenameStatement`, the sequence statements and the typestate an
> alter starts in, with `SequenceOption`, `SequenceOptions` and
> `SequenceType`, the options a sequence and an identity column share and the
> type a sequence counts in (`sql.ddl.sequence`); `CompositeAttribute`, in
> `extension` beside the type statements, the attribute a composite type
> holds, and `CompositeAlterStatement`, `AttributeChange` and
> `AttributeRenameStatement`, there too, the changes `ALTER TYPE` makes to a
> composite's attributes and the rename that stands alone
> (`sql.ddl.type-composite`); `Range`, `Multirange`, `RangeType` and
> `RangeElement`, a range value, a multirange value, which built-in range
> type a range is, and the sealed set of types the built-ins range over
> (`sql.value.range`); `RangeSubtype`, the sealed set of types a range type a
> schema created ranges over (`sql.value.created-range`); `RangeDefinition`, in `extension` beside the type
> statements, a range type's subtype and options (`sql.ddl.type-range`);
> `MergeStatement`, `PendingMerge`, `MatchedAction`, `NotMatchedAction`,
> `MergeUpdate` and `MergeInsert`: the MERGE statement, the typestate before
> its first arm, the actions each kind of arm takes, and the assignments an
> update writes and the row an insert writes (`sql.ast.merge`);
> `IntoSubQueryStatement`, the conversion the five statements a common table
> expression can hold share (`sql.ast`); `IntoKeyColumns`, the non-empty column tuple
> — one column or a tuple of one to twelve — that a table key is built from
> and an `ON CONFLICT` target names a key with (`sql.ddl.create-table`,
> `sql.ast.on-conflict`); `GeneratedKind`, which of PostgreSQL's two kinds a
> generated column is, stored or virtual (`sql.ddl.column-def`);
> `NotNullConstraint` and `ConstraintChange`, a `NOT NULL` constraint added at
> table level and what `ALTER CONSTRAINT` changes about one that exists
> (`sql.ddl.alter-table`); `ConstraintDrop`, `IntoConstraintDrop`,
> `DropBehavior` and `ConstraintRenameStatement`, a constraint of any kind
> dropped by name, the conversion a name takes into the plain drop, whether a
> drop restricts or cascades, and the statement that renames one
> (`sql.ddl.alter-table`); `NotValid` and `IntoAddedConstraint`, a foreign
> key or `CHECK` added `NOT VALID` and what the two `ADD` actions take for one
> (`sql.ddl.alter-table`); `Check` and `IntoCheck`, a `CHECK` constraint and
> the conversion an expression takes into one (`sql.ddl.create-table`),
> `Enforcement`, whether a foreign key or `CHECK` is enforced
> (`sql.ddl.enforcement`), `ReturningRow`, which version of a written
> row a RETURNING reference reads (`sql.ast.returning`), and SQL/JSON's
> items: `SqlJson`, the expression node's payload;
> `JsonExists`, `JsonValue` and `JsonQuery`, the three query functions, with
> `JsonExistsBehavior`, `JsonValueBehavior` and `JsonQueryBehavior`, what each
> answers on an empty or failed path, and `JsonValueType`, the types
> `JSON_VALUE` returns its scalar as; `JsonObject`, `JsonArray`,
> `JsonArrayQuery`, `JsonObjectAgg`, `JsonArrayAgg`, `JsonParse` and
> `JsonSerialize`, the constructors; `JsonInput`, an operand with or without
> `FORMAT JSON`; `JsonKind` and `JsonTest`, what `IS JSON` tests
> (`sql.ast.expr.sql-json`); and
> `WindowFunction`, what `OVER` may follow (`sql.ast.window-statement`); and
> `JsonTable`, `JsonTableBehavior`, `JsonTableColumn`, `JsonValueColumn`,
> `JsonQueryColumn`, `JsonExistsColumn` and `JsonNestedColumns`: `JSON_TABLE`,
> its `ON ERROR`, and its columns of each kind (`sql.ast.json-table`);
> `View`, `ViewCreateStatement`, `RecursiveViewCreateStatement`,
> `ViewCheckOption` and `ViewDropStatement`: the view statements, the
> recursive form that takes no check option, and which views a check option
> consults (`sql.ddl.view`); `MaterializedView`,
> `MaterializedViewCreateStatement`, `MaterializedViewRefreshStatement` and
> `MaterializedViewDropStatement`, the materialized view statements
> (`sql.ddl.materialized-view`); `TableCreateAsStatement`, a table created
> from a query (`sql.ddl.create-table-as`); `Schema`,
> `SchemaCreateStatement`, `SchemaDropStatement` and `SchemaRenameStatement`,
> the statements that create, drop and rename a schema (`sql.ddl.schema`);
> `Domain`, `DomainCreateStatement`, `PendingDomainAlter`,
> `DomainAlterStatement`, `DomainDropStatement`, `DomainCheck` and
> `IntoDomainCheck`, the statements that create, alter and drop a domain, the
> alter awaiting its one action, and a domain's `CHECK` with the conversion
> into it (`sql.ddl.domain`);
> `CopyFromStatement` and `CopyToStatement`, the two COPY statements;
> `CopyFormat` and `CopyDelimitedFormat`, the sealed traits a statement's
> format type implements, and `CopyText`, `CopyCsv` and `CopyBinary`, the
> three formats; `CopyIgnore` and `CopyLogVerbosity`, what `ON_ERROR ignore`
> carries; and `CopySource` and `IntoCopySource`, what a COPY TO reads and the
> conversion into it (`sql.ast.copy`);
> `TableFunction`, a set-returning function in `FROM` with its ordinal column,
> column names and `ROWS FROM` (`sql.ast.select.table-function`);
> `TableSample`, a table read through `TABLESAMPLE`, and `SampleMethod`, the
> method that draws it (`sql.ast.select.table-sample`);
> `POSTGRES_TARGET`, the PostgreSQL release a build targets (`sql.target`);
> `base_type`, the type a value of a domain travels as, every domain
> looked through to the type at the bottom (`sql.value.base-type`);
> `ExprTrait`, the operator vocabulary every expression type implements
> (`sql.ast.expr.operators`); and `ArrayElement`, what an array parameter's
> element type is read from (`sql.value.array`). An
> item leaves the list with the state it described: `Frame` went when a
> frame's bounds stopped being one enum, the frame builder replacing it
> (`sql.ast.window-statement`), `StandaloneIndexKind` when the primary-key
> index kind it screened the standalone renderer from did
> (`sql.ddl.index-create`), and `IndexConstraint` when the key a
> column could also declare did, `TableKey` replacing it
> (`sql.ddl.create-table`), `ForeignKeyDropStatement` when the drop named
> for one constraint kind did, `ConstraintDrop` replacing it
> (`sql.ddl.alter-table`), and the statement-dispatch wrappers
> `QueryStatement`, `TableStatement`, `IndexStatement`, `ForeignKeyStatement`
> and `SchemaStatement`, once residue, when it was settled that no builder
> produced them, no renderer took them and nothing read them, nothing
> replacing them: each statement is built and rendered as itself, and they
> MUST NOT return (`sql.ddl`). What the list and `extension` held when this rule
> was first written has no entry, being the baseline the record counts from;
> everything either has exported since is recorded here.

> [spec:pgorm:req:sql.ast.build+3]
> Every statement type implements the single `QueryStatementBuilder`, whose
> `Display` supertrait carries the value-inlined rendering. There is exactly
> one method per rendering, and no rendering method takes a builder argument:
> `QueryBuilder` is a stateless unit struct, so a passed one carried no
> information, and the `*_any` family that existed only to take it by reference
> — `build_any`, `build_collect_any`, `build_collect_any_into` — is gone along
> with the `QueryStatementWriter` half of the split.
>
> `build()` MUST return the pair `(String, Values)` where
> ordinary `SimpleExpr::Value` operands are replaced by numbered PostgreSQL
> placeholders (`$1`, `$2`, ...) per `[spec:pgorm:req:sql.render.placeholders]`
> — and the corresponding values are collected in order into
> `Values`. `to_string()`, reached through `Display`, MUST render the same
> statement with all values inlined as SQL literals instead of placeholders.
> `build_collect_into(sink)` is the single required method both are written in
> terms of, and `build_collect(sink)` returns the sink's accumulated text.
>
> `SimpleExpr::Constant` operands are always written inline as literals
> (`prepare_constant`), never parameterised, even under `build`; this is how
> internally generated constants (for example the `ESCAPE` character and the
> empty-condition `TRUE`/`FALSE`) stay out of the parameter list. Identifiers
> are double-quoted in the rendered SQL.
>
> The two renderings are not interchangeable and the difference MUST be
> documented where a reader meets it: each query statement's `Display` impl
> carries the note that it inlines rather than binds, and points at `build`.
> The inlined form escapes what it writes, so the warning is not about
> injection; it is that the server re-parses the literal and the type pinning
> a bound value carries (`[spec:pgorm:req:sql.render.cast-param-type]`) is
> lost.

## Scope

> [spec:pgorm:req:sql.scope+23]
> pgorm-query models the PostgreSQL a data-access layer writes, not the whole
> of PostgreSQL, and the boundary MUST be written down rather than discovered.
> A construct outside the builder is still reachable — `Expr::raw` and
> `SqlTemplate` in expression position, `FromItem::Template` in relation
> position, and `ConnectionTrait::execute` / `batch_execute` /
> `query_raw` for a whole statement — so what follows records a decision about
> the *typed* surface, never about what a caller can send. Two things travel
> outside any statement's result, so no route above reaches them, and each has
> a route of its own: `COPY`'s data flow, through `copy_in` and `copy_out`
> (`conn.copy`), and `LISTEN`'s notifications, through a listening connection
> (`conn.listen`). Each entry
> MUST carry a verdict and the reason for it; "deferred" MUST additionally say
> what closing it would take, because a deferral whose cost is unrecorded is
> indistinguishable from an oversight. An entry graduates by being implemented
> and deleted from this list, and a construct absent from both this list and
> the builder is a defect in this rule, not a silent no. Some boundaries are
> recorded beside the construct they bound rather than here: `CONCURRENTLY`
> (`sql.ddl.index-create`), a range type's `CANONICAL` (`sql.ddl.type-range`),
> a sequence's persistence, owner and schema (`sql.ddl.sequence`), a view's
> persistence and `ALTER VIEW` (`sql.ddl.view`), a materialized view's
> storage and `ALTER MATERIALIZED VIEW` (`sql.ddl.materialized-view`),
> `CREATE TABLE AS`'s persistence and storage, `SELECT INTO` and `EXECUTE`
> (`sql.ddl.create-table-as`), a schema's `AUTHORIZATION`, owner and element
> list (`sql.ddl.schema`), a domain's `SET SCHEMA`, owner and named `NOT
> NULL` (`sql.ddl.domain`), COPY TO's column list (`sql.ast.copy`), and
> syntax only PostgreSQL 19 parses (`sql.target`).
>
> Three tests decide which verdict an entry gets. *Does the ORM need it to do
> its job?* — the builder exists to render the statements an entity layer
> issues, so a construct no ORM path emits is out of scope however ordinary it
> is in SQL. *Does the typed form add anything over the raw one?* — a
> construct with no options, no identifiers to quote and no values to bind is
> a string, and wrapping a string in a builder buys nothing. *Is it a
> statement or a vocabulary?* — a whole statement kind costs a new AST node, a
> new renderer and a new `SubQueryStatement` arm; a clause on an existing
> statement costs a field.
>
> **Deferred** — worth building, not built: none at present.
>
> **Out of scope** — not the builder's job:
>
> - **Partitioning DDL** (`PARTITION BY`, `PARTITION OF`, `ATTACH`/`DETACH`).
>   A physical-layout decision made once per table by whoever owns the schema,
>   with a large option surface and no values to bind. Migrations write it as
>   raw SQL.
> - **`EXCLUDE` constraints.** An operator-class-per-column constraint whose
>   typed form would be most of an index builder again, for a constraint an
>   entity layer never generates.
> - **Table and column storage and persistence options** (`WITH (fillfactor =
>   …)`, `TABLESPACE`, `SET STORAGE`, `TEMPORARY`, `UNLOGGED`,
>   `SET { LOGGED | UNLOGGED }`). Physical tuning with no effect on any
>   statement the ORM issues, and no identifier or value that needs quoting.
> - **`GRANT` / `REVOKE`, and roles.** Privileges are an operational concern,
>   granted by whoever administers the database rather than by the process
>   reading from it; a library that can widen its own privileges is a hazard
>   and not a feature.
> - **Triggers and `CREATE FUNCTION`.** Both carry a body in another language
>   (`plpgsql`, `sql`, C), which no expression AST can model — the body is an
>   opaque string however it is delivered, so a builder around it is quoting
>   with extra steps.
> - **Row-level security policies.** Same shape as privileges, and a policy
>   that the application can rewrite is not a security boundary.
> - **Procedural `DO` blocks.** A string of another language, as triggers are.
> - **`CREATE DATABASE` and the other cluster-level statements.** Run once by
>   an operator, outside any transaction and outside the database the
>   application connects to. `CREATE EXTENSION` is built (`sql.ddl.extension`),
>   because an extension is what makes a *column type* available and so is
>   reachable from an entity.
> - **`COPY` to or from a file on the server, and `PROGRAM`.** The server
>   reads or writes the file, or runs the program, as its own operating-system
>   user, which is why PostgreSQL reserves both for superusers and the
>   `pg_read_server_files`, `pg_write_server_files` and
>   `pg_execute_server_program` roles. That is access to the database host
>   rather than to its data, and a library that can widen its own reach is a
>   hazard, as with privileges. The data an application moves travels over its
>   connection, which is what `FROM STDIN` and `TO STDOUT` are (`sql.ast.copy`,
>   `conn.copy`).
> - **`EXPLAIN`.** A diagnostic no ORM path issues, whose result is a plan
>   rather than rows of the entity's shape; `query_raw` sends it with a built
>   statement's text and parameters.
> - **`LOCK TABLE`.** The ORM's locking is row-level, through `SELECT … FOR
>   UPDATE` / `FOR SHARE`, which is built (`sql.render.locking`); a table lock
>   is a statement with a table name and a mode, sent through `execute` inside
>   the transaction it must belong to.
> - **The extension `TABLESAMPLE` methods** (`SYSTEM_ROWS`, `SYSTEM_TIME`).
>   Each exists only once its extension, `tsm_system_rows` or
>   `tsm_system_time`, is installed, and draws by a row count or a time
>   budget rather than a percentage, so no ORM path depends on one;
>   `SampleMethod` builds the two every server has
>   (`sql.ast.select.table-sample`), and `FromItem::Template` reaches these.

## Target release

> [spec:pgorm:req:sql.target+2]
> A build of pgorm targets one PostgreSQL major release. 18 is the floor and
> the default: a build that names no release targets 18, there is no `pg-18`
> feature to say so, and nothing older is kept working. The `pg-19` cargo
> feature targets 19. pgorm-query declares it, since the typed surface lives
> there; pgorm forwards it (`pg-19 = ["pgorm-query/pg-19"]`), and so do both
> bindings, pgorm-python and pgorm-napi (`pg-19 = ["pgorm/pg-19"]`), each
> module exporting the release as `POSTGRES_TARGET`
> (`bindings.postgres-target`) and pgorm-python's capability manifest
> listing `pg-19` among its `features` (`python.capabilities`). No other crate
> carries it, because no other crate's surface or behaviour differs between
> the two releases yet. `pgorm_query::POSTGRES_TARGET` is the release a build
> targets, 18 or 19: a dependent cannot read another crate's features through
> `cfg`, so this is how it learns the target, at compile time if it likes.
>
> The feature is additive, as Cargo requires. Unification turns it on for
> every crate in a build once any one asks, so it MAY add typed surface only
> 19 accepts and MAY change which release's answers the live suite holds, and
> it MUST NOT remove or reshape anything the default build has that 19 still
> accepts. 19-only surface therefore arrives as new items — a type, a method,
> a constructor, or a variant of a `#[non_exhaustive]` public enum, whose
> dependents already match it with an arm for what they do not know
> (`sql.surface`), each marked `doc(cfg(feature = "pg-19"))` — and never as a
> new variant of an exhaustive public enum, a new public field or a changed
> signature, any of which breaks a dependent that compiles today. A builder
> value renders the same SQL under either target: what the server does with
> that SQL is the server's, and a release difference in it is documented
> rather than rendered around.
>
> The type gates what the builder can know. Syntax only 19 parses is
> reachable only under `pg-19`, so code built for 18 cannot construct a
> statement 18 cannot parse. A difference that turns on what the server holds
> — which kind of constraint a name is, which constraints a table has, how
> many rows a subquery returns — is beyond what any builder value records, so
> it is documented per release instead: the rule and the rustdoc give 18's
> answer and 19's, naming each, and the live test holds 18's answer by
> default and 19's under `cfg(feature = "pg-19")` (the bindings' tests branch
> on their module's `POSTGRES_TARGET`, or pgorm-python's on the manifest's
> `features`). Each build's suite runs against the server it
> targets: `DATABASE_URL` names an 18 server for the default build and a 19
> one for `pg-19`. Three such differences exist today, and no 19-only syntax
> is built yet: `ALTER CONSTRAINT .. ENFORCED` / `NOT ENFORCED` on a `CHECK`
> (`sql.ddl.enforcement`), `SET EXPRESSION` on a virtual column under a
> `CHECK` (`sql.ddl.alter-table`), and `JSON_ARRAY` over a query of no rows
> (`sql.ast.expr.sql-json`).
>
> Every parse pgorm makes stays on PostgreSQL 18's grammar under either target
> until libpg_query releases a 19 one: the render oracle
> (`sql.render.oracle`), `sql!` and `prql!`, codegen's DDL reader, the
> paginator and metric fingerprints. Building 19-only syntax waits for that
> grammar, since every render has to parse, and until it lands `sql!` refuses
> 19-only syntax under `pg-19` too. That work decides how the parsers follow
> the target, and whether pgorm-sql-macro and pgorm-codegen gain the feature
> for it, keeping an 18 build's `sql!` from accepting what 18 cannot parse.

## SELECT statements

> [spec:pgorm:def:sql.ast.select+6]
> `SelectStatement` is the SELECT AST node. It accumulates: an optional carried
> WITH clause (`Option<Box<AnyWithClause>>`, boxed so a clause-less statement
> pays one pointer — `query.build.with` records why the clause lives on the
> statement rather than in a wrapper), an optional
> `SelectDistinct` (`All`, `Distinct`, `DistinctOn(Vec<ColumnRef>)` — the
> MySQL-era `DistinctRow`, which no builder set and no renderer spelled, is
> gone and MUST NOT return),
> a list of `SelectExpr` projections (each an expression with optional alias and
> optional window), `from` table references, `JoinExpr` joins, a WHERE
> `ConditionHolder`, a GROUP BY list of `GroupingElement`s
> (`sql.ast.select.grouping`), a HAVING `ConditionHolder`, a list of
> `(UnionType, SelectStatement)` unions, ORDER BY expressions, an optional row
> limit — a `LIMIT` or the tie limit of `sql.ast.select.tie-limit`, one slot
> for both — and an optional OFFSET (each set from `u64` via
> `limit`/`limit_with_ties`/`offset`, cleared via
> `reset_limit`/`reset_offset`), an optional `LockClause`, and the WINDOW
> clause, a list of named windows (`sql.ast.window-statement`). The GROUP BY
> list carries its set quantifier,
> `DISTINCT` once `group_by_distinct` is called
> (`sql.ast.select.group-distinct`).
>
> A `LockClause` pairs a `LockType` (`Update`, `NoKeyUpdate`, `Share`,
> `KeyShare`) with an optional `OF` list and optional `LockBehavior` (`Nowait`,
> `SkipLocked`); `lock`, `lock_with_tables`, `lock_with_behavior`,
> `lock_with_tables_behavior`, `lock_shared` (FOR SHARE) and `lock_exclusive`
> (FOR UPDATE) each overwrite the whole clause, so the last call wins. The
> `OF` list holds names, each an `IntoName`: the name a FROM item answers to,
> its alias or an unaliased table's own name. PostgreSQL's `OF` takes
> unqualified relation names and nothing else, refusing a schema-qualified one
> (`42601`), so a FROM item's source — an aliased table, a subquery, a
> function call, a sample — has no construction there
> (`[dec:pgorm:invalid-states-unrepresentable]`). Which item a name refers to
> is the server's to check: a name no FROM item answers to, an aliased table's
> own name among them, is `42P01`, and one naming a function, a table
> function, a join or a common table expression, which hold no rows to lock,
> is `0A000`; a subquery's name locks the rows it reads.
> Structural-control helpers `conditions(bool, then, else)`, `apply_if(Option, f)`
> and `apply(f)` let callers branch while chaining.

> [spec:pgorm:req:sql.ast.select.projection+2]
> Projections MUST accumulate in call order: `expr`/`exprs` push anything
> convertible to `SelectExpr`, `column`/`columns` push `SimpleExpr::Column`
> projections from any `IntoColumnRef` (bare column, `(table, column)`, or
> `(schema, table, column)` tuples), and `expr_as` attaches an `AS` alias.
> `clear_selects` MUST empty the projection list, and `selects()` MUST expose
> the accumulated list for inspection — the read side callers need to tell an
> empty projection from a populated one without rendering the statement.
>
> `distinct()` MUST set `SelectDistinct::Distinct`. `distinct_on(cols)` MUST set
> `SelectDistinct::DistinctOn` when the column collection is non-empty, and MUST
> clear the distinct flag entirely (render no DISTINCT at all) when the
> collection is empty.
>
> GROUP BY items accumulate in call order via `group_by_columns`,
> `group_by_col` and `add_group_by`, each adding plain expressions, and
> `group_by_element`, which adds a grouping element of `sql.ast.select.grouping`
> to the same list. HAVING accepts conditions through `cond_having` (any
> `IntoCondition`) and `and_having` (a `SimpleExpr` shorthand delegating to
> `cond_having`); both feed the HAVING `ConditionHolder` with the semantics of
> `sql.ast.condition.holder`.

> [spec:pgorm:req:sql.ast.select.from+4]
> FROM clauses MUST accumulate: calling `from` repeatedly produces multiple
> comma-separated FROM items (the "old-school join" form), and `from_clear`
> MUST remove all of them. The FROM item variants are: plain tables (with
> optional schema qualification via a 2-tuple), `from_as` (aliased
> table), `from_subquery` (`FromItem::SubQuery` with mandatory alias),
> `from_function` (`FromItem::FunctionCall` with alias), and `from_values`
> (`FromItem::ValuesList` rendering `(VALUES (..), (..)) AS "alias"`).
> `FromItem::JsonTable` (`sql.ast.json-table`) is made by `JsonTable::alias`
> and handed to the generic `from`, or to a join: the alias is the one thing
> a shorthand would take, and the builder already holds it. So is
> `FromItem::TableFunction`, made by `TableFunction::alias`
> (`sql.ast.select.table-function`), and `FromItem::TableSample` converts from
> a `TableSample` (`sql.ast.select.table-sample`), which `from` takes as it
> is.
>
> `FromItem::Template` (`[spec:pgorm:def:sql.types.table-ref]`) gets no
> shorthand of its own. Its constructor returns `Result`, and a shorthand
> would have to either return `Result` — breaking the `&mut Self` chain every
> other builder method keeps — or swallow the failure; instead it is built
> first and handed to the generic `from`, which already takes any
> `IntoFromItem`. A builder method is not owed to every variant; it is owed
> where it saves the caller a type name and nothing else.
>
> `from_values` MUST panic when given an empty tuple list (`assert!` on the
> collected rows); there is no non-panicking variant.

> [spec:pgorm:req:sql.ast.select.join+1]
> Joins MUST accumulate as `JoinExpr { join, table, lateral }` entries, where
> `join` is a `JoinKind` that carries the constraint with the join it belongs
> to: `Qualified(JoinType, JoinOn)` for a join with an `ON` clause, and `Cross`
> for the one join PostgreSQL takes without one. `JoinType` therefore names
> only the constrained joins — `Join`, `InnerJoin`, `LeftJoin`, `RightJoin`,
> `FullOuterJoin` — and the generic `join(JoinType, table, condition)` cannot
> spell a cross join, nor `cross_join(table)` a condition
> (`[dec:pgorm:invalid-states-unrepresentable]`). Named shorthands are
> `cross_join`, `left_join`, `right_join`, `inner_join`, and
> `full_outer_join`. `join_as` MUST alias the joined table; `join_subquery` MUST
> join a `SelectStatement` as an aliased subquery; `join_lateral` MUST do the
> same with the `lateral` flag set, rendering `JOIN LATERAL`.
>
> The ON condition is any `IntoCondition` (a bare `SimpleExpr` or a
> `Condition` tree) and MUST be stored as `JoinOn::Condition` wrapping a
> `ConditionHolder`, so multi-part conditions built with `Condition::all`/`any`
> render as chained `AND`/`OR` in the ON clause.

> [spec:pgorm:req:sql.ast.select.table-function+3]
> A set-returning function in FROM is a `TableFunction`, PostgreSQL's
> `func_table`: one call, built by `TableFunction::new(call)`, or
> `ROWS FROM (..)`, built by `TableFunction::rows_from(call)`, to which
> `and(call)` adds a call read beside the others. A second call makes it
> `ROWS FROM` whichever constructor began it, since the grammar has no other
> spelling for several; over one call `ROWS FROM` differs from the plain call
> only for a function returning a composite, whose columns it expands.
> `ROWS FROM` reads its calls side by side, row *n* of each forming row *n* of
> the relation, and pads the calls that run out first with NULLs.
> `with_ordinality()` adds `WITH ORDINALITY`, a final `bigint` column
> numbering the rows from 1, and `columns(names)` names the relation's columns
> in its alias, `AS "t"("a", "b")`, the ordinal column last, replacing any
> names given before. Fewer names leave the remaining columns their own; more
> than the relation has is the server's refusal (`42P10`). A call returning
> `record` takes a column definition list
> (`sql.ast.select.column-definitions`).
>
> `alias(name)` finishes it as `FromItem::TableFunction`, which `from`, the
> joins and every other FROM position take, as `JsonTable::alias` does. A
> single call with no ordinal, no column names and no column definitions is
> returned as the `FromItem::FunctionCall` that `from_function` builds, so
> `f(..) AS "t"` has one representation however it was spelled.
>
> The call is any `FunctionCall`. `Func::generate_series(start, stop)`,
> `Func::generate_series_step(start, stop, step)` and `Func::unnest(array)`
> are the set-returning constructors, `Function::GenerateSeries` and
> `Function::Unnest`, and the two the Python and JavaScript call vocabularies
> reach; `Func::named` calls any other by name. Both are overloaded on their
> argument types, so a bound argument carries its own type there,
> `UNNEST($1::int4[])` and `GENERATE_SERIES($1::int4, $2::int4)`
> (`[spec:pgorm:sem:sql.render.placeholder-typing]`): a bare placeholder is
> ambiguous to the server (`42725`). A cast chooses another overload, and a
> function called by name annotates nothing, its signature unknown to the
> renderer. A zero step is the server's refusal (`22023`).

> [spec:pgorm:req:sql.ast.select.column-definitions]
> A function returning `record` — `json_to_record`, `jsonb_to_record`,
> `json_to_recordset` and `jsonb_to_recordset`, whose `Func` constructors
> make `Function::JsonToRecord` and its three kin, or a function declared
> `RETURNS record` or `SETOF record`, called by `Func::named` — has no
> columns of its own, so PostgreSQL reads it in FROM only with a column
> definition list naming and typing them. `TableFunction::column_definition(name,
> column_type)` appends one column, a name and any `ColumnType`, to the list
> of the call added last, each call of a `ROWS FROM` carrying its own; a list
> is non-empty by construction, since a call has none until a column is
> defined. The JSON functions read each column from the object's key of its
> name, NULL where the object has none, and a value the column's type cannot
> take is the server's refusal (`22P02`).
>
> A lone call writes its list in the alias, `f(..) AS "t"("a" integer)`.
> PostgreSQL refuses that spelling beside `WITH ORDINALITY` (`42601`), and
> the alias's place for column names is the list's, so a lone call with a
> definition list and an ordinal column or column names is written as the
> one spelling that means it, `ROWS FROM (f(..) AS ("a" integer))`, its
> ordinal and names following as any `ROWS FROM`'s do — as a second call
> makes it `ROWS FROM` because the grammar has no other spelling. Inside
> `ROWS FROM` each call's list follows it, `g(..) AS ("b" text)`.
>
> Which function returns `record` is the server's to know, so the builder
> refuses nothing here, and these are the server's refusals: a call
> returning `record` without a list, and a list on a call returning
> anything else (both `42601`); two columns of one name (`42701`); and
> columns the function's rows do not match, which a `LANGUAGE sql` function
> reports as `42P13` and a PL/pgSQL one as `42804` when it returns a row.

> [spec:pgorm:req:sql.ast.select.table-sample]
> A table read through `TABLESAMPLE` is a `TableSample`:
> `TableSample::new(table, method, percentage)` over any `IntoNamedTable` — a
> table name, optionally schema-qualified and aliased — with `repeatable(seed)`
> setting the `REPEATABLE` seed, a second call replacing the first.
> `SampleMethod` is PostgreSQL's two built-in methods: `Bernoulli` draws each
> row on its own, `System` each page, reading less of the table and clustering
> the sample by page. The extension methods (`SYSTEM_ROWS`, `SYSTEM_TIME`) are
> not built.
>
> The percentage, the probability out of 100 that a row or page is drawn, and
> the seed are any `Into<f64>` and are bound as parameters, which the server
> types `real` and `double precision`; binding narrows the `real` one as it
> narrows any double bound there. A percentage outside 0 to 100, which an
> `f64` can hold, is the server's refusal (`2202H`), as is a NULL (`2202H`, and
> `2202G` for the seed). The same seed over unchanged data draws the same
> sample.
>
> A sample converts into `FromItem::TableSample`, so `from`, the joins, an
> UPDATE's FROM list, a DELETE's USING list and a MERGE's source all take it;
> its `table_name` is the sampled table's, its `qualifier` the alias or else
> the table name, and `FromItem::alias` re-aliases the table it samples. Only
> a table can be sampled, and the type holds nothing but a table name, so a
> sampled subquery, values list or function — which the grammar refuses
> (`42601`) — has no construction; a common table expression's name is a name
> the type cannot tell from a table's, and sampling one is the server's
> refusal (`0A000`).

> [spec:pgorm:req:sql.ast.select.tie-limit+2]
> `limit_with_ties(count)` sets a tie limit, `FETCH FIRST count ROWS WITH
> TIES`: the first `count` rows in the statement's ORDER BY and every further
> row equal to the last of them on all its keys. It shares one slot with
> `limit` — PostgreSQL's grammar takes a `LIMIT` or a `FETCH FIRST`, never both
> — so either replaces the other and `reset_limit` removes either;
> `has_tie_limit` says which is set. OFFSET composes with it as with `LIMIT`.
> The count is bound, which the server types `bigint`.
>
> `narrow_limit_to_one` narrows the slot to at most one row for a reader that
> asks for the first, and never widens it: a statement whose limit counts no
> row — `LIMIT 0`, or a tie limit of zero, which returns no row — gets
> `LIMIT 0`, and any other gets `LIMIT 1`, a tie limit included, since the
> first row a tie limit returns is the first row of the order. OFFSET is kept.
>
> The grammar refuses a tie limit in two places (`42601`): on a statement with
> no ORDER BY, since the ties are measured on the order, and beside
> `SKIP LOCKED`, since the tied set would then depend on which rows other
> transactions hold. `FOR UPDATE` and the weaker strengths, `NOWAIT`
> included, combine with it. Neither refused state is designed out
> (`[dec:pgorm:invalid-states-unrepresentable]`): the ORDER BY and the lock
> are set by other methods in any order, and `clear_order_by` can remove the
> order after the limit was set. So `check_tie_limit` refuses both before
> anything renders —
> `Error::TiesWithoutOrder` and `Error::TiesWithSkipLocked` — and every
> pgorm executor and the Python and JavaScript builders call it before they
> render (`[dec:pgorm:no-panic]`, `exec.tie-limit`), as the constructors of a
> view, a materialized view and a `CREATE TABLE AS` call it on the body they
> take (`sql.ddl.view`, `sql.ddl.materialized-view`,
> `sql.ddl.create-table-as`). Rendering is infallible and writes what the
> statement holds (`sql.render.tie-limit`), so a caller of pgorm-query alone
> who renders a `SELECT` without the check sends SQL the server refuses with
> that `42601`; a stored definition's body cannot, since none of those
> statements constructs over a body the check refuses. The check reads the
> statement's own clauses: a SELECT nested
> in it — a subquery, a CTE body, a set operation's arm — carrying a refused
> shape is the server's refusal.
>
> `check_clauses` is the one public check a caller about to render a
> `SELECT` as it stands calls: it runs `check_tie_limit` and then
> `check_windows` (`sql.ast.window-statement`), the first refusal found its
> error. Every pgorm executor, the Python and JavaScript builders, a stored
> definition's body (`sql.ddl.view`) and a COPY TO's source (`sql.ast.copy`)
> call it rather than pairing the two themselves, so each reports the same
> refusal for the same statement.

> [spec:pgorm:req:sql.ast.select.group-distinct]
> `group_by_distinct()` sets the GROUP BY list's set quantifier to
> `DISTINCT`: each grouping set the list stands for is grouped by once,
> however many combinations of its items reach it. `ROLLUP (a, b), ROLLUP
> (a)` stands for `(a, b)` twice, `(a)` three times and `()` once, and so
> yields the `(a)` groups three times over; with `DISTINCT` each set yields
> its groups once. The quantifier applies to the whole list, whichever call
> adds to it and in whichever order, and changes nothing over a list with no
> repeated set. With no GROUP BY at all there is no list to quantify, and
> nothing renders. The default quantifier, `ALL`, is never spelled, and a set
> quantifier cannot be unset, as `distinct` cannot.

> [spec:pgorm:def:sql.ast.select.grouping]
> A SELECT's GROUP BY list is a list of `GroupingElement`s — PostgreSQL's
> `grouping_element` — and each element stands for a list of grouping sets;
> the query groups by every combination of its items' sets, one set from
> each. The element's shape is private, so it is built only through its
> constructors:
>
> - `GroupingElement::set(exprs)` — one set of the given expressions, and
>   `GroupingElement::empty()`, the set of none, which groups the whole input
>   into one grand-total row (one row even over an empty input);
> - `GroupingElement::rollup(exprs)` — every leading prefix of the list,
>   longest first, down to the empty set: `ROLLUP (a, b)` is `(a, b)`, `(a)`
>   and `()`;
> - `GroupingElement::cube(exprs)` — every subset of the list, the empty one
>   included;
> - `GroupingElement::sets(first)` — `GROUPING SETS`, exactly the sets its
>   elements stand for, in turn and duplicates kept. It takes its first
>   element and returns a `GroupingSets` whose `add` appends further ones and
>   which converts into a `GroupingElement`, so the empty `GROUPING SETS ()`
>   the grammar rejects has no value to build. Its elements are elements, so
>   a `ROLLUP` or `CUBE` nests inside it and contributes all its own sets;
>   `ROLLUP` and `CUBE` take expressions, because the grammar nests nothing
>   inside them.
>
> An item of a `ROLLUP` or `CUBE` that is an `Expr::tuple` is one unit of the
> list, kept or dropped whole — `ROLLUP ((a, b), c)` has three prefixes, not
> four. A `ROLLUP` or `CUBE` over no expressions stands for the one empty set.
>
> `SelectStatement::group_by_element(element)` appends an element to the same
> list the plain methods of `sql.ast.select.projection` fill: those add each
> expression as a set of one, so a statement built only from them groups and
> renders exactly as a flat expression list did, and elements and plain
> expressions interleave in call order. The method lives beside the element
> type rather than on `SelectStatement`'s own file, which is at its function
> cap, and pgorm's `QuerySelect::group_by_element` passes through to it.
>
> `Func::grouping(first)` builds `GROUPING(…)`, the function that reads which
> of its arguments the current row's set leaves out: a bitmask whose last
> argument is bit 0, a bit being set when that argument is not grouped. It
> returns a `Grouping`, whose `arg` adds further arguments and which converts
> into `SimpleExpr::Grouping`. It is deliberately not a `FunctionCall`:
> PostgreSQL's grammar spells `GROUPING` as its own expression, with at least
> one argument and with no `FILTER`, `WITHIN GROUP`, `DISTINCT` or `OVER`, so
> as a `FunctionCall` it would admit all four and the windowed projections of
> `sql.ast.window-statement` would accept it. It is how a query tells the
> NULL a subtotal row puts in an omitted column from a NULL in the data; that
> each argument is one of the query's grouping expressions — so that it has
> nothing to read under `GROUP BY ()` — is PostgreSQL's check (`42803`), not
> the builder's.

> [spec:pgorm:sem:sql.ast.select.union+2]
> `union(UnionType, query)` appends one compound-query arm and `unions(iter)`
> extends with many; arms accumulate in call order and are never merged or
> deduplicated. `UnionType` names all six of PostgreSQL's set operations —
> each of the three operators in its duplicate-eliminating and its
> duplicate-keeping form: `Distinct` renders `UNION` and `All` renders
> `UNION ALL`, `Intersect`/`IntersectAll` render `INTERSECT`/`INTERSECT ALL`,
> and `Except`/`ExceptAll` render `EXCEPT`/`EXCEPT ALL`. Each appended arm is
> rendered as a parenthesised SELECT after the operator. The AST does not
> verify that the arms project the same columns — that is left to PostgreSQL.
>
> A chain MUST mean what it reads in call order: each arm combines everything
> appended before it, the receiver's own query first, with its own query, so
> `a.union(b).intersect(c)` is `(a UNION b) INTERSECT c`. PostgreSQL's
> precedence would read the same arms written flat otherwise — `INTERSECT`
> binds tighter than `UNION` and `EXCEPT`, which share one level and
> associate left — so an `INTERSECT` or `INTERSECT ALL` arm appended right
> after a `UNION` or `EXCEPT` arm (either form) MUST parenthesise everything
> written before it, and nothing else is regrouped: a chain whose flat
> reading is already its call-order reading renders flat. To combine with a
> combination, the combination is the arm: `c.intersect(a.union(b))`.
>
> The two UNION spellings are the inherited ones and MUST be documented on the
> enum itself, because `All` beside `IntersectAll` reads as a modifier and is
> not one: the four original names say which row set the operation yields,
> where the two new ones name the operator. Renaming them to `Union`/`UnionAll`
> is the fix that clause anticipates, not a defect it records.

## Ordering

> [spec:pgorm:req:sql.ast.order+3]
> `SelectStatement` and `WindowStatement` — the two statements PostgreSQL
> admits an ORDER BY on — share the `OrderedStatement` trait; the write
> statements do not implement it, per `sql.ast.update` and `sql.ast.delete`.
> Order expressions MUST accumulate in call order via `order_by` (column +
> `Order`), `order_by_expr`, `order_by_columns`, and the `*_with_nulls` variants
> which attach a `NullOrdering` (`First`/`Last`) rendered as `NULLS
> FIRST`/`NULLS LAST`. `clear_order_by` MUST remove all accumulated order
> expressions. There is no raw-string ordering verb: a verbatim SQL fragment
> reaches ORDER BY position only as an `Expr::raw` through `order_by_expr`.
>
> `Order` MUST support `Asc`, `Desc`, and `Field(Values)`; the `Field` variant
> renders a `CASE WHEN col=v_i THEN i ... ELSE n END` expression implementing
> explicit value ordering.

## Conditions

> [spec:pgorm:def:sql.ast.condition+2]
> `Condition` is a tree node holding a `condition_type`
> (`ConditionType::All` = conjunction, `ConditionType::Any` = disjunction), a
> `negate` flag, and child `ConditionExpression`s, where each child is either a
> nested `Condition` or a leaf `SimpleExpr`. `Condition::all()` and
> `Condition::any()` construct empty sets; `add` pushes a child; `add_option`
> pushes only when `Some`; `not()` toggles the negate flag; `is_empty`/`len`
> inspect the children. Those constructors are the only spelling — the type
> carries no shorthand alias and no shorthand macro.
>
> The `IntoCondition` trait converts arguments at API boundaries: an
> expression — any `ExprTrait` type (`sql.ast.expr.operators`), so a
> function call or an `Expr` as well as a `SimpleExpr` — becomes
> `Condition::all().add(expr)` and a `Condition` passes through unchanged,
> which is why `and_where`-style helpers and `cond_where` accept both.
> `Condition::add` takes the same two, every expression converting into a
> `ConditionExpression` leaf.

> [spec:pgorm:sem:sql.ast.condition.flattening]
> `Condition::add` flattens trivial nesting: when the added child is itself a
> `Condition` with exactly one member and no negation, the inner member is
> unwrapped and pushed directly, skipping the useless junction. Nested
> conditions with two or more members, or with `negate` set, are kept intact
> and render inside parentheses.
>
> When a `Condition` is lowered to a `SimpleExpr` (`to_simple_expr`), members
> are folded left-to-right with `OR` for `Any` and `AND` for `All`. An empty
> `Any` lowers to the constant `FALSE` and an empty `All` to the constant
> `TRUE` (as inline `SimpleExpr::Constant`s), and a set with `negate` wraps the
> folded expression in `NOT (...)`.

> [spec:pgorm:req:sql.ast.condition.holder+2]
> WHERE and HAVING clauses are backed by `ConditionHolder`, whose contents are
> an `Option<Condition>`: absent until a condition is added, `Some` thereafter.
> There is exactly one way in — `add_condition`, reached through `cond_where`
> and `cond_having` — so the holder has no second, incompatible representation
> to mix with, and the API surface offers no operation that can fail on it.
> `ConditionalStatement::and_where`, `and_where_option`, and `and_having` are
> shorthands that lift a `SimpleExpr` through `IntoCondition` into
> `Condition::all().add(expr)` and delegate; a chain style built from
> per-link `AND`/`OR` operators is deliberately absent, so the two styles
> cannot disagree about how links combine.
>
> Repeated `cond_where` calls MUST conjoin: if both the current and the added
> condition are non-negated `All` sets the additions are appended flat into the
> existing set; otherwise the current contents and the addition are combined
> under a fresh `Condition::all()`. Order of calls is preserved in the rendered
> output.

## Expressions

> [spec:pgorm:def:sql.ast.expr+8]
> `SimpleExpr` is the expression tree node, with variants `Column(ColumnRef)`,
> `Tuple`, `Unary(UnOper, ..)` (the only unary operator is `Not`),
> `FunctionCall`, `Binary(lhs, BinOper, rhs)`, `SubQuery(Option<SubQueryOper>, ..)`,
> `Value` (parameterised), `Values`, `Raw(&'static str)` (verbatim SQL),
> `Template(SqlTemplate)`, `Keyword`, `AsEnum`, `Case` and `SimpleCase` (the
> searched and simple forms of `sql.ast.case`), `Subscript` (an array
> subscript or slice, `sql.ast.expr.subscript`), `Grouping` (the
> `GROUPING()` of `sql.ast.select.grouping`), `Constant` (inlined literal),
> `LikePattern` (a `LIKE` pattern with its optional `ESCAPE`), and `SqlJson`
> (an SQL/JSON function, constructor or `IS JSON`, `sql.ast.expr.sql-json`). `SqlTemplate` holds a template with `$1`-style splices
> (`$$` escaping a literal `$`) already resolved against the expressions it
> substitutes; its segments are private and its only constructor is
> `SqlTemplate::new`, which returns `Result`, so the AST cannot hold a template
> whose placeholders and values disagree — see `sql.render.custom-expr`.
>
> `Expr` is the entry point, holding the one `SimpleExpr` it starts;
> `Expr::col`, `Expr::val`, `Expr::expr`, `Expr::tuple`, `Expr::value` and
> `Expr::raw` construct expressions from columns, values, other expressions,
> and raw SQL — `Expr::raw` bound to `&'static str`, so only program text can
> be written verbatim — and `Expr::template`, `Expr::template_with_expr` and
> `Expr::template_with_exprs` do the same for templates, each returning
> `Result` because each pairs a template with substitutions. What an
> expression becomes — compared, combined, cast, aggregated — is
> `ExprTrait`'s (`sql.ast.expr.operators`), which `Expr` implements as every
> other expression type does, so `Expr` carries no operator of its own.
>
> Subquery expressions carry an optional `SubQueryOper`: `Expr::exists`,
> `Expr::any`, `Expr::some`, `Expr::all` and `Expr::array` wrap a
> `SelectStatement` in `EXISTS(...)`, `ANY(...)`, `SOME(...)`, `ALL(...)` and
> `ARRAY(...)` respectively. `ARRAY(...)` is the subquery's one column
> collected into one array value — PostgreSQL refuses a wider subquery there
> (`42601`), as it does under `IN` — so it can be cast and compared as a
> value, `= ANY(ARRAY(...))` testing the membership `IN (...)` tests.
> `From` conversions lift `Value`-convertible Rust primitives, `FunctionCall`,
> `ColumnRef`, `Keyword`, `CaseStatement`, `SimpleCaseStatement`, each SQL/JSON
> builder, and finished
> `Expr` builders into
> `SimpleExpr`, which is what allows plain Rust values wherever
> `Into<SimpleExpr>` is accepted.

> [spec:pgorm:req:sql.ast.expr.operators+6]
> The operator vocabulary is one public trait, `ExprTrait`, and every type
> that is an expression in its own right MUST implement it: `Expr`,
> `SimpleExpr`, `FunctionCall`, `CaseStatement`, `SimpleCaseStatement`,
> `Grouping` and each SQL/JSON builder (`SqlJson`, `JsonExists`, `JsonValue`,
> `JsonQuery`, `JsonObject`, `JsonArray`, `JsonArrayQuery`, `JsonObjectAgg`,
> `JsonArrayAgg`, `JsonParse`, `JsonSerialize`). What one method returns is
> therefore built on by the next with no `Expr::expr` around it: a function
> call compares (`Func::lower(..).eq(..)`), an aggregate compares in `HAVING`
> (`Expr::col(..).count().gt(2)`), a cast compares
> (`Expr::col(..).cast_as(..).lt(10)`). Each method consumes its receiver and
> returns a `SimpleExpr`. The trait is a root export and in pgorm's prelude;
> a method named for a comparison takes `self` by value, which is what makes
> `eq` and the orderings the trait's where it is in scope, ahead of
> `PartialEq`'s and `PartialOrd`'s borrowing methods and of `Iterator`'s. The
> trait MUST provide combinators that produce `Binary`/`Unary` nodes: comparisons `eq`, `ne`, `gt`, `gte`, `lt`, `lte`, plus
> `equals`/`not_equals` for column-to-column comparison; arithmetic `add`,
> `sub`, `mul`, `div`, `modulo`; bit shifts `left_shift`, `right_shift`;
> `between`/`not_between` and the bound-sorting
> `between_symmetric`/`not_between_symmetric`, whose two bounds are typed
> apart, since one may be a value and the other an expression built on; `is_null`, `is_not_null`, `is`,
> `is_not`, and the null-safe `is_distinct_from`/`is_not_distinct_from`;
> logical
> `and`, `or`, and `not` (prefix `NOT`); string/pattern operators `like`,
> `not_like`, `ilike`, `not_ilike`, `similar_to` and `not_similar_to`
> (`sql.ast.expr.similar-to`) — a `LikeExpr` with an escape character MUST
> render an `ESCAPE` clause whose character is an inline constant — and
> `concat` (`||`).
>
> The null-safe pair is not a verbose spelling of `eq`/`ne`: under three-valued
> logic `a <> b` is NULL whenever either side is, so a nullable column passes
> neither the comparison nor its negation, and `IS DISTINCT FROM` is the only
> form that answers true or false for every pair. `between_symmetric` likewise
> exists for the failure it removes rather than for brevity — `BETWEEN` with
> its bounds written in the wrong order matches nothing and reports no error.
>
> PostgreSQL-specific operators MUST be available: full-text `matches` (`@@`)
> and containment `contains` (`@>`) / `contained` (`<@`), overlap `overlaps`
> (`&&`), and temporal
> `at_time_zone`, whose right operand is an ordinary expression — a bound zone
> name or a column of them — and whose direction follows the left operand's
> type, as the server's operator does. Containment and
> `concat` are type-general rather than string-specific — PostgreSQL defines
> one `@>`, one `<@` and one `||` across arrays, ranges, `tsquery` and `jsonb`
> alike — so these three combinators MUST also serve as the JSON containment
> and merge tests, and `sql.ast.expr.json` MUST NOT name duplicates of them.
>
> `overlaps` asks whether its operands share a value: two ranges, a
> multirange and a range or another multirange, two arrays. It renders
> `BinOper::Overlap`, which `binary` still reaches. Checked live on
> PostgreSQL 18.6: the empty range overlaps nothing, `(,)` included; beside a
> range column the server types a bare operand as that range (`r && $1`
> infers `int4range`) and beside a multirange as that multirange, so a range
> compared with a multirange is pinned with `cast_as` to reach the
> `multirange && range` operator, and a bound range left unpinned there is
> refused before it is sent; an element is no operand of `&&` at all
> (`42883`), unlike containment's element form.
> The escape hatch `binary(op, rhs)` accepts any `BinOper`, whose variants
> further include regex (`~`, `~*`), trigram similarity and distance operators,
> pgvector distance operators, `Overlap`, and `Raw(&'static str)` for
> arbitrary operator text. Casts are expressed with `cast_as`
> (`CAST(expr AS type)`) and `as_enum`; aggregate shorthands `max`, `min`,
> `sum`, `count`, `count_distinct`, and `if_null` wrap the expression in the
> corresponding function call. JSON field, path and key-existence access is
> `sql.ast.expr.json`.

> [spec:pgorm:req:sql.ast.expr.similar-to+3]
> `similar_to(pattern)` and `not_similar_to(pattern)`, on every expression
> (`ExprTrait`), build `SIMILAR TO` and `NOT SIMILAR TO`
> (`BinOper::SimilarTo`, `BinOper::NotSimilarTo`): whether the whole string
> matches a pattern in SQL's regular-expression dialect, which is LIKE's `%`
> and `_` with `|`, `*`, `+`, `?`, `{m,n}`, grouping and bracket classes
> added. The pattern is any `IntoLikeExpr`, the operand LIKE takes, held as
> `SimpleExpr::LikePattern`: it is bound, which the server types `text`, and a
> `LikeExpr` carries the `ESCAPE` the grammar admits only as a pattern's tail.
> The default escape is a backslash; `LikeExpr::escape(c)` replaces it with
> one character, written as an inline constant, and `LikeExpr::no_escape()`
> turns escaping off, `ESCAPE ''`, so every character of the pattern, a
> backslash included, is itself or a wildcard. Both hold for LIKE and ILIKE as
> for SIMILAR TO, the server accepting the empty escape for all three. An
> escape of more than one character, the server's refusal (`22025`), has no
> construction: `escape` takes a `char` and `no_escape` nothing. Both
> operators are of the LIKE family the renderer ranks by
> (`sql.render.precedence`), where the grammar ranks `SIMILAR`.
>
> A pattern is also built from literal text, a user's search among it:
> `LikeExpr::literal(text)` matches exactly the text,
> `LikeExpr::containing(text)` is `%text%`, `starting_with(text)` `text%` and
> `ending_with(text)` `%text`. The text's `%`, `_` and escape character are
> each preceded by the escape, so the text matches itself and nothing it
> would otherwise widen to. The escaping is the pattern's: it is done for
> the escape character in force when the statement renders, so `escape(c)`
> chosen after the text escapes it with `c`, and under `no_escape()` nothing
> is escaped and the text's `%` and `_` are wildcards. The set escaped is
> LIKE's; under `SIMILAR TO` the regular-expression characters keep their
> meaning. This is the one escaping pgorm has: the entity columns'
> `starts_with`, `ends_with` and `contains` (`entity.traits.column`) are
> these constructors.

> [spec:pgorm:req:sql.ast.expr.in+1]
> `is_in`/`is_not_in` MUST build an `IN`/`NOT IN` over a `Tuple` of the given
> operands. When the operand collection is empty, rendering MUST fall back to a
> constant comparison rather than emit invalid SQL such as `IN ()`, and the two
> fall-backs MUST carry the vacuous truth value of the predicate they stand in
> for: an empty `IN` renders the always-false `'a' = 'b'`, because membership in
> the empty set holds for nothing; an empty `NOT IN` renders the always-true
> `'a' = 'a'`, because non-membership in the empty set holds for everything,
> including a NULL operand.
>
> `in_tuples` MUST build multi-column membership tests
> (`(a, b) IN ((..), (..))`) from `IntoValueTuple` rows, and
> `in_subquery`/`not_in_subquery` MUST build `IN (SELECT ...)` /
> `NOT IN (SELECT ...)` from a `SelectStatement`.

> [spec:pgorm:req:sql.ast.expr.eq-any+2]
> `eq_any`/`ne_all` MUST build the same membership tests over a *single array
> parameter*: `eq_any` the comparison `= ANY(<array>)` and `ne_all` its
> complement `<> ALL(<array>)`, with the operand collection gathered into one
> `Value::Array` by `Value::array` rather than spread across a `Tuple`. They are
> the paved road beside `is_in`/`is_not_in`, which stay: `IN` spends one
> placeholder per element, so each list length is distinct SQL text that a
> per-connection prepared-statement cache must hold separately, and a long
> enough list reaches PostgreSQL's 65535-parameter limit. Prefer `is_in` for a
> short literal list written into the query, and `eq_any` when the length varies
> at runtime.
>
> The operand type MUST be bound `ArrayElement`, not `Into<Value>` alone: the
> array's element tag is read from the Rust type, so an empty list is still a
> typed array (see `sql.value.array`). Every `ValueType` is an `ArrayElement`,
> and so are `&str`, `&String` and `&[u8]`, so `eq_any(["a", "b"])` takes the
> string slices `eq("a")` does, binding one `text[]`. `ColumnTrait`'s
> `eq_any` and `ne_all` take the same bound.
>
> The negation MUST be spelled `<> ALL`, not `NOT (… = ANY …)`. The two are
> equivalent under three-valued logic — a NULL element makes both NULL, as it
> makes `NOT IN` NULL — so the choice is which shape reaches the planner, and
> PostgreSQL's own parser settles it: it reads `x NOT IN (…)` as an `A_Expr` of
> kind `AEXPR_IN` whose operator name is `<>`, the same operator-level negation
> `x <> ALL(…)` parses to (`AEXPR_OP_ALL`, name `<>`), where `NOT (x = ANY(…))`
> parses to a `BoolExpr` wrapped around a second node.
>
> An empty collection MUST NOT be special-cased. Unlike `IN`, an array
> comparison over an empty array is valid SQL carrying the right vacuous truth
> on its own: `= ANY` over an empty array is false for every operand and `<>
> ALL` true for every operand, NULL operands included — the same asymmetry
> `sql.ast.expr.in`'s two constant fall-backs exist to reproduce. The statement
> text is therefore identical at every cardinality, including zero.
>
> `ANY`/`SOME`/`ALL` are quantifiers over the right operand of a binary operator
> PostgreSQL reads one after — a comparison, LIKE and ILIKE and their negations,
> the regular-expression matches, containment and overlap, the range positions,
> `@@` and `@?` (`bindings.quantifiers` lists them) — not functions:
> `Func::any`/`some`/`all` render SQL only in that position, and are the escape
> hatch for the operators these two do not name.

> [spec:pgorm:req:sql.ast.expr.json+3]
> `ExprTrait` MUST provide the JSON *operator* vocabulary as typed
> combinators, so every expression has it, a field already extracted
> included:
> field access `get_json_field` (`->`) and `cast_json_field` (`->>`), path
> access `get_json_path` (`#>`) and `cast_json_path` (`#>>`), key existence
> `has_json_key` (`?`), `has_any_json_keys` (`?|`) and `has_all_json_keys`
> (`?&`), and path existence `json_path_exists` (`@?`). What one returns is
> an expression like any other, so `get_json_field("a").get_json_field("b")`
> reads a nested field in one chain.
>
> `json_path_exists` binds its path as `text` cast to `jsonpath`, as
> `JSON_EXISTS` binds its own (`[spec:pgorm:def:sql.ast.expr.sql-json]`):
> the path never becomes statement text, and the driver, which writes a
> value in the format of the type it is, is never asked to write a
> `jsonpath`. Its bound is `Into<String>`, a path being text. It is not
> `JSON_EXISTS` under another name: checked on PostgreSQL 18.6, a GIN index
> on the column, `jsonb_ops` or `jsonb_path_ops`, serves `@?` and no index
> serves `JSON_EXISTS`, and where a strict-mode path raises an error, as
> `strict $.b` over a document without `b` does, `@?` answers NULL.
>
> PostgreSQL's SQL/JSON functions — `JSON_EXISTS`, `JSON_VALUE`, `JSON_QUERY`
> and the constructors — are the neighbouring vocabulary,
> `[spec:pgorm:def:sql.ast.expr.sql-json]`, and replace none of these. The
> replacement was weighed operator by operator on PostgreSQL 18.6 and refused
> on both counts it needed: a GIN `jsonb_ops` index serves `?`, `?|` and `?&`
> and no index serves `JSON_EXISTS`; an expression index on `->>` or its cast
> is not matched by `JSON_VALUE`; and the answers differ (string array
> elements under `?`, a failed cast that raises, an object or a boolean under
> `->>`). That rule holds the evidence and says which to reach for when.
>
> The boundary is operators, not functions. PostgreSQL's `jsonb_*` calls —
> `jsonb_set`, `jsonb_build_object`, `jsonb_array_elements`, `jsonb_typeof` and
> the rest — are ordinary function applications a caller already spells with
> `Func::named`, and naming each one here would be a second, worse function-call
> syntax. The operators pgorm does not name (`-` and `#-` key/path deletion,
> and the jsonpath match `@@`) stay reachable through `binary`: the deletions
> with `BinOper::Raw`, `@@` with `BinOper::Matches`, the variant full-text
> `matches` renders.
>
> Containment (`@>`, `<@`) and concatenation (`||`) MUST NOT be duplicated
> here: PostgreSQL defines one operator each across every type that has them,
> so `contains`, `contained` and `concat` from `sql.ast.expr.operators` are
> already the JSON forms, and a JSON-specific alias would be a second name for
> the identical AST node.
>
> `get_json_path`, `cast_json_path`, `has_any_json_keys` and
> `has_all_json_keys` MUST gather their operand collection into a single
> `text[]` `Value::Array` via `Value::array`, exactly as `sql.ast.expr.eq-any`
> does: the operators take `text[]` and nothing else, so path depth and key
> count vary without varying the statement text. The element bound MUST be
> `Into<String>`, and `has_json_key`'s key bound likewise, rather than the
> `Into<SimpleExpr>` the two field accessors take. The asymmetry is the
> operators' own: `->` selects an object key by text *or* an array element by
> integer, so its right operand is any expression, while `jsonb ? integer` is
> not an operator PostgreSQL defines, so a numeric key MUST fail to compile
> instead of reaching the server.
>
> Empty collections MUST NOT be special-cased; each operator already carries
> the right vacuous truth. `?|` over an empty key list is false for every
> operand (no key of none can be present) and `?&` is true for every operand —
> the same asymmetry `eq_any`/`ne_all` carry, and the reason neither needs
> `sql.ast.expr.in`'s constant fall-backs. An empty `#>` path selects the
> document itself.
>
> The family's operand typing is PostgreSQL's, not pgorm's, and pgorm does not
> paper over it: `#>` and `#>>` apply to `json` and `jsonb` alike, while the
> `?` family is `jsonb`-only, so a `json` operand needs an explicit `cast_as`.
> `->>` and `#>>` return `text`, which ends a chain — no JSON operator applies
> to their result. Because these combinators return `SimpleExpr` and
> `SimpleExpr` carries no JSON methods, drilling in more than one step means
> re-entering the builder with `Expr::expr`; `#>` exists so that the common
> multi-step path needs one node instead of a nest of them.

> [spec:pgorm:req:sql.ast.expr.subscript+2]
> `SimpleExpr::Subscript(base, subscript)` MUST express PostgreSQL's array
> subscript, where `Subscript` is either `Index(expr)` — one element, `a[i]` —
> or `Slice(lower, upper)` with each bound an `Option<SimpleExpr>` — `a[l:u]`,
> `a[l:]`, `a[:u]` and `a[:]`. The node is reached from every expression
> (`ExprTrait`) through `index(i)`, `slice(l, u)`, `slice_from(l)`,
> `slice_to(u)`, and `subscript(Subscript)` for the one form the shorthands
> do not name, the slice with neither bound. Each returns a `SimpleExpr`,
> which goes on to be compared, or subscripted again, in the same chain.
>
> A chain of subscripts is one multi-dimensional access, and the AST keeps it
> as nested `Subscript` nodes that render as one: `.index(1).index(2)` is
> `a[1][2]`, the element at row 1, column 2. That is not the same query as
> subscripting the first access's result, `(a[1])[2]`: PostgreSQL types a
> single-index access as the array's element type, so the parenthesised form
> is refused (`42804`, cannot subscript type `integer`) where the chain
> answers the element, and `sql.render.subscript` therefore never
> parenthesises a subscripted base.
>
> The node carries PostgreSQL's semantics and does not adjust them, so what a
> caller from a zero-based language expects is wrong in four places, each
> held by the live suite: arrays count from 1; an index outside the bounds
> answers NULL rather than raising; a slice outside the bounds is cut to them,
> and is the empty array rather than NULL when nothing overlaps; and once any
> subscript in a chain is a slice, every one is, a plain index `i` beside a
> slice reading as `1:i`.
>
> Index and bounds are any `Into<SimpleExpr>`, so a column or an expression
> can index as readily as a literal; the server coerces each to `int4`, and a
> bound value is a placeholder it types as `int4`.

> [spec:pgorm:req:sql.ast.expr.collate+2]
> `SimpleExpr::Collate(operand, collation)` MUST express PostgreSQL's
> `COLLATE`, where the collation is a `Collation`: a name and an optional
> schema, both `Name`s. It is reached from every expression (`ExprTrait`)
> through `collate(c)`, which takes any `IntoCollation` — a name converts
> bare and a `(schema, name)` pair qualified, as a table name does — and
> returns the `SimpleExpr`, which goes on to be compared or ordered in the
> same chain. The right-hand side is a collation's *name* and
> never an expression, which is why the node is its own variant rather than a
> `BinOper`: an operator would admit `a COLLATE b` for an arbitrary `b`, and
> the grammar has no such form.
>
> A collation is an identifier and nothing else, so its name is quoted like
> every other (`sql.render.collate`). Quoting is what keeps the name the
> caller wrote: PostgreSQL folds an unquoted identifier to lower case, so a
> bare `C` names a collation `c` that does not exist (`42704`), and `default`
> is a reserved word that cannot stand bare at all. `TypeName`'s part policy,
> which writes the grammar's own type spellings bare, is a type position's
> and not this one's.
>
> The clause is wanted in three positions and each is reached through one of
> two slots. An expression takes it through this node. `ORDER BY` takes it
> the same way, through `order_by_expr` and its siblings, because
> PostgreSQL's grammar has no collation slot on a sort key: `COLLATE` there is
> part of the key's expression. A column definition takes it through
> `ColumnDef::collate` (`sql.ddl.column-def`), which is a slot of the column
> rather than an expression, since the grammar spells it as a clause of the
> column.
>
> A collation decides how text compares and sorts, and the difference it
> makes is the server's to show, so the live suite holds it: the same values
> order one way under `"C"` — every upper-case letter before every lower-case
> one — and another under a linguistic collation, whichever one the server
> has; a nondeterministic collation, where the server's ICU can make one,
> makes strings that differ in bytes compare equal; a column declared with a
> collation compares by it without an expression saying so, and a retype
> moves it to another; and an unknown collation is refused (`42704`), a
> lower-case `c` included. A `compile_fail` doctest on `Collation` shows that
> an expression does not convert into one.

> [spec:pgorm:def:sql.ast.keywords+6]
> `Keyword` represents bare SQL keywords usable as expressions, and the variant
> set is closed: `Null`, `CurrentDate`, `CurrentTime`, `CurrentTimestamp` and
> `Value`, constructed by `Expr::current_date()`, `Expr::current_time()`,
> `Expr::current_timestamp()` and `Expr::domain_value()`. `Value` renders
> `VALUE`, the value a domain's `CHECK` tests (`sql.ddl.domain`); anywhere
> else PostgreSQL reads that word as a column named `value`. There is no caller-supplied keyword — an
> arbitrary word reaches keyword position only as an `Expr::raw`, which says
> raw SQL where a `Keyword` would have said identifier. Name helpers:
> `Name::runtime` wraps a runtime string as an identifier and `Asterisk`
> expresses `*` — as a bare projection or
> table-qualified via `(Table, Asterisk)` rendering `"table".*`. `ColumnRef`
> spans `Column`, `TableColumn`, `SchemaTableColumn`, `Asterisk`, and
> `TableAsterisk`; `TableName` spans plain and schema-qualified tables, and
> `FromItem` spans a `TableName` with an optional alias, `SubQuery`,
> `ValuesList`, and `FunctionCall`.

## INSERT statements

> [spec:pgorm:def:sql.ast.insert+3]
> `InsertStatement` is the INSERT AST node: a target table (`into_table`,
> taking the `NamedTable` of `[spec:pgorm:def:sql.types.table-ref]` — a name
> with an optional alias, which is the whole of what PostgreSQL's insert target
> admits, so a subquery, values list or function call cannot be inserted into,
> and an alias renders as `INSERT INTO "t" AS "a"`), a
> column list (`columns`, which replaces any previous list), a value source, an
> optional `Overriding`, an optional `OnConflict`, an optional
> `ReturningClause`, an optional WITH clause
> attached by `with(..)` (`query.build.with`), and an optional
> default-values row count. The value source (`InsertValueSource`) is either
> `Values(Vec<Vec<SimpleExpr>>)` — multi-row VALUES accumulated one row per
> `values`/`values_panic` call — or `Select(..)` set by `select_from`, which
> makes the insert read from a query (`INSERT INTO .. SELECT ..`); setting a
> select source replaces any previously accumulated rows.
>
> `or_default_values()` / `or_default_values_many(n)` record a fallback used
> only when no columns and no values were supplied, rendering
> `VALUES (DEFAULT)` repeated `n` times; when columns and values are present
> the fallback is ignored.
>
> `overriding(Overriding)` sets the statement-scoped exemption an identity
> column's generation clause is otherwise absolute about: `SystemValue`
> accepts a supplied value for a `GENERATED ALWAYS AS IDENTITY` column that
> would otherwise raise `428C9`, and `UserValue` discards one supplied to a
> `BY DEFAULT` column. The two are the whole of PostgreSQL's
> `OVERRIDING { SYSTEM | USER } VALUE`, so the slot is a closed pair rather
> than a flag, and the last call wins. It belongs to the *statement* and not to
> the column definition, which is why the exemption can exist at all without
> weakening what `identity()` declares
> (`[spec:pgorm:req:sql.ddl.column-def]`): the declaration still refuses every
> insert that does not say this word.

> [spec:pgorm:req:sql.ast.insert.arity]
> `values(row)` MUST verify that the row length equals the declared column
> count and return `Err(Error::ColValNumMismatch { col_len, val_len })` on
> mismatch, appending the row only on success. `select_from` MUST apply the
> same check between the column count and the select's projection count.
> `values_panic` and `values_from_panic` are the unwrapping variants and MUST
> panic on the same mismatch.
>
> An empty row passes the check only when zero columns are declared, and is
> then silently discarded (no row appended) — so feeding zero rows leaves the
> statement without a values source. This is the AST half of pgorm's failsafe
> behavior for empty `insert_many` operations.

## ON CONFLICT

> [spec:pgorm:req:sql.ast.on-conflict+4]
> `OnConflict` (attached with `InsertStatement::on_conflict`, which accepts
> anything converting into one) MUST be one of exactly two shapes:
> `AnyDoNothing`, carrying nothing, for the arbiter-less clause PostgreSQL
> admits only for `DO NOTHING`; or `Targeted`, pairing a `ConflictArbiter` with
> a `ConflictAction`. There is no third shape, so a clause without an action
> and a `DO UPDATE` without the arbiter PostgreSQL demands for one are both
> unrepresentable per [dec:pgorm:invalid-states-unrepresentable].
>
> `ConflictArbiter` MUST be one of the grammar's two ways of naming an
> arbiter: `Inference(ConflictTarget)`, the index inference specification,
> or `Constraint(Name)`, a constraint named outright — `ON CONSTRAINT
> "name"`. They are alternatives, not parts of one clause: a column list or a
> `WHERE` after `ON CONSTRAINT` is a syntax error. Each is therefore reached
> through its own builder on the way to an action, and neither builder has
> the other's methods.
>
> `ConflictTarget` MUST hold at least one `ConflictElement` — `Column` or
> `Expr` — plus an optional filter standing for a partial index's predicate.
> `OnConflict::column`/`expr` take the first element and `and_column`,
> `and_columns`, `and_expr`, `and_exprs` add further ones, so the empty target
> list the PostgreSQL grammar rejects has no value to build; the filter lives
> on the target because `ON CONFLICT WHERE ..` with no target is rejected too.
> `and_where`, `and_where_option` and `cond_where` MUST fold into that filter
> through the same merge `sql.ast.condition.holder` specifies.
>
> `OnConflict::columns` MUST name a whole key's columns in one call. It takes
> an `IntoKeyColumns`, the conversion a table key is built from
> (`[spec:pgorm:req:sql.ddl.create-table]`): one column or a tuple of one
> to twelve, in order, so `OnConflict::columns((a, b))` is the target
> `column(a).and_column(b)` builds, written the way `.primary_key((a, b))`
> declares the key it arbitrates on. The conversion hands back the first
> column apart from the rest and has no impl for an empty tuple, a slice or a
> `Vec`, so an empty target does not compile; a target whose columns are a
> computed list starts at `column` and extends with `and_columns`. The target
> takes the columns and not a `TableKey`, whose name, `INCLUDE`,
> `NULLS NOT DISTINCT` and deferrability an inference target has no place
> for, and which it would therefore drop. A column named twice is not
> refused: inference matches an index by its set of columns, so the server
> arbitrates `ON CONFLICT (a, b, a)` on the key `(a, b)` as it does
> `(a, b)`, where a table key naming a column twice is refused (`42701`).
>
> `OnConflict::constraint(name)` MUST yield a `ConflictConstraint`, which holds
> the name and nothing else and whose only methods are the transitions to an
> action, so the column list and the predicate that only inference takes are
> unreachable from it; `compile_fail` doctests show both. The name is a bare
> `Name`, quoted like every other identifier, with no schema: the grammar
> spells it as one identifier, because a constraint is looked up on the table
> the statement inserts into. A constraint variant of `ConflictElement` would
> have admitted a constraint beside columns and a constraint under a `WHERE`;
> the typestate admits neither.
>
> `ConflictAction` MUST be either `DoNothing`, carrying nothing, or `Update`
> holding a non-empty `ConflictAssignments` and its own optional filter —
> the action's only filter, because PostgreSQL accepts `WHERE` after
> `DO UPDATE SET ..` and not after `DO NOTHING`, whichever arbiter precedes
> it. Each arbiter's `do_nothing` yields the first; each one's
> `update_column` and `value` take the first assignment and yield a
> `ConflictUpdate`, whose `update_column`/`update_columns` add `Column`
> assignments (rendering `"col" = "excluded"."col"`), whose `value`/`values`
> add `Expr` assignments (rendering `"col" = <expr>`), and whose
> `and_where`/`and_where_option`/`cond_where` fold into the update's filter.
> A `ConflictUpdate` converts into an `OnConflict`, which is how a builder
> chain reaches the statement.
>
> Which constraint a name may reach is the catalogue's to say, not the
> builder's, so the server settles it and the live suite holds each answer.
> A unique or primary-key constraint arbitrates either action, and a
> conflict on any other constraint of the row is still raised (`23505`),
> which is what separates the named clause from the arbiter-less one. An
> exclusion constraint arbitrates `DO NOTHING` and refuses `DO UPDATE`
> (`42809`); it is also the one arbiter that only a name reaches, because
> inference looks for a unique index and finds none (`42P10`). PostgreSQL
> 18's temporal key, a primary or unique key ending `WITHOUT OVERLAPS`
> (`[spec:pgorm:req:sql.ddl.create-table]`), is enforced as one and
> arbitrates as one, which the live suite holds: `OnConflict::columns` over
> its columns, period included, finds nothing to infer (`42P10`), and its
> name takes `DO NOTHING`, an overlapping row being the conflict, and refuses
> `DO UPDATE` (`42809`). A deferrable
> constraint is refused as an arbiter (`55000`, `sql.ddl.deferrability`); a
> check constraint has no index to arbitrate by (`42809`); and a unique index
> made by `CREATE UNIQUE INDEX` is not a constraint at all, so its name is
> not found (`42704`). A name is matched as written: a mixed-case constraint
> is found because the name is quoted, and bare it would fold to lower case
> and be refused.

## RETURNING

> [spec:pgorm:def:sql.ast.returning+3]
> `ReturningClause` expresses PostgreSQL's `RETURNING`: a list, and the names
> the list reads a written row's two versions by. The list has four forms:
> every column of the target (`RETURNING *`), column references, expressions,
> and expressions each under a name (`<expr> AS "<name>"`). The `Returning`
> helper (obtained from `Query::returning()`) constructs a clause in one of
> them via `all()`, `column(..)`, `columns(..)`, `expr(..)`, `exprs(..)`,
> `expr_as(expr, name)` and `exprs_as(pairs)`. A name is the result column's,
> which is how a list reading both versions of a column tells them apart,
> `old."v"` and `new."v"` both being called `v` otherwise; it is a
> caller-supplied identifier, quoted like every other name and registered with
> the identifier oracle (`security.ident-oracle`). Insert, update, and delete
> statements accept a clause through `returning(..)`, with shorthands
> `returning_col(..)` and `returning_all()`, and a MERGE through `returning(..)`
> (`sql.ast.merge`); `SelectStatement` has no RETURNING support. Each call
> replaces any previously set clause.
>
> PostgreSQL 18 lets the list read the row as it was before the write and as
> the statement left it, through the special relations `old` and `new`.
> `ReturningRow` (`Old`, `New`) names them, and paired with a column or with
> `Asterisk` it converts into the `RowColumn` and `RowAsterisk` forms of
> `ColumnRef` (`sql.types.column-ref`), so `(ReturningRow::Old, col)` stands
> wherever a column reference does. The forms are their own, not a table
> called `old`. A table-qualified `"old"."col"` parses to the same reference,
> because a quoted lower-case name is the name the keyword folds to, but the
> AST would then say a table. The live suite holds what each statement
> answers on PostgreSQL 18.6:
>
> - `UPDATE`: `old` is the row before the update, `new` the row after it.
> - `DELETE`: `new` reads NULL in every column.
> - `INSERT`, with `VALUES` or `SELECT`: `old` reads NULL in every column,
>   except for a row that `ON CONFLICT DO UPDATE` updated, whose `old` is the
>   row as it stood. A row that `DO NOTHING` skipped, or that the update's
>   `WHERE` filtered out, returns nothing.
> - `MERGE`: each row as its action has it, an inserted row's `old` and a
>   deleted row's `new` reading NULL (`sql.ast.merge`).
> - A target column named bare reads the new row, as it did before 18, and
>   on a MERGE the deleted row as it was.
>
> The names resolve in a RETURNING list and nowhere else: in a `SET` value,
> a `WHERE` condition or a `SELECT`, `old` names no relation (`42P01`). The
> builder does not refuse a version reference there, because refusing it
> would take an expression type of RETURNING's own; the server refuses it
> instead.
>
> `old` and `new` are scoped like any relation name, so a relation of the
> statement called `old` or `new` takes the name: the target table, its
> alias, or a `FROM` or `USING` item. The server then resolves `old."col"` to
> that relation without complaint. On a target called `old`, both versions'
> references read the new row; on a `FROM` item called `new`, `new."col"`
> reads that item's column. The builder could tell this from intent only by
> walking every expression of the list, so it does not try. The rename is
> the cure. `ReturningClause::old_as(name)` and `new_as(name)` write
> `RETURNING WITH (OLD AS "name", NEW AS "name")`. A renamed version answers
> only to its new name (the keyword then names nothing, `42P01`), so the list
> reads it through an ordinary qualifier, `(name, col)`, which an `alias`
> token makes one value with the declaration. A new name that clashes with
> one of the statement's relations is refused (`42712`) rather than taken, so
> a rename is loud where the default names are silent. Each version holds at
> most one name and the last call wins, so naming `OLD` twice (`42601`)
> cannot be built, and the `WITH` list renders only when it names something,
> so the empty `WITH ()` (`42601`) cannot either. The server refuses one name
> given to both versions (`42712`). The names are caller-supplied
> identifiers, quoted like every other name and registered with the
> identifier oracle (`security.ident-oracle`).

## UPDATE and DELETE statements

> [spec:pgorm:req:sql.ast.update+5]
> `UpdateStatement` MUST accumulate SET assignments in call order as
> `(column, expression)` pairs: `values(pairs)` pushes many, `value(col, expr)`
> pushes one, and any `Into<SimpleExpr>` is accepted on the right-hand side
> (values, keywords, `Expr::raw` fragments, subqueries). Duplicate columns are
> not deduplicated — each call appends. The statement also carries the target
> `table` — the `NamedTable` of `[spec:pgorm:def:sql.types.table-ref]`, so
> the target is a name with an optional alias and nothing else, rendering
> `UPDATE "t" AS "a" SET ..` when one is bound — a FROM relation list, a WHERE
> `ConditionHolder` (per `sql.ast.condition.holder`), an optional
> `ReturningClause`, and an optional WITH clause attached by `with(..)`
> (`query.build.with`). `get_values` MUST expose the accumulated assignment
> pairs for inspection.
>
> The FROM relation list MUST accumulate through `from(..)`, exactly as
> `sql.ast.select.from` describes for a SELECT, and MUST take the same
> currency: any `IntoFromItem`, so a plain table, an aliased table, a
> subquery, a function call, a values list or a `FromItem::Template` fragment
> all stand where a relation stands. It is a relation *list*, not a join tree:
> the statement holds no `JoinExpr`, so a caller writes the join condition as
> an ordinary WHERE predicate, which is how PostgreSQL's own `UPDATE .. FROM`
> reads and what its documentation recommends. That boundary is a deliberate
> first cut rather than an oversight — `JoinExpr`, `JoinKind` and `JoinOn` are
> `pub(crate)` per `sql.surface`, so joins in the write statements' relation
> lists would mean publishing the join shape and a whole
> `inner_join`/`left_join`/`right_join`/`full_outer_join`/`cross_join` family
> on two more statement types to buy syntax the WHERE already expresses. A
> caller who needs an outer join's null-extension drives the statement from a
> subquery FROM item that performs it.
>
> The statement MUST NOT carry ORDER BY expressions or a LIMIT: PostgreSQL
> admits neither on an UPDATE. `UpdateStatement` therefore does not implement
> `OrderedStatement` and has no `limit` method, so neither clause can be built
> to be rendered; an update over an ordered, limited set of rows is expressed
> by the caller as a subquery filter (`WHERE id IN (SELECT .. ORDER BY ..
> LIMIT ..)`). A FROM relation does not substitute for that: it widens what
> the statement can read, not how many rows it touches.

> [spec:pgorm:def:sql.ast.delete+4]
> `DeleteStatement` is the DELETE AST node: a target table set by
> `from_table` — the `NamedTable` of `[spec:pgorm:def:sql.types.table-ref]`,
> a name with an optional alias, rendering `DELETE FROM "t" AS "a"` when one is
> bound — a USING relation list, a WHERE `ConditionHolder` shared with the
> condition rules, and an optional `ReturningClause`. Like the other three
> statements it carries an optional WITH clause, attached by `with(..)`
> (`query.build.with`).
>
> USING is DELETE's spelling of UPDATE's FROM and MUST behave identically:
> `using(..)` accumulates a relation list of any `IntoFromItem`, carries no
> join tree, and leaves the join condition to WHERE, with the boundary
> `sql.ast.update` states. The two clauses differ in keyword only because
> PostgreSQL's grammar does — `FROM` is already spoken for on a DELETE by the
> target table — so a single `FromItem` list serves both and one rendering
> rule covers the pair.
>
> As with `sql.ast.update`, the statement MUST NOT carry ORDER BY expressions
> or a LIMIT — PostgreSQL admits neither on a DELETE — so it implements no
> `OrderedStatement` and offers no `limit`, and an ordered or limited delete is
> written as a subquery filter over a SELECT.

## MERGE statements

> [spec:pgorm:req:sql.ast.merge+2]
> `MergeStatement` is the MERGE AST node. It holds a target `NamedTable`
> (`[spec:pgorm:def:sql.types.table-ref]`: a name, bare or
> schema-qualified, with an optional alias), an `only` flag, a source
> `FromItem`, the join condition, an optional plain `WithClause`, its
> `WHEN` arms, and an optional RETURNING list. `Query::merge(target, source,
> on)` takes the three required
> parts: any `IntoNamedTable`, any `IntoFromItem` (a table, a subquery, a
> values list, a function call or a template fragment, the currency of
> `sql.ast.select.from`) and any `IntoCondition`. It returns a
> `PendingMerge`, which is not a statement: it implements neither
> `QueryStatementBuilder` nor `Display`. Each of its six methods
> (`when_matched`, `when_matched_and`, `when_not_matched`,
> `when_not_matched_and`, `when_not_matched_by_source` and
> `when_not_matched_by_source_and`) adds the first arm and returns the
> `MergeStatement`, so a MERGE with no WHEN clause (`42601`) has no
> constructor. `MergeStatement` has the same six methods as `&mut self`
> builders, plus `with(WithClause)`, `only()`, `returning(ReturningClause)`
> and `returning_action()`.
>
> There are three kinds of row, as in PostgreSQL 17's grammar: a target row
> the join condition paired with a source row (*matched*), a source row it
> paired with no target row (*not matched*, which PostgreSQL also spells
> `NOT MATCHED BY TARGET`), and a target row it paired with no source row
> (*not matched by source*). An arm's action MUST be typed by the kind of
> row the arm takes. `when_matched*` and `when_not_matched_by_source*` take
> any `Into<MatchedAction>`, the actions on a target row: `Update(MergeUpdate)`,
> `Delete` or `DoNothing`. `when_not_matched*` takes any
> `Into<NotMatchedAction>`: `Insert(MergeInsert)`, `InsertDefaultValues` or
> `DoNothing`. So an INSERT for a target row and an UPDATE or DELETE for an
> unmatched source row, each `42601`, do not typecheck, and `compile_fail`
> doctests show all three. A not-matched-by-source row has no source row,
> so its arm's condition and its update can read only the target; a
> reference to the source there is refused (`42P01`), which the expression
> types cannot see, so the server settles it as it settles the target
> reference a not-matched arm's condition may not make. `MergeUpdate` and `MergeInsert` hold `(column, expression)`
> pairs and are non-empty by construction: `value(col, expr)` takes the
> first pair, and `and_value` and `and_values` append further pairs in
> order. The pairing keeps an insert's column list and its `VALUES` row the
> same length, which a separate list and row could break, so the positional
> `INSERT VALUES (..)` with no column list is not offered. A column is a bare
> `Name`: PostgreSQL resolves it against the target and refuses one
> qualified by the table (`42703`). `MergeInsert::overriding` sets
> `OVERRIDING SYSTEM VALUE` or `OVERRIDING USER VALUE`, and the last call
> wins. It is reachable on the values form only, because
> `INSERT OVERRIDING .. DEFAULT VALUES` is `42601`.
>
> Each kind's arms MUST be held as its conditional arms in call order,
> followed by at most one unconditional arm. A row takes the first
> conditional arm of its kind whose condition holds, so the order is part of
> the statement's meaning, and the unconditional arm takes the rows they
> leave. PostgreSQL refuses an arm after an unconditional arm of the same
> kind as unreachable (`42601`), for each of the three kinds. That error is
> raised in parse analysis, so libpg_query alone would accept the statement.
> This builder cannot represent it: `when_matched`, `when_not_matched` and
> `when_not_matched_by_source` set the kind's one unconditional arm, the last
> call wins, and the arm renders after the kind's conditional arms whenever
> it was added. The kinds never compete for a row, so holding them apart
> changes nothing.
>
> `returning` sets the RETURNING list, a `ReturningClause`
> (`sql.ast.returning`), and the last call wins. The list reads the source
> row and the target row, whose two versions `ReturningRow` names. A column
> named bare resolves against both relations, so a name both have is
> ambiguous (`42702`) and is qualified by its table or alias, and `*` is the
> source's columns followed by the target's. A target column reads the row
> the merge left, or the deleted row as it was; `old` reads NULL in every
> column of an inserted row and `new` in every column of a deleted one; a
> row an arm left alone with `DO NOTHING` is not returned. `returning_action()` adds `merge_action()` as the list's first
> column, the text `INSERT`, `UPDATE` or `DELETE` naming the action the row
> took, and with no list it is the list. That method is the only constructor
> of `merge_action()`. PostgreSQL resolves the function in a MERGE's
> RETURNING list (a subquery within the list included) and refuses it
> anywhere else, a `WHEN` condition, another statement's RETURNING or a
> SELECT (`42601`). An expression variant would place it anywhere, so the
> builder has none, and it cannot be written outside a MERGE's list.
> `Func::named("merge_action")` does not reach it either: the name is a
> keyword PostgreSQL restricts, so the function-name policy quotes it, and
> the quoted name is no function (`42883`).
>
> `with` takes a `WithClause` rather than `AnyWithClause`, because
> PostgreSQL refuses `WITH RECURSIVE` on a MERGE (`42601`). A data-modifying
> common table expression is accepted. `only()` writes `ONLY` before the
> target, so the rows of tables that inherit from it are neither matched nor
> written.
>
> Every value travels through the bound-parameter pipeline of
> `sql.ast.build`, whether it sits in the join condition, an arm's
> condition, an assignment or the inserted row. The server types such a
> parameter by the column it is assigned to or compared with. A value in the
> source relation is assigned to no column, so a bound `VALUES` row, or an
> argument of a function called by name, has only the type an unknown
> parameter gets. Annotating it is the caller's obligation under
> `sql.render.placeholder-typing`, and the live suite holds the refusal
> (`42883`).
>
> A MERGE nests as a common table expression's body (`sql.ast`). No ORM path
> emits MERGE. The upsert an entity layer writes is `ON CONFLICT`
> (`sql.ast.on-conflict`). A caller runs a MERGE by building it and
> executing the `(String, Values)` pair, and with a RETURNING list reads the
> rows it wrote.

## COPY statements

> [spec:pgorm:def:sql.ast.copy+2]
> `CopyFromStatement<F>` (`COPY … FROM STDIN`) and `CopyToStatement<F>`
> (`COPY … TO STDOUT`) are the COPY AST nodes: the statements that open
> COPY's data flow, which `conn.copy` runs. Each is made by `new` with what
> PostgreSQL cannot do without. `CopyFromStatement::new(table, format)` takes
> the table, any `IntoTableName`, bare or schema-qualified.
> `CopyToStatement::new(source, format)` takes its source, any
> `IntoCopySource`, which converts into a `CopySource`: a table, or the rows
> of one of the five statements `IntoSubQueryStatement` covers (`sql.ast`),
> a `SELECT`'s or a write's `RETURNING` list. PostgreSQL refuses a write
> without one there (`0A000`), when the COPY starts. A COPY TO over one MUST
> NOT be buildable. `CopySource`'s contents are private, so it holds only
> what a conversion into it accepted. A table name and a `SELECT` are
> `IntoCopySource`. An INSERT, UPDATE, DELETE or MERGE, or a
> `SubQueryStatement`, converts through `CopySource`'s `TryFrom`, which
> refuses a write with no `RETURNING` list as
> `Error::CopyWithoutReturning`, naming the write. A write is no
> `IntoCopySource`, so handing one to `new` directly is a compile error. A
> MERGE's `returning_action` is a `RETURNING` list by itself. The check reads
> the source statement's own clauses, as the server's does: a `SELECT` whose
> data-modifying CTE returns nothing is a source, the server running that
> CTE as it runs one in any query (`sql.ast.with`). Neither statement is a
> `SubQueryStatement` or a `QueryStatementBuilder`: COPY nests nowhere, and it
> binds nothing, so its rendering is `Display` alone (`sql.render.copy`).
>
> A `SELECT` source carries what the server refuses of any `SELECT` when the
> COPY starts: a tie limit with no ORDER BY or beside `SKIP LOCKED`
> (`sql.ast.select.tie-limit`), and a WINDOW clause, or a reference to one,
> that it refuses (`sql.ast.window-statement`). A `SELECT` converts into a
> source infallibly, so the conversion does not check it.
> `CopyToStatement::check_source` runs the source's `check_tie_limit` and
> `check_windows`, and finds nothing to refuse in a table or a write, and every
> pgorm executor of a COPY TO and the Python and JavaScript builders run those
> checks before it renders (`conn.copy`, `[dec:pgorm:no-panic]`). A source is
> final once converted, so the `SELECT` checked is the one that renders.
> Rendering is infallible, as a `SELECT`'s is, so a caller of pgorm-query alone
> who skips the check sends SQL the server refuses. The check reads the
> source's own clauses: a `SELECT` nested in it, a write's included, is the
> server's refusal.
>
> The format is the statement's type parameter, chosen at construction from
> the closed set PostgreSQL 18 has: `CopyText`, `CopyCsv` and `CopyBinary`,
> unit types implementing the sealed `CopyFormat`; `CopyDelimitedFormat`, also
> sealed, marks the two text formats. An option MUST be a method only where
> the server accepts it in that direction and that format, so a combination
> it refuses has no spelling:
>
> - Every COPY FROM: a column list (`columns`, which a later call extends;
>   none supplies every column in the table's order), `FREEZE` (`freeze`), and
>   a `WHERE` through `ConditionalStatement`.
> - Text and CSV, both directions: `DELIMITER`, `NULL`, `HEADER` and `ENCODING`
>   (`delimiter`, `null_string`, `header`, `encoding`). COPY FROM adds
>   `DEFAULT` (`default_string`), `HEADER MATCH` (`header_match`) and
>   `ON_ERROR ignore` (`on_error_ignore`).
> - CSV, both directions: `QUOTE` and `ESCAPE` (`quote`, `escape`). COPY FROM
>   adds `FORCE_NOT_NULL` and `FORCE_NULL`, COPY TO adds `FORCE_QUOTE`, each over
>   named columns or over every column (`*`, the `_all` method).
> - Binary takes none of the text options. The server refuses `HEADER`,
>   `NULL`, `DEFAULT` and `ON_ERROR ignore` in binary mode (`0A000`), and
>   accepts `ENCODING` there without it changing a byte that is sent, so the
>   type leaves that out as well.
>
> `ON_ERROR stop` is the server's default and has no value, as `NOT
> DEFERRABLE` has none (`conn.tx`): selecting it says nothing. `CopyIgnore`
> carries the two options that mean nothing without `ignore`, so neither can be
> written without it: `REJECT_LIMIT`, a `NonZeroU32` because the server refuses
> zero, and `LOG_VERBOSITY` (`CopyLogVerbosity::Verbose` or `Silent`; no value
> is the server's default).
>
> COPY TO has no column list. `COPY t (a, b) TO STDOUT` copies what `COPY
> (SELECT a, b FROM t) TO STDOUT` copies, and the query form can also filter,
> order and join, so a subset of a table's columns is a `SELECT`.
>
> What turns on a value or on the catalog is left to the server, which
> refuses it when `conn.copy` starts the statement: a delimiter, quote or
> escape that is not a single one-byte character (`0A000`), a delimiter the
> text format reserves or a newline (`22023`), `NULL` and `DEFAULT` strings
> that are equal or hold the delimiter (`22023`), a forced column the COPY
> does not name (`42P10`), an encoding the server does not know (`22023`), and
> `FREEZE` on a table not created or truncated in the current subtransaction
> (`55000`). An empty forced-column list is no option at all: `FORCE_QUOTE ()`
> is a syntax error, and forcing no column is the option left out. COPY to or
> from a file on the server, and through `PROGRAM`, is not built
> (`sql.scope`).

> [spec:pgorm:def:sql.ast.copy.parts]
> A COPY statement's parts are read back through getters, so what a COPY
> carries can be described without rendering it (`conn.copy.rows`):
> `CopyFromStatement::get_table` and `get_columns`, its table and its column
> list, empty when it names none; and `CopyToStatement::get_source`, its
> `CopySource`, whose `get_table` and `get_query` give the table or the
> statement it reads, the other `None`. They borrow: nothing a getter returns
> changes the statement, so a source still holds only what its conversion
> accepted.

## WITH clauses and CTEs
## WITH clauses and CTEs

> [spec:pgorm:def:sql.ast.with+5]
> `CommonTableExpression` defines one named query in a WITH clause and MUST be
> complete the moment it exists: `CommonTableExpression::new(table_name, query)`
> takes both mandatory parts. The query is any statement implementing
> `IntoSubQueryStatement`, stored as a `SubQueryStatement`, a MERGE included
> (`sql.ast`). The AST does not restrict INSERT/UPDATE/DELETE/MERGE CTEs;
> validity is left to PostgreSQL, which runs a write CTE without RETURNING
> and refuses only a read of it (`0A000`), and refuses a write CTE below
> the top level (`0A000`). Only the genuinely optional parts remain
> builder methods: the column list (`column`/`columns`) and the `materialized`
> flag rendering `MATERIALIZED` / `NOT MATERIALIZED`.
> `CommonTableExpression::from_select` derives a CTE from a `SelectStatement`,
> naming it `cte_<table>` after the first FROM table and deriving column names
> from aliases or plain column projections; it returns `Option<Self>`, yielding
> `None` when the select has no FROM table to take a name from rather than
> producing a nameless CTE. `try_set_cols_from_select` performs only the column
> derivation and reports `false` (leaving columns untouched) when any projection
> is an expression or wildcard it cannot name.
>
> The two shapes a WITH clause can take are distinct types rather than a
> `recursive` flag. `WithClause` is the non-recursive form and holds a non-empty
> CTE collection: `WithClause::new(cte)` takes the first, `cte` appends further
> ones, and `ctes` iterates them in order. `RecursiveWithClause` is the
> recursive form, described by `sql.ast.with.recursive`. `AnyWithClause` is the
> closed sum of the two. `Into<AnyWithClause>` is what `stmt.with(clause)`
> accepts on each of select, insert, update and delete, and
> `MergeStatement::with` accepts the plain `WithClause` alone
> (`query.build.with`). Those setters are the only ways a clause reaches a
> statement.
>
> A clause has exactly one home, the statement that carries it. There is no
> wrapper type holding a clause plus the statement it prefixes, and therefore no
> `WithBody` bound to keep a select out of one: the invalid state that bound
> guarded is unconstructible because the second home no longer exists
> (`query.build.with.single`).

> [spec:pgorm:req:sql.ast.with.recursive+1]
> The recursive WITH form MUST be a distinct type, `RecursiveWithClause`, whose
> single `CommonTableExpression` is taken by `RecursiveWithClause::new` — a
> recursive WITH consists of exactly one CTE containing a union query, and the
> multi-CTE recursive clause is therefore not constructible. It renders
> `WITH RECURSIVE` and carries the optional `SEARCH` (`search`) and `CYCLE`
> (`cycle`) clauses, which only this form accepts, so no setting can be
> silently ignored. Neither this type nor `WithClause` can be built empty, so
> rendering asserts nothing and MUST NOT panic on any clause a caller can
> construct.
>
> `Search::new(order, expr, alias)` pairs a `SearchOrder` (`BREADTH`/`DEPTH`)
> with the expression tracking the path and the name of the generated order
> column; the name is a constructor argument rather than an optional alias on a
> `SelectExpr`, so the missing-alias failure cannot arise. `Cycle::new(expr,
> set, using)` likewise requires the node-identifying expression, the cycle-mark
> column name, and the path column name at construction, rendering
> `CYCLE <expr> SET <set> USING <using>`.

## Window statements

> [spec:pgorm:def:sql.ast.window-statement+8]
> `WindowStatement` describes an OVER window: PARTITION BY expressions
> (`partition_by`, and the `OverStatement` trait's
> `partition_by_columns`), ORDER BY expressions (shared
> `OrderedStatement` trait), and an optional `FrameClause`, set by
> `frame(f)` from anything `Into<FrameClause>`; a second call replaces the
> first.
>
> `FrameType` is all three of PostgreSQL's frame modes — `Range`, `Rows`,
> `Groups` — because the offset means something different under each and none
> of the three is expressible through the others: `Rows` counts rows, `Range`
> counts a distance in the ordering column's own values, and `Groups` counts
> whole peer groups, so `GROUPS 1 PRECEDING` reaches back past every row tied
> with the one before.
>
> A frame is begun from its mode, and its start is typed by the side of the
> current row it lies on, because PostgreSQL's grammar refuses every frame
> whose end comes before its start and none of those MUST construct
> (`[dec:pgorm:invalid-states-unrepresentable]`). `FrameType`'s four methods
> name the start — `unbounded_preceding()`, `preceding(offset)`,
> `current_row()`, `following(offset)`, and no `UNBOUNDED FOLLOWING` — and
> return a `FrameStart<S>`, whose marker `S` is `FramePreceding`,
> `FrameCurrentRow` or `FrameFollowing`. Its `and_*` methods give the frame an
> end, `BETWEEN <start> AND <end>`, and each side offers only the ends that may
> follow it: after a preceding start `and_preceding(offset)`,
> `and_current_row()`, `and_following(offset)` and `and_unbounded_following()`;
> after `CURRENT ROW` all but `and_preceding`; after a following start only
> `and_following` and `and_unbounded_following`. A preceding or current-row
> start also stands alone as a whole frame, converting into a `FrameClause`,
> and a following start does not, because PostgreSQL reads a lone start as
> running to the current row, which lies behind it. `ROWS UNBOUNDED
> FOLLOWING`, `… AND UNBOUNDED PRECEDING`, `BETWEEN CURRENT ROW AND 1
> PRECEDING`, `BETWEEN 1 FOLLOWING AND CURRENT ROW` and `ROWS 1 FOLLOWING`
> therefore have no construction. The public `Frame` enum, which could spell
> all five, is gone; the bound is the crate's own `FrameBound`.
>
> An offset is any `Into<SimpleExpr>`, not a count, because under `Range` it is
> a value of the type PostgreSQL pairs with the ordering column's — an
> `interval` over a timestamp, a `numeric` over a numeric — which a count
> cannot spell. The builder does not know the ordering column's type, so the
> pairing is the server's to check, and these refusals are the server's: an
> offset whose type the column's does not pair with (`0A000` under `Range`,
> and `42804` under `Rows` and `Groups`, whose offsets are `bigint` counts); a
> `Range` offset in a window without exactly one ORDER BY column (`42P20`); a
> `Groups` frame in a window with no ORDER BY (`42P20`); an offset that reads a
> column (`42P10`); and a negative one (`22013`). A bound offset is a `$N` the
> server types from the same pairing — `interval`, `numeric` or `bigint` — so
> a text value bound where it expects an `interval` fails at bind, and the
> offset is spelled with a cast (`sql.render.placeholder-typing`).
>
> `FrameClause::exclude(e)` adds the `EXCLUDE` clause, `FrameExclusion` being
> `CurrentRow`, `Group`, `Ties` or `NoOthers`; a second call replaces the
> first, and a lone start has the same method. The clause is grammatical only
> inside a frame, and it is a method of the frame rather than of the window,
> so a window without a frame has nothing to call it on. Each exclusion is
> relative to the current row's peers — the rows the window's ORDER BY ties
> with it — so `Group` and `Ties` differ from `CurrentRow` only where the
> ordering ties, and `NoOthers` removes nothing: it is the default, spelled.
>
> A select projection references a window in one of three ways
> (`WindowSelectType`): `Query` embeds a window of its own inline, `Extended`
> embeds one built on an entry of the statement's WINDOW clause, `OVER (
> "w" ORDER BY ... )` — `expr_window` and `expr_window_as` take either, as a
> `WindowDefinition` — while `Name` references a named window
> (`expr_window_name`, `expr_window_name_as` render `OVER "w"`) that the
> statement's WINDOW clause declares. The clause is a list, as
> PostgreSQL's is: `SelectStatement::window(name, window)` appends an entry,
> `"w" AS ( ... )`, the entries render in call order, and a statement reads as
> many named windows as it declares. An entry's window is a
> `WindowDefinition`, which a `WindowStatement` and an `ExtendedWindow` both
> convert into. `ExtendedWindow::new(base)` is a window built on an earlier
> entry, `"w2" AS ( "w1" ORDER BY ... )`, which PostgreSQL makes copy the
> base's PARTITION BY and any ORDER BY it has: it carries an ORDER BY (the
> shared `OrderedStatement` trait) and a frame (`frame(f)`, a second call
> replacing the first), and no PARTITION BY, because PostgreSQL refuses one
> there (`42P20`), so none has a construction.
>
> What PostgreSQL refuses of a clause turns on its other entries and on the
> projection, which separate calls build in any order, so it is not designed
> out (`[dec:pgorm:invalid-states-unrepresentable]`): two entries of one name
> (`42P20`); an extended window whose base is no earlier entry, a later one
> included (`42704`), whose base has a frame (`42P20`), or which adds an
> ORDER BY to a base that orders its rows, by its own ORDER BY or one copied
> down a chain of extended windows (`42P20`); a projection over a name no
> entry declares (`42704`); and a projection over an inline extended window
> held to the same three refusals as an extended entry, except that its base
> may be any entry, earlier or later, since PostgreSQL reads the inline
> window after the whole clause — the refusal then names no window, the
> inline one having no name. `check_windows` refuses each before anything
> renders — `Error::WindowRedefined`, `Error::WindowBaseUndefined`,
> `Error::WindowFrameCopied`, `Error::WindowOrderOverridden` and
> `Error::WindowUndefined` — and it runs wherever `check_tie_limit` does,
> both reached through `check_clauses` (`sql.ast.select.tie-limit`,
> `[dec:pgorm:no-panic]`): in every pgorm executor, in the Python and
> JavaScript builders, before a COPY TO over a `SELECT` (`sql.ast.copy`), and
> in the constructors of a view, a materialized view and a `CREATE TABLE AS`
> on the body they take (`sql.ddl.view`, `sql.ddl.materialized-view`,
> `sql.ddl.create-table-as`). Names are compared as PostgreSQL compares the
> quoted identifiers pgorm writes: by their text, case-exactly, whatever type
> minted them. Rendering is infallible and writes what the statement holds
> (`sql.render.window`), so a caller of pgorm-query alone who renders a
> `SELECT` without the check sends SQL the server refuses with those codes; a
> stored definition's body cannot, since none of those statements constructs
> over a body the check refuses.
> The check reads the statement's own clauses: a SELECT nested in it names
> its windows for itself, and one carrying a refused clause is the server's
> refusal.
>
> PostgreSQL accepts `OVER` only after a function call, so all four
> `expr_window*` constructors MUST take the windowed expression as a
> `WindowFunction` rather than anything convertible to `SimpleExpr` (per
> `[dec:pgorm:invalid-states-unrepresentable]`): a windowed column reference,
> arithmetic expression, `CASE` or `CAST` does not typecheck, so the AST with
> no valid rendering has no constructor. `WindowFunction` is sealed and
> implemented by the three things the grammar puts before `OVER`: a
> `FunctionCall`, and the SQL/JSON aggregates `JsonArrayAgg` and
> `JsonObjectAgg` (`sql.ast.expr.sql-json`), which are calls in the grammar
> but carry clauses a `FunctionCall` cannot. `SelectExpr`'s expression and window
> are therefore read-only after construction — `expr()` and `window()` read
> them, `SelectExpr::new`/`new_as` build the windowless forms, and the four
> constructors are the only source of a windowed one — so the pairing cannot
> be taken apart by mutating a projection in place. The alias stays writable:
> renaming a projection cannot invalidate the pairing.

## CASE expressions

> [spec:pgorm:def:sql.ast.case+3]
> PostgreSQL's two CASE forms are two types, because their arms have two
> shapes and a CASE mixing them has no spelling.
>
> `CaseStatement` builds the searched form: each `case(cond, then)` call
> appends a `WHEN <condition> THEN <result>` arm — the condition is any
> `IntoCondition`, so `Condition` trees render with their `AND`/`OR`/`NOT`
> structure inside the WHEN — and `finally(expr)` sets the optional `ELSE`
> result. `Expr::case(cond, then)` is its only constructor and takes the first
> arm, so a searched CASE holds at least one arm from the start: there is no
> armless `new()` and no `Default`. A `CaseStatement` converts into
> `SimpleExpr::Case`. `get_arms()` reads its arms back in order, each as its
> condition and its result, the condition as the expression it means — a
> `Condition` tree joined by its `AND`s and `OR`s and negated where it is, so
> one built from a single expression is that expression — and `get_else()`
> its `ELSE` result.
>
> `SimpleCaseStatement` builds the simple form, `CASE <operand> WHEN <value>
> THEN <result> … END`, whose arms hold *values* the one operand is compared
> with. `Expr::case_of(operand)` takes the operand and returns a
> `CaseOperand`, whose only method, `when(value, then)`, adds the first arm
> and yields the `SimpleCaseStatement`; that type's own `when` appends further
> arms in call order and `finally(expr)` sets the `ELSE`. Neither type offers
> the other's arm method, so a condition arm cannot join an operand CASE nor a
> value arm a searched one. The operand is a constructor argument rather than
> an optional field beside the arms, and the first arm is a step rather than a
> default: PostgreSQL's grammar requires at least one `WHEN`, so `CaseOperand`
> does not convert into an expression, and a simple CASE with no arm has no
> value to build. A `SimpleCaseStatement` converts into `SimpleExpr::SimpleCase`.
>
> The simple form is not an abbreviation of a searched CASE over `IS NULL`
> tests. Each arm is the comparison `operand = value`, which is unknown when
> either side is NULL, so a NULL operand matches no arm — `WHEN NULL`
> included — and yields the `ELSE`, or NULL without one; the searched form's
> `WHEN x IS NULL` is the spelling that catches it.
>
> Either form can be projected (with `expr_as`), compared, nested in the
> other's results, or used anywhere an expression is accepted. Neither can be
> built without a `WHEN` — the searched form's constructor takes the first arm
> and the simple form's operand step converts into nothing — so `(CASE END)`
> and `(CASE ELSE … END)`, which the grammar rejects, have no construction
> (`[dec:pgorm:invalid-states-unrepresentable]`).

## Casts

> [spec:pgorm:req:sql.ast.cast-shape+2]
> A cast has exactly ONE node shape: `SimpleExpr::AsEnum(TypeName, operand)`.
> Every spelling that produces one — `as_enum`, `cast_as`, `cast_as_type`
> (`ExprTrait`'s, on every expression), `as_range`
> (`[spec:pgorm:def:sql.value.created-range]`), the entity layer's enum and
> created-range casts, and the `cast_as_raw` escape hatch — builds
> that node, and there is no `Function::Cast`. Whether the type renders as a
> quoted identifier or as the caller's own verbatim text is carried *inside*
> the `TypeName` (`[spec:pgorm:def:sql.types.type-name]`), never by choosing
> a different node.
>
> What this forbids is the second, `FunctionCall`-shaped cast whose type rode
> as a `SimpleExpr::Raw` operand, and it forbids it for two reasons.
> A consumer reading a cast back off an expression must recognise one shape
> rather than enumerate them: `source_read_cast` recognising only the
> structured shape is why a `#[pgorm(select_as = "…")]` column's read cast
> was silently dropped, so the graph and pipeline decoded the column's stored
> type instead of its cast one — a defect no test of either shape alone could
> have found. And raw text in expression position is an injection site by
> construction; confining verbatim type text to one field of `TypeName`, set
> only by `TypeName::raw` and only from a `&'static str`, makes the places that
> can emit an unescaped type name a finite list that inspection can enumerate:
> the callers of `TypeName::raw`, `cast_as_raw` among them.

## Function calls

> [spec:pgorm:def:sql.ast.func+12]
> `FunctionCall` pairs a `Function` selector with argument expressions and
> per-argument modifiers (`FuncArgMod { distinct }`); `arg` appends one
> argument, `args` replaces the argument list. `get_func()` and `get_args()`
> read the two back, and `is_plain()` says the call carries nothing else — no
> `DISTINCT` argument and no aggregate clause — which is what a reader that
> rebuilds a call from those two needs to know (`codegen.entity.transform`). The `Function` enum covers the
> built-ins with typed constructors on the `Func` helper: aggregates `max`,
> `min`, `sum`, `avg`, `count`, `count_distinct` (the DISTINCT argument
> modifier), `bit_and`, `bit_or`; the ordered-set aggregates
> `percentile_cont` and `percentile_disc`; scalar helpers `abs`,
> `char_length`, `if_null`, `coalesce`, `lower`, `upper`, `round`,
> `round_with_precision`, `random`, `starts_with`, `gen_random_uuid`; the
> PostgreSQL full-text family `to_tsquery`, `to_tsvector`,
> `phraseto_tsquery`, `plainto_tsquery`, `websearch_to_tsquery` (each with an
> optional `regconfig` OID prepended as first argument), `ts_rank`,
> `ts_rank_cd`; array/subquery comparators `any`, `some`, `all`; the
> set-returning `generate_series`, `generate_series_step` and `unnest`
> (`sql.ast.select.table-function`); the record-returning `json_to_record`,
> `jsonb_to_record`, `json_to_recordset` and `jsonb_to_recordset`, which a
> column definition list gives their columns
> (`sql.ast.select.column-definitions`); and PostgreSQL 18's UUID family,
> below.
>
> The UUID family is `uuidv4()`, `uuidv7()`, `uuidv7_shifted(shift)`,
> `uuid_extract_timestamp(u)` and `uuid_extract_version(u)` (`func_uuid.rs`),
> rendering `UUIDV4`, `UUIDV7`, `UUID_EXTRACT_TIMESTAMP` and
> `UUID_EXTRACT_VERSION`. `uuidv4` is PostgreSQL 18's name for what
> `gen_random_uuid` returns, and both stay: `gen_random_uuid` is the spelling
> every release since 13 resolves. `uuidv7` mints a time-ordered UUID whose
> leading 48 bits are the Unix time in milliseconds; the live suite holds that
> values a session mints sort strictly in the order it minted them, even
> within one millisecond, so a `uuidv7()` column default fills a key that
> indexes like a sequence. A column takes one as its
> `DEFAULT` the way it takes any expression, `ColumnDef::default(Func::uuidv7())`
> (`[spec:pgorm:req:sql.ddl.column-def]`), and an entity as
> `#[pgorm(default_expr = Func::uuidv7())]`, `Func` being a prelude member for
> that reason (`[spec:pgorm:def:entity.prelude]`). `uuidv7_shifted` is
> `uuidv7`'s one-argument overload, which moves the embedded time by an
> `interval` and binds it as any argument is bound; PostgreSQL has no other
> `uuidv7` arity (`uuidv7(1)` is `42883`). The two readers are immutable, so
> they can stand in an expression index or a generated column, and each answers
> `NULL` rather than failing where it has nothing to read: the timestamp, at
> millisecond precision, only for a version 7 or version 1 UUID, and the
> version only for a UUID of the RFC 9562 variant.
>
> Beyond its arguments a call carries the two clauses PostgreSQL admits after
> them, each optional and each empty by default:
>
> - A FILTER condition, set by `filter(cond)` taking any `IntoCondition`.
>   Each call REPLACES the previous one rather than accumulating — a
>   conjunction is a single `Condition`, so accumulation would give one
>   meaning two spellings and make `filter` the only builder method on the
>   type whose second call means something other than its first.
> - A WITHIN GROUP ordering, a `Vec<OrderExpr>` accumulated by
>   `within_group(col, order)` and `within_group_expr(expr, order)`, mirroring
>   `order_by` / `order_by_expr` of `sql.ast.order`. It accumulates because
>   the hypothetical-set aggregates rank against several columns at once.
>   `OrderExpr`'s fields are not public, so these constructors are the only
>   way to build the payload and there is no list-taking form to offer.
>
> The pair is stored as one `Option<Box<..>>` rather than as two inline
> fields, and that is a size decision rather than a stylistic one.
> `SimpleExpr::FunctionCall` holds a `FunctionCall` inline, and `SelectExpr`,
> `Search` and `Cycle` hold a `SimpleExpr` inline in turn, so every byte here
> is paid by the whole expression tree — including by the scalar calls
> (`LOWER`, `COALESCE`, `ROUND`) that can never carry either clause. Two
> inline fields grew `AnyWithClause` past clippy's `large_enum_variant`
> threshold; one boxed field that is `None` until a clause is set costs eight
> bytes and one allocation only on the calls that actually aggregate.
>
> `get_func`, `get_args`, `get_mods`, `get_filter` and `get_within_group`
> expose the whole of a call for inspection: a consumer outside this crate
> reads a `FunctionCall` only through these, so a field added without one
> would be invisible to it — and since the modifiers are stored boxed and
> merged, the accessors are what make that storage an implementation detail
> rather than a shape a caller has to know. `get_within_group` MUST answer
> the empty slice, not an `Option`, when no ordering was supplied: absent and
> empty are the same statement about the call.
>
> The AST does not police which functions the two clauses are meaningful on.
> `FILTER` is valid on any aggregate and `WITHIN GROUP` only on an ordered-set
> or hypothetical-set one, but that is a property of the function the server
> resolves — including `Function::Named` ones this enum has never heard of —
> so the check belongs to PostgreSQL, which makes it, and not to a builder
> that would have to guess.
>
> Both clauses stop at this crate's own builders. `pgorm::pipeline`'s
> aggregate verbs do NOT expose them, and that is a boundary rather than an
> omission: the pipeline's SQL is written by prqlc from its RQ, never by this
> renderer, and prqlc has no representation to carry either clause — every
> function it constructs in `sql/gen_expr.rs` is built with `filter: None`
> and `within_group: vec![]`, with no PL or RQ construct that reaches them.
> The only route would be the written-SQL mechanism
> `[spec:pgorm:sem:pipeline.count-argument]` uses, which would mean
> respelling every aggregate verb as a written call and re-deriving its
> interaction with the OVER attachment those rewrites already perform. That
> is its own piece of work, not a pass-through, and until it is done a
> pipeline caller writes conditional aggregation the way PRQL does: filtering
> before the group, or aggregating over a `case`.
>
> A cast is not among them. `CAST` is written by `SimpleExpr::AsEnum`
> (`[spec:pgorm:req:sql.ast.cast-shape]`), so there is no `Function::Cast`
> and no `Func` constructor that produces one — a consumer matching on a
> `FunctionCall` never has to consider a cast. Nor is `GROUPING`: the
> grammar spells it as an expression of its own, so `Func::grouping` returns
> the `Grouping` of `sql.ast.select.grouping` rather than a `FunctionCall`.
> Nor are SQL/JSON's functions, for the same reason: `Func::json_exists`,
> `json_value`, `json_query`, `json_object`, `json_array`, `json_array_query`,
> `json_objectagg`, `json_arrayagg`, `json`, `json_scalar` and
> `json_serialize` return the builders of `sql.ast.expr.sql-json`, and
> `json_table` the FROM item of `sql.ast.json-table` (`func_json.rs`).
>
> `Func::named(name)` calls an arbitrary function by identifier
> (`Function::Named`). A `FunctionCall` converts into
> `SimpleExpr::FunctionCall`, and can serve as a FROM item through
> `SelectStatement::from_function` or a `TableFunction`.

## SQL/JSON

> [spec:pgorm:def:sql.ast.expr.sql-json+2]
> PostgreSQL's SQL/JSON — the query functions `JSON_EXISTS`, `JSON_VALUE` and
> `JSON_QUERY`, the constructors `JSON_OBJECT`, `JSON_ARRAY` (over values or
> over a one-column query), `JSON_OBJECTAGG`, `JSON_ARRAYAGG`, `JSON()`,
> `JSON_SCALAR()` and `JSON_SERIALIZE()`, and the `IS [NOT] JSON` predicate —
> is one expression node, `SimpleExpr::SqlJson(Box<SqlJson>)`, whose `SqlJson`
> enum has a variant per form. They are not `FunctionCall`s: their clauses
> (`PASSING`, `RETURNING`, wrapper and quotes behaviour, `ON EMPTY` /
> `ON ERROR`, `ON NULL`, `WITH UNIQUE KEYS`, `FORMAT JSON`) are grammar between
> the parentheses, which `Func::named`'s argument list cannot spell. Each form
> is built by a `Func` constructor returning its own builder, as `Func::grouping`
> returns a `Grouping` (`[spec:pgorm:def:sql.ast.func]`): `json_exists`,
> `json_value` and `json_query` take a context item and a path; `json_object()`
> and `json_array()` start empty; `json_array_query(select)`,
> `json_objectagg(key, value)`, `json_arrayagg(value)`, `json(input)` and
> `json_serialize(input)` take their operands; `json_scalar(expr)`, which has
> no clause at all, returns the `SimpleExpr` directly. `IS JSON` is
> `Expr::is_json(test)` and `Expr::is_not_json(test)`, where a `JsonTest` is a
> `JsonKind` (`Value`, `Scalar`, `Array`, `Object`), optionally
> `.with_unique_keys()`. The builders' fields are crate-private, so a value of
> each form holds only what its builder could set.
>
> **Paths are values.** The path of the three query functions is any
> `Into<String>` and is bound, never written into the statement's text
> (`sql.render.sql-json`); PostgreSQL accepts a parameter there. Where the
> server's grammar demands a literal instead — `JSON_TABLE`'s paths
> (`sql.ast.json-table`) — the path is a literal escaped by the value
> pipeline, not interpolated.
>
> **A JSON input** — a context item, a `PASSING` value, a constructor's value,
> the operand of `JSON()` and `JSON_SERIALIZE()` — is a `JsonInput`: any
> expression, and whether it is marked `FORMAT JSON`, which
> `Expr::format_json()` sets. `FORMAT JSON` makes a `text` value embed as the
> JSON it spells rather than as a JSON string, and lets a `bytea` value be read
> as JSON at all (unmarked, the server cannot cast `bytea` to `jsonb`). PostgreSQL
> reads JSON only as UTF-8, so `ENCODING UTF8` is never written: it is
> refused on a `text` value (`42804`) and changes nothing on `bytea`. A
> non-string operand marked `FORMAT JSON` is refused (`42804`); the builder
> does not know an expression's type, so that check is the server's.
>
> **`PASSING`** names a path variable: `passing(value, name)` binds `value`
> as `$name`. The name is an identifier, quoted like every other
> (`security.ident-oracle`), so the path's `$name` matches it exactly as
> written — a quoted name does not fold. PostgreSQL accepts a name given
> twice and a variable the path never reads; a variable the path reads and no
> `PASSING` names fails when the path is evaluated.
>
> **Behaviours are typed per function**, because the set each takes differs
> and the server refuses the rest (`42601`, "invalid ON ERROR behavior"):
>
> - `JSON_EXISTS` takes only `ON ERROR`, a `JsonExistsBehavior` — `True`,
>   `False`, `Unknown`, `Error`. Finding nothing is `false`, not an error, so it
>   has no `ON EMPTY` (`42601`), and it has no `RETURNING` (`42601`).
> - `JSON_VALUE` takes `on_empty` and `on_error`, each a `JsonValueBehavior` —
>   `Null`, `Error`, `Default(Value)`.
> - `JSON_QUERY` takes `on_empty` and `on_error`, each a `JsonQueryBehavior` —
>   those three, `EmptyArray` and `EmptyObject`. `EMPTY` alone means
>   `EMPTY ARRAY` and is not a second spelling here.
>
> Each behaviour is a slot a later call replaces, and `ON EMPTY` always renders
> before `ON ERROR`, so neither the reversed order nor a repeated clause
> (`42601`) can be built. Without a behaviour the server answers `false`
> (`JSON_EXISTS`) or `NULL`. A `Default` holds a `Value`, not an expression:
> PostgreSQL admits only a constant, a function call or an operator expression
> there and refuses a parameter (`42804`, "can only specify a constant,
> non-aggregate function, or operator expression for DEFAULT"), so the value
> is written as an escaped literal (`sql.render.sql-json`), and the server
> converts it to the `RETURNING` type — failing at parse time when it cannot
> (`22P02` for `'abc'` as `integer`; `42846` for an `integer` default to
> `jsonb`).
>
> **`JSON_QUERY`'s result shaping** is one slot: `with_wrapper()` (`WITH
> UNCONDITIONAL WRAPPER`, always an array), `with_conditional_wrapper()`
> (`WITH CONDITIONAL WRAPPER`, an array only when the path finds several
> items — one item, scalar or not, comes back as it is) and `omit_quotes()`
> (`OMIT QUOTES`, a scalar string without its quotes). The last call wins,
> because PostgreSQL refuses `OMIT QUOTES` beside either wrapper (`42601`,
> "SQL/JSON QUOTES behavior must not be specified when WITH WRAPPER is used").
> The defaults `WITHOUT WRAPPER` and `KEEP QUOTES` are never spelled, since an
> absent clause says them; without a wrapper, a path finding several items is
> an error. An unquoted string must still be a value of the returned type —
> `x` is `text` but no `jsonb`, so it fails and `ON ERROR` decides.
> `JSON_VALUE` has neither clause (`42601`).
>
> **`RETURNING`** takes a `ColumnType`, the type vocabulary a column is
> declared in, on every form that has one but `JSON_VALUE`: `JSON_QUERY`
> (default `jsonb`), the four constructors and two aggregates (default
> `json`) and `JSON_SERIALIZE` (default `text`; only a string type or `bytea`,
> `42804` otherwise). `JSON()` and `JSON_SCALAR()` take none (`42601`) and
> return `json`. `JSON_VALUE` (default `text`) takes a `JsonValueType`, which
> is `ColumnType`'s variants without `Json` and `JsonBinary`, each converting
> into its namesake; `TryFrom<ColumnType>` converts a type chosen at run time
> and hands those two back. They are left out because PostgreSQL 18.6 returns
> them wrongly (bug #19695): once one evaluation of `JSON_VALUE .. RETURNING
> jsonb` or `RETURNING json` is `NULL` — a JSON `null`, a path that finds
> nothing or fails, a `NULL` context item — every later evaluation of it in the
> statement is `NULL`, because the executor converts those two types without
> clearing the null flag the previous answer set. Checked live over a
> multi-row query: `RETURNING jsonb` and `json` stick, while `integer`,
> `text`, an array, a composite and a domain over `jsonb` (converted by a
> step that clears the flag) do not, and nor does a `JSON_TABLE` column of
> type `jsonb`, which is read as `JSON_QUERY` reads it. No rewrite answers the
> same: `JSON_QUERY` returns a JSON `null` as itself and an object or array
> rather than failing, so a caller wanting JSON out of a path reaches for it
> knowing those two differences, and a `Named` type is the caller's own
> spelling, not inspected. A test pins the defect, so the release that fixes
> it is noticed and the two variants return. The `FORMAT JSON [ENCODING UTF8]` a `RETURNING`
> may carry is not built: the output is UTF-8 JSON text either way, and a
> `bytea` result is the same bytes with or without it (live-checked);
> `JSON_VALUE` refuses it outright.
>
> **`ON NULL` and `UNIQUE KEYS`** each have the one spelling that differs from
> PostgreSQL's default, so each state has exactly one: an object keeps a
> `NULL` member unless `absent_on_null()`, an array drops a `NULL` element
> unless `null_on_null()`, and keys may repeat unless `with_unique_keys()`
> (then a repeat is `22030`). `JSON_OBJECT` and `JSON_OBJECTAGG` take the
> first and third, `JSON_ARRAY` and `JSON_ARRAYAGG` the second, `JSON()` and
> `IS JSON` only the third. The query form of `JSON_ARRAY` takes neither
> (`42601`) and always drops `NULL`s, so it is its own builder,
> `JsonArrayQuery`, with only `returning`; its query must have one column
> (`42601`), which the builder cannot see through `*`. Over a query of no
> rows it is the empty array from PostgreSQL 19, and `NULL` on 18, which
> answered it as the aggregate below does. An object's key may
> not be `NULL` (`22004`).
>
> **The aggregates** take `filter(cond)`, the `FILTER (WHERE ..)` of
> `sql.ast.func`, replacing on a second call. `JSON_ARRAYAGG` takes
> `order_by(expr, order)`, accumulating; `JSON_OBJECTAGG` takes no `ORDER BY`
> and neither takes `DISTINCT` (each `42601`), so neither is offered. Over no
> rows each returns `NULL`. Both are window functions too: `expr_window` and
> its siblings take any `WindowFunction` (`sql.ast.window-statement`).
>
> **`JSON_SERIALIZE` reads its operand through `JSON(..)`**
> (`sql.render.sql-json`). PostgreSQL 18.6's `JSON_SERIALIZE` over a `jsonb`
> operand returns bytes of jsonb's binary header rather than the document —
> `\x01` for a one-member object, `\x02` for two — whatever the `RETURNING`
> type, and `FORMAT JSON` on a `jsonb` operand is accepted and changes
> nothing. The parser builds the output conversion for a `json` input, so the
> `jsonb` bytes are read as text (reported to pgsql-bugs on 2026-03-03; no fix
> in 18.6). The builder cannot see an operand's type, and a cast to `json`
> would fail on `bytea` (`42846`), which `JSON_SERIALIZE` takes, marked or not.
> `JSON(..)` is the conversion that fits every operand: live on 18.6 it turns
> `jsonb` into the `json` of its canonical text, and makes of `json`, a
> string, an untyped literal and `bytea`, each with or without `FORMAT JSON`,
> exactly the `json` value `JSON_SERIALIZE` itself makes of them — the same
> text, the same `22P02` for a string that is not JSON, no unique-keys check.
> The one difference is an operand of another type, refused `42846` ("cannot
> cast type integer to json") rather than `42804`. A test pins the bare
> defect, so the release that fixes it is noticed and the `JSON(..)` can go.
>
> `JSON_VALUE` renders a JSON boolean as `t`/`f`, where `->>` renders
> `true`/`false`; the builder records that and does not paper over it.
> PostgreSQL bug #19046 — `JSON_ARRAY` or `JSON_OBJECT` over a column of a
> subquery on the nullable side of an outer join answering a value instead of
> `NULL` (17.6, 18rc1) — is a shape the builder can write, and needs no guard:
> it was fixed upstream in September 2025, and does not reproduce on 18.6.
>
> **The JSON operators stay, and replace nothing.** The operator decision of
> 2026-10-06 was to replace an operator of `sql.ast.expr.json` only where the
> SQL/JSON form has the same semantics and the same index use. Measured on
> PostgreSQL 18.6 over 200,000 rows with a GIN `jsonb_ops` index, a GIN
> `jsonb_path_ops` index and expression indexes on `(data->>'k')` and
> `((data->>'k')::int)` — and held by the live suite over 2,000 rows with
> sequential scans disabled, so a plan uses an index whenever one can serve
> it — none qualifies:
>
> - `?`, `?|` and `?&` are served by the `jsonb_ops` index (a bitmap index
>   scan; dropping it leaves a sequential scan, `jsonb_path_ops` serving none
>   of them). `JSON_EXISTS` is never index-served — a sequential scan even with
>   `enable_seqscan` off, under both opclasses — because it is not an
>   operator an opclass can index. `@?` and `@@` with a jsonpath are, by both.
>   The semantics differ as well: `?` matches a top-level string array
>   element and a top-level string (`'["a"]' ? 'a'` and `'"a"' ? 'a'` are
>   true, `JSON_EXISTS(.., '$.a')` false for both), while lax `JSON_EXISTS`
>   unwraps an array that `?` does not (`'[{"a":1}]'` is false under `?`, true
>   under `$.a`).
> - `->>` and `#>>` and their casts are served by expression indexes on
>   those exact expressions, which `JSON_VALUE` does not match, with or
>   without `RETURNING` (a sequential scan, forced or not). `JSON_VALUE` can be
>   indexed itself — an expression index on `JSON_VALUE(data, '$.k' RETURNING
>   int)` serves a query with that path as a constant or as a parameter under
>   a custom plan, though not under a generic one, exactly as an index on
>   `data->>'k'` serves `data->>$1`. The semantics differ: `(data->>'k')::int`
>   raises on a value that does not convert (`22P02`) where `JSON_VALUE`
>   follows `ON ERROR` (default `NULL`); a path reaching an object or array
>   gives `->>` its text and `JSON_VALUE` an error (`2203F`, `NULL` by
>   default); a JSON boolean is `true` under `->>` and `t` under
>   `JSON_VALUE`. A missing key is `NULL` under both.
> - `->` and `#>` agree with `JSON_QUERY` on what they find, but are matched
>   by their own expression indexes alone, and `JSON_QUERY` without a wrapper
>   errors where a lax path finds several items.
>
> So: reach for the operators where an index should serve the predicate or
> the projection — key existence under `jsonb_ops`, a field under an
> expression index — and for SQL/JSON where its semantics are the point: a
> conversion that must not raise (`RETURNING` with `ON ERROR`), a filter or
> wildcard path (`$.tags[*] ? (@ == $tag)`), a `json` operand (the `?` family
> is `jsonb`-only, `42883`), building JSON, or an index created on the
> SQL/JSON expression itself. Containment, which both families can say, is
> `@>` (`sql.ast.expr.operators`), served by either opclass.

> [spec:pgorm:def:sql.ast.json-table]
> `JSON_TABLE` is a FROM item, `FromItem::JsonTable(Box<JsonTable>, Name)`:
> rows read out of a JSON document, one per item its path finds, one column
> per column definition. `Func::json_table(context, path, column)` builds a
> `JsonTable` and takes its first column, because PostgreSQL refuses an empty
> list (`COLUMNS ()` is `42601`); `column(c)` appends more. `passing(value,
> name)` gives the paths a variable, bound, as it does for the query functions
> (`sql.ast.expr.sql-json`); `path_name(name)` names the root path (`AS name`);
> `on_error(b)` takes a `JsonTableBehavior` — `Error` or `Empty` — the only two
> PostgreSQL admits there (`NULL ON ERROR` is `42601`, "Only EMPTY [ ARRAY ] or
> ERROR is allowed in the top-level ON ERROR clause"); without one, a failing
> root path yields no rows, `EMPTY` spelled or not. `EMPTY ARRAY` is the same
> behaviour and is not a second spelling. `alias(name)` makes the `FromItem`.
> PostgreSQL would name an unaliased `JSON_TABLE` `json_table`, but the alias is
> required here as on every FROM item that is not a table
> (`sql.types.table-ref`), so its columns are qualified by a name the caller
> chose. Like a function in FROM, it is implicitly `LATERAL`: its context item
> may read the columns of the FROM items before it, whether it follows a comma
> or a join (`LEFT JOIN JSON_TABLE(..) ON TRUE` keeps a row whose document
> yields nothing).
>
> **Paths are literals here.** The root path is refused as a parameter
> (`0A000`, "only string constants are supported in JSON_TABLE path
> specification") and a column's or a nested path is a string constant in the
> grammar, a parameter being a syntax error (`42601`). Each is therefore a
> `String` the renderer writes through the value pipeline's literal escaping
> (`sql.render.json-table`), never interpolated; the `PASSING` values are
> still bound, so caller data that needs to reach a path goes through a
> variable.
>
> **A column** is a `JsonTableColumn`, made by one of five constructors:
>
> - `ordinality(name)` — `name FOR ORDINALITY`, the row's number from 1.
> - `value(name, type)` — `name type`, a `JsonValueColumn`: the scalar its
>   path finds, read as `JSON_VALUE` reads it, with `on_empty` and `on_error`
>   taking a `JsonValueBehavior`. The server refuses `EMPTY ARRAY` or `EMPTY
>   OBJECT` on a scalar column (`42601`, "Only ERROR, NULL, or DEFAULT
>   expression is allowed in ON ERROR for scalar columns"), which the type
>   leaves out. A `json`, `jsonb`, array or composite type is read as
>   `JSON_QUERY` reads it, under the same three behaviours.
> - `query(name, type)` — `name type FORMAT JSON`, a `JsonQueryColumn`: the
>   JSON its path finds, read as `JSON_QUERY` reads it, with the shaping slot
>   (`with_wrapper`, `with_conditional_wrapper`, `omit_quotes`) and
>   `JsonQueryBehavior`s. `FORMAT JSON` is always written, which is what makes
>   the column `JSON_QUERY`'s whatever its type; the type must be a string
>   type, `json`, `jsonb` or `bytea` (`0A000`, "cannot use JSON format with
>   non-string output types", otherwise).
> - `exists(name, type)` — `name type EXISTS`, a `JsonExistsColumn`: whether
>   its path finds anything, as a `boolean`, an integer or text; `on_error`
>   takes a `JsonExistsBehavior` and there is no `ON EMPTY` (`42601`), as for
>   `JSON_EXISTS`.
> - `nested(path, column)` — `NESTED PATH path COLUMNS (..)`, a
>   `JsonNestedColumns` taking its first column for the same reason the table
>   does (`42601` for an empty one), with `column(c)` and `path_name(name)`.
>   A nested path's rows join their parent's as an outer join would: a parent
>   whose nested path finds nothing keeps one row, its nested columns `NULL`.
>
> Each of the first four takes `path(p)`; without one a column reads
> `$."name"` — its own name, matched exactly as written, since the name is
> quoted. Column names and path names share one namespace across the whole
> table, nested levels included, and a name given twice is refused (`42712`,
> "duplicate JSON_TABLE column or path name"); the builder does not track
> names, so that check is the server's. Column names, path names, the
> `PASSING` names and the alias are caller-supplied identifiers, quoted like
> every other and registered with the identifier oracle
> (`security.ident-oracle`). PostgreSQL 18's grammar has no `PLAN` clause
> (`42601`), so there is none to build.
