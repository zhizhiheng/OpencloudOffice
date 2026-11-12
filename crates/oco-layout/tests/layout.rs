//! Layout tests. Documents are built in memory; expected numbers are either
//! simple arithmetic on the input or values measured from Word's own output
//! (see docs/layout.md), in which case the test says so.
//!
//! These tests need real fonts: the Liberation and Carlito families from the
//! system, and for Chinese text the fonts fetched by tools/fonts/fetch.sh.
//! A test whose fonts are missing prints a notice and passes vacuously.

use std::io::{Cursor, Write};
use std::path::Path;

use oco_docx::Docx;
use oco_layout::{content, layout, Content, FontDb, Model, Options, Page, Position};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const RELS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const LOREM: &str = "The quick brown fox jumps over the lazy dog while the farmer watches from the gate. \
    Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et \
    dolore magna aliqua. A typesetting engine must decide where each line ends and where each page ends.";
const HANZI: &str =
    "文档排版的目标是让读者把注意力放在内容上合适的字号行距和页边距能够减轻阅读负担而前后一致的标题层级\
    和编号方式则帮助读者快速建立全文的结构排版规则并不复杂难的是在每一页每一段都严格执行";

struct Source<'a> {
    body: &'a str,
    styles: &'a str,
    numbering: &'a str,
    /// Page setup; A4 with one-inch margins and no grid unless given.
    section: &'a str,
    settings: &'a str,
}

impl Default for Source<'_> {
    fn default() -> Self {
        Source {
            body: "",
            styles: "",
            numbering: "",
            section: r#"<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>"#,
            settings: r#"<w:compat><w:compatSetting w:name="compatibilityMode" w:val="15"/></w:compat>"#,
        }
    }
}

fn docx(source: &Source) -> Docx {
    let part = |kind: &str, target: &str, id: u8| {
        format!(r#"<Relationship Id="rId{id}" Type="{RELS}/{kind}" Target="{target}"/>"#)
    };
    let parts = [
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}{}{}</Relationships>"#,
                part("styles", "styles.xml", 1),
                part("numbering", "numbering.xml", 2),
                part("settings", "settings.xml", 3)
            ),
        ),
        (
            "word/document.xml",
            format!(
                "<w:document {W}><w:body>{}<w:sectPr>{}</w:sectPr></w:body></w:document>",
                source.body, source.section
            ),
        ),
        ("word/styles.xml", format!("<w:styles {W}>{}</w:styles>", source.styles)),
        ("word/numbering.xml", format!("<w:numbering {W}>{}</w:numbering>", source.numbering)),
        ("word/settings.xml", format!("<w:settings {W}>{}</w:settings>", source.settings)),
    ];
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, content) in parts {
        writer.start_file(name, options).unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    Docx::from_bytes(&writer.finish().unwrap().into_inner()).unwrap()
}

/// Loads the fonts, or `None` (with a notice) when one of `needed` is absent.
fn fonts(needed: &[(&str, char)]) -> Option<FontDb> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut db = FontDb::default();
    db.load_dir(&root.join("fonts"));
    for dir in ["liberation", "crosextra", "dejavu"] {
        db.load_dir(&Path::new("/usr/share/fonts/truetype").join(dir));
    }
    for (family, sample) in needed {
        // A family that is not installed resolves to some other font.
        let found = db
            .choose(Some(family), false, false, *sample, Model::LibreOffice)
            .map(|c| db.get(c.font).family().to_owned());
        if found.as_deref() != Some(&family.to_lowercase()) {
            eprintln!("skipped: font {family:?} is not installed (run tools/fonts/fetch.sh)");
            return None;
        }
    }
    Some(db)
}

fn pages(source: &Source, fonts: &FontDb, model: Model) -> Vec<Page> {
    layout(&docx(source), fonts, &Options { model }).pages
}

/// Baseline-to-baseline distances between consecutive lines of the first page.
fn pitches(page: &Page) -> Vec<f32> {
    page.lines.windows(2).map(|pair| pair[1].baseline - pair[0].baseline).collect()
}

fn paragraph(run_props: &str, para_props: &str, text: &str) -> String {
    format!(
        r#"<w:p><w:pPr>{para_props}</w:pPr><w:r><w:rPr>{run_props}</w:rPr><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#
    )
}

fn font(family: &str, half_points: u32) -> String {
    format!(r#"<w:rFonts w:ascii="{family}" w:hAnsi="{family}" w:eastAsia="{family}"/><w:sz w:val="{half_points}"/>"#)
}

fn close(actual: f32, expected: f32, tolerance: f32) -> bool {
    (actual - expected).abs() <= tolerance
}

#[test]
fn single_line_pitch_matches_word() {
    let Some(fonts) =
        fonts(&[("Liberation Serif", 'a'), ("Liberation Sans", 'a'), ("Carlito", 'a'), ("DejaVu Sans", 'a')])
    else {
        return;
    };
    // Measured from Word 365's PDF output (docx4j golden `line-auto`).
    for (family, half_points, line, expected) in [
        ("Liberation Serif", 24, 240, 13.80),
        ("Liberation Serif", 24, 276, 15.87),
        ("Liberation Serif", 24, 360, 20.70),
        ("Liberation Serif", 24, 480, 27.60),
        ("Carlito", 22, 240, 13.44),
        ("Liberation Sans", 20, 240, 11.52),
        ("DejaVu Sans", 20, 240, 11.64),
    ] {
        let body =
            paragraph(&font(family, half_points), &format!(r#"<w:spacing w:line="{line}" w:lineRule="auto"/>"#), LOREM);
        let pages = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word);
        let pitches = pitches(&pages[0]);
        assert!(pitches.len() >= 2, "{family}: expected the text to wrap");
        for pitch in pitches {
            assert!(
                close(pitch, expected, 0.03),
                "{family} {half_points}/2 pt at line={line}: {pitch} pt, Word has {expected} pt"
            );
        }
    }
}

#[test]
fn extra_leading_of_multiple_spacing_goes_below_the_text() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // Measured from Word 365's PDF output (docx4j golden `line-auto`): the
    // first baseline on the page is at 83.2 pt whatever the multiple, and
    // after a single-spaced paragraph the first line of one at 1.15 lines
    // comes 13.8 pt later, one single pitch, not 13.8 pt and a share of the
    // extra. LibreOffice raises the ascent in proportion instead.
    let spaced = |line: u32| {
        paragraph(&font("Liberation Serif", 24), &format!(r#"<w:spacing w:line="{line}" w:lineRule="auto"/>"#), LOREM)
    };
    for line in [240, 276, 480] {
        let pages = pages(&Source { body: &spaced(line), ..Source::default() }, &fonts, Model::Word);
        assert!(close(pages[0].lines[0].baseline, 83.2, 0.03), "line={line}: {}", pages[0].lines[0].baseline);
    }
    let body = spaced(240) + &spaced(276);
    let word = pitches(&pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word)[0]);
    let single = word.iter().filter(|pitch| close(**pitch, 13.80, 0.03)).count();
    let wide = word.iter().filter(|pitch| close(**pitch, 15.87, 0.03)).count();
    assert_eq!(single + wide, word.len(), "{word:?}");
    // The step from the first paragraph into the second is a single pitch.
    assert_eq!(wide, word.len() / 2, "{word:?}");
    let libreoffice = pitches(&pages(&Source { body: &body, ..Source::default() }, &fonts, Model::LibreOffice)[0]);
    assert!(libreoffice.iter().any(|pitch| !close(*pitch, 13.80, 0.03) && !close(*pitch, 15.87, 0.03)));
}

#[test]
fn exact_and_at_least_line_spacing() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    for (rule, line, expected) in
        [("exact", 480, 24.0), ("exact", 180, 9.0), ("atLeast", 120, 13.80), ("atLeast", 400, 20.0)]
    {
        let body = paragraph(
            &font("Liberation Serif", 24),
            &format!(r#"<w:spacing w:line="{line}" w:lineRule="{rule}"/>"#),
            LOREM,
        );
        let pages = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word);
        for pitch in pitches(&pages[0]) {
            assert!(close(pitch, expected, 0.03), "{rule} {line}: {pitch} pt");
        }
    }
}

#[test]
fn chinese_fonts_set_at_1_3_times_their_size_whatever_font_is_drawn() {
    let Some(fonts) = fonts(&[("Source Han Serif CN", '中')]) else { return };
    // Measured from Word 365's PDF (docx4j golden `fonts-cjk-linebox`):
    // 宋体 at 10 pt sets at 12.97 pt, which is 1.3 em rounded to 1/600 inch.
    let body = paragraph(&font("宋体", 20), "", HANZI);
    let source = Source { body: &body, ..Source::default() };
    for pitch in pitches(&pages(&source, &fonts, Model::Word)[0]) {
        assert!(close(pitch, 12.96, 0.03), "{pitch}");
    }
    // The font drawn in its place would on its own metrics give 14.37 pt.
    for pitch in pitches(&pages(&source, &fonts, Model::LibreOffice)[0]) {
        assert!(close(pitch, 14.37, 0.05), "{pitch}");
    }
}

#[test]
fn a_document_grid_does_not_double_the_lines_of_substituted_fonts() {
    let Some(fonts) = fonts(&[("Source Han Serif CN", '中')]) else { return };
    // 小四 (12 pt) 宋体 on the default grid of 15.6 pt: one grid row in Word,
    // because 12 x 1.3 = 15.6. By the substitute's metrics the text is 17.2 pt
    // tall and needs two rows, which is how such documents double in length.
    let body = paragraph(&font("宋体", 24), "", HANZI);
    let grid = r#"<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1800" w:bottom="1440" w:left="1800"/><w:docGrid w:type="lines" w:linePitch="312"/>"#;
    let source = Source { body: &body, section: grid, ..Source::default() };
    for pitch in pitches(&pages(&source, &fonts, Model::Word)[0]) {
        assert!(close(pitch, 15.6, 0.01), "{pitch}");
    }
    for pitch in pitches(&pages(&source, &fonts, Model::LibreOffice)[0]) {
        assert!(close(pitch, 31.2, 0.01), "{pitch}");
    }
    // Exact spacing ignores the grid, and so does a paragraph that opts out.
    for (props, expected) in
        [(r#"<w:spacing w:line="400" w:lineRule="exact"/>"#, 20.0), (r#"<w:snapToGrid w:val="0"/>"#, 15.6)]
    {
        let body = paragraph(&font("宋体", 24), props, HANZI);
        let source = Source { body: &body, section: grid, ..Source::default() };
        for pitch in pitches(&pages(&source, &fonts, Model::Word)[0]) {
            assert!(close(pitch, expected, 0.03), "{props}: {pitch}");
        }
    }
}

#[test]
fn space_between_paragraphs_is_the_larger_of_after_and_before() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // Measured from Word 365's PDF (docx4j golden `spacing-adjacent`): the
    // gap is 12 pt for (after, before) of (12, 0), (0, 12), (12, 12) and (6, 12).
    for (after, before, expected) in
        [(0, 0, 0.0), (240, 0, 12.0), (0, 240, 12.0), (240, 240, 12.0), (120, 240, 12.0), (240, 120, 12.0)]
    {
        let one = |spacing: String, text: &str| paragraph(&font("Liberation Serif", 24), &spacing, text);
        let body = one(format!(r#"<w:spacing w:after="{after}"/>"#), "First.")
            + &one(format!(r#"<w:spacing w:before="{before}"/>"#), "Second.");
        let pages = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word);
        let gap = pitches(&pages[0])[0] - 13.80;
        assert!(close(gap, expected, 0.03), "after {after}, before {before}: gap {gap}");
    }
}

#[test]
fn indents_in_twips_and_in_characters() {
    let Some(fonts) = fonts(&[("Source Han Serif CN", '中')]) else { return };
    let x_of = |props: &str, half_points: u32| -> (f32, f32) {
        let body = paragraph(&font("宋体", half_points), props, HANZI);
        let page = &pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word)[0];
        (page.lines[0].x0 - 72.0, page.lines[1].x0 - 72.0)
    };
    let near = |(first, rest): (f32, f32), expected: (f32, f32)| {
        close(first, expected.0, 0.01) && close(rest, expected.1, 0.01)
    };
    assert!(near(x_of("", 21), (0.0, 0.0)));
    assert!(near(x_of(r#"<w:ind w:firstLine="420"/>"#, 21), (21.0, 0.0)));
    assert!(near(x_of(r#"<w:ind w:left="840" w:hanging="420"/>"#, 21), (21.0, 42.0)));
    // Two characters is twice the font size, whatever the twips beside it say.
    assert!(near(x_of(r#"<w:ind w:firstLineChars="200" w:firstLine="420"/>"#, 21), (21.0, 0.0)));
    assert!(near(x_of(r#"<w:ind w:firstLineChars="200" w:firstLine="420"/>"#, 32), (32.0, 0.0)));
    assert!(near(
        x_of(r#"<w:ind w:leftChars="300" w:left="630" w:hangingChars="100" w:hanging="210"/>"#, 21),
        (21.0, 31.5)
    ));
}

#[test]
fn punctuation_never_starts_a_line() {
    let Some(fonts) = fonts(&[("Source Han Serif CN", '中')]) else { return };
    // The measure holds exactly ten 20 pt characters. The comma is the
    // eleventh: it hangs in the margin rather than start the next line. A
    // closing bracket may not hang, so the character before it comes down too.
    let narrow =
        r#"<w:pgSz w:w="6880" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>"#;
    let lines = |text: &str| -> Vec<String> {
        let body = paragraph(&font("宋体", 40), r#"<w:jc w:val="left"/>"#, text);
        pages(&Source { body: &body, section: narrow, ..Source::default() }, &fonts, Model::Word)[0]
            .lines
            .iter()
            .map(|l| l.text.clone())
            .collect()
    };
    assert_eq!(lines("一二三四五六七八九十，甲乙丙"), ["一二三四五六七八九十，", "甲乙丙"]);
    assert_eq!(lines("一二三四五六七八九（十甲乙丙"), ["一二三四五六七八九", "（十甲乙丙"]);
    assert_eq!(lines("一二三四五六七八九十甲》乙丙"), ["一二三四五六七八九十", "甲》乙丙"]);
    // A closing bracket needs room only for its inked half, so it fits where
    // a full character would not.
    assert_eq!(lines("一二三四五六七八九）十甲乙丙"), ["一二三四五六七八九）", "十甲乙丙"]);
    // A Latin word that does not fit moves down whole.
    assert_eq!(lines("一二三四五六七 Typesetting 八"), ["一二三四五六七 ", "Typesetting 八"]);
}

#[test]
fn list_labels_sit_at_left_minus_hanging() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in Word (docx4j golden `numbering-level-vs-style-indent`):
    // with left 1440 and hanging 360, the label starts 54 pt and the text
    // 72 pt from the margin.
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/>
        <w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>
        <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#;
    let item = paragraph(
        &font("Liberation Serif", 24),
        r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>"#,
        LOREM,
    );
    let page =
        &pages(&Source { body: &(item.clone() + &item), numbering, ..Source::default() }, &fonts, Model::Word)[0];
    assert!(page.lines[0].text.starts_with("1.\tThe quick"), "{}", page.lines[0].text);
    assert!(close(page.lines[0].x0 - 72.0, 54.0, 0.01));
    assert!(close(page.lines[1].x0 - 72.0, 72.0, 0.01));
    let second = page.lines.iter().find(|l| l.text.starts_with("2.")).expect("the second item is numbered 2");
    assert!(close(second.x0 - 72.0, 54.0, 0.01));
}

#[test]
fn right_aligned_labels_end_where_left_aligned_ones_begin() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="9"/><w:numFmt w:val="decimal"/>
        <w:lvlText w:val="%1."/><w:lvlJc w:val="right"/><w:pPr><w:ind w:left="840" w:hanging="420"/></w:pPr></w:lvl></w:abstractNum>
        <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#;
    let item = paragraph(
        &font("Liberation Serif", 24),
        r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>"#,
        LOREM,
    );
    let page =
        &pages(&Source { body: &(item.clone() + &item), numbering, ..Source::default() }, &fonts, Model::Word)[0];
    let nine = &page.lines[0];
    let ten = page.lines.iter().find(|l| l.text.starts_with("10.")).expect("the second item is numbered 10");
    // "9." and "10." end at the same place, 21 pt in, so the wider one starts one digit further left ...
    assert!(ten.x0 < nine.x0 - 4.0, "{} {}", nine.x0, ten.x0);
    // ... and the text of both starts at the left indent, as the following lines do.
    let text_start = |line: &oco_layout::PlacedLine| -> f32 {
        let glyphs: Vec<f32> = page
            .items
            .iter()
            .filter_map(|item| match item {
                oco_layout::Item::Glyph { x, y, .. } if (*y - line.baseline).abs() < 0.01 => Some(*x),
                _ => None,
            })
            .collect();
        // The first glyph after the gap left by the tab.
        glyphs.windows(2).find(|pair| pair[1] - pair[0] > 8.0).map(|pair| pair[1]).unwrap()
    };
    assert!(close(text_start(nine) - 72.0, 42.0, 0.01), "{}", text_start(nine) - 72.0);
    assert!(close(text_start(ten) - 72.0, 42.0, 0.01), "{}", text_start(ten) - 72.0);
    assert!(close(page.lines[1].x0 - 72.0, 42.0, 0.01));
}

#[test]
fn headings_stay_with_the_paragraph_they_introduce() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    let body_text = |n: usize| paragraph(&font("Liberation Serif", 24), "", &format!("Line {n}."));
    // 50 lines of 13.8 pt fill 690 of the page's 698 pt; the heading would
    // fit alone on line 51, but not together with the paragraph after it.
    let mut body: String = (0..50).map(body_text).collect();
    body += &paragraph(&font("Liberation Serif", 24), "<w:keepNext/>", "Heading");
    body += &body_text(99);
    let pages = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word);
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].lines.len(), 50);
    assert_eq!(pages[1].lines[0].text, "Heading");
}

#[test]
fn pages_say_where_in_the_content_they_begin() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    let serif = font("Liberation Serif", 24);
    // Hidden and deleted text takes no part in layout, so it is not counted;
    // a tab and a break each count as one, like a character.
    let body = format!(
        r#"<w:p><w:r><w:rPr>{serif}</w:rPr><w:t>first</w:t></w:r></w:p>
           <w:p>
             <w:r><w:rPr>{serif}<w:vanish/></w:rPr><w:t>hidden</w:t></w:r>
             <w:del w:id="1" w:author="a"><w:r><w:delText>gone</w:delText></w:r></w:del>
             <w:r><w:rPr>{serif}</w:rPr><w:t>ab</w:t><w:tab/><w:t>c</w:t><w:br w:type="page"/>
               <w:lastRenderedPageBreak/><w:t>second page</w:t></w:r>
           </w:p>"#
    );
    let source = Source { body: &body, ..Source::default() };
    let docx = docx(&source);
    let result = layout(&docx, &fonts, &Options { model: Model::Word });
    let starts: Vec<_> = result.pages.iter().map(|page| page.start).collect();
    assert_eq!(
        starts,
        [Some(Position { paragraph: 0, offset: 0 }), Some(Position { paragraph: 1, offset: 5 })],
        "the second page begins after `ab`, the tab, `c` and the break"
    );

    // The same count, made from the document, finds Word's own record of
    // that page start at the same place.
    let mut pieces = Vec::new();
    content(&docx, docx.document.paragraphs()[1], |piece| pieces.push(piece));
    assert_eq!(pieces.iter().position(|piece| *piece == Content::PageBreakHint), Some(5));
    assert_eq!(
        pieces[..5],
        [
            Content::Char('a'),
            Content::Char('b'),
            Content::Tab,
            Content::Char('c'),
            Content::Break(oco_docx::BreakKind::Page)
        ]
    );
}

#[test]
fn a_page_that_begins_mid_paragraph_begins_at_a_line_start() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // Enough text to run well past one page, in one paragraph without
    // widow control so it may split anywhere.
    let text = LOREM.repeat(40);
    let body = paragraph(&font("Liberation Serif", 24), r#"<w:widowControl w:val="0"/>"#, &text);
    let result = layout(&docx(&Source { body: &body, ..Source::default() }), &fonts, &Options { model: Model::Word });
    assert!(result.pages.len() >= 2);
    let start = result.pages[1].start.expect("the page begins with body text");
    assert_eq!(start.paragraph, 0);
    // The text on the second page is the paragraph's text from that offset on.
    let rest: String = text.chars().skip(start.offset).collect();
    let first_line = &result.pages[1].lines[0].text;
    assert!(start.offset > 0 && rest.starts_with(first_line.trim_end()), "{first_line:?} at {}", start.offset);
}

#[test]
fn a_list_label_is_not_part_of_the_content_a_page_start_counts() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/>
        <w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>
        <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#;
    let body = format!(
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr>
           <w:r><w:rPr>{}</w:rPr><w:t>ab</w:t><w:br w:type="page"/><w:t>cd</w:t></w:r></w:p>"#,
        font("Liberation Serif", 24)
    );
    let pages = pages(&Source { body: &body, numbering, ..Source::default() }, &fonts, Model::Word);
    assert!(pages[0].lines[0].text.starts_with("1."), "{}", pages[0].lines[0].text);
    assert_eq!(pages[1].start, Some(Position { paragraph: 0, offset: 3 }));
}

#[test]
fn fonts_that_are_neither_installed_nor_replaced_are_reported() {
    let Some(fonts) = fonts(&[("Liberation Sans", 'a')]) else { return };
    let body = [
        paragraph(&font("Liberation Sans", 24), "", "installed"),
        // Arial has a designated replacement with the same widths.
        paragraph(&font("Arial", 24), "", "replaced"),
        paragraph(&font("No Such Font", 24), "", "missing"),
    ]
    .concat();
    let result = layout(&docx(&Source { body: &body, ..Source::default() }), &fonts, &Options { model: Model::Word });
    assert_eq!(result.missing_fonts, ["No Such Font"]);
}

#[test]
fn a_face_name_is_read_as_a_family_and_a_style() {
    let Some(fonts) = fonts(&[("Liberation Sans", 'a')]) else { return };
    // Some documents ask for "Arial Bold" rather than Arial in bold.
    for name in ["Liberation Sans Bold", "Arial Bold"] {
        let choice = fonts.choose(Some(name), false, false, 'a', Model::Word).unwrap();
        let font = fonts.get(choice.font);
        assert_eq!(font.family(), "liberation sans", "{name}");
        assert!(font.bold && !font.italic && choice.known_family, "{name}");
    }
    let choice = fonts.choose(Some("Arial Bold Italic"), false, false, 'a', Model::Word).unwrap();
    assert!(fonts.get(choice.font).bold && fonts.get(choice.font).italic);
}

#[test]
fn the_last_line_of_a_page_need_only_fit_as_far_as_its_text() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // Triple spacing at 12 pt: 41.4 pt from line to line, of which the text
    // takes 13.8 pt and the rest is leading below it. The page is 697.9 pt
    // tall: sixteen whole lines, and the text of a seventeenth. Word puts
    // seventeen on the page (its record of where page 2 begins in
    // `paragraph-line space.docx` of the Open XML SDK's test files and in
    // LibreOffice's `firstheadernofooter.docx`).
    let one = paragraph(&font("Liberation Serif", 24), r#"<w:spacing w:line="720" w:lineRule="auto"/>"#, "One line.");
    let pages = pages(&Source { body: &one.repeat(20), ..Source::default() }, &fonts, Model::Word);
    assert_eq!(pages.iter().map(|page| page.lines.len()).collect::<Vec<_>>(), [17, 3]);
    assert!(close(pitches(&pages[0])[0], 41.4, 0.01));
}
