use pgorm::TryGetableFromJson;
use pgorm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "json_vec")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub str_vec: Option<StringVec>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StringVec(pub Vec<String>);

impl TryGetableFromJson for StringVec {}

impl From<StringVec> for Value {
    fn from(source: StringVec) -> Self {
        pgorm::Value::Json(serde_json::to_value(source).ok().map(std::boxed::Box::new))
    }
}

impl pgorm_query::ValueType for StringVec {
    fn try_from(v: Value) -> Result<Self, pgorm_query::ValueTypeError> {
        match v {
            pgorm::Value::Json(Some(json)) => Ok(
                serde_json::from_value(*json).map_err(|_| pgorm::pgorm_query::ValueTypeError)?
            ),
            _ => Err(pgorm::pgorm_query::ValueTypeError),
        }
    }

    fn type_name() -> String {
        stringify!(StringVec).to_owned()
    }

    fn array_type() -> pgorm::pgorm_query::ArrayType {
        pgorm::pgorm_query::ArrayType::Json
    }

    fn column_type() -> pgorm_query::ColumnType {
        pgorm_query::ColumnType::Json
    }
}

impl pgorm::pgorm_query::Nullable for StringVec {
    fn null() -> pgorm::Value {
        pgorm::Value::Json(None)
    }
}
