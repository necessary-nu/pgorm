use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "pi")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Decimal(Some((11, 10))))]
    pub decimal: Decimal,
    #[pgorm(column_type = ColumnType::Decimal(Some((11, 10))))]
    pub decimal_opt: Option<Decimal>,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
