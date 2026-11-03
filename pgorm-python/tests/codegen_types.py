"""Type-check this consumer against the installed generated application stubs."""

from decimal import Decimal
from typing import assert_type
from pgorm import Connection, Interval
from pgorm.app import (
    Deliveries,
    DeliveriesRow,
    Delivery,
    Offers,
    OffersRow,
    Parcel,
    Account,
    AccountActive,
    AccountModel,
    AccountNotes,
    AccountOnly,
    IntervalSpan,
    IntervalSpanActive,
    IntervalSpanModel,
    NoteModel,
    RequiredNotes,
    StockLine,
    StockLineActive,
    StockLineModel,
)


async def consumer(connection: Connection) -> None:
    active = Account.active().set_id(1).set_display_name("Nora").set_note(None)
    assert_type(active, AccountActive)
    model = await active.insert(connection)
    assert_type(model, AccountModel)
    assert_type(model.id, int)
    assert_type(model.display_name, str)
    assert_type(model.note, str | None)
    assert_type(model.mood, str)
    assert_type(model.into_active(), AccountActive)
    assert_type(model.with_value("note", "local"), AccountModel)
    assert_type(await Account.find().all(connection), list[AccountModel])
    assert_type(await Account.find().optional(connection), AccountModel | None)
    assert_type(
        await AccountNotes.find().all(connection),
        list[tuple[AccountModel, NoteModel | None]],
    )
    assert_type(
        await AccountNotes.find().cursor("id").first(2).all(connection),
        list[tuple[AccountModel, NoteModel | None]],
    )
    assert_type(
        await RequiredNotes.find().all(connection), list[tuple[AccountModel, NoteModel]]
    )
    assert_type(await AccountOnly.find().optional(connection), AccountModel | None)
    span = await IntervalSpan.find().one(connection)
    assert_type(span, IntervalSpanModel)
    assert_type(span.months, Interval)
    assert_type(span.maybe, Interval | None)
    assert_type(span.history, list[Interval])
    assert_type(span.budgets, list[Interval] | None)
    assert_type(
        IntervalSpan.active().set_months(Interval(1)).set_budgets(None),
        IntervalSpanActive,
    )
    line = await StockLine.find().one(connection)
    assert_type(line, StockLineModel)
    assert_type(line.qty, int | None)
    assert_type(line.small, int)
    assert_type(line.codes, list[str])
    assert_type(line.labels, list[str] | None)
    assert_type(line.amount, Decimal)
    assert_type(StockLine.active().set_qty(None).set_small(1), StockLineActive)
    delivered = await Deliveries.insert(
        Delivery.active().set_label("box"), "parcel", Parcel.active().set_weight_g(1)
    ).returning_model(connection)
    assert_type(delivered, DeliveriesRow)
    assert_type(await Deliveries.find().kind("parcel").all(connection), list[DeliveriesRow])
    assert_type(await Offers.find().all(connection), list[OffersRow])
