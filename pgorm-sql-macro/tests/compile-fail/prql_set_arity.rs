use pgorm_sql_macro::prql;

// `intersect` is INTERSECT ALL of whole rows by position, so operands whose
// projections differ in arity are refused at build time rather than joined
// on the positions they share.
fn main() {
    let _ = prql!(
        "from album | select {artist_id, title} | intersect (from artist | select {artist_id})"
    );
}
