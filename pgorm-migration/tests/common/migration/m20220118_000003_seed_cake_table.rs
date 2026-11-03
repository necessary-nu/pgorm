use pgorm_migration::pgorm::entity::*;
use pgorm_migration::pgorm::set;
use pgorm_migration::prelude::*;

#[derive(MigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, tx: &DatabaseTransaction<'_>) -> Result<(), Error> {
        cake::ActiveModel {
            name: set("Cheesecake"),
            ..Default::default()
        }
        .insert(tx)
        .await?;

        Ok(())
    }
}

mod cake {
    use pgorm_migration::pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "cake")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}
