# Disjoint tables

This chapter covers a Rust enum stored across tables: a base table whose
every row has a kind, and one table for each kind that carries data of its
own, keyed by the base row's key. The enum is the whole description, and
PostgreSQL enforces it with constraints alone, so a state the enum cannot
describe cannot be written, by pgorm or by anything else connected to the
database. There are no triggers (`[spec:pgorm:req:sql.scope]` keeps them out
of the builder) and no single-table mapping, in which every kind's columns
would sit side by side in the base, mostly null.

A schema pgorm did not write may hold the same tables without the kind
column or the constraints: a base, and tables each keyed by a foreign key
from its primary key onto the base's. Nothing there holds a base row to one
kind's table, so such a schema is read through an unkinded declaration
([Unkinded tables](#unkinded-tables)), which decodes each row by which table
has a row for it, and is never written through.

## Declaration

> [spec:pgorm:def:disjoint.declare+5]
> `#[derive(Disjoint)]` on an enum MUST declare a disjoint entity over the
> base entity its container attribute names, `#[pgorm(base =
> vehicle::Entity)]`. Each variant is a kind of base row. A variant with one
> unnamed field holds the `Model` of its kind's entity, `Car(car::Model)`,
> and declares that every base row of that kind has exactly one row in that
> entity's table. A unit variant, `Bike`, declares a kind with no table of
> its own, whose base rows have no row anywhere else. A variant of any other
> shape or with a discriminant, a payload that is not an entity's `Model`
> (a further model of an entity included), two variants holding the same
> entity, two variants of one value, and an enum with no variants or with
> generic parameters MUST NOT compile. The derive is given its enum as
> configured, so a variant that `#[cfg]` removes is no kind, of the kind type
> or of anything else the derive generates, and a key that `cfg_attr` gives is
> read as written.
>
> The derive MUST generate the kind type, `<Enum>Kind`, an `ActiveEnum`
> stored as a PostgreSQL enum with one value per variant, in declaration
> order, each spelled as its variant's name in snake_case unless the
> variant's `#[pgorm(value = "...")]` gives another, and a value over the 63
> bytes PostgreSQL holds in an enum's value MUST NOT compile, refused at its
> `value` or else its variant as an `ActiveEnum`'s is. The type is named
> `<enum>_kind`, the enum's name in snake_case, unless `enum_name = "..."`
> gives another, and `schema_name = "..."` qualifies it as it does an
> `ActiveEnum`'s (`[spec:pgorm:syn:macros.derive.active-enum]`).
> `kind_attrs(..)` gives the kind type further attributes after its own
> derives, each written as it would be on the type, so a
> `kind_attrs(derive(Serialize, Deserialize), serde(..))` makes it
> serializable as a field of a serializable base `Model` needs. The base's
> kind column is the one `kind = vehicle::Column::Kind` names, the base's
> `Column::Kind` by default, and a base with no such column MUST NOT
> compile; a base row of a kind the enum does not list cannot exist, because
> the kind type has no such value. The derive MUST also generate
> `<Enum>ActiveModel`, the same variants each holding its kind entity's
> `ActiveModel`, which is what a write takes
> (`[spec:pgorm:sem:disjoint.write]`). Each enum the derive generates, the
> kind type, `<Enum>ActiveModel` and an unkinded entity's kind enum
> ([Unkinded tables](#unkinded-tables)), carries
> `#[allow(clippy::enum_variant_names)]`: its variants are the enum's, which
> the lint judges where the enum is declared, and an `allow` written there
> does not reach them.
>
> Each kind's entity MUST have a primary key of the same types, in the same
> order, as the base's, since its row is keyed by the base row's key, and
> one whose key types differ MUST NOT compile. Any key qualifies, composite
> keys included, and a kind's key columns may be named differently from the
> base's. A kind entity is otherwise an ordinary entity: it can be read,
> joined and written on its own, subject to the constraints
> (`[spec:pgorm:sem:disjoint.schema]`), and a foreign key elsewhere can point
> at the base, meaning a row of any kind, or at one kind's table. What the
> types cannot tell — a kind column that is nullable, generated, or not of
> the kind type (its name, schema and values), when a write sets it to the
> kind it writes, or a kind entity whose key the database generates or fills
> (drawn from a sequence, an identity, a generated column or a default), when
> its values must come from the base row — MUST be refused as an
> `Error::Type` naming the table and column, by `check_disjoint::<E>()`,
> which `Schema::create_disjoint` and every read and write run before
> anything is sent.

## Schema

> [spec:pgorm:sem:disjoint.schema+2]
> `Schema::create_disjoint::<E>()` MUST run `check_disjoint::<E>()` first,
> returning its refusal as its own, then refuse the keys below, and otherwise
> return the statements that create a disjoint entity's type and tables, a
> `Vec<DdlStatement>` in the order they run (`DdlStatement` being a `CREATE
> TYPE`, a `CREATE TABLE` or an `ALTER TABLE`, each rendering as its
> statement):
>
> 1. the kind type;
> 2. the base table as `Schema::create_table_from_entity` writes it
>    (`[spec:pgorm:sem:schema.from-entity]`), with a `UNIQUE` constraint on
>    its key columns followed by its kind column;
> 3. each kind's table as `create_table_from_entity` writes it, with a kind
>    column named as the base's, of the kind type and `GENERATED ALWAYS AS
>    ('<value>') STORED`, and a foreign key from its key columns and that
>    column to the base's `UNIQUE` columns, `ON DELETE CASCADE`. A
>    belongs-to relation the kind's entity declares from exactly its key to
>    the base's key writes no foreign key of its own, since this one implies
>    it, and the key is written with no identity, serial or default, which
>    `check_disjoint` refuses (`[spec:pgorm:def:disjoint.declare]`);
> 4. when any kind has a table, one `ALTER TABLE` on the base adding, for
>    each such kind and each key column, a column `<value>_<column>` of the
>    key column's type, `GENERATED ALWAYS AS (CASE WHEN <kind column> =
>    '<value>' THEN <column> END) STORED`, and for each such kind a foreign
>    key from its columns to the kind table's key.
>
> A key these constraints cannot be written over MUST be refused as an
> `Error::Type` naming the table and column, before any statement is
> returned, rather than left to the server: a base key column that is a
> generated column, which the base's generated columns would read and
> PostgreSQL refuses a generated column reading (`42P17`); a base key column
> of a domain over an enum, which no foreign key can match (`42804`), every
> kind's key being of the base key's types; and a kind's key ending `WITHOUT
> OVERLAPS`, which the base's plain foreign key onto that table matches no
> key of (`42830`). A temporal base key is held, each kind table's foreign key
> reaching the base's `UNIQUE` key, which takes its period as a plain column.
>
> The base's foreign keys come last because each side references the other.
> The types the tables' other columns use are not among the statements;
> `create_enum_from_entity` creates them, the kind type included when given
> the base. Together the constraints MUST make PostgreSQL refuse every state
> the enum does not describe: a base row of a kind with a table but no row
> in it, a row in a kind's table whose base row is absent or of another
> kind, and so a base row with rows in two kinds' tables. A generated column
> is null for every kind but its own, and a null skips its foreign key, so
> each base row is held to exactly its own kind's table. A kind without a
> table constrains nothing beyond its value. Deleting a base row deletes its
> kind's row by the cascade; deleting a kind's row alone, or changing a base
> row's kind while its old kind's row remains, is refused.
>
> Every foreign key here MUST be `DEFERRABLE INITIALLY IMMEDIATE`. Each
> write pgorm sends is one statement, whose checks run at its end, after
> every part of its `WITH`, so a violation is reported by the statement that
> caused it, inside a transaction as outside one; a change written by hand
> across several statements runs under `SET CONSTRAINTS ... DEFERRED`. The
> generated columns MUST be `STORED`: PostgreSQL 18 makes a generated column
> `VIRTUAL` unless told otherwise, and a virtual column can use no
> user-defined type, the kind type included.
>
> The base's generated columns and each kind table's kind column are
> bookkeeping: no model has a field for them, no read selects them and no
> write names them. Every constraint is named. The constraint names and the
> generated columns' names MUST be a function of the tables, columns and
> values alone, so that a later statement can name them, and MUST fit
> PostgreSQL's 63-byte identifier limit. They are composed as derived
> foreign keys' are (`[spec:pgorm:req:entity.relation.fk]`), each table bare
> and each list `-`-joined: the base's `UNIQUE` is `uq-{base}-{key
> columns}-{kind column}`, a kind table's foreign key `fk-{table}-{key
> columns}-{kind column}`, and the base's to it `fk-{base}-{its generated
> columns}`; and each, the generated columns' `<value>_<column>` too, is
> bounded as prefixed result names are
> (`[spec:pgorm:sem:query.graph.writer]`), kept when shorter than 63 bytes
> and otherwise spelled as a head and a hash of the whole in exactly 63. A
> generated name equal to a column an entity declares, or to another name
> generated on the same table, MUST be refused with `Error::Type` naming
> both, before any statement is returned.

## Reads

> [spec:pgorm:sem:disjoint.read+2]
> `E::find()` MUST read base rows with their kinds' rows in one statement:
> the base `LEFT JOIN`ed to each kind's table on the key alone, each row read
> as the pair `(base::Model, E)`. A row decodes by its base row's kind: a
> kind with a table decodes its joined row into its variant, and a kind
> without one is its unit variant. A row whose kind has a table but whose
> joined row is absent, and a row with a joined row present for any other
> kind, can come only from a schema `[spec:pgorm:sem:disjoint.schema]` did
> not create, and MUST each be an `Error::Query` naming the base table, the
> key and the kinds involved, never a variant read without its row and never
> a row dropped: `all` fails at it, and `stream` yields it as one `Err` item
> among the rows. It is not `Error::Type`, which is `check_disjoint`'s
> refusal of a declaration before anything is sent; here the declaration
> passed and the rows the server returned contradict it, as a key that a
> `HasOne` relation matches twice contradicts `load_one`'s.
>
> The read MUST take what a `Select` of the base takes — filters, ordering,
> limit and offset, pagination and keyset cursors over the base's columns —
> and `find_by_id(key)`, composite keys included. `kind(k)` and
> `kinds([..])` restrict it to base rows of those kinds, and
> `filter_kind::<M>(condition)`, `M` a kind's `Model`, to base rows of that
> kind whose row passes the condition: the kind is part of the condition, so
> one that a missing row satisfies (`IS NULL`) reads no row of another kind.
> A condition on a kind table's column given to `filter` is an ordinary
> `WHERE`, under which every other kind's row holds that column as `NULL`.
> Its terminals are `one`, `all` and `stream`, `one` being
> `Error::RecordNotFound` when no row matches. Each runs `check_disjoint`
> before anything is sent, and so do `paginate`, `count` and `cursor_by`,
> which therefore return a `Result`. The number of kinds is not bounded.
> Each joined table's result columns are named by the same prefixed,
> 63-byte-bounded composition a graph's sources are, the base's under `s0_`
> and each kind with a table's under the next index in declaration order, so
> a kind's column never shadows a base column or another kind's of the same
> name. No bookkeeping column is selected.
>
> Each table of the read MUST be read under an identifier of its own, since
> PostgreSQL refuses two of one name in a statement (`42712`) even in two
> schemas. The base is read under its bare name, and each kind's table, in
> declaration order, under its bare name unless the base or an earlier kind's
> table has it — `market.listing` and `archive.listing` are both `listing` —
> when it is joined under an alias: its schema-qualified name as one
> identifier, `"archive.listing"`, or, where the read already has that, as it
> has an unqualified table's bare name, the same with `_2`, `_3` and on
> appended, each bounded to 63 bytes as the result names are. The join, the
> projection and the read's own conditions qualify the table's columns by
> that identifier, which `SelectDisjoint::kind_qualifier::<M>()` and
> `DisjointTable::qualifier` give. A typed column qualifies by its table's
> bare name, so a condition on the columns of a kind's table read under an
> alias names them through the alias, as one on an aliased graph slot's does
> (`[spec:pgorm:req:query.graph.aliases]`).

## Writes

> [spec:pgorm:sem:disjoint.write+2]
> Every write MUST be one statement, its parts joined by data-modifying
> `WITH` clauses, so that it applies whole or not at all with no
> transaction, and its constraints are checked once, at its end
> (`[spec:pgorm:sem:disjoint.schema]`). Each is a `WriteDisjoint<E>`, whose
> construction runs `check_disjoint::<E>()` and the refusals below, each an
> `Error::Type` naming the table and column, so a refused write sends
> nothing. Its terminal, `exec_returning_model`, returns the pair a read of
> the written row gives, `(base::Model, E)`, decoded as a read's row is
> (`[spec:pgorm:sem:disjoint.read]`). Every value is written under its
> column's `save_as` cast, as an ordinary insert or update writes it.
>
> - `E::insert(base, kind)`, given the base's `ActiveModel` and an
>   `<Enum>ActiveModel`, MUST insert the base row with the kind's value in
>   its kind column and, for a kind with a table, the kind's row with its key
>   taken from the inserted base row, so a key the database generates reaches
>   it. The base's kind column and the kind row's key columns MUST be
>   `NotSet`, since the kind and the key each come from one place; a value
>   held in either, `Set` or `Unchanged`, MUST be refused.
> - `E::update(base, kind)` MUST update the set columns of the base row and
>   of its kind's row, both found by the base's key, which the base's
>   `ActiveModel` holds `Set` or `Unchanged` as for `Update::one`, a `NotSet`
>   part being `Error::PrimaryKeyNotSet`. A kind other than the row's current
>   one, and a key no row has, MUST be `Error::RecordNotFound` with both rows
>   left as they were, a kind changed by a write that committed while this
>   one waited for the row included: changing kind is its own operation, so
>   the base's kind column `Set`, and a kind row's key column `Set`, MUST be
>   refused. An update setting nothing in either is a read of the row under
>   the same conditions, as `UpdateOne`'s `exec_returning_model` reads its
>   row back.
> - `E::change_kind(key, kind)` MUST delete the row's row from every kind
>   table but the new kind's, its current kind's among them, insert the new
>   kind's row with its key taken from the base row, where that kind has a
>   table, and set the base row's kind. The new row's key columns MUST be
>   `NotSet`, as an insert's are. A key no row has, and a row that already
>   has that kind, MUST be `Error::RecordNotFound` with nothing changed.
>
> Each part is named after the identifier the read reads its table under
> (`[spec:pgorm:sem:disjoint.read]`), and a part writing a table read under an
> alias targets the table under it, so tables of one name write as parts of
> their own. An unqualified name in the statement means the part of that name
> where there is one, and a kind's table with no schema that is read under an
> alias has the bare name of an earlier table, whose part is so named, so a
> write would read that part for it: every write of such a shape MUST be
> refused, as `Error::Type` naming both tables, before anything is sent.
>
> Deleting needs nothing of its own: the base entity's delete removes a base
> row and, by the cascade, its kind's row.

## Unkinded tables

> [spec:pgorm:sem:disjoint.unkinded+1]
> `#[derive(Disjoint)]` with the container key `unkinded`, `#[pgorm(base =
> shop::Entity, unkinded)]`, MUST declare an unkinded disjoint entity: one
> whose base has no kind column, a base row's kind being the kind whose table
> has a row for it. A variant holding a kind's `Model` is a kind with a table,
> keyed by the base's key as `[spec:pgorm:def:disjoint.declare]` requires and
> refused as it refuses; a unit variant is the kind of a base row with a row
> in no kind's table, so there is at most one, and with none such a row is not
> one the entity describes. Having no kind column, the declaration has no kind
> type: beside `unkinded`, the container's `kind`, `enum_name` and
> `schema_name`, a variant's `value`, a second unit variant, and no variant
> holding a kind's `Model`, which leaves nothing to tell kinds apart by,
> MUST NOT compile. The derive MUST generate `<Enum>Kind` as an enum of the kinds in
> declaration order that no column holds, with the derives the kind type has
> and `kind_attrs(..)` after them, and not an `ActiveEnum`; and it generates
> no `<Enum>ActiveModel`. A kind is named, by an error and by its
> `DisjointShape`, as its variant's name in snake_case.
>
> Nothing in such a schema holds a base row to one kind's table, so pgorm
> MUST NOT write through it, and that is the compiler's refusal, not a
> run-time one: `insert`, `update` and `change_kind` are `Disjoint`'s only
> where it is also `KindedDisjoint`, as `Schema::create_disjoint` takes only
> a `KindedDisjoint`, and the derive implements that trait for a declaration
> with a kind column alone, so a call on an unkinded entity does not compile,
> the error naming it read-only. A binding registers one read-only
> (`[spec:pgorm:req:bindings.disjoint-unkinded]`), its registration taking
> any `Disjoint`. `check_disjoint` refuses nothing of an unkinded entity,
> what it refuses being of a kind column and of the keys a write fills; a
> `DisjointShape` of one refuses each write and `create` as `Error::Type`
> naming its base as read-only, before anything is sent.
> `DisjointShape::unkinded(base, kinds)` MUST describe one whose tables are
> known only at run time, as `DisjointShape::new` describes a kinded one,
> refusing as `Error::Type` what `new` refuses of the kinds — none, a value
> given twice, one table for two kinds or for a kind and the base, a kind
> table keyed by other types than the base — and what the derive refuses of
> an unkinded enum, a second kind without a table and no kind with one.
>
> `E::find()` and `find_by_id(key)` MUST read it as
> `[spec:pgorm:sem:disjoint.read]` reads a kinded entity — one statement,
> the base `LEFT JOIN`ed to each kind's table on the key, the same
> projection, the same terminals, filters, ordering, limit, offset,
> pagination, `count` and keyset cursors — but for the kind. A row decodes as
> the kind whose table's joined row is present, judged by the absence witness
> a kinded read uses, and as the unit kind when no kind's is. `kind(k)` and
> `kinds([..])` restrict by presence: to base rows whose row in `k`'s table
> is present, its first key column `IS NOT NULL` in the join, or for the unit
> kind to base rows whose row in every kind's table is absent; and
> `filter_kind::<M>(condition)` to base rows whose row in `M`'s table is
> present and passes the condition. A base row with rows present in two or
> more kinds' tables, and one with a row in none where the entity declares no
> unit kind, MUST each be an `Error::Query` naming the base table, the key
> and the kinds involved, as a kinded read's inconsistent rows are: `all`
> fails at it, `stream` yields it as one `Err` item among the rows, and it is
> never dropped nor read as one of its kinds, a restriction to one of them
> included, which reads it and so reports it.

## Changing the kinds

> [spec:pgorm:sem:disjoint.evolve+2]
> A change to a kinded disjoint entity's kinds after its tables exist MUST be
> written by `Schema` from the declaration as it will be, as a
> `DisjointChange`: an ordered list of steps, each a list of statements that
> run in one transaction, with a commit between one step and the next, and
> where a step leaves data to the program, a gap between them that says so.
> Each statement names the constraints and generated columns it alters by the
> names `[spec:pgorm:sem:disjoint.schema]` gives them. Run through
> pgorm-migration, each step is one migration and every step but the last
> answers `commits_after()` (`[spec:pgorm:req:migration.commit-after]`). Each
> function is the `DisjointShape` method of the same change over
> `DisjointShape::of::<E>()` — `add_kind`, `add_kind_recreating`,
> `remove_kind`, `add_kind_table` and `remove_kind_table`, each taking the
> kind's value — so a declaration described at run time has them too.
>
> - `Schema::add_disjoint_kind::<E>(kind)` MUST add a kind the declaration
>   holds and the database does not, in two steps: `ALTER TYPE ... ADD VALUE`
>   placed where the kind stands in declaration order, `AFTER` the kind
>   declared before it, or `BEFORE` the one after it when it is the first;
>   then, for a kind with a table, that table as `create_disjoint` writes it
>   and the base's generated key columns and foreign key for it. Two steps
>   because PostgreSQL refuses to use an added value before the transaction
>   that added it commits, and both the kind table's constant kind column and
>   the base's generated columns use it; a kind without a table uses it
>   nowhere, and its change is the first step alone. No existing table is
>   rewritten except to add the base's generated columns.
> - `Schema::add_disjoint_kind_recreating::<E>(kind)` MUST make the same
>   change in one step that rewrites the tables instead: a new enum type with
>   every value in declaration order, named as the kind type with `_next`
>   appended in its schema and bounded to 63 bytes as the schema's names are,
>   the base's and each kind table's kind column retyped to it, the disjoint
>   constraints and generated columns dropped first and written again after,
>   the old type dropped and the new one renamed to the old name. The base's
>   kind column is retyped `USING` its text, no cast joining two enum types
>   (`42804`), after its default is dropped, which PostgreSQL would otherwise
>   refuse to cast as well, and the declaration's is set again after the
>   rename. A kind table's kind column is a generated column, which
>   PostgreSQL retypes by no `USING` (`42611`), so it is among the generated
>   columns dropped and written again, of the new type. Each table MUST be
>   rewritten once: retyping the base's kind column and adding its stored
>   generated key columns each rewrite it, so one `ALTER TABLE`, which
>   PostgreSQL carries out in one rewrite, drops those columns, retypes the
>   kind column and writes them again over the new type, whose rename they
>   survive, and their foreign keys, which reference the kind tables, are
>   added last, once every kind table has its kind column again and an added
>   kind's table exists. It takes an exclusive lock on the base and on every
>   kind table for as long as the rewrite runs, which is the price of one
>   atomic migration.
> - `Schema::remove_disjoint_kind::<E>(value)` MUST remove a value the
>   database's kind type holds and the declaration does not, by the recreating
>   route, there being no `ALTER TYPE ... DROP VALUE`. Its first statement
>   MUST fail while any base row still has that kind — a `CHECK` on the base
>   refusing the value, named `ck-{base}-{kind column}-{value}` as the
>   schema's names are composed and bounded, added and so validated against
>   every row (`23514`), then dropped — so the change applies to an entity
>   with no such rows or not at all. A kind with a table loses its table only
>   through `remove_disjoint_kind_table`, which comes first.
> - `Schema::add_disjoint_kind_table::<E>(kind)` MUST give a kind the
>   database holds as a unit kind the table the declaration now gives it, in
>   two steps with a gap for the program between them: the table, and the
>   base's generated key columns with a foreign key added `NOT VALID`, since
>   every base row of that kind lacks its row until the program writes them;
>   then, once it has, `VALIDATE CONSTRAINT`, which refuses the change if any
>   row is still missing.
> - `Schema::remove_disjoint_kind_table::<E>(kind, table)` MUST drop the
>   kind's table, `table`, and the base's generated key columns and foreign
>   key for it, turning it into a unit kind, in one step. The table is an
>   argument because the declaration as it will be no longer names it. It
>   discards every row of that table, so it exists only as its own call,
>   never as a side effect of another change.
>
> Every function runs `check_disjoint` on the declaration first, refuses a
> change the declaration does not describe (a kind added that it does not
> hold, a value removed that it still holds, a table given to a kind it
> declares without one, and the converse) with `Error::Type` naming the kind,
> and refuses an unkinded declaration at compile time, as `create_disjoint`
> does.

## References to one of several entities

> [spec:pgorm:def:disjoint.one-of+1]
> `#[derive(OneOf)]` on an enum MUST declare a reference from one entity to
> exactly one of several others, stored as one nullable column per target,
> each a foreign key the entity's own `belongs_to` relation declares:
> `#[pgorm(entity = comment::Entity)]` on the enum, and on each variant the
> column holding that target's key, `#[pgorm(column = Column::PostId)]
> Post(post::Entity)`, the payload naming the target entity. At most one unit
> variant MAY say that a row references none of them. A column that is not
> the entity's, is not nullable, appears twice, or whose type is not the
> target's key type, a target the entity has no `Related` relation to, two
> variants naming one target, and an enum with no variant naming a target
> MUST NOT compile. A column's type is the type of the field the entity's
> `Model` declares for it, reachable from the column through the
> `ColumnField` the `Model` derive implements
> (`[spec:pgorm:sem:macros.derive.model]`).
>
> The derive MUST generate `<Enum>Key`, the same variants each holding its
> target's key value (the unit variant, if any, holding nothing), and
> `<Enum>Model`, the same variants each holding its target's `Model`. They
> derive `Clone`, `Debug` and `PartialEq`, and `Clone` and `Debug`, and
> `key_attrs(..)` and `model_attrs(..)` give each further attributes after
> its own derives, as `kind_attrs(..)` gives a disjoint entity's kind type
> (`[spec:pgorm:def:disjoint.declare]`). A model's reference reads as its
> `<Enum>Key`, `OneOf::key(&model)` — the variant whose column is set — and
> a row with two columns set, or none where the enum has no unit variant,
> MUST be an `Error::Query` naming the table, the key and the columns, never
> read as one of them; setting an `<Enum>Key` on the entity's `ActiveModel`,
> `OneOf::set(&mut active, key)`, sets that variant's column and sets every
> other one to null. Loading the targets of a list of rows,
> `OneOf::load(&models, db)`, MUST read each target table once, for the keys
> its variant holds, as `load_one` reads a relation, and give each row its
> `<Enum>Model`. The relation it reads is the entity's `Related` relation to
> the target, and one that does not join exactly the variant's column to the
> target's key MUST be refused as `Error::Type` before anything is sent; a
> key its target table does not hold, which `load_one` gives as `None`, MUST
> be an `Error::Query` naming the table, the row's key, the target's key and
> the column, since the variant holds a `Model`.
>
> `Schema::create_one_of::<E>()` MUST return the `ALTER TABLE` adding a
> `CHECK` on the entity's table: `num_nonnulls(<columns>) = 1`, or `<= 1`
> where the enum has a unit variant, so PostgreSQL refuses a row referencing
> two targets, and refuses one referencing none unless the enum says a row
> may. It is named `ck-{table}-{enum}`, the table bare and the enum's name in
> snake_case, bounded as prefixed result names are
> (`[spec:pgorm:sem:query.graph.writer]`). The foreign keys are the
> relations' own, written by `create_table_from_entity`.
>
> Codegen MUST generate the enum for nullable foreign-key columns covered by
> exactly such a `CHECK`, named for the constraint, from DDL text and from
> statements described in code alike, and generate the columns as the
> ordinary nullable fields they are when no such `CHECK` covers them. Exactly
> such a `CHECK` is an enforced table `CHECK` whose expression is
> `num_nonnulls` of one or more distinct columns of the table compared `= 1`
> or `<= 1`, each column nullable and the one column of a foreign key of the
> table onto the one-column primary key of a table, of that key's type, and
> that table one the entity's `Relation` reaches by that foreign key's
> variant alone, so that the entity is `Related` to it. The enum is declared
> in the table's module after its `Relation` enum, `#[pgorm(entity =
> Entity)]`, deriving `Clone`, `Copy`, `Debug` and `OneOf` and allowing
> `clippy::enum_variant_names`, as a disjoint enum does. It is named in
> UpperCamelCase for the constraint's name after `ck-{table}-` where the name
> begins so, as `create_one_of` writes it, so a schema it wrote generates the
> enum it was written from, and for the whole name otherwise; an unnamed
> `CHECK` is named as PostgreSQL names it, `{table}_check`, or
> `{table}_{column}_check` over one column, numbered from 1 where an earlier
> `CHECK` of the table took that name. It has a variant per
> column, in the `CHECK`'s order, named for its target table in
> UpperCamelCase and holding its entity, `#[pgorm(column = Column::PostId)]
> Post(super::post::Entity)`, and for `<= 1` a unit variant `None`. A name
> the enum, its `<Enum>Key` or `<Enum>Model` would take that the module
> already declares or its code names otherwise, and two variants of one name,
> MUST be refused as ``TransformError("table `<table>`: its one-of
> reference's <item> `<name>` is named as <the other>")``.
