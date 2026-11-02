use pgorm_migration::prelude::*;

#[derive(MigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // Replace the sample below with your own migration scripts.
    async fn up(&self, tx: &DatabaseTransaction<'_>) -> Result<(), Error> {
        let table = Table::create(Post::Table)
            .if_not_exists()
            .col(
                ColumnDef::new(Post::Id)
                    .integer()
                    .not_null()
                    .auto_increment(),
            ).primary_key(Post::Id)
            .col(ColumnDef::new(Post::Title).string().not_null())
            .col(ColumnDef::new(Post::Text).string().not_null())
            .to_owned();
        tx.execute(&table.to_string(), &[]).await?;

        Ok(())
    }
}

#[derive(SqlName)]
enum Post {
    Table,
    Id,
    Title,
    Text,
}
