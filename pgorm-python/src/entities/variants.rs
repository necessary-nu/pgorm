//! Every column type pgorm-query names against the kind a registered
//! entity's column binds and reads as, so a type pgorm-query gains fails this
//! test until the binding gives it a kind or names it as one without.

use pgorm::pgorm_query::{ColumnType, tests_cfg::Variants};

use super::metadata::InputKind;

// [spec:pgorm:req:bindings.entities+4/test]
#[test]
fn only_types_without_a_kind_take_explicit_values() {
    let explicit: Vec<&str> = ColumnType::variants()
        .iter()
        .filter(|ty| matches!(InputKind::from_column(ty), InputKind::Explicit))
        .map(Variants::variant_name)
        .collect();
    assert_eq!(explicit, ["Bit", "VarBit", "Named", "LTree"]);
}
