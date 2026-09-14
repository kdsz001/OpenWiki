//! Folding overlapping copies of the same passage together. Re-selecting a sentence a few times
//! copies several overlapping pieces of it; only the most complete one is worth keeping.

/// Characters two pieces must share end-to-start before they count as the same passage.
const MIN_OVERLAP: usize = 4;
/// Beyond this length only full containment is checked (the overlap scan grows quadratically).
const MAX_OVERLAP_CHARS: usize = 2000;

/// True when `short` adds nothing worth keeping beyond `long`: it lies inside `long`, or the two
/// overlap end-to-start by at least half of `short` (and at least MIN_OVERLAP characters).
/// Texts that merely begin the same way, like two items of a list, are not fragments.
pub fn is_fragment_of(short: &str, long: &str) -> bool {
    let (short, long) = (short.trim(), long.trim());
    if short.is_empty() {
        return false;
    }
    if long.contains(short) {
        return true;
    }
    let a: Vec<char> = short.chars().collect();
    let b: Vec<char> = long.chars().collect();
    if a.len() > b.len() || a.len() > MAX_OVERLAP_CHARS {
        return false;
    }
    let need = MIN_OVERLAP.max((a.len() + 1) / 2);
    (need..a.len()).any(|k| a[a.len() - k..] == b[..k] || b[b.len() - k..] == a[..k])
}

#[cfg(test)]
mod tests {
    use super::is_fragment_of;

    #[test]
    fn a_piece_inside_a_fuller_copy_is_a_fragment() {
        assert!(is_fragment_of("恢复供电、坏线能换切入", "后能恢复供电、坏线能换切入。"));
    }

    #[test]
    fn copies_overlapping_end_to_start_are_fragments() {
        assert!(is_fragment_of("从耗尽后能恢复供电、", "后能恢复供电、坏线能换切入。"));
        assert!(is_fragment_of("恢复供电、坏线能换切入", "从耗尽后能恢复供电、坏线能"));
    }

    #[test]
    fn items_that_only_begin_the_same_way_stay_apart() {
        assert!(!is_fragment_of("- 修复了 A 问题", "- 修复了 B 问题"));
    }

    #[test]
    fn a_tiny_overlap_is_not_enough() {
        assert!(!is_fragment_of("今天天气不错我们出去", "出去玩吧，别在家里闷着了"));
    }

    #[test]
    fn surrounding_whitespace_is_ignored() {
        assert!(is_fragment_of("  坏线能换切入\n", "恢复供电、坏线能换切入"));
    }

    #[test]
    fn unrelated_texts_are_not_fragments() {
        assert!(!is_fragment_of("词月购买约8,795单", "恢复供电、坏线能换切入"));
    }
}
