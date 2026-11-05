//! Compile-failure verification for the derives whose contract is a *refusal*.
//!
//! Each fixture in `tests/compile-fail/` is paired with a `.stderr` snapshot
//! that trybuild asserts against. Only refusals the derives *word themselves*
//! belong here: a snapshot of a downstream `E0277` re-renders whenever rustc
//! or trybuild changes how it lays a diagnostic out, which says nothing about
//! the macro. Contracts of that shape are pinned on the generated tokens in
//! `sql_type_match`'s unit tests instead.

// [spec:pgorm:req:macros.derive.one-form/test]    a string `column_type` or function, a column read or written two ways, `save_with` and `save_array_with` apart, a literal name on the entity-trait derive, and an `on_condition` closure not of two tables are refused
// [spec:pgorm:req:macros.derive.entity-model.reject+4/test]    struct must be `Model`; the entity must have a primary key
// [spec:pgorm:req:macros.derive.code-values+4/test]    a string where Rust code is expected is refused at the string, saying to remove the quotes: a relation's target, action and condition type, `rs_type`, `db_type`, `array_type`, `PartialModel`'s `entity` and `from_expr`, and a model field's `variant` and `default_expr`
// [spec:pgorm:sem:macros.derive.entity-model.casing+4/test]    field names deriving no identifier
// [spec:pgorm:syn:macros.derive.entity-model.attrs+10/test]    an unknown key at struct level, at field level, and in each derive reading a subset of the same vocabulary; a string where Rust code is expected; a column read or written two ways, and `save_with` and `save_array_with` apart; a `unique(..)` of no columns or one, a column twice, a path naming no column or a VIRTUAL generated one, a quoted column, a name twice or past 63 bytes, an unknown key, and one on a model given no `table_name`
// [spec:pgorm:sem:macros.derive.entity+7/test]    a literal name given to the entity-trait derive
// [spec:pgorm:sem:macros.derive.from-query-result+4/test]    an unknown field key, and a projection key refused as `PartialModel`'s
// [spec:pgorm:sem:macros.derive.model+7/test]    a further model refuses the composite's container and field keys, and `enum_name` naming `variant`
// [spec:pgorm:sem:macros.derive.column+7/test]    an unknown variant key, and any key on the enum
// [spec:pgorm:sem:macros.derive.active-model+9/test]    `IntoActiveModel` refuses a field key
// [spec:pgorm:sem:macros.derive.partial-model+6/test]    an unknown field key, and the both-keys conflict the accumulating parser makes reachable
// [spec:pgorm:sem:macros.derive.value-type+7/test]    non-tuple input and an unread `#[pgorm(...)]` key
// [spec:pgorm:syn:macros.derive.active-enum+6/test]    `db_type` without `rs_type`, `rs_type` without `db_type`, a number or a discriminant in a PostgreSQL enum, an unknown key at either level, named, and a PostgreSQL enum's value over 63 bytes, given or derived from its variant, spelt in one byte a character or more, refused at the value or the variant
// [spec:pgorm:syn:macros.derive.relation+7/test]    `belongs_to` without `from` is rejected, `from` / `to` of unequal arity are rejected while the arity is still known, an `on_condition` closure takes two tables, a variant is one kind of relation, the enum's keys are `entity` alone, `hooks` included, and `related` is only `false`, on an entity one variant reaches
// [spec:pgorm:sem:macros.derive.boilerplate/test]    the refusals the derives word for what they write: a lone `rs_type`, a PostgreSQL enum's number or discriminant, and a `related = false` that says nothing
// [spec:pgorm:req:entity.relation.builder+1/test]    a builder given no columns has no conversion into a `RelationDef`
// [spec:pgorm:sem:macros.derive.entity-model.primary-key+7/test]    an identity beside a second identity form, `auto_increment`, a default or a nullable field, `auto_increment = true` on a composite key, a thirteenth key column, and `primary_key` given twice on one field
// [spec:pgorm:sem:macros.derive.entity-model.column-def+10/test]    a generated column given both kinds, beside an identity, a default or `auto_increment`, and a virtual one keyed, unique or indexed
// [spec:pgorm:def:disjoint.declare+5/test]    the refusals the disjoint derive words: not an enum, generic, no `base`, a quoted `base`, a key twice or unknown, no variants, a variant of another shape, a discriminant, a key on the payload, two kinds of one value, and a kind's value over 63 bytes, given or derived from its variant, refused at the value or the variant
// [spec:pgorm:sem:disjoint.unkinded+1/test]    the refusals an `unkinded` declaration words: `kind`, `enum_name` or `schema_name` beside it, `unkinded` twice, a variant's `value`, a second unit variant, and no variant holding a kind's `Model`
// [spec:pgorm:def:disjoint.one-of+4/test]    the refusals the one-of derive words: not an enum, generic, no `entity`, a quoted `entity` or `column`, a key twice or unknown, no variant naming a target, a variant of another shape, a discriminant, a target with no `column`, a unit variant with one, a second unit variant, a key on the target, and a name over 59 bytes in snake_case, which the CHECK `create_one_of` names `ck-{enum}` could not keep whole
#[test]
fn compile_fail_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("./tests/compile-fail/*.rs");
}
