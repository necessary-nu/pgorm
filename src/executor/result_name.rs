//! The one bounding of a composed name to PostgreSQL's identifier limit.
//!
//! PostgreSQL truncates identifiers to 63 bytes (`NAMEDATALEN - 1`) — an
//! alias past the bound is silently cut server-side, so a projection minting
//! `s0_{61 chars}` and a decode requesting all 64 bytes would never meet.
//! Every prefixed name is therefore composed here, by the writers and the
//! readers alike, so both sides of the contract spell a long name the same
//! bounded way; a binding that projects and decodes prefixed sources of its
//! own composes their names through it too. The columns and constraints a
//! disjoint schema names are bounded here as well, so a later statement that
//! names one spells it as the schema did.

/// PostgreSQL's identifier bound: `NAMEDATALEN - 1` bytes.
const IDENT_MAX_BYTES: usize = 63;

/// The result-set name of `col` under `pre`: the plain concatenation when it
/// is strictly under PostgreSQL's 63-byte identifier bound, otherwise the
/// longest UTF-8-whole head within 47 bytes, padded with `_` back to 47 when
/// a multibyte boundary shortened it, followed by the 64-bit FNV-1a hash of
/// the full name as 16 hex digits. The hash covers the whole name, so two
/// long names sharing a head still get distinct spellings.
///
/// Every bounded spelling is therefore exactly 63 bytes — the padding is
/// what keeps that true across every UTF-8 retreat — and a composition of
/// exactly 63 bytes is re-spelled though it would fit, so every plain
/// spelling is strictly shorter than every bounded one and a column
/// literally named like a bounded spelling composes to a different alias
/// instead of silently reading another column's slot. Within the bounded
/// namespace the hash owns the final 16 bytes unconditionally, so equal
/// spellings mean equal hashes of equal-or-colliding full names. What
/// remains is two distinct long names hashing alike — a 64-bit FNV
/// collision the contract accepts and does not check for.
// [spec:pgorm:sem:query.graph.writer+4]
pub fn result_column_name(pre: &str, col: &str) -> String {
    bounded_identifier(format!("{pre}{col}"))
}

/// `full` bounded as [`result_column_name`] bounds its composition: kept
/// when strictly under 63 bytes, otherwise a 47-byte head and the hash of
/// the whole, exactly 63 bytes.
// [spec:pgorm:sem:query.graph.writer+4]
// [spec:pgorm:sem:disjoint.schema+2]    the 63-byte bound on the names a disjoint schema gives
pub(crate) fn bounded_identifier(full: String) -> String {
    if full.len() < IDENT_MAX_BYTES {
        return full;
    }
    let head_max = IDENT_MAX_BYTES - 16;
    let hash = fnv1a(full.as_bytes());
    let mut end = head_max;
    while !full.is_char_boundary(end) {
        end -= 1;
    }
    let pad = "_".repeat(head_max - end);
    format!("{}{pad}{hash:016x}", &full[..end])
}

/// 64-bit FNV-1a. The spelling of a bounded alias must be reproducible by
/// the decode of another build, so the hash has to be stable across
/// releases and platforms — which `DefaultHasher` does not guarantee.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:pgorm:sem:query.graph.writer+4/test]    names under the bound pass
    // through untouched; names at or past it are bounded, deterministic,
    // distinct for shared heads, and never split a UTF-8 sequence
    #[test]
    fn bounded_names_stay_within_the_identifier_limit() {
        assert_eq!(result_column_name("s0_", "id"), "s0_id");
        assert_eq!(result_column_name("", "name"), "name");

        let long_a = "a".repeat(61);
        let bounded = result_column_name("s0_", &long_a);
        assert_eq!(bounded.len(), IDENT_MAX_BYTES);
        assert!(bounded.starts_with("s0_"));
        assert_eq!(bounded, result_column_name("s0_", &long_a));

        // Two names sharing a 47-byte head differ only past the head, so
        // only the hash tells them apart.
        let long_b = format!("{}b", "a".repeat(60));
        assert_ne!(bounded, result_column_name("s0_", &long_b));

        // A multibyte name: the head never splits a UTF-8 sequence, and the
        // padding brings the spelling back to the full 63 bytes.
        let wide = "å".repeat(40);
        let bounded = result_column_name("s0_", &wide);
        assert_eq!(bounded.len(), IDENT_MAX_BYTES);
        assert!(bounded.starts_with("s0_å"));
    }

    // [spec:pgorm:sem:query.graph.writer+4/test]    the plain and bounded
    // namespaces cannot meet: a column literally named like a bounded
    // spelling composes at 63 bytes and is re-spelled through its own hash,
    // so it never aliases the long column's slot
    #[test]
    fn bounded_spellings_never_equal_plain_ones() {
        let long = "a".repeat(61);
        let bounded = result_column_name("s0_", &long);

        // The deterministic adversary: the column whose plain composition IS
        // the long column's bounded spelling.
        let mimic = bounded
            .strip_prefix("s0_")
            .expect("the bounded spelling keeps its prefix");
        assert_eq!(format!("s0_{mimic}").len(), IDENT_MAX_BYTES);
        assert_ne!(result_column_name("s0_", mimic), bounded);

        // Every plain spelling is strictly shorter than every bounded one.
        let at_bound = "b".repeat(60);
        assert_eq!(result_column_name("s0_", &at_bound).len(), IDENT_MAX_BYTES);
        assert_ne!(
            result_column_name("s0_", &at_bound),
            format!("s0_{at_bound}")
        );
        let under_bound = "b".repeat(59);
        assert_eq!(
            result_column_name("s0_", &under_bound),
            format!("s0_{under_bound}")
        );
    }

    // [spec:pgorm:sem:query.graph.writer+4/test]    a UTF-8 boundary retreat
    // cannot shorten a bounded spelling back into the plain namespace: the
    // head is padded to 47 bytes, so the spelling stays exactly 63 for every
    // retreat width a 2-, 3- or 4-byte character can force
    #[test]
    fn utf8_retreat_keeps_bounded_spellings_at_full_width() {
        // Each character straddles byte 47 of the composition (3-byte prefix
        // plus 44 ASCII bytes puts the boundary one byte into it), forcing a
        // retreat of 1, 2 or 3 bytes respectively.
        for wide in ['å', '€', '𝄞'] {
            let col = format!("{}{wide}{}", "a".repeat(43), "b".repeat(16));
            let bounded = result_column_name("s0_", &col);
            assert_eq!(bounded.len(), IDENT_MAX_BYTES, "for {wide}");
            assert!(!bounded.contains(wide), "the head stops before {wide}");

            // The deterministic adversary: a valid plain column spelling the
            // retreated head plus the hash, without the padding.
            let mimic = format!("{}{}", "a".repeat(43), &bounded[47..]);
            assert!(format!("s0_{mimic}").len() < IDENT_MAX_BYTES);
            assert_ne!(result_column_name("s0_", &mimic), bounded, "for {wide}");
        }
    }
}
