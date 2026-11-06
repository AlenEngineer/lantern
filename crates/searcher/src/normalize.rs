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
pub fn tokens(text: &str) -> Vec<String> {
    fold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
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
}
