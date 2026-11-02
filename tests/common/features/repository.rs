use super::edit_log;
use pgorm::entity::prelude::*;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Entity, Serialize)]
#[pgorm(table_name = "repository")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: String,
    pub owner: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    async fn before_save<C>(self, db: &C, _: bool) -> Result<Self, Error>
    where
        C: ConnectionTrait,
    {
        let model = self.clone().try_into_model()?;
        insert_edit_log("before_save", &model, db).await?;
        Ok(self)
    }

    async fn after_save<C>(model: Model, db: &C, _: bool) -> Result<Model, Error>
    where
        C: ConnectionTrait,
    {
        insert_edit_log("after_save", &model, db).await?;
        Ok(model)
    }

    async fn before_delete<C>(self, db: &C) -> Result<Self, Error>
    where
        C: ConnectionTrait,
    {
        let model = self.clone().try_into_model()?;
        insert_edit_log("before_delete", &model, db).await?;
        Ok(self)
    }

    async fn after_delete<C>(self, db: &C) -> Result<Self, Error>
    where
        C: ConnectionTrait,
    {
        let model = self.clone().try_into_model()?;
        insert_edit_log("after_delete", &model, db).await?;
        Ok(self)
    }
}

async fn insert_edit_log<T, M, C>(action: T, model: &M, db: &C) -> Result<(), Error>
where
    T: Into<String>,
    M: Serialize,
    C: ConnectionTrait,
{
    edit_log::ActiveModel {
        action: set(action.into()),
        values: set(serde_json::json!(model)),
        ..Default::default()
    }
    .insert(db)
    .await?;

    Ok(())
}
