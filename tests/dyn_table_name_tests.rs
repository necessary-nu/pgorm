#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, features::*, setup::*};
use pgorm::entity::prelude::*;
use pgorm::pipeline::Pipeline;
use pgorm_query::{Expr, Query};
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("dyn_table_name_tests").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    dyn_table_name_lazy_static(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:exec.crud.update+7/test]    `UpdateMany::exec` returns the
// rows-affected count
// [spec:pgorm:sem:exec.crud.delete+1/test]    `DeleteMany::exec` returns the
// rows-affected count
// [spec:pgorm:req:entity.traits.entity-name+6/test]    a statement over an instance
// whose table name differs from the default's reads the instance's table, under the
// default's name its columns are qualified by
// [spec:pgorm:sem:pipeline.qualify+4/test]    and so does a pipeline over one
pub async fn dyn_table_name_lazy_static(db: &DatabaseConnection) -> Result<(), Error> {
    use dyn_table_name_lazy_static::*;

    for i in 1..=2 {
        let entity = Entity {
            table_name: TableName::from_str_truncate(format!("dyn_table_name_lazy_static_{}", i)),
        };

        let model = Model {
            id: 1,
            name: "1st Row".into(),
        };
        let insert = Insert::one_in(entity, model.clone().into_active_model());
        assert_eq!(
            insert.as_query().to_string(),
            format!(
                r#"INSERT INTO "dyn_table_name_lazy_static_{i}" AS "dyn_table_name_lazy_static" ("id", "name") VALUES (1, '1st Row')"#
            )
        );
        assert_eq!(insert.exec_returning_pk(db).await?, 1);

        let select = Entity::find_in(entity);
        assert_eq!(
            select.as_query().to_string(),
            format!(
                r#"SELECT "dyn_table_name_lazy_static"."id", "dyn_table_name_lazy_static"."name" FROM "dyn_table_name_lazy_static_{i}" AS "dyn_table_name_lazy_static""#
            )
        );
        assert_eq!(select.clone().one(db).await?, model.clone());
        assert_eq!(Entity::find_by_id_in(entity, 1).one(db).await?, model);

        let piped = Pipeline::from(entity).filter(pgorm::pipeline::ExprOps::eq(Column::Id, 1));
        assert_eq!(
            piped.clone().into_sql().map_err(Error::from)?.0,
            format!(
                "SELECT * FROM dyn_table_name_lazy_static_{i} AS dyn_table_name_lazy_static \
                 WHERE id = 1"
            )
        );
        assert_eq!(
            piped.all::<Model, _>(db).await?,
            std::slice::from_ref(&model)
        );

        // Nothing to set: the row is read back from the instance's table.
        assert_eq!(
            Update::one_in(entity, model.clone().into_active_model())?
                .exec_returning_model(db)
                .await?,
            model
        );

        let update = Update::many(entity).set(ActiveModel {
            name: set("1st Row (edited)"),
            ..model.clone().into_active_model()
        });
        assert_eq!(update.exec(db).await?, 1);

        let edited = Model {
            id: 1,
            name: "1st Row (edited)".into(),
        };
        assert_eq!(select.clone().one(db).await?, edited);
        assert_eq!(
            Update::one_in(
                entity,
                ActiveModel {
                    id: set(1),
                    name: set("1st Row (again)"),
                }
            )?
            .exec_returning_model(db)
            .await?
            .name,
            "1st Row (again)"
        );

        let delete = Delete::many(entity).filter(Column::Id.eq(1));
        assert_eq!(delete.exec(db).await?, 1);
        assert_eq!(select.clone().one_opt(db).await?, None);

        Insert::many_in(entity, [model.clone().into_active_model()])
            .exec(db)
            .await?;
        assert_eq!(Delete::one_in(entity, model.clone())?.exec(db).await?, 1);
        Insert::one_in(entity, model.clone().into_active_model())
            .exec(db)
            .await?;
        assert_eq!(Entity::delete_by_id_in(entity, 1).exec(db).await?, 1);
        assert_eq!(select.one_opt(db).await?, None);
    }

    Ok(())
}
