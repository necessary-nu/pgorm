---
id [dec:pgorm:invalid-states-unrepresentable]
epitome "Make invalid states unrepresentable: prefer type-level designs that prevent misuse over runtime checks."
state @approved
category @executive
scope {
    rules ([spec:pgorm:req:sql.ast.condition.holder] [spec:pgorm:req:query.build.insert.uniform-columns] [spec:pgorm:sem:exec.crud.try-insert] [spec:pgorm:def:entity.active-model.active-value])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Validate at runtime and return errors for every misuse."
        rejected_because "A misuse the type system rejects costs nothing at runtime and cannot ship; runtime validation is the fallback rung, not the design."
    }
    {
        option "Document the invalid combinations and trust callers."
        rejected_because "Documentation neither prevents nor recovers; it only apologizes in advance."
    }
)
consequences {
    accepted (
        "New and redesigned API surface models its states so that invalid combinations do not construct — typestate, distinct types, and closed enums are preferred even at some verbosity cost."
        "Where a state genuinely cannot be designed out, the failure follows [dec:pgorm:no-panic]: a typed error at the nearest fallible boundary."
    )
    deferred ("Inherited runtime-checked seams (the condition-holder mixing panic, insert arity and uniform-columns panics, the dynamically emptied select list) predate this decision; each is a candidate for type-level redesign when its area is next touched, with rule bumps.")
}
edges {
    related_to ([dec:pgorm:no-panic])
}
---

## Rationale

The strongest error handling is the error that cannot occur. Rust's
type system can carry most of pgorm's protocol rules — a query that
must have a projection, a condition holder that is chain-style or
condition-style but never both, an insert whose models share one
column set — and where it does, misuse dies at compile time instead
of in production.

This ranks the failure-handling ladder explicitly: first make the
invalid state unrepresentable; where that is impractical, return a
typed error per [dec:pgorm:no-panic]; a panic on caller input is
never the design. The crate already contains the pattern done well —
`ActiveValue`'s three-state enum, `TryInsert` as a distinct type
rather than a flag — and inherited runtime-checked seams are
converted toward it deliberately, area by area, not by blanket
rewrite.
