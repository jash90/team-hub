//! Order keys for boards and backlogs: strings that sort lexicographically, with room to
//! put a new one between any two neighbours without renumbering the rest.

const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
const BASE: usize = ALPHABET.len();

fn digit(rank: Option<&str>, index: usize, missing: usize) -> usize {
    rank.and_then(|text| text.as_bytes().get(index))
        .map(|byte| ALPHABET.iter().position(|candidate| candidate == byte).unwrap_or(0))
        .unwrap_or(missing)
}

/// A rank strictly between `prev` and `next`; either may be absent for "no bound".
/// Ranks never end in the lowest digit, so there is always room below an existing one.
pub fn rank_between(prev: Option<&str>, next: Option<&str>) -> String {
    let mut upper = next.filter(|next| prev.is_none_or(|prev| prev < *next));
    let mut result = String::new();

    for index in 0.. {
        let low = digit(prev, index, 0);
        let high = digit(upper, index, BASE);

        if high > low + 1 {
            result.push(ALPHABET[(low + high) / 2] as char);
            break;
        }

        result.push(ALPHABET[low] as char);

        if high == low + 1 {
            upper = None;
        }
    }

    result
}

/// `count` ascending ranks that all fit between `prev` and `next`.
pub fn ranks_between(prev: Option<&str>, next: Option<&str>, count: usize) -> Vec<String> {
    let mut ranks: Vec<String> = Vec::with_capacity(count);

    for _ in 0..count {
        let lower = ranks.last().map(String::as_str).or(prev);
        let rank = rank_between(lower, next);
        ranks.push(rank);
    }

    ranks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_between_its_neighbours() {
        let first = rank_between(None, None);
        let before = rank_between(None, Some(&first));
        let after = rank_between(Some(&first), None);
        let middle = rank_between(Some(&before), Some(&first));

        assert!(before < middle && middle < first && first < after);
    }

    #[test]
    fn adjacent_ranks_still_have_room() {
        let middle = rank_between(Some("a"), Some("b"));

        assert!("a" < middle.as_str() && middle.as_str() < "b");
        assert!(rank_between(None, Some("1")).as_str() < "1");
    }

    #[test]
    fn repeated_insertion_at_the_front_and_back_keeps_order() {
        let mut front = rank_between(None, None);
        let mut back = front.clone();

        for _ in 0..200 {
            let next_front = rank_between(None, Some(&front));
            let next_back = rank_between(Some(&back), None);

            assert!(next_front < front && next_back > back);
            assert!(!next_front.ends_with('0'));
            front = next_front;
            back = next_back;
        }
    }

    #[test]
    fn many_ranks_are_ascending() {
        let ranks = ranks_between(Some("c"), Some("d"), 50);

        assert!(ranks.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(ranks.iter().all(|rank| rank.as_str() > "c" && rank.as_str() < "d"));
    }
}
