# Personal data

This chapter covers personal data as the GDPR defines it: information
relating to an identified or identifiable natural person, the data subject
(Art. 4(1)). A program declares three things once, in a privacy module of its
own: its **subjects**, whose data it is; its **processing activities**, why it
processes the data, on which lawful basis and shared with whom; and its
**categories** of personal data, which activity each serves, how long it is
kept and what erasing it means. A field holding personal data names its
category and nothing else, and a column holding a subject's identifier says
whose the row is by its type. Nothing is written on a column that is not
personal.

From those declarations pgorm finds every row about a subject, exports and
erases it, erases what has outlived its retention, writes the database's share
of the record of processing activities, and keeps personal values out of the
text it produces itself. The declarations are compiled into the database too,
through a generated migration frozen as any other generated DDL is, so the
export and erasure a program calls are the database functions a support
engineer in `psql` or another service calls, and there is one implementation
of what erasing a subject means.

| GDPR | What pgorm does |
|---|---|
| Art. 5(1)(e), storage limitation | each category's retention, and the sweep that erases what outlived it (`personal.sweep`) |
| Art. 6, lawful basis; Art. 9, special categories | declared on activities and categories (`personal.activity`, `personal.category`) |
| Art. 15, access; Art. 20, portability | the export (`personal.export`) |
| Art. 17, erasure, and its exceptions in 17(3) | the erasure and its holds (`personal.erase`) |
| Art. 30, records of processing | generated from the declarations (`personal.records`) |
| Art. 32, security | masking in pgorm's own output, and pseudonymisation (`personal.not-secret`, `personal.pseudonymise`) |

## What pgorm does and does not do

pgorm supplies an inventory and the mechanics to act on it. Whether the
inventory is right, and whether the application around it meets the law, are
the application's.

> [spec:pgorm:def:personal.boundary]
> pgorm MUST be described, in this chapter, the guide and the API reference,
> as supplying an inventory of the personal data its entities and models hold
> and the mechanics to act on it, and never as making an application
> compliant with the GDPR or any other law. What a program declares is its
> own judgement: which data is personal and whose, which lawful basis an
> activity rests on (Art. 6), whether a category is special (Art. 9), and
> whether a hold applies (Art. 17(3)); pgorm checks that the declarations
> are complete and consistent, not that they are right. Outside pgorm, and
> named so in the guide: backups and replicas, and how long they keep what an
> erasure removed; the server's own logs, `log_statement`'s among them, and
> the bound parameters they print, which `log_parameter_max_length` and
> `log_parameter_max_length_on_error` set to `0` keep out; caches, search
> indexes, analytics and warehouse copies; files and objects the rows point
> to; the recipients the data was disclosed to, whom Art. 19 has the
> controller tell of an erasure; raw SQL and pgorm-query used directly, which
> no declaration reaches; who may read a column, which is the database's
> roles and column privileges; a request's identity checks and deadlines
> (Art. 12); rectification (Art. 16), which is an ordinary update;
> restriction of processing (Art. 18) and objection (Art. 21); and the parts
> of the Art. 30 record only the organisation knows (`personal.records`).

## Declarations

The three declarations are types, so the compiler checks every reference to
them, and they live in one module that every entity names them from. A
subject type wraps the identifier its values are; an activity and a category
are unit structs whose derive holds their keys.

> [spec:pgorm:def:personal.subject]
> `#[derive(Subject)]` on a `ValueType` newtype MUST implement `Subject` for
> it, which makes the type the identifier of a kind of data subject: each of
> its values identifies one person. `#[pgorm(subject(name = "educator", label
> = "Educators"))]` gives the subject's name, by default the type's name in
> snake_case, of lowercase letters, digits and `_` and at most 56 bytes so
> that `export_<name>`, the longest name pgorm gives one, fits PostgreSQL's
> 63-byte identifiers; and its label, the category of data subjects the
> record names (Art. 30(1)(c)), by default its name. The `ValueType` derive
> MUST pass over the `subject(..)` key and the `Subject` derive over
> `ValueType`'s keys.
>
> A field of a model whose type is a subject `S`, or an `Option<S>`, is a
> subject column of `S`: its row is linked to the person its value
> identifies, which the derive learns from the type's `Subject`
> implementation, however the type is spelled, so nothing is written on the
> field. A row whose subject column is `NULL` is linked to nobody by it, and
> an array of a subject's values links no row. A subject's home table is the
> inventory's table whose primary key is one subject column of it; a subject
> MAY have none, as one an external identity provider identifies has none.

A scope's columns are marked (`[spec:pgorm:def:scope.declare]`) because a
scope changes every statement on its table. A subject column changes nothing
pgorm builds, and says whom its row is about wherever it stands, a foreign key
as much as a primary key, so its type is the whole declaration.

> [spec:pgorm:def:personal.activity]
> `#[derive(Activity)]` on a unit struct MUST implement `Activity`: a
> processing activity, one entry of the record of processing (Art. 30(1)).
> Its keys are `name`, its identifier in the catalog and the record, by
> default the type's name in snake_case; `label`; `purposes = ["..", ..]`,
> the purposes it processes data for, at least one (Art. 5(1)(b), Art.
> 30(1)(b)); `basis`, its lawful basis under Art. 6(1), one of `Consent`,
> `Contract`, `LegalObligation`, `VitalInterests`, `PublicTask` and
> `LegitimateInterests`, points (a) to (f); `special`, the condition of Art.
> 9(2) under which it processes a special category, one of
> `ExplicitConsent`, `Employment`, `VitalInterests`, `NotForProfit`,
> `MadePublic`, `LegalClaims`, `SubstantialPublicInterest`,
> `HealthOrSocialCare`, `PublicHealth` and `ArchivingOrResearch`, points (a)
> to (j); `recipients = ["..", ..]`, the categories of recipients the data is
> disclosed to (Art. 30(1)(d)); and `transfers = [("..", ".."), ..]`, each a
> third country or international organisation and the safeguard the
> transfer rests on (Art. 30(1)(e)). `purposes` and `basis` are required.
> pgorm records each as written and judges none (`personal.boundary`).

> [spec:pgorm:def:personal.category]
> `#[derive(PersonalData)]` on a unit struct MUST implement `PersonalData`: a
> category of personal data, shared by every field that names it. Its keys
> are `name`, by default the type's name in snake_case, and `label`;
> `activity`, the activity processing it, or a parenthesised list of
> several; `retain`, how long its fields are kept: `until_erasure`, until
> the subject is erased or the program deletes the row, or `retain(for =
> <period>, from = <field>)`, a period of `days(n)`, `months(n)` or
> `years(n)` with `n` positive, counted from a clock
> (`personal.retention-clock`); `erase`, what erasing one of its fields does
> (`personal.erasure`): `null`, `replace(<literal>)`, `pseudonymise` or
> `delete_row`; `hold`, the ground of Art. 17(3) on which its fields are
> kept past a subject's erasure until their period ends: `Expression`,
> `LegalObligation` (point (b), a task in the public interest's and
> official authority's included), `PublicHealth`, `ArchivingOrResearch` and
> `LegalClaims`, points (a) to (e); `special`, the special category of Art.
> 9(1) it is: `RacialOrEthnicOrigin`, `PoliticalOpinions`,
> `ReligiousOrPhilosophicalBeliefs`, `TradeUnionMembership`, `Genetic`,
> `Biometric` (processed to identify a person uniquely), `Health` and
> `SexLifeOrOrientation`; `source`, where its data comes from: `subject`,
> the default, for what the subject gave or their use of the service
> recorded, `derived`, for what the controller inferred, or
> `third_party("..")`, naming the source (Art. 14(2)(f), Art. 15(1)(g)); and
> `anon`, a PostgreSQL Anonymizer masking rule (`personal.anonymizer`).
> `activity`, `retain` and `erase` are required: each is a choice about the
> data, and a default would be a choice the program did not make.

> [spec:pgorm:def:personal.field]
> In the composite `Entity` derive a field keyed `#[pgorm(personal =
> Category)]` MUST declare its column personal data of that category, its
> type kept as written (`personal.not-secret`). Its subject is found from
> its row: the row's one subject column when it holds one, and otherwise the
> one `of` names, `of = <field>`, a subject column of the same model, or `of
> = via(path::Entity)`, the subject of the row in that entity a belongs-to
> relation of the model reaches, a row that MUST itself link exactly one
> subject. `of` on the container sets it for every personal field of the
> model that gives none. A subject column MAY itself be personal, naming the
> category whose erasure detaches or deletes its row, its subject its own
> value: a person's link to a row is personal data too, as an enrolment's
> `educator_id` is. `personal_label = ".."` names the field in exports and
> the record, by default the category's label and the field's name, `CV
> answer: qualifications`.

So `message.body`, its sender's words, is `#[pgorm(personal = MessageBody,
of = sender_id)]`: the row holds two subject columns of one subject, and only
the field knows which person it belongs to. A message's export reaches its
sender, and erasing the sender erases it; its recipient's own link to the row
is `recipient_id`, which may name a category of its own.

> [spec:pgorm:req:personal.retention-clock]
> A category retained for a period MUST count it, on each row, from the
> row's clock: the model's field keyed `#[pgorm(retain_from)]` when it has
> one, the clock of every timed category on the model, and otherwise the
> field the category's `from` names. A clock MUST be a date, a timestamp or
> a timestamp with time zone, or an `Option` of one. A row whose clock is
> `NULL` has not started its period and is not swept, so a category kept for
> six years after a contract's end waits for the end. A model holding a
> field of a timed category without a clock, which neither has a
> `retain_from` field nor the field the category's `from` names, a category
> naming no `from` included, MUST NOT compile, and neither MUST a model with
> two `retain_from` fields. A category retained `until_erasure` has no
> clock.

The period is the category's, since the category is the unit of a retention
policy; the clock is the table's, since only the table knows when its row
began. A category names the field most tables keep it in, `created_at`, and a
table keeping it elsewhere says so once.

> [spec:pgorm:sem:personal.erasure]
> Erasing a personal field MUST do what its category's `erase` says: `null`
> writes `NULL`; `replace(v)` writes `v`; `pseudonymise` writes the value's
> keyed pseudonym (`personal.pseudonymise`), leaving `NULL` as it is; and
> `delete_row` deletes the row, whose reason to exist was the field. On one
> row each field is erased by its own category, and a row any of whose
> erased fields' categories deletes it is deleted rather than updated. A
> subject's erasure MUST NOT erase a field of a held category while its
> row's period runs: the field is kept and reported (`personal.erase`), and
> the sweep erases it when the period ends (`personal.sweep`).
> Pseudonymised data is still personal data (Art. 4(5), Recital 26): whoever
> holds the key can test a guessed value against a pseudonym. So pgorm MUST
> report a pseudonymised field as pseudonymised, never as erased, and
> `pseudonymise` is for a field whose equality must outlive its subject, a
> join's or a count's, not a way to anonymise it.

> [spec:pgorm:req:personal.opt-out]
> A model whose table links a subject, by a subject column or by a
> belongs-to relation into a subject's home table, and that holds no
> personal field MUST say so with `#[pgorm(no_personal_data)]` on its
> container: the one line by which a program records that it reviewed the
> table and holds nothing in it personal beyond the link. Without it the
> table is a gap (`personal.completeness`); beside a personal field it MUST
> NOT compile. The table's columns are recorded as reviewed
> (`personal.catalog`), so a column added later changes the generated layer
> and is reviewed when its file is frozen (`personal.drift`).

> [spec:pgorm:req:personal.checks]
> What the declarations cannot mean MUST NOT compile, each refusal worded by
> pgorm, naming the model, the field and the category it reads, and each
> held by a compile-fail snapshot:
>
> - a `personal` key naming a type that is not `PersonalData`, an `of`
>   naming a field that is not a subject column, and an `of = via(E)` whose
>   `E` the model is not `Related` to;
> - a personal field of a row holding no subject column and given no `of`,
>   and one of a row holding two or more subject columns, of one subject or
>   of several, given no `of`;
> - a field of a `null` category whose column is not nullable; of a
>   `replace` category whose literal does not convert into the field's type,
>   or whose column is unique, since every erased row would hold the one
>   value; and of a `pseudonymise` category whose column is not text;
> - a personal key on a field that is `ignore`d, generated, immutable
>   (`[spec:pgorm:req:scope.immutable]`, a scope column's included) or part
>   of the primary key, unless its category deletes the row, since erasure
>   cannot write the column;
> - a subject column personal in a category that replaces or pseudonymises;
> - `personal` twice on one field, a timed category's field without a clock
>   (`personal.retention-clock`), and `no_personal_data` beside a personal
>   field (`personal.opt-out`);
> - an activity without a purpose or a basis; a category with a `special`
>   key one of whose activities names no Art. 9(2) condition; a category
>   with a `hold` and no timed `retain`; and a subject's, activity's or
>   category's name pgorm would not take (`personal.subject`).
>
> A derive cannot see another's declaration, so a refusal reading one, a
> category's strategy from a model's field or an activity's condition from a
> category, MUST be a constant the compiler evaluates as the crate compiles,
> its message the derive's own.

> [spec:pgorm:def:personal.as-data]
> The declarations MUST also be data a program can build as it runs, as
> scopes are (`[spec:pgorm:def:scope.declare]`): `SubjectDef`, `ActivityDef`
> and `CategoryDef` hold a subject, an activity and a category with the keys
> their derives take, each derive's trait giving its own; a column's part is
> its `ColumnDef`'s, `subject` for a subject column, `personal` with the
> category and the `Link` its subject is found by for a personal one, and
> `retain_from` for a clock; and a `TableShape`
> (`[spec:pgorm:sem:schema.from-shape]`) records `no_personal_data`.
> `TableShape::of::<E>()` MUST carry an entity's, so an entity and a table
> described as data reach an inventory by one path. What `personal.checks`
> refuses at compile time a shape MUST be refused for when it reaches an
> inventory (`personal.inventory`), in the same words, since a shape built
> as the program runs has no compile time to be refused in.

## The inventory

Rust has no list of a program's types, so a program lists its entities once,
as it lists its migrations. The completeness check finds what the list holds
and cannot account for, and `verify` finds a table the list forgot, from the
database.

> [spec:pgorm:def:personal.inventory]
> `Inventory::new()` MUST start an empty inventory, `entity::<E>()` and
> `shape(TableShape)` add a table to it, and `build()` MUST return the
> `Inventory` or refuse it with `Error::Personal(PersonalError::Declaration)`
> naming what it refuses, before anything is generated or sent: a table
> added twice; two subjects, activities or categories of one name; a subject
> with two home tables; a `via` whose relation is not a belongs-to of its
> table, whose target the inventory lacks, or whose target does not link
> exactly one subject; a pseudonymised column declared shorter than a
> pseudonym (`personal.pseudonymise`); a table holding fields of one
> subject's held category beside a category of the same subject that deletes
> the row; a cycle among the belongs-to relations of the tables erasure or
> the sweep deletes from; and a belongs-to relation into a table erasure or
> the sweep deletes from whose `on_delete` is `Restrict`, `NoAction` or
> unset, since a row erasure leaves would reference a row it deletes. A
> relation is exempt when every row it references from is deleted first by
> the same erasure: its column is the subject column through which its
> table's fields of a category deleting the row, with no hold, are erased.
> The sweep has no such exemption, since its rows go by their clocks and not
> together.
>
> `schema(name)` MUST set the schema the generated layer lives in,
> `pgorm_personal` by default, and `pgcrypto_schema(name)` the schema
> pgcrypto's functions are called in, `public` by default. `pgorm::personal`
> is compiled with the `with-json` feature, which is on by default.

> [spec:pgorm:req:personal.completeness]
> `Inventory::completeness()` MUST list, as `Gap`s, what the inventory holds
> and cannot account for: a table that links a subject, by a subject column
> or by a belongs-to relation into a subject's home table, and holds neither
> a personal field nor `no_personal_data`; and a belongs-to relation into a
> subject's home table whose column is not of the subject's type, which
> links the row to a person while its type says nothing of it. An empty list
> is what a program's test asserts. A column added to a table that links a
> subject is not a gap: it changes the layer, and the drift test asks for a
> new file (`personal.drift`), whose review is the column's. A table the
> inventory lacks is `verify`'s to find (`personal.verify`).

This replaces marking every column that is not personal. Only a personal field
is marked; a table with none says so once; a column added later reaches review
through the drift test rather than passing silently as not personal.

## The database layer

The code is the source of truth, and the database holds a generated copy of
it: a catalog, the columns' comments, and the functions that export, erase
and sweep. One generator writes it into a migration file frozen as any other
generated DDL is (`[spec:pgorm:def:migration.frozen]`), and the API calls its
functions rather than building statements of its own.

> [spec:pgorm:def:personal.catalog]
> The layer MUST create in its schema the catalog tables `subjects` (`name`,
> `label`, the home table's `home_schema` and `home_table`, and `key_type`),
> `activities` (`name`, `label`, `purposes`, `basis`, `special_condition`,
> `recipients`, `transfers`), `categories` (`name`, `label`, `activities`,
> `retain_for`, an `interval`, `NULL` for until erasure, `retain_from`,
> `erasure`, `replacement`, `hold`, `special`, `source`), `columns`, a row
> for each column of each table of the inventory that links a subject or
> holds a personal field, with its `table_schema`, `table_name` and
> `column_name`, the `subject` it identifies if it is a subject column, its
> `category`, `link` and `label` if it is personal, and whether it is its
> row's `clock`, and `layer`, one row holding the layer's `digest`
> (`personal.frozen`). Each file MUST create the tables where they are not
> and replace their rows whole, so the catalog states the declarations its
> file was frozen from and no other. A column
> the catalog lists with neither a subject nor a category is one the program
> reviewed and holds not personal (`personal.opt-out`). The catalog grants
> nothing: it is documentation the database's own tools can read, and who
> reads it is the program's grant.
>
> A personal column's comment, as its `ColumnDef` gives it, MUST be the
> comment it declares, if any, followed by a line of its classification,
> `personal data: cv_answer of educator; erased by delete_row; kept until
> erasure`, so `Schema::create_comments_from_entity`
> (`[spec:pgorm:sem:schema.from-entity]`) writes the classification wherever
> it writes comments, and the layer MUST write the same comment on each
> personal column.

> [spec:pgorm:sem:personal.export-function]
> For each subject the layer MUST create `<schema>.export_<subject>(id <key
> type>) RETURNS jsonb`, `STABLE`, which reads, for the person `id`
> identifies, every row of every table holding a personal field of the
> subject, by the field's subject column or through its `via` relations, and
> returns the export `personal.export` describes: each row's primary key and
> each of the subject's personal fields on it, held categories' included,
> with the catalog's entry for each category and activity those fields name.
> A row holds a subject's field only by that field's link, so a message's
> body is exported to its sender, whom its `of` names, and not to its
> recipient.

> [spec:pgorm:sem:personal.erase-function]
> For each subject the layer MUST create `<schema>.erase_<subject>(id <key
> type>, key bytea DEFAULT NULL) RETURNS jsonb`, `VOLATILE`, which erases
> every personal field of the person `id` identifies as `personal.erasure`
> says, inside the transaction of the statement calling it, so all of it
> happens or none. It MUST first lock the subject's home row `FOR UPDATE`,
> where the subject has a home table, so that a row inserted referencing it,
> whose foreign-key check takes `FOR KEY SHARE` on it, waits for the
> erasure's transaction to end; then erase table by table, each before every
> table it references through a belongs-to relation of the inventory, so a
> row is deleted before the row it references; and touch only rows with
> something left to erase, so a second erasure of the same person changes
> nothing and reports nothing. It returns the report `personal.erase`
> describes, of counts, categories and dates, never a value or a key; rows
> the database deletes by a foreign key's `CASCADE` are the database's, and
> the report does not count them. A call for a subject one of whose
> categories pseudonymises, given no key, MUST raise before anything
> changes, with SQLSTATE `22004`.

> [spec:pgorm:req:personal.pseudonymise]
> A pseudonym MUST be the text `pseudonym:` followed by the 64 lowercase
> hexadecimal digits of the HMAC-SHA-256 of the value's UTF-8 bytes under the
> key, computed in the database by pgcrypto's `hmac(bytea, bytea, text)`,
> called in the inventory's pgcrypto schema, which a layer that pseudonymises
> creates the extension in with `CREATE EXTENSION IF NOT EXISTS`, under the
> privilege the server asks of it. A value already of that form MUST be left
> as it is, so erasing or sweeping twice never pseudonymises a pseudonym.
> Equal values under one key have equal pseudonyms, so a join or a uniqueness
> that held over the values holds over the pseudonyms; a column declared
> shorter than a pseudonym's 74 characters is refused by `build()`.
>
> The key is not pgorm's to keep. pgorm MUST NOT store it in the database,
> the catalog, a frozen file or anything else it writes, and MUST bind it as
> a parameter, never in a statement's text (`personal.key`). Keeping it apart
> from the rows is what makes the result pseudonymised (Art. 4(5)): a key
> stored beside them would let whoever holds a dump test a guessed value
> against every pseudonym. Where the key lives, who may read it, and whether
> and how it rotates are the program's: a key changed between two erasures
> gives one value two pseudonyms, which pgorm does not reconcile, and a lost
> key undoes nothing already pseudonymised. A caller in `psql` writes the key
> into its statement's text, which the server logs wherever `log_statement`
> logs that statement.

> [spec:pgorm:req:personal.function-security]
> Each generated function MUST be `SECURITY DEFINER`, so that a role granted
> `EXECUTE` on it exports or erases without privileges on the tables, as a
> support role in `psql` does; MUST set its `search_path` to `pg_catalog,
> pg_temp` and name every object it touches by its schema, as PostgreSQL's
> documentation advises for such a function; and its file MUST `REVOKE ALL
> ON FUNCTION .. FROM PUBLIC` after creating it, since PostgreSQL grants
> `EXECUTE` to `PUBLIC` by default, and a definer's erasure open to every
> role lets any of them erase anyone. Granting `EXECUTE` is the program's, as
> every role's privileges are. A file MUST create each function with `CREATE
> OR REPLACE`, which keeps its owner and its grants; a function's signature
> is fixed by its subject's name and key type, so a later file replaces it
> rather than adding another. A function of a subject no longer declared
> MUST be dropped by the file, its grants with it.

> [spec:pgorm:req:personal.frozen]
> `Inventory::migration_file(name)` MUST return the layer as a
> `MigrationFile` (`[spec:pgorm:def:migration.frozen]`), its text opening
> with the line `-- pgorm: personal-data <digest>`, the digest the lowercase
> hexadecimal SHA-256 of the text after that line, which the catalog's
> `layer` row holds too; then, in order, the schema, the catalog, the
> columns' comments, pgcrypto where the layer pseudonymises, the functions
> and their revocations, and the drop of the functions no longer declared.
> The text MUST depend on the declarations alone, in an order they fix, so
> one inventory writes one file on every machine. A file is frozen once and
> never rewritten: a changed declaration is a new file, which replaces the
> layer whole.

> [spec:pgorm:req:personal.drift]
> `Inventory::check_frozen(dir)` MUST be the drift test. It reads the `.sql`
> files in `dir` as `SqlMigration::read_dir` does
> (`[spec:pgorm:req:migration.sql-file.read]`), takes the last, in the order
> the runner applies them, whose text opens with the personal-data line, and
> MUST refuse with `PersonalError::Drift`, naming that file, or none, and
> both digests, when there is none or its text is not the text
> `migration_file` writes now. It holds the frozen layer to the declarations
> as the codegen goldens are held to their sources: it reads files and
> connects to nothing, so it runs as a unit test, and it is answered by
> freezing a new file, never by editing the last. A program's test calling
> it fails whenever a subject, activity or category, a personal field, a
> link, a clock or a column of a table that links a subject changes without
> the file that carries the change.

## Export and erasure

Each call is one statement that checks the layer before it runs: the program
and the database must mean the same declarations, or neither claims to have
exported or erased anything.

> [spec:pgorm:req:personal.export]
> `Inventory::export(&conn, &subject)`, on any `Connection`
> (`[spec:pgorm:def:read-only.traits]`), MUST call the subject's export
> function and return a `SubjectExport`: the subject's name, label and
> identifier; the layer's digest; the activities and categories its fields
> name, each once, with every key it was declared with; and, table by table,
> each row's primary key and its fields, each with its field and column
> name, its label, its category, its value as JSON, and `portable`, true
> when its category's source is the subject and each of its activities
> rests on consent or a contract (Art. 6(1)(a) and (b)), the data Art. 20
> lets the subject take elsewhere. Art. 15(1)'s purposes, categories,
> recipients, retention and source reach each field through its category.
> The export is the subject's own data: its `Debug` MUST mask every value,
> and serializing it (`serde::Serialize`) MUST write them, since handing
> them to the subject is what it is for.
>
> The call MUST be one round trip that compares the catalog's `layer` digest
> with the inventory's and calls the function only where they agree; a
> disagreement or a missing layer MUST be `PersonalError::Drift` naming
> both, nothing read. A subject the inventory does not hold MUST be
> `PersonalError::UnknownSubject`, and a native Turso connection MUST be
> refused with the driver's unsupported-operation error, since the layer is
> PostgreSQL's.

> [spec:pgorm:req:personal.erase]
> `Inventory::erase(&conn, &subject)`, on a `ConnectionMut`
> (`[spec:pgorm:req:read-only.writes]`), MUST call the subject's erase
> function under the digest check `personal.export` makes, and return an
> `ErasureReport`: the subject, the digest, and table by table the rows
> deleted and updated, and for each field its category, its erasure and the
> rows it changed; and for each held field its category, its ground, the
> rows held and the date the last of their periods ends, absent while one
> has not started. A pseudonymised field is reported as pseudonymised. The
> function runs inside the transaction of the statement calling it, so on a
> `DatabaseTransaction` the erasure commits or rolls back with the program's
> own work, a record of the request among it. The erasure of a subject one
> of whose categories pseudonymises MUST be given the key, through
> `Inventory::key(&key).erase(..)`, and refused with
> `PersonalError::KeyRequired` before anything is sent when it is not.

> [spec:pgorm:req:personal.key]
> `PseudonymKey::new(bytes)` MUST refuse a key shorter than 32 bytes with
> `PersonalError::KeyTooShort`, and hold the key so that nothing of pgorm's
> prints it: its `Debug` prints `<redacted>`, and it has no `Display`,
> `Deref` or `Serialize`. `Inventory::key(&key)` MUST lend a view whose
> `erase` and `sweep` bind it to the function as a `bytea` parameter, and
> pgorm MUST NOT keep it past the call (`personal.pseudonymise`).

> [spec:pgorm:req:personal.verify]
> `Inventory::verify(&conn)` MUST read the database's catalogs and return
> the `Gap`s the inventory cannot see alone: a layer missing, or holding
> another digest, which a deployment whose migrations are behind or ahead of
> the program shows; a table outside the layer's schema with a foreign key
> into a subject's home table that the inventory lacks; a personal column
> the database lacks or holds under another type; a foreign key into a
> table erasure or the sweep deletes from that the inventory's relations do
> not declare, or declare with another `ON DELETE` action, since `build`'s
> order and refusals were computed from the relations; and pgcrypto missing
> from its schema where the layer pseudonymises. Beside `completeness()`, an
> empty list is what a program's test against a database migrated to its
> head asserts.

## Retention

> [spec:pgorm:sem:personal.sweep]
> The layer MUST create `<schema>.sweep(batch integer, key bytea DEFAULT
> NULL) RETURNS jsonb`, which erases, by each timed category's strategy, its
> fields on every row whose clock plus its period lies before the
> transaction's time (`now()`, `current_date` against a date,
> `localtimestamp` against a timestamp without time zone), held categories'
> included, since a hold lasts only as long as the period (Art. 5(1)(e),
> Art. 17(3)): at most `batch` rows of each table and category a call, the
> tables in the order erasure takes them, touching only rows with something
> left to erase; and returns the rows it changed for each table and
> category, and whether any are left. `Inventory::sweep(&conn, batch)`, on a
> `ConnectionMut`, MUST call it under the digest check `personal.export`
> makes and return a `SweepReport` of the same, given the key through
> `key(&key)` where a timed category pseudonymises, as `erase` is. A
> category retained until erasure is never swept. Scheduling the sweep is
> the program's: a task calling `sweep` until nothing is left, or pg_cron's
> `cron.schedule` where that extension is installed, which managed services
> offer or do not.

## Records of processing

> [spec:pgorm:req:personal.records]
> `Inventory::records()` MUST return `Records`, the database's share of the
> record of processing activities (Art. 30(1)), generated from the
> declarations: for each activity a field of the inventory serves, its name
> and label, its purposes (point (b)), its lawful basis and any Art. 9(2)
> condition, the categories of data subjects whose fields it processes, and
> the categories of personal data with each one's label, special category
> and source (point (c)), its recipients (point (d)), its transfers and
> their safeguards (point (e)), each category's retention as its time limit
> for erasure (point (f)), and the tables and columns holding each category.
> `Records` MUST implement `serde::Serialize` and `Display`, the latter a
> Markdown document of one section per activity. The controller's name and
> contact details (point (a)) and its security measures (point (g)) are the
> organisation's to add, and `Records` holds neither; an activity no field
> serves is not in it, since what pgorm records is the processing of the
> data the database holds.

## Masking

Personal is not secret. A name is shown, searched and sent, so a personal
field stays the type it is; pgorm keeps it out only of the text pgorm writes
itself, where nobody chose to show it.

> [spec:pgorm:def:personal.not-secret]
> A personal field MUST keep its type: pgorm neither wraps nor hides it in
> the program's own code, its serialization included. pgorm MUST mask it
> only where pgorm writes text, a model's and an active model's `Debug`,
> `Value`s and the statements holding them, its errors, and its logs and
> metrics, through the machinery that masks a `Redacted<T>` (the `redact`
> container), printing `<personal>` where a secret prints `<redacted>`. A
> value that must never be shown, a credential or a national identity
> number, is a `Redacted<T>`, which masks everywhere and is read by
> `expose()`, and the two compose: `#[pgorm(personal = Identity)]
> national_id: Redacted<String>` is personal data masked as a secret.

> [spec:pgorm:req:personal.mask-debug]
> The composite `Entity` derive MUST implement `Debug` for a `Model` holding
> a personal field and for its `ActiveModel`, printing each personal field
> as `<personal>`, an active value's state kept (`Set(<personal>)`), and
> every other field as the standard derive prints it; a model without a
> personal field is left to the program's derive. A program's own
> `#[derive(Debug)]` on such a model conflicts with pgorm's and does not
> compile, so no forgotten debug print shows a personal value. A
> `PartialModel`, a `FromQueryResult` type, a tuple or JSON a program
> selects a personal column into is the program's and masks nothing, as a
> redacted column read into a plain type is not masked.

> [spec:pgorm:req:personal.mask-values]
> A value a personal column's definition converts, an active model's field
> an insert or update binds and a plain value one of its predicates binds
> (`Column::Email.eq(..)`, `is_in`, `between` and the rest), MUST become the
> redacted `Value` a `Redacted<T>` becomes, marked personal: bound as its
> inner value, and printed `<personal>` by `Value`'s and `Values`' `Debug`
> and `Display`, by a statement's `Debug` and its `build()` output, and by
> inline rendering. A `PseudonymKey` bound to a call MUST be a redacted
> value too, printed `<redacted>`.

> [spec:pgorm:req:personal.mask-errors]
> `pgorm::Error`'s `Display` and `Debug` MUST scrub personal values as they
> scrub redacted ones: a statement's personal parameters wherever the
> server's message echoes their text, a `DETAIL` echoing a row (`Failing row
> contains ..`) of a table holding a personal column, which is dropped, and
> a key's values (`Key (email)=(..)`), masked when the key names a personal
> column. `as_db_error()` stays the explicit route to the server's error as
> it was.

> [spec:pgorm:req:personal.mask-logs]
> pgorm's logs and what it reports to a metrics collector MUST NOT carry a
> personal value: a `QueryContext`'s `sql()` is a statement's text, its
> values bound rather than inlined; the error it reports is the scrubbed
> `pgorm::Error`; and pgorm's `tracing` events and `LoggingMetrics` print
> only these. The server's own logs are the server's (`personal.boundary`).

## PostgreSQL Anonymizer

The PostgreSQL Anonymizer extension masks columns by rules given as security
labels: for chosen roles as they read, in place, and in anonymised dumps of a
database for development. It is optional, and many managed services do not
offer it, so its labels are a file of their own.

> [spec:pgorm:req:personal.anonymizer]
> `Inventory::anonymizer_file(name)` MUST return a `MigrationFile` opening
> with the line `-- pgorm: personal-data-anon <digest>`, then a `SECURITY
> LABEL FOR anon ON COLUMN .. IS '..'` statement for each personal column
> with a masking rule: a `null` category's is `MASKED WITH VALUE NULL`, a
> `replace(v)` category's `MASKED WITH VALUE` the literal, and a category's
> `anon = ".."`, written as the rule verbatim, gives any category one and
> overrides those; a closing comment lists each personal column with none.
> The file MUST be separate from the layer's and its own choice, since the
> server refuses a security label for a provider it has not loaded, and
> pgorm neither installs nor configures the extension.
> `Inventory::check_frozen_anonymizer(dir)` MUST hold it to the declarations
> as `check_frozen` holds the layer (`personal.drift`).

## The guide

> [spec:pgorm:req:personal.guide]
> The Rust guide MUST gain a chapter, `personal-data`, with the change that
> delivers the API: the privacy module declaring subjects, activities and
> categories over the bakery's customers and messages; fields naming their
> category, `of`, a clock and `no_personal_data`; the inventory, its
> completeness check and `verify`; freezing the layer and the drift test;
> export and erasure; and what stays outside pgorm (`personal.boundary`),
> said as plainly. Each later change adds its section with it, masking, the
> sweep and the record, the Anonymizer file, and the Python and JavaScript
> guides' chapter of the same id says what their binding lacks until its
> change brings it (`[spec:pgorm:req:guide.dialects]`).
