//! Tiny fuzzy matcher used by the quick filter, search and command palette.

/// Returns a score if every character of `needle` appears in `haystack` in order
/// (case-insensitive). Higher is better. An empty needle matches everything.
pub fn score(needle: &str, haystack: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = haystack.chars().flat_map(char::to_lowercase).collect();
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();

    // Fast path: plain substring matches rank highest, earlier is better.
    let hay_s: String = hay.iter().collect();
    let needle_s: String = needle.iter().collect();
    if let Some(pos) = hay_s.find(&needle_s) {
        let prefix_bonus = if pos == 0 { 50 } else { 0 };
        return Some(1000 + prefix_bonus - pos as i32 - hay.len() as i32);
    }

    let mut score = 0;
    let mut hi = 0;
    let mut prev_match: Option<usize> = None;
    for nc in needle {
        let mut found = false;
        while hi < hay.len() {
            if hay[hi] == nc {
                score += 10;
                if prev_match == Some(hi.wrapping_sub(1)) {
                    score += 15; // consecutive
                }
                if hi == 0 || matches!(hay[hi - 1], ' ' | '_' | '-' | '.' | '/') {
                    score += 20; // word boundary
                }
                prev_match = Some(hi);
                hi += 1;
                found = true;
                break;
            }
            hi += 1;
        }
        if !found {
            return None;
        }
    }
    Some(score - hay.len() as i32)
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn matches_subsequence() {
        assert!(score("dcm", "Documents").is_some());
        assert!(score("xyz", "Documents").is_none());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn substring_beats_scattered() {
        assert!(score("doc", "doc.txt").unwrap() > score("doc", "d_o_c.txt").unwrap());
        assert!(score("doc", "doc.txt").unwrap() > score("doc", "my_doc.txt").unwrap());
    }
}
