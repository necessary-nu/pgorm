"""Static rejection cases; each marker must correspond to an installed-stub diagnostic."""

from contextlib import AbstractAsyncContextManager

import pgorm as p
from pgorm import pipeline as pl, schema as s


async def invalid(connection: p.Connection) -> None:
    await connection.execute("SELECT 1")  # expected-error: arg-type
    pending = p.merge(p.Table("t"), p.Table("s"), p.col("id") == 1)
    await connection.query(pending)  # expected-error: arg-type
    pending.when_matched(p.MergeAction.insert("id", 1))  # expected-error: arg-type
    pending.when_matched(p.MergeAction.insert_defaults())  # expected-error: arg-type
    pending.when_not_matched_by_source(p.MergeAction.insert("id", 1))  # expected-error: arg-type
    pending.when_not_matched(p.MergeAction.update("id", 1))  # expected-error: arg-type
    pending.when_not_matched(p.MergeAction.delete())  # expected-error: arg-type
    pending.when_matched(p.MergeAction.do_nothing()).when_not_matched(p.MergeAction.do_nothing())
    p.MergeAction("delete")  # expected-error: arg-type
    p.ConflictTarget("id")  # expected-error: arg-type
    scalar: int = await connection.one(  # expected-error: assignment
        p.select(p.literal(1))
    )
    connection.transaction(mode="invented")  # expected-error: arg-type
    connection.transaction(isolation="invented")  # expected-error: arg-type
    await connection.begin()  # expected-error: attr-defined
    async with connection.transaction() as tx:
        await tx.begin()  # expected-error: attr-defined
        await tx.commit()  # expected-error: attr-defined
        await tx.rollback()  # expected-error: attr-defined
        await tx.close()  # expected-error: attr-defined
        tx.transaction(mode="read_only")  # expected-error: call-arg
        held: AbstractAsyncContextManager[p.Transaction] = tx  # expected-error: assignment
    p.Expr()  # expected-error: call-arg
    p.Compiled()  # expected-error: call-arg
    p.col("id").__eq__(other=1)  # expected-error: call-arg
    s.DataType("not_a_type")  # expected-error: arg-type
    p.DataType("not_a_type")  # expected-error: arg-type
    s.create_table(p.Table("t")).column("raw SQL")  # expected-error: arg-type
    s.CreateTable(p.Table("t"))  # expected-error: arg-type
    s.CreateIndex(p.Table("t"))  # expected-error: arg-type
    s.CreateType("mood")  # expected-error: arg-type
    s.CreateDomain("code")  # expected-error: arg-type
    s.CreateSequence("ticket")  # expected-error: arg-type
    view = p.select(p.literal(1))
    s.CreateView(view)  # expected-error: arg-type
    s.CreateRecursiveView(view)  # expected-error: arg-type
    s.CreateMaterializedView(view)  # expected-error: arg-type
    s.CreateTableAs(view)  # expected-error: arg-type
    p.select(p.literal(1)).limit("3")  # expected-error: arg-type
    p.select(p.literal(1)).limit_with_ties(None)  # expected-error: arg-type
    await connection.copy_in_rows(p.CopyFrom(p.Table("t"), "text"), [[1]])  # expected-error: arg-type
    await connection.copy_out_rows(p.CopyTo(p.Table("t"), "csv"))  # expected-error: arg-type
    p.CopyFrom(p.Table("t"), "text").quote("'")  # expected-error: misc
    p.CopyFrom(p.Table("t"), "binary").header()  # expected-error: misc
    p.CopyTo(p.Table("t"), "binary").null_string("")  # expected-error: misc
    p.CopyFrom(p.Table("t"), "csv").force_quote_all()  # expected-error: attr-defined
    p.CopyTo(p.Table("t"), "csv").freeze()  # expected-error: attr-defined
    p.CopyFrom(p.Table("t"), "yaml")  # expected-error: type-var
    await connection.copy_in(p.CopyFrom(p.Table("t"), "text"), "1\tone\n")  # expected-error: arg-type
    await connection.copy_in(p.CopyTo(p.Table("t"), "text"), b"")  # expected-error: arg-type
    p.select(p.literal(1)).union(p.RawSQL("SELECT 2"))  # expected-error: arg-type
    p.select(p.literal(1)).window("w", p.col("id"))  # expected-error: arg-type
    p.ExtendedWindow("w").partition_by(p.col("id"))  # expected-error: attr-defined
    p.GroupingElement.sets()  # expected-error: call-arg
    p.Table("t").tablesample("bernoulli", 10)  # expected-error: arg-type
    p.TableFunction(p.Table("t"))  # expected-error: arg-type
    p.case_when("x > 0", 1)  # expected-error: arg-type
    p.case_of(p.col("x")).else_(0)  # expected-error: attr-defined
    p.case_when(p.col("x") > 0, 1).else_(0).when(p.col("x") < 0, 2)  # expected-error: attr-defined
    p.select(p.case_of(p.col("x")))  # expected-error: arg-type
    p.col("x").like(p.any_(["a%"])) & p.col("doc").path_exists(p.all_(p.col("paths")))
    p.col("x").similar_to(p.any_(["a%"]))  # expected-error: arg-type
    p.col("doc").path_exists(p.col("path"))  # expected-error: arg-type
    pl.from_(p.Table("t")).filter_with(42)  # expected-error: arg-type
    stalls = p.Model("stalls", {"id": p.Column("i64", primary_key=True)})
    pl.col(stalls, stalls.col("id"))  # expected-error: call-overload
    pl.from_(stalls).into_model(3)  # expected-error: call-overload
    pl.from_(stalls).select_sources()  # expected-error: call-overload
    pl.from_(stalls).select_sources(stalls, qualifiers=["stalls"])  # expected-error: call-overload
    shop_rows: list[p.ModelRecord] = await pl.from_(stalls).select_sources(stalls).all(connection)  # expected-error: assignment
    optional: p.Record | None = await connection.query(  # expected-error: assignment
        p.select(p.literal(1))
    )
    sqlstate: str = p.DatabaseError("client failure").sqlstate  # expected-error: assignment
    accounts = p.Model("accounts", {"id": p.Column("i64", primary_key=True)})
    p.Model("accounts", {"id": p.Column("i64")}, unique_keys=[{"fields": "id", "nulls_distinct": True}])  # expected-error: typeddict-unknown-key
    p.Column("i32", generated="sometimes")  # expected-error: arg-type
    # [spec:pgorm:req:bindings.model-array-casts/test]    save_array_with is a function of an expression
    p.Column("text", save_with=lambda value: value, save_array_with="upper")  # expected-error: arg-type
    # [spec:pgorm:req:bindings.schema-from-model/test]    the DDL options are typed: expressions, a closed case style, a belongs_to's actions
    p.Column("i32", default_expr=1)  # expected-error: arg-type
    p.Column("i32", generated_stored="price")  # expected-error: arg-type
    p.Column("i32", column_type="not_a_type")  # expected-error: arg-type
    p.Model("accounts", {"id": p.Column("i64")}, rename_all="Camel")  # expected-error: arg-type
    accounts.belongs_to(accounts, from_="id", to="id", on_delete="drop")  # expected-error: arg-type
    accounts.has_many(accounts, from_="id", to="id", on_delete="cascade")  # expected-error: call-arg
    s.from_model(p.entity("accounts"))  # expected-error: arg-type
    accounts.graph().join_one(accounts.col("id"))  # expected-error: arg-type
    accounts.find().cursor("id").first("2")  # expected-error: arg-type
    accounts.find().join(accounts.col("id"))  # expected-error: arg-type
    accounts.col("id").between(1, 2, symmetric="yes")  # expected-error: arg-type
    rows: list[p.ModelRecord] = await accounts.graph().all_grouped(connection)  # expected-error: assignment
    await accounts.graph().limit(2).all_grouped(connection)  # expected-error: misc
    await accounts.graph().offset(None).count(connection)  # expected-error: misc
    accounts.graph().limit(2).cursor("id")  # expected-error: misc
    accounts.graph().where(accounts.col("id") == 1).offset(1).paginate(10)  # expected-error: misc
    # [spec:pgorm:req:bindings.relations+3/test]    a relation's condition type is a closed choice, and a load's query a model query
    selves = accounts.has_many(accounts, from_="id", to="id")
    accounts.has_many(accounts, from_="id", to="id", condition_type="either")  # expected-error: arg-type
    await selves.load(connection, [], accounts)  # expected-error: arg-type
    # [spec:pgorm:req:bindings.related/test]    a related path names its junction's relation as via
    accounts.related(selves)  # expected-error: call-arg
    # [spec:pgorm:req:bindings.linked/test]    a chain is made of relations, and a hop is its index
    accounts.linked(accounts)  # expected-error: arg-type
    accounts.linked(selves).hop("r0")  # expected-error: arg-type
    # [spec:pgorm:req:bindings.cursors+1/test]    a slot cursor's source is its index
    accounts.graph().join_one(selves).cursor_on("1", "id")  # expected-error: arg-type
    accounts.graph().join_one(selves).limit(2).cursor_on(1, "id")  # expected-error: misc
    # [spec:pgorm:req:bindings.model-hooks/test]    a model's hooks are the four named functions, and a record write takes its database
    p.Model("accounts", {"id": p.Column("i64", primary_key=True)}, hooks={"before_insert": lambda values, insert: values})  # expected-error: arg-type
    await accounts.active({"id": 1}).insert()  # expected-error: call-arg
    insert, update, delete = accounts.insert({"id": 1}), accounts.update({"id": 2}), accounts.delete()
    # [spec:pgorm:req:bindings.model-try-inserts/test]    a try-insert's RETURNING reads one row or a batch by its row count
    await insert.on_conflict_do_nothing().returning().all(connection)  # expected-error: attr-defined
    await accounts.insert_many([]).on_empty_do_nothing().returning().one(connection)  # expected-error: attr-defined
    insert.on_conflict_do_nothing().returning_upsert(connection)  # expected-error: attr-defined
    # [spec:pgorm:req:bindings.model-write-sources/test]    FROM and USING read a model, a table or a FROM item, and an insert reads neither
    update.from_("accounts")  # expected-error: arg-type
    delete.using(accounts.col("id"))  # expected-error: arg-type
    insert.using(accounts)  # expected-error: attr-defined
    insert.where(accounts.col("id") == 1)  # expected-error: attr-defined
    insert.all_rows()  # expected-error: attr-defined
    await insert.returning_change(connection)  # expected-error: attr-defined
    insert.inspect(terminal="returning_changes")  # expected-error: arg-type
    update.on_conflict(p.Conflict.do_nothing())  # expected-error: attr-defined
    await update.all_rows().returning_upsert(connection)  # expected-error: attr-defined
    update.inspect(terminal="returning_upserts")  # expected-error: arg-type
    delete.on_conflict(p.Conflict.do_nothing())  # expected-error: attr-defined
    await delete.all_rows().returning_changes(connection)  # expected-error: attr-defined
    delete.inspect(terminal="returning_change")  # expected-error: call-arg
    # [spec:pgorm:req:bindings.model-writes+3/test]    a model's insert is typed by its row count, each with its own upsert terminal alone
    await insert.returning_upserts(connection)  # expected-error: attr-defined
    insert.inspect(terminal="returning_upserts")  # expected-error: arg-type
    batch = accounts.insert_many([{"id": 1}])
    await batch.returning_upsert(connection)  # expected-error: attr-defined
    batch.inspect(terminal="returning_upsert")  # expected-error: arg-type
    await insert.add({"id": 3}).returning_upsert(connection)  # expected-error: attr-defined
    await batch.add_many([{"id": 4}]).returning_upsert(connection)  # expected-error: attr-defined
    accounts.columns["id"].value(1)  # expected-error: attr-defined
    accounts.columns["id"].validate(p.Value(1, "i64"))  # expected-error: attr-defined
    accounts.columns["id"].describe()  # expected-error: attr-defined
    p.entity("app.Account").col("id").describe()  # expected-error: attr-defined
    # [spec:pgorm:req:bindings.entity-versions+3/test]    an entity's insert is typed by its row count, each with its own upsert terminal alone
    registered = p.entity("app.Account")
    # [spec:pgorm:req:bindings.entity-active-changes/test]    set_if_not_equals names a column and its value; is_changed takes nothing
    registered.active().set_if_not_equals("id")  # expected-error: call-arg
    registered.active().is_changed("id")  # expected-error: call-arg
    await registered.insert(registered.active()).returning_upserts(connection)  # expected-error: attr-defined
    await registered.insert_many([registered.active()]).returning_upsert(connection)  # expected-error: attr-defined
    # [spec:pgorm:req:bindings.entity-try-inserts/test]    a try-insert has TryInsert's terminals of its row count, and no upsert terminal
    await registered.insert(registered.active()).on_conflict_do_nothing().returning_upsert(connection)  # expected-error: attr-defined
    await registered.insert(registered.active()).on_empty_do_nothing().returning_pks(connection)  # expected-error: attr-defined
    await registered.insert_many([]).on_conflict_do_nothing().returning_model(connection)  # expected-error: attr-defined
    tried: int = await registered.insert(registered.active()).on_conflict_do_nothing().execute(connection)  # expected-error: assignment
    # [spec:pgorm:req:bindings.disjoint-unkinded/test]    an unkinded disjoint model has no writes and no schema, and takes no kind options
    shops = p.Disjoint("shops", accounts, {"online": accounts, "dormant": None}, unkinded=True)
    shops.insert({"id": 1}, "online")  # expected-error: misc
    shops.update({"id": 1}, "online")  # expected-error: misc
    shops.change_kind({"id": 1}, "dormant")  # expected-error: misc
    s.create_disjoint(shops)  # expected-error: arg-type
    # [spec:pgorm:req:bindings.disjoint-evolve/test]    an unkinded disjoint model's kinds are not changed
    s.add_disjoint_kind(shops, "online")  # expected-error: arg-type
    s.remove_disjoint_kind_table(shops, "online", "online")  # expected-error: arg-type
    # [spec:pgorm:req:bindings.one-of+1/test]    a one-of reference's CHECK is written from a reference, and its targets are (field, Model) pairs
    s.create_one_of(accounts)  # expected-error: arg-type
    p.OneOf("ref", accounts, {"self": accounts})  # expected-error: dict-item
    p.Disjoint("shops", accounts, {"dormant": None}, unkinded=True, kind="state")  # expected-error: call-overload
    kinded: p.Disjoint = shops  # expected-error: assignment
    # [spec:pgorm:req:bindings.migrations/test]    a migrator takes migrations, runs on a pool, and reads on an executor
    s.Migrations([object()])  # expected-error: list-item
    s.Migrations([s.SqlMigration("m1", "SELECT 1")], migration_table_name=1)  # expected-error: arg-type
    await s.Migrations([]).up(connection)  # expected-error: arg-type
    await s.Migrations([]).up(p.Pool("postgres://"), "1")  # expected-error: arg-type
    s.SqlMigration.read_dir(1)  # expected-error: arg-type
    waiting: list[str] = await s.Migrations([]).get_pending_migrations(connection)  # expected-error: assignment
    # [spec:pgorm:req:bindings.model-projections/test]    a projection reads no rows until into_tuple, into_values or into_model says what each becomes
    projection = accounts.find().select_only().column(accounts.col("id"))
    await projection.all(connection)  # expected-error: attr-defined
    ids: list[int] = await projection.into_tuple().all(connection)  # expected-error: assignment
    projection.expr_as(p.call("count", accounts.col("id")), "n", "i64")  # expected-error: arg-type
    projection.column(p.col("id"))  # expected-error: arg-type
    # [spec:pgorm:req:bindings.partial-models/test]    a partial model's field is a model column, a FromExpr or a Skip
    p.PartialModel(accounts, {"id": "id"})  # expected-error: dict-item
    # [spec:pgorm:req:bindings.cursor-secondary-order+1/test]    a secondary ordering names a model, not a table's name
    accounts.find().cursor("id").add_secondary_order_by("accounts", "id")  # expected-error: arg-type
    # [spec:pgorm:req:bindings.model-clauses/test]    a model query's lock takes a LockStrength
    accounts.find().lock("update")  # expected-error: arg-type

    # [spec:pgorm:req:bindings.template/test]    a template is a t-string, never a str built at run time
    p.template("char_length(name) < 40")  # expected-error: arg-type
    # [spec:pgorm:req:bindings.batch-execute/test]    a script is text, never a RawSQL
    await connection.batch_execute(p.RawSQL("SELECT 1"))  # expected-error: arg-type
    # [spec:pgorm:req:bindings.raw-models/test]    a raw read takes a RawSQL, never bare text
    p.from_statement("SELECT 1")  # expected-error: arg-type
    # [spec:pgorm:req:bindings.json-operators/test]    a JSON path is a list of keys
    p.col("meta").get_json_path("size")  # expected-error: arg-type
    # [spec:pgorm:req:bindings.transaction-retry/test]    retries is a keyword-only int
    await connection.transaction_with_retry(lambda tx: tx.one(p.RawSQL("SELECT 1")), retries="3")  # expected-error: arg-type
