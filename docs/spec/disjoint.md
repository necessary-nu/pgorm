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

> [spec:pgorm:sem:disjoint.schema+1]
> `Schema::create_disjoint::<E>()` MUST run `check_disjoint::<E>()` first,
> returning its refusal as its own, and otherwise return the statements that
> create a disjoint entity's type and tables, a `Vec<DdlStatement>` in the
> order they run (`DdlStatement` being a `CREATE TYPE`, a `CREATE TABLE` or
> an `ALTER TABLE`, each rendering as its statement):
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

> [spec:pgorm:sem:disjoint.read+1]
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

## Writes

> [spec:pgorm:sem:disjoint.write+1]
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
> Deleting needs nothing of its own: the base entity's delete removes a base
> row and, by the cascade, its kind's row.

## Unkinded tables

> [spec:pgorm:sem:disjoint.unkinded]
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
> the error naming it read-only. Nor is one registrable with a binding
> (`[spec:pgorm:req:bindings.disjoint]`), whose registration takes a
> `KindedDisjoint`. `check_disjoint` refuses nothing of an unkinded entity,
> what it refuses being of a kind column and of the keys a write fills; a
> `DisjointShape` of one refuses each write and `create` as `Error::Type`
> naming its base as read-only, before anything is sent.
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
