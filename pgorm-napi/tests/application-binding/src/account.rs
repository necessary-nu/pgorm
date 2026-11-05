//! An application's account: an entity with an enum column, a renamed and
//! indexed column, comments, defaults its `ActiveModelBehavior::new` sets,
//! and hooks that change and refuse writes, so the binding is seen to run
//! them and to generate its schema.

use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
#[pgorm(enum_name = "mood", schema_name = "napi_entities")]
pub enum Mood {
    #[pgorm(string_value = "calm")]
    Calm,
    #[pgorm(string_value = "busy")]
    Busy,
}

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(
    table_name = "accounts",
    schema_name = "napi_entities",
    comment = "Accounts' native schema",
    hooks
)]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    #[pgorm(
        column_name = "display name",
        indexed,
        comment = "The user's display name"
    )]
    pub name: String,
    pub note: Option<String>,
    pub version: i32,
    pub mood: Mood,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::note::Entity)]
    Note,
}

// [spec:pgorm:req:bindings.entity-writes+1/test]
#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    fn new() -> Self {
        Self {
            version: pgorm::set(1),
            mood: pgorm::set(Mood::Calm),
            ..<Self as ActiveModelTrait>::default()
        }
    }

    async fn before_save<C>(mut self, _: &C, insert: bool) -> Result<Self, pgorm::Error>
    where
        C: pgorm::ConnectionTrait,
    {
        let name = self.name.try_as_ref().cloned().unwrap_or_default();
        if name == "reject_before" {
            return Err(pgorm::Error::Custom("before_save refused".into()));
        }
        if insert {
            self.name = pgorm::set(format!("{name}|before"));
        } else if let Some(version) = self.version.try_as_ref().copied() {
            self.version = pgorm::set(version + 1);
        }
        Ok(self)
    }

    async fn after_save<C>(model: Model, _: &C, _: bool) -> Result<Model, pgorm::Error>
    where
        C: pgorm::ConnectionTrait,
    {
        if model.name.starts_with("reject_after") {
            return Err(pgorm::Error::Custom("after_save refused".into()));
        }
        Ok(model)
    }

    async fn before_delete<C>(self, _: &C) -> Result<Self, pgorm::Error>
    where
        C: pgorm::ConnectionTrait,
    {
        if self.id.try_as_ref() == Some(&99) {
            return Err(pgorm::Error::Custom("before_delete refused".into()));
        }
        Ok(self)
    }
}
