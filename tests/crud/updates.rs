pub use super::*;
use pgorm::set;
use pgorm::{Error, Paginate, query::*};
use uuid::Uuid;

pub async fn test_update_cake(db: &DatabaseConnection) {
    let seaside_bakery = bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    };
    let bakery_insert_res = Insert::one(seaside_bakery)
        .exec_returning_pk(db)
        .await
        .expect("could not insert bakery");

    let mud_cake = cake::ActiveModel {
        name: set("Mud Cake"),
        price: set(rust_dec(10.25)),
        gluten_free: set(false),
        serial: set(Uuid::new_v4()),
        bakery_id: set(Some(bakery_insert_res)),
        ..Default::default()
    };

    let cake_insert_res = Insert::one(mud_cake)
        .exec_returning_pk(db)
        .await
        .expect("could not insert cake");

    let cake: Option<cake::Model> = Cake::find_by_id(cake_insert_res)
        .one_opt(db)
        .await
        .expect("could not find cake");

    assert!(cake.is_some());
    let cake_model = cake.unwrap();
    assert_eq!(cake_model.name, "Mud Cake");
    assert_eq!(cake_model.price, rust_dec(10.25));
    assert!(!cake_model.gluten_free);

    let large_number = "1234_5678_9012.3456".parse().unwrap();

    let mut cake_am: cake::ActiveModel = cake_model.into();
    cake_am.name = set("Extra chocolate mud cake");
    cake_am.price = set(large_number);

    let _cake_update_res: cake::Model = cake_am.update(db).await.expect("could not update cake");

    let cake: Option<cake::Model> = Cake::find_by_id(cake_insert_res)
        .one_opt(db)
        .await
        .expect("could not find cake");
    let cake_model = cake.unwrap();
    assert_eq!(cake_model.name, "Extra chocolate mud cake");
    assert_eq!(cake_model.price, large_number);
    assert!(!cake_model.gluten_free);
}

pub async fn test_update_bakery(db: &DatabaseConnection) {
    let seaside_bakery = bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    };
    let bakery_insert_res = Insert::one(seaside_bakery)
        .exec_returning_pk(db)
        .await
        .expect("could not insert bakery");

    let bakery: Option<bakery::Model> = Bakery::find_by_id(bakery_insert_res)
        .one_opt(db)
        .await
        .expect("could not find bakery");

    assert!(bakery.is_some());
    let bakery_model = bakery.unwrap();
    assert_eq!(bakery_model.name, "SeaSide Bakery");
    assert!((bakery_model.profit_margin - 10.40).abs() < f64::EPSILON);

    let mut bakery_am: bakery::ActiveModel = bakery_model.into();
    bakery_am.name = set("SeaBreeze Bakery");
    bakery_am.profit_margin = set(12.00);

    let _bakery_update_res: bakery::Model =
        bakery_am.update(db).await.expect("could not update bakery");

    let bakery: Option<bakery::Model> = Bakery::find_by_id(bakery_insert_res)
        .one_opt(db)
        .await
        .expect("could not find bakery");
    let bakery_model = bakery.unwrap();
    assert_eq!(bakery_model.name, "SeaBreeze Bakery");
    assert!((bakery_model.profit_margin - 12.00).abs() < f64::EPSILON);
}

pub async fn test_update_deleted_customer(db: &DatabaseConnection) {
    let init_n_customers = Customer::find().count(db).await.unwrap();

    let customer = customer::ActiveModel {
        name: set("John"),
        notes: set(None),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert customer");

    assert_eq!(
        Customer::find().count(db).await.unwrap(),
        init_n_customers + 1
    );

    let customer_id = customer.id;

    let _ = customer.delete(db).await;
    assert_eq!(Customer::find().count(db).await.unwrap(), init_n_customers);

    let customer = customer::ActiveModel {
        id: set(customer_id),
        name: set("John 2"),
        ..Default::default()
    };

    let customer_update_res = customer.update(db).await;

    assert_eq!(customer_update_res, Err(Error::RecordNotFound));

    assert_eq!(Customer::find().count(db).await.unwrap(), init_n_customers);

    let customer: Option<customer::Model> = Customer::find_by_id(customer_id)
        .one_opt(db)
        .await
        .expect("could not find customer");

    assert_eq!(customer, None);
}
