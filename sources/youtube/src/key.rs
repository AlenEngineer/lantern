//! Setting up a YouTube Data API key: checking one, the Google pages that
//! make one, and searching YouTube in the browser without one.
//!
//! A check is one `videos.list` call for a known video with `part=id`,
//! which costs 1 unit of the key's 10,000 a day. What went wrong is read
//! from the reasons in the API's JSON error, not guessed from the HTTP
//! status: a key with the API turned off and a key over its quota both
//! answer 403.

use serde::Deserialize;

const VIDEOS: &str = "https://www.googleapis.com/youtube/v3/videos";

/// A video that is always there: the first one on YouTube.
const KNOWN_VIDEO: &str = "jNQXAC9IVRw";

/// Google Cloud's page to make a project.
pub const CREATE_PROJECT: &str = "https://console.cloud.google.com/projectcreate";

/// Google Cloud's page to turn on the YouTube Data API v3 for a project.
pub const TURN_ON_API: &str =
    "https://console.cloud.google.com/apis/library/youtube.googleapis.com";

/// Google Cloud's page to make an API key.
pub const CREATE_KEY: &str = "https://console.cloud.google.com/apis/credentials";

/// What `!yt` says with neither a key nor yt-dlp: the dashboard looks for
/// it to offer a search in the browser instead.
pub const NEEDS_SETUP: &str =
    "YouTube search needs a YouTube Data API key, or yt-dlp chosen, in Settings → YouTube";

/// What checking a key found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCheck {
    /// The key searches.
    Works,
    /// The key's project hasn't the YouTube Data API turned on. `turn_on`
    /// is the page that does it, for that project when Google says which.
    ApiOff {
        /// The Google Cloud page that turns it on.
        turn_on: String,
    },
    /// Not a key Google knows.
    BadKey,
    /// The key is limited to other APIs, sites or apps.
    Restricted,
    /// Today's quota is spent.
    QuotaUsed,
    /// No answer from YouTube: offline, or blocked.
    Unreachable(String),
    /// An answer this doesn't know, with Google's own words.
    Other(String),
}

impl KeyCheck {
    /// In plain words, for the operator.
    pub fn says(&self) -> String {
        match self {
            KeyCheck::Works => "Works".into(),
            KeyCheck::ApiOff { .. } => {
                "This key doesn't have the YouTube Data API turned on".into()
            }
            KeyCheck::BadKey => "That isn't a valid key".into(),
            KeyCheck::Restricted => {
                "This key is limited to other APIs or apps: allow the YouTube Data API v3 for it"
                    .into()
            }
            KeyCheck::QuotaUsed => "Today's searches are used up".into(),
            KeyCheck::Unreachable(_) => "Can't reach YouTube".into(),
            KeyCheck::Other(message) => format!("YouTube said: {message}"),
        }
    }
}

/// Checks `key` with one call (1 unit of its quota). Blocks.
pub fn check(key: &str) -> KeyCheck {
    match get(VIDEOS, &[("part", "id"), ("id", KNOWN_VIDEO), ("key", key)]) {
        Ok((status, body)) => read_answer(status, &body),
        Err(why) => KeyCheck::Unreachable(why),
    }
}

/// A GET that answers with any status and its body, so an error's reasons
/// can be read. `Err` only when there was no answer at all.
pub(crate) fn get(url: &str, query: &[(&str, &str)]) -> Result<(u16, String), String> {
    let mut request = ureq::get(url).config().http_status_as_error(false).build();
    for (name, value) in query {
        request = request.query(*name, *value);
    }
    let mut response = request.call().map_err(|err| err.to_string())?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| err.to_string())?;
    Ok((status, body))
}

/// What an API answer says about the key it was asked with.
pub fn read_answer(status: u16, body: &str) -> KeyCheck {
    if (200..300).contains(&status) {
        return KeyCheck::Works;
    }
    let Ok(answer) = serde_json::from_str::<Answer>(body) else {
        return KeyCheck::Other(format!("HTTP {status}"));
    };
    let error = answer.error;
    let reasons: Vec<&str> = error
        .errors
        .iter()
        .filter_map(|e| e.reason.as_deref())
        .chain(error.details.iter().filter_map(|d| d.reason.as_deref()))
        .collect();
    let has = |names: &[&str]| reasons.iter().any(|r| names.contains(r));
    if has(&["keyInvalid", "API_KEY_INVALID"]) {
        KeyCheck::BadKey
    } else if has(&["accessNotConfigured", "SERVICE_DISABLED"]) {
        let turn_on = error
            .details
            .iter()
            .find_map(|d| d.metadata.as_ref()?.activation_url.clone())
            .unwrap_or_else(|| TURN_ON_API.to_string());
        KeyCheck::ApiOff { turn_on }
    } else if has(&[
        "quotaExceeded",
        "dailyLimitExceeded",
        "rateLimitExceeded",
        "RATE_LIMIT_EXCEEDED",
    ]) {
        KeyCheck::QuotaUsed
    } else if has(&[
        "API_KEY_SERVICE_BLOCKED",
        "API_KEY_HTTP_REFERRER_BLOCKED",
        "API_KEY_IP_ADDRESS_BLOCKED",
        "API_KEY_ANDROID_APP_BLOCKED",
        "API_KEY_IOS_APP_BLOCKED",
        "ipRefererBlocked",
    ]) {
        KeyCheck::Restricted
    } else {
        KeyCheck::Other(
            error
                .message
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| format!("HTTP {status}")),
        )
    }
}

/// YouTube's own search for `text`, to open in the browser.
pub fn browser_search(text: &str) -> String {
    let query: String = url::form_urlencoded::byte_serialize(text.trim().as_bytes()).collect();
    format!("https://www.youtube.com/results?search_query={query}")
}

/// Google's error answer: `{"error": {"message", "errors": [{"reason"}],
/// "details": [{"reason", "metadata": {"activationUrl"}}]}}`.
#[derive(Deserialize)]
struct Answer {
    error: ApiError,
}

#[derive(Deserialize)]
struct ApiError {
    message: Option<String>,
    #[serde(default)]
    errors: Vec<Reason>,
    #[serde(default)]
    details: Vec<Detail>,
}

#[derive(Deserialize)]
struct Reason {
    reason: Option<String>,
}

#[derive(Deserialize)]
struct Detail {
    reason: Option<String>,
    metadata: Option<Metadata>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    activation_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_that_works_says_so() {
        let body = include_str!("../fixtures/key-works.json");
        assert_eq!(read_answer(200, body), KeyCheck::Works);
        assert_eq!(KeyCheck::Works.says(), "Works");
    }

    #[test]
    fn a_project_without_the_api_gets_the_page_that_turns_it_on() {
        let body = include_str!("../fixtures/key-api-off.json");
        assert_eq!(
            read_answer(403, body),
            KeyCheck::ApiOff {
                turn_on: "https://console.developers.google.com/apis/api/youtube.googleapis.com/overview?project=123456789012".into()
            }
        );
        // Without Google's link, the general page.
        let bare =
            r#"{"error":{"code":403,"message":"off","errors":[{"reason":"accessNotConfigured"}]}}"#;
        assert_eq!(
            read_answer(403, bare),
            KeyCheck::ApiOff {
                turn_on: TURN_ON_API.into()
            }
        );
    }

    #[test]
    fn a_bad_key_and_a_spent_quota_are_told_apart_though_both_can_be_403() {
        let bad = include_str!("../fixtures/key-invalid.json");
        assert_eq!(read_answer(400, bad), KeyCheck::BadKey);
        let quota = include_str!("../fixtures/key-quota.json");
        assert_eq!(read_answer(403, quota), KeyCheck::QuotaUsed);
        assert_eq!(KeyCheck::QuotaUsed.says(), "Today's searches are used up");
        let blocked = include_str!("../fixtures/key-restricted.json");
        assert_eq!(read_answer(403, blocked), KeyCheck::Restricted);
    }

    #[test]
    fn an_answer_it_doesnt_know_keeps_googles_words() {
        let odd = r#"{"error":{"code":500,"message":"Backend Error","errors":[{"reason":"backendError"}]}}"#;
        assert_eq!(
            read_answer(500, odd),
            KeyCheck::Other("Backend Error".into())
        );
        assert_eq!(
            read_answer(502, "<html>"),
            KeyCheck::Other("HTTP 502".into())
        );
    }

    #[test]
    fn the_browser_search_is_the_typed_text_url_encoded() {
        assert_eq!(
            browser_search(" sublime gracia & paz "),
            "https://www.youtube.com/results?search_query=sublime+gracia+%26+paz"
        );
        assert_eq!(
            browser_search("¿Cuán grande es Él?"),
            "https://www.youtube.com/results?search_query=%C2%BFCu%C3%A1n+grande+es+%C3%89l%3F"
        );
    }
}
