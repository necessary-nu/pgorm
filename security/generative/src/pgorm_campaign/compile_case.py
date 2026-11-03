"""The unit the compile suite generates: one bounded Rust program and its verdict.

Compile coverage answers a different question from runtime coverage, so it is
modelled separately all the way down. A runtime program is judged by what
PostgreSQL returned; a compile case is judged by what rustc *refused*, and a
refusal only counts when the compiler names the rejection the case predicted.
"""

import hashlib
from dataclasses import dataclass, field

# rustc reaches these in order, and a body that fails at one never reaches the
# next. The suppression is per body, not per crate: a measured run of every
# source rejection in a single crate still reported all sixteen, so typeck and
# borrowck cases demonstrably coexist. Resolution is the one that bites — an
# unresolved import swallows everything the module would otherwise have said —
# and grouping by phase is the cheap general form of that, at one extra build.
# Coherence bites per trait rather than per body: once two impls of a trait
# conflict (E0119), rustc checks no impl of that trait against its supertraits,
# anywhere in the crate, so a conflict would hide a sibling case's E0271.
PHASES = ("resolve", "coherence", "typeck", "borrowck")

# What a case claims, and therefore what a run can conclude.
#
#   accept  — the program is legal; anything rustc says about it is a defect.
#   reject  — the program is illegal in one named way.
#   refuse  — generation itself must fail, before any compiler sees source.
#
# `refuse` is not a weaker `reject`. A generator that errors is pgorm-codegen
# holding its own boundary; a compiler that errors is rustc holding the next
# one. A suite that scored them alike would read a broken generator as a
# well-defended type system.
VERDICTS = ("accept", "reject", "refuse")

KINDS = ("source", "codegen")


class CompileCaseError(ValueError):
    """A case was described in a way the suite cannot run or judge."""


# [spec:pgorm:req:generative.compile-suite]
@dataclass(frozen=True)
class Expectation:
    """How a rejection is recognised, and by what.

    `codes` is the ordinary form and the one to reach for: an `E0499` is a
    claim about the language, and it survives rustc rewording its diagnostics.
    `message` exists because a handful of region errors carry no code at all —
    the brand-mixing rejections among them — and a suite that could not express
    them would silently drop the surfaces it was written to cover. It matches a
    substring of the compiler's own message, never a rendered layout, so it
    does not re-bless every time a diagnostic gains a note.
    """

    codes: tuple[str, ...] = ()
    message: str = ""

    def __post_init__(self):
        if not self.codes and not self.message:
            raise CompileCaseError("a rejection must name a code or a message")
        for code in self.codes:
            if not (code.startswith("E0") and code[1:].isdigit()):
                raise CompileCaseError("not a rustc error code: " + code)

    @property
    def coded(self):
        return bool(self.codes)

    def matches(self, code, message):
        if code and code in self.codes:
            return True
        return bool(self.message) and self.message in message

    def data(self):
        return {"codes": list(self.codes), "message": self.message}


# [spec:pgorm:req:generative.compile-suite]
@dataclass(frozen=True)
class CompileCase:
    """One generated program, its predicted verdict, and what it discharges.

    `obligation` names the `outside_runtime` entry the case exists to cover.
    Nothing else ties compile evidence back to the matrix, and a case that
    covered nothing would inflate a count without moving coverage.
    """

    id: str
    obligation: str
    verdict: str
    phase: str
    kind: str = "source"
    # Rust module text for a `source` case; unused by `codegen` cases, which
    # carry a request for the generator instead.
    source: str = ""
    # The generator request for a `codegen` case: DDL plus writer options.
    request: dict = field(default_factory=dict)
    expects: Expectation | None = None
    # Extra crate dependencies this case's source needs, as Cargo spellings.
    needs: tuple[str, ...] = ()
    note: str = ""

    def __post_init__(self):
        if self.verdict not in VERDICTS:
            raise CompileCaseError("unknown verdict: " + self.verdict)
        if self.phase not in PHASES:
            raise CompileCaseError("unknown phase: " + self.phase)
        if self.kind not in KINDS:
            raise CompileCaseError("unknown case kind: " + self.kind)
        if not self.id or not self.obligation:
            raise CompileCaseError("a case needs an id and an obligation")
        if self.verdict in ("reject", "refuse") and self.expects is None:
            raise CompileCaseError("a refused case must say how it is refused")
        if self.verdict == "accept" and self.expects is not None:
            raise CompileCaseError("an accepted case predicts no diagnostic")
        if self.kind == "source" and not self.source.strip():
            raise CompileCaseError("a source case needs source: " + self.id)
        if self.kind == "codegen" and not self.request:
            raise CompileCaseError("a codegen case needs a request: " + self.id)
        if self.verdict == "refuse" and self.kind != "codegen":
            raise CompileCaseError("only generation can be refused: " + self.id)

    @property
    def digest(self):
        """Content identity, so a case names the crate that was built for it.

        cargo keys freshness on the package name. Reproducers already learned
        this the expensive way: share a target directory between two crates of
        the same name and the second inherits the first one's artifacts, then
        reports a stale success. A stale success is the one answer a compile
        check must never give, so the name carries the content.
        """
        body = "\0".join(
            [
                self.id,
                self.obligation,
                self.verdict,
                self.phase,
                self.kind,
                self.source,
                repr(sorted(self.request.items())),
            ]
        )
        return hashlib.sha256(body.encode()).hexdigest()

    def data(self):
        value = {
            "id": self.id,
            "obligation": self.obligation,
            "verdict": self.verdict,
            "phase": self.phase,
            "kind": self.kind,
            "digest": self.digest,
        }
        if self.expects is not None:
            value["expects"] = self.expects.data()
        if self.needs:
            value["needs"] = list(self.needs)
        if self.note:
            value["note"] = self.note
        return value


def rejects(*codes, message=""):
    """An expectation, spelled at the call site the way a case reads."""
    return Expectation(codes=tuple(codes), message=message)


# [spec:pgorm:req:generative.compile-suite]
def validated(cases):
    """Refuse duplicate identities before anything is built for them."""
    seen = set()
    for case in cases:
        if not isinstance(case, CompileCase):
            raise CompileCaseError("not a compile case")
        if case.id in seen:
            raise CompileCaseError("duplicate case id: " + case.id)
        seen.add(case.id)
    return tuple(cases)


__all__ = [
    "KINDS",
    "PHASES",
    "VERDICTS",
    "CompileCase",
    "CompileCaseError",
    "Expectation",
    "rejects",
    "validated",
]
