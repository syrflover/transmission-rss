//! AniList's description as plain text (`docs/specs/library.md`, 시즌 정보 and
//! 오른쪽 카드 열): the line breaks and paragraphs stay, a tag is dropped and its
//! text stays, and character references are decoded, so the screen puts text on
//! the page and never markup.
//!
//! - `<br>` (in any case, with or without `/`) is a line break and a paragraph
//!   tag (`<p>`, `</p>`) a paragraph break. Every other tag (`<i>`, `<b>`,
//!   `<script>`, `<svg onload=…>`) goes and what is between its pair stays as
//!   text: `<script>alert(1)</script>` is the text `alert(1)`, never a script.
//! - Something that only looks like a tag stays as it was written: a `<` that
//!   does not start a tag name (`a < b`, `<3`), a tag with no end before the
//!   next `<` or the end of the text, one longer than [`MAX_TAG`].
//! - Character references (`&amp;`, `&#039;`, `&#x1F600;`, and the named ones of
//!   [`NAMED`]) are decoded once, after the tags are gone, so `&lt;br&gt;` is the
//!   text `<br>` and nothing more. A reference the function does not know stays
//!   as it was written.
//! - Control characters other than the line break and the tab are dropped, a
//!   run of more than one blank line becomes one, and every line is trimmed.

/// The longest tag, in bytes, that is taken as one.
const MAX_TAG: usize = 2000;

#[derive(Debug, PartialEq, Eq)]
enum Tag {
    LineBreak,
    ParagraphBreak,
    Other,
}

/// The tag at the start of `text` (which starts with `<`): its length in
/// bytes and what it is, or `None` when it is not a tag.
fn tag_at(text: &str) -> Option<(usize, Tag)> {
    let bytes = text.as_bytes();
    let mut i = 1;
    if bytes.get(i) == Some(&b'/') {
        i += 1;
    }
    let name_start = i;
    if !bytes.get(i).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    while bytes
        .get(i)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-')
    {
        i += 1;
    }
    let name = text[name_start..i].to_ascii_lowercase();
    match bytes.get(i) {
        Some(b'>' | b'/') => {}
        Some(b) if b.is_ascii_whitespace() => {}
        _ => return None,
    }
    let mut quote: Option<u8> = None;
    while i < bytes.len() && i < MAX_TAG {
        let b = bytes[i];
        if b == b'<' {
            return None;
        }
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'>' => {
                    let kind = match name.as_str() {
                        "br" => Tag::LineBreak,
                        "p" => Tag::ParagraphBreak,
                        _ => Tag::Other,
                    };
                    return Some((i + 1, kind));
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// The named references decoded, besides the numeric ones.
const NAMED: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", '\u{A0}'),
    ("iexcl", '¡'),
    ("cent", '¢'),
    ("pound", '£'),
    ("yen", '¥'),
    ("sect", '§'),
    ("copy", '©'),
    ("laquo", '«'),
    ("reg", '®'),
    ("deg", '°'),
    ("plusmn", '±'),
    ("micro", 'µ'),
    ("para", '¶'),
    ("middot", '·'),
    ("raquo", '»'),
    ("frac12", '½'),
    ("iquest", '¿'),
    ("times", '×'),
    ("divide", '÷'),
    ("agrave", 'à'),
    ("aacute", 'á'),
    ("acirc", 'â'),
    ("atilde", 'ã'),
    ("auml", 'ä'),
    ("aring", 'å'),
    ("aelig", 'æ'),
    ("ccedil", 'ç'),
    ("egrave", 'è'),
    ("eacute", 'é'),
    ("ecirc", 'ê'),
    ("euml", 'ë'),
    ("igrave", 'ì'),
    ("iacute", 'í'),
    ("icirc", 'î'),
    ("iuml", 'ï'),
    ("ntilde", 'ñ'),
    ("ograve", 'ò'),
    ("oacute", 'ó'),
    ("ocirc", 'ô'),
    ("otilde", 'õ'),
    ("ouml", 'ö'),
    ("oslash", 'ø'),
    ("ugrave", 'ù'),
    ("uacute", 'ú'),
    ("ucirc", 'û'),
    ("uuml", 'ü'),
    ("yacute", 'ý'),
    ("szlig", 'ß'),
    ("Agrave", 'À'),
    ("Aacute", 'Á'),
    ("Acirc", 'Â'),
    ("Auml", 'Ä'),
    ("Aring", 'Å'),
    ("Eacute", 'É'),
    ("Ouml", 'Ö'),
    ("Uuml", 'Ü'),
    ("ndash", '–'),
    ("mdash", '—'),
    ("lsquo", '‘'),
    ("rsquo", '’'),
    ("sbquo", '‚'),
    ("ldquo", '“'),
    ("rdquo", '”'),
    ("bdquo", '„'),
    ("dagger", '†'),
    ("bull", '•'),
    ("hellip", '…'),
    ("permil", '‰'),
    ("prime", '′'),
    ("Prime", '″'),
    ("lsaquo", '‹'),
    ("rsaquo", '›'),
    ("euro", '€'),
    ("trade", '™'),
    ("larr", '←'),
    ("uarr", '↑'),
    ("rarr", '→'),
    ("darr", '↓'),
    ("hearts", '♥'),
    ("infin", '∞'),
    ("ne", '≠'),
    ("le", '≤'),
    ("ge", '≥'),
];

/// The character a numeric reference stands for, as HTML reads it: the
/// Windows-1252 characters for 128–159, and U+FFFD for what is not a character.
fn numeric(code: u32) -> char {
    const CP1252: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    match code {
        0 => '\u{FFFD}',
        0x80..=0x9F => CP1252[(code - 0x80) as usize],
        _ => char::from_u32(code).unwrap_or('\u{FFFD}'),
    }
}

/// The reference at the start of `text` (which starts with `&`): its length in
/// bytes and the character, or `None`.
fn reference_at(text: &str) -> Option<(usize, char)> {
    let end = text.find(';')?;
    // A reference is short; a `;` far away is not its end.
    if end > 32 {
        return None;
    }
    let body = &text[1..end];
    let character = if let Some(digits) = body.strip_prefix('#') {
        let code = match digits.strip_prefix(['x', 'X']) {
            Some(hex) if !hex.is_empty() && hex.len() <= 7 => u32::from_str_radix(hex, 16).ok()?,
            Some(_) => return None,
            None if !digits.is_empty() && digits.len() <= 8 => digits.parse().ok()?,
            None => return None,
        };
        numeric(code)
    } else {
        NAMED.iter().find(|(name, _)| *name == body)?.1
    };
    Some((end + 1, character))
}

fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let from = &rest[at..];
        match reference_at(from) {
            Some((len, character)) => {
                out.push(character);
                rest = &from[len..];
            }
            None => {
                out.push('&');
                rest = &from[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `source` as plain text (see the module docs).
pub fn plain_text(source: &str) -> String {
    let source = source.replace("\r\n", "\n").replace('\r', "\n");
    let mut stripped = String::with_capacity(source.len());
    let mut rest = source.as_str();
    while let Some(at) = rest.find('<') {
        stripped.push_str(&rest[..at]);
        let from = &rest[at..];
        match tag_at(from) {
            Some((len, tag)) => {
                match tag {
                    Tag::LineBreak => stripped.push('\n'),
                    Tag::ParagraphBreak => stripped.push_str("\n\n"),
                    Tag::Other => {}
                }
                rest = &from[len..];
            }
            None => {
                stripped.push('<');
                rest = &from[1..];
            }
        }
    }
    stripped.push_str(rest);

    let decoded: String = decode(&stripped)
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let mut lines: Vec<&str> = Vec::new();
    let mut blank_run = 0;
    for line in decoded.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_owned()
}

/// The paragraphs of a plain text from [`plain_text`]: the blocks between blank
/// lines, each keeping its own line breaks.
pub fn paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_breaks_and_paragraphs_stay() {
        assert_eq!(plain_text("one<br>two<br><br>three"), "one\ntwo\n\nthree");
        assert_eq!(plain_text("a<BR/>b<br />c<Br  >d"), "a\nb\nc\nd");
        assert_eq!(plain_text("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(plain_text("<p>first</p><p>second</p>"), "first\n\nsecond");
        // More than one blank line becomes one; the ends and the line ends are trimmed.
        assert_eq!(plain_text("  a  <br><br><br><br> b <br>"), "a\n\nb");
        assert_eq!(
            paragraphs(&plain_text("one<br>two<br><br>three<br><br><br>four")),
            ["one\ntwo", "three", "four"]
        );
    }

    #[test]
    fn other_tags_go_and_their_text_stays() {
        assert_eq!(
            plain_text("an <i>italic</i> and <b>bold</b>"),
            "an italic and bold"
        );
        assert_eq!(plain_text("<script>alert(1)</script>"), "alert(1)");
        assert_eq!(plain_text("x<svg onload=alert(1)>y</svg>"), "xy");
        assert_eq!(plain_text("<img src=x onerror=alert(1)>text"), "text");
        assert_eq!(
            plain_text(r#"<a href="https://example.org/?a>b" title='it''s'>link</a>"#),
            "link"
        );
        assert_eq!(plain_text("<!-- note -->kept"), "<!-- note -->kept");
    }

    #[test]
    fn something_that_only_looks_like_a_tag_stays_as_written() {
        assert_eq!(plain_text("a < b and c > d"), "a < b and c > d");
        assert_eq!(plain_text("I <3 it"), "I <3 it");
        // No end before the next `<`: the first `<` is text, the second starts a real tag.
        assert_eq!(plain_text("x <b and <i>y</i>"), "x <b and y");
        assert_eq!(plain_text("unfinished <b"), "unfinished <b");
        let long = format!("<b {}>kept</b>", "a".repeat(MAX_TAG));
        assert!(plain_text(&long).starts_with("<b aaa"));
        // A quote that never closes does not eat the text after it.
        assert_eq!(plain_text(r#"<b title="x>kept"#), r#"<b title="x>kept"#);
    }

    #[test]
    fn character_references_are_decoded_once() {
        assert_eq!(plain_text("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(
            plain_text("it&#039;s &quot;fine&quot;&hellip;"),
            "it's \"fine\"…"
        );
        assert_eq!(plain_text("&#x41;&#66;&#X43;"), "ABC");
        assert_eq!(plain_text("caf&eacute; &mdash; ok"), "café — ok");
        // Decoded after the tags are gone, once: these are text, not markup or another reference.
        assert_eq!(plain_text("&lt;br&gt;"), "<br>");
        assert_eq!(
            plain_text("&lt;script&gt;x&lt;/script&gt;"),
            "<script>x</script>"
        );
        assert_eq!(plain_text("&amp;lt;"), "&lt;");
        // What is not a reference stays.
        assert_eq!(
            plain_text("fish & chips; &unknown; &#; &#x; &#xZZ;"),
            "fish & chips; &unknown; &#; &#x; &#xZZ;"
        );
        // Out of range and surrogates are the replacement character; 128–159 are Windows-1252.
        assert_eq!(
            plain_text("&#1114112;&#xD800;&#0;"),
            "\u{FFFD}\u{FFFD}\u{FFFD}"
        );
        assert_eq!(plain_text("&#150;&#133;"), "–…");
    }

    #[test]
    fn controls_are_dropped_and_text_in_any_script_stays() {
        assert_eq!(plain_text("a\u{0}b\u{7}c\u{202E}d\te"), "abc\u{202E}d\te");
        assert_eq!(
            plain_text("リコリス・リコイル<br>두 번째 줄 &mdash; Привет"),
            "リコリス・リコイル\n두 번째 줄 — Привет"
        );
        assert_eq!(plain_text(""), "");
        assert_eq!(plain_text("<br><br>"), "");
    }

    #[test]
    fn a_long_mixed_description_keeps_its_paragraphs_and_words() {
        let source = "A girl&#039;s tale.<br><br><i>Second</i> paragraph with <b>bold</b> &amp; \
                      a <script>alert(\"x\")</script> and <svg/onload=alert(1)>.<br>\
                      Third line.<br><br>~!Spoiler!~";
        let text = plain_text(source);
        assert_eq!(
            paragraphs(&text),
            [
                "A girl's tale.",
                "Second paragraph with bold & a alert(\"x\") and .\nThird line.",
                "~!Spoiler!~"
            ]
        );
        assert!(!text.contains('<'));
    }
}
