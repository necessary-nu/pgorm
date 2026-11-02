use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[pgorm(table_name = "tenant_task_note")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub tenant_id: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub task_id: i32,
    #[pgorm(column_type = ColumnType::Text)]
    pub body: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[pgorm(
        belongs_to = "super::tenant_task::Entity",
        from = "(Column::TenantId, Column::TaskId)",
        to = "(super::tenant_task::Column::TenantId, super::tenant_task::Column::Id)",
        on_delete = "Cascade"
    )]
    TenantTask,
}

impl Related<super::tenant_task::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::TenantTask.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
