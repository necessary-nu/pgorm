use crate::common::setup::rust_dec;
use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "cake", hooks)]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
    #[pgorm(column_type = ColumnType::Decimal(Some((16, 4))))]
    pub price: Decimal,
    pub bakery_id: Option<i32>,
    pub gluten_free: bool,
    pub serial: Uuid,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::bakery::Entity,
        from = Column::BakeryId,
        to = super::bakery::Column::Id,
        on_update = ForeignKeyAction::Cascade,
        on_delete = ForeignKeyAction::SetNull
    )]
    Bakery,
    #[pgorm(has_many = super::lineitem::Entity)]
    Lineitem,
}

impl Related<super::baker::Entity> for Entity {
    fn to() -> RelationDef {
        super::cakes_bakers::Relation::Baker.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::cakes_bakers::Relation::Cake.def().rev())
    }
}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    fn new() -> Self {
        Self {
            serial: set(Uuid::new_v4()),
            ..ActiveModelTrait::default()
        }
    }

    async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, Error>
    where
        C: ConnectionTrait,
    {
        if self.price.as_ref() == &rust_dec(0) {
            Err(Error::Custom(format!(
                "[before_save] Invalid Price, insert: {insert}"
            )))
        } else {
            Ok(self)
        }
    }

    async fn after_save<C>(model: Model, _db: &C, insert: bool) -> Result<Model, Error>
    where
        C: ConnectionTrait,
    {
        if model.price < rust_dec(0) {
            Err(Error::Custom(format!(
                "[after_save] Invalid Price, insert: {insert}"
            )))
        } else {
            Ok(model)
        }
    }

    async fn before_delete<C>(self, _db: &C) -> Result<Self, Error>
    where
        C: ConnectionTrait,
    {
        if self.name.as_ref().contains("(err_on_before_delete)") {
            Err(Error::Custom(
                "[before_delete] Cannot be deleted".to_owned(),
            ))
        } else {
            Ok(self)
        }
    }

    async fn after_delete<C>(self, _db: &C) -> Result<Self, Error>
    where
        C: ConnectionTrait,
    {
        if self.name.as_ref().contains("(err_on_after_delete)") {
            Err(Error::Custom("[after_delete] Cannot be deleted".to_owned()))
        } else {
            Ok(self)
        }
    }
}
