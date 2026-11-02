use super::pgorm_active_enums::*;
use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "custom_active_model")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub weight: Option<f32>,
    pub amount: Option<i32>,
    pub category: Option<Category>,
    pub color: Option<Color>,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Clone, Debug, PartialEq, IntoActiveModel)]
pub struct CustomActiveModel {
    pub weight: Option<f32>,
    pub amount: Option<Option<i32>>,
    pub category: Option<Category>,
    pub color: Option<Option<Color>>,
}

impl IntoActiveValue<Category> for Category {
    fn into_active_value(self) -> ActiveValue<Category> {
        set(self)
    }
}

impl IntoActiveValue<Color> for Color {
    fn into_active_value(self) -> ActiveValue<Color> {
        set(self)
    }
}
