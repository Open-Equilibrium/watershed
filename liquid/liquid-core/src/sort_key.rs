//! Sort keys that order siblings: a new key always fits between two neighbors, so placing an
//! object writes only inside that object (ADR-0238).
//!
//! The scheme is fractional indexing as Figma and Linear use it: an integer part whose first
//! character encodes its length, then an optional fraction. Appending or prepending
//! increments or decrements the integer, so keys grow only logarithmically.

use crate::{Error, Place};

const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const SMALLEST: &[u8] = b"A00000000000000000000000000";

/// The longest key the core writes, as long as a name (ADR-0251); a key grows by about one
/// character per six objects placed at one spot, so only some 6,000 of them reach it. A
/// longer stored key, as another device may send, counts as no key.
pub(crate) const MAX_KEY: usize = 1_000;

fn value(digit: u8) -> usize {
    DIGITS.iter().position(|&d| d == digit).unwrap_or(0)
}

/// The key for an item placed at `place` among `sorted`, ordered by key, then identity
/// (ADR-0247); `id` and `key` read an item's identity and key. After an item whose key
/// others share, as two devices placing at one spot leave them, it goes after them all,
/// since only identities order them.
pub(crate) fn at<T, I: PartialEq + ?Sized>(
    sorted: &[T],
    place: Place<&I>,
    id: impl Fn(&T) -> &I,
    key: impl Fn(&T) -> Option<&str>,
) -> Result<String, Error> {
    let index = match place {
        Place::First => 0,
        Place::After(after) => {
            let at = sorted
                .iter()
                .position(|item| id(item) == after)
                .ok_or(Error::NotFound)?;
            let tied = key(&sorted[at]);
            at + 1
                + sorted[at + 1..]
                    .iter()
                    .take_while(|item| key(item) == tied)
                    .count()
        }
        Place::Last => sorted.len(),
    };
    let key_at = |i: usize| sorted.get(i).and_then(&key);
    between(index.checked_sub(1).and_then(key_at), key_at(index))
}

/// The key for an item placed right before `before` among `sorted`, as [at] reads them.
/// Before an item whose key others share, it goes before them all.
pub(crate) fn before<T, I: PartialEq + ?Sized>(
    sorted: &[T],
    before: &I,
    id: impl Fn(&T) -> &I,
    key: impl Fn(&T) -> Option<&str>,
) -> Result<String, Error> {
    let at = sorted
        .iter()
        .position(|item| id(item) == before)
        .ok_or(Error::NotFound)?;
    let tied = key(&sorted[at]);
    let below = sorted[..at]
        .iter()
        .rev()
        .map(&key)
        .find(|other| *other != tied);
    between(below.flatten(), tied)
}

/// A key after `before` and before `after`; `None` is the start or the end. Equal or
/// misordered neighbors, from two devices placing objects at one spot concurrently, get a
/// key after `before`; ties order by identity. A neighbor that is no valid key counts as
/// the start or the end. A key past [`MAX_KEY`] is refused (ADR-0251).
fn between(before: Option<&str>, after: Option<&str>) -> Result<String, Error> {
    let valid = |key: &&[u8]| key.len() <= MAX_KEY && integer(key).is_some();
    let before = before.map(str::as_bytes).filter(valid);
    let after = after
        .map(str::as_bytes)
        .filter(valid)
        .filter(|after| before.is_none_or(|before| before < *after));
    let key = key_between(before, after);
    if key.len() > MAX_KEY {
        return Err(Error::TooLong);
    }
    Ok(String::from_utf8(key).expect("ASCII digits"))
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
    use super::{MAX_KEY, at, between, midpoint};
    use crate::{Error, Place};

    #[test]
    fn a_place_after_tied_keys_goes_after_them_and_before_the_next() {
        // Two devices placed x and y at one spot; ties order by identity.
        let sorted = [("x", "a0"), ("y", "a0"), ("z", "a1")];
        let place = |place| at(&sorted, place, |item| item.0, |item| Some(item.1));
        let after = place(Place::After("x")).unwrap();
        assert!("a0" < after.as_str() && after.as_str() < "a1", "{after}");
        assert!(place(Place::First).unwrap().as_str() < "a0");
        assert!(place(Place::Last).unwrap().as_str() > "a1");
        assert!(matches!(place(Place::After("w")), Err(Error::NotFound)));
    }

    #[test]
    fn keys_fit_between_any_two_neighbors() {
        let mut keys = vec![between(None, None).unwrap()];
        for round in 0..200 {
            let i = (round * 7) % (keys.len() + 1);
            let key = between(
                i.checked_sub(1).map(|j| keys[j].as_str()),
                keys.get(i).map(String::as_str),
            );
            keys.insert(i, key.unwrap());
        }
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn keys_stay_short_when_appending_or_prepending() {
        let (mut last, mut first) = (between(None, None).unwrap(), between(None, None).unwrap());
        for _ in 0..10_000 {
            let next = between(Some(&last), None).unwrap();
            let previous = between(None, Some(&first)).unwrap();
            assert!(last < next && previous < first);
            (last, first) = (next, previous);
        }
        assert!(last.len() <= 4 && first.len() <= 4, "{last} {first}");
    }

    #[test]
    fn prepending_at_the_smallest_integer_keeps_the_order() {
        let high = format!("A{}1", "0".repeat(25));
        let first = between(None, Some(&high)).unwrap();
        let second = between(None, Some(&first)).unwrap();
        assert!(second < first && first < high, "{second} {first} {high}");
    }

    #[test]
    fn equal_neighbors_get_a_key_after_both() {
        assert!(between(Some("a1"), Some("a1")).unwrap().as_str() > "a1");
    }

    #[test]
    fn a_long_fraction_still_gets_a_midpoint() {
        let fraction = "z".repeat(100_000);
        assert_eq!(
            midpoint(fraction.as_bytes(), None),
            format!("{fraction}V").into_bytes()
        );
    }

    #[test]
    fn a_placement_needing_a_key_past_the_cap_is_refused() {
        let place = |lower: &str| {
            let sorted = [("x", lower), ("y", "a1")];
            at(
                &sorted,
                Place::After("x"),
                |item| item.0,
                |item| Some(item.1),
            )
        };
        let longest = format!("a0{}", "z".repeat(MAX_KEY - 3));
        assert_eq!(place(&longest).unwrap().len(), MAX_KEY);
        let full = format!("{longest}z");
        assert!(matches!(place(&full), Err(Error::TooLong)));
    }

    #[test]
    fn a_stored_key_past_the_cap_counts_as_no_key() {
        let over = format!("a0{}", "z".repeat(MAX_KEY));
        assert_eq!(
            between(Some(&over), Some("a1")).unwrap(),
            between(None, Some("a1")).unwrap()
        );
        assert_eq!(
            between(Some("a1"), Some(&over)).unwrap(),
            between(Some("a1"), None).unwrap()
        );
    }
}
