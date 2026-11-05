use pgorm::{DdlStatement, Schema};
use pgorm_migration::prelude::*;

pub mod car {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "car")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub seats: i16,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod van {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "van")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub doors: i16,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The fleet as it was declared first: cars, each with a row of its own, and
/// bikes.
pub mod fleet {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "fleet")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: super::FleetKind,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Eq, pgorm::Disjoint)]
#[pgorm(base = fleet::Entity)]
pub enum Fleet {
    Car(car::Model),
    Bike,
}

/// The fleet as it is declared once vans, each with a row of its own, join
/// it.
pub mod vanned_fleet {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "fleet")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: super::VannedFleetKind,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Eq, pgorm::Disjoint)]
#[pgorm(base = vanned_fleet::Entity, enum_name = "fleet_kind")]
pub enum VannedFleet {
    Car(car::Model),
    Van(van::Model),
    Bike,
}

/// A migration running the statements `statements` writes, named by hand.
pub struct Statements {
    name: &'static str,
    statements: fn() -> Result<Vec<DdlStatement>, Error>,
    commits_after: bool,
}

impl MigrationName for Statements {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Statements {
    async fn up(&self, tx: &DatabaseTransaction<'_>) -> Result<(), Error> {
        for statement in (self.statements)()? {
            tx.execute(&statement.to_string(), &[]).await?;
        }
        Ok(())
    }

    fn commits_after(&self) -> bool {
        self.commits_after
    }
}

fn van_step(index: usize) -> Result<Vec<DdlStatement>, Error> {
    let change = Schema::new().add_disjoint_kind::<VannedFleet>(VannedFleetKind::Van)?;
    Ok(change
        .steps()
        .get(index)
        .map(|step| step.statements().to_vec())
        .unwrap_or_default())
}

const CREATED: Statements = Statements {
    name: "m20240401_000001_create_fleet",
    statements: || Schema::new().create_disjoint::<Fleet>(),
    commits_after: false,
};

/// The fleet's tables.
pub struct Created;

#[async_trait::async_trait]
impl MigratorTrait for Created {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CREATED)]
    }
}

/// [`Created`], then the van kind added in two migrations, one per step of
/// the change, the first asking to be committed so the second can use the
/// value it adds.
pub struct Vanned;

#[async_trait::async_trait]
impl MigratorTrait for Vanned {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(CREATED),
            Box::new(Statements {
                name: "m20240401_000002_add_van_kind",
                statements: || van_step(0),
                commits_after: true,
            }),
            Box::new(Statements {
                name: "m20240401_000003_add_van_table",
                statements: || van_step(1),
                commits_after: false,
            }),
        ]
    }
}

/// The fleet's history as it was frozen when each change was made: the
/// tables `Fleet` declared, then the van kind `VannedFleet` adds, a file for
/// each step of the change, the first committing after it runs.
pub struct Frozen;

#[async_trait::async_trait]
impl MigratorTrait for Frozen {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(sql_migration!(
                "tests/sql/frozen/m20240401_000001_create_fleet.sql"
            )),
            Box::new(sql_migration!(
                "tests/sql/frozen/m20240401_000002_add_van_1.sql"
            )),
            Box::new(sql_migration!(
                "tests/sql/frozen/m20240401_000002_add_van_2.sql"
            )),
        ]
    }
}

/// The same history generated as each migration runs, from the declaration
/// as it stands once vans have joined: the first migration already creates
/// the kind the second adds, so a database built from nothing cannot replay
/// it.
pub struct Recomputed;

#[async_trait::async_trait]
impl MigratorTrait for Recomputed {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(Statements {
                name: "m20240401_000001_create_fleet",
                statements: || Schema::new().create_disjoint::<VannedFleet>(),
                commits_after: false,
            }),
            Box::new(Statements {
                name: "m20240401_000002_add_van_kind",
                statements: || van_step(0),
                commits_after: true,
            }),
            Box::new(Statements {
                name: "m20240401_000003_add_van_table",
                statements: || van_step(1),
                commits_after: false,
            }),
        ]
    }
}
