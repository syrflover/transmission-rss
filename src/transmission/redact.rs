/// Replaces secret strings in error text before it is logged or stored.
///
/// Errors from HTTP clients quote the request URL, and feed URLs carry
/// tokens in their query. A `Redactor` knows the secret values and blanks
/// them out of any text passed through [`Redactor::apply`].
///
/// Its `Debug` output only counts the secrets it holds.
#[derive(Clone, Default)]
pub struct Redactor {
    /// Longest first, so a secret that contains another is replaced whole.
    secrets: Vec<String>,
}

impl std::fmt::Debug for Redactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redactor")
            .field("secrets", &format_args!("<{} hidden>", self.secrets.len()))
            .finish()
    }
}

/// What a redacted secret is replaced with.
pub const REDACTED: &str = "***";

/// The shortest channel-URL query value that [`Redactor::add_query_value`]
/// registers, in characters.
///
/// A channel's query values are all secret until the user says otherwise, so
/// they include ones like `r=1080` or `f=0` that are not tokens. Replacing such
/// a value wherever it occurs would garble unrelated text (`HTTP status 503`
/// becoming `5***3`). Real tokens (passkeys, API keys, hashes) are far longer.
/// This limit only concerns value-based replacement in free text: the value of
/// a secret-named query parameter is masked in a URL whatever its length.
pub const MIN_QUERY_SECRET_LEN: usize = 8;

impl Redactor {
    /// A redactor that changes nothing.
    pub fn none() -> Self {
        Redactor::default()
    }

    /// Registers a channel-URL query value as a secret, unless it is shorter than
    /// [`MIN_QUERY_SECRET_LEN`] characters (see there). Otherwise as [`Redactor::add`].
    pub fn add_query_value(&mut self, value: &str) {
        if value.chars().count() >= MIN_QUERY_SECRET_LEN {
            self.add(value);
        }
    }

    /// Registers a secret whatever its length, such as a credential given for
    /// Transmission. Empty strings are ignored. The percent-encoded and
    /// percent-decoded spellings are registered as well, since URLs are
    /// re-encoded on their way through HTTP clients.
    pub fn add(&mut self, secret: &str) {
        if secret.is_empty() {
            return;
        }
        let encoded: String = url::form_urlencoded::byte_serialize(secret.as_bytes()).collect();
        let decoded = percent_decode(secret);
        for form in [secret.to_owned(), encoded, decoded] {
            if !form.is_empty() && !self.secrets.contains(&form) {
                self.secrets.push(form);
            }
        }
        self.secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    }

    pub fn extend(&mut self, other: &Redactor) {
        for secret in &other.secrets {
            if !self.secrets.contains(secret) {
                self.secrets.push(secret.clone());
            }
        }
        self.secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    }

    pub fn apply(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for secret in &self.secrets {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), REDACTED);
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }
}

/// `%XX` and `+` decoding, as a query value is read.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len()
                && bytes[i + 1].is_ascii_hexdigit()
                && bytes[i + 2].is_ascii_hexdigit() =>
            {
                let hex = &text[i + 1..i + 3];
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'%'));
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_every_spelling_of_a_secret() {
        let mut r = Redactor::none();
        r.add("a b&c");
        let text = "raw a b&c, encoded a+b%26c";
        let out = r.apply(text);
        assert!(!out.contains("a b&c") && !out.contains("a+b%26c"), "{out}");
    }

    #[test]
    fn short_query_values_are_not_replaced_in_text_but_credentials_always_are() {
        let mut r = Redactor::none();
        r.add_query_value("0");
        r.add_query_value("1080");
        r.add_query_value("1080p");
        r.add_query_value("1234567"); // one short of the limit
        assert!(r.is_empty());
        let text = "HTTP status 503 for 1080p, page 0, id 1234567";
        assert_eq!(r.apply(text), text);

        // A value at the limit is a secret, in every spelling.
        r.add_query_value("s3cr3t/42");
        assert_eq!(r.apply("x s3cr3t/42 y s3cr3t%2F42 z"), "x *** y *** z");

        // A credential is one whatever its length.
        r.add("pw");
        assert_eq!(r.apply("user:pw@host"), "user:***@host");
    }

    #[test]
    fn debug_shows_no_secret() {
        let mut r = Redactor::none();
        r.add("hunter2-hunter2");
        let shown = format!("{r:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
    }

    #[test]
    fn empty_and_absent_secrets_change_nothing() {
        let mut r = Redactor::none();
        r.add("");
        assert!(r.is_empty());
        assert_eq!(r.apply("nothing to hide"), "nothing to hide");
    }

    #[test]
    fn longer_secrets_are_replaced_whole() {
        let mut r = Redactor::none();
        r.add("abc");
        r.add("abcdef");
        assert_eq!(r.apply("x abcdef y"), "x *** y");
    }
}
