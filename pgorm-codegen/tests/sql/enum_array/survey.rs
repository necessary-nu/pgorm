use pgorm::entity::prelude::*;

use super::pgorm_active_enums::{Mood, Tier};

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "survey")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub moods: Vec<Mood>,
    pub past_moods: Option<Vec<Mood>>,
    pub tiers: Vec<Tier>,
    pub past_tiers: Option<Vec<Tier>>,
    #[pgorm(
        column_type = ColumnType::Domain {
            name: Name::runtime("mood_log"),
            schema: None,
            base: Arc::new(
                ColumnType::Array(Arc::new(Mood::db_type().get_column_type().clone())),
            ),
        },
        nullable
    )]
    pub log: Option<Vec<Mood>>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
