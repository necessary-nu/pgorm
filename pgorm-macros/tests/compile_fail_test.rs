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
// [spec:pgorm:req:macros.derive.code-values+2/test]    a string where Rust code is expected is refused at the string, saying to remove the quotes: a relation's target, action and condition type, `rs_type`, `db_type`, `array_type`, `PartialModel`'s `entity` and `from_expr`, and a model field's `variant` and `default_expr`
// [spec:pgorm:sem:macros.derive.entity-model.casing+4/test]    field names deriving no identifier
// [spec:pgorm:syn:macros.derive.entity-model.attrs+8/test]    an unknown key at struct level, at field level, and in each derive reading a subset of the same vocabulary; a string where Rust code is expected; a column read or written two ways, and `save_with` and `save_array_with` apart
// [spec:pgorm:sem:macros.derive.entity+6/test]    a literal name given to the entity-trait derive
// [spec:pgorm:sem:macros.derive.from-query-result+4/test]    an unknown field key, and a projection key refused as `PartialModel`'s
// [spec:pgorm:sem:macros.derive.model+5/test]    a further model refuses the composite's container and field keys, and `enum_name` naming `variant`
// [spec:pgorm:sem:macros.derive.column+7/test]    an unknown variant key, and any key on the enum
// [spec:pgorm:sem:macros.derive.active-model+7/test]    `IntoActiveModel` refuses a field key
// [spec:pgorm:sem:macros.derive.partial-model+5/test]    an unknown field key, and the both-keys conflict the accumulating parser makes reachable
// [spec:pgorm:sem:macros.derive.value-type+6/test]    non-tuple input and an unread `#[pgorm(...)]` key
// [spec:pgorm:syn:macros.derive.active-enum+5/test]    `db_type` without `rs_type`, `rs_type` without `db_type`, a number or a discriminant in a PostgreSQL enum, and an unknown key at either level, named
// [spec:pgorm:syn:macros.derive.relation+7/test]    `belongs_to` without `from` is rejected, `from` / `to` of unequal arity are rejected while the arity is still known, an `on_condition` closure takes two tables, a variant is one kind of relation, the enum's keys are `entity` alone, `hooks` included, and `related` is only `false`, on an entity one variant reaches
// [spec:pgorm:sem:macros.derive.boilerplate/test]    the refusals the derives word for what they write: a lone `rs_type`, a PostgreSQL enum's number or discriminant, and a `related = false` that says nothing
// [spec:pgorm:req:entity.relation.builder+1/test]    a builder given no columns has no conversion into a `RelationDef`
// [spec:pgorm:sem:macros.derive.entity-model.primary-key+7/test]    an identity beside a second identity form, `auto_increment`, a default or a nullable field, `auto_increment = true` on a composite key, a thirteenth key column, and `primary_key` given twice on one field
// [spec:pgorm:sem:macros.derive.entity-model.column-def+10/test]    a generated column given both kinds, beside an identity, a default or `auto_increment`, and a virtual one keyed, unique or indexed
#[test]
fn compile_fail_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("./tests/compile-fail/*.rs");
}
