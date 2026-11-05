use pgorm::entity::prelude::*;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
pub enum Outlet {
    WebShop,
    PopUpShop,
    ClosedShop,
}
