use super::*;
use crate::common::setup::create_table;
use pgorm::{Connection, DatabasePool, error::*, pgorm_query};
use pgorm_query::{ColumnDef, ForeignKey, ForeignKeyAction, Name, Table, TableKey};

pub async fn create_tables(db: &DatabasePool) -> Result<(), Error> {
    let db = &db.get().await?;

    create_bakery_table(db).await?;
    create_baker_table(db).await?;
    create_customer_table(db).await?;
    create_order_table(db).await?;
    create_cake_table(db).await?;
    create_cakes_bakers_table(db).await?;
    create_lineitem_table(db).await?;

    Ok(())
}

pub async fn create_bakery_table<C>(db: &C) -> Result<u64, Error>
where
    C: Connection,
{
    let stmt = Table::create(bakery::Entity)
        .col(
            ColumnDef::new(bakery::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(bakery::Column::Id)
        .col(ColumnDef::new(bakery::Column::Name).string().not_null())
        .col(
            ColumnDef::new(bakery::Column::ProfitMargin)
                .double()
                .not_null(),
        )
        .to_owned();

    create_table(db, &stmt, Bakery).await
}

pub async fn create_baker_table<C>(db: &C) -> Result<u64, Error>
where
    C: Connection,
{
    let stmt = Table::create(baker::Entity)
        .col(
            ColumnDef::new(baker::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(baker::Column::Id)
        .col(ColumnDef::new(baker::Column::Name).string().not_null())
        .col(
            ColumnDef::new(baker::Column::ContactDetails)
                .json()
                .not_null(),
        )
        .col(ColumnDef::new(baker::Column::BakeryId).integer())
        .foreign_key(
            ForeignKey::create(
                baker::Entity,
                baker::Column::BakeryId,
                bakery::Entity,
                bakery::Column::Id,
            )
            .name(Name::runtime("fk-baker-bakery_id"))
            .on_delete(ForeignKeyAction::SetNull)
            .on_update(ForeignKeyAction::Cascade)
            .to_owned(),
        )
        .to_owned();

    create_table(db, &stmt, Baker).await
}

pub async fn create_customer_table<C>(db: &C) -> Result<u64, Error>
where
    C: Connection,
{
    let stmt = Table::create(customer::Entity)
        .col(
            ColumnDef::new(customer::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(customer::Column::Id)
        .col(ColumnDef::new(customer::Column::Name).string().not_null())
        .col(ColumnDef::new(customer::Column::Notes).text())
        .to_owned();

    create_table(db, &stmt, Customer).await
}

pub async fn create_order_table<C>(db: &C) -> Result<u64, Error>
where
    C: Connection,
{
    let stmt = Table::create(order::Entity)
        .col(
            ColumnDef::new(order::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(order::Column::Id)
        .col(
            ColumnDef::new(order::Column::Total)
                .decimal_len(16, 4)
                .not_null(),
        )
        .col(ColumnDef::new(order::Column::BakeryId).integer().not_null())
        .col(
            ColumnDef::new(order::Column::CustomerId)
                .integer()
                .not_null(),
        )
        .col(
            ColumnDef::new(order::Column::PlacedAt)
                .timestamp()
                .not_null(),
        )
        .foreign_key(
            ForeignKey::create(
                order::Entity,
                order::Column::BakeryId,
                bakery::Entity,
                bakery::Column::Id,
            )
            .name(Name::runtime("fk-order-bakery_id"))
            .to_owned(),
        )
        .foreign_key(
            ForeignKey::create(
                order::Entity,
                order::Column::CustomerId,
                customer::Entity,
                customer::Column::Id,
            )
            .name(Name::runtime("fk-order-customer_id"))
            .on_delete(ForeignKeyAction::Cascade)
            .on_update(ForeignKeyAction::Cascade)
            .to_owned(),
        )
        .to_owned();

    create_table(db, &stmt, Order).await
}

pub async fn create_lineitem_table<C>(db: &C) -> Result<u64, Error>
where
    C: Connection,
{
    let stmt = Table::create(lineitem::Entity)
        .col(
            ColumnDef::new(lineitem::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(lineitem::Column::Id)
        .col(
            ColumnDef::new(lineitem::Column::Price)
                .decimal_len(16, 4)
                .not_null(),
        )
        .col(
            ColumnDef::new(lineitem::Column::Quantity)
                .integer()
                .not_null(),
        )
        .col(
            ColumnDef::new(lineitem::Column::OrderId)
                .integer()
                .not_null(),
        )
        .col(
            ColumnDef::new(lineitem::Column::CakeId)
                .integer()
                .not_null(),
        )
        .foreign_key(
            ForeignKey::create(
                lineitem::Entity,
                lineitem::Column::OrderId,
                order::Entity,
                order::Column::Id,
            )
            .name(Name::runtime("fk-lineitem-order_id"))
            .on_delete(ForeignKeyAction::Cascade)
            .on_update(ForeignKeyAction::Cascade)
            .to_owned(),
        )
        .foreign_key(
            ForeignKey::create(
                lineitem::Entity,
                lineitem::Column::CakeId,
                cake::Entity,
                cake::Column::Id,
            )
            .name(Name::runtime("fk-lineitem-cake_id"))
            .to_owned(),
        )
        .to_owned();

    create_table(db, &stmt, Lineitem).await
}

pub async fn create_cakes_bakers_table<C>(db: &C) -> Result<u64, Error>
where
    C: Connection,
{
    let stmt = Table::create(cakes_bakers::Entity)
        .col(
            ColumnDef::new(cakes_bakers::Column::CakeId)
                .integer()
                .not_null(),
        )
        .col(
            ColumnDef::new(cakes_bakers::Column::BakerId)
                .integer()
                .not_null(),
        )
        .primary_key(
            TableKey::new(cakes_bakers::Column::CakeId)
                .name(Name::runtime("pk-cakes_bakers"))
                .col(cakes_bakers::Column::BakerId),
        )
        .foreign_key(
            ForeignKey::create(
                cakes_bakers::Entity,
                cakes_bakers::Column::CakeId,
                cake::Entity,
                cake::Column::Id,
            )
            .name(Name::runtime("fk-cakes_bakers-cake_id"))
            .on_delete(ForeignKeyAction::Cascade)
            .on_update(ForeignKeyAction::Cascade)
            .to_owned(),
        )
        .foreign_key(
            ForeignKey::create(
                cakes_bakers::Entity,
                cakes_bakers::Column::BakerId,
                baker::Entity,
                baker::Column::Id,
            )
            .name(Name::runtime("fk-cakes_bakers-baker_id"))
            .to_owned(),
        )
        .to_owned();

    create_table(db, &stmt, CakesBakers).await
}

pub async fn create_cake_table<C>(db: &C) -> Result<u64, Error>
where
    C: Connection,
{
    let stmt = Table::create(cake::Entity)
        .col(
            ColumnDef::new(cake::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(cake::Column::Id)
        .col(ColumnDef::new(cake::Column::Name).string().not_null())
        .col(
            ColumnDef::new(cake::Column::Price)
                .decimal_len(16, 4)
                .not_null(),
        )
        .col(ColumnDef::new(cake::Column::BakeryId).integer())
        .foreign_key(
            ForeignKey::create(
                cake::Entity,
                cake::Column::BakeryId,
                bakery::Entity,
                bakery::Column::Id,
            )
            .name(Name::runtime("fk-cake-bakery_id"))
            .on_delete(ForeignKeyAction::SetNull)
            .on_update(ForeignKeyAction::Cascade)
            .to_owned(),
        )
        .col(
            ColumnDef::new(cake::Column::GlutenFree)
                .boolean()
                .not_null(),
        )
        .col(ColumnDef::new(cake::Column::Serial).uuid().not_null())
        .to_owned();

    create_table(db, &stmt, Cake).await
}
