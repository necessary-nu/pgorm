use pgorm::Iterable;
use pgorm::SqlName;
use pgorm::prelude::*;
use pgorm_macros::Entity;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "user", rename_all = "camelCase")]
pub struct Model {
    #[pgorm(primary_key)]
    id: i32,
    username: String,
    first_name: String,
    middle_name: String,
    #[pgorm(column_name = "lAsTnAmE")]
    last_name: String,
    orders_count: i32,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}

#[test]
fn test_column_names() {
    let columns: Vec<String> = Column::iter().map(|item| item.to_string()).collect();

    assert_eq!(
        columns,
        vec![
            "id",
            "username",
            "firstName",
            "middleName",
            "lAsTnAmE",
            "ordersCount",
        ]
    );
}
