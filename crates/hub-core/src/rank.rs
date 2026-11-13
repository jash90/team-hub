//! Order keys for boards and backlogs: strings that sort lexicographically, with room to
//! put a new one between any two neighbours without renumbering the rest.

const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
const BASE: usize = ALPHABET.len();

fn digit(rank: Option<&str>, index: usize, missing: usize) -> usize {
    rank.and_then(|text| text.as_bytes().get(index))
        .map(|byte| ALPHABET.iter().position(|candidate| candidate == byte).unwrap_or(0))
        .unwrap_or(missing)
}

/// The next rank after `prev` for appending at the end.
///
/// Appended ranks count upwards: a letter saying how many digits follow, then that many
/// base-36 digits (`a1`, `a2`, … `az`, `b01`, …). Ten thousand appends stay within four
/// characters, where halving the remaining space each time would add a character every few
/// issues. A rank that is not of this shape (one made by an insertion, or an old one) is
/// stepped past.
fn rank_after(prev: Option<&str>) -> Option<String> {
    let Some(prev) = prev else { return Some("a1".to_string()) };
    let bytes = prev.as_bytes();
    let head = *bytes.first()?;

    if !head.is_ascii_lowercase() {
        return (head < b'a').then(|| "a1".to_string());
    }

    let width = usize::from(head - b'a') + 1;
    let digits = bytes.get(1..=width).filter(|digits| digits.iter().all(|digit| ALPHABET.contains(digit)));

    let Some(digits) = digits else {
        // Too short to be a counter: start the next width, which sorts after it.
        return (head < b'z').then(|| counter(head + 1, 1));
    };
    let mut next = digits.to_vec();

    for position in (0..next.len()).rev() {
        let value = ALPHABET.iter().position(|candidate| *candidate == next[position]).unwrap_or(0);

        if value + 1 < BASE {
            next[position] = ALPHABET[value + 1];

            return Some(format!("{}{}", head as char, String::from_utf8_lossy(&next)));
        }

        next[position] = ALPHABET[0];
    }

    (head < b'z').then(|| counter(head + 1, 1))
}

/// The counter rank with the given head letter and value.
fn counter(head: u8, value: usize) -> String {
    let width = usize::from(head - b'a') + 1;
    let mut digits = vec![ALPHABET[0]; width];
    let mut rest = value;

    for position in (0..width).rev() {
        digits[position] = ALPHABET[rest % BASE];
        rest /= BASE;
    }

    format!("{}{}", head as char, String::from_utf8_lossy(&digits))
}

/// A rank strictly between `prev` and `next`; either may be absent for "no bound".
/// Ranks never end in the lowest digit, so there is always room below an existing one.
pub fn rank_between(prev: Option<&str>, next: Option<&str>) -> String {
    if next.is_none() {
        if let Some(rank) = rank_after(prev) {
            return rank;
        }
    }

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
    fn appending_ten_thousand_issues_keeps_ranks_short() {
        let mut last: Option<String> = None;

        for _ in 0..10_000 {
            let next = rank_between(last.as_deref(), None);

            assert!(last.as_deref().is_none_or(|last| last < next.as_str()), "{last:?} then {next}");
            last = Some(next);
        }

        assert!(last.is_some_and(|rank| rank.len() <= 4));
    }

    #[test]
    fn appending_steps_past_inserted_and_old_ranks() {
        for prev in ["a5i", "i", "zz", "9", "az", "bzz", "a", "zr", "c0zzi"] {
            let next = rank_between(Some(prev), None);

            assert!(next.as_str() > prev, "{next} should sort after {prev}");
        }

        let between = rank_between(Some("a5"), Some("a6"));
        assert!("a5" < between.as_str() && between.as_str() < "a6");
        assert!(rank_between(None, Some("a1")).as_str() < "a1");
    }

    #[test]
    fn many_ranks_are_ascending() {
        let ranks = ranks_between(Some("c"), Some("d"), 50);

        assert!(ranks.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(ranks.iter().all(|rank| rank.as_str() > "c" && rank.as_str() < "d"));
    }
}
