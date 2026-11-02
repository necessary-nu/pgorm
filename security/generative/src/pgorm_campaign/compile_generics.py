"""Graph slot tuples and pipeline source tuples: shapes chosen by instantiation.

A registered graph is one instantiation of `SelectGraph<E, S>`. Runtime
programs pick among the registered ones; they cannot mint an `S` the binding
was not built with, and the arity ceiling — the point at which the generated
impls run out — is reachable only by writing a program that asks for one more
slot than exists. Both sides of that ceiling are generated here.
"""

from .compile_case import CompileCase, rejects
from .compile_entities import entity_module

OBLIGATION = "fresh-graph-generics"

# `grow!` generates the edge methods for slot tuples up to five, so each
# method call adds one and six slots is the ceiling. `source_tuple!` stops at
# six directly. Exceeding either by one is the negative.
GRAPH_CEILING = 6
SOURCE_CEILING = 6

PAIR = "\n\n".join(
    [
        "    pub mod parent {\n"
        + "\n".join(
            "    " + line if line else line
            for line in entity_module(
                table="g_parent", columns=(("label", "String", ""),)
            ).splitlines()
        )
        + "\n    }",
        "    pub mod child {\n"
        + "\n".join(
            "    " + line if line else line
            for line in (
                entity_module(
                    table="g_child",
                    columns=(("parent_id", "i32", ""),),
                    relation=(
                        "        #[pgorm(belongs_to = super::parent::Entity, "
                        "from = Column::ParentId, to = super::parent::Column::Id)]\n"
                        "        Parent,"
                    ),
                )
            ).splitlines()
        )
        + "\n    }",
    ]
)

# The edge each join walks: the child's `belongs_to`, reversed, so the parent
# is the root and every slot is another copy of the child under its own alias.
EDGE = "child::Relation::Parent.def().rev()"


def _join(index, optional):
    method = "join_maybe_as" if optional else "join_one_as"
    return f'            .{method}::<child::Entity>({EDGE}, pgorm::alias("s{index}"))'


def graph_builder(slots):
    """Render a graph built up to `slots` joins, alternating required/optional."""
    lines = ["        let graph = parent::Entity::graph()"]
    lines.extend(_join(index, index % 2 == 1) for index in range(1, slots + 1))
    lines[-1] += ";"
    lines.append("        let _ = graph;")
    return "\n".join(lines)


def _slot_types(slots):
    kinds = ["Opt" if index % 2 == 1 else "Req" for index in range(1, slots + 1)]
    return "(" + ", ".join(f"{kind}<child::Entity>" for kind in kinds) + ",)"


def graph_arity_cases():
    """One accepted program per reachable arity, plus the rejected seventh."""
    cases = []
    for slots in range(1, GRAPH_CEILING + 1):
        body = (
            PAIR
            + "\n\n    use pgorm::entity::prelude::*;\n\n"
            + f"    pub fn build() {{\n{graph_builder(slots)}\n    }}"
        )
        cases.append(
            CompileCase(
                id=f"graph-arity-{slots}",
                obligation=OBLIGATION,
                verdict="accept",
                phase="typeck",
                source=body,
                note=f"a graph instantiated with {slots} slots",
            )
        )
    over = GRAPH_CEILING + 1
    cases.append(
        CompileCase(
            id=f"graph-arity-{over}",
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=(
                PAIR
                + "\n\n    use pgorm::entity::prelude::*;\n\n"
                + f"    pub fn build() {{\n{graph_builder(over)}\n    }}"
            ),
            expects=rejects("E0599"),
            note="past the ceiling the receiver has no edge method at all",
        )
    )
    return cases


def graph_decode_cases():
    """Name the decoded row type, which is where the slot tuple becomes data."""
    cases = []
    for slots in (1, GRAPH_CEILING):
        body = (
            PAIR
            + "\n\n    use pgorm::entity::prelude::*;\n"
            + "    use pgorm::{GraphItem, Opt, Req};\n\n"
            + "    pub fn shape(rows: Vec<GraphItem<parent::Entity, "
            + _slot_types(slots)
            + ">>) -> usize {\n        rows.len()\n    }"
        )
        cases.append(
            CompileCase(
                id=f"graph-decode-{slots}",
                obligation=OBLIGATION,
                verdict="accept",
                phase="typeck",
                source=body,
                note=f"the {slots}-slot row type is nameable and inhabited",
            )
        )
    over = GRAPH_CEILING + 1
    cases.append(
        CompileCase(
            id=f"graph-decode-{over}",
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=(
                PAIR
                + "\n\n    use pgorm::entity::prelude::*;\n"
                + "    use pgorm::{GraphItem, Opt, Req};\n\n"
                + "    pub fn shape(rows: Vec<GraphItem<parent::Entity, "
                + _slot_types(over)
                + ">>) -> usize {\n        rows.len()\n    }"
            ),
            expects=rejects("E0277"),
            note="the row type of an over-wide graph has no decode impl",
        )
    )
    return cases


def _sources(count):
    names = ["a::Entity" if index % 2 == 0 else "b::Entity" for index in range(count)]
    return "(" + ", ".join(names) + ",)"


SOURCE_PAIR = "\n\n".join(
    [
        "    pub mod a {\n"
        + "\n".join(
            "    " + line if line else line
            for line in entity_module(
                table="src_a", columns=(("label", "String", ""),)
            ).splitlines()
        )
        + "\n    }",
        "    pub mod b {\n"
        + "\n".join(
            "    " + line if line else line
            for line in entity_module(
                table="src_b", columns=(("weight", "i64", ""),)
            ).splitlines()
        )
        + "\n    }",
    ]
)


def source_arity_cases():
    """`select_sources` over every generated tuple width, and one past it."""
    cases = []
    for count in range(2, SOURCE_CEILING + 1):
        body = (
            SOURCE_PAIR
            + "\n\n    use pgorm::pipeline::Pipeline;\n\n"
            + "    pub fn build() {\n"
            + f"        let _ = Pipeline::from(a::Entity).select_sources({_sources(count)});\n"
            + "    }"
        )
        cases.append(
            CompileCase(
                id=f"source-arity-{count}",
                obligation=OBLIGATION,
                verdict="accept",
                phase="typeck",
                source=body,
                note=f"a {count}-source projection",
            )
        )
    over = SOURCE_CEILING + 1
    cases.append(
        CompileCase(
            id=f"source-arity-{over}",
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=(
                SOURCE_PAIR
                + "\n\n    use pgorm::pipeline::Pipeline;\n\n"
                + "    pub fn build() {\n"
                + f"        let _ = Pipeline::from(a::Entity).select_sources({_sources(over)});\n"
                + "    }"
            ),
            expects=rejects("E0277"),
            note="a seventh source has no SourceList impl",
        )
    )
    return cases


def source_shape_cases():
    """A source list refuses members that are not sources, and mis-shaped rows."""
    return [
        CompileCase(
            id="source-not-a-source",
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=(
                SOURCE_PAIR
                + "\n\n    use pgorm::pipeline::Pipeline;\n\n"
                + "    pub fn build() {\n"
                + "        let _ = Pipeline::from(a::Entity)\n"
                + "            .select_sources((a::Entity, a::Column::Id));\n"
                + "    }"
            ),
            expects=rejects("E0277"),
            note="a column is not a selectable source",
        ),
        CompileCase(
            id="source-row-shape",
            obligation=OBLIGATION,
            verdict="reject",
            phase="typeck",
            source=(
                SOURCE_PAIR
                + "\n\n    use pgorm::pipeline::{Pipeline, SourceList};\n\n"
                + "    pub type Row = <(a::Entity, b::Entity) as SourceList>::Row;\n\n"
                + "    pub fn shape(row: Row) -> Option<a::Model> {\n"
                + "        row.1\n    }"
            ),
            expects=rejects("E0308"),
            note="the decoded row is typed slot by slot, in declaration order",
        ),
    ]


# [spec:pgorm:req:generative.compile-suite]
def cases():
    return (
        graph_arity_cases()
        + graph_decode_cases()
        + source_arity_cases()
        + source_shape_cases()
    )


__all__ = ["GRAPH_CEILING", "OBLIGATION", "SOURCE_CEILING", "cases", "graph_builder"]
