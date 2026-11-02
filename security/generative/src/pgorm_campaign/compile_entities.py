"""Fresh entity derives and the type boundaries they mint.

An installed binding registers its entities once; every runtime program then
draws from that fixed set. The shapes a derive *could* produce — a composite
key, a schema-qualified enum, a column whose Rust name is not its SQL name —
are settled at compile time and are invisible to any number of runtime
programs. They are varied here instead.
"""

from .compile_case import CompileCase, rejects

OBLIGATION = "new-rust-entities"

PRELUDE = "    use pgorm::entity::prelude::*;"

# Column shapes worth minting a derive for: each one lands in a different arm
# of the derive's type mapping, and a `set()` against it is a different
# `TryFrom`.
COLUMNS = (
    ("small", "i16", ""),
    ("count", "i32", ""),
    ("big", "i64", ""),
    ("flag", "bool", ""),
    ("ratio", "f64", ""),
    ("label", "String", ""),
    ("note", "Option<String>", ""),
    ("blob", "Vec<u8>", ""),
    ("maybe", "Option<i32>", ""),
)


def _field(name, rust, attrs):
    lines = []
    if attrs:
        lines.append("        #[pgorm(" + attrs + ")]")
    lines.append(f"        pub {name}: {rust},")
    return lines


def entity_module(
    *, table, columns, schema=None, keys=(("id", "i32"),), relation="", hooks=False
):
    """Render one `Entity` entity as module body text.

    The relation enum is always emitted, empty when the entity has no edges:
    the derive requires it, and an entity generated without one would fail for
    a reason that says nothing about the shape under test. The derives write
    the rest: the behaviour, unless `hooks` says the case writes its own, and
    a `Related` impl for each entity one relation reaches.
    """
    header = f'table_name = "{table}"'
    if schema:
        header = f'schema_name = "{schema}", ' + header
    if hooks:
        header += ", hooks"
    lines = [
        PRELUDE,
        "",
        # `Eq` is left off deliberately: the column set varies across `f64`,
        # and a derive that only compiles for some of the shapes under test
        # would fail for a reason that says nothing about the entity.
        "    #[derive(Clone, Debug, PartialEq, Entity)]",
        f"    #[pgorm({header})]",
        "    pub struct Model {",
    ]
    composite = len(keys) > 1
    for name, rust in keys:
        key = "primary_key, auto_increment = false" if composite else "primary_key"
        lines.extend(_field(name, rust, key))
    for name, rust, attrs in columns:
        lines.extend(_field(name, rust, attrs))
    lines.append("    }")
    lines.append("")
    lines.append("    #[derive(Copy, Clone, Debug, Relation)]")
    lines.append("    pub enum Relation {" + ("" if relation else "}"))
    if relation:
        lines.append(relation)
        lines.append("    }")
    return "\n".join(lines)


def _nested(name, body):
    """Wrap an entity as a child module so one case can hold several."""
    return f"    pub mod {name} {{\n" + _shift(body) + "\n    }"


def _shift(text):
    return "\n".join(("    " + line) if line else line for line in text.splitlines())


def _accept(identity, body, note):
    return CompileCase(
        id=identity,
        obligation=OBLIGATION,
        verdict="accept",
        phase="typeck",
        source=body,
        note=note,
    )


def column_cases():
    """One derive per column shape, then one carrying all of them at once."""
    cases = []
    for index, column in enumerate(COLUMNS):
        body = entity_module(table=f"col_{column[0]}", columns=(column,))
        cases.append(
            _accept(
                f"entity-column-{column[0]}",
                body,
                f"column {index} maps {column[1]} through the derive",
            )
        )
    cases.append(
        _accept(
            "entity-columns-all",
            entity_module(table="col_all", columns=COLUMNS),
            "every column shape in one derive",
        )
    )
    return cases


def naming_cases():
    """Table, schema and per-column renaming, which only a derive can vary."""
    renamed = (
        ("value", "String", 'column_name = "sql_value"'),
        ("kind", "i32", "variant = Kind"),
        ("r#type", "i32", ""),
    )
    return [
        _accept(
            "entity-schema-qualified",
            entity_module(table="qualified", schema="fixture", columns=COLUMNS[:2]),
            "schema_name rides the derive into the entity's identity",
        ),
        _accept(
            "entity-renamed-columns",
            entity_module(table="renamed", columns=renamed),
            "column_name, variant and a raw identifier together",
        ),
    ]


def key_cases():
    """Key shapes change `PrimaryKey::ValueType`, and so change every lookup."""
    composite = entity_module(
        table="composite",
        columns=(("payload", "String", ""),),
        keys=(("left", "i32"), ("right", "i64")),
    )
    text_key = entity_module(
        table="text_key",
        columns=(("payload", "i32", ""),),
        keys=(("code", "String"),),
    )
    return [
        _accept("entity-key-composite", composite, "a two-column primary key"),
        _accept("entity-key-text", text_key, "a non-integer primary key"),
        _accept(
            "entity-key-lookup",
            composite + "\n\n    pub fn lookup() -> Select<Entity> {\n"
            "        Entity::find_by_id((1_i32, 2_i64))\n    }",
            "the composite ValueType is a tuple at the call site",
        ),
    ]


def _temporal(table):
    """A room keyed `(id, valid_at WITHOUT OVERLAPS)`, its period last."""
    return "\n".join(
        [
            PRELUDE,
            "",
            "    #[derive(Clone, Debug, PartialEq, Eq, Entity)]",
            f'    #[pgorm(table_name = "{table}")]',
            "    pub struct Model {",
            "        #[pgorm(primary_key, auto_increment = false)]",
            "        pub id: i32,",
            "        #[pgorm(primary_key, without_overlaps)]",
            "        pub valid_at: Range<Date>,",
            "        pub rate: i32,",
            "    }",
            "",
            "    #[derive(Copy, Clone, Debug, Relation)]",
            "    pub enum Relation {}",
        ]
    )


def temporal_key_cases():
    """A key `WITHOUT OVERLAPS`: its period is the key's last part, so the
    `ValueType` a lookup takes still ends with a range."""
    room = _temporal("temporal_room")
    # One item per generated line, as `_temporal` writes its module.
    alone = "\n".join(  # noqa: FLY002
        [
            PRELUDE,
            "",
            "    #[derive(Clone, Debug, PartialEq, Eq, Entity)]",
            '    #[pgorm(table_name = "temporal_alone")]',
            "    pub struct Model {",
            "        #[pgorm(primary_key, without_overlaps)]",
            "        pub valid_at: Range<Date>,",
            "        pub rate: i32,",
            "    }",
            "",
            "    #[derive(Copy, Clone, Debug, Relation)]",
            "    pub enum Relation {}",
        ]
    )
    return [
        _accept("entity-key-temporal", room, "a period ends the key, WITHOUT OVERLAPS"),
        _accept(
            "entity-key-temporal-lookup",
            room + "\n\n    pub fn version(valid_at: Range<Date>) -> Select<Entity> {\n"
            "        Entity::find_by_id((1_i32, valid_at))\n    }",
            "a lookup by key names one version: the period is part of its value",
        ),
        CompileCase(
            id="entity-key-temporal-alone",
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=alone,
            expects=rejects(
                message="a WITHOUT OVERLAPS key needs a column besides its period"
            ),
            note="the derive refuses the key PostgreSQL would: a period and nothing else",
        ),
    ]


def key_check_cases():
    """A relation's NOT ENFORCED, deferral and PERIOD, which the derive reads
    into the relation and schema generation writes on the foreign key."""
    room = _temporal("checked_room")
    stay = entity_module(
        table="checked_stay",
        columns=(
            ("room_id", "i32", ""),
            ("guest_id", "i32", ""),
            ("during", "Range<Date>", ""),
        ),
        relation=(
            "        #[pgorm(belongs_to = super::room::Entity, from = Column::RoomId, "
            "to = super::room::Column::Id, from_period = Column::During, "
            "to_period = super::room::Column::ValidAt, "
            "deferrability = Deferrability::DeferrableInitiallyDeferred)]\n"
            "        Room,\n"
            "        #[pgorm(belongs_to = super::room::Entity, from = Column::GuestId, "
            "to = super::room::Column::Id, enforcement = Enforcement::NotEnforced)]\n"
            "        Unchecked,"
        ),
    )
    accepted = "\n\n".join(
        [
            _nested("room", room),
            _nested(
                "stay",
                stay + "\n\n    pub fn statement()"
                " -> Result<pgorm::pgorm_query::TableCreateStatement, pgorm::Error> {\n"
                "        pgorm::Schema::new().create_table_from_entity(Entity)\n    }",
            ),
        ]
    )
    unknown = entity_module(
        table="checked_unknown",
        columns=(("parent_id", "i32", ""),),
        relation=(
            "        #[pgorm(belongs_to = Entity, from = Column::ParentId, "
            "to = Column::Id, enforcement = Enforcement::Lax)]\n"
            "        Parent,"
        ),
    )
    quoted = entity_module(
        table="checked_quoted",
        columns=(("parent_id", "i32", ""),),
        relation=(
            "        #[pgorm(belongs_to = Entity, from = Column::ParentId, "
            'to = Column::Id, enforcement = "NotEnforced")]\n'
            "        Parent,"
        ),
    )
    half = entity_module(
        table="checked_half",
        columns=(("parent_id", "i32", ""), ("during", "Range<Date>", "")),
        relation=(
            "        #[pgorm(belongs_to = Entity, from = Column::ParentId, "
            "to = Column::Id, from_period = Column::During)]\n"
            "        Parent,"
        ),
    )
    return [
        _accept(
            "entity-relation-key-checks",
            accepted,
            "a deferred PERIOD foreign key and one the server never enforces",
        ),
        CompileCase(
            id="entity-relation-enforcement-unknown",
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=unknown,
            expects=rejects("E0599"),
            note="enforcement is one of the two states PostgreSQL has, a variant of its enum",
        ),
        CompileCase(
            id="entity-relation-key-quoted",
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=quoted,
            expects=rejects(
                message="`enforcement` is Rust code, written bare: remove the quotes"
            ),
            note="an attribute value that is Rust code is written bare, never in a string",
        ),
        CompileCase(
            id="entity-relation-period-half",
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=half,
            expects=rejects(message="Missing attribute 'to_period'"),
            note="a PERIOD pairs a period on each side",
        ),
    ]


def enum_cases():
    """`ActiveEnum`, including the schema-qualified identity: an enum of names
    is a PostgreSQL enum of its variants' names in snake_case, the type named
    for the enum, unless it says otherwise."""
    body = (
        PRELUDE + "\n\n" + "    #[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]\n"
        '    #[pgorm(schema_name = "fixture")]\n'
        "    pub enum State {\n"
        "        Open,\n"
        '        #[pgorm(string_value = "shut")]\n'
        "        Closed,\n"
        "    }\n\n"
        "    #[derive(Clone, Debug, PartialEq, Entity)]\n"
        '    #[pgorm(table_name = "with_enum")]\n'
        "    pub struct Model {\n"
        "        #[pgorm(primary_key)]\n"
        "        pub id: i32,\n"
        "        pub state: State,\n"
        "    }\n\n"
        "    #[derive(Copy, Clone, Debug, Relation)]\n"
        "    pub enum Relation {}"
    )
    lone = (
        PRELUDE + "\n\n"
        "    #[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]\n"
        "    #[pgorm(rs_type = String)]\n"
        "    pub enum State {\n"
        "        Open,\n"
        "    }"
    )
    return [
        _accept(
            "entity-enum-qualified",
            body,
            "a schema-qualified active enum used as a column type",
        ),
        CompileCase(
            id="entity-enum-lone-rs-type",
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=lone,
            expects=rejects(
                message="a PostgreSQL enum of strings names neither",
            ),
            note="an enum stored as `rs_type` names its column type too",
        ),
    ]


def behaviour_cases():
    """The behaviour the `Entity` derive writes, and the hooks a case writes
    under `hooks` instead."""
    hooked = entity_module(
        table="hooked",
        columns=(("label", "String", ""),),
        hooks=True,
    )
    plain = entity_module(table="plain", columns=(("label", "String", ""),))
    return [
        _accept(
            "entity-hooks",
            hooked + "\n\n    impl ActiveModelBehavior for ActiveModel {\n"
            "        fn new() -> Self {\n"
            "            Self {\n"
            '                label: pgorm::set("untitled"),\n'
            "                ..ActiveModelTrait::default()\n"
            "            }\n        }\n    }",
            "under `hooks` the entity's module implements the behaviour",
        ),
        CompileCase(
            id="entity-behaviour-twice",
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=plain + "\n\n    impl ActiveModelBehavior for ActiveModel {}",
            expects=rejects("E0119"),
            note="without `hooks` the derive implements the behaviour, so a second is a conflict",
        ),
    ]


def relation_cases():
    """A derived relation, and the `Related` impl the derive writes for it,
    which a lookup of the related entity walks."""
    child = entity_module(
        table="rel_child",
        columns=(("parent_id", "i32", ""),),
        relation=(
            "        #[pgorm(belongs_to = super::parent::Entity, "
            "from = Column::ParentId, to = super::parent::Column::Id)]\n"
            "        Parent,"
        ),
    )
    body = "\n\n".join(
        [
            _nested("parent", entity_module(table="rel_parent", columns=())),
            _nested(
                "child",
                child
                + "\n\n    pub fn parent_of(child: &Model) -> Select<super::parent::Entity> {\n"
                "        child.find_related(super::parent::Entity)\n    }",
            ),
        ]
    )
    tenant_child = entity_module(
        table="rel_tenant_child",
        columns=(("parent_id", "i32", ""),),
        keys=(("tenant_id", "i32"), ("id", "i32")),
        relation=(
            "        #[pgorm(belongs_to = super::parent::Entity, "
            "from = (Column::TenantId, Column::ParentId), "
            "to = (super::parent::Column::TenantId, super::parent::Column::Id))]\n"
            "        Parent,"
        ),
    )
    composite = "\n\n".join(
        [
            _nested(
                "parent",
                entity_module(
                    table="rel_tenant_parent",
                    columns=(),
                    keys=(("tenant_id", "i32"), ("id", "i32")),
                ),
            ),
            _nested(
                "child",
                tenant_child
                + "\n\n    pub fn parent_of(child: &Model) -> Select<super::parent::Entity> {\n"
                "        child.find_related(super::parent::Entity)\n    }",
            ),
        ]
    )
    return [
        _accept(
            "entity-relation-pair",
            body,
            "belongs_to, and the Related impl the derive writes for it",
        ),
        _accept(
            "entity-relation-composite",
            composite,
            "a two-column foreign key: tuple from/to onto a two-column key",
        ),
    ]


def negative_cases():
    """Boundaries a fresh derive erects, each rejected by a named code."""
    entity = entity_module(table="neg_target", columns=(("label", "String", ""),))
    composite = entity_module(
        table="neg_composite",
        columns=(("payload", "i32", ""),),
        keys=(("left", "i32"), ("right", "i64")),
    )
    variants = (
        (
            "entity-active-value-type",
            (
                "    pub fn build() -> ActiveModel {\n"
                "        ActiveModel {\n"
                "            label: pgorm::set(7_i32),\n"
                "            ..Default::default()\n"
                "        }\n    }"
            ),
            entity,
            rejects("E0277"),
            "a column's Rust type refuses a value it has no conversion from",
        ),
        (
            "entity-missing-field",
            "    pub fn read(model: Model) -> i32 {\n        model.absent\n    }",
            entity,
            rejects("E0609"),
            "the derive mints exactly the declared fields and no others",
        ),
        (
            "entity-key-arity",
            (
                "    pub fn lookup() -> Select<Entity> {\n"
                "        Entity::find_by_id(1_i32)\n    }"
            ),
            composite,
            # `find_by_id` is bounded by `Into<ValueType>`, so a key of the
            # wrong shape is an unsatisfied conversion rather than a type
            # mismatch at the argument.
            rejects("E0277"),
            "a composite key's ValueType is a tuple, not its first column",
        ),
        (
            "entity-key-value-type",
            (
                "    pub fn lookup() -> Select<Entity> {\n"
                '        Entity::find_by_id("1")\n    }'
            ),
            entity,
            rejects("E0277"),
            "a key lookup is typed by the declared primary key",
        ),
    )
    return [
        CompileCase(
            id=identity,
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=base + "\n\n" + tail,
            expects=expectation,
            note=note,
        )
        for identity, tail, base, expectation, note in variants
    ]


# [spec:pgorm:req:generative.compile-suite]
def cases():
    return (
        column_cases()
        + naming_cases()
        + key_cases()
        + temporal_key_cases()
        + enum_cases()
        + behaviour_cases()
        + relation_cases()
        + key_check_cases()
        + negative_cases()
    )


__all__ = ["OBLIGATION", "cases", "entity_module"]
