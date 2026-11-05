#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
use pgorm::{ManyRows, OneRow, TryInsertResult, ValueHolder, entity::prelude::*, types::ToSql};
pub use pgorm_query::{Expr, Query, QueryBuilder, Values};
use serde_json::json;

fn holders(values: Values) -> Vec<ValueHolder> {
    values.into_iter().map(ValueHolder).collect()
}

fn params(holders: &[ValueHolder]) -> Vec<&(dyn ToSql + Sync)> {
    holders.iter().map(|v| v as &(dyn ToSql + Sync)).collect()
}

// [spec:pgorm:def:exec.cursor.binding+6/test]    every built statement value is
// wrapped in `ValueHolder` for binding — here `String`, `Double` and `Int`
#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    use bakery::*;

    let ctx = TestContext::new("returning_tests").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    let mut insert = Query::insert();
    insert
        .into_table(Entity)
        .columns([Column::Name, Column::ProfitMargin])
        .values_panic(["Bakery Shop".into(), 0.5.into()]);

    let mut update = Query::update();
    update
        .table(Entity)
        .values([
            (Column::Name, "Bakery Shop".into()),
            (Column::ProfitMargin, 0.5.into()),
        ])
        .and_where(Column::Id.eq(1));

    let columns = [Column::Id, Column::Name, Column::ProfitMargin];
    let returning = Query::returning().exprs(columns.into_iter().map(|c| c.into_returning_expr()));

    insert.returning(returning.clone());
    let (sql, values) = insert.build();
    let bound = holders(values);
    let insert_res = db.query_one(&sql, &params(&bound)).await?;
    let _id: i32 = insert_res.try_get("id")?;
    let _name: String = insert_res.try_get("name")?;
    let _profit_margin: f64 = insert_res.try_get("profit_margin")?;

    update.returning(returning.clone());
    let (sql, values) = update.build();
    let bound = holders(values);
    let update_res = db.query_one(&sql, &params(&bound)).await?;
    let _id: i32 = update_res.try_get("id")?;
    let _name: String = update_res.try_get("name")?;
    let _profit_margin: f64 = update_res.try_get("profit_margin")?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.crud.update+7/test]    `UpdateMany::exec_returning_models`
// returns every updated model, and refuses an update with no column to set
// rather than passing an empty `Vec` off as the rows it updated
#[pgorm_macros::test]
async fn update_many() {
    pub use common::{TestContext, features::*};
    use edit_log::*;

    let run = || async {
        let ctx = TestContext::new("returning_tests_update_many").await;
        create_tables(&ctx.db).await?;
        let db = ctx.db.get().await?;

        Insert::one(
            Model {
                id: 1,
                action: "before_save".into(),
                values: json!({ "id": "unique-id-001" }),
            }
            .into_active_model(),
        )
        .exec(&db)
        .await?;

        Insert::one(
            Model {
                id: 2,
                action: "before_save".into(),
                values: json!({ "id": "unique-id-002" }),
            }
            .into_active_model(),
        )
        .exec(&db)
        .await?;

        Insert::one(
            Model {
                id: 3,
                action: "before_save".into(),
                values: json!({ "id": "unique-id-003" }),
            }
            .into_active_model(),
        )
        .exec(&db)
        .await?;

        assert_eq!(
            Entity::find().all(&db).await?,
            [
                Model {
                    id: 1,
                    action: "before_save".into(),
                    values: json!({ "id": "unique-id-001" }),
                },
                Model {
                    id: 2,
                    action: "before_save".into(),
                    values: json!({ "id": "unique-id-002" }),
                },
                Model {
                    id: 3,
                    action: "before_save".into(),
                    values: json!({ "id": "unique-id-003" }),
                },
            ]
        );

        // Update many with returning
        assert_eq!(
            Update::many(Entity)
                .col_expr(
                    Column::Values,
                    Expr::value(json!({ "remarks": "save log" }))
                )
                .filter(Column::Action.eq("before_save"))
                .exec_returning_models(&db)
                .await?,
            [
                Model {
                    id: 1,
                    action: "before_save".into(),
                    values: json!({ "remarks": "save log" }),
                },
                Model {
                    id: 2,
                    action: "before_save".into(),
                    values: json!({ "remarks": "save log" }),
                },
                Model {
                    id: 3,
                    action: "before_save".into(),
                    values: json!({ "remarks": "save log" }),
                },
            ]
        );

        // Nothing to set: the rows the filter matches are not the rows this
        // update changed, so there is no `Vec` to hand back.
        assert_eq!(
            Update::many(Entity)
                .filter(Column::Action.eq("before_save"))
                .exec_returning_models(&db)
                .await,
            Err(Error::NothingToSet)
        );

        drop(db);
        ctx.delete().await;

        Result::<(), Error>::Ok(())
    };

    run().await.unwrap();
}

fn bakery_model(name: &str, margin: f64) -> bakery::ActiveModel {
    bakery::ActiveModel {
        name: set(name),
        profit_margin: set(margin),
        ..Default::default()
    }
}

// [spec:pgorm:sem:exec.crud.insert-returning+3/test]    `exec_returning_model`
// decodes a full-column RETURNING (and fails with RecordNotFound when the
// insert matched nothing); `exec` reports rows affected
#[pgorm_macros::test]
async fn insert_returning_modes() -> Result<(), Error> {
    use pgorm_query::OnConflict;

    let ctx = TestContext::new("returning_tests_insert_modes").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    // Every entity column is returned, so the server-assigned primary key is
    // part of the decoded model.
    let inserted = Insert::one(bakery_model("SeaSide Bakery", 10.4))
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        inserted,
        bakery::Model {
            id: 1,
            name: "SeaSide Bakery".to_owned(),
            profit_margin: 10.4,
        }
    );

    // `exec` yields only the rows-affected count.
    let affected = Insert::many([
        bakery_model("Top Bakery", 15.0),
        bakery_model("Third Bakery", 20.5),
        bakery_model("Fourth Bakery", 5.25),
    ])
    .exec(&db)
    .await?;
    assert_eq!(affected, 3);
    assert_eq!(Bakery::find().count(&db).await?, 4);
    assert_eq!(
        Insert::one(bakery_model("Fifth Bakery", 1.0))
            .exec(&db)
            .await?,
        1
    );

    // Nothing to decode: a conflicting insert returns no row, and
    // `SelectorRaw::one_opt` reporting `None` becomes RecordNotFound.
    let conflicted = Insert::one(bakery::ActiveModel {
        id: set(inserted.id),
        name: set("Duplicate Bakery"),
        profit_margin: set(0.5),
    })
    .on_conflict(OnConflict::do_nothing())
    .exec_returning_model(&db)
    .await;
    assert_eq!(conflicted, Err(Error::RecordNotFound));

    // The same conflict is not an error for `exec`: no row
    // was written, so the count is zero.
    let skipped = Insert::one(bakery::ActiveModel {
        id: set(inserted.id),
        name: set("Duplicate Bakery"),
        profit_margin: set(0.5),
    })
    .on_conflict(OnConflict::do_nothing())
    .exec(&db)
    .await?;
    assert_eq!(skipped, 0);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.crud.try-insert+4/test]    `TryInsertResult` across all
// three executions: Empty without touching the database, Inserted on success,
// Conflicted from a skipped `ON CONFLICT` insert, and any other error
// propagating
// [spec:pgorm:sem:query.build.insert.empty-failsafe+5/test]    the same three
// entry points reading the one recorded empty state: an insert over an empty
// iterator and an insert of an all-NotSet model both return Empty with the
// database left untouched
#[pgorm_macros::test]
async fn try_insert_result_variants() -> Result<(), Error> {
    use pgorm::TryInsertResult;
    use pgorm_query::OnConflict;

    let ctx = TestContext::new("returning_tests_try_insert").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    let empty = || std::iter::empty::<bakery::ActiveModel>();

    // A column-less insert statement short-circuits to Empty on each entry
    // point, without issuing SQL.
    assert!(matches!(
        Insert::many(empty())
            .on_empty_do_nothing()
            .exec(&db)
            .await?,
        TryInsertResult::Empty
    ));
    assert!(matches!(
        Insert::many(empty())
            .on_empty_do_nothing()
            .exec(&db)
            .await?,
        TryInsertResult::Empty
    ));
    assert!(matches!(
        Insert::many(empty())
            .on_empty_do_nothing()
            .exec_returning_models(&db)
            .await?,
        TryInsertResult::Empty
    ));
    assert_eq!(Bakery::find().all(&db).await?, []);

    // A model that sets no column reaches the same state, so it reads as Empty
    // on all three entry points too rather than inserting a row of defaults.
    let blank = || bakery::ActiveModel {
        ..Default::default()
    };

    assert!(matches!(
        Insert::one(blank()).on_empty_do_nothing().exec(&db).await?,
        TryInsertResult::Empty
    ));
    assert!(matches!(
        Insert::one(blank()).on_empty_do_nothing().exec(&db).await?,
        TryInsertResult::Empty
    ));
    assert!(matches!(
        Insert::one(blank())
            .on_empty_do_nothing()
            .exec_returning_model(&db)
            .await?,
        TryInsertResult::Empty
    ));
    assert_eq!(Bakery::find().all(&db).await?, []);

    // Success wraps the inner result.
    let inserted = Insert::one(bakery_model("SeaSide Bakery", 10.4))
        .on_empty_do_nothing()
        .exec_returning_model(&db)
        .await?;
    match inserted {
        TryInsertResult::Inserted(model) => assert_eq!(model.name, "SeaSide Bakery"),
        other => panic!("unexpected result: {other:?}"),
    }

    let counted = Insert::one(bakery_model("Top Bakery", 15.0))
        .on_empty_do_nothing()
        .exec(&db)
        .await?;
    assert!(matches!(counted, TryInsertResult::Inserted(1)));

    let duplicate = || bakery::ActiveModel {
        id: set(1),
        name: set("Duplicate Bakery"),
        profit_margin: set(0.5),
    };
    let on_conflict = OnConflict::do_nothing;

    // An `ON CONFLICT DO NOTHING` clause that skips the row reads as Conflicted
    // on every entry point: `exec_returning_pk` from RecordNotInserted, `exec`
    // from a zero rows-affected count, and `exec_returning_model` from the
    // absent RETURNING row.
    assert!(matches!(
        Insert::one(duplicate())
            .on_conflict(on_conflict())
            .on_empty_do_nothing()
            .exec_returning_pk(&db)
            .await?,
        TryInsertResult::Conflicted
    ));
    assert!(matches!(
        Insert::one(duplicate())
            .on_conflict(on_conflict())
            .on_empty_do_nothing()
            .exec(&db)
            .await?,
        TryInsertResult::Conflicted
    ));
    assert!(matches!(
        Insert::one(duplicate())
            .on_conflict(on_conflict())
            .on_empty_do_nothing()
            .exec_returning_model(&db)
            .await?,
        TryInsertResult::Conflicted
    ));

    // With no conflict clause to attribute it to, a failing insert propagates
    // its error rather than being folded into a variant.
    assert!(matches!(
        Insert::one(duplicate())
            .on_empty_do_nothing()
            .exec_returning_model(&db)
            .await,
        Err(Error::Postgres(_))
    ));

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.crud.update+7/test]    the empty-SET refusal of both
// `UpdateMany` terminals — including through the reachable input, a model read
// back from the database and handed straight to `set` — against the `Ok(0)` of
// a `WHERE` that matched nothing, and the no-op read of
// `UpdateOne::exec_returning_model`
// [spec:pgorm:def:error.model+13/test]    `Error::NothingToSet` has a
// construction site, and it is this one
#[pgorm_macros::test]
async fn update_noop_and_record_check() -> Result<(), Error> {
    use pgorm::ActiveValue::Unchanged;

    let ctx = TestContext::new("returning_tests_update_noop").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    let seaside = Insert::one(bakery_model("SeaSide Bakery", 10.4))
        .exec_returning_model(&db)
        .await?;

    // Nothing to SET is refused rather than reported as a count: the filter
    // matches an existing row, so `Ok(0)` would be a lie about the WHERE.
    assert_eq!(
        Update::many(Bakery)
            .filter(bakery::Column::Id.eq(seaside.id))
            .exec(&db)
            .await,
        Err(Error::NothingToSet)
    );

    // The reachable spelling of the same input: a model read back from the
    // database is all `Unchanged`, and `set` skips those fields.
    let round_tripped = Bakery::find_by_id(seaside.id).one(&db).await?.into_active();
    assert_eq!(
        Update::many(Bakery)
            .set(round_tripped.clone())
            .filter(bakery::Column::Id.eq(seaside.id))
            .exec(&db)
            .await,
        Err(Error::NothingToSet)
    );

    // Both terminals answer it the same way.
    assert_eq!(
        Update::many(Bakery)
            .set(round_tripped)
            .filter(bakery::Column::Id.eq(seaside.id))
            .exec_returning_models(&db)
            .await,
        Err(Error::NothingToSet)
    );

    // Nothing was written by any of the three.
    assert_eq!(Bakery::find_by_id(seaside.id).one(&db).await?, seaside);

    // With something to SET the same call reports the rows it changed.
    let applied = Update::many(Bakery)
        .col_expr(bakery::Column::ProfitMargin, Expr::value(12.5_f64))
        .filter(bakery::Column::Id.eq(seaside.id))
        .exec(&db)
        .await?;
    assert_eq!(applied, 1);

    // An update whose WHERE matches nothing is `Ok(0)`, not an error: the count
    // is the whole answer, and the caller decides what zero means.
    let missed = Update::many(Bakery)
        .col_expr(bakery::Column::Name, Expr::value("Nowhere Bakery"))
        .filter(bakery::Column::Id.eq(9999))
        .exec(&db)
        .await?;
    assert_eq!(missed, 0);

    // On `UpdateOne`'s no-op path nothing is written; the current model is
    // re-read under the statement's own `WHERE` instead.
    let refetched = Update::one(bakery::ActiveModel {
        id: Unchanged(seaside.id),
        name: Unchanged(seaside.name.clone()),
        profit_margin: Unchanged(seaside.profit_margin),
    })?
    .exec_returning_model(&db)
    .await?;
    assert_eq!(
        refetched,
        bakery::Model {
            id: seaside.id,
            name: seaside.name.clone(),
            profit_margin: 12.5,
        }
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.crud.update+7/test]    the no-op read runs under the
// statement's complete `WHERE`, so a caller-added guard predicate excludes an
// unchanged row exactly as it would a changed one
#[pgorm_macros::test]
async fn no_change_update_honors_added_filters() -> Result<(), Error> {
    use pgorm::ActiveValue::Unchanged;

    let ctx = TestContext::new("returning_tests_noop_filters").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    let seaside = Insert::one(bakery_model("SeaSide Bakery", 10.4))
        .exec_returning_model(&db)
        .await?;

    let unchanged = || bakery::ActiveModel {
        id: Unchanged(seaside.id),
        name: Unchanged(seaside.name.clone()),
        profit_margin: Unchanged(seaside.profit_margin),
    };

    // A guard the row satisfies: the no-op read returns the model.
    let guarded = Update::one(unchanged())?
        .filter(bakery::Column::ProfitMargin.eq(seaside.profit_margin))
        .exec_returning_model(&db)
        .await?;
    assert_eq!(guarded, seaside);

    // A guard the row fails: `Error::RecordNotFound`, exactly as a real
    // `UPDATE` matching zero rows would report.
    let missed = Update::one(unchanged())?
        .filter(bakery::Column::Name.eq("Someone Else's Bakery"))
        .exec_returning_model(&db)
        .await;
    assert_eq!(missed.unwrap_err(), Error::RecordNotFound);

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:def:exec.crud.exec-result+1/test]    `ExecResult` is a transparent
// wrapper over the rows-affected `u64` and exposes nothing else
#[pgorm_macros::test]
async fn exec_result_is_a_transparent_row_count() {
    use pgorm::ExecResult;

    assert_eq!(
        std::mem::size_of::<ExecResult>(),
        std::mem::size_of::<u64>()
    );
    assert_eq!(
        std::mem::align_of::<ExecResult>(),
        std::mem::align_of::<u64>()
    );

    // The only accessor is `rows_affected`; it carries no primary key.
    let accessor: fn(&ExecResult) -> u64 = ExecResult::rows_affected;
    let _ = accessor;
}

/// Every CRUD terminal's return shape, pinned by explicit annotation.
///
/// Compiled but never run: the assertions are the type annotations, so a
/// terminal that is renamed, dropped, or changes shape fails the build. A live
/// round trip would prove less — these are claims about the surface, not about
/// the database.
// [spec:pgorm:req:exec.crud.exec-vocabulary+2/test]    `exec` is a count on
// every builder that has one; each returning form names what it yields, one
// for a single row and a `Vec` for a batch
#[allow(dead_code)]
async fn exec_terminals_name_their_shape<C: ConnectionTrait>(db: &C) -> Result<(), Error> {
    let insert = || Insert::<bakery::ActiveModel, OneRow>::one(bakery_model("Shape", 1.0));

    let _rows: u64 = insert().exec(db).await?;
    let _pk: i32 = insert().exec_returning_pk(db).await?;
    let _model: bakery::Model = insert().exec_returning_model(db).await?;

    let try_insert = || insert().on_empty_do_nothing();

    let _t_rows: TryInsertResult<u64> = try_insert().exec(db).await?;
    let _t_pk: TryInsertResult<i32> = try_insert().exec_returning_pk(db).await?;
    let _t_model: TryInsertResult<bakery::Model> = try_insert().exec_returning_model(db).await?;

    let batch = || Insert::<bakery::ActiveModel, ManyRows>::many([bakery_model("Batch", 1.0)]);

    let _b_rows: u64 = batch().exec(db).await?;
    let _pks: Vec<i32> = batch().exec_returning_pks(db).await?;
    let _models: Vec<bakery::Model> = batch().exec_returning_models(db).await?;

    let try_batch = || batch().on_empty_do_nothing();

    let _tb_rows: TryInsertResult<u64> = try_batch().exec(db).await?;
    let _tb_pks: TryInsertResult<Vec<i32>> = try_batch().exec_returning_pks(db).await?;
    let _tb_models: TryInsertResult<Vec<bakery::Model>> =
        try_batch().exec_returning_models(db).await?;

    // `UpdateOne` offers only the model form: there is no count-shaped answer
    // to updating one keyed row, so there is no `exec` to misread as one.
    let _updated: bakery::Model = Update::one(bakery_model("Shape", 2.0))?
        .exec_returning_model(db)
        .await?;

    // Both many-row terminals need a column to set: with none they report
    // `Error::NothingToSet` rather than a shape.
    let update_many =
        || Update::many(bakery::Entity).col_expr(bakery::Column::Name, "Shape".into());

    let _update_rows: u64 = update_many().exec(db).await?;
    let _updated_many: Vec<bakery::Model> = update_many().exec_returning_models(db).await?;

    // Deletes have no returning form at all: the count is the whole answer.
    let _delete_one_rows: u64 = Delete::one(bakery_model("Shape", 3.0))?.exec(db).await?;
    let _delete_many_rows: u64 = Delete::many(bakery::Entity).exec(db).await?;

    Ok(())
}
