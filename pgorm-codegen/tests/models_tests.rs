//! The declared-model writers over `tests/models/schema.sql`: what each
//! writes is the module its binding commits and imports, byte for byte, and
//! what a declared model cannot say is refused by name. `PGORM_MODELS_WRITE=1`
//! writes the committed modules again.

use pgorm_codegen::{
    ModelWriterOptions, PYTHON_MODULE, TYPESCRIPT_MODULE, TableIdent, UnkindedDisjoint,
    python_models_from_sql, typescript_models_from_sql,
};
use std::path::Path;

const SCHEMA: &str = include_str!("models/schema.sql");

/// The module the binding at `committed` (from the crate's directory)
/// commits, which `written` must be, or which it becomes under
/// `PGORM_MODELS_WRITE=1`.
fn holds(committed: &str, written: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(committed);
    if std::env::var_os("PGORM_MODELS_WRITE").is_some() {
        std::fs::write(&path, written).expect("the committed module is writable");
        return;
    }
    let held = std::fs::read_to_string(&path).expect("the committed module is readable");
    pretty_assertions::assert_eq!(held, written, "{committed}");
}

fn one(output: pgorm_codegen::WriterOutput, name: &str) -> String {
    let [file] = <[_; 1]>::try_from(output.files).expect("one file");
    assert_eq!(file.name, name);
    file.content
}

// [spec:pgorm:def:codegen.models/test]    python_models_from_sql writes the one module models.py, which is the module pgorm-python commits and imports
// [spec:pgorm:sem:codegen.models.columns/test]    every column type a declared model takes, an enum, a domain over an enum, money, a created range, arrays, defaults, an identity and a generated column, each declared with its kind and the column type and fill it states
// [spec:pgorm:sem:codegen.models.relations/test]    each foreign key a belongs_to and its inverse a has_many, and the junction a related path
// [spec:pgorm:sem:codegen.models.disjoint/test]    the tables create_disjoint writes are a Disjoint over the base's and kinds' models
// [spec:pgorm:sem:codegen.models.one-of/test]    the CHECK create_one_of writes is a OneOf named for it
// [spec:pgorm:req:codegen.models.layout/test]    the module is laid out as black lays it out
#[test]
fn python_models_are_the_committed_module() {
    let output = python_models_from_sql(SCHEMA, ModelWriterOptions::default()).expect("generates");
    holds(
        "../pgorm-python/tests/models_from_sql/models.py",
        &one(output, PYTHON_MODULE),
    );
}

// [spec:pgorm:def:codegen.models/test]    typescript_models_from_sql writes the one module models.ts, which is the module pgorm-napi commits, type-checks and imports
// [spec:pgorm:sem:codegen.models.columns/test]    every column type a declared model takes, each field its column in lowerCamelCase under renameAll
// [spec:pgorm:sem:codegen.models.relations/test]    each relation's from and to name fields
// [spec:pgorm:req:codegen.models.layout/test]    the module is laid out as deno fmt and prettier lay it out
#[test]
fn typescript_models_are_the_committed_module() {
    let output =
        typescript_models_from_sql(SCHEMA, ModelWriterOptions::default()).expect("generates");
    holds(
        "../pgorm-napi/tests/models-from-sql/models.ts",
        &one(output, TYPESCRIPT_MODULE),
    );
}

/// What the Python writer refuses of `ddl`.
fn refused(ddl: &str) -> String {
    python_models_from_sql(ddl, ModelWriterOptions::default())
        .expect_err("refused")
        .to_string()
}

// [spec:pgorm:req:codegen.models.refused/test]    a deferrable key, a DEFAULT NULL, a serial outside the key, a bpchar and a name two things take are each refused by name
#[test]
fn what_a_declared_model_cannot_say_is_refused() {
    assert_eq!(
        refused(
            "CREATE TABLE a (id int PRIMARY KEY); CREATE TABLE b (id int PRIMARY KEY, \
             a_id int REFERENCES a (id) DEFERRABLE INITIALLY DEFERRED);"
        ),
        "table `b`: its relation to `a`: its foreign key is DEFERRABLE INITIALLY DEFERRED, \
         which a declared model's relation cannot say"
    );
    assert_eq!(
        refused("CREATE TABLE a (id int PRIMARY KEY, note text DEFAULT NULL);"),
        "table `a`: column `note` defaults to NULL, which a declared model has no word for"
    );
    assert_eq!(
        refused("CREATE TABLE a (id int PRIMARY KEY, n serial);"),
        "table `a`: column `n` is drawn from a sequence outside an integer primary key, which \
         a declared model's auto_increment cannot say"
    );
    assert_eq!(
        refused("CREATE TABLE a (id int PRIMARY KEY, c bpchar);"),
        "table `a`: column `c` is a `bpchar`, which no declared model's column_type spells"
    );
    assert_eq!(
        refused("CREATE TABLE model (id int PRIMARY KEY);"),
        "the model of table `model` is named `Model`, as the `Model` the module may import is"
    );
    assert_eq!(
        refused("CREATE TABLE a_b (id int PRIMARY KEY); CREATE TABLE \"aB\" (id int PRIMARY KEY);"),
        "the model of table `a_b` is named `AB`, as the model of table `aB` is"
    );
}

// [spec:pgorm:def:codegen.models/test]    schema_name qualifies an unqualified table and type, and unkinded_disjoint names a set read as an unkinded disjoint model
#[test]
fn options_qualify_and_name_unkinded_sets() {
    let ddl = "CREATE TYPE mood AS ENUM ('calm'); \
               CREATE TABLE shop (id int PRIMARY KEY, mood mood NOT NULL); \
               CREATE TABLE web_shop (shop_id int PRIMARY KEY REFERENCES shop (id), url text NOT NULL);";
    let table = |name: &str| TableIdent {
        table: name.to_owned(),
        schema: None,
    };
    let options = ModelWriterOptions {
        schema_name: Some("app".to_owned()),
        unkinded_disjoint: vec![UnkindedDisjoint {
            base: table("shop"),
            kinds: vec![table("web_shop")],
            unit: Some("dormant".to_owned()),
        }],
    };
    let python = one(
        python_models_from_sql(ddl, options.clone()).expect("generates"),
        PYTHON_MODULE,
    );
    assert!(
        python.contains("MOOD = TypeName(\"mood\", schema=\"app\")"),
        "{python}"
    );
    assert!(python.contains("schema=\"app\",\n)"), "{python}");
    assert!(
        python.contains(
            "ShopDisjoint = Disjoint(\n    \"shop\", Shop, {\"web_shop\": WebShop, \"dormant\": None}, unkinded=True\n)"
        ),
        "{python}"
    );
    let typescript = one(
        typescript_models_from_sql(ddl, options).expect("generates"),
        TYPESCRIPT_MODULE,
    );
    assert!(typescript.contains("{ unkinded: true }"), "{typescript}");
}

// [spec:pgorm:sem:codegen.models.names/test]    a model is its table in UpperCamelCase, a relation `{table}_{variant}` with the Rust variants of a self reference and of two keys to one table, a type outside the default schema prefixed by it, and a keyword followed by `_`
#[test]
fn names_follow_the_rust_entity() {
    let ddl = "CREATE SCHEMA kitchen; \
               CREATE TYPE kitchen.oven_mood AS ENUM ('calm'); \
               CREATE TYPE \"delete\" AS ENUM ('soft'); \
               CREATE TABLE bakery (id int PRIMARY KEY); \
               CREATE TABLE \"none\" (id int PRIMARY KEY); \
               CREATE TABLE cake_order (id int PRIMARY KEY, \
                 parent_id int REFERENCES cake_order (id), \
                 bakery_id int NOT NULL REFERENCES bakery (id), \
                 pickup_bakery_id int REFERENCES bakery (id), \
                 mood kitchen.oven_mood, purge \"delete\");";
    let python = one(
        python_models_from_sql(ddl, ModelWriterOptions::default()).expect("generates"),
        PYTHON_MODULE,
    );
    for name in [
        "CakeOrder = Model(",
        "None_ = Model(",
        "cake_order_self_ref = CakeOrder.belongs_to(CakeOrder,",
        "cake_order_bakery1 = CakeOrder.belongs_to(Bakery, from_=\"pickup_bakery_id\"",
        "cake_order_bakery2 = CakeOrder.belongs_to(Bakery, from_=\"bakery_id\"",
        "KITCHEN_OVEN_MOOD = TypeName(\"oven_mood\", schema=\"kitchen\")",
        "DELETE = TypeName(\"delete\",",
    ] {
        assert!(python.contains(name), "{name} in {python}");
    }
    let typescript = one(
        typescript_models_from_sql(ddl, ModelWriterOptions::default()).expect("generates"),
        TYPESCRIPT_MODULE,
    );
    for name in [
        "export const CakeOrder = model(",
        "export const None = model(",
        "export const cakeOrderSelfRef = CakeOrder.belongsTo(CakeOrder,",
        "export const cakeOrderBakery1 = CakeOrder.belongsTo(Bakery,",
        "export const cakeOrderBakery2 = CakeOrder.belongsTo(Bakery,",
        "export const kitchenOvenMood = new TypeName(\"oven_mood\", { schema: \"kitchen\" })",
        "export const delete_ = new TypeName(\"delete\",",
    ] {
        assert!(typescript.contains(name), "{name} in {typescript}");
    }
}
