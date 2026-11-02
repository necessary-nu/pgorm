//! An intentionally test-only HTTP boundary over pgorm and independent vulnerable controls.
use axum::{extract::{Path, Query as HttpQuery, State}, http::StatusCode, routing::get, Router};
use pgorm::{entity::prelude::*, DecodeRaw};
use pgorm::pgorm_query::{self as q, Expr, Func, Query, Values};
use pgorm_sqlmap_adapter::harness::{declared_input, fields};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Write, sync::{Arc, Mutex}};
use tokio_postgres::SimpleQueryMessage;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

mod item {
    use pgorm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "items")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub tenant: i32,
        pub name: String,
        pub note: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

#[derive(Clone)]
struct App {
    db: pgorm::DatabasePool,
    raw: Arc<tokio_postgres::Client>,
    serial: Arc<tokio::sync::Mutex<()>>,
    evidence: Arc<Mutex<std::fs::File>>,
    counts: Arc<Mutex<BTreeMap<String, u64>>>,
    /// Each case's declared test parameter, from the manifest the runner inventoried.
    fields: Arc<BTreeMap<String, String>>,
}

fn names(rows: Vec<item::Model>) -> Vec<String> {
    rows.into_iter().map(|r| format!("{}:{}:{}:{}", r.id, r.tenant, r.name, r.note)).collect()
}

async fn strings(db: &pgorm::DatabaseConnection, query: (String, Values)) -> Result<Vec<String>> {
    Ok(query.into_tuple::<String>().all(db).await?)
}

// [spec:pgorm:req:security.sqlmap.adapter]
// [spec:pgorm:req:security.sqlmap.matrix]
async fn protected(app: &App, case: &str, input: &str) -> Result<Value> {
    use item::{Column as C, Entity as E};
    use pgorm::pipeline::{col, IntoSource, JoinSide, Pipeline};
    let db = app.db.get().await?;
    let a = || Name::runtime("items");
    let n = || Name::runtime("name");
    let select = || Query::select().column(n()).from(a()).to_owned();
    let values = match case {
        "select" => names(E::find().filter(C::Name.eq(input)).order_by_asc(C::Id).all(&db).await?),
        "insert" => {
            let source = Query::select().expr(Expr::val(10)).expr(Expr::val(1)).expr(Expr::val("alice")).expr(Expr::val("inserted")).and_where(Expr::val("alice").eq(input)).to_owned();
            let insert = Query::insert().into_table(a()).columns([C::Id,C::Tenant,C::Name,C::Note]).select_from(source)?.returning_col(C::Name).to_owned();
            strings(&db, insert.build()).await?
        },
        "update-value" => {
            pgorm::Update::many(E).col_expr(C::Note, Expr::val("touched").into()).filter(C::Id.eq(1)).filter(C::Name.eq(input)).exec(&db).await?;
            names(E::find().filter(C::Id.eq(1)).all(&db).await?)
        },
        "update-guard" => {
            pgorm::Update::many(E).col_expr(C::Note, Expr::val("changed").into()).filter(C::Tenant.eq(1)).filter(C::Name.eq(input)).exec(&db).await?;
            names(E::find().filter(C::Note.eq("changed")).all(&db).await?)
        },
        "delete" | "tenant-or" | "empty-in" | "empty-or" => {
            let predicate = match case {
                "tenant-or" => Condition::any().add(C::Name.eq(input)).add(C::Name.eq("bob")),
                "empty-in" => Condition::all().add(C::Name.eq(input)).add(C::Id.is_in(Vec::<i32>::new())),
                "empty-or" => Condition::all().add(C::Name.eq(input)).add(Condition::any()),
                _ => Condition::all().add(C::Name.eq(input)),
            };
            let count = pgorm::Delete::many(E).filter(C::Tenant.eq(1)).filter(predicate).exec(&db).await?;
            vec![count.to_string()]
        },
        "graph-root" => {
            E::graph().filter(C::Name.eq(input)).all(&db).await?.into_iter().map(|r| r.name).collect()
        },
        "graph-joined" | "graph-alias" => {
            let alias = if case == "graph-alias" { input } else { "peer" };
            let relation = E::belongs_to(E).columns(C::Id,C::Id).into();
            let graph = E::graph().join_one_as::<E>(relation, Name::runtime(alias));
            let graph = if case == "graph-joined" {graph.filter(Expr::col((Name::runtime(alias), n())).eq(input))} else {graph};
            graph.all(&db).await?.into_iter().map(|(r, s)| format!("{}:{}",r.name,s.name)).collect()
        },
        "pipeline-literal" | "pipeline-bound" => {
            use pgorm::pipeline::ExprOps;
            let pipeline = Pipeline::from(E);
            let pipeline = if case == "pipeline-bound" {
                pipeline.filter_with(|b| col(a(),n()).eq(b.bind(input)))
            } else { pipeline.filter(col(a(),n()).eq(input)) };
            strings(&db, pipeline.select(col(a(),n())).into_sql()?).await?
        },
        "pipeline-projection" => strings(&db, Pipeline::from(E).select(col(a(), Name::runtime(input))).into_sql()?).await?,
        "pipeline-sources" => {
            use pgorm::pipeline::ExprOps;
            let peer = Name::runtime("peer");
            Pipeline::from(E).join(JoinSide::Inner, E.named("peer"), col(a(),Name::runtime("id")).eq(col(peer.clone(),Name::runtime("id"))))
                .filter(col(peer.clone(),n()).eq(input)).select_sources((E,E.named("peer")))
                .all(&db).await?.into_iter().map(|(r,s)|format!("{r:?}:{s:?}")).collect()
        },
        "schema" => strings(&db, Query::select().column(n()).from((Name::runtime(input),a())).build()).await?,
        "table" => strings(&db, Query::select().column(n()).from(Name::runtime(input)).build()).await?,
        "column" => strings(&db, Query::select().column(Name::runtime(input)).from(a()).build()).await?,
        "alias" => {
            let sql = Query::select().expr_as(Expr::val("alice"),Name::runtime(input)).to_string();
            let row = db.query_one(&sql, &[]).await?;
            vec![row.columns()[0].name().to_owned(), row.get(0)]
        },
        "order" => strings(&db,select().order_by(Name::runtime(input),q::Order::Asc).build()).await?,
        "group" => strings(&db,Query::select().column(Name::runtime(input)).from(a()).group_by_col(Name::runtime(input)).build()).await?,
        "function" => strings(&db,Query::select().expr(Func::named(Name::runtime(input)).arg(Expr::val("Alice"))).build()).await?,
        "cast" => strings(&db,Query::select().column(n()).from(a()).and_where(Expr::col(n()).eq(Expr::val("alice").cast_as(Name::runtime(input)))).build()).await?,
        "enum" => {
            let sql = Query::select().column(n()).from(Name::runtime("reviews")).and_where(Expr::col(Name::runtime("status")).eq(Expr::val("ready").cast_as_type(q::TypeName::new(Name::runtime(input)).schema(Name::runtime("fixture"))))).to_string();
            strings(&db,(sql,Values(vec![]))).await?
        },
        "enum-ddl" => {
            let sql = q::Table::create(Name::runtime("enum_probe")).col(q::ColumnDef::new(Name::runtime("value")).enumeration(Name::runtime(input),[Name::runtime("ready"),Name::runtime("waiting")])).to_string();
            db.batch_execute(&sql).await?;
            db.query_all("SELECT udt_name FROM information_schema.columns WHERE table_schema='fixture' AND table_name='enum_probe' ORDER BY ordinal_position",&[]).await?.into_iter().map(|r|r.get(0)).collect()
        },
        "parameters" => {
            let source = "SELECT $1::text AS \"value$1\" WHERE $1::text IS NOT NULL /* $1 /* $2 */ */ -- $1\n AND $$ $1 $$ = $tag$ $1 $tag$";
            let sql = q::inject_parameters(source,[input.into()])?;
            strings(&db,(sql,Values(vec![]))).await?
        },
        "contains" => names(E::find().filter(C::Name.contains(input)).all(&db).await?),
        "starts-with" => names(E::find().filter(C::Name.starts_with(input)).all(&db).await?),
        "ends-with" => names(E::find().filter(C::Name.ends_with(input)).all(&db).await?),
        "like" => names(E::find().filter(C::Name.like(input)).all(&db).await?),
        "direction" => {
            let order = match input { "asc" => q::Order::Asc, "desc" => q::Order::Desc, _ => return Ok(json!({"rejected":"expected asc or desc"})) };
            strings(&db,select().order_by(n(),order).build()).await?
        },
        "limit" | "offset" => {
            let Ok(value) = input.parse::<u64>() else { return Ok(json!({"rejected":"expected unsigned integer"})); };
            let mut query = select(); query.order_by(n(),q::Order::Asc);
            if case == "limit" {query.limit(value);} else {query.offset(value);}
            strings(&db,query.build()).await?
        },
        "stored-value" | "stored-identifier" => {
            db.execute("INSERT INTO stored VALUES ($1)", &[&input]).await?;
            let saved: String = db.query_one("SELECT value FROM stored",&[]).await?.get(0);
            if case == "stored-value" { names(E::find().filter(C::Name.eq(saved)).all(&db).await?) }
            else { strings(&db,Query::select().column(Name::runtime(saved)).from(a()).build()).await? }
        },
        _ => return Err(format!("unknown case: {case}").into()),
    };
    let sentinel = db.query_one("SELECT count(*)::int FROM items WHERE id=3 AND tenant=2 AND name='alice' AND note='sentinel'",&[]).await?.get::<_,i32>(0);
    Ok(json!({"rows":values,"invariant":sentinel==1}))
}

// [spec:pgorm:req:security.sqlmap.controls]
// These concatenations are the independent positive controls. Do not use ORM escaping here.
async fn control(app: &App, case: &str, input: &str) -> Result<Value> {
    let sql = match case {
        // A single-row VALUES literal has no truth slot and its tuple arity blocks every
        // level-3 boundary; guarding an INSERT ... SELECT on the value reaches a WHERE, where
        // the same '-delimited literal exposes boolean/error/time/stacked, and the inner
        // SELECT's set-operation position surfaced through RETURNING also exposes UNION.
        "insert" => format!("INSERT INTO items SELECT 10,1,'alice','inserted' WHERE 'alice'='{input}' RETURNING name"),
        // A SET assignment is not a truth slot at level 3; the same value in the WHERE is.
        "update-value" => format!("UPDATE items SET note='touched' WHERE id=1 AND name='{input}' RETURNING name"),
        "update-guard" => format!("UPDATE items SET note='changed' WHERE tenant=1 AND name='{input}' RETURNING name"),
        "delete" | "tenant-or" | "empty-in" | "empty-or" => format!("DELETE FROM items WHERE tenant=1 AND name='{input}' RETURNING name"),
        "schema" => format!("SELECT name FROM \"{input}\".items"),
        "table" => format!("SELECT name FROM \"{input}\""),
        "column" | "group" | "pipeline-projection" | "stored-identifier" => format!("SELECT \"{input}\" FROM items"),
        "order" => format!("SELECT name FROM items ORDER BY \"{input}\""),
        "alias" | "graph-alias" => format!("SELECT name FROM items AS \"{input}\""),
        "function" => format!("SELECT {input}('Alice')"),
        // The unquoted CAST type closes with `)` into a WHERE truth slot, reaching B/E/S/T/U.
        "cast" => format!("SELECT name FROM items WHERE name = CAST('alice' AS {input})"),
        // A projected enum cast leaves the closed type name in front of an enum-typed value,
        // where AND is a type error and a UNION branch cannot carry a text marker; filtering an
        // enum column on the same cast puts the closed identifier in a WHERE truth slot instead.
        "enum" => format!("SELECT name FROM reviews WHERE status = CAST('ready' AS fixture.\"{input}\")"),
        "enum-ddl" => format!("CREATE TABLE enum_probe (value \"{input}\"); SELECT udt_name FROM information_schema.columns WHERE table_schema='fixture' AND table_name='enum_probe'"),
        "direction" => format!("SELECT name FROM items ORDER BY name {input}"),
        "limit" => format!("SELECT name FROM items LIMIT {input}"),
        "offset" => format!("SELECT name FROM items OFFSET {input}"),
        "parameters" => format!("SELECT '{input}'::text"),
        "contains" => format!("SELECT name FROM items WHERE name LIKE '%{input}%'"),
        "starts-with" => format!("SELECT name FROM items WHERE name LIKE '{input}%'"),
        "ends-with" => format!("SELECT name FROM items WHERE name LIKE '%{input}'"),
        "like" => format!("SELECT name FROM items WHERE name LIKE '{input}'"),
        "stored-value" => {
            app.raw.execute("INSERT INTO stored VALUES ($1)",&[&input]).await?;
            let saved: String = app.raw.query_one("SELECT value FROM stored",&[]).await?.get(0);
            format!("SELECT name FROM items WHERE name='{saved}'")
        },
        _ => format!("SELECT name FROM items WHERE name='{input}'"),
    };
    let rows:Vec<Vec<Option<String>>> = app.raw.simple_query(&sql).await?.into_iter().filter_map(|r|match r {
        SimpleQueryMessage::Row(r) => Some((0..r.len()).map(|i|r.get(i).map(str::to_owned)).collect()), _=>None
    }).collect();
    Ok(json!({"rows":rows,"invariant":true}))
}

// [spec:pgorm:req:security.sqlmap.fixtures]
// [spec:pgorm:req:security.sqlmap.observability]
// [spec:pgorm:req:security.sqlmap.profiles+4]
async fn route(State(app):State<App>, Path((mode,case)):Path<(String,String)>, HttpQuery(query):HttpQuery<Vec<(String,String)>>) -> (StatusCode,String) {
    // Only the case's declared parameter is read, so the scanner's -p and the route agree.
    let Some(field) = app.fields.get(&case) else { return (StatusCode::NOT_FOUND,format!("unknown case: {case}")); };
    let input = match declared_input(field,&query) { Ok(input) => input, Err(e) => return (StatusCode::BAD_REQUEST,e.to_string()) };
    let _serial = app.serial.lock().await;
    let mut invoked = false;
    let result: Result<Value> = async {
        app.raw.batch_execute(include_str!("../../fixtures.sql")).await?;
        invoked = true;
        match mode.as_str() { "protected"=>protected(&app,&case,input).await,"control"=>control(&app,&case,input).await,_=>Err("unknown mode".into()) }
    }.await;
    let (status,body) = match result {
        Ok(v) if v.get("rejected").is_some() => (StatusCode::UNPROCESSABLE_ENTITY,v),
        Ok(v)=>(StatusCode::OK,v),
        Err(e)=>{
            let mut details=vec![e.to_string()]; let mut source=e.source();
            while let Some(e)=source {details.push(e.to_string());source=e.source();}
            (StatusCode::INTERNAL_SERVER_ERROR,json!({"error":details}))
        }
    };
    let key=format!("{mode}/{case}");
    if invoked { *app.counts.lock().expect("counts mutex").entry(key.clone()).or_default()+=1; }
    let event=json!({"route":key,"parameter":field,"input":input,"invoked":invoked,"status":status.as_u16(),"response":body});
    if let Err(e) = writeln!(app.evidence.lock().expect("evidence mutex"),"{event}") {
        return (StatusCode::INTERNAL_SERVER_ERROR,format!("evidence write failed: {e}"));
    }
    (status,body.to_string())
}

// [spec:pgorm:def:security.sqlmap]
// [spec:pgorm:req:security.sqlmap.isolation]
#[tokio::main]
async fn main() -> Result<()> {
    let config:tokio_postgres::Config=std::env::var("SQLMAP_FIXTURE_URL")?.parse()?;
    let (raw,connection)=config.connect(tokio_postgres::NoTls).await?;
    tokio::spawn(async move { if let Err(e)=connection.await {eprintln!("fixture connection ended: {e}");} });
    let privileged:bool=raw.query_one("SELECT rolsuper OR rolcreatedb OR rolcreaterole OR pg_has_role(current_user,'pg_execute_server_program','member') OR pg_has_role(current_user,'pg_read_server_files','member') OR pg_has_role(current_user,'pg_write_server_files','member') FROM pg_roles WHERE rolname=current_user",&[]).await?.get(0);
    if privileged {return Err("adapter refuses a privileged database role".into());}
    let fields=Arc::new(fields(&serde_json::from_slice(&std::fs::read(std::env::var("SQLMAP_MANIFEST")?)?)?)?);
    let app=App {db:pgorm::connect(config),raw:Arc::new(raw),serial:Arc::new(tokio::sync::Mutex::new(())),evidence:Arc::new(Mutex::new(std::fs::OpenOptions::new().append(true).create_new(true).open(std::env::var("SQLMAP_EVIDENCE")?)?)),counts:Default::default(),fields};
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    println!("{}",json!({"port":listener.local_addr()?.port()}));
    std::io::stdout().flush()?;
    let router=Router::new().route("/case/{mode}/{case}",get(route)).route("/health",get(||async{"ready"})).route("/counts",get(|State(a):State<App>|async move {axum::Json(a.counts.lock().expect("counts mutex").clone())})).with_state(app);
    axum::serve(listener,router).with_graceful_shutdown(async{let _=tokio::signal::ctrl_c().await;}).await?;
    Ok(())
}
