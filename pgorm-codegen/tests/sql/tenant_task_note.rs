use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
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

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::tenant_task::Entity,
        from = (Column::TenantId, Column::TaskId),
        to = (super::tenant_task::Column::TenantId, super::tenant_task::Column::Id),
        on_delete = ForeignKeyAction::Cascade
    )]
    TenantTask,
}
