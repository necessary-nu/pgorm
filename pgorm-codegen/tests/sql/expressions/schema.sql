-- Tables whose columns carry expressions codegen reads: `stock_item` one of
-- each construct a DEFAULT or a generation expression may hold, `formula` the
-- operators nested every way their precedence matters and the searched CASE,
-- and `stock_code` a key the server computes.
CREATE TABLE stock_item (
    id int GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    code text NOT NULL DEFAULT md5(random()::text),
    name text NOT NULL DEFAULT 'it''s',
    quantity int NOT NULL DEFAULT -1,
    reorder_at bigint NOT NULL DEFAULT 3000000000,
    price numeric NOT NULL DEFAULT 1.50,
    active bool NOT NULL DEFAULT true,
    archived bool NOT NULL DEFAULT false,
    note text DEFAULT NULL,
    stocked_on text NOT NULL DEFAULT CURRENT_DATE::text,
    tag_count int NOT NULL DEFAULT cardinality('{}'::text[]),
    label varchar NOT NULL DEFAULT 'x'::character varying,
    total numeric GENERATED ALWAYS AS (price * quantity) STORED,
    shown text GENERATED ALWAYS AS (upper(name) || ' #' || quantity::text) VIRTUAL,
    in_stock bool GENERATED ALWAYS AS (quantity > 0 AND NOT (note IS NULL) OR active) STORED,
    spare int GENERATED ALWAYS AS ((quantity + 1) * 2 % 7)
);

CREATE TABLE formula (
    id int PRIMARY KEY,
    a int,
    b int,
    c int,
    p bool,
    q bool,
    r bool,
    s text,
    t text,
    not_first bool GENERATED ALWAYS AS ((NOT p) = q) STORED,
    sub_right int GENERATED ALWAYS AS (a - (b - c)) STORED,
    sub_left int GENERATED ALWAYS AS (a - b - c) STORED,
    div_product int GENERATED ALWAYS AS (a / (b * c)) STORED,
    mod_sum int GENERATED ALWAYS AS (a % (b + c)) STORED,
    concat_right text GENERATED ALWAYS AS (s || (t || s)) STORED,
    or_then_and bool GENERATED ALWAYS AS ((p OR q) AND r) STORED,
    and_then_or bool GENERATED ALWAYS AS (p AND (q OR r)) STORED,
    not_and bool GENERATED ALWAYS AS (NOT (p AND q)) STORED,
    not_not bool GENERATED ALWAYS AS (NOT NOT p) STORED,
    not_is_null bool GENERATED ALWAYS AS (NOT p IS NULL) STORED,
    is_null_compared bool GENERATED ALWAYS AS ((a IS NULL) = p) STORED,
    sum_is_null bool GENERATED ALWAYS AS ((a + b) IS NULL) STORED,
    sum_cast text GENERATED ALWAYS AS ((a + b)::text) STORED,
    compare_left bool GENERATED ALWAYS AS ((a = b) = p) STORED,
    compare_right bool GENERATED ALWAYS AS (p = (q = r)) STORED,
    compare_both bool GENERATED ALWAYS AS ((a < b) = (b < c)) STORED,
    concat_compared bool GENERATED ALWAYS AS ((a || s) = t) STORED,
    negative_left int GENERATED ALWAYS AS (-1 * a) STORED,
    negative_right int GENERATED ALWAYS AS (a - -1) STORED,
    coalesced int GENERATED ALWAYS AS (coalesce(a, b, 0)) VIRTUAL,
    spread int GENERATED ALWAYS AS (greatest(a, b) - least(b, c)) STORED,
    nulled int GENERATED ALWAYS AS (nullif(a, 0)) VIRTUAL,
    ranked int GENERATED ALWAYS AS (CASE WHEN a > b THEN a WHEN a < b THEN b ELSE c END) STORED,
    tested bool GENERATED ALWAYS AS (CASE WHEN p THEN q END) STORED,
    case_sum int GENERATED ALWAYS AS ((CASE WHEN p IS NULL THEN 0 ELSE a END) + 1) VIRTUAL
);

CREATE TABLE stock_code (
    base int NOT NULL,
    code int GENERATED ALWAYS AS (base * 2) STORED PRIMARY KEY
);
