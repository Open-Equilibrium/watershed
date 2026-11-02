//! Sort keys that order siblings: a new key always fits between two neighbors, so placing an
//! object writes only inside that object (ADR-0238).

const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn value(digit: u8) -> usize {
    DIGITS.iter().position(|&d| d == digit).unwrap_or(0)
}

/// A key after `before` and before `after`; `None` is the start or the end. Keys never end
/// in `0`, so one always fits before another. Equal neighbors, from two devices placing
/// objects at one spot concurrently, get a key after both; ties order by identity.
pub(crate) fn between(before: Option<&str>, after: Option<&str>) -> String {
    let before = before.unwrap_or("").as_bytes();
    let after = after.map(str::as_bytes).filter(|after| before < *after);
    String::from_utf8(midpoint(before, after)).expect("ASCII digits")
}

fn midpoint(before: &[u8], after: Option<&[u8]>) -> Vec<u8> {
    let digit = |key: &[u8], i: usize| key.get(i).copied().unwrap_or(b'0');
    if let Some(after) = after {
        let shared = (0..after.len())
            .take_while(|&i| digit(before, i) == after[i])
            .count();
        if shared > 0 {
            let mut key = after[..shared].to_vec();
            key.extend(midpoint(
                before.get(shared..).unwrap_or(&[]),
                Some(&after[shared..]),
            ));
            return key;
        }
    }
    let low = value(digit(before, 0));
    let high = after.map_or(DIGITS.len(), |after| value(after[0]));
    if high - low > 1 {
        vec![DIGITS[(low + high) / 2]]
    } else if let Some(after) = after.filter(|after| after.len() > 1) {
        vec![after[0]]
    } else {
        let mut key = vec![DIGITS[low]];
        key.extend(midpoint(before.get(1..).unwrap_or(&[]), None));
        key
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
        assert!(keys.iter().all(|key| !key.ends_with('0')));
    }

    #[test]
    fn equal_neighbors_get_a_key_after_both() {
        assert!(between(Some("i"), Some("i")).as_str() > "i");
    }
}
