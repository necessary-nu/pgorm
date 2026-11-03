# Disjoint tables

This chapter covers a Rust enum stored across tables: a base table whose
every row has a kind, and one table for each kind that carries data of its
own, keyed by the base row's key. The enum is the whole description, and
PostgreSQL enforces it with constraints alone, so a state the enum cannot
describe cannot be written, by pgorm or by anything else connected to the
database. There are no triggers (`[spec:pgorm:req:sql.scope]` keeps them out
of the builder) and no single-table mapping, in which every kind's columns
would sit side by side in the base, mostly null.

## Declaration

> [spec:pgorm:def:disjoint.declare+1]
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
> generic parameters MUST NOT compile.
>
> The derive MUST generate the kind type, `<Enum>Kind`, an `ActiveEnum`
> stored as a PostgreSQL enum with one value per variant, in declaration
> order, each spelled as its variant's name in snake_case unless the
> variant's `#[pgorm(value = "...")]` gives another. The type is named
> `<enum>_kind`, the enum's name in snake_case, unless `enum_name = "..."`
> gives another, and `schema_name = "..."` qualifies it as it does an
> `ActiveEnum`'s (`[spec:pgorm:syn:macros.derive.active-enum]`). The base's
> kind column is the one `kind = vehicle::Column::Kind` names, the base's
> `Column::Kind` by default, and a base with no such column MUST NOT
> compile; a base row of a kind the enum does not list cannot exist, because
> the kind type has no such value. The derive MUST also generate
> `<Enum>ActiveModel`, the same variants each holding its kind entity's
> `ActiveModel`, which is what a write takes
> (`[spec:pgorm:sem:disjoint.write]`).
>
> Each kind's entity MUST have a primary key of the same types, in the same
> order, as the base's, since its row is keyed by the base row's key, and
> one whose key types differ MUST NOT compile. Any key qualifies, composite
> keys included, and a kind's key columns may be named differently from the
> base's. A kind entity is otherwise an ordinary entity: it can be read,
> joined and written on its own, subject to the constraints
> (`[spec:pgorm:sem:disjoint.schema]`), and a foreign key elsewhere can point
> at the base, meaning a row of any kind, or at one kind's table. What the
> types cannot tell — a kind column that is nullable or not of the kind type
> (its name, schema and values), or a kind entity whose key the database
> generates (drawn from a sequence, an identity or a generated column), when
> its values must come from the base row — MUST be refused as an
> `Error::Type` naming the table and column, by `check_disjoint::<E>()`,
> which `Schema::create_disjoint` and every read and write run before
> anything is sent.

## Schema

> [spec:pgorm:sem:disjoint.schema]
> `Schema::create_disjoint::<E>()` MUST return the statements that create a
> disjoint entity's type and tables, in the order they run:
>
> 1. the kind type;
> 2. the base table as `Schema::create_table_from_entity` writes it
>    (`[spec:pgorm:sem:schema.from-entity]`), with a `UNIQUE` constraint on
>    its key columns followed by its kind column;
> 3. each kind's table as `create_table_from_entity` writes it, with a kind
>    column named as the base's, of the kind type and `GENERATED ALWAYS AS
>    ('<value>') STORED`, and a foreign key from its key columns and that
>    column to the base's `UNIQUE` columns, `ON DELETE CASCADE`;
> 4. one `ALTER TABLE` on the base adding, for each kind with a table and
>    each key column, a column `<value>_<column>` of the key column's type,
>    `GENERATED ALWAYS AS (CASE WHEN <kind column> = '<value>' THEN <column>
>    END) STORED`, and for each such kind a foreign key from its columns to
>    the kind table's key.
>
> The base's foreign keys come last because each side references the other.
> Together the constraints MUST make PostgreSQL refuse every state the enum
> does not describe: a base row of a kind with a table but no row in it, a
> row in a kind's table whose base row is absent or of another kind, and so
> a base row with rows in two kinds' tables. A generated column is null for
> every kind but its own, and a null skips its foreign key, so each base row
> is held to exactly its own kind's table. A kind without a table constrains
> nothing beyond its value. Deleting a base row deletes its kind's row by
> the cascade; deleting a kind's row alone, or changing a base row's kind
> while its old kind's row remains, is refused.
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
> PostgreSQL's 63-byte identifier limit. A generated name equal to a column
> an entity declares MUST be refused with `Error::Type` naming both, before
> any statement is returned.

## Reads

> [spec:pgorm:sem:disjoint.read]
> `E::find()` MUST read base rows with their kinds' rows in one statement:
> the base `LEFT JOIN`ed to each kind's table on the key, each row read as
> the pair `(base::Model, E)`. A row decodes by its base row's kind: a kind
> with a table decodes its joined row into its variant, and a kind without
> one is its unit variant. A row whose kind has a table but whose joined row
> is absent, and a row with a joined row present for any other kind, can
> come only from a schema `[spec:pgorm:sem:disjoint.schema]` did not create,
> and MUST each be an error naming the base table, the key and the kinds
> involved, never a variant read without its row and never a row dropped.
>
> The read MUST take what a `Select` of the base takes — filters, ordering,
> limit and offset, pagination and keyset cursors — and `find_by_id(key)`.
> `kind(k)` and `kinds([..])` restrict it to base rows of those kinds, and a
> filter on a kind table's column reads only base rows of that kind whose
> row passes it. Its terminals are `one`, `all` and `stream`. The number of
> kinds is not bounded. Each joined table's result columns are named by the
> same prefixed, 63-byte-bounded composition a graph's sources are, so a
> kind's column never shadows a base column or another kind's of the same
> name.

## Writes

> [spec:pgorm:sem:disjoint.write]
> Every write MUST be one statement, its parts joined by data-modifying
> `WITH` clauses, so that it applies whole or not at all with no
> transaction, and its constraints are checked once, at its end
> (`[spec:pgorm:sem:disjoint.schema]`). Each returns the pair a read of the
> written row gives.
>
> - `E::insert(base, kind)`, given the base's `ActiveModel` and an
>   `<Enum>ActiveModel`, MUST insert the base row with the kind's value in
>   its kind column and, for a kind with a table, the kind's row with its key
>   taken from the inserted base row, so a key the database generates reaches
>   it. The base's kind column and the kind row's key columns MUST be
>   `NotSet`, since the kind and the key each come from one place; a value
>   set in either MUST be refused before anything is sent.
> - `E::update(base, kind)` MUST update the set columns of the base row and
>   of its kind's row, both found by the base's key. A kind other than the
>   row's current one, and a key no row has, MUST be `Error::RecordNotFound`
>   with both rows left as they were: changing kind is its own operation.
> - `E::change_kind(key, kind)` MUST delete the row's current kind row,
>   where its kind has a table, insert the new kind's row, where that kind
>   has one, and set the base row's kind. A key no row has, and a row that
>   already has that kind, MUST be `Error::RecordNotFound` with nothing
>   changed.
>
> Deleting needs nothing of its own: the base entity's delete removes a base
> row and, by the cascade, its kind's row.
