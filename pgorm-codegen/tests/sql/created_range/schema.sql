CREATE TYPE floatrange AS RANGE (SUBTYPE = float8, SUBTYPE_DIFF = float8mi);
CREATE TYPE booking.slot AS RANGE (SUBTYPE = int4);
CREATE TYPE textrange AS RANGE (SUBTYPE = text, COLLATION = "C");
CREATE TABLE measurement (
    id serial PRIMARY KEY,
    spans floatmultirange NOT NULL,
    span floatrange NOT NULL,
    slot booking.slot,
    slots booking.slot_multirange,
    label textrange NOT NULL,
    spares floatrange[],
    ledgers floatmultirange[]
);
