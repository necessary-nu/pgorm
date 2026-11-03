use pgorm::entity::prelude::*;
use pgorm::tests_cfg::car;

#[derive(Debug, ActiveEnum)]
pub enum GivenValue {
    #[pgorm(string_value = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
    Long,
}

#[derive(Debug, ActiveEnum)]
pub enum VariantName {
    LabeLabelLabelLabelLabelLabelLabelLabelLabelLabelLabel,
}

#[derive(Debug, ActiveEnum)]
pub enum Multibyte {
    #[pgorm(string_value = "éaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
    Accented,
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum KindValue {
    #[pgorm(value = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
    Car(car::Model),
}

#[derive(Disjoint)]
#[pgorm(base = pgorm::tests_cfg::vehicle::Entity)]
pub enum KindName {
    Car(car::Model),
    LabeLabelLabelLabelLabelLabelLabelLabelLabelLabelLabel,
}

fn main() {}
