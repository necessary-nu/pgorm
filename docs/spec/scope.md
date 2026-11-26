# Scopes

This chapter covers tables whose statements are held to a condition the
application supplies, such as `company_id = $me.company`. A scope is the
application's own type, and every statement on a scoped entity is built
under a context holding its scopes, so a statement that forgets one is a
compile error naming the entity and its scopes. Holding the rows to the
condition in the database, by row-level security or otherwise, is the
application's; pgorm's part is that its builders cannot write the statement
without it. Raw SQL and pgorm-query used directly are outside scopes.

## Declaration

> [spec:pgorm:def:scope.declare+1]
> `#[derive(Scope)]` MUST implement `Scope` for its type, which makes the
> type a scope. In the composite `Entity` derive a field keyed
> `#[pgorm(scope)]` MUST declare a column scope: the field's type is the
> scope, and a statement under a context holding a value of it is held to
> the column equal to that value, compared under the column's casts as
> `Column::eq` compares it. A column's type alone MUST NOT make it a
> scope column. The container key `scope(T, ..)` MUST declare each type it
> names a scope written by hand, whose `ScopeOn<Entity>` the program
> implements (`[spec:pgorm:def:scope.written]`). A `scope` key on a field
> that is `Option` or `nullable`, `ignore`d or generated, `scope` given twice
> on one field, and an empty `scope()` MUST NOT compile, each refused at the
> key. A primary key column MAY be a scope column.
>
> The derive MUST implement `EntityScopes` for every entity, an unscoped one
> with its defaults: `scopes()`, the scopes as data (`Scopes`: their names,
> each the last segment of its type as written, and how a context's values
> become the condition on a row read under a given name and a pipeline's
> filter), and `fill_scopes`, what an insert under a context does to a row
> (`[spec:pgorm:sem:scope.insert]`). `EntityScopes` is a supertrait of
> `Entity`, so a hand-written entity implements it too, as it does
> `ScopedIn`.

## Contexts

> [spec:pgorm:def:scope.context]
> A context is a `ScopeContext`, which answers how it holds a scope: one
> value, every value, or not at all. A scope value MUST be a context holding
> itself, and `Provides<S>` MUST hold for every context holding `S`, which is
> what the compiler checks a statement's context against.
> `#[derive(ScopeContext)]` on a struct MUST make it a context holding each
> field's scope, a field being a scope or an `Every` of one, read from how
> its type is written, and implement `Provides` for each. A struct with
> generic parameters, a field holding a scope another field holds, and
> anything but a struct MUST NOT compile, each refused by the derive.
> `NoContext` holds nothing; it is the context of every form taking none.

> [spec:pgorm:def:scope.every]
> `Every<S>` MUST be a context holding every value of `S`: it satisfies `S`
> wherever a statement needs it, puts no condition on the statement for it,
> and fills no column of a row an insert writes, which leaves the row to set
> its `S` columns itself.

## Building a statement

> [spec:pgorm:req:scope.compile]
> Every statement constructor that starts from an entity MUST have a form
> ending in `_for` taking the context first: `find_for`, `find_by_id_for`,
> `graph_for` and `delete_by_id_for`; `Insert::one_for`, `many_for`,
> `add_for` and `add_many_for`, and `TryInsert`'s; `Update::one_for` and
> `many_for`; `Delete::one_for` and `many_for`; an `ActiveModel`'s
> `insert_for`, `update_for` and `delete_for`; a model's `find_related_for`,
> `find_linked_for` and `delete_for`; `Related::find_related_for` and
> `Linked::find_linked_for`; a disjoint entity's `find_for`,
> `find_by_id_for`, `insert_for`, `update_for` and `change_kind_for`; a
> one-of reference's `load_for`; and `Pipeline::from_for`. Each MUST be
> bounded by `ScopedIn<Cx>` for its context `Cx`, which holds for an entity
> under every context holding each of its scopes and for an unscoped entity
> under every context.
>
> The forms taking no context, the `_in` forms among them, MUST be bounded by
> `ScopedIn<NoContext>`, so for a scoped entity they do not compile, and
> neither do a graph slot of an entity the graph's context does not cover, a
> loader given the entity alone, and an entity given to a pipeline as a
> source. A form taking both an entity value and a context does not exist; a
> scoped entity's `_for` forms read its `Default` names. The compile error
> MUST be the entity's own, written by the derive through a trait of the
> entity's that every context providing its scopes implements:
> "`` `note::Entity` is scoped by `CompanyId` and `UserId`, which `{context}`
> does not hold ``", its label "`` `note::Entity` needs a context holding
> `CompanyId` and `UserId` ``", and a note pointing to the `_for` forms, the
> entity named by its path and the context by its type, `NoContext` for a
> form taking none. A language binding's registered entity MUST be bounded by
> `ScopedIn<NoContext>`, so a scoped Rust entity does not register.

## Reading

> [spec:pgorm:sem:scope.read]
> A read under a context MUST AND each scope the context holds the value of
> into its `WHERE`, qualified by the name the entity's columns are: a
> `find_for` and a `find_by_id_for`, the key and the scopes side by side,
> whose row outside the scopes is `Error::RecordNotFound` from `one`; a
> graph's root; a disjoint entity's base, whose kinds' rows are keyed by it;
> each target a one-of `load_for` reads, so a reference outside the scopes is
> the load's absent-key error; and a pipeline from `from_for`, as a `filter`
> stage binding each value. A scope under `Every` MUST add nothing.

> [spec:pgorm:sem:scope.join]
> A join into a scoped table MUST be held to its scopes or refused. A graph
> keeps its context: each slot MUST be bounded by `ScopedIn` of it, and each
> slot and hop MUST AND its joined table's scopes into that join's `ON`,
> qualified by the name the table is joined under, a slot's being its
> entity's and a hop's the relation's. The `Relation` derive MUST record a
> relation's sides' scopes in its `RelationDef`, which `rev` swaps. A select's
> join through a `RelationDef` MUST, joined with its context (`join_for`,
> `join_rev_for`, `join_as_for`, `join_as_rev_for`), AND the joined side's
> scopes into its `ON`, and joined by a form taking none, be refused with
> `ScopeError::Unscoped` when the statement would run; the typed joins and
> `find_related` / `find_linked` take the same path, their junctions and hops
> included. A loader's join back to the models it is given, and through their
> junction, is keyed by those models and MUST NOT be scoped again.

## Writing

> [spec:pgorm:sem:scope.insert]
> An insert under a context MUST fill each column scope's column the row
> leaves `NotSet` with the value the context holds, keep a row holding that
> value already, and refuse a row holding another with
> `ScopeError::Conflict`; under `Every` of the scope it MUST fill nothing and
> refuse a row leaving the column `NotSet` with `ScopeError::Unset`. A scope
> written by hand MUST be filled by its `ScopeOn::insert`, and not at all
> under `Every`. An `Insert` holds a model's refusal and leaves its row out,
> and a disjoint insert fills the base's row and a scoped kind's row alike.

> [spec:pgorm:sem:scope.write]
> An update or a delete by key under a context MUST carry the context's
> scopes beside the key, so a row outside them is no row:
> `UpdateOne::exec_returning_model` reports `Error::RecordNotFound` and a
> delete counts `0`. A bulk update or delete under a context MUST AND the
> scopes into its `WHERE`, and so MUST a disjoint update and change of kind
> on the base row they find.

> [spec:pgorm:req:scope.immutable]
> A column whose definition is `immutable` (`ColumnDef::immutable`) is
> written when its row is inserted and never changed after; every column
> scope's column MUST be one, since a row cannot leave its scope. An update
> setting one MUST be refused with `Error::ImmutableColumn` naming the table
> and column, before anything is sent: `Update::one` and its `_for` and `_in`
> forms at construction, an `UpdateMany` whose `set` or `col_expr` names one
> when it would run, an upsert whose `ON CONFLICT .. DO UPDATE SET` assigns
> one when it would run, and a disjoint update setting one in the base's or
> the kind's row. pgorm writes no `DO UPDATE SET` of its own, so an upsert
> leaves such a column out unless it names it.

## Refusals

> [spec:pgorm:req:scope.refusals]
> A statement refused for its scopes MUST fail with `Error::Scope` carrying a
> `ScopeError`, and nothing MUST be sent: `NotHeld`, a context not holding a
> scope a table is scoped by, which only a context implementing
> `ScopeContext` by hand can reach once the types hold; `Conflict` and
> `Unset` of an insert; `Unscoped`, a join into a scoped table with no
> context; `Unpiped`, a pipeline over a scope written by hand; and
> `Refused`, a hand-written scope's own words. A builder that cannot return
> the refusal where it arises MUST hold it and report it from every path that
> would run the statement: a select, its projections and selectors, its
> paginator and cursor, a graph, a disjoint read, an insert, a bulk update
> and delete, and a pipeline, whose `into_sql` reports `PipelineError::Scope`
> and which an embedding pipeline carries. Rendering a statement, by
> `build`, `as_query`, `into_query` or `to_string`, reports nothing: a
> statement taken out of pgorm is pgorm-query's.

## Scopes written by hand

> [spec:pgorm:def:scope.written]
> `ScopeOn<E>`, implemented for a scope type the entity's `scope(..)` names,
> MUST give the condition a value puts on a row of `E` read under a given
> name, the table's own or a join's alias, which reads and joins put where a
> column scope's goes, and `insert`, which fills or refuses a row an insert
> under the value writes. Under `Every` of the scope neither is called. The
> condition is pgorm-query's, which a pipeline cannot read, so `from_for`
> under a context holding a value of such a scope MUST be refused with
> `ScopeError::Unpiped`. The columns the condition reads are its own; pgorm
> marks immutable only the columns `#[pgorm(scope)]` marks.

## Raw SQL

> [spec:pgorm:req:scope.raw+2]
> Raw SQL, through `sql!`, `prql!` and the methods taking SQL text, and
> pgorm-query's builders used directly, MUST stay outside scopes. `sql!`
> MUST expand to an expression spanned at the invocation, its literal
> included, as `prql!`'s expansion is (`read-only.sql-macro`), so that clippy's `disallowed_macros` configured with
> `pgorm::sql` and `pgorm::prql` finds both, under those paths and under a
> name a `use` gives them. The Rust guide's entities chapter MUST teach
> scopes and give the clippy configuration a project bans raw SQL with: the
> two macros under `disallowed-macros`, and under `disallowed-methods` the
> raw-text methods `Connection::execute`, `execute_raw`, `query_one`,
> `query_opt`, `query_all`, `query_raw` and `batch_execute`,
> `SelectorRaw::from_statement`, `into_tuple` and `with_columns`,
> `Select::from_raw_sql` and `FromQueryResult::find_by_statement`, each
> written by its path from pgorm's root, and a case meant to run allowed
> where it is written with `#[expect(clippy::disallowed_macros)]` or
> `#[allow(..)]`.
