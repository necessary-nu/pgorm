use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[pgorm(table_name = "tenant_task")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub tenant_id: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::Text)]
    pub title: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[pgorm(has_many = "super::tenant_task_note::Entity")]
    TenantTaskNote,
}

impl Related<super::tenant_task_note::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::TenantTaskNote.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
