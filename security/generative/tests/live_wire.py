"""Check native value snapshots against every value kind the extension advertises.

A kind the portable format carries must pass the format's validator and come
back unchanged through the campaign's public-constructor materializer. A kind it
does not carry must be one of ``NATIVE_ONLY`` (the multiranges and created
ranges matrix.json's ``range-values`` entry declares unsupported), and its
snapshot must still rebuild the same value through pgorm's public constructors,
so the format's boundary is declared rather than silent. Every advertised kind
needs a non-NULL case: a kind without one fails the check by name.
"""

import hashlib
import json
import uuid
from datetime import UTC, date, datetime, time
from decimal import Decimal
from pathlib import Path

import pgorm as p
import pgorm._native as native

from pgorm_campaign import wire
from pgorm_campaign.native_values import materialize, range_value

ENUM = p.TypeName('State" 雪', schema="fixture")
SCALARS = {
    "bool": True,
    "i8": -128,
    "i16": -32768,
    "i32": -(2**31),
    "i64": -(2**63),
    "u32": 2**32 - 1,
    "u64": 2**64 - 1,
    "f32": -0.0,
    "f64": float("inf"),
    "text": "O'Brien 雪; -- %_\\",
    "char": "雪",
    "bytes": b"\x00\xff'",
    "json": None,
    "decimal": Decimal("-0.00100"),
    "uuid": uuid.UUID("12345678-1234-5678-1234-567812345678"),
    "date": date(2024, 1, 2),
    "time": time(3, 4, 5, 123000),
    # Naive on purpose: the `timestamp without time zone` round trip.
    "datetime": datetime(2024, 1, 2, 3, 4, 5, 123456),
    "datetime_utc": datetime(2024, 1, 2, 3, 4, 5, tzinfo=UTC),
    "ipnetwork": "192.0.2.129/24",
    "mac_address": b"\x00\x11\x22\x33\x44\xff",
    "vector": [1.5, -0.0],
    "interval": p.Interval(months=1, days=-2, microseconds=3000004),
}
# Two ascending bounds of each range type's subtype.
RANGES = {
    "int4range": (-(2**31), 2**31 - 1),
    "int8range": (-(2**63), 2**63 - 1),
    "numrange": (Decimal("-0.00100"), Decimal("2.500")),
    "daterange": (date(2024, 1, 2), date(2024, 2, 1)),
    "tsrange": (datetime(2024, 1, 2, 3, 4, 5, 123456), datetime(2024, 1, 3)),
    "tstzrange": (
        datetime(2024, 1, 2, 3, 4, 5, tzinfo=UTC),
        datetime(2024, 1, 2, 3, 4, 5, 1, tzinfo=UTC),
    ),
}
MULTIRANGES = {
    "int4multirange": "int4range",
    "int8multirange": "int8range",
    "nummultirange": "numrange",
    "datemultirange": "daterange",
    "tsmultirange": "tsrange",
    "tstzmultirange": "tstzrange",
}
# Every subtype a created range type can range over, by two ascending bounds.
# The text bounds hold the quotes, backslash, comma, brackets, braces and
# spaces its text form has to quote or escape.
CREATED = {
    "i16": (-(2**15), 2**15 - 1),
    "i32": RANGES["int4range"],
    "i64": RANGES["int8range"],
    "f32": (-0.0, 1.5),
    "f64": (float("-inf"), 2.5),
    "decimal": RANGES["numrange"],
    "text": ("O'Brien 雪; -- %_\\", 'z"],{ ('),
    "date": RANGES["daterange"],
    "time": (time(3, 4, 5, 123000), time(23, 59, 59, 999999)),
    "datetime": RANGES["tsrange"],
    "datetime_utc": RANGES["tstzrange"],
    "uuid": (uuid.UUID(int=1), uuid.UUID("12345678-1234-5678-1234-567812345678")),
}
CREATED_KINDS = {
    "created_range": p.CreatedRange,
    "created_multirange": p.CreatedMultirange,
}
NATIVE_ONLY = frozenset(MULTIRANGES) | frozenset(CREATED_KINDS)


def shapes(lower, upper):
    """Every bracket pairing, each side unbounded, both, and the empty range."""
    return [
        *(p.Range(lower, upper, bounds) for bounds in ("[)", "[]", "()", "(]")),
        p.Range(None, upper, "(]"),
        p.Range(lower, None, "[)"),
        p.Range(),
        p.Range.empty(),
    ]


def inputs():
    yield from SCALARS.items()
    yield ENUM, "O'Brien 雪"
    for kind, bounds in RANGES.items():
        for shape in shapes(*bounds):
            yield kind, shape
    for kind, member in MULTIRANGES.items():
        yield kind, p.Multirange(shapes(*RANGES[member]))
        yield kind, p.Multirange()
    for subtype, bounds in CREATED.items():
        name = f'Span" {subtype} 雪'
        for shape in shapes(*bounds):
            yield p.CreatedRange(name, subtype, schema="fixture"), shape
        spans = p.CreatedMultirange(name + " multi", subtype, schema="fixture")
        yield spans, p.Multirange(shapes(*bounds))
        yield spans, p.Multirange()


def values(kind, data):
    """The value, its typed NULL and its arrays."""
    scalar = (
        p.Value.json(data)
        if isinstance(kind, str) and kind == "json"
        else p.Value(data, kind)
    )
    yield scalar
    yield p.Value.null(kind)
    yield p.Value.array(kind, [scalar, None])
    yield p.Value.array(kind, [])
    yield p.Value.array(kind, None)


def carried(tag):
    try:
        wire.type_tag(tag)
    except wire.FormatError:
        return False
    return True


def kind_of(tag):
    if tag["kind"] in CREATED_KINDS:
        created = CREATED_KINDS[tag["kind"]]
        return created(tag["name"], tag["subtype"], schema=tag["schema"])
    return tag["kind"]


def rebuild(snapshot):
    """The same value through pgorm's public constructors, from its snapshot."""
    tag = snapshot["type"]
    if tag["kind"] == "array":
        items = (
            None
            if snapshot["sql_null"]
            else [rebuild(item) for item in snapshot["data"]]
        )
        return p.Value.array(kind_of(tag["element"]), items)
    if snapshot["sql_null"]:
        return p.Value.null(kind_of(tag))
    if tag["kind"] in MULTIRANGES:
        # Each member travels as the portable format's own range payload.
        member = {"kind": MULTIRANGES[tag["kind"]]}
        ranges = [
            range_value(member, wire.scalar(member, item)["data"], p)
            for item in snapshot["data"]
        ]
        return p.Value(p.Multirange(ranges), tag["kind"])
    # A created range or multirange travels as its text form.
    return p.Value(snapshot["data"], kind_of(tag))


def check(value):
    snapshot = value.snapshot()
    assert json.loads(json.dumps(snapshot)) == snapshot
    tag = snapshot["type"]
    kind = (tag["element"] if tag["kind"] == "array" else tag)["kind"]
    if carried(tag):
        assert kind not in NATIVE_ONLY, (
            f"{kind}: the portable format carries it, so it is not NATIVE_ONLY"
        )
        assert wire.validate(json.loads(json.dumps(snapshot))) == snapshot
        rebuilt = materialize(snapshot, p)
    else:
        assert kind in NATIVE_ONLY, f"{kind}: the portable format refuses it"
        rebuilt = rebuild(snapshot)
    assert rebuilt.snapshot() == snapshot, f"{kind}: {snapshot} does not rebuild"
    return snapshot


def created_subtypes(advertised):
    """Every advertised kind outside CREATED is refused as a created subtype."""
    for subtype in sorted(advertised - CREATED.keys()):
        for created in CREATED_KINDS.values():
            try:
                created("span", subtype)
            except p.ConstructionError:
                continue
            raise AssertionError(f"{subtype}: a created subtype with no case")


# [spec:pgorm:req:generative.format/test]
# [spec:pgorm:req:generative.execution/test]
def main():
    advertised = set(p.capabilities()["value_types"])
    observed = [check(value) for kind, data in inputs() for value in values(kind, data)]
    created_subtypes(advertised)
    constructed = {
        snapshot["type"]["kind"] for snapshot in observed if not snapshot["sql_null"]
    }
    missing, unadvertised = advertised - constructed, constructed - advertised
    assert not missing and not unadvertised, (
        f"advertised kinds with no case: {sorted(missing)}; "
        f"cases of kinds not advertised: {sorted(unadvertised)}"
    )
    report = {
        "passed": True,
        "portable_kinds": sorted(constructed - NATIVE_ONLY),
        "native_only_kinds": sorted(NATIVE_ONLY),
        "snapshots": len(observed),
        "native_sha256": hashlib.sha256(Path(native.__file__).read_bytes()).hexdigest(),
        "values_sha256": hashlib.sha256(
            json.dumps(observed, sort_keys=True).encode()
        ).hexdigest(),
    }
    output = Path("target/generative-program")
    output.mkdir(parents=True, exist_ok=True)
    (output / "values.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
