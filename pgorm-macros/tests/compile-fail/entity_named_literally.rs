use pgorm::entity::prelude::*;

// A literal name is the composite derive's to write; the entity-trait derive
// stands beside a hand-written `EntityName`, for a name computed at run time.
#[derive(Copy, Clone, Default, Debug, EntityTrait)]
#[pgorm(table_name = "fruit")]
pub struct Entity;

// The column-naming keys are the composite derive's too.
mod cased {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, EntityTrait)]
    #[pgorm(rename_all = "camelCase")]
    pub struct Entity;
}

// `entity` names the entity a model or relation belongs to; this struct is it.
mod pointed {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, EntityTrait)]
    #[pgorm(entity = Fruit)]
    pub struct Entity;
}

// A key no derive reads.
mod misspelt {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, EntityTrait)]
    #[pgorm(colum = FruitColumn)]
    pub struct Entity;
}

fn main() {}
