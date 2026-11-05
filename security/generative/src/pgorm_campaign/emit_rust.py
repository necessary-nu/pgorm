"""Emit standalone Rust source reproducing a portable program.

The emitted ``main.rs`` links the `pgorm-generative-replay` harness and `pgorm`
and nothing else, reaches the same named builder APIs the campaign interpreter
reaches, and prints one executor-shaped report. Identifiers and payloads travel
as Rust data through a single string-literal escaper; a builder operation is
never replaced by captured SQL.

The Python binding is itself a thin wrapper over ``pgorm_query`` and
``pgorm::pipeline``, so each instruction lowers to the Rust calls
``pgorm-python`` makes for the same instruction.
"""

from . import baseline, catalog, parameters
from .emit_rust_effects import EffectEmitter
from .emit_rust_expr import ExprEmitter
from .emit_rust_models import ModelEmitter
from .emit_rust_pipeline import PipelineEmitter
from .emit_rust_values import (
    PL,
    PRELUDE,
    REPLAY,
    Q,
    UnsupportedInstruction,
    ValueEmitter,
    literal,
)
from .program import CALLBACK_INPUTS, Program

HEADER = """//! Standalone reproducer for generated program {digest}.
//!
//! The declared baseline is applied on startup, so this expects a database
//! prepared the way the campaign prepares one. The URL arrives in
//! `PGORM_REPLAY_URL`, else `DATABASE_URL`, else `argv[1]`; credentials are
//! never compiled in. One JSON report in the executor shape is printed on
//! stdout.

use {replay}::{{Error, Harness, Report}};

const PROGRAM_SHA256: &str = {digest_literal};
const FIXTURE_SQL: &str = {fixture};"""

URL = """fn url() -> Result<String, Error> {{
    match std::env::var("PGORM_REPLAY_URL") {{
        Ok(url) if !url.is_empty() => Ok(url),
        _ => Harness::url_from_environment(),
    }}
}}"""

HELPERS = {
    "params": """fn holders(values: &{q}::Values) -> Vec<pgorm::ValueHolder> {{
    values.0.iter().cloned().map(pgorm::ValueHolder).collect()
}}

fn params(values: &[pgorm::ValueHolder]) -> Vec<&(dyn pgorm::types::ToSql + Sync)> {{
    values
        .iter()
        .map(|value| value as &(dyn pgorm::types::ToSql + Sync))
        .collect()
}}""",
    "tagged": """fn tagged(values: &{q}::Values) -> Vec<{replay}::wire::Tagged> {{
    values
        .0
        .iter()
        .cloned()
        .map({replay}::wire::Tagged::from_value)
        .collect()
}}""",
    "identifier": """fn identifier(value: &{q}::Value) -> Result<{q}::Name, Error> {{
    match value {{
        {q}::Value::String(Some(name))
            if !name.is_empty() && !name.contains('\\u{{0}}') && name.len() <= 63 =>
        {{
            Ok({q}::Name::runtime(name.as_str()))
        }}
        // The binding reads an identifier from the Python scalar, which is a
        // string or nothing at all.
        _ => Err(Error::Format({replay}::FormatError::new(
            "identifier parts require 1-63 UTF-8 bytes without NUL",
        ))),
    }}
}}""",
    "decimal": """fn decimal(units: i128, scale: u32) -> Result<{prelude}::Decimal, Error> {{
    {prelude}::Decimal::try_from_i128_with_scale(units, scale).map_err(|error| {{
        Error::Format({replay}::FormatError::new(error.to_string()))
    }})
}}""",
    "parsed": """fn parsed<T: std::str::FromStr>(text: &str, what: &str) -> Result<T, Error> {{
    text.parse()
        .map_err(|_| Error::Format({replay}::FormatError::new(format!("invalid {{what}} payload"))))
}}""",
    "compiled": """fn compiled(
    result: Result<(String, {q}::Values), {pl}::PipelineError>,
) -> Result<(String, {q}::Values), Error> {{
    result.map_err(|error| Error::Format({replay}::FormatError::new(error.to_string())))
}}""",
    "arity": """fn arity(result: Result<(), {q}::error::Error>) -> Result<(), Error> {{
    result.map_err(|error| Error::Format({replay}::FormatError::new(error.to_string())))
}}""",
    "decoded": """/// A model a later instruction reads back out of a step's retained rows.
///
/// The binding raises on a reference to a row a terminal never produced rather
/// than reading past the end of the result, so this reports instead of panicking.
fn decoded<M: Clone>(rows: &[M], row: usize) -> Result<M, Error> {{
    rows.get(row).cloned().ok_or_else(|| {{
        Error::Format({replay}::FormatError::new(
            "result reference requires a decoded model",
        ))
    }})
}}""",
    "present": """/// An optional graph slot a later instruction requires to have matched.
fn present<M>(value: Option<M>) -> Result<M, Error> {{
    value.ok_or_else(|| {{
        Error::Format({replay}::FormatError::new(
            "result reference requires a decoded model",
        ))
    }})
}}""",
}

MAIN = """#[tokio::main]
async fn main() -> std::process::ExitCode {{
    let mut report = Report::new(PROGRAM_SHA256);
    let outcome = match connected().await {{
        Ok(harness) => run(&harness, &mut report).await,
        Err(error) => Err(error),
    }};
    let failed = match outcome {{
        Ok(()) => false,
        Err(error) => {{
            eprintln!("{{error}}");
            true
        }}
    }};
    report.emit();
    if failed {{
        std::process::ExitCode::FAILURE
    }} else {{
        std::process::ExitCode::SUCCESS
    }}
}}

async fn connected() -> Result<Harness, Error> {{
    let harness = Harness::connect(&url()?).await?;
    harness.apply(FIXTURE_SQL).await?;
    Ok(harness)
}}"""


# [spec:pgorm:req:generative.replay]
class Emitter(ValueEmitter, ExprEmitter, PipelineEmitter, ModelEmitter, EffectEmitter):
    """Render one validated program as Rust source, node by node."""

    def __init__(self, data, digest):
        self.data = data
        self.digest = digest
        self.nodes = {node["id"]: node for node in data["nodes"]}
        self.order = [node["id"] for node in data["nodes"]]
        self.owned = {item["id"]: [] for item in data["binders"]}
        for identity in self.order:
            scope = self.nodes[identity]["scope"]
            if scope != "root":
                self.owned[scope].append(identity)
        self.types = {}
        for node in data["nodes"]:
            operation = catalog.OPERATIONS[node["op"]]
            self.types[node["id"]] = (
                self.types[node["inputs"]["query"]]
                if operation.output == "same"
                else operation.output
            )
        self.scheduled = {step["id"]: step for step in data["steps"]}
        self.consumers = {}
        for owner in list(data["nodes"]) + list(data["steps"]):
            for reference in parameters.input_ids(owner["inputs"]):
                self.consumers[reference] = self.consumers.get(reference, 0) + 1
        self.results = {
            node["data"]["step"]
            for node in data["nodes"]
            if node["op"] in ("result.value", "entity.result")
        }
        self.helpers = set()
        self.rendered = {}
        self.consumed = {}
        self.current = set()

    # -- naming -------------------------------------------------------------

    def var(self, reference):
        return "n_" + reference.replace("-", "_")

    def connection(self, scope):
        return "connection" if scope == "root" else "tx_" + scope

    def use(self, reference):
        """A node's value at a use site: pipeline shapes inline, others clone."""
        node = self.nodes[reference]
        kind = self.types[reference]
        self.current.add(reference)
        if kind in ("pexpr", "source", "graph") and node["scope"] == "root":
            return self.inline(reference)
        if kind == "cursor" and self.consumers.get(reference, 0) > 1:
            # `Cursor<GraphRow<E, S>, K>` derives Clone, which puts the bound on
            # `GraphRow` — a marker that carries no data and does not implement
            # it. A cursor is therefore moved along its chain, never shared.
            raise UnsupportedInstruction(
                "a graph cursor is not Clone, so it cannot feed two instructions"
            )
        if kind in ("grouped", "cursor"):
            return self.var(reference)
        return self.var(reference) + ".clone()"

    def inline(self, reference):
        """Render a brand-polymorphic node fresh at each site.

        A `pgorm::pipeline::Expr` is branded by the stage that consumes it, so a
        root-scope pipeline expression cannot be a `let` binding shared by two
        stages; it is pure construction, so re-rendering is the same value.
        """
        if reference not in self.rendered:
            outer, self.current = self.current, set()
            source = self.expression(self.nodes[reference], None)
            self.rendered[reference] = (source, self.current)
            self.current = outer
        source, inner = self.rendered[reference]
        self.current |= inner
        return source

    def uses(self, references):
        return [self.use(reference) for reference in references]

    # -- assembly -----------------------------------------------------------

    def reachable(self):
        """Root-scope nodes the effects actually need, in program order."""
        pending, seen = [], set()
        for step in self.data["steps"]:
            pending.extend(parameters.input_ids(step["inputs"]))
        while pending:
            reference = pending.pop()
            if reference in seen:
                continue
            seen.add(reference)
            node = self.nodes[reference]
            if node["op"] in CALLBACK_INPUTS and "binder" in node["data"]:
                # A callback's private nodes stay inside it, but whatever they
                # read from the root scope still has to exist by then.
                for identity in self.owned[node["data"]["binder"]]:
                    pending.extend(
                        item
                        for item in parameters.input_ids(self.nodes[identity]["inputs"])
                        if self.nodes[item]["scope"] == "root"
                    )
            pending.extend(
                item
                for item in parameters.input_ids(node["inputs"])
                if self.nodes[item]["scope"] == "root"
            )
        return [identity for identity in self.order if identity in seen]

    def declarations(self):
        """Render every candidate declaration, recording what each consumed."""
        blocks = {}
        for identity in self.reachable():
            node = self.nodes[identity]
            self.current = set()
            if node["op"] in CALLBACK_INPUTS:
                lines = self.stage(node)
            elif self.types[identity] in ("pexpr", "source", "graph"):
                # Inlined at each use site: a brand cannot be shared by a `let`,
                # and a graph registration is a factory path rather than a value.
                self.inline(identity)
                lines = []
            else:
                lines = [
                    f"    let {self.var(identity)} = {self.expression(node, None)};"
                ]
                if node["op"] == "raw.template":
                    self.helpers.add("arity")
                    variable = self.var(identity)
                    lines.append(
                        f"    arity({Q}::inject_parameters(&{variable}.0, "
                        f"{variable}.1.0.iter().cloned()).map(|_| ()))?;"
                    )
            self.consumed[identity] = self.current
            blocks[identity] = lines
        return blocks

    def live(self, blocks, needed):
        """Declarations an emitted use actually reaches, so nothing sits idle."""
        pending, seen = list(needed), set()
        while pending:
            reference = pending.pop()
            if reference in seen or reference not in blocks:
                continue
            seen.add(reference)
            pending.extend(self.consumed.get(reference, ()))
        return [identity for identity in blocks if identity in seen]

    def deferred_blocks(self, blocks):
        """Declarations that read a step's retained rows, directly or through one.

        These cannot stand at the top of `run`: the `r_*` binding they name is
        introduced by the step that produced it, so they are emitted at the
        first step that uses them instead.
        """
        reading = {
            identity
            for identity in blocks
            if self.nodes[identity]["op"] in ("result.value", "entity.result")
        }
        growing = True
        while growing:
            growing = False
            for identity in blocks:
                if identity in reading:
                    continue
                if reading & set(self.consumed.get(identity, ())):
                    reading.add(identity)
                    growing = True
        return reading

    def declare(self, used, lines, indent):
        """Emit the deferred declarations this step has just become able to make."""
        extra = indent[4:]
        for identity in self.live(self.blocks, used):
            if identity in self.emitted or identity not in self.deferred:
                continue
            self.emitted.add(identity)
            for line in "\n".join(self.blocks[identity]).splitlines():
                lines.append(extra + line if line.strip() else line)

    def run(self):
        self.blocks = self.declarations()
        self.deferred = self.deferred_blocks(self.blocks)
        self.emitted = set()
        self.current = set()
        steps = []
        self.steps(list(self.data["steps"]), steps, "    ")
        transactional = any(step["op"] == "begin" for step in self.data["steps"])
        binding = "let mut connection" if transactional else "let connection"
        lines = [
            "async fn run(harness: &Harness, report: &mut Report) -> Result<(), Error> {",
            f"    {binding} = harness.pool().get().await?;",
        ]
        for identity in self.live(self.blocks, self.current):
            if identity not in self.deferred:
                lines.extend(self.blocks[identity])
        lines.extend(steps)
        lines.append("    Ok(())")
        lines.append("}")
        return "\n".join(lines)

    def render(self):
        # The body is rendered first: it decides which helpers travel with it.
        body = self.run()
        fixture = literal(baseline.render(self.data["fixture"]))
        parts = [
            HEADER.format(
                digest=self.digest,
                digest_literal=literal(self.digest),
                fixture=fixture,
                replay=REPLAY,
            ),
            URL.format(),
        ]
        for name in sorted(self.helpers):
            parts.append(
                HELPERS[name].format(q=Q, pl=PL, replay=REPLAY, prelude=PRELUDE)
            )
        parts.extend([body, MAIN.format()])
        return "\n\n".join(parts) + "\n"


# [spec:pgorm:req:generative.replay]
def render(program):
    """Emit a standalone Rust main.rs reproducing a validated Program."""
    if not isinstance(program, Program):
        program = (
            Program(program)
            if isinstance(program, (str, bytes))
            else Program.from_dict(program)
        )
    return Emitter(program.data(), program.digest).render()


__all__ = ["Emitter", "UnsupportedInstruction", "literal", "render"]
