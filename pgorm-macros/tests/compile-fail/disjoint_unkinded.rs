use pgorm::entity::prelude::*;
use pgorm::tests_cfg::web_shop;

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::shop::Entity, unkinded, kind = pgorm::tests_cfg::shop::Column::Name)]
pub enum KindColumn {
    WebShop(web_shop::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::shop::Entity, unkinded, enum_name = "shop_kind")]
pub enum EnumName {
    WebShop(web_shop::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::shop::Entity, schema_name = "retail", unkinded)]
pub enum SchemaName {
    WebShop(web_shop::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::shop::Entity, unkinded, unkinded)]
pub enum UnkindedTwice {
    WebShop(web_shop::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::shop::Entity, unkinded)]
pub enum Valued {
    #[pgorm(value = "online")]
    WebShop(web_shop::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::shop::Entity, unkinded)]
pub enum TwoUnits {
    WebShop(web_shop::Model),
    Dormant,
    Closed,
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::shop::Entity, unkinded)]
pub enum NoTables {
    Dormant,
}

fn main() {}
