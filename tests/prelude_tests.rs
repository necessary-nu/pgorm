//! The prelude as a contract.
//!
//! This file imports `pgorm::entity::prelude::*` and, apart from the entity
//! module it queries, nothing else from pgorm. Everything below therefore
//! compiles only while the prelude still carries the names it uses — dropping
//! one is a break for callers who never named it, so it is a break here.

use std::num::NonZeroU64;

use pgorm::entity::prelude::*;
use pgorm::tests_cfg::cake;

#[derive(Debug, PartialEq, Eq, FromQueryResult)]
struct NameOnly {
    name: String,
}

/// The connection vocabulary, named in signatures rather than called: a pool, a
/// checked-out connection, a transaction, and the two traits over them.
async fn _connection_vocabulary(
    pool: &DatabasePool,
    conn: &mut DatabaseConnection,
) -> Result<(), Error> {
    let _: DatabaseConnection = pool.get().await?;
    TransactionTrait::transaction(conn, async |txn: &mut DatabaseTransaction<'_>| {
        txn.execute("SELECT 1", &[]).await
    })
    .await
    .map(|_: u64| ())
    .map_err(|error| Error::Custom(error.to_string()))
}

/// The paginator and the loader are traits, so their methods are unreachable
/// unless the prelude carries them.
async fn _paged<C: ConnectionTrait>(db: &C) -> Result<u64, Error> {
    cake::Entity::find()
        .paginate(db, NonZeroU64::new(10).expect("10 is not zero"))
        .num_pages()
        .await
}

// [spec:pgorm:def:entity.prelude+10/test]    a query built, an active model set, a
// statement decoded and a column enumerated, with the prelude as the only import
#[test]
fn prelude_carries_what_a_query_needs() {
    // `QuerySelect`, `QueryFilter`, `QueryOrder` and `QueryTrait`: every method
    // in this chain belongs to a trait, and `as_query` to a fourth.
    let sql = cake::Entity::find()
        .select_only()
        .column(cake::Column::Name)
        .filter(cake::Column::Id.gt(1))
        .filter(Condition::any().add(cake::Column::Name.like("Ch%")))
        .order_by_asc(cake::Column::Id)
        .limit(5)
        .as_query()
        .to_string();
    assert_eq!(
        sql,
        r#"SELECT "cake"."name" FROM "cake" WHERE "cake"."id" > 1 AND "cake"."name" LIKE 'Ch%' ORDER BY "cake"."id" ASC LIMIT 5"#
    );

    // `JoinType` names the join a relation is walked through.
    let joined = cake::Entity::find()
        .join(JoinType::InnerJoin, cake::Relation::Fruit.def())
        .as_query()
        .to_string();
    assert!(joined.contains("INNER JOIN"), "{joined}");

    // The `ActiveValue` vocabulary: the three variants, and the `Value` a set
    // one carries.
    let active = cake::ActiveModel {
        id: NotSet,
        name: Set("Cheesecake".to_owned()),
    };
    assert!(active.id.is_not_set());
    assert_eq!(
        active.name.clone().into_value(),
        Some(Value::from("Cheesecake"))
    );
    let unchanged: ActiveValue<i32> = Unchanged(1);
    assert!(unchanged.is_unchanged());

    // A model converts back to an active model, and an active model tries back
    // to a model.
    let model = cake::Model {
        id: 1,
        name: "Cheesecake".to_owned(),
    };
    let round: cake::ActiveModel = model.clone().into_active_model();
    assert_eq!(round.try_into_model().expect("every field is set"), model);

    // `DecodeSelect` names the row type once, on the statement.
    let _decoded = cake::Entity::find()
        .select_only()
        .column(cake::Column::Name)
        .into_query()
        .into_model::<NameOnly>();

    // `Iterable` is what makes a column enum enumerable.
    assert_eq!(
        cake::Column::iter()
            .map(|c| c.as_str().to_owned())
            .collect::<Vec<_>>(),
        ["id".to_owned(), "name".to_owned()]
    );
}

// [spec:pgorm:def:entity.prelude+10/test]    `Func`, the vocabulary an entity's
// `default_expr` names a function default in, with the prelude as the only import
#[test]
fn prelude_carries_the_function_vocabulary() {
    let sql = cake::Entity::find()
        .select_only()
        .expr(Func::uuidv7())
        .as_query()
        .to_string();
    assert_eq!(sql, r#"SELECT UUIDV7() FROM "cake""#);
}

/// An entity whose interval columns carry a qualifier, written as pgorm-codegen
/// writes one, with the prelude as the only import.
mod interval_columns {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "interval_columns")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth)))]
        pub months: Interval,
        #[pgorm(column_type = ColumnType::Interval(IntervalSpec::Any(Some(IntervalPrecision::P3))))]
        pub precise: Interval,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// [spec:pgorm:def:entity.prelude+10/test]    the interval vocabulary: a field's
// `Interval` and the `IntervalSpec`, `PgInterval` and `IntervalPrecision` its
// `column_type` names resolve through the glob
#[test]
fn prelude_carries_the_interval_vocabulary() {
    assert_eq!(
        interval_columns::Column::Months.def().get_column_type(),
        &ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth))
    );
    assert_eq!(
        interval_columns::Column::Precise.def().get_column_type(),
        &ColumnType::Interval(IntervalSpec::Any(IntervalPrecision::new(3)))
    );
}
