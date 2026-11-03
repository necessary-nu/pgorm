#![allow(unused_imports, dead_code)]

//! A disjoint entity's declaration: what its types cannot tell, refused by
//! `check_disjoint` as an error naming the table and column, and its kind
//! type against a live server, where a base row of a kind the enum does not
//! list cannot be written.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test disjoint_declare_tests`

pub mod common;
pub use common::TestContext;

use pgorm::tests_cfg::vehicle::{self, Vehicle, VehicleKind};
use pgorm::{ConnectionTrait, Error, Schema, check_disjoint, entity::prelude::*};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

/// A kind entity keyed as every base below is, by one `i32` it is given.
pub mod given_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "given_key")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub note: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A base whose kind column may be null.
pub mod nullable_base {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "nullable_base")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: Option<super::NullableKind>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = nullable_base::Entity)]
pub enum Nullable {
    Given(given_key::Model),
}

/// A base whose kind column is text.
pub mod text_base {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "text_base", schema_name = "depot")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = text_base::Entity)]
pub enum Texted {
    Given(given_key::Model),
}

/// A base whose kind column is `vehicle_kind`, beside an enum naming its own
/// kind type `vehicle_kind` with other values.
pub mod fleet_base {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "fleet_base")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: pgorm::tests_cfg::vehicle::VehicleKind,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = fleet_base::Entity, enum_name = "vehicle_kind")]
pub enum Fleet {
    Car(given_key::Model),
    Boat,
}

/// A base whose kind column is computed rather than written.
pub mod stamped_base {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "stamped_base")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(generated_stored = Expr::val("given"))]
        pub kind: super::StampedKind,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = stamped_base::Entity)]
pub enum Stamped {
    Given(given_key::Model),
}

/// A kind entity whose one-column key is drawn from a sequence, as an `i32`
/// key is unless it says otherwise.
pub mod serial_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "serial_key")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A kind entity whose key is an identity.
pub mod identity_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "identity_key")]
    pub struct Model {
        #[pgorm(primary_key, identity_by_default)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A kind entity whose key is computed from its other columns.
pub mod computed_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "computed_key")]
    pub struct Model {
        #[pgorm(primary_key, generated_stored = Expr::col(Column::Seed).add(1))]
        pub id: i32,
        pub seed: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A kind entity whose key the database fills when a write leaves it out.
pub mod default_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "default_key")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false, default_value = 0)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A base for the key cases: its kind column holds `KeyedKind`.
pub mod keyed_base {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "keyed_base")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: super::KeyedKind,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The kind with a sequence-drawn key comes after a sound one and a unit one,
/// so its refusal shows the check visits every kind, not only the first.
#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = keyed_base::Entity)]
pub enum Keyed {
    Given(given_key::Model),
    Unit,
    Serial(serial_key::Model),
}

/// An identity key and a generated one, each under `keyed_base`: their kind
/// types are named and valued as `Keyed`'s, and a kind column's type is
/// compared by name, schema and values.
#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = keyed_base::Entity, enum_name = "keyed_kind")]
pub enum IdentityKeyed {
    #[pgorm(value = "given")]
    Identity(identity_key::Model),
    Unit,
    #[pgorm(value = "serial")]
    Other,
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = keyed_base::Entity, enum_name = "keyed_kind")]
pub enum DefaultKeyed {
    #[pgorm(value = "given")]
    Defaulted(default_key::Model),
    Unit,
    #[pgorm(value = "serial")]
    Other,
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = keyed_base::Entity, enum_name = "keyed_kind")]
pub enum ComputedKeyed {
    #[pgorm(value = "given")]
    Computed(computed_key::Model),
    Unit,
    #[pgorm(value = "serial")]
    Other,
}

/// A two-column base, and a kind whose key's second column is an identity.
pub mod tenant_base {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "tenant_base")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub tenant_id: i32,
        #[pgorm(primary_key, identity)]
        pub id: i64,
        pub kind: super::TenantedKind,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod tenant_part {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "tenant_part")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub tenant: i32,
        #[pgorm(primary_key, identity)]
        pub base: i64,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = tenant_base::Entity)]
pub enum Tenanted {
    Part(tenant_part::Model),
}

fn refusal<D: Disjoint>() -> String {
    match check_disjoint::<D>() {
        Err(Error::Type(message)) => message,
        other => panic!("expected the declaration refused as a type error, got {other:?}"),
    }
}

// [spec:pgorm:def:disjoint.declare+3/test]    a nullable kind column, a generated one, one of text and one of another enum the kind type's name spells are each refused as `Error::Type` naming the table and column
#[test]
fn kind_column_not_of_kind_type_is_refused() {
    assert_eq!(
        refusal::<Nullable>(),
        "the kind column `nullable_base`.`kind` is nullable: every base row of a disjoint \
         entity has a kind, so the column is `NOT NULL`"
    );
    assert_eq!(
        refusal::<Stamped>(),
        "the kind column `stamped_base`.`kind` is a generated column: a write sets a base \
         row's kind to the kind it writes, so no expression computes it"
    );
    assert_eq!(
        refusal::<Texted>(),
        "the kind column `depot.text_base`.`kind` is `String(None)`, not the kind type \
         `texted_kind` ('given')"
    );
    assert_eq!(
        refusal::<Fleet>(),
        "the kind column `fleet_base`.`kind` is `vehicle_kind` ('car', 'truck', 'bike'), not \
         the kind type `vehicle_kind` ('car', 'boat')"
    );
}

// [spec:pgorm:def:disjoint.declare+3/test]    a kind entity whose key is drawn from a sequence, an identity, a generated column, a default, or a composite key's identity part is refused as `Error::Type` naming the kind's table and column
#[test]
fn kind_key_the_database_generates_is_refused() {
    assert_eq!(
        refusal::<Keyed>(),
        "`serial_key`.`id`, the key of the kind `serial`, is drawn from a sequence \
         (`auto_increment`): a kind's row takes its key from its base row, so the database \
         does not generate it"
    );
    assert_eq!(
        refusal::<IdentityKeyed>(),
        "`identity_key`.`id`, the key of the kind `given`, is an identity: a kind's row takes \
         its key from its base row, so the database does not generate it"
    );
    assert_eq!(
        refusal::<DefaultKeyed>(),
        "`default_key`.`id`, the key of the kind `given`, is given a default: a kind's row \
         takes its key from its base row, so the database does not generate it"
    );
    assert_eq!(
        refusal::<ComputedKeyed>(),
        "`computed_key`.`id`, the key of the kind `given`, is a generated column: a kind's row \
         takes its key from its base row, so the database does not generate it"
    );
    assert_eq!(
        refusal::<Tenanted>(),
        "`tenant_part`.`base`, the key of the kind `part`, is an identity: a kind's row takes \
         its key from its base row, so the database does not generate it"
    );
}

// [spec:pgorm:def:disjoint.declare+3/test]    a base whose own key the database generates is sound, as is one whose kind column is the kind type
#[test]
fn a_generated_base_key_passes_the_check() {
    assert!(vehicle::PrimaryKey::auto_increment());
    assert!(check_disjoint::<Vehicle>().is_ok());
}

// [spec:pgorm:def:disjoint.declare+3/test]    the kind type created on a live server has the variants' values in declaration order, a base row of a listed kind round-trips through the kind column, and one of a kind the enum does not list cannot be written
#[pgorm_macros::test]
async fn kind_type_holds_only_the_listed_kinds_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_declare").await;
    let db = ctx.db.get().await?;

    let schema = Schema::new();
    let kind_type = schema.create_enum_from_active_enum::<VehicleKind>()?;
    db.execute(&kind_type.to_string(), &[]).await?;
    db.execute(
        &schema
            .create_table_from_entity(vehicle::Entity)?
            .to_string(),
        &[],
    )
    .await?;

    let labels: Vec<String> = db
        .query_all(
            "SELECT enumlabel::text AS label FROM pg_enum \
             WHERE enumtypid = 'vehicle_kind'::regtype ORDER BY enumsortorder",
            &[],
        )
        .await?
        .iter()
        .map(|row| row.get("label"))
        .collect();
    assert_eq!(labels, ["car", "truck", "bike"]);

    let bike = vehicle::ActiveModel {
        kind: set(VehicleKind::Bike),
        name: set("roadster"),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    let read = vehicle::Entity::find_by_id(bike.id).one(&db).await?;
    assert_eq!(read.kind, VehicleKind::Bike);

    let boat = db
        .execute(
            "INSERT INTO vehicle (kind, name) VALUES ('boat', 'dinghy')",
            &[],
        )
        .await;
    match boat {
        Err(Error::Postgres(error)) => assert_eq!(
            error.code(),
            Some(&SqlState::INVALID_TEXT_REPRESENTATION),
            "{error}"
        ),
        other => panic!("expected a kind the enum does not list refused, got {other:?}"),
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}
