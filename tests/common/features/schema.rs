use super::*;
use crate::common::setup::{create_enum, create_table, create_table_without_asserts};
use pgorm::{ConnectionMut, DatabasePool, EntityName, Schema, error::*, pgorm_query};
use pgorm_query::{
    ColumnDef, ColumnType, ForeignKeyCreateStatement, IntoName, Name, StringLen, alias,
    extension::Type,
};

pub async fn create_tables(db: &DatabasePool) -> Result<(), Error> {
    let db = &db.get().await?;

    create_log_table(db).await?;
    create_metadata_table(db).await?;
    create_repository_table(db).await?;
    create_self_join_table(db).await?;
    create_byte_primary_key_table(db).await?;
    create_transaction_log_table(db).await?;

    let create_enum_stmts = {
        let schema = Schema::new();
        let enum_create_stmt = Type::create(alias("tea"))
            .values(["EverydayTea", "BreakfastTea"])
            .to_owned();
        assert_eq!(
            enum_create_stmt.to_string(),
            schema.create_enum_from_active_enum::<Tea>()?.to_string()
        );
        vec![enum_create_stmt]
    };
    create_enum(db, &create_enum_stmts, ActiveEnum).await?;

    create_active_enum_table(db).await?;
    create_active_enum_child_table(db).await?;
    create_insert_default_table(db).await?;
    create_pi_table(db).await?;
    create_uuid_fmt_table(db).await?;
    create_edit_log_table(db).await?;
    create_teas_table(db).await?;
    create_binary_table(db).await?;
    create_bits_table(db).await?;
    create_dyn_table_name_lazy_static_table(db).await?;
    create_value_type_table(db).await?;

    create_json_vec_table(db).await?;
    create_json_struct_table(db).await?;
    create_json_string_vec_table(db).await?;
    create_json_struct_vec_table(db).await?;

    create_value_type_postgres_table(db).await?;
    create_collection_table(db).await?;
    create_event_trigger_table(db).await?;
    create_categories_table(db).await?;

    Ok(())
}

pub async fn create_log_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let stmt = pgorm_query::Table::create(applog::Entity)
        .comment("app logs")
        .col(
            ColumnDef::new(applog::Column::Id)
                .integer()
                .not_null()
                .comment("ID")
                .auto_increment(),
        )
        .primary_key(applog::Column::Id)
        .col(
            ColumnDef::new(applog::Column::Action)
                .string()
                .not_null()
                .comment("action"),
        )
        .col(
            ColumnDef::new(applog::Column::Json)
                .json()
                .not_null()
                .comment("action data"),
        )
        .col(
            ColumnDef::new(applog::Column::CreatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .comment("create time"),
        )
        .to_owned();

    create_table(db, &stmt, Applog).await
}

pub async fn create_metadata_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let stmt = pgorm_query::Table::create(metadata::Entity)
        .col(ColumnDef::new(metadata::Column::Uuid).uuid().not_null())
        .primary_key(metadata::Column::Uuid)
        .col(ColumnDef::new(metadata::Column::Type).string().not_null())
        .col(ColumnDef::new(metadata::Column::Key).string().not_null())
        .col(ColumnDef::new(metadata::Column::Value).string().not_null())
        .col(ColumnDef::new_with_type(metadata::Column::Bytes, ColumnType::Bytea).not_null())
        .col(ColumnDef::new(metadata::Column::Date).date())
        .col(ColumnDef::new(metadata::Column::Time).time())
        .to_owned();

    create_table(db, &stmt, Metadata).await
}

pub async fn create_repository_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let stmt = pgorm_query::Table::create(repository::Entity)
        .col(ColumnDef::new(repository::Column::Id).string().not_null())
        .primary_key(repository::Column::Id)
        .col(
            ColumnDef::new(repository::Column::Owner)
                .string()
                .not_null(),
        )
        .col(ColumnDef::new(repository::Column::Name).string().not_null())
        .col(ColumnDef::new(repository::Column::Description).string())
        .to_owned();

    create_table(db, &stmt, Repository).await
}

pub async fn create_self_join_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let stmt = pgorm_query::Table::create(self_join::Entity)
        .col(ColumnDef::new(self_join::Column::Uuid).uuid().not_null())
        .primary_key(self_join::Column::Uuid)
        .col(ColumnDef::new(self_join::Column::UuidRef).uuid())
        .col(ColumnDef::new(self_join::Column::Time).time())
        .foreign_key(
            ForeignKeyCreateStatement::new(
                SelfJoin,
                self_join::Column::UuidRef,
                SelfJoin,
                self_join::Column::Uuid,
            )
            .name(Name::runtime("fk-self_join-uuid_ref"))
            .to_owned(),
        )
        .to_owned();

    create_table(db, &stmt, SelfJoin).await
}

pub async fn create_byte_primary_key_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let mut primary_key_col = ColumnDef::new(byte_primary_key::Column::Id);
    primary_key_col.bytea();

    let stmt = pgorm_query::Table::create(byte_primary_key::Entity)
        .col(primary_key_col.not_null())
        .primary_key(byte_primary_key::Column::Id)
        .col(
            ColumnDef::new(byte_primary_key::Column::Value)
                .string()
                .not_null(),
        )
        .to_owned();

    create_table_without_asserts(db, &stmt).await
}

pub async fn create_active_enum_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let create_table_stmt = pgorm_query::Table::create(active_enum::Entity.table_ref())
        .col(
            ColumnDef::new(active_enum::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(active_enum::Column::Id)
        .col(ColumnDef::new(active_enum::Column::Category).string_len(1))
        .col(ColumnDef::new(active_enum::Column::Color).integer())
        .col(
            ColumnDef::new(active_enum::Column::Tea)
                .enumeration(TeaEnum, [TeaVariant::EverydayTea, TeaVariant::BreakfastTea]),
        )
        .to_owned();

    create_table(db, &create_table_stmt, ActiveEnum).await
}

pub async fn create_active_enum_child_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let create_table_stmt = pgorm_query::Table::create(active_enum_child::Entity.table_ref())
        .col(
            ColumnDef::new(active_enum_child::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(active_enum_child::Column::Id)
        .col(
            ColumnDef::new(active_enum_child::Column::ParentId)
                .integer()
                .not_null(),
        )
        .col(ColumnDef::new(active_enum_child::Column::Category).string_len(1))
        .col(ColumnDef::new(active_enum_child::Column::Color).integer())
        .col(
            ColumnDef::new(active_enum_child::Column::Tea)
                .enumeration(TeaEnum, [TeaVariant::EverydayTea, TeaVariant::BreakfastTea]),
        )
        .foreign_key(
            ForeignKeyCreateStatement::new(
                ActiveEnumChild,
                active_enum_child::Column::ParentId,
                ActiveEnum,
                active_enum::Column::Id,
            )
            .name(Name::runtime("fk-active_enum_child-active_enum"))
            .to_owned(),
        )
        .to_owned();

    create_table(db, &create_table_stmt, ActiveEnumChild).await
}

pub async fn create_transaction_log_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let stmt = pgorm_query::Table::create(transaction_log::Entity)
        .col(
            ColumnDef::new(transaction_log::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(transaction_log::Column::Id)
        .col(
            ColumnDef::new(transaction_log::Column::Date)
                .date()
                .not_null(),
        )
        .col(
            ColumnDef::new(transaction_log::Column::Time)
                .time()
                .not_null(),
        )
        .col(
            ColumnDef::new(transaction_log::Column::DateTime)
                .timestamp()
                .not_null(),
        )
        .col(
            ColumnDef::new(transaction_log::Column::DateTimeTz)
                .timestamp_with_time_zone()
                .not_null(),
        )
        .to_owned();

    create_table(db, &stmt, TransactionLog).await
}

pub async fn create_insert_default_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let create_table_stmt = pgorm_query::Table::create(insert_default::Entity.table_ref())
        .col(
            ColumnDef::new(insert_default::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(insert_default::Column::Id)
        .to_owned();

    create_table(db, &create_table_stmt, InsertDefault).await
}

pub async fn create_json_vec_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let create_table_stmt = pgorm_query::Table::create(json_vec::Entity.table_ref())
        .col(
            ColumnDef::new(json_vec::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(json_vec::Column::Id)
        .col(ColumnDef::new(json_vec::Column::StrVec).json())
        .to_owned();

    create_table(db, &create_table_stmt, JsonVec).await
}

pub async fn create_json_struct_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let stmt = pgorm_query::Table::create(json_struct::Entity)
        .col(
            ColumnDef::new(json_struct::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(json_struct::Column::Id)
        .col(ColumnDef::new(json_struct::Column::Json).json().not_null())
        .col(
            ColumnDef::new(json_struct::Column::JsonValue)
                .json()
                .not_null(),
        )
        .col(ColumnDef::new(json_struct::Column::JsonValueOpt).json())
        .to_owned();

    create_table(db, &stmt, JsonStruct).await
}

pub async fn create_json_string_vec_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let create_table_stmt = pgorm_query::Table::create(JsonStringVec.table_ref())
        .col(
            ColumnDef::new(json_vec_derive::json_string_vec::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(json_vec_derive::json_string_vec::Column::Id)
        .col(ColumnDef::new(json_vec_derive::json_string_vec::Column::StrVec).json())
        .to_owned();

    create_table(db, &create_table_stmt, JsonStringVec).await
}

pub async fn create_json_struct_vec_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let create_table_stmt = pgorm_query::Table::create(JsonStructVec.table_ref())
        .col(
            ColumnDef::new(json_vec_derive::json_struct_vec::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(json_vec_derive::json_struct_vec::Column::Id)
        .col(
            ColumnDef::new(json_vec_derive::json_struct_vec::Column::StructVec)
                .json_binary()
                .not_null(),
        )
        .to_owned();

    create_table(db, &create_table_stmt, JsonStructVec).await
}

pub async fn create_collection_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    db.execute("CREATE EXTENSION IF NOT EXISTS citext", &[])
        .await?;

    let stmt = pgorm_query::Table::create(collection::Entity)
        .col(
            ColumnDef::new(collection::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(collection::Column::Id)
        .col(
            ColumnDef::new(collection::Column::Name)
                .named(alias("citext"))
                .not_null(),
        )
        .col(
            ColumnDef::new(collection::Column::Integers)
                .array(pgorm_query::ColumnType::Integer)
                .not_null(),
        )
        .col(
            ColumnDef::new(collection::Column::IntegersOpt).array(pgorm_query::ColumnType::Integer),
        )
        .col(
            ColumnDef::new(collection::Column::Teas)
                .array(pgorm_query::ColumnType::Enum {
                    schema: None,
                    name: TeaEnum.into_name(),
                    variants: vec![
                        TeaVariant::EverydayTea.into_name(),
                        TeaVariant::BreakfastTea.into_name(),
                    ],
                })
                .not_null(),
        )
        .col(
            ColumnDef::new(collection::Column::TeasOpt).array(pgorm_query::ColumnType::Enum {
                schema: None,
                name: TeaEnum.into_name(),
                variants: vec![
                    TeaVariant::EverydayTea.into_name(),
                    TeaVariant::BreakfastTea.into_name(),
                ],
            }),
        )
        .col(
            ColumnDef::new(collection::Column::Colors)
                .array(pgorm_query::ColumnType::Integer)
                .not_null(),
        )
        .col(ColumnDef::new(collection::Column::ColorsOpt).array(pgorm_query::ColumnType::Integer))
        .col(
            ColumnDef::new(collection::Column::Uuid)
                .array(pgorm_query::ColumnType::Uuid)
                .not_null(),
        )
        .col(
            ColumnDef::new(collection::Column::UuidHyphenated)
                .array(pgorm_query::ColumnType::Uuid)
                .not_null(),
        )
        .to_owned();

    create_table(db, &stmt, Collection).await
}

pub async fn create_pi_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let stmt = pgorm_query::Table::create(pi::Entity)
        .col(
            ColumnDef::new(pi::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(pi::Column::Id)
        .col(
            ColumnDef::new(pi::Column::Decimal)
                .decimal_len(11, 10)
                .not_null(),
        )
        .col(ColumnDef::new(pi::Column::DecimalOpt).decimal_len(11, 10))
        .to_owned();

    create_table(db, &stmt, Pi).await
}

pub async fn create_event_trigger_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let stmt = pgorm_query::Table::create(event_trigger::Entity)
        .col(
            ColumnDef::new(event_trigger::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(event_trigger::Column::Id)
        .col(
            ColumnDef::new(event_trigger::Column::Events)
                .array(pgorm_query::ColumnType::String(StringLen::None))
                .not_null(),
        )
        .to_owned();

    create_table(db, &stmt, EventTrigger).await
}

pub async fn create_uuid_fmt_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let stmt = pgorm_query::Table::create(uuid_fmt::Entity)
        .col(
            ColumnDef::new(uuid_fmt::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(uuid_fmt::Column::Id)
        .col(ColumnDef::new(uuid_fmt::Column::Uuid).uuid().not_null())
        .col(
            ColumnDef::new(uuid_fmt::Column::UuidBraced)
                .uuid()
                .not_null(),
        )
        .col(
            ColumnDef::new(uuid_fmt::Column::UuidHyphenated)
                .uuid()
                .not_null(),
        )
        .col(
            ColumnDef::new(uuid_fmt::Column::UuidSimple)
                .uuid()
                .not_null(),
        )
        .col(ColumnDef::new(uuid_fmt::Column::UuidUrn).uuid().not_null())
        .to_owned();

    create_table(db, &stmt, UuidFmt).await
}

pub async fn create_edit_log_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let stmt = pgorm_query::Table::create(edit_log::Entity)
        .col(
            ColumnDef::new(edit_log::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(edit_log::Column::Id)
        .col(ColumnDef::new(edit_log::Column::Action).string().not_null())
        .col(ColumnDef::new(edit_log::Column::Values).json().not_null())
        .to_owned();

    create_table(db, &stmt, EditLog).await
}

pub async fn create_teas_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let create_table_stmt = pgorm_query::Table::create(teas::Entity.table_ref())
        .col(
            ColumnDef::new(teas::Column::Id)
                .enumeration(TeaEnum, [TeaVariant::EverydayTea, TeaVariant::BreakfastTea])
                .not_null(),
        )
        .primary_key(teas::Column::Id)
        .col(ColumnDef::new(teas::Column::Category).string_len(1))
        .col(ColumnDef::new(teas::Column::Color).integer())
        .to_owned();

    create_table(db, &create_table_stmt, Teas).await
}

pub async fn create_categories_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let create_table_stmt = pgorm_query::Table::create(categories::Entity.table_ref())
        .col(ColumnDef::new(categories::Column::Id).integer().not_null())
        .primary_key(categories::Column::Id)
        .col(
            ColumnDef::new(categories::Column::Categories)
                .array(ColumnType::String(StringLen::N(1))),
        )
        .to_owned();

    create_table(db, &create_table_stmt, Categories).await
}

pub async fn create_binary_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let create_table_stmt = pgorm_query::Table::create(binary::Entity.table_ref())
        .col(
            ColumnDef::new(binary::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(binary::Column::Id)
        .col(ColumnDef::new(binary::Column::Binary).bytea().not_null())
        .col(ColumnDef::new(binary::Column::Binary10).bytea().not_null())
        .col(
            ColumnDef::new(binary::Column::VarBinary16)
                .bytea()
                .not_null(),
        )
        .to_owned();

    create_table(db, &create_table_stmt, Binary).await
}

pub async fn create_bits_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let create_table_stmt = pgorm_query::Table::create(bits::Entity.table_ref())
        .col(
            ColumnDef::new(bits::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(bits::Column::Id)
        .col(ColumnDef::new(bits::Column::Bit0).bit(None).not_null())
        .col(ColumnDef::new(bits::Column::Bit1).bit(Some(1)).not_null())
        .col(ColumnDef::new(bits::Column::Bit8).bit(Some(8)).not_null())
        .col(ColumnDef::new(bits::Column::Bit16).bit(Some(16)).not_null())
        .col(ColumnDef::new(bits::Column::Bit32).bit(Some(32)).not_null())
        .col(ColumnDef::new(bits::Column::Bit64).bit(Some(64)).not_null())
        .to_owned();

    create_table(db, &create_table_stmt, Bits).await
}

pub async fn create_dyn_table_name_lazy_static_table<C>(db: &C) -> Result<(), Error>
where
    C: ConnectionMut,
{
    use dyn_table_name_lazy_static::*;

    let entities = [
        Entity {
            table_name: TableName::from_str_truncate("dyn_table_name_lazy_static_1"),
        },
        Entity {
            table_name: TableName::from_str_truncate("dyn_table_name_lazy_static_2"),
        },
    ];
    for entity in entities {
        let create_table_stmt = pgorm_query::Table::create(entity.table_ref())
            .col(
                ColumnDef::new(Column::Id)
                    .integer()
                    .not_null()
                    .auto_increment(),
            )
            .primary_key(Column::Id)
            .col(ColumnDef::new(Column::Name).string().not_null())
            .to_owned();

        create_table(db, &create_table_stmt, entity).await?;
    }

    Ok(())
}

pub async fn create_value_type_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let general_stmt = pgorm_query::Table::create(value_type::value_type_general::Entity)
        .col(
            ColumnDef::new(value_type::value_type_general::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(value_type::value_type_general::Column::Id)
        .col(
            ColumnDef::new(value_type::value_type_general::Column::Number)
                .integer()
                .not_null(),
        )
        .to_owned();

    create_table(db, &general_stmt, value_type::value_type_general::Entity).await
}
pub async fn create_value_type_postgres_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionMut,
{
    let postgres_stmt = pgorm_query::Table::create(value_type::value_type_pg::Entity)
        .col(
            ColumnDef::new(value_type::value_type_pg::Column::Id)
                .integer()
                .not_null()
                .auto_increment(),
        )
        .primary_key(value_type::value_type_pg::Column::Id)
        .col(
            ColumnDef::new(value_type::value_type_pg::Column::Number)
                .integer()
                .not_null(),
        )
        .col(
            ColumnDef::new(json_vec::Column::StrVec)
                .array(pgorm_query::ColumnType::String(StringLen::None))
                .not_null(),
        )
        .to_owned();

    create_table(db, &postgres_stmt, value_type::value_type_pg::Entity).await
}
