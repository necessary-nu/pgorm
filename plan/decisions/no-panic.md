---
id [dec:pgorm:no-panic]
epitome "Library code must not panic on user input; user-facing failure paths return Result."
state @approved
category @ban
scope {
    rules ([spec:pgorm:def:error.model] [spec:pgorm:sem:sql.value.accessor-panics] [spec:pgorm:sem:sql.ddl.panics] [spec:pgorm:req:sql.ast.condition.holder] [spec:pgorm:req:query.build.insert.uniform-columns] [spec:pgorm:req:conn.pool])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Panic at the misuse site with a clear message, matching the builder's inherited panic conventions."
        rejected_because "A panic in a library escalates a caller's recoverable mistake into process death; the operator's directive is explicit that things should not panic."
    }
    {
        option "Keep panics but document every one under missing_panics_doc."
        rejected_because "Documentation does not make the failure recoverable; it only makes the escalation predictable."
    }
)
consequences {
    accepted (
        "New and changed user-facing failure paths return Error (or another Result channel) rather than panicking, even where the surrounding inherited code still panics."
        "Where an infallible rendering API (to_string/build) cannot carry an error, the guard lives at the nearest fallible boundary — typically the ORM execution layer."
    )
    deferred ("The inherited panic surface (value accessor panics, condition-mixing panic, insert arity panics, connect()'s panic on pool-build failure) predates this decision and is spec-documented; retrofitting it to Result is future work, node by node, each with a rule bump. The DDL builder panics, originally on this list, were closed by construction under unrep.ddl-empty-builders.")
}
codifies ([spec:pgorm:def:error.model])
---

## Rationale

pgorm is a library: its callers own the process, and a caller's
recoverable mistake — an unset primary key, an emptied select list, a
misassembled statement — must come back to them as a value they can
handle, not as an unwound stack. `Error` (née `DbErr`) exists precisely to be that
channel, and every execution-adjacent path already returns
`Result<_, Error>`.

Panics remain legitimate for exactly one thing: internal invariant
violations, where continuing would be incoherent and the bug is
pgorm's own. The distinction is who made the mistake — the caller's
mistakes are typed errors; pgorm's own bugs may panic.

The inherited SeaORM surface predates this rule and still panics in
documented places. Those panics are spec-pinned as current behaviour
and are not silently rewritten; they are converted deliberately,
node by node, with their rules bumped, whenever a touched area brings
one into reach. New code, and any changed behaviour, follows the ban
from the start.
