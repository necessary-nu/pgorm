"""Codegen inputs: DDL and writer options, judged by whether the output builds.

`pgorm-codegen` is checked elsewhere against golden token text, which answers
whether the generator still emits what it used to emit and nothing about
whether a compiler accepts it. These cases feed its real output to rustc.

Two refusals live on this surface and must not be confused. Options the writer
cannot read and DDL the bridge does not carry are refused by the *generator*,
before any source exists; everything it does emit is then the compiler's to
judge. A suite that scored a generator refusal as a compile rejection would
report a type system holding a line that was never reached.
"""

from .compile_case import CompileCase, rejects

OBLIGATION = "new-rust-entities"

SIMPLE = "CREATE TABLE owner (\n    id serial PRIMARY KEY,\n    name text NOT NULL\n);"

RELATED = (
    SIMPLE + "\n\nCREATE TABLE task (\n"
    "    id bigserial PRIMARY KEY,\n"
    "    owner_id integer NOT NULL REFERENCES owner (id),\n"
    "    title text NOT NULL,\n"
    "    note text\n);"
)

JUNCTION = (
    RELATED
    + "\n\nCREATE TABLE label (\n    id serial PRIMARY KEY,\n    name text NOT NULL\n);"
    "\n\nCREATE TABLE task_label (\n"
    "    task_id bigint NOT NULL REFERENCES task (id),\n"
    "    label_id integer NOT NULL REFERENCES label (id),\n"
    "    PRIMARY KEY (task_id, label_id)\n);"
)

COMPOSITE_FOREIGN_KEY = (
    "CREATE TABLE tenant_task (\n"
    "    tenant_id integer NOT NULL,\n"
    "    id integer NOT NULL,\n"
    "    title text NOT NULL,\n"
    "    PRIMARY KEY (tenant_id, id)\n);"
    "\n\nCREATE TABLE tenant_task_note (\n"
    "    tenant_id integer NOT NULL,\n"
    "    id integer NOT NULL,\n"
    "    task_id integer NOT NULL,\n"
    "    body text,\n"
    "    PRIMARY KEY (tenant_id, id),\n"
    "    FOREIGN KEY (tenant_id, task_id) REFERENCES tenant_task (tenant_id, id)\n);"
)

ENUMERATED = (
    "CREATE TYPE task_state AS ENUM ('open', 'closed');\n\n"
    "CREATE TABLE task (\n"
    "    id serial PRIMARY KEY,\n"
    "    state task_state NOT NULL,\n"
    "    previous task_state\n);"
)

WIDE = (
    "CREATE TABLE wide (\n"
    "    id bigserial PRIMARY KEY,\n"
    "    weight double precision,\n"
    "    exact numeric(10, 2),\n"
    "    tags text[],\n"
    "    due timestamptz,\n"
    "    day date,\n"
    "    body jsonb,\n"
    "    ref uuid,\n"
    "    raw bytea,\n"
    "    flag boolean NOT NULL\n);"
)

KEYWORDS = (
    "CREATE TABLE rust_keyword (\n"
    "    id serial PRIMARY KEY,\n"
    '    "type" integer NOT NULL,\n'
    '    "match" text,\n'
    '    "ref" boolean NOT NULL,\n'
    '    "self" text\n);'
)


def _case(identity, sql, note, **options):
    """An accepted codegen case: generation succeeds and the output compiles."""
    request = {"id": identity, "sql": sql}
    request.update(options)
    needs = ("serde",) if options.get("with_serde") else ()
    return CompileCase(
        id=identity,
        obligation=OBLIGATION,
        verdict="accept",
        phase="typeck",
        kind="codegen",
        request=request,
        needs=needs,
        note=note,
    )


def schema_cases():
    """One generation per DDL shape, at the writer's default options."""
    schemas = (
        ("codegen-simple", SIMPLE, "one table, one serial key"),
        ("codegen-related", RELATED, "a foreign key becomes a derived relation"),
        ("codegen-junction", JUNCTION, "a composite-key junction and its two edges"),
        (
            "codegen-composite-foreign-key",
            COMPOSITE_FOREIGN_KEY,
            "a two-column foreign key onto a two-column key, and its inverse",
        ),
        ("codegen-enum", ENUMERATED, "an enum type, nullable and not"),
        ("codegen-wide", WIDE, "every column type the bridge maps"),
        ("codegen-keywords", KEYWORDS, "column names that are Rust keywords"),
    )
    return [_case(identity, sql, note) for identity, sql, note in schemas]


def option_cases():
    """The writer options, each varied against a schema that exercises it."""
    return [
        _case(
            "codegen-schema-name",
            RELATED,
            "schema_name reaches every generated entity",
            schema_name="fixture",
        ),
        _case(
            "codegen-lib",
            RELATED,
            "the index file is a crate root rather than a module",
            lib=True,
        ),
        _case(
            "codegen-copy-enums",
            ENUMERATED,
            "with_copy_enums adds Copy to the generated enum",
            with_copy_enums=True,
        ),
        _case(
            "codegen-serde-both",
            RELATED,
            "serialize and deserialize together",
            with_serde="both",
        ),
        _case(
            "codegen-serde-skips",
            RELATED,
            "the two serde skip flags, which only apply under serde",
            with_serde="both",
            serde_skip_deserializing_primary_key=True,
            serde_skip_hidden_column=True,
        ),
        _case(
            "codegen-serde-serialize",
            ENUMERATED,
            "serialize only, over a generated enum",
            with_serde="serialize",
        ),
        _case(
            "codegen-model-extras",
            SIMPLE,
            "extra model derives and attributes land on the model",
            model_extra_derives=["Hash", "PartialOrd"],
            model_extra_attributes=["allow(dead_code)"],
        ),
        _case(
            "codegen-enum-extras",
            ENUMERATED,
            "extra enum derives and attributes land on the enum",
            enum_extra_derives=["Hash"],
            enum_extra_attributes=["allow(dead_code)"],
        ),
    ]


def refusal_cases():
    """Generation refusals: the library's own boundary, not the compiler's."""
    variants = (
        (
            "codegen-derive-unbalanced",
            {"model_extra_derives": ["Clone("]},
            "`model_extra_derives` entry",
            "an unbalanced delimiter does not lex as token text",
        ),
        (
            "codegen-attribute-unterminated",
            {"model_extra_attributes": ['derive("']},
            "`model_extra_attributes` entry",
            "an unterminated string literal does not lex",
        ),
        (
            "codegen-enum-derive-unbalanced",
            {"enum_extra_derives": ["Hash("]},
            "`enum_extra_derives` entry",
            "the enum derive list is validated on the same terms",
        ),
    )
    cases = [
        CompileCase(
            id=identity,
            obligation=OBLIGATION,
            verdict="refuse",
            phase="typeck",
            kind="codegen",
            request={"id": identity, "sql": SIMPLE, **options},
            expects=rejects(message=fragment),
            note=note,
        )
        for identity, options, fragment, note in variants
    ]
    cases.append(
        CompileCase(
            id="codegen-unsupported-ddl",
            obligation=OBLIGATION,
            verdict="refuse",
            phase="typeck",
            kind="codegen",
            request={
                "id": "codegen-unsupported-ddl",
                "sql": "CREATE VIEW open_tasks AS SELECT 1;",
            },
            expects=rejects(message="unsupported DDL"),
            note="a statement the bridge does not carry is named, not skipped",
        )
    )
    return cases


# [spec:pgorm:req:generative.compile-suite]
def cases():
    return schema_cases() + option_cases() + refusal_cases()


__all__ = ["OBLIGATION", "cases"]
