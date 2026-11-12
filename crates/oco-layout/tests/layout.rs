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
    /// Headers and footers: the relationship id each is named by, whether it
    /// is a `header` or a `footer`, and its content.
    stories: &'a [(&'a str, &'a str, &'a str)],
    /// The application the document says saved it; none if empty.
    saved_by: &'a str,
}

impl Default for Source<'_> {
    fn default() -> Self {
        Source {
            body: "",
            styles: "",
            numbering: "",
            section: r#"<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>"#,
            settings: r#"<w:compat><w:compatSetting w:name="compatibilityMode" w:val="15"/></w:compat>"#,
            stories: &[],
            saved_by: "",
        }
    }
}

fn docx(source: &Source) -> Docx {
    let part = |kind: &str, target: &str, id: u8| {
        format!(r#"<Relationship Id="rId{id}" Type="{RELS}/{kind}" Target="{target}"/>"#)
    };
    let story_relationships: String = source
        .stories
        .iter()
        .map(|(id, kind, _)| format!(r#"<Relationship Id="{id}" Type="{RELS}/{kind}" Target="{id}.xml"/>"#))
        .collect();
    let mut parts = vec![
        (
            "word/_rels/document.xml.rels".to_owned(),
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}{}{}{story_relationships}</Relationships>"#,
                part("styles", "styles.xml", 1),
                part("numbering", "numbering.xml", 2),
                part("settings", "settings.xml", 3)
            ),
        ),
        (
            "word/document.xml".to_owned(),
            format!(
                r#"<w:document {W} xmlns:r="{RELS}"><w:body>{}<w:sectPr>{}</w:sectPr></w:body></w:document>"#,
                source.body, source.section
            ),
        ),
        ("word/styles.xml".to_owned(), format!("<w:styles {W}>{}</w:styles>", source.styles)),
        ("word/numbering.xml".to_owned(), format!("<w:numbering {W}>{}</w:numbering>", source.numbering)),
        ("word/settings.xml".to_owned(), format!("<w:settings {W}>{}</w:settings>", source.settings)),
    ];
    if !source.saved_by.is_empty() {
        let properties = "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties";
        parts.push((
            "docProps/app.xml".to_owned(),
            format!(r#"<Properties xmlns="{properties}"><Application>{}</Application></Properties>"#, source.saved_by),
        ));
    }
    for (id, kind, content) in source.stories {
        let root = if *kind == "header" { "hdr" } else { "ftr" };
        parts.push((format!("word/{id}.xml"), format!("<w:{root} {W}>{content}</w:{root}>")));
    }
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
fn on_a_grid_a_multiple_counts_grid_rows_not_the_rows_of_the_text() {
    let Some(fonts) = fonts(&[("Source Han Serif CN", '中')]) else { return };
    // As measured in WPS Office (testdata/zh-open/06-contract): 12 pt text
    // that needs two rows of a 15.6 pt grid is 31.2 pt apart at one and a
    // half lines, the two rows it needs anyway, and not three. 宋体 at
    // 12 pt needs one row, and a row and a half is 23.4 pt.
    let grid = r#"<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1800" w:bottom="1440" w:left="1800"/><w:docGrid w:type="lines" w:linePitch="312"/>"#;
    for (family, line, expected) in [
        ("Source Han Serif CN", 240, 31.2),
        ("Source Han Serif CN", 360, 31.2),
        ("Source Han Serif CN", 720, 46.8),
        ("宋体", 360, 23.4),
        ("宋体", 480, 31.2),
    ] {
        let body = paragraph(&font(family, 24), &format!(r#"<w:spacing w:line="{line}" w:lineRule="auto"/>"#), HANZI);
        let source = Source { body: &body, section: grid, ..Source::default() };
        for pitch in pitches(&pages(&source, &fonts, Model::Word)[0]) {
            assert!(close(pitch, expected, 0.01), "{family} at line={line}: {pitch}, expected {expected}");
        }
    }
}

#[test]
fn text_sits_in_a_line_of_fixed_height_by_its_fonts_own_proportions() {
    let Some(fonts) = fonts(&[("Source Han Serif CN", '中'), ("Liberation Serif", 'a')]) else { return };
    // As measured in WPS Office (testdata/zh-open/07-regulation): in a line
    // of exactly 20 pt the baseline of 思源宋体 is at 1151/1437 of the
    // height, the font's own ascent and descent, although a line that takes
    // its height from the font is given 30% more room. For a Western font
    // the two are the same (Liberation Serif: its ascent with the external
    // leading above it, 1912 of 2355 units), which is what Word shows.
    for (family, text, share) in
        [("Source Han Serif CN", HANZI, 1151.0 / 1437.0), ("Liberation Serif", LOREM, 1912.0 / 2355.0)]
    {
        let body = paragraph(&font(family, 24), r#"<w:spacing w:line="400" w:lineRule="exact"/>"#, text);
        let page = &pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word)[0];
        assert!(close(page.lines[0].baseline, 72.0 + 20.0 * share, 0.02), "{family}: {}", page.lines[0].baseline);
        assert!(close(pitches(page)[0], 20.0, 0.01));
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

#[test]
fn a_line_squeezes_its_punctuation_to_take_one_more_character() {
    let Some(fonts) = fonts(&[("Source Han Serif CN", '中')]) else { return };
    // As measured in WPS Office (testdata/zh-open): a line of full-width
    // text may run over by up to half an em when it holds a punctuation mark
    // with blank to give up, and the mark is narrowed by the excess.
    // The measure here is 20.6 em of 10.5 pt text.
    let section =
        r#"<w:pgSz w:w="7206" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>"#;
    let squeezing = r#"<w:characterSpacingControl w:val="compressPunctuation"/>"#;
    let text = "一二三四五，六七八九十一二三四五六七八九十甲乙丙丁戊己庚辛";
    let body = paragraph(&font("Source Han Serif CN", 21), "", text);
    let first_line = |settings: &str| {
        let pages = pages(&Source { body: &body, section, settings, ..Source::default() }, &fonts, Model::Word);
        pages[0].lines[0].clone()
    };
    let (plain, squeezed) = (first_line(""), first_line(squeezing));
    assert_eq!(plain.text.chars().count(), 20, "{}", plain.text);
    assert_eq!(squeezed.text.chars().count(), 21, "{}", squeezed.text);
    // The squeezed line ends at the margin: twenty-one characters in 20.6 em.
    assert!(close(squeezed.x1, 72.0 + 20.6 * 10.5, 0.05), "{}", squeezed.x1);

    // Without a mark to squeeze, or when it would take more than half an em,
    // the line breaks where it did.
    let without_mark = paragraph(&font("Source Han Serif CN", 21), "", &text.replace('，', "中"));
    let narrow =
        r#"<w:pgSz w:w="7164" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>"#;
    for (body, section) in [(&without_mark, section), (&body, narrow)] {
        let pages = pages(&Source { body, section, settings: squeezing, ..Source::default() }, &fonts, Model::Word);
        assert_eq!(pages[0].lines[0].text.chars().count(), 20, "{}", pages[0].lines[0].text);
    }
}

/// A table of fixed layout with half-point borders, whose cells hold the
/// given paragraphs.
fn table(grid: &[u32], row_props: &str, rows: &[&[&str]]) -> String {
    let edges: String = ["top", "left", "bottom", "right", "insideH", "insideV"]
        .map(|e| format!(r#"<w:{e} w:val="single" w:sz="4"/>"#))
        .concat();
    let columns: String = grid.iter().map(|w| format!(r#"<w:gridCol w:w="{w}"/>"#)).collect();
    let rows: String = rows
        .iter()
        .map(|cells| {
            let cells: String = cells.iter().map(|content| format!("<w:tc>{content}</w:tc>")).collect();
            format!("<w:tr><w:trPr>{row_props}</w:trPr>{cells}</w:tr>")
        })
        .collect();
    format!(
        r#"<w:tbl><w:tblPr><w:tblBorders>{edges}</w:tblBorders><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid>{columns}</w:tblGrid>{rows}</w:tbl>"#
    )
}

#[test]
fn table_text_stands_in_its_column_below_the_rule() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in Word (docx4j's probes `table-fixed` and
    // `table-indent-compat15`): text begins 5.4 pt, the built-in cell margin,
    // and half the border in from the edge of its cell; the rule above a
    // row takes up its own half point, and so does the one under the table.
    let cell = |text: &str| paragraph(&font("Liberation Serif", 24), "", text);
    let (a, b, c, d) = (cell("a"), cell("b"), cell("c"), cell("d"));
    let body = table(&[2000, 3000], "", &[&[&a, &b], &[&c, &d]]) + &cell("after");
    let page = &pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word)[0];
    let at = |text: &str| page.lines.iter().find(|line| line.text == text).unwrap_or_else(|| panic!("{text}"));
    assert!(close(at("a").x0, 72.0 + 5.4 + 0.25, 0.01), "{}", at("a").x0);
    assert!(close(at("b").x0, 72.0 + 100.0 + 5.4 + 0.25, 0.01), "{}", at("b").x0);
    // 11.2 pt is the ascent of the font with its leading; 13.8 pt its line.
    assert!(close(at("a").baseline, 72.0 + 0.5 + 11.2, 0.03), "{}", at("a").baseline);
    assert!(close(at("b").baseline, at("a").baseline, 0.001));
    assert!(close(at("c").baseline - at("a").baseline, 13.8 + 0.5, 0.01));
    assert!(close(at("after").baseline - at("c").baseline, 13.8 + 0.5, 0.01));
}

#[test]
fn a_row_breaks_between_lines_at_the_foot_of_a_page_unless_told_not_to() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // Lines of exactly 20 pt: the page of 697.9 pt holds 34. Thirty of them
    // come first; a one-row table of ten follows.
    let line =
        |text: &str| paragraph(&font("Liberation Serif", 24), r#"<w:spacing w:line="400" w:lineRule="exact"/>"#, text);
    let filler: String = (0..30).map(|n| line(&format!("filler {n}"))).collect();
    let cell: String = (0..10).map(|n| line(&format!("cell {n}"))).collect();
    let on_pages = |row_props: &str| {
        let body = filler.clone() + &table(&[4000], row_props, &[&[&cell]]);
        let pages = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word);
        pages.iter().map(|page| page.lines.iter().filter(|l| l.text.starts_with("cell")).count()).collect::<Vec<_>>()
    };
    // Four lines fit under the filler, less the half point of the rule: the
    // row breaks after its fourth line. (As Word breaks the rows of docx4j's
    // probe `table-rowsplit`, all 448 lines of which fall where Word has them.)
    assert_eq!(on_pages(""), [4, 6]);
    assert_eq!(on_pages("<w:cantSplit/>"), [0, 10]);
    // A row of exact height does not break either.
    assert_eq!(on_pages(r#"<w:trHeight w:val="4000" w:hRule="exact"/>"#), [0, 10]);
}

#[test]
fn the_lines_of_a_cell_are_not_left_alone_where_a_row_breaks() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in Word (docx4j's probe `table-rowsplit`): a row whose
    // cell is a paragraph of two lines goes to the next page whole where
    // only the first line fits; the care taken of widows and orphans holds
    // inside cells. Lines of exactly 20 pt, 34 to the page.
    let exact = r#"<w:spacing w:line="400" w:lineRule="exact"/>"#;
    let line = |text: &str| paragraph(&font("Liberation Serif", 24), exact, text);
    // One paragraph of `count` lines, by line breaks.
    let lines = |count: usize, props: &str| {
        let breaks = "cell<w:br/>".repeat(count - 1);
        format!(
            r#"<w:p><w:pPr>{exact}{props}</w:pPr><w:r><w:rPr>{}</w:rPr><w:t>{breaks}cell</w:t></w:r></w:p>"#,
            font("Liberation Serif", 24)
        )
        .replace("cell<w:br/>", "</w:t><w:t>cell</w:t><w:br/><w:t>")
    };
    let on_pages = |fillers: usize, cell: &str| {
        let filler: String = (0..fillers).map(|n| line(&format!("filler {n}"))).collect();
        let body = filler + &table(&[4000], "", &[&[cell]]);
        let pages = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word);
        pages.iter().map(|page| page.lines.iter().filter(|l| l.text.starts_with("cell")).count()).collect::<Vec<_>>()
    };
    // Room for one line under 33 fillers: two lines stay together.
    assert_eq!(on_pages(33, &lines(2, "")), [0, 2]);
    // Unless the paragraph does without that care.
    assert_eq!(on_pages(33, &lines(2, r#"<w:widowControl w:val="0"/>"#)), [1, 1]);
    // Five lines break two and three, not three and two or four and one,
    // where there is room for four; and not at all where it is for one.
    assert_eq!(on_pages(30, &lines(5, "")), [3, 2]);
    assert_eq!(on_pages(31, &lines(5, "")), [3, 2]);
    assert_eq!(on_pages(32, &lines(5, "")), [2, 3]);
    assert_eq!(on_pages(33, &lines(5, "")), [0, 5]);
    // Lines kept together are.
    assert_eq!(on_pages(31, &lines(5, "<w:keepLines/>")), [0, 5]);
}

#[test]
fn the_rule_over_a_row_comes_on_top_of_its_least_height() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in Word (docx4j's probe `table-rowheight`): from rule to
    // rule a row of at least 30 pt is 30.5 pt with a half-point rule, a row
    // of exactly 30 pt is 30 pt, and a row of one 13.8 pt line 14.3 pt.
    let cell = |text: &str| paragraph(&font("Liberation Serif", 24), "", text);
    let step = |row_props: &str| {
        let (a, b) = (cell("above"), cell("below"));
        let body = table(&[4000], row_props, &[&[&a], &[&b]]);
        let page = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word).remove(0);
        page.lines[1].baseline - page.lines[0].baseline
    };
    assert!(close(step(""), 14.3, 0.01), "{}", step(""));
    assert!(close(step(r#"<w:trHeight w:val="600"/>"#), 30.5, 0.01), "{}", step(r#"<w:trHeight w:val="600"/>"#));
    assert!(close(step(r#"<w:trHeight w:val="600" w:hRule="exact"/>"#), 30.0, 0.01));
    // Less than the content asks for changes nothing.
    assert!(close(step(r#"<w:trHeight w:val="100"/>"#), 14.3, 0.01));
}

#[test]
fn a_row_keeps_with_the_next_if_its_first_paragraph_does() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in Word (docx4j's probe `table-row-keepnext`): a row goes
    // to the next page with what follows it if the first paragraph of its
    // first cell is to be kept with the next, whatever its other paragraphs
    // say. Lines of exactly 20 pt, 34 to the page and 17.9 pt over; a table
    // of two rows comes after 33, and a paragraph after the table.
    let exact = r#"<w:spacing w:line="400" w:lineRule="exact"/>"#;
    let line = |text: &str, keep: bool| {
        let props = format!("{exact}{}", if keep { "<w:keepNext/>" } else { "" });
        paragraph(&font("Liberation Serif", 24), &props, text)
    };
    let first_page = |cells: [[bool; 2]; 2]| {
        let filler: String = (0..33).map(|n| line(&format!("filler {n}"), false)).collect();
        let row = |name: &str, keeps: [bool; 2]| [line(name, keeps[0]), line("other", keeps[1])];
        let (one, two) = (row("row one", cells[0]), row("row two", cells[1]));
        let body =
            filler + &table(&[2000, 2000], "", &[&[&one[0], &one[1]], &[&two[0], &two[1]]]) + &line("after", false);
        let pages = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word);
        pages[0].lines.iter().filter(|l| l.text.starts_with("row")).count()
    };
    // Nothing is kept: the first row has room, the second has not.
    assert_eq!(first_page([[false, false], [false, false]]), 1);
    // The first row is kept with the second, in its first cell.
    assert_eq!(first_page([[true, false], [false, false]]), 0);
    // The last cell's paragraph does not count.
    assert_eq!(first_page([[false, true], [false, true]]), 1);
    // Both rows kept, with each other and with the paragraph after: all
    // go over.
    assert_eq!(first_page([[true, true], [true, true]]), 0);
}

#[test]
fn an_empty_paragraph_is_a_line_of_its_own_font() {
    let Some(fonts) = fonts(&[("Carlito", 'a')]) else { return };
    // As measured in Word (docx4j's probes `tables-banding-band-size` and
    // `spacing-empty-heading`): where every default font is the same Western
    // font, an empty paragraph is one line of that font, 13.44 pt for 11 pt
    // Carlito, and not the taller line of a Chinese font brought in because
    // the paragraph mark was taken for East Asian text.
    let styles = r#"<w:docDefaults><w:rPrDefault><w:rPr>
        <w:rFonts w:ascii="Carlito" w:hAnsi="Carlito" w:eastAsia="Carlito" w:cs="Carlito"/><w:sz w:val="22"/>
        </w:rPr></w:rPrDefault></w:docDefaults>"#;
    let body = "<w:p><w:r><w:t>above</w:t></w:r></w:p><w:p/><w:p/><w:p><w:r><w:t>below</w:t></w:r></w:p>";
    let page = &pages(&Source { body, styles, ..Source::default() }, &fonts, Model::Word)[0];
    assert!(close(page.lines[1].baseline - page.lines[0].baseline, 3.0 * 13.44, 0.03), "{:?}", pitches(page));
}

#[test]
fn an_empty_paragraph_is_of_the_western_font_unless_its_mark_is_hinted_east_asian() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a'), ("Source Han Serif CN", '中')]) else { return };
    // As measured in Word (docx4j's probe `firstheadernofooter`, among
    // others): where the Western and the East Asian font differ, an empty
    // paragraph is a line of the Western one, 13.8 pt for 12 pt Liberation
    // Serif. Only a mark hinted as East Asian is a line of the other font,
    // as tall as a line with a Chinese character in it.
    let styles = r#"<w:docDefaults><w:rPrDefault><w:rPr>
        <w:rFonts w:ascii="Liberation Serif" w:hAnsi="Liberation Serif" w:eastAsia="Source Han Serif CN"/>
        <w:sz w:val="24"/></w:rPr></w:rPrDefault></w:docDefaults>"#;
    let step = |middle: &str| {
        let body = format!("<w:p><w:r><w:t>above</w:t></w:r></w:p>{middle}<w:p><w:r><w:t>below</w:t></w:r></w:p>");
        let page = &pages(&Source { body: &body, styles, ..Source::default() }, &fonts, Model::Word)[0];
        page.lines.last().unwrap().baseline - page.lines[0].baseline
    };
    let (empty, chinese) = (step("<w:p/>"), step("<w:p><w:r><w:t>中</w:t></w:r></w:p>"));
    let hinted = step(r#"<w:p><w:pPr><w:rPr><w:rFonts w:hint="eastAsia"/></w:rPr></w:pPr></w:p>"#);
    assert!(close(empty, 2.0 * 13.8, 0.03), "{empty}");
    assert!(chinese > empty + 2.0, "{chinese} {empty}");
    assert!(close(hinted, chinese, 0.01), "{hinted} {chinese}");
}

const A4_WITH_STORIES: &str = r#"<w:pgSz w:w="11906" w:h="16838"/>
    <w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708"/>"#;

#[test]
fn headers_and_footers_stand_at_their_distances_and_push_the_body() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in Word (docx4j's probes `page-header-footer` and
    // `page-tall-header`): a header begins 35.4 pt, its distance, from the
    // top of the page, and a footer ends as far from the bottom. The body
    // begins at the margin, 72 pt, unless the header reaches lower.
    let line = |text: &str| paragraph(&font("Liberation Serif", 24), "", text);
    let references = r#"<w:headerReference w:type="default" r:id="h"/><w:footerReference w:type="default" r:id="f"/>"#;
    let section = format!("{references}{A4_WITH_STORIES}");
    let lay = |header: &str| {
        let stories = [("h", "header", header), ("f", "footer", "<w:p><w:r><w:t>foot</w:t></w:r></w:p>")];
        let body = line("body");
        let source = Source { body: &body, section: &section, stories: &stories, ..Source::default() };
        pages(&source, &fonts, Model::Word).remove(0)
    };
    // 12 pt Liberation Serif: a line of 13.8 pt, its baseline 11.2 pt down.
    let short = lay(&line("head"));
    let at = |page: &Page, text: &str| page.lines.iter().find(|l| l.text == text).map(|l| l.baseline).unwrap();
    assert!(close(at(&short, "head"), 35.4 + 11.2, 0.03), "{}", at(&short, "head"));
    assert!(close(at(&short, "body"), 72.0 + 11.2, 0.03), "{}", at(&short, "body"));
    // The footer is of the document's default 10 pt: a line of 11.5 pt
    // whose baseline is 9.35 pt down, ending 35.4 pt above the page's foot.
    assert!(close(at(&short, "foot"), 841.9 - 35.4 - 11.5 + 9.35, 0.05), "{}", at(&short, "foot"));
    assert!(close(short.body.0, 72.0, 0.01) && close(short.body.1, 841.9 - 72.0, 0.01), "{:?}", short.body);

    // Five lines of header reach 35.4 + 69 = 104.4 pt down: the body waits.
    let tall = lay(&line("head").repeat(5));
    assert!(close(at(&tall, "body"), 104.4 + 11.2, 0.03), "{}", at(&tall, "body"));
    assert!(close(tall.body.0, 104.4, 0.01), "{:?}", tall.body);

    // A top margin written as a negative number holds, and the two overlap.
    let fixed = section.replace(r#"w:top="1440""#, r#"w:top="-1440""#);
    let header = line("head").repeat(5);
    let stories = [("h", "header", header.as_str())];
    let body = line("body");
    let page =
        pages(&Source { body: &body, section: &fixed, stories: &stories, ..Source::default() }, &fonts, Model::Word)
            .remove(0);
    assert!(close(at(&page, "body"), 72.0 + 11.2, 0.03), "{}", at(&page, "body"));
}

#[test]
fn each_page_gets_the_header_meant_for_it_with_its_number() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    let text = |text: &str| format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>");
    let numbered = r#"<w:p><w:r><w:t xml:space="preserve">page </w:t></w:r>
        <w:fldSimple w:instr=" PAGE "><w:r><w:t>9</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> of </w:t></w:r>
        <w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> NUMPAGES </w:instrText></w:r>
        <w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>9</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
    let (first, even, odd) = (text("first"), text("even"), text("odd"));
    let stories = [
        ("a", "header", first.as_str()),
        ("b", "header", even.as_str()),
        ("c", "header", odd.as_str()),
        ("n", "footer", numbered),
    ];
    let section = format!(
        r#"<w:headerReference w:type="first" r:id="a"/><w:headerReference w:type="even" r:id="b"/>
           <w:headerReference w:type="default" r:id="c"/><w:footerReference w:type="default" r:id="n"/>
           {A4_WITH_STORIES}<w:titlePg/>"#
    );
    // Four pages: a paragraph on each, every one after the first asking for a new page.
    let body = paragraph("", "", "one") + &paragraph("", "<w:pageBreakBefore/>", "x").repeat(3);
    let lay = |settings: &str| {
        let source = Source { body: &body, section: &section, settings, stories: &stories, ..Source::default() };
        let pages = pages(&source, &fonts, Model::Word);
        pages
            .iter()
            .map(|page| (page.lines[0].text.clone(), page.lines.last().unwrap().text.clone()))
            .collect::<Vec<_>>()
    };
    let shown = |pairs: &[(&str, &str)]| pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
    // The first page has the title page's header and, having no footer of
    // its own kind, none; the numbers are the pages', not the ones stored.
    assert_eq!(
        lay("<w:evenAndOddHeaders/>"),
        shown(&[("first", "one"), ("even", "x"), ("odd", "page 3 of 4"), ("even", "x")])
    );
    // Unless the document tells odd and even pages apart, every other page
    // has the default header and footer.
    assert_eq!(
        lay(""),
        shown(&[("first", "one"), ("odd", "page 2 of 4"), ("odd", "page 3 of 4"), ("odd", "page 4 of 4")])
    );
}

#[test]
fn a_section_without_a_header_of_its_own_has_the_one_before_and_may_renumber() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    let number = r#"<w:p><w:fldSimple w:instr="PAGE"><w:r><w:t>0</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> of </w:t></w:r>
        <w:fldSimple w:instr="NUMPAGES"><w:r><w:t>0</w:t></w:r></w:fldSimple></w:p>"#;
    let stories = [("h", "header", "<w:p><w:r><w:t>head</w:t></w:r></w:p>"), ("n", "footer", number)];
    // The first section names the header and footer; the second names
    // neither, starts on a new page, and numbers from iv in roman: its
    // first page is iv, and the pages after it count on from there. The
    // count of pages stays in figures (as WPS shows it; testdata/zh-open,
    // 11-header-footer).
    let body = format!(
        r#"<w:p><w:pPr><w:sectPr><w:headerReference r:id="h"/><w:footerReference r:id="n"/>{A4_WITH_STORIES}</w:sectPr></w:pPr>
           <w:r><w:t>one</w:t></w:r></w:p><w:p><w:r><w:t>two</w:t></w:r></w:p>
           <w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>three</w:t></w:r></w:p>"#
    );
    let section = format!(r#"{A4_WITH_STORIES}<w:pgNumType w:start="4" w:fmt="lowerRoman"/>"#);
    let source = Source { body: &body, section: &section, stories: &stories, ..Source::default() };
    let pages = pages(&source, &fonts, Model::Word);
    let texts: Vec<Vec<&str>> = pages.iter().map(|page| page.lines.iter().map(|l| l.text.as_str()).collect()).collect();
    assert_eq!(texts, [["head", "one", "1 of 3"], ["head", "two", "iv of 3"], ["head", "three", "v of 3"]]);
}

#[test]
fn only_a_footer_that_is_there_takes_room_from_the_body() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // The footer's distance, 100 pt, is more than the bottom margin, 72 pt.
    // With a footer of one 11.5 pt line the body ends above it; on a page
    // without one the distance means nothing and the body ends at the
    // margin (Word, docx4j's probe `firstheadernofooter`).
    let far = A4_WITH_STORIES.replace(r#"w:footer="708""#, r#"w:footer="2000""#);
    let stories = [("f", "footer", "<w:p><w:r><w:t>foot</w:t></w:r></w:p>")];
    let body = paragraph("", "", "body");
    let foot_of_body = |section: &str| {
        let source = Source { body: &body, section, stories: &stories, ..Source::default() };
        pages(&source, &fonts, Model::Word)[0].body.1
    };
    let with = foot_of_body(&format!(r#"<w:footerReference w:type="default" r:id="f"/>{far}"#));
    assert!(close(with, 841.9 - 100.0 - 11.5, 0.03), "{with}");
    let without = foot_of_body(&far);
    assert!(close(without, 841.9 - 72.0, 0.03), "{without}");
    // A bottom margin written as a negative number holds against the footer.
    let fixed = far.replace(r#"w:bottom="1440""#, r#"w:bottom="-1440""#);
    let held = foot_of_body(&format!(r#"<w:footerReference w:type="default" r:id="f"/>{fixed}"#));
    assert!(close(held, 841.9 - 72.0, 0.03), "{held}");
}

#[test]
fn a_floating_table_in_a_header_takes_no_room() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // Word, as recorded in LibreOffice's test document
    // `floating-table-header.docx`, which Word has on one page: a table that
    // floats is not part of the header's height, so the body does not wait
    // for it. Here the paragraph after such a table begins where it began.
    let line = |text: &str| paragraph(&font("Liberation Serif", 24), "", text);
    let cell = [line("a1"), line("a2"), line("a3"), line("a4"), line("a5")].concat();
    let floating = table(&[2000], "", &[&[&cell]]).replace("<w:tblPr>", r#"<w:tblPr><w:tblpPr w:tblpY="1"/>"#);
    let lay = |table: &str| {
        let header = [line("head"), table.to_owned(), line("under")].concat();
        let stories = [("h", "header", header.as_str())];
        let section = format!(r#"<w:headerReference w:type="default" r:id="h"/>{A4_WITH_STORIES}"#);
        let body = line("body");
        let source = Source { body: &body, section: &section, stories: &stories, ..Source::default() };
        pages(&source, &fonts, Model::Word).remove(0)
    };
    let at = |page: &Page, text: &str| page.lines.iter().find(|l| l.text == text).map(|l| l.baseline).unwrap();
    // In the flow, the table's five lines carry the header past the margin.
    let inline = lay(&floating.replace(r#"<w:tblpPr w:tblpY="1"/>"#, ""));
    assert!(inline.body.0 > 72.0 + 30.0, "{:?}", inline.body);
    assert!(at(&inline, "under") - at(&inline, "head") > 5.0 * 13.8);
    // Floating, it leaves the two paragraphs one under the other, and the
    // body at its margin.
    let page = lay(&floating);
    assert!(close(page.body.0, 72.0, 0.01), "{:?}", page.body);
    assert!(close(at(&page, "under") - at(&page, "head"), 13.8, 0.03), "{}", at(&page, "under"));
    assert!(close(at(&page, "body"), 72.0 + 11.2, 0.03), "{}", at(&page, "body"));
    assert!(page.lines.iter().any(|l| l.text.ends_with("a5")), "the table is still drawn");
}

#[test]
fn the_border_under_a_paragraph_takes_its_space_and_its_width() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in WPS (testdata/zh-open, 05-official-notice): a 3 pt
    // border 6 pt under a paragraph moves what follows down by 9 pt.
    let bordered = |size: u32, text: &str| {
        let border =
            format!(r#"<w:pBdr><w:bottom w:val="single" w:sz="{size}" w:space="6" w:color="FF0000"/></w:pBdr>"#);
        paragraph(&font("Liberation Serif", 24), &border, text)
    };
    let plain = |text: &str| paragraph(&font("Liberation Serif", 24), "", text);
    let steps = |body: String| {
        let page = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word).remove(0);
        pitches(&page)
    };
    let close_to = |found: &[f32], wanted: &[f32]| {
        found.len() == wanted.len() && found.iter().zip(wanted).all(|(a, b)| close(*a, *b, 0.03))
    };
    // 12 pt Liberation Serif is a line of 13.8 pt.
    let one = steps(bordered(24, "a") + &plain("b") + &plain("c"));
    assert!(close_to(&one, &[13.8 + 9.0, 13.8]), "{one:?}");
    // Paragraphs with the same border are bordered as one: the border, and
    // the room it takes, come after the last of them.
    let joined = steps(bordered(24, "a") + &bordered(24, "b") + &plain("c"));
    assert!(close_to(&joined, &[13.8, 13.8 + 9.0]), "{joined:?}");
    // A different border is a border of its own: 3 pt and 1.5 pt, 6 pt away.
    let apart = steps(bordered(24, "a") + &bordered(12, "b") + &plain("c"));
    assert!(close_to(&apart, &[13.8 + 9.0, 13.8 + 7.5]), "{apart:?}");
}

#[test]
fn the_border_under_a_header_is_part_of_its_height() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in WPS (testdata/zh-open, 11-header-footer): under a header
    // that reaches below the top margin, the body begins lower by the space
    // and the width of the header's bottom border, here 1 pt and 0.75 pt.
    let line = |props: &str| paragraph(&font("Liberation Serif", 24), props, "head");
    let rule = r#"<w:pBdr><w:bottom w:val="single" w:sz="6" w:space="1" w:color="auto"/></w:pBdr>"#;
    let top_of_body = |last: &str| {
        let header = line("").repeat(4) + &line(last);
        let stories = [("h", "header", header.as_str())];
        let section = format!(r#"<w:headerReference w:type="default" r:id="h"/>{A4_WITH_STORIES}"#);
        let body = paragraph("", "", "body");
        let source = Source { body: &body, section: &section, stories: &stories, ..Source::default() };
        pages(&source, &fonts, Model::Word)[0].body.0
    };
    // Five lines of 13.8 pt from 35.4 pt down: 104.4 pt.
    assert!(close(top_of_body(""), 104.4, 0.02), "{}", top_of_body(""));
    assert!(close(top_of_body(rule), 104.4 + 1.75, 0.02), "{}", top_of_body(rule));
}

/// A table with half-point borders that is fitted to its content. Each cell
/// is given by its properties (the content of `w:tcPr`) and its content.
fn fitted(table_props: &str, grid: &[u32], rows: &[&[(&str, &str)]]) -> String {
    let edges: String = ["top", "left", "bottom", "right", "insideH", "insideV"]
        .map(|e| format!(r#"<w:{e} w:val="single" w:sz="4"/>"#))
        .concat();
    let columns: String = grid.iter().map(|w| format!(r#"<w:gridCol w:w="{w}"/>"#)).collect();
    let rows: String = rows
        .iter()
        .map(|cells| {
            let cells: String = cells
                .iter()
                .map(|(props, content)| format!("<w:tc><w:tcPr>{props}</w:tcPr>{content}</w:tc>"))
                .collect();
            format!("<w:tr>{cells}</w:tr>")
        })
        .collect();
    format!(
        "<w:tbl><w:tblPr>{table_props}<w:tblBorders>{edges}</w:tblBorders></w:tblPr><w:tblGrid>{columns}</w:tblGrid>{rows}</w:tbl>"
    )
}

/// Where the line that starts with `text` begins.
fn starts(page: &Page, text: &str) -> f32 {
    page.lines.iter().find(|line| line.text.starts_with(text)).unwrap_or_else(|| panic!("no line {text:?}")).x0
}

const LONG: &str = "The quick brown fox jumps over the lazy dog while the farmer watches from the gate. Lorem ipsum \
    dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.";

#[test]
fn an_auto_fit_table_shares_its_room_by_what_its_columns_ask() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in Word (docx4j's probe `table-width`, where the grid is
    // three equal columns that Word does not follow). A column is at least
    // its longest word and would like to be its longest paragraph, with the
    // cell margins; each gets its least and the same fraction of the rest
    // of what it would like. "short" keeps its 24 pt and 10.8 pt of margins;
    // "medium length cell" breaks after "medium" or after "length"
    // according to the room.
    let cell = |text: &str| paragraph(&font("Liberation Serif", 24), "", text);
    let lay = |width: &str| {
        let (short, medium, long, a) = (cell("short"), cell("medium length cell"), cell(LONG), cell("a"));
        let auto = r#"<w:tcW w:w="0" w:type="auto"/>"#;
        let body = fitted(
            width,
            &[3000, 3000, 3000],
            &[&[(auto, &short), (auto, &medium), (auto, &long)], &[(auto, &a), (auto, &a), (auto, &a)]],
        );
        let page = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word).remove(0);
        (starts(&page, "short"), starts(&page, "medium"), starts(&page, "The quick"))
    };
    // Text begins 5.4 pt and half the border in from the middle of the rule.
    let wanted = |found: (f32, f32, f32), third: f32| {
        assert!(close(found.0, 77.66, 0.03) && close(found.1, 112.46, 0.03) && close(found.2, third, 0.1), "{found:?}");
    };
    // The table takes all the room: the width of the text less its border.
    wanted(lay(r#"<w:tblW w:w="0" w:type="auto"/>"#), 178.49);
    wanted(lay(r#"<w:tblW w:w="0" w:type="auto"/><w:tblLayout w:type="autofit"/>"#), 178.49);
    // A stated width, and half the room.
    wanted(lay(r#"<w:tblW w:w="5000" w:type="dxa"/>"#), 167.93);
    wanted(lay(r#"<w:tblW w:w="2500" w:type="pct"/>"#), 166.58);
}

#[test]
fn stated_cell_widths_decide_before_the_grid_and_before_content() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in Word (docx4j's probes `table-grid-vs-tcw-compat15` and
    // `table-autofit`).
    let cell = |text: &str| paragraph(&font("Liberation Serif", 24), "", text);
    let width = |twips: u32| format!(r#"<w:tcW w:w="{twips}" w:type="dxa"/>"#);
    let lay = |table: &str, cells: [&str; 3], texts: [&str; 3]| {
        let (a, b, c) = (cell(texts[0]), cell(texts[1]), cell(texts[2]));
        let body = fitted(table, &[2000, 3000, 4000], &[&[(cells[0], &a), (cells[1], &b), (cells[2], &c)]]);
        let page = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word).remove(0);
        // The left edges of the second and third columns, from the first's.
        let at = |text: &str| starts(&page, &text[..5]) - starts(&page, &texts[0][..5]);
        (at(texts[1]), at(texts[2]))
    };
    let nine = r#"<w:tblW w:w="9000" w:type="dxa"/>"#;
    let wraps = [
        "D c1 left edge here; this text wraps.",
        "D c2 left edge here; this text wraps.",
        "D c3 left edge here; this text wraps.",
    ];
    // The cells say 200, 150 and 100 pt; the grid says the reverse.
    let (w4, w3, w2) = (width(4000), width(3000), width(2000));
    let stated = lay(nine, [&w4, &w3, &w2], wraps);
    assert!(close(stated.0, 200.0, 0.05) && close(stated.1, 350.0, 0.05), "{stated:?}");
    // A column that states nothing has what the table has left, 100 pt,
    // though its text on one line would be 180 pt: it gives way before
    // the columns that state their widths do.
    let rest = lay(nine, ["", &w3, &w4], wraps);
    assert!(close(rest.0, 100.0, 0.05) && close(rest.1, 250.0, 0.05), "{rest:?}");
    // Stated widths that come to more than the room, 582.2 pt for 450.8 pt,
    // give way like any others: each column keeps its least, 42.46 pt for
    // the longest word here, and the same fraction of the rest. Word has
    // the columns 111.89 pt and 168.79 pt wide.
    let (w28, w44, w4444) = (width(2800), width(4400), width(4444));
    let over = lay(r#"<w:tblW w:w="0" w:type="auto"/>"#, [&w28, &w44, &w4444], wraps);
    assert!(close(over.0, 111.89, 0.15) && close(over.1, 111.89 + 168.79, 0.2), "{over:?}");
    // The room is less by the table's indent: it reaches from where the
    // indent puts it to the far margin. (Word, in a document it saved,
    // LibreOffice's fdo78904_fileHang: one column stated at 442.8 pt in a
    // text 432 pt wide is 414 pt behind an indent of 18 pt.) Here the
    // third column is down to its least, 42.46 pt, and the two that state
    // 237.5 pt each share the rest of 450.8 pt less 18 pt.
    let w475 = width(4750);
    let indented =
        lay(r#"<w:tblW w:w="0" w:type="auto"/><w:tblInd w:w="360" w:type="dxa"/>"#, [&w475, &w475, ""], wraps);
    assert!(close(indented.0, (450.8 - 18.0 - 42.46) / 2.0, 0.1), "{indented:?}");
}

#[test]
fn a_cell_that_spans_columns_widens_them_in_proportion() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in Word (docx4j's probe `table-span`): the first column
    // holds "one" and "first" and would be 30.1 pt, but the cell that spans
    // it and the second asks for more than the two together, and what is
    // lacking goes to each in proportion to what it asks itself. Word has
    // the columns 31.32, 384.80 and 34.82 pt wide.
    let cell = |text: &str| paragraph(&font("Liberation Serif", 24), "", text);
    let auto = r#"<w:tcW w:w="0" w:type="auto"/>"#;
    let two = format!(r#"{auto}<w:gridSpan w:val="2"/>"#);
    let texts = [
        cell("spans two columns: Pagination is the process of dividing a document into discrete pages, either electronic pages or printed pages."),
        cell("third"),
        cell("one"),
        cell("two Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat."),
        cell("three"),
        cell("first"),
        cell("spans two: A typesetting engine must decide where each line ends and where each page ends; small differences accumulate."),
    ];
    let body = fitted(
        r#"<w:tblW w:w="0" w:type="auto"/>"#,
        &[3000, 3000, 3000],
        &[
            &[(&two, &texts[0]), (auto, &texts[1])],
            &[(auto, &texts[2]), (auto, &texts[3]), (auto, &texts[4])],
            &[(auto, &texts[5]), (&two, &texts[6])],
        ],
    );
    let page = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word).remove(0);
    assert!(close(starts(&page, "one"), 77.66, 0.03), "{}", starts(&page, "one"));
    assert!(close(starts(&page, "two Ut"), 108.98, 0.05), "{}", starts(&page, "two Ut"));
    assert!(close(starts(&page, "three"), 493.78, 0.15), "{}", starts(&page, "three"));
}

#[test]
fn a_table_in_a_cell_is_part_of_what_the_cell_holds() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // As measured in Word (docx4j's probe `table-nested`): a table of 150 pt
    // in a cell makes the column 150 pt, its border and the cell's margins
    // wide, 161.3 pt, however little text there is beside it; and its lines
    // are lines of the page like any others.
    let cell = |text: &str| paragraph(&font("Liberation Serif", 24), "", text);
    let (a, b) = (cell("in a"), cell("in b"));
    let inner = table(&[1500, 1500], "", &[&[&a, &b]]);
    let (left, middle, right) = (cell(LONG), inner + &cell("after"), cell("outer right"));
    let auto = r#"<w:tcW w:w="0" w:type="auto"/>"#;
    let body =
        fitted(r#"<w:tblW w:w="0" w:type="auto"/>"#, &[3000; 3], &[&[(auto, &left), (auto, &middle), (auto, &right)]]);
    let page = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word).remove(0);
    // From the text of the inner table's first cell to that of the next
    // column: 150 pt and the two cell margins between the tables' edges.
    let across = starts(&page, "outer") - starts(&page, "in a");
    assert!(close(across, 150.0 + 5.4 + 0.25, 0.05), "{across}");
    assert!(close(starts(&page, "in b") - starts(&page, "in a"), 75.0, 0.01));

    // A table that states its width keeps it though it is fitted to its
    // content: the column cannot be less. (Word, in a document it saved,
    // POI's bug65738: a table stated at 210.5 pt in a column stated at
    // 432 pt that had to give way left the column 314.6 pt.)
    let stated = fitted(r#"<w:tblW w:w="3000" w:type="dxa"/>"#, &[1500, 1500], &[&[(auto, &a), (auto, &b)]]);
    let (left, middle, right) = (cell(LONG), stated + &cell("after"), cell("outer right"));
    let body =
        fitted(r#"<w:tblW w:w="0" w:type="auto"/>"#, &[3000; 3], &[&[(auto, &left), (auto, &middle), (auto, &right)]]);
    let page = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word).remove(0);
    let across = starts(&page, "outer") - starts(&page, "in a");
    assert!(close(across, 150.0 + 5.4 + 0.25, 0.05), "{across}");
}

#[test]
fn columns_that_cannot_all_have_their_least_are_cut_down_alike() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    // Word, in a document it saved (LibreOffice's test document tdf123054):
    // where the words are too long for the page the table is still as wide
    // as the room there is, and the words are broken.
    let cell = |text: &str| paragraph(&font("Liberation Serif", 24), "", text);
    let (short, long) = (cell(&"e".repeat(60)), cell(&"e".repeat(120)));
    let auto = r#"<w:tcW w:w="0" w:type="auto"/>"#;
    let body =
        fitted(r#"<w:tblW w:w="0" w:type="auto"/>"#, &[3000; 2], &[&[(auto, &short), (auto, &long)]]) + &cell("after");
    let page = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word).remove(0);
    // 60 and 120 letters of 5.33 pt with 10.8 pt of margins: 330.4 and
    // 650.0 pt, cut down to the 450.8 pt there are.
    let second = page.lines.iter().map(|line| line.x0).fold(0.0, f32::max);
    assert!(close(second - 77.65, 450.8 * 330.4 / 980.4, 0.3), "{second}");
    assert!(page.lines.len() > 4, "the words are broken across lines");
}

#[test]
fn what_cannot_be_measured_has_the_width_word_left_in_the_grid() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    let cell = |family: &str, text: &str| paragraph(&font(family, 24), "", text);
    let auto = r#"<w:tcW w:w="0" w:type="auto"/>"#;
    let second = |first: &str, saved_by: &str| {
        let other = cell("Liberation Serif", "other");
        let body = fitted(r#"<w:tblW w:w="0" w:type="auto"/>"#, &[3000, 2000], &[&[(auto, first), (auto, &other)]]);
        let page = pages(&Source { body: &body, saved_by, ..Source::default() }, &fonts, Model::Word).remove(0);
        starts(&page, "other") - 77.65
    };
    // A picture is not read, so the cell's content cannot be measured: the
    // column is as wide as the grid has it, 150 pt, not as its one word.
    let picture = "<w:p><w:r><w:t>word</w:t></w:r><w:r><w:drawing/></w:r></w:p>";
    assert!(close(second(picture, ""), 150.0, 0.05), "{}", second(picture, ""));
    assert!(second(&cell("Liberation Serif", "word"), "") < 40.0);
    // Text in a font that is missing is measured in another, which may be
    // wider or narrower. In a document that Word saved the grid is Word's
    // own measure; in any other it is not to be relied on.
    let missing = cell("No Such Font", "word");
    assert!(close(second(&missing, "Microsoft Office Word"), 150.0, 0.05));
    assert!(second(&missing, "") < 40.0);
}

#[test]
fn a_row_begins_after_the_columns_it_leaves_out() {
    let Some(fonts) = fonts(&[("Liberation Serif", 'a')]) else { return };
    let cell = |text: &str| paragraph(&font("Liberation Serif", 24), "", text);
    let width = |twips: u32| format!(r#"<w:tcW w:w="{twips}" w:type="dxa"/>"#);
    let (w1, w2) = (width(1000), width(2000));
    let (a, b, c) = (cell("a"), cell("b"), cell("c"));
    let table = fitted(r#"<w:tblW w:w="0" w:type="auto"/>"#, &[1000, 2000], &[&[(&w1, &a), (&w2, &b)], &[(&w2, &c)]]);
    // The second row has one cell, in the second column.
    let body = table.replacen(
        "<w:tr><w:tc><w:tcPr><w:tcW w:w=\"2000\"",
        "<w:tr><w:trPr><w:gridBefore w:val=\"1\"/></w:trPr><w:tc><w:tcPr><w:tcW w:w=\"2000\"",
        1,
    );
    assert_ne!(body, table);
    let page = pages(&Source { body: &body, ..Source::default() }, &fonts, Model::Word).remove(0);
    assert!(close(starts(&page, "b") - starts(&page, "a"), 50.0, 0.01));
    assert!(close(starts(&page, "c"), starts(&page, "b"), 0.01), "{}", starts(&page, "c"));
}
