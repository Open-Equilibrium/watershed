//! Sort keys that order siblings: a new key always fits between two neighbors, so placing an
//! object writes only inside that object (ADR-0238).
//!
//! The scheme is fractional indexing as Figma and Linear use it: an integer part whose first
//! character encodes its length, then an optional fraction. Appending or prepending
//! increments or decrements the integer, so keys grow only logarithmically.

const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const SMALLEST: &[u8] = b"A00000000000000000000000000";

fn value(digit: u8) -> usize {
    DIGITS.iter().position(|&d| d == digit).unwrap_or(0)
}

/// A key after `before` and before `after`; `None` is the start or the end. Equal or
/// misordered neighbors, from two devices placing objects at one spot concurrently, get a
/// key after `before`; ties order by identity. A neighbor that is no valid key counts as
/// the start or the end.
pub(crate) fn between(before: Option<&str>, after: Option<&str>) -> String {
    let before = before
        .map(str::as_bytes)
        .filter(|key| integer(key).is_some());
    let after = after
        .map(str::as_bytes)
        .filter(|key| integer(key).is_some())
        .filter(|after| before.is_none_or(|before| before < *after));
    String::from_utf8(key_between(before, after)).expect("ASCII digits")
}

fn key_between(before: Option<&[u8]>, after: Option<&[u8]>) -> Vec<u8> {
    match (before, after) {
        (None, None) => b"a0".to_vec(),
        (None, Some(after)) => {
            let int = integer(after).expect("a valid key");
            if int == SMALLEST {
                return [int, &midpoint(b"", Some(&after[int.len()..]))].concat();
            }
            if int.len() < after.len() {
                return int.to_vec();
            }
            match decrement(int) {
                // The smallest integer is no key on its own, so it takes a fraction.
                Some(smaller) if smaller == SMALLEST => [SMALLEST, &midpoint(b"", None)].concat(),
                Some(smaller) => smaller,
                None => [int, &midpoint(b"", Some(&after[int.len()..]))].concat(),
            }
        }
        (Some(before), None) => {
            let int = integer(before).expect("a valid key");
            increment(int).unwrap_or_else(|| [int, &midpoint(&before[int.len()..], None)].concat())
        }
        (Some(before), Some(after)) => {
            let (low, high) = (
                integer(before).expect("a valid key"),
                integer(after).expect("a valid key"),
            );
            if low == high {
                return [
                    low,
                    &midpoint(&before[low.len()..], Some(&after[high.len()..])),
                ]
                .concat();
            }
            match increment(low) {
                Some(next) if next.as_slice() < after => next,
                _ => [low, &midpoint(&before[low.len()..], None)].concat(),
            }
        }
    }
}

/// The integer part of `key`, whose head `a`–`z` or `Z`–`A` gives its length.
fn integer(key: &[u8]) -> Option<&[u8]> {
    let length = match *key.first()? {
        head @ b'a'..=b'z' => usize::from(head - b'a') + 2,
        head @ b'A'..=b'Z' => usize::from(b'Z' - head) + 2,
        _ => return None,
    };
    let fraction = key.get(length..)?;
    // The smallest integer without a fraction leaves no room before it.
    let valid = key != SMALLEST
        && key[1..].iter().all(|digit| DIGITS.contains(digit))
        && fraction.last() != Some(&b'0');
    valid.then(|| &key[..length])
}

fn increment(int: &[u8]) -> Option<Vec<u8>> {
    let (head, mut digits) = (int[0], int[1..].to_vec());
    for digit in digits.iter_mut().rev() {
        if *digit == DIGITS[DIGITS.len() - 1] {
            *digit = DIGITS[0];
        } else {
            *digit = DIGITS[value(*digit) + 1];
            return Some([&[head], digits.as_slice()].concat());
        }
    }
    match head {
        b'Z' => Some(vec![b'a', DIGITS[0]]),
        b'z' => None,
        _ => {
            let head = head + 1;
            if head > b'a' {
                digits.push(DIGITS[0]);
            } else {
                digits.pop();
            }
            Some([&[head], digits.as_slice()].concat())
        }
    }
}

fn decrement(int: &[u8]) -> Option<Vec<u8>> {
    let (head, mut digits) = (int[0], int[1..].to_vec());
    for digit in digits.iter_mut().rev() {
        if *digit == DIGITS[0] {
            *digit = DIGITS[DIGITS.len() - 1];
        } else {
            *digit = DIGITS[value(*digit) - 1];
            return Some([&[head], digits.as_slice()].concat());
        }
    }
    match head {
        b'a' => Some(vec![b'Z', DIGITS[DIGITS.len() - 1]]),
        b'A' => None,
        _ => {
            let head = head - 1;
            if head < b'Z' {
                digits.push(DIGITS[DIGITS.len() - 1]);
            } else {
                digits.pop();
            }
            Some([&[head], digits.as_slice()].concat())
        }
    }
}

/// A fraction strictly between `before` and `after`, which never ends in `0`. A loop rather
/// than recursion, so a long fraction costs no stack.
fn midpoint(mut before: &[u8], mut after: Option<&[u8]>) -> Vec<u8> {
    let digit = |key: &[u8], i: usize| key.get(i).copied().unwrap_or(DIGITS[0]);
    let mut key = Vec::new();
    loop {
        if let Some(bound) = after {
            let shared = (0..bound.len())
                .take_while(|&i| digit(before, i) == bound[i])
                .count();
            if shared > 0 {
                key.extend_from_slice(&bound[..shared]);
                before = before.get(shared..).unwrap_or(&[]);
                after = Some(&bound[shared..]);
                continue;
            }
        }
        let low = value(digit(before, 0));
        let high = after.map_or(DIGITS.len(), |after| value(after[0]));
        if high - low > 1 {
            key.push(DIGITS[(low + high).div_ceil(2)]);
            return key;
        }
        if let Some(after) = after.filter(|after| after.len() > 1) {
            key.push(after[0]);
            return key;
        }
        key.push(DIGITS[low]);
        before = before.get(1..).unwrap_or(&[]);
        after = None;
    }
}

#[cfg(test)]
mod tests {
    use super::between;

    #[test]
    fn keys_fit_between_any_two_neighbors() {
        let mut keys = vec![between(None, None)];
        for round in 0..200 {
            let i = (round * 7) % (keys.len() + 1);
            let key = between(
                i.checked_sub(1).map(|j| keys[j].as_str()),
                keys.get(i).map(String::as_str),
            );
            keys.insert(i, key);
        }
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn keys_stay_short_when_appending_or_prepending() {
        let (mut last, mut first) = (between(None, None), between(None, None));
        for _ in 0..10_000 {
            let next = between(Some(&last), None);
            let previous = between(None, Some(&first));
            assert!(last < next && previous < first);
            (last, first) = (next, previous);
        }
        assert!(last.len() <= 4 && first.len() <= 4, "{last} {first}");
    }

    #[test]
    fn prepending_at_the_smallest_integer_keeps_the_order() {
        let high = format!("A{}1", "0".repeat(25));
        let first = between(None, Some(&high));
        let second = between(None, Some(&first));
        assert!(second < first && first < high, "{second} {first} {high}");
    }

    #[test]
    fn equal_neighbors_get_a_key_after_both() {
        assert!(between(Some("a1"), Some("a1")).as_str() > "a1");
    }

    #[test]
    fn a_long_fraction_still_gets_a_key() {
        let lower = format!("a0{}", "z".repeat(100_000));
        assert_eq!(between(Some(&lower), Some("a1")), format!("{lower}V"));
    }
}
