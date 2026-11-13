//! Bangs: `!b juan 3:16` asks only the sources whose
//! [bang](lantern_resource::Source::bang) is `b`.

use lantern_resource::Query;

/// Which sources a typed query is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// No bang: every source.
    Everything,
    /// A known bang, and the sources that answer it. The query's source
    /// filter holds their ids.
    Bang(Bang),
    /// A bang no source answers. The query asks no one.
    Unknown(String),
}

/// What the operator typed, read for a leading bang.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// The text after the bang, limited to its sources.
    pub query: Query,
    /// Which sources the query is for.
    pub scope: Scope,
}

/// One bang and the names of the sources that answer it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bang {
    /// Without its `!`, lowercased.
    pub bang: String,
    /// Their display names.
    pub sources: Vec<String>,
    /// Every one of them is [on request](lantern_resource::Capabilities::on_request),
    /// so the bang is worth searching only once the operator asks.
    pub on_request: bool,
}

/// Splits `!b juan 3` into `("b", "juan 3")`. Only a leading bang counts,
/// and `!b` alone has empty text.
pub fn split(text: &str) -> Option<(&str, &str)> {
    let rest = text.trim_start().strip_prefix('!')?;
    let (bang, text) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    Some((bang, text.trim_start()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_leading_bang_splits() {
        assert_eq!(split("!b juan 3"), Some(("b", "juan 3")));
        assert_eq!(
            split("  !yt   amazing grace"),
            Some(("yt", "amazing grace"))
        );
        assert_eq!(split("!f"), Some(("f", "")));
        assert_eq!(split("!"), Some(("", "")));
        assert_eq!(split("juan !b"), None);
    }
}
