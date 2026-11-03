use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(schema_name = "crm", table_name = "company")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub tenant_id: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub account_id: i64,
    pub vat: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::account::Entity,
        from = (Column::AccountId, Column::TenantId),
        to = (super::account::Column::Id, super::account::Column::Tenant)
    )]
    Account,
}
