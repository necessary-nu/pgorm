use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(schema_name = "crm", table_name = "person")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub tenant_id: i32,
    #[pgorm(primary_key, auto_increment = false)]
    pub account_id: i64,
    pub given_name: String,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::account::Entity,
        from = (Column::TenantId, Column::AccountId),
        to = (super::account::Column::Tenant, super::account::Column::Id)
    )]
    Account,
}
