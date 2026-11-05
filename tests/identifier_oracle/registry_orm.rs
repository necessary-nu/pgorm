//! The registry, continued: pgorm's own identifier render sites — the
//! pipeline, the graph, the `QuerySelect` and write helpers, relation and
//! entity names, and schema generation. See `registry.rs` for how to add one.
//!
//! Some pgorm sites exist only against a live server and are registered in
//! `live_capture.rs`: pgorm-pool's savepoint name (its SQL is built inside
//! tokio-postgres), the migration ledger's custom table name (built inside
//! the migrator and run against the database), and the cursor's qualifiers —
//! `Cursor::new`'s table, `set_secondary_order_by`, `add_secondary_order_by`,
//! a graph cursor's alias tiebreak — which render only on execution.

use pgorm::{
    DeleteMany, EntityName, EntityTrait, JoinType, QueryFilter, QuerySelect, QueryTrait,
    RelationTrait, Schema, Select,
    pgorm_query::{
        Expr, ForeignKeyCreateStatement, Func, IntoNamedTable, Name, Query, WindowStatement,
    },
    pipeline::{
        ExprOps, JoinSide, Pipeline, PipelineError, col, count_rows, named_runtime, that, this,
    },
    tests_cfg::{cake, fruit},
};
use pgorm_query::ExprTrait;

use super::{
    fixtures::{cast_named, dyn_named, range_named},
    oracle::{
        Policy::{Pipeline as PipelinePolicy, PipelineAlias, PipelineBare, Quoted, TypePart},
        Rendered, Site,
    },
};

fn n_(name: &str) -> Name {
    Name::runtime(name)
}

fn fixed(name: &'static str) -> Name {
    Name::runtime(name)
}

/// A pipeline's compiled SQL, or its refusal.
fn pipeline(compiled: Result<(String, pgorm::Values), PipelineError>) -> Rendered {
    Rendered::from_result(compiled.map(|(sql, _)| sql))
}

/// A pgorm query's `build()` text.
fn built<Q: QueryTrait>(query: &Q) -> Rendered {
    Rendered::sql(query.build().0)
}

pub fn sites() -> Vec<Site> {
    let mut sites = pipeline_sites();
    sites.extend(select_sites());
    sites.extend(entity_sites());
    sites
}

// ---------------------------------------------------------------------------
// pgorm: the pipeline. Every identifier is screened at `into_sql` (`"` or NUL
// anywhere, a leading `$` and a lone `*` refused) and then quoted — or not —
// by prqlc.
// ---------------------------------------------------------------------------

fn pipeline_sites() -> Vec<Site> {
    vec![
        Site {
            id: "pgorm/pipeline.from.relation",
            api: "Pipeline::from(Name)",
            kinds: &["RangeVar.relname"],
            policy: PipelineBare,
            render: |n| pipeline(Pipeline::from(n_(n)).into_sql()),
        },
        Site {
            id: "pgorm/pipeline.from-schema.schema",
            api: "Pipeline::from_schema(Name, table)",
            kinds: &["RangeVar.schemaname"],
            policy: PipelinePolicy,
            render: |n| pipeline(Pipeline::from_schema(n_(n), fixed("t")).into_sql()),
        },
        Site {
            id: "pgorm/pipeline.from-schema.table",
            api: "Pipeline::from_schema(schema, Name)",
            kinds: &["RangeVar.relname"],
            policy: PipelinePolicy,
            render: |n| pipeline(Pipeline::from_schema(fixed("s"), n_(n)).into_sql()),
        },
        Site {
            id: "pgorm/pipeline.col.column",
            api: "pipeline::col(table, Name)",
            kinds: &["ColumnRef.fields[0]"],
            policy: PipelinePolicy,
            render: |n| {
                pipeline(
                    Pipeline::from(fixed("t"))
                        .select(col(fixed("t"), n_(n)))
                        .into_sql(),
                )
            },
        },
        Site {
            id: "pgorm/pipeline.col.table",
            api: "pipeline::col(Name, column) over Pipeline::from(Name)",
            kinds: &["RangeVar.relname"],
            policy: PipelineBare,
            render: |n| {
                pipeline(
                    Pipeline::from(n_(n))
                        .select(col(n_(n), fixed("c")))
                        .into_sql(),
                )
            },
        },
        Site {
            id: "pgorm/pipeline.name.filter",
            api: "ExprOps for Name in Pipeline::filter",
            kinds: &["ColumnRef.fields[0]"],
            policy: PipelineBare,
            render: |n| pipeline(Pipeline::from(fixed("t")).filter(n_(n).eq(1)).into_sql()),
        },
        Site {
            id: "pgorm/pipeline.name.sort",
            api: "ExprList for Name in Pipeline::sort",
            kinds: &["ColumnRef.fields[0]"],
            policy: PipelineBare,
            render: |n| pipeline(Pipeline::from(fixed("t")).sort(n_(n)).into_sql()),
        },
        Site {
            id: "pgorm/pipeline.name.group",
            api: "ExprList for Name in Pipeline::group",
            kinds: &["ColumnRef.fields[0]", "ColumnRef.fields[0]"],
            policy: PipelineBare,
            render: |n| {
                pipeline(
                    Pipeline::from(fixed("t"))
                        .group(n_(n))
                        .aggregate(count_rows())
                        .into_sql(),
                )
            },
        },
        Site {
            id: "pgorm/pipeline.this.column",
            api: "pipeline::this(Name) in a join condition",
            kinds: &["ColumnRef.fields[1]"],
            policy: PipelinePolicy,
            render: |n| {
                pipeline(
                    Pipeline::from(fixed("t"))
                        .join(
                            JoinSide::Inner,
                            fixed("u"),
                            this(n_(n)).eq(that(fixed("c"))),
                        )
                        .into_sql(),
                )
            },
        },
        Site {
            id: "pgorm/pipeline.that.column",
            api: "pipeline::that(Name) in a join condition",
            kinds: &["ColumnRef.fields[1]"],
            policy: PipelinePolicy,
            render: |n| {
                pipeline(
                    Pipeline::from(fixed("t"))
                        .join(
                            JoinSide::Inner,
                            fixed("u"),
                            this(fixed("c")).eq(that(n_(n))),
                        )
                        .into_sql(),
                )
            },
        },
        Site {
            id: "pgorm/pipeline.join.relation",
            api: "Pipeline::join(side, Name, condition)",
            kinds: &[
                "ColumnRef.fields[0]",
                "RangeVar.relname",
                "ColumnRef.fields[0]",
            ],
            policy: PipelineBare,
            render: |n| {
                pipeline(
                    Pipeline::from(fixed("t"))
                        .join(
                            JoinSide::Inner,
                            n_(n),
                            this(fixed("c")).eq(that(fixed("c"))),
                        )
                        .into_sql(),
                )
            },
        },
        Site {
            id: "pgorm/pipeline.append.relation",
            api: "Pipeline::append(Name)",
            kinds: &["RangeVar.relname"],
            policy: PipelineBare,
            render: |n| pipeline(Pipeline::from(fixed("t")).append(n_(n)).into_sql()),
        },
        Site {
            id: "pgorm/pipeline.as-runtime.derive",
            api: "ExprOps::as_runtime(Name) in Pipeline::derive",
            kinds: &["ResTarget.name"],
            policy: PipelineAlias,
            render: |n| {
                pipeline(
                    Pipeline::from(fixed("t"))
                        .derive(col(fixed("t"), fixed("c")).as_runtime(n_(n)))
                        .into_sql(),
                )
            },
        },
        Site {
            id: "pgorm/pipeline.as-runtime.select",
            api: "ExprOps::as_runtime(Name) in Pipeline::select",
            kinds: &["ResTarget.name"],
            policy: PipelineAlias,
            render: |n| {
                pipeline(
                    Pipeline::from(fixed("t"))
                        .select(col(fixed("t"), fixed("c")).as_runtime(n_(n)))
                        .into_sql(),
                )
            },
        },
        Site {
            id: "pgorm/pipeline.named-runtime.alias",
            api: "pipeline::named_runtime(relation, Name)",
            kinds: &[
                "ColumnRef.fields[0]",
                "RangeVar.alias.aliasname",
                "ColumnRef.fields[0]",
            ],
            policy: PipelineAlias,
            render: |n| {
                pipeline(
                    Pipeline::from(fixed("t"))
                        .join(
                            JoinSide::Inner,
                            named_runtime(fixed("u"), n_(n)),
                            this(fixed("c")).eq(col(n_(n), fixed("c"))),
                        )
                        .into_sql(),
                )
            },
        },
        Site {
            id: "pgorm/pipeline.select-sources.qualifier",
            api: "Pipeline::select_sources(named_runtime(Entity, Name))",
            kinds: &[
                "ColumnRef.fields[0]",
                "RangeVar.alias.aliasname",
                "ColumnRef.fields[0]",
                "ColumnRef.fields[0]",
                "ColumnRef.fields[0]",
            ],
            policy: PipelineAlias,
            render: |n| {
                pipeline(
                    Pipeline::from(cake::Entity)
                        .join(
                            JoinSide::Inner,
                            named_runtime(fruit::Entity, n_(n)),
                            cake::Column::Id.eq(col(n_(n), fixed("cake_id"))),
                        )
                        .select_sources(named_runtime(fruit::Entity, n_(n)))
                        .into_sql(),
                )
            },
        },
        Site {
            id: "pgorm/pipeline.entity.table-name",
            api: "Pipeline::from(entity) with a runtime EntityName::table_name",
            kinds: &["RangeVar.relname"],
            policy: PipelineBare,
            render: |n| pipeline(Pipeline::from(dyn_named::Entity::table(n)).into_sql()),
        },
        // The read cast's type is written by pgorm through `TypeName`'s part
        // policy, the same as the `SelectStatement` writer's, rather than
        // handed to prqlc: it is held to that policy, not the pipeline's.
        Site {
            id: "pgorm/pipeline.select-sources.read-cast",
            api: "Pipeline::select_sources over a ColumnTrait::select_as cast to a runtime type",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                cast_named::with_type(n, || {
                    pipeline(
                        Pipeline::from(cast_named::Entity)
                            .select_sources(cast_named::Entity)
                            .into_sql(),
                    )
                })
            },
        },
    ]
}

// ---------------------------------------------------------------------------
// pgorm: the graph, QuerySelect and the write helpers, over pgorm-query
// ---------------------------------------------------------------------------

fn cakes() -> Select<cake::Entity> {
    cake::Entity::find()
}

fn select_sites() -> Vec<Site> {
    vec![
        Site {
            id: "pgorm/graph.join-maybe-as.alias",
            api: "SelectGraph::join_maybe_as(relation, Name)",
            kinds: &[
                "ColumnRef.fields[0]",
                "RangeVar.alias.aliasname",
                "ColumnRef.fields[0]",
                "ColumnRef.fields[0]",
                "ColumnRef.fields[0]",
            ],
            policy: Quoted,
            render: |n| {
                built(
                    &cake::Entity::graph()
                        .join_maybe_as::<fruit::Entity>(cake::Relation::Fruit.def(), n_(n)),
                )
            },
        },
        Site {
            id: "pgorm/graph.join-one-as.alias",
            api: "SelectGraph::join_one_as(relation, Name)",
            kinds: &[
                "ColumnRef.fields[0]",
                "RangeVar.alias.aliasname",
                "ColumnRef.fields[0]",
                "ColumnRef.fields[0]",
                "ColumnRef.fields[0]",
            ],
            policy: Quoted,
            render: |n| {
                built(
                    &cake::Entity::graph()
                        .join_one_as::<fruit::Entity>(cake::Relation::Fruit.def(), n_(n)),
                )
            },
        },
        Site {
            id: "pgorm/select.column-as.alias",
            api: "QuerySelect::column_as(column, Name)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| built(&cakes().select_only().column_as(cake::Column::Id, n_(n))),
        },
        Site {
            id: "pgorm/select.expr-as.alias",
            api: "QuerySelect::expr_as(expr, Name)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| built(&cakes().select_only().expr_as(Expr::val(1), n_(n))),
        },
        Site {
            id: "pgorm/select.tbl-col-as.table",
            api: "QuerySelect::tbl_col_as((Name, column), alias)",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| {
                built(
                    &cakes()
                        .select_only()
                        .tbl_col_as((n_(n), fixed("c")), fixed("a")),
                )
            },
        },
        Site {
            id: "pgorm/select.join-as.alias",
            api: "QuerySelect::join_as(type, relation, Name)",
            kinds: &["ColumnRef.fields[0]", "RangeVar.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                built(&cakes().join_as(JoinType::LeftJoin, cake::Relation::Fruit.def(), n_(n)))
            },
        },
        Site {
            id: "pgorm/select.join-as-rev.alias",
            api: "QuerySelect::join_as_rev(type, relation, Name)",
            kinds: &["ColumnRef.fields[0]", "RangeVar.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                built(&cakes().join_as_rev(JoinType::LeftJoin, fruit::Relation::Cake.def(), n_(n)))
            },
        },
        Site {
            id: "pgorm/select.join-lateral.alias",
            api: "QuerySelect::join_lateral(type, query, Name, condition)",
            kinds: &["RangeSubselect.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                built(&cakes().join_lateral(
                    JoinType::LeftJoin,
                    Query::select().expr(Expr::val(1)).take(),
                    n_(n),
                    Expr::val(true).eq(true),
                ))
            },
        },
        Site {
            id: "pgorm/select.window.name",
            api: "QuerySelect::window(Name, window)",
            kinds: &["WindowDef.name"],
            policy: Quoted,
            render: |n| {
                built(&cakes().window(n_(n), WindowStatement::partition_by(cake::Column::Id)))
            },
        },
        Site {
            id: "pgorm/select.window-expr-as.window",
            api: "QuerySelect::window_expr_as(call, Name, alias)",
            kinds: &["FuncCall.over.name"],
            policy: Quoted,
            render: |n| {
                built(&cakes().select_only().window_expr_as(
                    Func::count(Expr::col(cake::Column::Id)),
                    n_(n),
                    fixed("a"),
                ))
            },
        },
        Site {
            id: "pgorm/select.window-expr-as.alias",
            api: "QuerySelect::window_expr_as(call, window, Name)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                built(&cakes().select_only().window_expr_as(
                    Func::count(Expr::col(cake::Column::Id)),
                    fixed("w"),
                    n_(n),
                ))
            },
        },
        Site {
            id: "pgorm/select.distinct-on",
            api: "QuerySelect::distinct_on([Name])",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| built(&cakes().distinct_on([n_(n)])),
        },
        Site {
            id: "pgorm/select.belongs-to-tbl-alias",
            api: "QuerySelect::belongs_to_tbl_alias(model, Name)",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| {
                let model = cake::Model {
                    id: 1,
                    name: String::new(),
                };
                built(&fruit::Entity::find().belongs_to_tbl_alias(&model, n_(n)))
            },
        },
        Site {
            id: "pgorm/update-many.col-expr",
            api: "UpdateMany::col_expr(Name, expr)",
            kinds: &["ResTarget.name"],
            policy: Quoted,
            render: |n| {
                built(&pgorm::Update::many(cake::Entity).col_expr(n_(n), Expr::val(1).into()))
            },
        },
        Site {
            id: "pgorm/update-many.from",
            api: "UpdateMany::from(Name)",
            kinds: &["RangeVar.relname"],
            policy: Quoted,
            render: |n| {
                built(
                    &pgorm::Update::many(cake::Entity)
                        .col_expr(cake::Column::Name, Expr::val("x").into())
                        .from(n_(n)),
                )
            },
        },
        Site {
            id: "pgorm/delete-many.using",
            api: "DeleteMany::using(Name)",
            kinds: &["RangeVar.relname"],
            policy: Quoted,
            render: |n| {
                let delete: DeleteMany<cake::Entity> =
                    pgorm::Delete::many(cake::Entity).using(n_(n));
                built(&delete)
            },
        },
        Site {
            id: "pgorm/relation.from-alias",
            api: "RelationDef::from_alias(Name) through QuerySelect::join",
            kinds: &["ColumnRef.fields[0]"],
            policy: Quoted,
            render: |n| {
                built(&cakes().join(
                    JoinType::LeftJoin,
                    cake::Relation::Fruit.def().from_alias(n_(n)),
                ))
            },
        },
        Site {
            id: "pgorm/relation.fk-name",
            api: "RelationBuilder::fk_name(&str) into ForeignKeyCreateStatement",
            kinds: &["Constraint.conname"],
            policy: Quoted,
            render: |n| {
                let relation: pgorm::RelationDef = fruit::Entity::belongs_to(cake::Entity)
                    .columns(fruit::Column::CakeId, cake::Column::Id)
                    .fk_name(n)
                    .into();
                Rendered::sql(ForeignKeyCreateStatement::from(relation).to_string())
            },
        },
    ]
}

// ---------------------------------------------------------------------------
// pgorm: runtime entity names (`EntityName::table_name` / `schema_name`
// computed per instance) and the schema generator's derived names
// ---------------------------------------------------------------------------

fn entity_sites() -> Vec<Site> {
    vec![
        Site {
            id: "pgorm/entity.table-name.update-many",
            api: "Update::many(entity) with a runtime EntityName::table_name",
            kinds: &["UpdateStmt.relation.relname"],
            policy: Quoted,
            render: |n| {
                built(
                    &pgorm::Update::many(dyn_named::Entity::table(n))
                        .col_expr(dyn_named::Column::Name, Expr::val("x").into()),
                )
            },
        },
        Site {
            id: "pgorm/entity.table-name.delete-many",
            api: "Delete::many(entity) with a runtime EntityName::table_name",
            kinds: &["DeleteStmt.relation.relname"],
            policy: Quoted,
            render: |n| built(&pgorm::Delete::many(dyn_named::Entity::table(n))),
        },
        Site {
            id: "pgorm/entity.schema-name.table-ref",
            api: "EntityTrait::table_ref() with a runtime EntityName::schema_name",
            kinds: &["RangeVar.schemaname"],
            policy: Quoted,
            render: |n| {
                Rendered::sql(
                    Query::select()
                        .expr(Expr::val(1))
                        .from(dyn_named::Entity::schema(n).table_ref())
                        .to_string(),
                )
            },
        },
        Site {
            id: "pgorm/entity.schema-name.select-from",
            api: "SelectStatement::from(entity) with a runtime EntityName::schema_name",
            kinds: &["RangeVar.schemaname"],
            policy: Quoted,
            render: |n| {
                Rendered::sql(
                    Query::select()
                        .expr(Expr::val(1))
                        .from(dyn_named::Entity::schema(n))
                        .to_string(),
                )
            },
        },
        Site {
            id: "pgorm/entity.schema-name.join",
            api: "SelectStatement::join(kind, entity, on) with a runtime EntityName::schema_name",
            kinds: &["RangeVar.schemaname"],
            policy: Quoted,
            render: |n| {
                Rendered::sql(
                    Query::select()
                        .expr(Expr::val(1))
                        .from(fixed("u"))
                        .join(
                            JoinType::InnerJoin,
                            dyn_named::Entity::schema(n),
                            Expr::val(true).eq(true),
                        )
                        .to_string(),
                )
            },
        },
        Site {
            id: "pgorm/entity.schema-name.insert-into",
            api: "InsertStatement::into_table(entity) with a runtime EntityName::schema_name",
            kinds: &["InsertStmt.relation.schemaname"],
            policy: Quoted,
            render: |n| {
                Rendered::sql(
                    Query::insert()
                        .into_table(dyn_named::Entity::schema(n))
                        .columns([dyn_named::Column::Id])
                        .values_panic([1.into()])
                        .to_string(),
                )
            },
        },
        Site {
            id: "pgorm/entity.schema-name.update",
            api: "UpdateStatement::table(entity) with a runtime EntityName::schema_name",
            kinds: &["UpdateStmt.relation.schemaname"],
            policy: Quoted,
            render: |n| {
                Rendered::sql(
                    Query::update()
                        .table(dyn_named::Entity::schema(n))
                        .value(dyn_named::Column::Id, 1)
                        .to_string(),
                )
            },
        },
        Site {
            id: "pgorm/entity.schema-name.delete-from",
            api: "DeleteStatement::from_table(entity) with a runtime EntityName::schema_name",
            kinds: &["DeleteStmt.relation.schemaname"],
            policy: Quoted,
            render: |n| {
                Rendered::sql(
                    Query::delete()
                        .from_table(dyn_named::Entity::schema(n))
                        .to_string(),
                )
            },
        },
        Site {
            id: "pgorm/schema.create-table-from-entity",
            api: "Schema::create_table_from_entity(entity) with a runtime table name",
            kinds: &["CreateStmt.relation.relname"],
            policy: Quoted,
            render: |n| match Schema::new().create_table_from_entity(dyn_named::Entity::table(n)) {
                Ok(table) => Rendered::sql(table.to_string()),
                Err(error) => Rendered::Refused(error.to_string()),
            },
        },
        Site {
            id: "pgorm/schema.create-index-from-entity",
            api: "Schema::create_index_from_entity(entity) with a runtime table name",
            kinds: &["IndexStmt.idxname", "IndexStmt.relation.relname"],
            policy: Quoted,
            render: |n| {
                let indexes = Schema::new().create_index_from_entity(dyn_named::Entity::table(n));
                match indexes.first() {
                    Some(index) => Rendered::sql(index.to_string()),
                    None => Rendered::Refused("the fixture entity declares no index".to_owned()),
                }
            },
        },
        Site {
            id: "pgorm/column.save-as.created-range",
            api: "ColumnTrait::eq on a ColumnType::CreatedRange { name: Name, .. } column",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                range_named::with_type(false, None, n, || {
                    built(&range_named::Entity::find().filter(pgorm::ColumnTrait::eq(
                        &range_named::Column::Id,
                        pgorm::pgorm_query::Range::from(1..5),
                    )))
                })
            },
        },
        Site {
            id: "pgorm/column.save-as.created-range-schema",
            api: "ColumnTrait::eq on a ColumnType::CreatedRange { schema: Some(Name), .. } column",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                range_named::with_type(false, Some(n), "ty", || {
                    built(&range_named::Entity::find().filter(pgorm::ColumnTrait::eq(
                        &range_named::Column::Id,
                        pgorm::pgorm_query::Range::from(1..5),
                    )))
                })
            },
        },
        Site {
            id: "pgorm/column.save-as.created-multirange",
            api: "ColumnTrait::eq on a ColumnType::CreatedMultirange { name: Name, .. } column",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                range_named::with_type(true, None, n, || {
                    built(&range_named::Entity::find().filter(pgorm::ColumnTrait::eq(
                        &range_named::Column::Id,
                        pgorm::pgorm_query::Multirange::from(vec![
                            pgorm::pgorm_query::Range::from(1..5),
                        ]),
                    )))
                })
            },
        },
        Site {
            id: "pgorm/column.save-as.created-multirange-schema",
            api: "ColumnTrait::eq on a ColumnType::CreatedMultirange { schema: Some(Name), .. } column",
            kinds: &["TypeCast.type_name.names[0]"],
            policy: TypePart,
            render: |n| {
                range_named::with_type(true, Some(n), "ty", || {
                    built(&range_named::Entity::find().filter(pgorm::ColumnTrait::eq(
                        &range_named::Column::Id,
                        pgorm::pgorm_query::Multirange::from(vec![
                            pgorm::pgorm_query::Range::from(1..5),
                        ]),
                    )))
                })
            },
        },
        Site {
            id: "pgorm/entity.named-table.alias",
            api: "EntityTrait::table_ref().alias(Name) through Query::select",
            kinds: &["RangeVar.alias.aliasname"],
            policy: Quoted,
            render: |n| {
                Rendered::sql(
                    Query::select()
                        .expr(Expr::val(1))
                        .from(cake::Entity.table_ref().into_named_table().alias(n_(n)))
                        .to_string(),
                )
            },
        },
    ]
}
