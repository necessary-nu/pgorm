use pgorm::entity::prelude:: * ;

#[derive(Clone, Debug, PartialEq, Entity, Eq, ts_rs::TS, utoipa::ToSchema)]
#[pgorm(table_name = "cake")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Text, nullable)]
    pub name: Option<String> ,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::fruit::Entity)]
    Fruit,
}

impl Related<super::filling::Entity> for Entity {
    fn to() -> RelationDef {
        super::cake_filling::Relation::Filling.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::cake_filling::Relation::Cake.def().rev())
    }
}
