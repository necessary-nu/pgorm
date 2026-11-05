use pgorm::entity::prelude::*;

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
pub enum Outlet {
    WebShop,
    PopUpShop,
    ClosedShop,
}
