//! Made samples only; no downloaded subtitle file is kept here.

use super::*;

fn ass(styles: &[(&str, &str)], lines: &[(&str, &str, &str, &str)]) -> String {
    let mut out = String::from(
        "[Script Info]\nTitle: sample\n\n[V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, Bold, Italic, Alignment\n",
    );
    for (name, font) in styles {
        out += &format!("Style: {name},{font},20,&H00FFFFFF,0,0,2\n");
    }
    out += "\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n";
    for (start, end, style, text) in lines {
        out += &format!("Dialogue: 0,{start},{end},{style},,0,0,0,,{text}\n");
    }
    out
}

fn read_ass(text: &str) -> Script {
    read(text.as_bytes(), "ass").unwrap()
}

/// Thirty lines, one every ten seconds, `0:00:10.00` on.
fn thirty() -> Vec<(String, String, String)> {
    (1..=30)
        .map(|n| {
            (
                format!("0:{:02}:{:02}.00", n / 6, n % 6 * 10),
                format!("0:{:02}:{:02}.50", n / 6, n % 6 * 10 + 5),
                format!("line {n}"),
            )
        })
        .collect()
}

fn ass_of(lines: &[(String, String, String)]) -> String {
    let lines: Vec<(&str, &str, &str, &str)> = lines
        .iter()
        .map(|(s, e, t)| (s.as_str(), e.as_str(), "Default", t.as_str()))
        .collect();
    ass(&[("Default", "Arial")], &lines)
}

#[test]
fn an_ass_with_two_lines_added_and_twelve_changed() {
    let old = thirty();
    let mut new = old.clone();
    for (n, line) in new
        .iter_mut()
        .enumerate()
        .filter(|(n, _)| n % 2 == 0)
        .take(12)
    {
        line.2 = format!("fixed {n}");
    }
    // Two lines between others, at times no old line has; one of them next to
    // a changed line.
    new.push(("0:00:12.00".into(), "0:00:13.00".into(), "added a".into()));
    new.push(("0:02:31.00".into(), "0:02:32.00".into(), "added b".into()));
    let diff = compare(&read_ass(&ass_of(&old)), &read_ass(&ass_of(&new)));
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (2, 12, 0)
    );
    assert_eq!(diff.dialogue.lines.len(), 14);
    assert_eq!(diff.timing.count, 0);
    let changed: Vec<&DialogueLine> = diff
        .dialogue
        .lines
        .iter()
        .filter(|l| l.kind == Change::Changed)
        .collect();
    assert_eq!(changed[0].old.as_ref().unwrap().text, "line 1");
    assert_eq!(changed[0].new.as_ref().unwrap().text, "fixed 0");
    let added: Vec<&DialogueLine> = diff
        .dialogue
        .lines
        .iter()
        .filter(|l| l.kind == Change::Added)
        .collect();
    assert!(added.iter().all(|l| l.old.is_none()));
    let texts: Vec<&str> = added
        .iter()
        .map(|l| l.new.as_ref().unwrap().text.as_str())
        .collect();
    assert_eq!(texts, ["added a", "added b"]);
    assert!(diff.differs());
    // Lines are in time order.
    let starts: Vec<u64> = diff
        .dialogue
        .lines
        .iter()
        .map(|l| l.new.as_ref().or(l.old.as_ref()).unwrap().start)
        .collect();
    assert!(starts.windows(2).all(|w| w[0] <= w[1]), "{starts:?}");
}

#[test]
fn a_removed_line_has_no_new_text() {
    let old = thirty();
    let mut new = old.clone();
    new.remove(4);
    let diff = compare(&read_ass(&ass_of(&old)), &read_ass(&ass_of(&new)));
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (0, 0, 1)
    );
    let line = &diff.dialogue.lines[0];
    assert_eq!(line.kind, Change::Removed);
    assert!(line.new.is_none());
    assert_eq!(line.old.as_ref().unwrap().text, "line 5");
}

#[test]
fn an_srt_with_only_its_timings_shifted() {
    let srt = |shift: [u32; 5]| {
        (0..5)
            .map(|n| {
                let at = n * 10 + 1;
                format!(
                    "{}\r\n00:00:{:02},{:03} --> 00:00:{:02},{:03}\r\nline <i>{}</i>\r\n\r\n",
                    n + 1,
                    at,
                    shift[n as usize],
                    at + 2,
                    shift[n as usize],
                    n
                )
            })
            .collect::<String>()
    };
    // Three lines move by half a second, one by 10 ms (not a move), one not.
    let old = read(srt([0, 0, 0, 0, 0]).as_bytes(), "srt").unwrap();
    let new = read(srt([500, 500, 10, 500, 0]).as_bytes(), "SRT").unwrap();
    let diff = compare(&old, &new);
    assert_eq!(diff.dialogue, Dialogue::default());
    assert_eq!(diff.timing.count, 3);
    assert_eq!(diff.timing.lines.len(), 3);
    assert_eq!(diff.timing.lines[0].text, "line 0");
    assert_eq!(
        diff.timing.lines[0].old,
        Span {
            start: 1000,
            end: 3000
        }
    );
    assert_eq!(
        diff.timing.lines[0].new,
        Span {
            start: 1500,
            end: 3500
        }
    );
    assert!(diff.differs());
}

#[test]
fn a_style_whose_font_name_alone_changed() {
    let lines = [("0:00:01.00", "0:00:02.00", "Default", "hello")];
    let old = read_ass(&ass(&[("Default", "Arial"), ("Sign", "Gulim")], &lines));
    let new = read_ass(&ass(
        &[("Default", "Noto Sans CJK KR"), ("Sign", "Gulim")],
        &lines,
    ));
    let diff = compare(&old, &new);
    assert_eq!(diff.dialogue, Dialogue::default());
    assert_eq!(diff.timing, Timing::default());
    let styles = diff.styles.clone().unwrap();
    assert!(styles.added.is_empty() && styles.removed.is_empty());
    assert_eq!(
        styles.changed,
        vec![StyleChange {
            name: "Default".into(),
            fields: vec![FieldChange {
                field: "Fontname".into(),
                old: "Arial".into(),
                new: "Noto Sans CJK KR".into(),
            }],
        }]
    );
    let fonts = diff.fonts.clone().unwrap();
    assert_eq!(fonts.added, ["Noto Sans CJK KR"]);
    assert_eq!(fonts.removed, ["Arial"]);
    assert!(diff.differs());
}

#[test]
fn styles_added_and_removed_and_same_numbers_and_colours() {
    let lines = [("0:00:01.00", "0:00:02.00", "Default", "hello")];
    let old = ass(&[("Default", "Arial"), ("Old", "Arial")], &lines);
    let new = ass(&[("DEFAULT", "arial"), ("New", "Arial")], &lines)
        .replace("20,&H00FFFFFF", "20.0,&H00ffffff");
    let diff = compare(&read_ass(&old), &read_ass(&new));
    let styles = diff.styles.unwrap();
    assert_eq!(styles.added, ["New"]);
    assert_eq!(styles.removed, ["Old"]);
    assert!(styles.changed.is_empty(), "{:?}", styles.changed);
    let fonts = diff.fonts.unwrap();
    assert!(fonts.added.is_empty() && fonts.removed.is_empty());
}

#[test]
fn inline_font_overrides_are_fonts_used() {
    let text = ass(
        &[("Default", "@Arial")],
        &[(
            "0:00:01.00",
            "0:00:02.00",
            "Default",
            r"{\fnMalgun Gothic\b1}a{\t(0,100,\fnGulim)}b {\fs30}c",
        )],
    );
    let script = read_ass(&text);
    let fonts: Vec<&str> = script
        .fonts
        .as_ref()
        .unwrap()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(fonts, ["Arial", "Gulim", "Malgun Gothic"]);
}

fn smi_text() -> String {
    "<SAMI>\r\n<HEAD><TITLE>x</TITLE>\r\n<STYLE TYPE=\"text/css\"><!--\r\n\
     .KRCC { Name: Korean; lang: ko-KR; }\r\n--></STYLE></HEAD>\r\n<BODY>\r\n\
     <SYNC Start=1000><P Class=KRCC>안녕하세요<br>반갑습니다\r\n\
     <SYNC Start=3000><P Class=KRCC>&nbsp;\r\n\
     <SYNC Start=5000><P Class=KRCC><font color=\"red\">네 &amp; 아니요</font>\r\n\
     <SYNC Start=7500><P Class=KRCC>&nbsp;\r\n</BODY>\r\n</SAMI>\r\n"
        .to_owned()
}

#[test]
fn the_same_smi_in_cp949_and_in_utf8() {
    let text = smi_text();
    let (cp949, _, errors) = encoding_rs::EUC_KR.encode(&text);
    assert!(!errors);
    assert!(std::str::from_utf8(&cp949).is_err());
    let legacy = read(&cp949, "smi").unwrap();
    let modern = read(text.as_bytes(), ".SMI").unwrap();
    assert_eq!(legacy.encoding, Encoding::Cp949);
    assert_eq!(modern.encoding, Encoding::Utf8);
    let diff = compare(&legacy, &modern);
    assert_eq!(diff.dialogue, Dialogue::default());
    assert_eq!(diff.timing, Timing::default());
    assert!(!diff.differs());
    assert_eq!(diff.old.encoding, Encoding::Cp949);
    assert_eq!(diff.new.encoding, Encoding::Utf8);
    assert_eq!((diff.old.cues, diff.new.cues), (2, 2));
}

#[test]
fn smi_syncs_with_nbsp_end_the_cue_and_are_no_cue() {
    let script = read(smi_text().as_bytes(), "smi").unwrap();
    assert_eq!(script.tracks.len(), 1);
    let cues = &script.tracks[0].cues;
    assert_eq!(cues.len(), 2);
    assert_eq!(cues[0].text, "안녕하세요\n반갑습니다");
    assert_eq!((cues[0].start, cues[0].end), (1000, 3000));
    assert_eq!(cues[1].text, "네 & 아니요");
    assert_eq!((cues[1].start, cues[1].end), (5000, 7500));
    // The class of the only language is kept.
    assert_eq!(script.tracks[0].class.as_deref(), Some("KRCC"));
}

#[test]
fn an_smi_with_two_language_classes() {
    let two = |english: &str| {
        format!(
            "<SAMI><BODY>\n\
             <SYNC Start=1000><P Class=KRCC>안녕<P Class=ENCC>Hello\n\
             <SYNC Start=2000><P Class=KRCC>&nbsp;<P Class=ENCC>&nbsp;\n\
             <SYNC Start=3000><P Class=KRCC>잘 가<P Class=ENCC>{english}\n\
             <SYNC Start=4000><P Class=KRCC>&nbsp;<P Class=ENCC>&nbsp;\n\
             </BODY></SAMI>"
        )
    };
    let old = read(two("Bye").as_bytes(), "smi").unwrap();
    let new = read(two("Goodbye").as_bytes(), "smi").unwrap();
    let classes: Vec<_> = old.tracks.iter().map(|t| t.class.as_deref()).collect();
    assert_eq!(classes, [Some("KRCC"), Some("ENCC")]);
    let diff = compare(&old, &new);
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (0, 1, 0)
    );
    let line = &diff.dialogue.lines[0];
    assert_eq!(line.class.as_deref(), Some("ENCC"));
    assert_eq!(line.old.as_ref().unwrap().text, "Bye");
    assert_eq!(line.new.as_ref().unwrap().text, "Goodbye");
    assert_eq!(diff.new.cues, 4);
}

#[test]
fn a_language_class_only_one_side_has_is_all_added_or_removed() {
    let with = |classes: &[(&str, &str)]| {
        let paragraphs: String = classes
            .iter()
            .map(|(class, text)| format!("<P Class={class}>{text}"))
            .collect();
        format!(
            "<SAMI><BODY><SYNC Start=1000>{paragraphs}\n\
             <SYNC Start=2000><P>&nbsp;</BODY></SAMI>"
        )
    };
    let old = read(with(&[("KRCC", "안녕"), ("ENCC", "Hi")]).as_bytes(), "smi").unwrap();
    let new = read(
        with(&[("krcc", "안녕"), ("JPCC", "やあ")]).as_bytes(),
        "smi",
    )
    .unwrap();
    let diff = compare(&old, &new);
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (1, 0, 1)
    );
}

#[test]
fn smi_against_ass_compares_the_dialogue_and_says_what_it_did_not() {
    let smi = read(smi_text().as_bytes(), "smi").unwrap();
    let ass = read_ass(&ass(
        &[("Default", "Arial")],
        &[
            (
                "0:00:01.00",
                "0:00:03.00",
                "Default",
                r"안녕하세요\N반갑습니다",
            ),
            ("0:00:05.00", "0:00:07.50", "Default", "{\\i1}네 & 아니요"),
        ],
    ));
    let diff = compare(&smi, &ass);
    assert!(!diff.differs(), "{diff:?}");
    assert_eq!(diff.styles, None);
    assert_eq!(diff.fonts, None);
    let items: Vec<Item> = diff.not_compared.iter().map(|n| n.item).collect();
    assert_eq!(items, [Item::Styles, Item::Fonts]);
    assert!(diff.not_compared[0].reason.contains("SMI"));
}

#[test]
fn an_ass_with_one_start_time_moved_counts_one_timing_change_and_no_dialogue_change() {
    let old = thirty();
    let mut new = old.clone();
    // The first line starts half a second later; its end and text stay.
    new[0].0 = "0:00:10.50".into();
    let diff = compare(&read_ass(&ass_of(&old)), &read_ass(&ass_of(&new)));
    assert_eq!(diff.dialogue, Dialogue::default());
    assert_eq!(diff.timing.count, 1);
    assert_eq!(diff.timing.lines[0].text, "line 1");
    assert_eq!(
        (
            diff.timing.lines[0].old.start,
            diff.timing.lines[0].new.start
        ),
        (10_000, 10_500)
    );
    assert!(diff.differs());
}

#[test]
fn an_ass_against_an_srt_of_the_same_dialogue_does_not_differ_and_says_styles_and_fonts_were_not_compared(
) {
    let srt = read(
        "1\n00:00:01,000 --> 00:00:03,000\nhello\n\n2\n00:00:05,000 --> 00:00:07,500\nworld\n"
            .as_bytes(),
        "srt",
    )
    .unwrap();
    let ass = read_ass(&ass(
        &[("Default", "Arial")],
        &[
            ("0:00:01.00", "0:00:03.00", "Default", "hello"),
            ("0:00:05.00", "0:00:07.50", "Default", "world"),
        ],
    ));
    let diff = compare(&srt, &ass);
    assert!(!diff.differs(), "{diff:?}");
    assert_eq!(diff.dialogue, Dialogue::default());
    assert_eq!(diff.timing, Timing::default());
    assert_eq!((diff.styles.as_ref(), diff.fonts.as_ref()), (None, None));
    let items: Vec<Item> = diff.not_compared.iter().map(|n| n.item).collect();
    assert_eq!(items, [Item::Styles, Item::Fonts]);
    assert!(diff.not_compared[0].reason.contains("SRT"));
}

#[test]
fn srt_against_srt_says_there_are_no_styles() {
    let srt = "1\n00:00:01,000 --> 00:00:02,000\nhi\n";
    let script = read(srt.as_bytes(), "srt").unwrap();
    let diff = compare(&script, &script);
    assert!(!diff.differs());
    assert_eq!(diff.not_compared.len(), 2);
}

#[test]
fn an_smi_in_no_known_encoding_is_unreadable() {
    // 0xFF is no lead byte of UTF-8 or of CP949.
    let bytes = b"<SAMI><BODY><SYNC Start=1000><P Class=KRCC>\xFF\xFF\xFE</BODY></SAMI>".to_vec();
    assert!(String::from_utf8(bytes.clone()).is_err());
    let error = read(&bytes, "smi").unwrap_err();
    assert_eq!(error.reason, "SMI의 인코딩을 알 수 없어요");
    // UTF-16 without a BOM is not guessed either.
    let bare: Vec<u8> = "<SAMI>".encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert!(read(&bare, "smi").is_err());
}

#[test]
fn pictures_and_unknown_formats_are_unreadable() {
    let sup = read(b"PG\x00\x01\x02", "sup").unwrap_err();
    assert_eq!(sup.reason, "이미지 자막이라 내용을 비교할 수 없어요");
    assert_eq!(read(b"x", "idx").unwrap_err(), sup);
    assert_eq!(read(b"\x00\x00\x01\xBA", ".SUB").unwrap_err(), sup);
    assert_eq!(
        read(b"{1}{2}hi", "sub").unwrap_err().reason,
        "읽을 수 없는 형식이에요"
    );
    assert_eq!(
        read(b"hi", "txt").unwrap_err().reason,
        "읽을 수 없는 형식이에요"
    );
}

#[test]
fn a_file_that_is_not_its_format_is_unreadable() {
    for (bytes, extension) in [
        (&b"hello"[..], "ass"),
        (b"hello", "srt"),
        (b"hello", "smi"),
        (b"", "srt"),
        (b"<html><body>no</body></html>", "smi"),
    ] {
        assert_eq!(
            read(bytes, extension).unwrap_err().reason,
            "자막 형식으로 읽지 못했어요",
            "{extension}"
        );
    }
}

#[test]
fn utf16_with_a_bom_in_both_byte_orders() {
    let text = "1\r\n00:00:01,000 --> 00:00:02,000\r\n한글 자막\r\n";
    let mut le = vec![0xFF, 0xFE];
    le.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    let mut be = vec![0xFE, 0xFF];
    be.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
    let mut utf8 = vec![0xEF, 0xBB, 0xBF];
    utf8.extend(text.as_bytes());
    let reads: Vec<Script> = [&le, &be, &utf8]
        .iter()
        .map(|bytes| read(bytes, "srt").unwrap())
        .collect();
    assert_eq!(reads[0].encoding, Encoding::Utf16);
    assert_eq!(reads[1].encoding, Encoding::Utf16);
    assert_eq!(reads[2].encoding, Encoding::Utf8);
    for script in &reads {
        assert_eq!(script.tracks[0].cues[0].text, "한글 자막");
        assert_eq!(script.tracks[0].cues[0].start, 1000);
        assert!(!compare(&reads[0], script).differs());
    }
    // An odd length cannot be UTF-16.
    le.pop();
    assert!(read(&le, "srt").is_err());
}

#[test]
fn ass_override_tags_comments_and_drawings_are_not_dialogue() {
    let text = "\u{feff}[Script Info]\r\nTitle: x\r\n\r\n[Events]\r\n\
        Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\n\
        Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\an8\\i1}Hello,{\\c&HFF&} world\\Nsecond\\hline\r\n\
        Comment: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,a comment\r\n\
        Dialogue: 0,0:00:05.00,0:00:06.00,Sign,,0,0,0,,{\\p1}m 0 0 l 10 10{\\p0}\r\n\
        Dialogue: 0,0:00:07.00,0:00:08.00,Default,,0,0,0,,{\\pos(1,2)}\r\n\
        Dialogue: 0,0:00:09.00,0:00:10.00,Default,,0,0,0,,  spaced   out  \r\n";
    let script = read(text.as_bytes(), "ssa").unwrap();
    let cues = &script.tracks[0].cues;
    assert_eq!(cues.len(), 2);
    assert_eq!(cues[0].text, "Hello, world\nsecond line");
    assert_eq!(cues[0].style.as_deref(), Some("Default"));
    assert_eq!((cues[0].start, cues[0].end), (1000, 2000));
    assert_eq!(cues[1].text, "spaced out");
    // Only the override tags differ: not a dialogue change.
    let plain = text.replace("{\\an8\\i1}", "").replace("{\\c&HFF&}", "");
    let diff = compare(&script, &read(plain.as_bytes(), "ass").unwrap());
    assert!(!diff.differs());
}

#[test]
fn srt_tags_entities_and_blocks_are_removed() {
    let srt = "1\n00:00:01,500 --> 00:00:02,000 X1:0 X2:9\n\
               {\\an8}<i>Tom &amp; Jerry</i>\n<font color=\"#fff\">&lt;ok&gt;</font>\n\n\
               2\n00:00:03,000 --> 00:00:04,000\na < b <br> c\n";
    let script = read(srt.as_bytes(), "srt").unwrap();
    let cues = &script.tracks[0].cues;
    assert_eq!(cues[0].text, "Tom & Jerry\n<ok>");
    assert_eq!((cues[0].start, cues[0].end), (1500, 2000));
    assert_eq!(cues[1].text, "a < b\nc");
}

#[test]
fn webvtt_reads_like_srt() {
    let vtt =
        "WEBVTT\n\nNOTE a note\n\nintro\n00:01.000 --> 00:02.500 line:90%\nHi <v Bob>there</v>\n";
    let script = read(vtt.as_bytes(), "vtt").unwrap();
    assert_eq!(script.format, Format::Vtt);
    let cue = &script.tracks[0].cues[0];
    assert_eq!(cue.text, "Hi there");
    assert_eq!((cue.start, cue.end), (1000, 2500));
}

#[test]
fn one_side_empty_is_all_added_or_all_removed() {
    let full = read_ass(&ass_of(&thirty()));
    let none = read_ass(&ass_of(&[]));
    let added = compare(&none, &full);
    assert_eq!(
        (
            added.dialogue.added,
            added.dialogue.changed,
            added.dialogue.removed
        ),
        (30, 0, 0)
    );
    assert!(added.dialogue.lines.iter().all(|l| l.old.is_none()));
    let removed = compare(&full, &none);
    assert_eq!(
        (
            removed.dialogue.added,
            removed.dialogue.changed,
            removed.dialogue.removed
        ),
        (0, 0, 30)
    );
    assert!(removed.dialogue.lines.iter().all(|l| l.new.is_none()));
    assert_eq!(removed.timing.count, 0);
}

#[test]
fn identical_files_have_no_difference() {
    let script = read_ass(&ass_of(&thirty()));
    let diff = compare(&script, &script);
    assert_eq!(diff.dialogue, Dialogue::default());
    assert_eq!(diff.timing, Timing::default());
    assert_eq!(diff.styles, Some(Styles::default()));
    assert_eq!(diff.fonts, Some(Fonts::default()));
    assert!(diff.not_compared.is_empty());
    assert!(!diff.differs());
}

#[test]
fn repeated_lines_align_in_order() {
    // "yes" at the start and the end of both; one in the middle added.
    let lines = |middle: &[&str]| {
        let mut all = vec!["yes"];
        all.extend(middle);
        all.push("yes");
        all.iter()
            .enumerate()
            .map(|(n, t)| {
                (
                    format!("0:00:{:02}.00", n * 2),
                    format!("0:00:{:02}.50", n * 2),
                    (*t).to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    let old = read_ass(&ass_of(&lines(&["no"])));
    let new = read_ass(&ass_of(&lines(&["no", "yes"])));
    let diff = compare(&old, &new);
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (1, 0, 0)
    );
}

#[test]
fn lines_at_one_time_in_another_order_are_the_same() {
    let build = |first: &str, second: &str| {
        ass_of(&[
            ("0:00:01.00".into(), "0:00:02.00".into(), first.into()),
            ("0:00:01.00".into(), "0:00:02.00".into(), second.into()),
        ])
    };
    let diff = compare(
        &read_ass(&build("A: hi", "B: hello")),
        &read_ass(&build("B: hello", "A: hi")),
    );
    assert!(!diff.differs(), "{:?}", diff.dialogue);
}

#[test]
fn leftover_lines_in_a_gap_pair_in_order_when_times_do_not_overlap() {
    let old = ass_of(&[("0:00:01.00".into(), "0:00:02.00".into(), "old words".into())]);
    let new = ass_of(&[("0:00:05.00".into(), "0:00:06.00".into(), "new words".into())]);
    let diff = compare(&read_ass(&old), &read_ass(&new));
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (0, 1, 0)
    );
}

#[test]
fn a_diff_serializes_compactly_and_back() {
    let old = read_ass(&ass(
        &[("Default", "Arial")],
        &[
            ("0:00:01.00", "0:00:02.00", "Default", "안녕"),
            ("0:00:03.00", "0:00:04.00", "Default", "응"),
        ],
    ));
    let new = read_ass(&ass(
        &[("Default", "Gulim")],
        &[
            ("0:00:01.00", "0:00:02.00", "Default", "안녕하세요"),
            ("0:00:03.50", "0:00:04.50", "Default", "응"),
            ("0:00:05.00", "0:00:06.00", "Default", "네"),
        ],
    ));
    let diff = compare(&old, &new);
    let json = serde_json::to_value(&diff).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "old": {"format": "ass", "encoding": "UTF-8", "cues": 2},
            "new": {"format": "ass", "encoding": "UTF-8", "cues": 3},
            "dialogue": {"added": 1, "changed": 1, "removed": 0, "lines": [
                {"kind": "changed",
                 "old": {"text": "안녕", "start": 1000, "end": 2000},
                 "new": {"text": "안녕하세요", "start": 1000, "end": 2000}},
                {"kind": "added",
                 "new": {"text": "네", "start": 5000, "end": 6000}}]},
            "timing": {"count": 1, "lines": [
                {"text": "응", "old": {"start": 3000, "end": 4000},
                 "new": {"start": 3500, "end": 4500}}]},
            "styles": {"changed": [{"name": "Default", "fields": [
                {"field": "Fontname", "old": "Arial", "new": "Gulim"}]}]},
            "fonts": {"added": ["Gulim"], "removed": ["Arial"]}
        })
    );
    let back: Diff = serde_json::from_value(json).unwrap();
    assert_eq!(back, diff);
    // What cannot be compared survives the round trip.
    let srt = read(b"1\n00:00:01,000 --> 00:00:02,000\nhi\n", "srt").unwrap();
    let partial = compare(&srt, &old);
    let back: Diff = serde_json::from_str(&serde_json::to_string(&partial).unwrap()).unwrap();
    assert_eq!(back, partial);
    assert_eq!(back.styles, None);
    assert_eq!(back.not_compared.len(), 2);
}

#[test]
fn a_few_thousand_lines_compare() {
    let many = |changed: bool| {
        (0..4000)
            .map(|n| {
                (
                    format!("{}:{:02}:{:02}.00", n / 3600, n / 60 % 60, n % 60),
                    format!("{}:{:02}:{:02}.50", n / 3600, n / 60 % 60, n % 60),
                    match changed && n % 100 == 0 {
                        true => format!("changed {n}"),
                        false => format!("line {n}"),
                    },
                )
            })
            .collect::<Vec<_>>()
    };
    let diff = compare(
        &read_ass(&ass_of(&many(false))),
        &read_ass(&ass_of(&many(true))),
    );
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (0, 40, 0)
    );
}

#[test]
fn a_p_tag_missing_its_gt_is_not_left_in_the_text() {
    let smi = "<SAMI><HEAD><STYLE TYPE=\"text/css\"><!--\n\
               .KRCC { Name: Korean; lang: ko-KR; }\n--></STYLE></HEAD><BODY>\n\
               <SYNC Start=1000><P Class=KRCC>안녕\n\
               <SYNC Start=2000><P Class=KRCC 여기 뚫리면 끝이라고!\n\
               <SYNC Start=3000><P Class=KRCC><font color=red 빨강 <i>기울임</i>\n\
               <SYNC Start=4000><P Class=KRCC>&nbsp;\n\
               <SYNC Start=5000><P Class=KRCC>a<x 문자\n\
               <SYNC Start=6000><P Class=KRCC>&nbsp;\n</BODY></SAMI>";
    let script = read(smi.as_bytes(), "smi").unwrap();
    assert_eq!(script.tracks.len(), 1, "{:?}", script.tracks);
    let track = &script.tracks[0];
    assert_eq!(track.class.as_deref(), Some("KRCC"));
    assert_eq!(track.lang.as_deref(), Some("ko-kr"));
    let texts: Vec<&str> = track.cues.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(
        texts,
        ["안녕", "여기 뚫리면 끝이라고!", "빨강 기울임", "a<x 문자"]
    );
    assert!(track
        .cues
        .iter()
        .all(|c| !c.text.contains("Class=") && !c.text.contains("<P")));
}

#[test]
fn a_few_class_less_paragraphs_belong_to_the_files_one_class() {
    let smi = "<SAMI><BODY>\n\
               <SYNC Start=1000><P Class=KRCC>하나\n\
               <SYNC Start=2000><P>둘\n\
               <SYNC Start=3000><P Class=KRCC>&nbsp;\n</BODY></SAMI>";
    let script = read(smi.as_bytes(), "smi").unwrap();
    assert_eq!(script.tracks.len(), 1);
    assert_eq!(script.tracks[0].class.as_deref(), Some("KRCC"));
    let texts: Vec<&str> = script.tracks[0]
        .cues
        .iter()
        .map(|c| c.text.as_str())
        .collect();
    assert_eq!(texts, ["하나", "둘"]);
    // The class-less cue ends where its class's next sync is.
    assert_eq!(script.tracks[0].cues[1].end, 3000);
}

#[test]
fn an_ass_against_two_classes_compares_with_the_bigger_one() {
    let smi = "<SAMI><BODY>\n\
               <SYNC Start=1000><P Class=ENCC>Hi\n\
               <SYNC Start=2000><P Class=ENCC>&nbsp;\n\
               <SYNC Start=3000><P Class=ENCC>Bye<P Class=KRCC>잘 가\n\
               <SYNC Start=4000><P Class=ENCC>&nbsp;<P Class=KRCC>&nbsp;\n\
               <SYNC Start=5000><P Class=KRCC>그래\n\
               <SYNC Start=6000><P Class=KRCC>&nbsp;\n</BODY></SAMI>";
    let smi = read(smi.as_bytes(), "smi").unwrap();
    assert_eq!(smi.tracks[0].class.as_deref(), Some("ENCC"));
    let ass = read_ass(&ass(
        &[("Default", "Arial")],
        &[
            ("0:00:03.00", "0:00:04.00", "Default", "잘 가"),
            ("0:00:05.00", "0:00:06.00", "Default", "그래"),
        ],
    ));
    // Korean has 2 cues, English 2: a tie goes to Korean, though second.
    let diff = compare(&ass, &smi);
    assert!(!diff.differs(), "{:?}", diff.dialogue);
    assert_eq!(
        diff.not_compared
            .iter()
            .filter(|n| n.item == Item::Dialogue)
            .count(),
        1
    );
    assert!(diff.not_compared.iter().any(|n| n.reason.contains("ENCC")));

    // The bigger class wins, in either order of the arguments.
    let more = "<SAMI><BODY>\n\
                <SYNC Start=1000><P Class=KRCC>가<P Class=XXCC>x\n\
                <SYNC Start=2000><P Class=KRCC>&nbsp;<P Class=XXCC>&nbsp;\n\
                <SYNC Start=3000><P Class=XXCC>y\n\
                <SYNC Start=4000><P Class=XXCC>&nbsp;\n\
                <SYNC Start=5000><P Class=XXCC>z\n\
                <SYNC Start=6000><P Class=XXCC>&nbsp;\n</BODY></SAMI>";
    let more = read(more.as_bytes(), "smi").unwrap();
    let one = read(b"1\n00:00:03,000 --> 00:00:04,000\ny\n", "srt").unwrap();
    let diff = compare(&more, &one);
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (0, 0, 2)
    );
    assert_eq!(diff.dialogue.lines[0].class.as_deref(), Some("XXCC"));
    // The same name wins over size.
    let named = read(
        b"<SAMI><BODY><SYNC Start=1000><P Class=krcc>\xea\xb0\x80</BODY></SAMI>",
        "smi",
    )
    .unwrap();
    let diff = compare(&named, &more);
    assert_eq!(diff.dialogue.lines.len(), 0, "{:?}", diff.dialogue);
}

// ---------------------------------------------------------------- hostile files

/// Reads `text` as `ext` and says how long it took. Each shape below cost the
/// square of its length before; a generous bound keeps a slow machine from
/// failing a linear read.
fn timed(text: &str, ext: &str) -> (Result<Script, Unreadable>, std::time::Duration) {
    let at = std::time::Instant::now();
    let read = read(text.as_bytes(), ext);
    (read, at.elapsed())
}

const QUICK: std::time::Duration = std::time::Duration::from_secs(5);

#[test]
fn an_ass_line_of_blocks_that_never_close_is_read_in_linear_time() {
    let text = ass(
        &[("Default", "Arial")],
        &[("0:00:01.00", "0:00:02.00", "Default", &"{".repeat(1 << 20))],
    );
    let (read, took) = timed(&text, "ass");
    assert!(took < QUICK, "{took:?}");
    assert_eq!(read.unwrap().tracks[0].cues.len(), 1);
}

#[test]
fn a_text_full_of_ampersands_is_read_in_linear_time() {
    let text = format!(
        "1\n00:00:01,000 --> 00:00:02,000\n{}\n",
        "&".repeat(1 << 20)
    );
    let (read, took) = timed(&text, "srt");
    assert!(took < QUICK, "{took:?}");
    assert_eq!(read.unwrap().tracks[0].cues.len(), 1);
}

#[test]
fn many_smi_syncs_at_one_time_are_read_in_linear_time() {
    // More cues than the limit, so the file is unreadable; but each sync's end
    // is found before the cues are counted, and that is what must be quick.
    let syncs = "<SYNC Start=1000><P Class=KRCC>같은 때\n".repeat(100_000);
    let text = format!("<SAMI><BODY>{syncs}<SYNC Start=2000><P Class=KRCC>&nbsp;</BODY></SAMI>");
    let (over, took) = timed(&text, "smi");
    assert!(took < QUICK, "{took:?}");
    assert!(reason(over).contains("대사 줄이 너무 많아"));

    // Each ends at the next later start, not at its equals.
    let syncs = "<SYNC Start=1000><P Class=KRCC>같은 때\n".repeat(3);
    let text = format!("<SAMI><BODY>{syncs}<SYNC Start=2000><P Class=KRCC>다음</BODY></SAMI>");
    let script = read(text.as_bytes(), "smi").unwrap();
    let ends: Vec<(u64, u64)> = script.tracks[0]
        .cues
        .iter()
        .map(|c| (c.start, c.end))
        .collect();
    assert_eq!(
        ends,
        [(1000, 2000), (1000, 2000), (1000, 2000), (2000, 2000)]
    );
}

#[test]
fn smi_tags_that_never_close_are_read_in_linear_time() {
    let text = format!(
        "<SAMI><BODY><SYNC Start=1000><P Class=KRCC>{}{}</BODY></SAMI>",
        "<i a=\"".repeat(1 << 17),
        "<font ".repeat(1 << 17)
    );
    let (read, took) = timed(&text, "smi");
    assert!(took < QUICK, "{took:?}");
    assert!(read.is_ok());
}

fn reason(read: Result<Script, Unreadable>) -> String {
    read.expect_err("unreadable").reason
}

#[test]
fn a_script_of_more_cues_than_the_limit_is_unreadable() {
    let lines = |n: usize| -> Vec<(String, String, String)> {
        (0..n)
            .map(|i| {
                (
                    format!("0:00:{:02}.00", i % 60),
                    format!("0:00:{:02}.50", i % 60),
                    format!("line {i}"),
                )
            })
            .collect()
    };
    assert_eq!(
        read_ass(&ass_of(&lines(MAX_CUES))).tracks[0].cues.len(),
        MAX_CUES
    );
    let over = read(ass_of(&lines(MAX_CUES + 1)).as_bytes(), "ass");
    assert!(reason(over).contains("대사 줄이 너무 많아"));

    // Two SMI classes are counted together.
    let half = MAX_CUES / 2 + 1;
    let mut text = String::from("<SAMI><BODY>");
    for i in 0..half {
        text += &format!(
            "<SYNC Start={}><P Class=KRCC>한 {i}<P Class=ENCC>en {i}\n",
            i * 1000
        );
    }
    text += "</BODY></SAMI>";
    assert!(reason(read(text.as_bytes(), "smi")).contains("대사 줄이 너무 많아"));
}

#[test]
fn too_many_or_too_long_styles_fonts_classes_and_fields_are_unreadable() {
    let styles: Vec<(String, String)> = (0..=MAX_STYLES)
        .map(|i| (format!("S{i}"), "Arial".to_owned()))
        .collect();
    let styles: Vec<(&str, &str)> = styles
        .iter()
        .map(|(n, f)| (n.as_str(), f.as_str()))
        .collect();
    assert!(reason(read(ass(&styles, &[]).as_bytes(), "ass")).contains("스타일이 너무 많아"));
    assert!(read(ass(&styles[..MAX_STYLES], &[]).as_bytes(), "ass").is_ok());

    let long = "x".repeat(MAX_STYLE_BYTES);
    assert!(
        reason(read(ass(&[("Default", &long)], &[]).as_bytes(), "ass"))
            .contains("스타일 줄이 너무 길어")
    );

    // Fonts are counted once each, however often a line names them.
    let named = |n: usize| {
        (0..n)
            .map(|i| format!("{{\\fnF{}}}", i % 1_001))
            .collect::<String>()
            + "말"
    };
    let many = named(1_000);
    assert!(read(
        ass(&[], &[("0:00:01.00", "0:00:02.00", "Default", &many)]).as_bytes(),
        "ass"
    )
    .is_ok());
    let repeated = format!("{}{}", "{\\fnF0}".repeat(100_000), many);
    assert!(read(
        ass(&[], &[("0:00:01.00", "0:00:02.00", "Default", &repeated)]).as_bytes(),
        "ass"
    )
    .is_ok());
    let over = named(1_001);
    let text = ass(
        &[("Default", "Arial")],
        &[("0:00:01.00", "0:00:02.00", "Default", &over)],
    );
    assert!(reason(read(text.as_bytes(), "ass")).contains("폰트가 너무 많아"));

    let name = format!("{{\\fn{}}}말", "가".repeat(MAX_NAME_BYTES));
    let text = ass(&[], &[("0:00:01.00", "0:00:02.00", "Default", &name)]);
    assert!(reason(read(text.as_bytes(), "ass")).contains("이름이 너무 길어"));

    let mut smi = String::from("<SAMI><BODY><SYNC Start=1000>");
    for i in 0..=MAX_CLASSES {
        smi += &format!("<P Class=C{i}>{i}");
    }
    smi += "</BODY></SAMI>";
    assert!(reason(read(smi.as_bytes(), "smi")).contains("언어 구분(Class)이 너무 많아"));

    let fields = (0..=MAX_FORMAT_FIELDS)
        .map(|i| format!("F{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let text = format!("[Script Info]\n\n[Events]\nFormat: {fields}\n");
    assert!(reason(read(text.as_bytes(), "ass")).contains("형식 줄의 항목이 너무 많아"));
}

#[test]
fn a_format_line_too_long_is_unreadable_before_any_style_copies_its_names() {
    // Every style copies the format's names: a few long ones and many short
    // styles would hold the names many times over.
    // Kept small, so a reader without the limit holds megabytes, not more.
    let styled = |format: &str| {
        let styles: String = (0..MAX_STYLES)
            .map(|i| format!("Style: S{i},Arial{}\n", ",0".repeat(8)))
            .collect();
        format!("[Script Info]\n\n[V4+ Styles]\nFormat: {format}\n{styles}")
    };
    let long = format!(
        "Name, Fontname, {}",
        vec!["x".repeat(MAX_STYLE_BYTES); 8].join(", ")
    );
    assert!(reason(read(styled(&long).as_bytes(), "ass")).contains("형식 줄이 너무 길어"));

    let fits = format!("Name, Fontname, {}", "x".repeat(MAX_STYLE_BYTES - 16));
    assert_eq!(fits.len(), MAX_STYLE_BYTES);
    let script = read(styled(&fits).as_bytes(), "ass").unwrap();
    assert_eq!(script.styles.unwrap().len(), MAX_STYLES);
    assert!(
        reason(read(styled(&format!("{fits}x")).as_bytes(), "ass")).contains("형식 줄이 너무 길어")
    );
}

#[test]
fn an_smi_of_more_paragraphs_than_the_limit_is_unreadable() {
    let smi = |n: usize| {
        format!(
            "<SAMI><BODY><SYNC Start=1000><P Class=KRCC>말<SYNC Start=2000>{}</BODY></SAMI>",
            "<P Class=KRCC>".repeat(n - 1)
        )
    };
    let script = read(smi(MAX_SMI_PARAGRAPHS).as_bytes(), "smi").unwrap();
    assert_eq!(script.tracks[0].cues.len(), 1);
    assert!(
        reason(read(smi(MAX_SMI_PARAGRAPHS + 1).as_bytes(), "smi")).contains("대사 줄이 너무 많아")
    );
}

#[test]
fn a_font_in_two_spellings_keeps_the_dialogues() {
    // Dialogue fonts are gathered first, then the styles', and the first
    // spelling of a name stays.
    let text = ass(
        &[("Default", "Noto Sans")],
        &[("0:00:01.00", "0:00:02.00", "Default", "{\\fnNOTO SANS}말")],
    );
    let fonts = read_ass(&text).fonts.unwrap();
    let names: Vec<&str> = fonts.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["NOTO SANS"]);
}

#[test]
fn an_smi_start_past_any_time_is_no_sync_and_never_overflows() {
    let text = "<SAMI><BODY><SYNC Start=18446744073709551615><P Class=KRCC>끝\
                <SYNC Start=9999999999><P Class=KRCC>마지막</BODY></SAMI>";
    let script = read(text.as_bytes(), "smi").unwrap();
    let cues = &script.tracks[0].cues;
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].start, 9_999_999_999);
    assert!(!compare(&script, &script).differs());

    let at_end = |start| Cue {
        start,
        end: start,
        text: "끝".to_owned(),
        shown: "끝".to_owned(),
        style: None,
    };
    // An empty range counts as a millisecond, which the last moment has no
    // room for: it must not overflow.
    assert!(!overlaps(&at_end(u64::MAX), &at_end(u64::MAX)));
    assert!(overlaps(&at_end(u64::MAX - 1), &at_end(u64::MAX - 1)));
}
