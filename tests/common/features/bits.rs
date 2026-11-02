use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "bits")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Bit(None), select_as = "BIGINT", save_as = "BIT")]
    pub bit0: i64,
    #[pgorm(column_type = ColumnType::Bit(Some(1)), select_as = "BIGINT", save_as = "BIT(1)")]
    pub bit1: i64,
    #[pgorm(column_type = ColumnType::Bit(Some(8)), select_as = "BIGINT", save_as = "BIT(8)")]
    pub bit8: i64,
    #[pgorm(
        column_type = ColumnType::Bit(Some(16)),
        select_as = "BIGINT",
        save_as = "BIT(16)"
    )]
    pub bit16: i64,
    #[pgorm(
        column_type = ColumnType::Bit(Some(32)),
        select_as = "BIGINT",
        save_as = "BIT(32)"
    )]
    pub bit32: i64,
    #[pgorm(
        column_type = ColumnType::Bit(Some(64)),
        select_as = "BIGINT",
        save_as = "BIT(64)"
    )]
    pub bit64: i64,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
