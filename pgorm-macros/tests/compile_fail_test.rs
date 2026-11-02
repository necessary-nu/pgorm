//! Compile-failure verification for the derives whose contract is a *refusal*.
//!
//! Each fixture in `tests/compile-fail/` is paired with a `.stderr` snapshot
//! that trybuild asserts against. Only refusals the derives *word themselves*
//! belong here: a snapshot of a downstream `E0277` re-renders whenever rustc
//! or trybuild changes how it lays a diagnostic out, which says nothing about
//! the macro. Contracts of that shape are pinned on the generated tokens in
//! `sql_type_match`'s unit tests instead.

// [spec:pgorm:req:macros.derive.one-form/test]    a string `column_type` or function, a column read or written two ways, `save_with` and `save_array_with` apart, a literal name on the entity-trait derive, and an `on_condition` closure not of two tables are refused
// [spec:pgorm:req:macros.derive.entity-model.reject+3/test]    struct must be `Model`; the entity must have a primary key
// [spec:pgorm:sem:macros.derive.entity-model.casing+3/test]    field names deriving no identifier, and an `enum_name` that spells none
// [spec:pgorm:syn:macros.derive.entity-model.attrs+6/test]    an unknown key at struct level, at field level, and in each derive reading a subset of the same vocabulary; a string where Rust code is expected; a column read or written two ways, and `save_with` and `save_array_with` apart
// [spec:pgorm:sem:macros.derive.entity+5/test]    a literal name given to the entity-trait derive
// [spec:pgorm:sem:macros.derive.from-query-result+3/test]    an unknown field key
// [spec:pgorm:sem:macros.derive.partial-model+4/test]    an unknown field key, and the both-keys conflict the accumulating parser makes reachable
// [spec:pgorm:sem:macros.derive.value-type+5/test]    non-tuple input and an unread `#[pgorm(...)]` key
// [spec:pgorm:syn:macros.derive.active-enum+3/test]    `rs_type` / `db_type` are mandatory
// [spec:pgorm:syn:macros.derive.relation+5/test]    `belongs_to` without `from` is rejected, `from` / `to` of unequal arity are rejected while the arity is still known, an `enforcement` or `deferrability` naming no variant is rejected at its value, and an `on_condition` closure takes two tables
// [spec:pgorm:req:entity.relation.builder+1/test]    a builder given no columns has no conversion into a `RelationDef`
// [spec:pgorm:sem:macros.derive.entity-model.primary-key+7/test]    an identity beside a second identity form, `auto_increment`, a default or a nullable field, `auto_increment = true` on a composite key, a thirteenth key column, and `primary_key` given twice on one field
// [spec:pgorm:sem:macros.derive.entity-model.column-def+9/test]    a generated column given both kinds, beside an identity, a default or `auto_increment`, and a virtual one keyed, unique or indexed
#[test]
fn compile_fail_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("./tests/compile-fail/*.rs");
}
