//! Reading a subtitle file into a [`Script`]: the encoding, then the
//! format's cues, styles and fonts. See the parent module for the rules.

use std::collections::{HashMap, HashSet};

use super::{
    Cue, Encoding, Font, Format, Script, Style, Track, Unreadable, MAX_CLASSES, MAX_CUES,
    MAX_FONTS, MAX_FORMAT_FIELDS, MAX_NAME_BYTES, MAX_SMI_PARAGRAPHS, MAX_START_DIGITS, MAX_STYLES,
    MAX_STYLE_BYTES,
};

const NO_BREAK_SPACE: char = '\u{a0}';

pub fn read(bytes: &[u8], extension: &str) -> Result<Script, Unreadable> {
    let extension = extension.trim_start_matches('.').to_ascii_lowercase();
    let format = match extension.as_str() {
        "ass" | "ssa" => Format::Ass,
        "srt" => Format::Srt,
        "vtt" => Format::Vtt,
        "smi" | "sami" => Format::Smi,
        "sup" | "idx" => return Err(Unreadable::image()),
        // VobSub's `.sub` is a picture stream; MicroDVD's starts with `{`.
        "sub" if bytes.first() != Some(&b'{') => return Err(Unreadable::image()),
        _ => return Err(Unreadable::new("읽을 수 없는 형식이에요")),
    };
    let (text, encoding) = decode(bytes)
        .ok_or_else(|| Unreadable::new(format!("{}의 인코딩을 알 수 없어요", format.label())))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut script = Script {
        format,
        encoding,
        tracks: Vec::new(),
        styles: None,
        fonts: None,
    };
    match format {
        Format::Ass => {
            let ass = read_ass(text)?;
            script.tracks = vec![Track {
                class: None,
                lang: None,
                cues: ass.cues,
            }];
            script.styles = Some(ass.styles);
            script.fonts = Some(ass.fonts);
        }
        Format::Srt | Format::Vtt => {
            let cues = read_srt(text)?;
            script.tracks = vec![Track {
                class: None,
                lang: None,
                cues,
            }];
        }
        Format::Smi => script.tracks = read_smi(text)?,
    }
    if script.tracks.iter().map(|t| t.cues.len()).sum::<usize>() > MAX_CUES {
        return Err(too_many_cues());
    }
    Ok(script)
}

fn unparsed() -> Unreadable {
    Unreadable::new("자막 형식으로 읽지 못했어요")
}

fn too_many_cues() -> Unreadable {
    Unreadable::new("대사 줄이 너무 많아 내용을 비교하지 않았어요")
}

fn too_long_name() -> Unreadable {
    Unreadable::new("폰트나 언어 구분의 이름이 너무 길어 내용을 비교하지 않았어요")
}

/// The fonts a script uses, one per key with its first spelling, kept to
/// [`MAX_FONTS`] as they are found so a file naming endless fonts never
/// holds them all.
#[derive(Default)]
struct FontSet {
    fonts: Vec<Font>,
    keys: HashSet<String>,
}

impl FontSet {
    fn insert(&mut self, name: &str) -> Result<(), Unreadable> {
        if name.trim().len() > MAX_NAME_BYTES {
            return Err(too_long_name());
        }
        self.add(Font::new(name))
    }

    fn add(&mut self, font: Font) -> Result<(), Unreadable> {
        if font.key.is_empty() || self.keys.contains(&font.key) {
            return Ok(());
        }
        if self.fonts.len() == MAX_FONTS {
            return Err(Unreadable::new(
                "쓰는 폰트가 너무 많아 내용을 비교하지 않았어요",
            ));
        }
        self.keys.insert(font.key.clone());
        self.fonts.push(font);
        Ok(())
    }

    fn extend(&mut self, other: FontSet) -> Result<(), Unreadable> {
        other.fonts.into_iter().try_for_each(|font| self.add(font))
    }

    /// The fonts sorted by key.
    fn into_sorted(self) -> Vec<Font> {
        let mut fonts = self.fonts;
        fonts.sort_by(|a, b| a.key.cmp(&b.key));
        fonts
    }
}

/// The text and the encoding it was in, or `None` when no known encoding
/// reads the bytes without an error.
fn decode(bytes: &[u8]) -> Option<(String, Encoding)> {
    let utf16 = |rest: &[u8], decoder: &'static encoding_rs::Encoding| {
        decoder
            .decode_without_bom_handling_and_without_replacement(rest)
            .map(|text| (text.into_owned(), Encoding::Utf16))
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => {
            Some((String::from_utf8(rest.to_vec()).ok()?, Encoding::Utf8))
        }
        [0xFF, 0xFE, rest @ ..] => utf16(rest, encoding_rs::UTF_16LE),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, encoding_rs::UTF_16BE),
        // UTF-16 without a BOM is full of NULs, which no subtitle has.
        _ if bytes.contains(&0) => None,
        _ => match std::str::from_utf8(bytes) {
            Ok(text) => Some((text.to_owned(), Encoding::Utf8)),
            Err(_) => encoding_rs::EUC_KR
                .decode_without_bom_handling_and_without_replacement(bytes)
                .map(|text| (text.into_owned(), Encoding::Cp949)),
        },
    }
}

// ---------------------------------------------------------------- ASS

struct Ass {
    cues: Vec<Cue>,
    styles: Vec<Style>,
    fonts: Vec<Font>,
}

const ASS_DEFAULT_EVENTS: &str =
    "Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text";

fn read_ass(text: &str) -> Result<Ass, Unreadable> {
    let mut seen_section = false;
    let mut section = String::new();
    let mut events = EventFormat::of(ASS_DEFAULT_EVENTS)?;
    let mut style_format: Vec<String> = Vec::new();
    let mut cues = Vec::new();
    let mut styles = Vec::new();
    let mut fonts = FontSet::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line.to_ascii_lowercase();
            seen_section |= section == "[script info]" || section == "[events]";
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim_start();
        if section == "[events]" {
            match key.as_str() {
                "format" => events = EventFormat::of(value)?,
                "dialogue" => {
                    if let Some((cue, used)) = ass_dialogue(value, &events)? {
                        if cues.len() == MAX_CUES {
                            return Err(too_many_cues());
                        }
                        cues.push(cue);
                        fonts.extend(used)?;
                    }
                }
                _ => {}
            }
        } else if section.ends_with("styles]") && section.starts_with("[v4") {
            match key.as_str() {
                "format" => style_format = format_names(value)?,
                "style" => {
                    if value.len() > MAX_STYLE_BYTES {
                        return Err(Unreadable::new(
                            "스타일 줄이 너무 길어 내용을 비교하지 않았어요",
                        ));
                    }
                    if styles.len() == MAX_STYLES {
                        return Err(Unreadable::new(
                            "스타일이 너무 많아 내용을 비교하지 않았어요",
                        ));
                    }
                    let values: Vec<&str> = value.split(',').map(str::trim).collect();
                    if let Some(name) = values.first() {
                        styles.push(Style {
                            name: (*name).to_owned(),
                            fields: style_format
                                .iter()
                                .zip(&values)
                                .skip(1)
                                .map(|(field, value)| (field.clone(), (*value).to_owned()))
                                .collect(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    if !seen_section {
        return Err(unparsed());
    }
    for style in &styles {
        if let Some((_, name)) = style
            .fields
            .iter()
            .find(|(field, _)| field.eq_ignore_ascii_case("fontname"))
        {
            fonts.insert(name)?;
        }
    }
    sort_cues(&mut cues);
    Ok(Ass {
        cues,
        styles,
        fonts: fonts.into_sorted(),
    })
}

/// The names of a `Format:` line, at most [`MAX_FORMAT_FIELDS`] of them.
/// The line is at most [`MAX_STYLE_BYTES`] too, since every style copies its
/// names.
fn format_names(format: &str) -> Result<Vec<String>, Unreadable> {
    if format.len() > MAX_STYLE_BYTES {
        return Err(Unreadable::new(
            "형식 줄이 너무 길어 내용을 비교하지 않았어요",
        ));
    }
    let names: Vec<String> = format
        .split(',')
        .take(MAX_FORMAT_FIELDS + 1)
        .map(|n| n.trim().to_owned())
        .collect();
    match names.len() > MAX_FORMAT_FIELDS {
        true => Err(Unreadable::new(
            "형식 줄의 항목이 너무 많아 내용을 비교하지 않았어요",
        )),
        false => Ok(names),
    }
}

/// Where an `[Events]` `Format:` line puts the fields a cue needs, found
/// once for the lines after it.
struct EventFormat {
    fields: usize,
    start: Option<usize>,
    end: Option<usize>,
    text: Option<usize>,
    style: Option<usize>,
}

impl EventFormat {
    fn of(format: &str) -> Result<Self, Unreadable> {
        let names = format_names(format)?;
        let index = |name: &str| names.iter().position(|f| f.eq_ignore_ascii_case(name));
        Ok(Self {
            fields: names.len(),
            start: index("start"),
            end: index("end"),
            text: index("text"),
            style: index("style"),
        })
    }
}

/// One `Dialogue:` line's value: the cue, and the fonts its overrides name.
/// `None` for a line that is malformed, empty, or a drawing.
fn ass_dialogue(value: &str, format: &EventFormat) -> Result<Option<(Cue, FontSet)>, Unreadable> {
    let (Some(start), Some(end), Some(text)) = (format.start, format.end, format.text) else {
        return Ok(None);
    };
    let fields: Vec<&str> = value.splitn(format.fields, ',').collect();
    if fields.len() != format.fields {
        return Ok(None);
    }
    let (Some(start_ms), Some(end_ms)) = (parse_time(fields[start]), parse_time(fields[end]))
    else {
        return Ok(None);
    };
    let (shown, fonts) = ass_text(fields[text])?;
    let shown = shown.trim().to_owned();
    let text = collapse(&shown);
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some((
        Cue {
            start: start_ms,
            end: end_ms,
            text,
            shown,
            style: format.style.map(|i| fields[i].trim().to_owned()),
        },
        fonts,
    )))
}

/// The text a viewer sees from an ASS text field, and the fonts its
/// `\fn` overrides name. Text drawn as a vector shape (`\p1`) is dropped.
fn ass_text(raw: &str) -> Result<(String, FontSet), Unreadable> {
    let mut shown = String::new();
    let mut fonts = FontSet::default();
    let mut drawing = false;
    // Once no `}` follows a `{`, none follows a later one either: looking
    // again at each `{` would cost the square of the text's length.
    let mut closes = true;
    let mut rest = raw;
    while let Some(c) = rest.chars().next() {
        if c == '{' && closes {
            match rest.find('}') {
                Some(end) => {
                    for part in rest[1..end].split('\\').skip(1) {
                        override_part(part, &mut fonts, &mut drawing)?;
                    }
                    rest = &rest[end + 1..];
                    continue;
                }
                None => closes = false,
            }
        }
        if c == '\\' {
            let replaced = match rest[1..].chars().next() {
                Some('N' | 'n') => Some('\n'),
                Some('h') => Some(NO_BREAK_SPACE),
                _ => None,
            };
            if let Some(r) = replaced {
                if !drawing {
                    shown.push(r);
                }
                rest = &rest[2..];
                continue;
            }
        }
        if !drawing {
            shown.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    Ok((shown, fonts))
}

/// One `\tag` of an override block: a font name, or whether a vector
/// drawing (`\p1`) starts or ends.
fn override_part(part: &str, fonts: &mut FontSet, drawing: &mut bool) -> Result<(), Unreadable> {
    if let Some(name) = part.strip_prefix("fn") {
        // Inside `\t(...)` the name is followed by the closing `)`.
        let name = match name.contains('(') {
            true => name,
            false => name.trim_end_matches(')'),
        };
        if !name.trim().is_empty() {
            fonts.insert(name)?;
        }
    } else if let Some(digits) = part.strip_prefix('p') {
        if digits.starts_with(|d: char| d.is_ascii_digit()) {
            let digits = digits.trim_end_matches(|d: char| !d.is_ascii_digit());
            *drawing = !digits.trim_start_matches('0').is_empty();
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- SRT

/// SRT and WebVTT: a time line `a --> b` and the text lines after it, up to
/// a blank line. Unparsed when no time line reads.
fn read_srt(text: &str) -> Result<Vec<Cue>, Unreadable> {
    let lines: Vec<&str> = text.lines().collect();
    let mut cues = Vec::new();
    let mut parsed = false;
    let mut i = 0;
    while i < lines.len() {
        let times = lines[i].split_once("-->").and_then(|(a, b)| {
            let end = b.split_whitespace().next()?;
            Some((parse_time(a)?, parse_time(end)?))
        });
        i += 1;
        let Some((start, end)) = times else { continue };
        parsed = true;
        let mut raw = Vec::new();
        while i < lines.len() && !lines[i].trim().is_empty() {
            raw.push(lines[i]);
            i += 1;
        }
        let shown = markup_text(&raw.join("\n"), false);
        let text = collapse(&shown);
        if !text.is_empty() {
            if cues.len() == MAX_CUES {
                return Err(too_many_cues());
            }
            cues.push(Cue {
                start,
                end,
                text,
                shown,
                style: None,
            });
        }
    }
    sort_cues(&mut cues);
    parsed.then_some(cues).ok_or_else(unparsed)
}

// ---------------------------------------------------------------- SMI

/// SMI: `<SYNC Start=ms>` tags, each with one `<P Class=...>` per language.
/// A class's cue runs to the next sync that has that class; a sync whose
/// text is empty (`&nbsp;`) ends it without starting another. Unparsed
/// when the file has no sync.
fn read_smi(text: &str) -> Result<Vec<Track>, Unreadable> {
    // The style rules sit inside a comment (`<!-- ... -->`).
    let langs = style_languages(&text.to_ascii_lowercase());
    let text = without_comments(text);
    let lower = text.to_ascii_lowercase();
    let syncs = tags(&lower, "sync");
    if syncs.is_empty() {
        return Err(unparsed());
    }
    let body_end = lower.find("</body>").unwrap_or(lower.len());
    // Per class (by lowercase name): its first spelling and its events.
    let mut order: Vec<String> = Vec::new();
    let mut classes: HashMap<String, (String, Vec<(u64, String)>)> = HashMap::new();
    let mut paragraphs_kept = 0;
    for (k, &(at, tag_end)) in syncs.iter().enumerate() {
        // Times are milliseconds: a few digits more than any video has, so
        // no sum of them overflows.
        let Some(start) = attr(&text[at..tag_end], "start")
            .filter(|v| v.len() <= MAX_START_DIGITS)
            .and_then(|v| v.parse::<u64>().ok())
        else {
            continue;
        };
        let end = syncs
            .get(k + 1)
            .map_or(body_end, |next| next.0)
            .min(body_end)
            .max(tag_end);
        let content = &text[tag_end..end];
        let content_lower = &lower[tag_end..end];
        let ps = tags(content_lower, "p");
        // Counted before they are kept, so a sync of endless `<P>` holds none.
        paragraphs_kept += ps.len().max(1);
        if paragraphs_kept > MAX_SMI_PARAGRAPHS {
            return Err(too_many_cues());
        }
        let mut paragraphs: Vec<(Option<String>, &str)> = Vec::new();
        if ps.is_empty() {
            paragraphs.push((None, content));
        }
        for (n, &(p_at, p_end)) in ps.iter().enumerate() {
            let until = ps.get(n + 1).map_or(content.len(), |next| next.0);
            let class = attr(&content[p_at..p_end], "class").filter(|c| !c.is_empty());
            paragraphs.push((class, &content[p_end..until]));
        }
        for (class, raw) in paragraphs {
            if class.as_ref().is_some_and(|c| c.len() > MAX_NAME_BYTES) {
                return Err(too_long_name());
            }
            let key = class.clone().unwrap_or_default().to_ascii_lowercase();
            if !classes.contains_key(&key) && order.len() == MAX_CLASSES {
                return Err(Unreadable::new(
                    "언어 구분(Class)이 너무 많아 내용을 비교하지 않았어요",
                ));
            }
            let entry = classes.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                (class.clone().unwrap_or_default(), Vec::new())
            });
            entry.1.push((start, raw.to_owned()));
        }
    }
    // A few class-less paragraphs in a file whose others all have one class
    // are that class's (a `<P>` left without its `Class`).
    let named: Vec<&String> = order.iter().filter(|k| !k.is_empty()).collect();
    let loose = classes
        .get("")
        .map(|(_, events)| {
            events
                .iter()
                .filter(|(_, raw)| !collapse(&markup_text(raw, true)).is_empty())
                .count()
        })
        .unwrap_or(0);
    if let [only] = named[..] {
        if loose <= LOOSE_PARAGRAPHS && classes.contains_key("") {
            let only = only.clone();
            let (_, events) = classes.remove("").ok_or_else(unparsed)?;
            order.retain(|k| !k.is_empty());
            classes
                .get_mut(&only)
                .ok_or_else(unparsed)?
                .1
                .extend(events);
        }
    }
    let mut tracks = Vec::new();
    for key in order {
        let (name, mut events) = classes.remove(&key).ok_or_else(unparsed)?;
        events.sort_by_key(|(start, _)| *start);
        let ends = next_starts(&events);
        let mut cues = Vec::new();
        for ((start, raw), end) in events.iter().zip(ends) {
            let shown = markup_text(raw, true);
            let text = collapse(&shown);
            if text.is_empty() {
                continue;
            }
            cues.push(Cue {
                start: *start,
                end,
                text,
                shown,
                style: None,
            });
        }
        if !cues.is_empty() {
            tracks.push(Track {
                lang: langs.get(&key).cloned(),
                class: Some(name).filter(|n| !n.is_empty()),
                cues,
            });
        }
    }
    Ok(tracks)
}

/// For each event sorted by start, the first later start after its own, or
/// its own start for the last ones: found from the back, so a run of syncs
/// with one start costs no more than the run.
fn next_starts(events: &[(u64, String)]) -> Vec<u64> {
    let mut ends = vec![0; events.len()];
    let mut later = None;
    let mut to = events.len();
    while to > 0 {
        let start = events[to - 1].0;
        let from = events[..to]
            .iter()
            .rposition(|(other, _)| *other != start)
            .map_or(0, |i| i + 1);
        ends[from..to].fill(later.unwrap_or(start));
        later = Some(start);
        to = from;
    }
    ends
}

/// The most paragraphs with text a class-less remainder may have to be
/// counted as the file's one class.
const LOOSE_PARAGRAPHS: usize = 5;

/// The `lang:` of each class the `<STYLE>` block declares
/// (`.KRCC { Name: Korean; lang: ko-KR; }`), by lowercase class name, the
/// value in lowercase.
fn style_languages(lower: &str) -> HashMap<String, String> {
    let mut langs = HashMap::new();
    let Some(start) = lower.find("<style") else {
        return langs;
    };
    let block = &lower[start..];
    let block = &block[..block.find("</style").unwrap_or(block.len())];
    for rule in block.split('}') {
        let Some((selector, declarations)) = rule.split_once('{') else {
            continue;
        };
        let class = selector
            .trim_end()
            .rsplit(|c: char| c.is_whitespace() || c == '>' || c == ';')
            .next()
            .and_then(|s| s.strip_prefix('.'));
        let lang = declarations.split(';').find_map(|d| {
            let (key, value) = d.split_once(':')?;
            (key.trim() == "lang").then(|| value.trim().to_owned())
        });
        if let (Some(class), Some(lang)) = (class, lang) {
            langs.insert(class.to_owned(), lang);
        }
    }
    langs
}

fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("<!--") {
        out.push_str(&rest[..at]);
        match rest[at..].find("-->") {
            Some(end) => rest = &rest[at + end + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Where each `<name ...>` tag is in `lower`: its `<` and its end (see
/// [`tag_end`]). The tag name ends at a space, `>` or `/`.
fn tags(lower: &str, name: &str) -> Vec<(usize, usize)> {
    let open = format!("<{name}");
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find(&open).map(|i| i + from) {
        let after = at + open.len();
        let named = lower[after..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || c == '>' || c == '/');
        if named {
            let (len, _) = tag_end(&lower[at..]);
            found.push((at, at + len));
            from = at + len;
        } else {
            from = after;
        }
    }
    found
}

/// The length of the tag that starts at `s` (a `<`), and whether it has its
/// `>`. A tag that runs into its text without one (no `>` before the next
/// `<` or the line's end) ends after the last `name=value` attribute it
/// has, so the text that follows stays text.
fn tag_end(s: &str) -> (usize, bool) {
    let body = &s[1..];
    match body.find(['<', '>', '\n']) {
        Some(i) if body[i..].starts_with('>') => (i + 2, true),
        _ => (1 + attributes_end(body), false),
    }
}

/// Where the tag name and its `name=value` attributes (the value quoted or
/// not) end in `body`, the tag without its `<`.
fn attributes_end(body: &str) -> usize {
    let name = body.trim_start_matches('/');
    let mut at = body.len() - name.len();
    at += name
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(name.len());
    loop {
        let rest = body[at..].trim_start();
        let n = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        if n == 0 {
            return at;
        }
        let Some(value) = rest[n..].trim_start().strip_prefix('=') else {
            return at;
        };
        let value = value.trim_start();
        let len = match value.chars().next() {
            Some(q @ ('"' | '\'')) => match value[1..].find(q) {
                Some(close) => close + 2,
                None => return at,
            },
            _ => value
                .find(|c: char| c.is_whitespace() || c == '<' || c == '>')
                .unwrap_or(value.len()),
        };
        if len == 0 {
            return at;
        }
        // `value` is the end of `body`.
        at = body.len() - value.len() + len;
    }
}

/// A tag's attribute value (`Start=1000`, `Class="KRCC"`), the name in
/// lowercase letters.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name).map(|i| i + from) {
        from = at + name.len();
        let before_ok = lower[..at]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || c == '"' || c == '\'');
        let after = tag[from..].trim_start();
        let Some(value) = after.strip_prefix('=').filter(|_| before_ok) else {
            continue;
        };
        let value = value.trim_start();
        let (quote, value) = match value.chars().next() {
            Some(q @ ('"' | '\'')) => (Some(q), &value[1..]),
            _ => (None, value),
        };
        let end = value
            .find(|c: char| match quote {
                Some(q) => c == q,
                None => c.is_whitespace() || c == '>' || c == '/',
            })
            .unwrap_or(value.len());
        return Some(value[..end].trim().to_owned());
    }
    None
}

// ---------------------------------------------------------------- shared

fn sort_cues(cues: &mut [Cue]) {
    cues.sort_by(|a, b| (a.start, a.end, &a.text).cmp(&(b.start, b.end, &b.text)));
}

/// `h:mm:ss.cc` (ASS), `hh:mm:ss,mmm` (SRT), `mm:ss.mmm` (WebVTT) in
/// milliseconds. The fraction is a decimal fraction of a second.
fn parse_time(text: &str) -> Option<u64> {
    let digits = |s: &str, max: usize| {
        (!s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u64>().ok())
            .flatten()
    };
    let parts: Vec<&str> = text.trim().split(':').collect();
    let (h, m, s) = match parts[..] {
        [h, m, s] => (digits(h, 6)?, digits(m, 6)?, s),
        [m, s] => (0, digits(m, 6)?, s),
        _ => return None,
    };
    let (whole, fraction) = s.split_once([',', '.']).unwrap_or((s, ""));
    let whole = digits(whole, 6)?;
    let millis = match fraction.is_empty() {
        true => 0,
        false => {
            digits(fraction, 12)?;
            format!("{:0<3}", &fraction[..fraction.len().min(3)])
                .parse()
                .ok()?
        }
    };
    Some(((h * 60 + m) * 60 + whole) * 1000 + millis)
}

/// The text a viewer sees from SRT, WebVTT or SMI markup: `{\...}` blocks
/// and tags removed, `<br>` a line break, entities decoded. SMI's source
/// line breaks are plain blanks (`html_blanks`).
fn markup_text(raw: &str, html_blanks: bool) -> String {
    let mut no_blocks = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find("{\\") {
        no_blocks.push_str(&rest[..at]);
        match rest[at..].find('}') {
            Some(end) => rest = &rest[at + end + 1..],
            None => {
                rest = &rest[at..];
                break;
            }
        }
    }
    no_blocks.push_str(rest);
    if html_blanks {
        no_blocks = no_blocks.replace(['\r', '\n', '\t'], " ");
    }
    let mut stripped = String::with_capacity(no_blocks.len());
    strip_tags(&no_blocks, &mut stripped);
    decode_entities(&stripped.replace('\r', ""))
        .trim()
        .to_owned()
}

/// The tags a subtitle's text may have; an unreadable (`>`-less) one is
/// dropped only when it is one of these, so a `<` in plain text stays.
const KNOWN_TAGS: [&str; 22] = [
    "p", "sync", "font", "br", "i", "b", "u", "s", "strike", "big", "small", "sub", "sup", "ruby",
    "rt", "rp", "span", "div", "center", "body", "sami", "head",
];

/// Removes the tags, `<br>` a line break. A tag with no `>` is dropped up to
/// its last attribute and the text after it stays.
fn strip_tags(text: &str, out: &mut String) {
    let mut rest = text;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tag = &rest[at..];
        let after = &tag[1..];
        let is_tag = after
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!');
        let name = after
            .trim_start_matches('/')
            .split(|c: char| !c.is_ascii_alphanumeric())
            .next()
            .unwrap_or("");
        let (len, closed) = tag_end(tag);
        let known = KNOWN_TAGS.iter().any(|k| name.eq_ignore_ascii_case(k));
        if !is_tag || !(closed || (known && !after.starts_with('!'))) {
            out.push('<');
            rest = after;
            continue;
        }
        if !after.starts_with('/') && name.eq_ignore_ascii_case("br") {
            out.push('\n');
        }
        rest = &tag[len..];
    }
    out.push_str(rest);
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        // A name is short: looking further for its `;` would cost the
        // square of a text full of `&`.
        let named = after
            .bytes()
            .take(11)
            .position(|b| b == b';')
            .map(|end| (&after[..end], end + 1));
        let decoded = match named {
            Some(("amp", n)) => Some(('&', n)),
            Some(("lt", n)) => Some(('<', n)),
            Some(("gt", n)) => Some(('>', n)),
            Some(("quot", n)) => Some(('"', n)),
            Some(("apos", n)) => Some(('\'', n)),
            Some(("nbsp", n)) => Some((NO_BREAK_SPACE, n)),
            Some((number, n)) if number.starts_with('#') => {
                let code = match number[1..].strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number[1..].parse().ok(),
                };
                code.and_then(char::from_u32).map(|c| (c, n))
            }
            // Many SMIs leave the `;` off `&nbsp`.
            _ if after.starts_with("nbsp") => Some((NO_BREAK_SPACE, 4)),
            _ => None,
        };
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = &after[used..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Each line's blanks collapsed to one space, empty lines dropped.
fn collapse(shown: &str) -> String {
    shown
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
