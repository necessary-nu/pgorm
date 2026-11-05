use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(schema_name = "schema_name", table_name = "fruit")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::String(StringLen::N(255u32)))]
    pub name: String,
    pub cake_id: Option<i32> ,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::cake::Entity,
        from = Column::CakeId,
        to = super::cake::Column::Id,
    )]
    Cake,
    #[pgorm(has_many = super::vendor::Entity)]
    Vendor,
}
