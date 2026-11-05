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
    optional: p.Record | None = await connection.query(  # expected-error: assignment
        p.select(p.literal(1))
    )
    sqlstate: str = p.DatabaseError("client failure").sqlstate  # expected-error: assignment
    accounts = p.Model("accounts", {"id": p.Column("i64", primary_key=True)})
    p.Model("accounts", {"id": p.Column("i64")}, unique_keys=[{"fields": "id", "nulls_distinct": True}])  # expected-error: typeddict-unknown-key
    p.Column("i32", generated="sometimes")  # expected-error: arg-type
    accounts.graph().join_one(accounts.col("id"))  # expected-error: arg-type
    accounts.find().cursor("id").first("2")  # expected-error: arg-type
    accounts.find().join(accounts.col("id"))  # expected-error: arg-type
    accounts.col("id").between(1, 2, symmetric="yes")  # expected-error: arg-type
    rows: list[p.ModelRecord] = await accounts.graph().all_grouped(connection)  # expected-error: assignment
    insert, update, delete = accounts.insert({"id": 1}), accounts.update({"id": 2}), accounts.delete()
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
    accounts.columns["id"].value(1)  # expected-error: attr-defined
    accounts.columns["id"].validate(p.Value(1, "i64"))  # expected-error: attr-defined
    accounts.columns["id"].describe()  # expected-error: attr-defined
    p.entity("app.Account").col("id").describe()  # expected-error: attr-defined
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
