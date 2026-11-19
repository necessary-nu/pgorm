"""Assemble initial acceptance out of separately identified evidence.

Five things are gathered here and none of them is folded into another: the
runtime campaign report, Python/Rust replay parity, the bounded compile suite,
the named regressions that closed resolved discoveries, and the stress
demonstration. Each keeps the vocabulary it was produced with, so a reader can
tell a constructor call from a compile invocation from an oracle decision.

Acceptance is not the union of things that passed. A component that did not run
is recorded as not run, an obligation still outstanding is recorded as
outstanding, and every open finding is listed by name. The report fails when a
required component is missing, which is what stops absence from reading as
success.
"""

VERSION = 1

# Every retained finding directory, with the state it is in. The registry is
# checked against the directory listing: a finding added to the tree without an
# entry here makes acceptance fail rather than quietly disappear from the
# report.
FINDINGS = {
    "distinct-append": {
        "finding": "pipeline-append-distinct",
        "state": "resolved",
        "summary": "a distinct projection appended to itself refused to compile",
        "regressions": [
            {
                "test": "deduplicated_relations_still_combine",
                "path": "src/pipeline/tests.rs",
                "module": "pipeline::tests::deduplicated_relations_still_combine",
                "fix": "08e1ec95",
                "fix_subject": "fix: Settle grouped arity before a set operation composes",
                "without_the_fix": (
                    "removing the hoist in Pipeline::set_op leaves the "
                    "deduplicating group in place and the set operation is "
                    "refused for a column-count mismatch that is not there"
                ),
            }
        ],
    },
    "pipeline-join-distinct-order": {
        "finding": "pipeline-join-distinct-order",
        "state": "resolved",
        "summary": "a joined, deduplicated pipeline compiled to two projection orders",
        "regressions": [
            {
                "test": "a_joined_deduplicated_relation_compiles_once",
                "path": "src/pipeline/tests.rs",
                "module": "pipeline::tests::a_joined_deduplicated_relation_compiles_once",
                "fix": "d1ba69a1",
                "fix_subject": "fix: Pin a prqlc whose star expansion orders columns totally",
                "without_the_fix": (
                    "depending on registry prqlc in place of the fork "
                    "restores the hash-ordered star expansion and the same "
                    "pipeline compiles to two different projection orders"
                ),
            }
        ],
    },
    "pipeline-renamed-column-order": {
        "finding": "pipeline-renamed-column-order",
        "state": "open",
        "summary": "a renamed column in a deduplicated select is emitted last",
    },
    "pipeline-hidden-order": {
        "finding": "pipeline-hidden-order",
        "state": "open",
        "summary": "renamed hidden sort columns read from a CTE that never exposed them",
    },
    "pipeline-quoted-output": {
        "finding": "pipeline-quoted-output",
        "state": "open",
        "summary": "two authored quotes in a derived column name become one",
    },
    "pipeline-native-panic": {
        "finding": "pipeline-native-panic",
        "state": "open",
        "summary": "PRQL SQL generation panics on a generated pipeline",
    },
    "set-precedence": {
        "finding": "set-precedence",
        "state": "open",
        "summary": "chained set operations lose the pipeline's stage grouping",
    },
    "window-semantics": {
        "finding": "window-semantics",
        "state": "resolved",
        "summary": "count(expr) emitted COUNT(*) and an authored frame was dropped",
        "regressions": [
            {
                "test": "a_counted_expression_reaches_the_aggregate",
                "path": "src/pipeline/tests.rs",
                "module": "pipeline::tests::a_counted_expression_reaches_the_aggregate",
                "without_the_fix": (
                    "prqlc's standard library discards the counted column and "
                    "renders COUNT(*), so the aggregate answers the row count "
                    "wherever the counted expression is nullable"
                ),
            },
            {
                "test": "every_frame_direction_reaches_both_render_paths",
                "path": "src/pipeline/tests.rs",
                "module": (
                    "pipeline::tests::every_frame_direction_reaches_both_render_paths"
                ),
                "without_the_fix": (
                    "prqlc emits a frame only for the calls its standard "
                    "library annotates as frame-aware, which left out first "
                    "and last until upstream's #6435 and leaves out lag, lead "
                    "and the ranking functions, so an authored frame never "
                    "reached FIRST_VALUE or LAST_VALUE"
                ),
            },
        ],
    },
}

COMPONENTS = ("runtime", "parity", "compile", "regressions", "stress")

DISCLAIMERS = (
    (
        "The stress demonstration is construction through one native build only. "
        "It does not replace full live coverage and does not prove that all ORM "
        "programs are safe."
    ),
    (
        "A construction count and an oracle decision are different claims and are "
        "never added together anywhere in this document."
    ),
    (
        "Open findings below are unresolved defects in the library or its "
        "dependencies. Acceptance records them; it does not close them."
    ),
    (
        "Coverage obligations reported as outstanding were not discharged by the "
        "runs this document assembles."
    ),
)


def registry(directories):
    """Check the retained findings against the registry, in both directions."""
    known, found = set(FINDINGS), set(directories)
    return {
        "unregistered": sorted(found - known),
        "missing": sorted(known - found),
        "consistent": known == found,
    }


def resolved():
    """Findings whose discoveries were closed, each with its named regressions."""
    return [
        {"directory": name, **entry}
        for name, entry in sorted(FINDINGS.items())
        if entry["state"] == "resolved"
    ]


def opened():
    """Findings that remain open, listed so acceptance cannot omit them."""
    return [
        {"directory": name, **entry}
        for name, entry in sorted(FINDINGS.items())
        if entry["state"] == "open"
    ]


def _runtime(document):
    """The runtime campaign's own counts, kept in the fields it produced them in."""
    if not isinstance(document, dict):
        return {"executed": False, "reason": "no campaign report was supplied"}
    coverage = document.get("coverage", {})
    full = coverage.get("full_matrix", {})
    return {
        "executed": True,
        "passed": document.get("passed"),
        "profile": document.get("profile", {}).get("name"),
        "profile_claim": document.get("profile", {}).get("coverage", {}).get("claim"),
        "counts": document.get("counts", {}),
        "coverage": {
            "obligations": coverage.get("obligations"),
            "declared_satisfied": coverage.get("declared_satisfied"),
            "declared_total": coverage.get("declared_total"),
            "full_matrix_outstanding": full.get("outstanding"),
            "full_matrix_total": full.get("total"),
        },
        "controls": {
            key: value
            for key, value in (document.get("controls") or {}).items()
            if not isinstance(value, (list, dict))
        },
        "findings": [item["item"] for item in document.get("findings", ())],
        "source": document.get("source", {}),
        "seconds": document.get("timing", {}).get("total_seconds"),
    }


def _missing(name):
    return {"executed": False, "reason": "no " + name + " evidence was supplied"}


def _compile(document):
    if not isinstance(document, dict):
        return _missing("compile suite")
    return {
        "executed": True,
        "passed": document.get("passed"),
        **{key: value for key, value in document.items() if key.startswith("compile_")},
        "unattributed": document.get("unattributed"),
        "obligations": document.get("obligations"),
        "seconds": document.get("seconds"),
    }


def _parity(document):
    if not isinstance(document, dict):
        return _missing("replay parity")
    return {
        "executed": True,
        "passed": document.get("passed"),
        **{
            key: document[key]
            for key in (
                "families_declared",
                "families_attempted",
                "families_with_parity_established",
                "families_in_parity",
                "families_diverging",
                "families_without_established_parity",
                "seconds",
            )
            if key in document
        },
    }


def _stress(document):
    if not isinstance(document, dict):
        return _missing("stress")
    return {
        "executed": True,
        "passed": document.get("passed"),
        **{
            key: document[key]
            for key in (
                "target_distinct_programs",
                "distinct_generated_programs_constructed",
                "programs_attempted",
                "duplicate_programs_discarded",
                "reached_postgresql",
                "oracle_decided",
                "native_build_identity",
                "native_build_identity_unchanged",
                "extension_builds_during_stress",
                "per_program_builds",
                "throughput",
                "construction_errors",
            )
            if key in document
        },
    }


def _regressions(evidence):
    """Named regressions, paired with whatever was actually run for them."""
    outcomes = {item["test"]: item for item in (evidence or ())}
    entries = []
    for item in resolved():
        for regression in item["regressions"]:
            entries.append(
                {
                    "finding": item["finding"],
                    "directory": item["directory"],
                    **regression,
                    "evidence": outcomes.get(
                        regression["test"], {"executed": False, "reason": "not run"}
                    ),
                }
            )
    return entries


def _shortfalls(sections, findings, regressions):
    """Everything acceptance is explicitly not claiming, gathered in one list."""
    outstanding = []
    for name in COMPONENTS:
        if not sections[name].get("executed"):
            outstanding.append(name + ": " + sections[name]["reason"])
        elif sections[name].get("passed") is False:
            outstanding.append(name + ": the component did not pass")
    runtime = sections["runtime"]
    if runtime.get("executed"):
        coverage = runtime["coverage"]
        if coverage.get("obligations") != "full-matrix":
            outstanding.append(
                "runtime coverage was "
                + str(coverage.get("obligations"))
                + ", not the full matrix"
            )
        if coverage.get("full_matrix_outstanding"):
            outstanding.append(
                str(coverage["full_matrix_outstanding"])
                + " of "
                + str(coverage.get("full_matrix_total"))
                + " full-matrix obligations were not discharged"
            )
        # A component that "did not pass" says nothing about how much of it did
        # not pass. The retained items are named so the scale is in the report
        # rather than only in the run directory.
        if runtime.get("findings"):
            outstanding.append(
                "the runtime campaign retained "
                + str(len(runtime["findings"]))
                + " findings: "
                + ", ".join(runtime["findings"])
            )
    parity = sections["parity"]
    if parity.get("families_diverging"):
        outstanding.append(
            "replay parity diverges for "
            + ", ".join(parity["families_diverging"])
            + "; see the parity report's per-step checks"
        )
    if parity.get("families_without_established_parity"):
        outstanding.append(
            "replay parity could not be established for "
            + ", ".join(parity["families_without_established_parity"])
        )
    stress = sections["stress"]
    for name, count in (stress.get("construction_errors") or {}).items():
        outstanding.append(
            "stress: "
            + str(count)
            + " programs failed construction with "
            + name
            + "; see construction_failure_samples in the stress report"
        )
    outstanding.extend(
        "open finding: " + item["finding"] + " — " + item["summary"]
        for item in findings
    )
    outstanding.extend(
        "regression " + item["test"] + ": " + item["evidence"]["reason"]
        for item in regressions
        if not item["evidence"].get("executed")
    )
    return outstanding


# [spec:pgorm:req:generative.acceptance]
def assemble(
    *,
    runtime,
    parity,
    compile_document,
    stress,
    regressions,
    directories,
    attempts=(),
):
    """Build the acceptance document from the evidence each component produced.

    `attempts` carries profiles that were started and did not finish. They are
    not evidence of anything and are not scored; they are here so a report that
    ran smoke cannot be read as having declined to try the full matrix.
    """
    sections = {
        "runtime": _runtime(runtime),
        "parity": _parity(parity),
        "compile": _compile(compile_document),
        "stress": _stress(stress),
        "regressions": _regression_section(regressions),
    }
    named = _regressions(regressions)
    findings = opened()
    checked = registry(directories)
    outstanding = _shortfalls(sections, findings, named)
    for attempt in attempts:
        if not attempt.get("completed"):
            outstanding.append(
                "profile "
                + str(attempt.get("profile"))
                + " was started and did not complete: "
                + str(attempt.get("reason"))
            )
    if not checked["consistent"]:
        outstanding.append(
            "finding registry disagrees with the retained directories: " + str(checked)
        )
    document = {
        "version": VERSION,
        "kind": "initial-acceptance",
        "components": sections,
        "profile_attempts": list(attempts),
        "named_regressions": named,
        "open_findings": findings,
        "resolved_findings": [item["finding"] for item in resolved()],
        "finding_registry": checked,
        "outstanding": outstanding,
        "disclaimers": list(DISCLAIMERS),
    }
    document["complete"] = not outstanding
    document["passed"] = (
        all(
            sections[name].get("executed") and sections[name].get("passed") is not False
            for name in COMPONENTS
        )
        and checked["consistent"]
    )
    return document


def _regression_section(evidence):
    """Regressions are a component too, and are scored as one."""
    named = _regressions(evidence)
    ran = [item for item in named if item["evidence"].get("executed")]
    if not named:
        return {"executed": False, "reason": "no resolved finding names a regression"}
    if not ran:
        return {"executed": False, "reason": "no named regression was executed"}
    return {
        "executed": True,
        "passed": all(item["evidence"].get("passed") for item in ran)
        and len(ran) == len(named),
        "named": len(named),
        "executed_count": len(ran),
        "counterfactual_shown": sum(
            1
            for item in ran
            if item["evidence"].get("counterfactual", {}).get("failed")
        ),
    }


def render(document):
    """The markdown acceptance summary; the JSON document remains the evidence."""
    sections = document["components"]
    lines = [
        "# Initial acceptance",
        "",
        "Status: " + ("PASS" if document["passed"] else "INCOMPLETE"),
        "",
        "## Components",
        "",
    ]
    for name in COMPONENTS:
        entry = sections[name]
        state = (
            "not run — " + entry["reason"]
            if not entry.get("executed")
            else "passed"
            if entry.get("passed")
            else "ran, did not pass"
        )
        lines.append("- **" + name + "**: " + state)
    if document["profile_attempts"]:
        lines += ["", "## Profiles started", ""]
        for attempt in document["profile_attempts"]:
            lines.append(
                "- `"
                + str(attempt.get("profile"))
                + "`: "
                + ("completed" if attempt.get("completed") else "did not complete")
                + " — "
                + str(attempt.get("reason"))
            )
    lines += ["", "## Named regressions", ""]
    for item in document["named_regressions"]:
        evidence = item["evidence"]
        counterfactual = evidence.get("counterfactual") or {}
        lines.append(
            "- `"
            + item["finding"]
            + "` -> `"
            + item["module"]
            + "` in `"
            + item["path"]
            + "`: "
            + ("passes here" if evidence.get("passed") else evidence.get("reason", ""))
            + "; without the fix ("
            + str(counterfactual.get("removal", "no removal declared"))
            + ") "
            + str(counterfactual.get("reason", "not attempted"))
        )
    lines += ["", "## Open findings", ""]
    for item in document["open_findings"]:
        lines.append("- `" + item["finding"] + "`: " + item["summary"])
    lines += ["", "## Outstanding", ""]
    if document["outstanding"]:
        lines.extend("- " + item for item in document["outstanding"])
    else:
        lines.append("- nothing outstanding")
    lines += ["", "## This report does not claim", ""]
    lines.extend("- " + item for item in document["disclaimers"])
    return "\n".join(lines) + "\n"


__all__ = [
    "COMPONENTS",
    "DISCLAIMERS",
    "FINDINGS",
    "VERSION",
    "assemble",
    "opened",
    "registry",
    "render",
    "resolved",
]
