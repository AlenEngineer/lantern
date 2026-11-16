use std::cmp::Ordering;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Lowercases and strips diacritics: `"Canción ÑANDÚ"` → `"cancion nandu"`.
///
/// Operators type fast and without accents; this makes that not matter.
pub fn fold(text: &str) -> String {
    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Splits folded text into words. Anything that isn't a letter or a digit
/// separates words, so `"Juan 3:16"` → `["juan", "3", "16"]`.
///
/// A number loses the zeros it starts with: a file called `004` is found
/// by `4`, and `04` finds it too.
pub fn tokens(text: &str) -> Vec<String> {
    fold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|word| match number(word) {
            Some(digits) => digits.to_owned(),
            None => word.to_owned(),
        })
        .collect()
}

/// `word` without the zeros it starts with, if it is a number.
fn number(word: &str) -> Option<&str> {
    if !word.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = word.trim_start_matches('0');
    // All zeros is zero.
    Some(if digits.is_empty() { "0" } else { digits })
}

/// Orders two folded texts the way a person reads them: a run of digits
/// counts as the number it is, so `"himno 4"` comes before `"himno 40"`
/// and both before `"himno 100"`.
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        let (Some(x), Some(y)) = (a.chars().next(), b.chars().next()) else {
            return a.len().cmp(&b.len());
        };
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let (m, rest_a) = digits(a);
            let (n, rest_b) = digits(b);
            // Without leading zeros, the longer number is the larger.
            let order = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
            if order != Ordering::Equal {
                return order;
            }
            (a, b) = (rest_a, rest_b);
        } else if x == y {
            (a, b) = (&a[x.len_utf8()..], &b[y.len_utf8()..]);
        } else {
            return x.cmp(&y);
        }
    }
}

/// The digits `text` starts with, without leading zeros, and what follows.
fn digits(text: &str) -> (&str, &str) {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (run, rest) = text.split_at(end);
    let run = run.trim_start_matches('0');
    (run, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_accents_and_case() {
        assert_eq!(fold("Canción ÁRBOL Ñandú"), "cancion arbol nandu");
    }

    #[test]
    fn tokenizes_on_punctuation() {
        assert_eq!(tokens("Juan 3:16"), ["juan", "3", "16"]);
        assert_eq!(
            tokens("  Santo_santo-SANTO!  "),
            ["santo", "santo", "santo"]
        );
        assert!(tokens(" - ").is_empty());
    }

    #[test]
    fn a_number_drops_its_leading_zeros() {
        assert_eq!(tokens("004"), ["4"]);
        assert_eq!(tokens("Himno 040.mp4"), ["himno", "40", "mp4"]);
        assert_eq!(tokens("000 0"), ["0", "0"]);
        // Only a whole word of digits is a number.
        assert_eq!(tokens("0a7 rv1909"), ["0a7", "rv1909"]);
    }

    #[test]
    fn natural_order_counts_numbers() {
        let mut titles = ["himno 100", "himno 40", "himno 4", "10", "9", "himno"];
        titles.sort_by(|a, b| natural(a, b));
        assert_eq!(
            titles,
            ["9", "10", "himno", "himno 4", "himno 40", "himno 100"]
        );
        assert_eq!(natural("a 007", "a 7"), Ordering::Equal);
        assert_eq!(natural("a2b", "a10"), Ordering::Less);
    }
}
