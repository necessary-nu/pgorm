use pgorm::entity::prelude::*;
use pgorm::{
    RowIndex, TryGetError, TryGetable,
    pgorm_query::{ArrayType, ColumnType, ValueType},
};

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "event_trigger")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub events: Events,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Events(pub Vec<Event>);

impl From<Events> for Value {
    fn from(events: Events) -> Self {
        let Events(events) = events;
        Value::Array(
            ArrayType::String,
            Some(Box::new(
                events
                    .into_iter()
                    .map(|Event(s)| Value::String(Some(Box::new(s))))
                    .collect(),
            )),
        )
    }
}

impl TryGetable for Events {
    fn try_get_by<I: RowIndex + std::fmt::Display>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, TryGetError> {
        let vec: Vec<String> = res.try_get_by(idx).map_err(TryGetError::Db)?;
        Ok(Events(vec.into_iter().map(Event).collect()))
    }
}

impl ValueType for Events {
    fn try_from(v: Value) -> Result<Self, pgorm_query::ValueTypeError> {
        let value: Option<Vec<String>> = ValueType::try_from(v)?;
        let vec = match value {
            Some(v) => v.into_iter().map(Event).collect(),
            None => vec![],
        };
        Ok(Events(vec))
    }

    fn type_name() -> String {
        stringify!(Events).to_owned()
    }

    fn array_type() -> ArrayType {
        ArrayType::String
    }

    fn column_type() -> ColumnType {
        ColumnType::Array(Arc::new(ColumnType::String(StringLen::None)))
    }
}
