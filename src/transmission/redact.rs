/// Replaces secret strings in error text before it is logged or stored.
///
/// Errors from HTTP clients quote the request URL, and feed URLs carry
/// tokens in their query. A `Redactor` knows the secret values and blanks
/// them out of any text passed through [`Redactor::apply`].
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    /// Longest first, so a secret that contains another is replaced whole.
    secrets: Vec<String>,
}

/// What a redacted secret is replaced with.
pub const REDACTED: &str = "***";

impl Redactor {
    /// A redactor that changes nothing.
    pub fn none() -> Self {
        Redactor::default()
    }

    /// Registers a secret. Empty strings are ignored. The percent-encoded and
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
