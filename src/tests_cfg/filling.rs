use crate as pgorm;
use crate::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "filling")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
    pub vendor_id: Option<i32>,
    #[pgorm(ignore)]
    pub ignored_attr: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "super::vendor::Entity",
        from = "Column::VendorId",
        to = "super::vendor::Column::Id"
    )]
    Vendor,
}

impl Related<super::cake::Entity> for Entity {
    fn to() -> RelationDef {
        super::cake_filling::Relation::Cake.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::cake_filling::Relation::Filling.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
