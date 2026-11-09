use pgorm_migration::prelude::*;

/// A migration whose name and checksum are pinned by hand rather than derived,
/// so the same name can be presented twice, or presented again reporting a
/// different checksum.
pub struct Pinned {
    pub name: &'static str,
    pub checksum: Option<&'static str>,
}

impl MigrationName for Pinned {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Pinned {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        tx.execute(&format!("CREATE TABLE \"{}\" (id INT)", self.name), &[])
            .await?;
        Ok(())
    }

    fn checksum(&self) -> Option<String> {
        self.checksum.map(str::to_owned)
    }
}

/// The name two of the migrators below deliberately share.
pub const REPEATED: &str = "m20240101_000001_pinned";

/// Two migrations answering `name()` with the same string.
pub struct Duplicated;

#[async_trait::async_trait]
impl MigratorTrait for Duplicated {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(Pinned {
                name: REPEATED,
                checksum: None,
            }),
            Box::new(Pinned {
                name: "m20240101_000002_other",
                checksum: None,
            }),
            Box::new(Pinned {
                name: REPEATED,
                checksum: None,
            }),
        ]
    }
}

/// One migration reporting a checksum.
pub struct Checksummed;

#[async_trait::async_trait]
impl MigratorTrait for Checksummed {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Pinned {
            name: REPEATED,
            checksum: Some("digest-one"),
        })]
    }
}

/// The same migration, edited: same name, different checksum.
pub struct Edited;

#[async_trait::async_trait]
impl MigratorTrait for Edited {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Pinned {
            name: REPEATED,
            checksum: Some("digest-two"),
        })]
    }
}

/// The same migration reporting no checksum at all.
pub struct Unchecked;

#[async_trait::async_trait]
impl MigratorTrait for Unchecked {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Pinned {
            name: REPEATED,
            checksum: None,
        })]
    }
}
