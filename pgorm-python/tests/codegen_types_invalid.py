"""Each marked misuse must be rejected by the generated type surface."""

from pgorm import Connection
from pgorm.app import Account, AccountNotes, Deliveries, Delivery, Note, Offers


async def invalid(connection: Connection) -> None:
    Account.active().set_id("wrong")  # expected: arg-type
    model = await Account.find().one(connection)
    _name: int = model.display_name  # expected: assignment
    model.missing_field  # expected: attr-defined
    rows = await AccountNotes.find().all(connection)
    rows[0][1].body  # expected: union-attr
    Deliveries.insert(Delivery.active(), "boat")  # expected: arg-type
    Deliveries.insert(Note.active(), "letter")  # expected: arg-type
    Offers.insert(Delivery.active(), "draft")  # expected: attr-defined
