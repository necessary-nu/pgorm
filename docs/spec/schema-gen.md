# Schema Generation from Entities

`src/schema/` projects compile-time entity definitions (`EntityTrait`) into
`pgorm_query` DDL statements. `Schema` is a stateless helper — `Schema::new()`
takes no backend argument because pgorm is PostgreSQL-only. The statements are
returned to the caller (typically migrations or test setup); nothing here
executes SQL.

## Table projection

> [spec:pgorm:sem:schema.from-entity+12]
> `Schema::create_table_from_entity::<E>()` returns one `TableCreateStatement`
> for `E`, or the refusal below: the table ref from `entity.table_ref()`, the
> entity comment if any,
> and one column per `E::Column` variant projected from `ColumnTrait::def()` —
> the declared `ColumnType` (with `Enum { name, .. }` rewritten to a named
> type reference naming the Postgres enum), `NOT NULL` unless the column is
> nullable, plus the column's default — a
> `DEFAULT` expression, a `GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY` or a
> `GENERATED ALWAYS AS (<expr>) { STORED | VIRTUAL }` of the kind it declares,
> whichever its definition holds (`entity.traits.column-def`) — and its
> comment. A key is the table's (`sql.ddl.create-table`): the entity's
> primary key becomes the table's, each `unique` column a one-column
> unique key, added in `E::Column` order, and then each key
> `EntityName::unique_keys` returns, in its order, under the name it gives
> or none and `NULLS NOT DISTINCT` when it says so
> (`[spec:pgorm:req:entity.traits.entity-name]`).
>
> The primary key is `E::PrimaryKey`'s columns in order, at any arity, the
> last written `WITHOUT OVERLAPS` when `E::PrimaryKey::without_overlaps()`
> says the key is temporal; a scalar column beside the period needs the
> `btree_gist` extension, which schema generation does not create
> (`[spec:pgorm:req:sql.ddl.extension]`; `42704` without it). A
> one-column key is left unnamed, so PostgreSQL names it `{table}_pkey`, and
> its column is drawn from the serial family when
> `E::PrimaryKey::auto_increment()` is true and the column holds no default of
> its own: PostgreSQL refuses `serial` beside a `DEFAULT`, an identity or a
> generation expression (42601), and a key filled by any of them is generated
> already. A composite key
> (arity > 1) is named `pk-{table}`, and no column of it is drawn from the
> serial family whatever
> `auto_increment()` says: a composite key's generated column is the one whose
> definition holds an identity, rendered on that column alone, so
> `(tenant_id, id)` with `id` an identity builds a table whose inserts name
> `tenant_id` and get `id`. Foreign keys are generated from `E::Relation` entries
> whose `RelationDef` has `is_owner == false` (the belongs-to side); owner-side
> relations produce no constraint. A foreign key carries the full table name
> of both sides — schema qualification included, via `unpack_table_name` — so
> a `REFERENCES` clause names the table the relation points at rather than
> whatever `search_path` resolves; the derived constraint name still uses the
> bare table.
>
> Every name generation derives rather than is given — a composite key's
> `pk-{table}`, a foreign key's `fk-{table}-{columns}`
> (`[spec:pgorm:req:entity.relation.fk]`), an index's `idx-{table}-{column}`
> (`schema.from-entity.index`) — MUST fit PostgreSQL's 63-byte identifier
> limit, and is composed and bounded once for all of them, as the names a
> disjoint schema gives are (`[spec:pgorm:sem:disjoint.schema]`): kept when
> shorter than 63 bytes, and otherwise spelled as its first 47 bytes and a
> hash of the whole in exactly 63, as prefixed result names are
> (`[spec:pgorm:sem:query.graph.writer]`). Left whole, a longer name is
> truncated by the server to its first 63 bytes with only a notice
> (`42622`), so two names sharing them become one — a second foreign key
> or index under it is refused (`42710`, `42P07`) — and a later statement
> naming the constraint by the name generation composed misses it. A name
> the entity gives (`fk_name`) is written as given.
>
> A foreign key PostgreSQL cannot implement is refused rather than written.
> The server implements none with a column on either side typed as a domain
> over an enum, directly or through further domains: it answers `42804`
> (*incompatible types*) between two columns of one such domain, between
> two domains over the same enum, and between such a column and the enum
> itself, in either direction. When any pair a belongs-to relation matches
> for equality has such a column on either side, generation MUST return
> `Error::Type` naming the entity, the relation and the pair, and MUST NOT
> leave the constraint out in silence. The side's type is read from the
> record a `ColumnPairs` keeps when its pair is named by entity columns
> (`entity.traits.column.enum-cast`), as the `Relation` derive and
> `RelationBuilder` name them; a pair named by bare names records none, so it
> is not checked and its key is written as declared. Every other pairing is
> written: an enum with the same enum, a domain over a base that is no enum
> with that base or the same domain, and a domain over an array of an enum,
> or an array of such a domain, with the same type, all of which the server
> keys. The only other way generation fails is a unique key
> `EntityName::unique_keys` describes with no columns, which an `EntityName`
> written by hand can say and PostgreSQL cannot write (`UNIQUE ()`): it is
> `Error::Type` naming the entity.
>
> Comments ride on the create statement (`get_comment()`,
> `ColumnSpec::Comment`) but are inert there — executing it attaches nothing
> (`[spec:pgorm:req:sql.ddl.create-table]`). They are a second statement
> stream instead: `Schema::create_comments_from_entity::<E>()` returns the
> `COMMENT ON` statements for the same entity — the entity comment first when
> `E::comment()` is set, then one per column whose `ColumnDef` carries a
> comment, in `E::Column` order — each targeting `entity.table_ref()`, so a
> comment lands on the same qualified name the table projection uses. The Vec
> is empty when no comment is declared. `table_ref()` is a `TableName`, which
> always names a table, so the comment target needs no conversion and the
> stream has no failure mode.

## Tables described as data

> [spec:pgorm:sem:schema.from-shape]
> A `TableShape` MUST describe a table as data — its `TableName`, its
> columns in order, each a name and an ORM `ColumnDef`, its primary key, a
> one-column key's serial family (`auto_increment`), a temporal key's
> `WITHOUT OVERLAPS`, its unique keys, its belongs-to relations and its
> comment — and `Schema::create_table_from_shape`,
> `create_enum_from_shape`, `create_index_from_shape` and
> `create_comments_from_shape` MUST write from it exactly the statements
> `schema.from-entity`, `schema.from-entity.enum` and
> `schema.from-entity.index` describe, in the same order, under the same
> derived names and with the same refusals. `TableShape::of::<E>()`
> describes an entity, and the four entity methods MUST be those four over
> it, so an entity's statements and a shape's come from one writer.
> `TableShape::new` describes a table known only at run time, as a language
> binding's model is: a column `with_unstated_default`, which the database
> fills by a default the description does not state, has no `DEFAULT` to
> write, so `create_table_from_shape` MUST refuse its table with
> `Error::Type` naming the column, as the disjoint schema refuses one
> (`[spec:pgorm:sem:disjoint.schema]`).
>
> What a shape cannot describe MUST be refused as it is described, with
> `Error::Type`: two columns of one name; a primary key of no column, of a
> column the table lacks or of one column twice; a unique key as
> `schema.from-entity` refuses one, or over a `VIRTUAL` generated column,
> which PostgreSQL cannot key (`0A000`), or named as another of the table's
> keys is, its composite primary key's `pk-{table}` included (`42P07`); and
> a relation that is an owner (`has_one`, `has_many`), which holds no key of
> the table, or pairs a column the table lacks. A relation's foreign key is
> written as its `RelationDef` converts (`[spec:pgorm:req:entity.relation.fk]`),
> and refused as `schema.from-entity` refuses a key over a domain over an
> enum, from the types its `ColumnPairs` records.

## Secondary indexes

> [spec:pgorm:sem:schema.from-entity.index+2]
> `Schema::create_index_from_entity::<E>()` returns one `IndexCreateStatement`
> per column whose `ColumnDef` has the `indexed` flag, named
> `idx-{table}-{column}`, bounded to 63 bytes as every derived name is
> (`schema.from-entity`), over that single column, and an empty `Vec` when no
> column is indexed. Each statement targets `entity.table_ref()`, the same ref
> the table projection uses, so the index is schema-qualified
> (`ON "{schema}"."{table}"`) exactly when the entity declares a
> `schema_name` and bare (`ON "{table}"`) when it does not. Unique columns are
> not covered here — uniqueness is emitted as a column-level unique key by the
> table projection, not as a separate index statement — and multi-column
> indexes cannot be expressed.

## Postgres enum types

> [spec:pgorm:sem:schema.from-entity.enum+3]
> `Schema::create_enum_from_entity::<E>()` scans `E::Column` and returns one
> `TypeCreateStatement` (`CREATE TYPE {name} AS ENUM ({variants})`) per enum
> its columns resolve to, preserving declared variant order; a column that
> resolves to no enum contributes no statement, so this form cannot fail. A
> column type resolves through one level of `ColumnType::Array`, so a column
> holding an array of a database enum names the enum the array is over: the
> table projection renders such a column as `{name}[]`, which no database can
> accept unless the element type exists, so an entity reaching an enum only
> through an array still carries the `CREATE TYPE` that makes its table
> creatable. The resolved enums are then deduplicated by type name across the
> entity's columns, first occurrence winning — two columns over one enum (a
> scalar and an array of it, say) yield one statement, because Postgres has no
> `CREATE TYPE IF NOT EXISTS` and re-creating a type is an error rather than a
> no-op. Both halves match the generation path
> (`[spec:pgorm:sem:codegen.entity.transform]`), which registers an
> `ActiveEnum` for every column whose array-inner type is `ColumnType::Enum`,
> keyed by enum name.
> `Schema::create_enum_from_active_enum::<A>()` builds the same statement from
> `A::db_type()` for a single `ActiveEnum`, and returns `Error::Type` naming the
> enum if the resolved column type is not `ColumnType::Enum` — an `ActiveEnum`
> backed by a plain column type has no database enum to create.
