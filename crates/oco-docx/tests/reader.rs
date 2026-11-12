//! End-to-end tests of the reader: packages are built in memory so each test
//! states exactly the markup it depends on, plus two real-world fixtures.

use std::io::{Cursor, Write};

use oco_docx::{Block, BreakKind, Docx, Error, FontRef, RevisionKind, RunContent, RunProps};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const PKG_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const DOC_RELS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn zip(parts: &[(&str, String)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, content) in parts {
        writer.start_file(*name, options).unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// A package holding a document with the given body and style definitions.
fn docx(body: &str, styles: &str) -> Docx {
    let bytes = zip(&[
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="{PKG_RELS}"><Relationship Id="rId1" Type="{DOC_RELS}/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<Relationships xmlns="{PKG_RELS}"><Relationship Id="rId1" Type="{DOC_RELS}/styles" Target="styles.xml"/></Relationships>"#
            ),
        ),
        (
            "word/document.xml",
            format!("<w:document {W}><w:body>{body}</w:body></w:document>"),
        ),
        (
            "word/styles.xml",
            format!("<w:styles {W}>{styles}</w:styles>"),
        ),
    ]);
    Docx::from_bytes(&bytes).unwrap()
}

/// Effective properties of every run, in document order.
fn resolved_runs(docx: &Docx) -> Vec<RunProps> {
    let resolver = docx.resolver();
    docx.document
        .paragraphs()
        .into_iter()
        .flat_map(|p| {
            p.runs
                .iter()
                .map(|r| resolver.run(p.props.style_id.as_deref(), &r.props))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn name(font: &str) -> Option<FontRef> {
    Some(FontRef::Name(font.to_owned()))
}

#[test]
fn layers_defaults_style_chain_and_direct_formatting() {
    let styles = r#"
        <w:docDefaults>
          <w:rPrDefault><w:rPr><w:sz w:val="21"/><w:rFonts w:ascii="Liberation Serif" w:eastAsia="Source Han Serif SC"/></w:rPr></w:rPrDefault>
          <w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault>
        </w:docDefaults>
        <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
        <w:style w:type="paragraph" w:styleId="Base">
          <w:basedOn w:val="Normal"/>
          <w:pPr><w:jc w:val="center"/><w:spacing w:before="240"/></w:pPr>
          <w:rPr><w:i/><w:sz w:val="24"/><w:rFonts w:eastAsia="Source Han Sans SC"/></w:rPr>
        </w:style>
        <w:style w:type="paragraph" w:styleId="Derived">
          <w:basedOn w:val="Base"/>
          <w:pPr><w:spacing w:after="0"/></w:pPr>
          <w:rPr><w:sz w:val="32"/></w:rPr>
        </w:style>"#;
    let body = r#"
        <w:p><w:pPr><w:pStyle w:val="Derived"/><w:jc w:val="right"/></w:pPr>
          <w:r><w:t>styled</w:t></w:r>
          <w:r><w:rPr><w:sz w:val="40"/><w:i w:val="0"/></w:rPr><w:t>direct</w:t></w:r>
        </w:p>
        <w:p><w:r><w:t>plain</w:t></w:r></w:p>"#;
    let docx = docx(body, styles);
    let resolver = docx.resolver();
    let paragraphs = docx.document.paragraphs();

    // Spacing merges attribute by attribute across levels; alignment is replaced.
    let para = resolver.paragraph(&paragraphs[0].props);
    assert_eq!(para.style_id.as_deref(), Some("Derived"));
    assert_eq!(para.justification.as_deref(), Some("right"));
    assert_eq!(
        (para.spacing.before, para.spacing.after, para.spacing.line),
        (Some(240), Some(0), Some(259))
    );

    // A paragraph naming no style uses the default paragraph style.
    let plain = resolver.paragraph(&paragraphs[1].props);
    assert_eq!(plain.style_id.as_deref(), Some("Normal"));
    assert_eq!(
        (plain.justification, plain.spacing.after),
        (None, Some(160))
    );

    let runs = resolved_runs(&docx);
    assert_eq!((runs[0].size, runs[0].italic), (Some(32), Some(true)));
    assert_eq!(runs[0].fonts.ascii, name("Liberation Serif"));
    assert_eq!(runs[0].fonts.east_asia, name("Source Han Sans SC"));
    assert_eq!((runs[1].size, runs[1].italic), (Some(40), Some(false)));
    assert_eq!((runs[2].size, runs[2].italic), (Some(21), None));
}

#[test]
fn toggle_properties_flip_between_paragraph_and_character_styles() {
    let styles = r#"
        <w:style w:type="paragraph" w:styleId="BoldPara"><w:rPr><w:b/></w:rPr></w:style>
        <w:style w:type="paragraph" w:styleId="PlainPara"/>
        <w:style w:type="character" w:styleId="BoldBase"><w:rPr><w:b/></w:rPr></w:style>
        <w:style w:type="character" w:styleId="BoldChar"><w:basedOn w:val="BoldBase"/><w:rPr><w:b/></w:rPr></w:style>"#;
    let run = |rpr: &str| format!("<w:r><w:rPr>{rpr}</w:rPr><w:t>x</w:t></w:r>");
    let char_style = r#"<w:rStyle w:val="BoldChar"/>"#;
    let body = format!(
        r#"<w:p><w:pPr><w:pStyle w:val="BoldPara"/></w:pPr>{}{}{}{}</w:p>
           <w:p><w:pPr><w:pStyle w:val="PlainPara"/></w:pPr>{}{}</w:p>"#,
        run(""),
        run(char_style),
        run(&format!("{char_style}<w:b/>")),
        run(r#"<w:b w:val="0"/>"#),
        run(char_style),
        run(""),
    );
    let bold: Vec<_> = resolved_runs(&docx(&body, styles))
        .into_iter()
        .map(|r| r.bold)
        .collect();
    assert_eq!(
        bold,
        [
            Some(true),  // bold paragraph style
            Some(false), // ... toggled off by a bold character style
            Some(true),  // direct formatting is absolute
            Some(false), // direct formatting is absolute
            Some(true),  // a derived style overrides its base; it does not toggle it
            None,        // nothing specifies it
        ]
    );
}

#[test]
fn toggle_property_on_in_document_defaults_stays_on() {
    let styles = r#"
        <w:docDefaults><w:rPrDefault><w:rPr><w:b/></w:rPr></w:rPrDefault></w:docDefaults>
        <w:style w:type="paragraph" w:styleId="BoldPara"><w:rPr><w:b/></w:rPr></w:style>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="BoldPara"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#;
    assert_eq!(resolved_runs(&docx(body, styles))[0].bold, Some(true));
}

#[test]
fn first_line_and_hanging_indents_replace_each_other() {
    let styles = r#"<w:style w:type="paragraph" w:styleId="List"><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:style>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="List"/><w:ind w:firstLine="420" w:firstLineChars="200"/></w:pPr></w:p>"#;
    let docx = docx(body, styles);
    let indent = docx
        .resolver()
        .paragraph(&docx.document.paragraphs()[0].props)
        .indent;
    assert_eq!(
        (indent.left, indent.hanging, indent.first_line),
        (Some(720), None, Some(420))
    );
    assert_eq!(indent.first_line_chars, Some(200));
}

#[test]
fn based_on_cycle_does_not_hang() {
    let styles = r#"
        <w:style w:type="paragraph" w:styleId="A"><w:basedOn w:val="B"/><w:rPr><w:sz w:val="20"/></w:rPr></w:style>
        <w:style w:type="paragraph" w:styleId="B"><w:basedOn w:val="A"/><w:rPr><w:i/></w:rPr></w:style>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="A"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#;
    let run = &resolved_runs(&docx(body, styles))[0];
    assert_eq!((run.size, run.italic), (Some(20), Some(true)));
}

#[test]
fn reads_tracked_changes() {
    let body = r#"
        <w:p>
          <w:r><w:t xml:space="preserve">keep </w:t></w:r>
          <w:ins w:id="1" w:author="Ada" w:date="2026-01-02T03:04:05Z"><w:r><w:t>added</w:t></w:r></w:ins>
          <w:del w:id="2" w:author="Bob"><w:r><w:delText>removed</w:delText></w:r></w:del>
          <w:moveFrom w:id="3" w:author="Ada"><w:r><w:t>moved</w:t></w:r></w:moveFrom>
          <w:moveTo w:id="4" w:author="Ada"><w:r><w:t>!</w:t></w:r></w:moveTo>
        </w:p>"#;
    let docx = docx(body, "");
    let paragraph = docx.document.paragraphs()[0];
    let kinds: Vec<_> = paragraph
        .runs
        .iter()
        .map(|r| r.revision.as_ref().map(|rev| rev.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            None,
            Some(RevisionKind::Inserted),
            Some(RevisionKind::Deleted),
            Some(RevisionKind::MovedFrom),
            Some(RevisionKind::MovedTo)
        ]
    );
    let inserted = paragraph.runs[1].revision.as_ref().unwrap();
    assert_eq!(inserted.author.as_deref(), Some("Ada"));
    assert_eq!(inserted.date.as_deref(), Some("2026-01-02T03:04:05Z"));
    assert_eq!(paragraph.runs[2].text(), "removed");
    assert_eq!(paragraph.text(), "keep added!");
}

#[test]
fn reads_run_content_and_whitespace() {
    let body = r#"
        <w:p><w:r>
          <w:t> trimmed </w:t><w:t xml:space="preserve"> kept </w:t><w:tab/><w:t>a</w:t>
          <w:br/><w:br w:type="page"/><w:noBreakHyphen/>
        </w:r></w:p>"#;
    let docx = docx(body, "");
    assert_eq!(
        docx.document.paragraphs()[0].runs[0].content,
        [
            RunContent::Text("trimmed kept ".into()),
            RunContent::Tab,
            RunContent::Text("a".into()),
            RunContent::Break(BreakKind::Line),
            RunContent::Break(BreakKind::Page),
            RunContent::Text("\u{2011}".into()),
        ]
    );
}

#[test]
fn descends_into_wrappers_and_reports_what_it_skips() {
    let mc = r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#;
    let body = format!(
        r#"<w:sdt><w:sdtContent>
             <w:p>
               <w:bookmarkStart w:id="0" w:name="b"/>
               <w:hyperlink w:anchor="b"><w:r><w:t>link</w:t></w:r></w:hyperlink>
               <w:sdt><w:sdtContent><w:r><w:t>-sdt</w:t></w:r></w:sdtContent></w:sdt>
               <mc:AlternateContent {mc}>
                 <mc:Choice Requires="w14"><w:r><w:t>-choice</w:t></w:r></mc:Choice>
                 <mc:Fallback><w:r><w:t>-fallback</w:t></w:r></mc:Fallback>
               </mc:AlternateContent>
               <w:r><w:drawing/><w:t>-pic</w:t></w:r>
               <w:bookmarkEnd w:id="0"/>
             </w:p>
           </w:sdtContent></w:sdt>
           <w:tbl><w:tblPr/><w:tr><w:tc><w:tcPr/><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
           <w:sectPr/>"#
    );
    let docx = docx(&body, "");
    let texts: Vec<_> = docx
        .document
        .paragraphs()
        .iter()
        .map(|p| p.text())
        .collect();
    assert_eq!(texts, ["link-sdt-fallback-pic", "cell"]);
    assert!(
        matches!(&docx.document.body[1], Block::Table(t) if t.rows.len() == 1 && t.rows[0].cells.len() == 1)
    );

    let skipped: Vec<_> = docx.coverage.ranked().into_iter().collect();
    assert_eq!(
        skipped,
        [
            ("w:r/w:drawing", 1),
            ("w:sectPr", 1),
            ("w:tbl/w:tblPr", 1),
            ("w:tc/w:tcPr", 1)
        ]
    );
}

#[test]
fn reads_iso_strict_documents() {
    let strict = r#"xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main""#;
    let strict_rels = "http://purl.oclc.org/ooxml/officeDocument/relationships";
    let bytes = zip(&[
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="{PKG_RELS}"><Relationship Id="rId1" Type="{strict_rels}/officeDocument" Target="/word/doc.xml"/></Relationships>"#
            ),
        ),
        (
            "word/doc.xml",
            format!(
                r#"<w:document {strict}><w:body><w:p><w:r><w:rPr><w:b w:val="true"/></w:rPr><w:t>strict</w:t></w:r></w:p></w:body></w:document>"#
            ),
        ),
    ]);
    let docx = Docx::from_bytes(&bytes).unwrap();
    assert_eq!(docx.document.paragraphs()[0].text(), "strict");
    assert_eq!(resolved_runs(&docx)[0].bold, Some(true));
}

#[test]
fn rejects_files_that_are_not_documents() {
    assert!(matches!(
        Docx::from_bytes(b"plain text"),
        Err(Error::Zip(_))
    ));
    let no_document = zip(&[("unrelated.txt", "hello".to_owned())]);
    assert!(matches!(
        Docx::from_bytes(&no_document),
        Err(Error::NoMainDocument)
    ));
    let broken = zip(&[("word/document.xml", "<w:document".to_owned())]);
    assert!(
        matches!(Docx::from_bytes(&broken), Err(Error::Xml { part, .. }) if part == "word/document.xml")
    );
}

// ------------------------------------------------------------------ fixtures
// Regenerate with `python3 tools/gen_fixtures.py`.

fn fixture(name: &str) -> Docx {
    Docx::open(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn word_template_fixture() {
    let docx = fixture("word-template.docx");
    let resolver = docx.resolver();
    let paragraphs = docx.document.paragraphs();
    let run = |p: usize, r: usize| {
        resolver.run(
            paragraphs[p].props.style_id.as_deref(),
            &paragraphs[p].runs[r].props,
        )
    };

    // Heading 1 takes its fonts from the theme's major (heading) slots; the
    // East Asian one is picked by the document's `zh-CN` theme language.
    let heading = resolver.paragraph(&paragraphs[0].props);
    assert_eq!(
        (heading.style_id.as_deref(), heading.outline_level),
        (Some("Heading1"), Some(0))
    );
    assert_eq!(
        docx.styles.get("Heading1").unwrap().name.as_deref(),
        Some("heading 1")
    );
    let title = run(0, 0);
    assert_eq!(
        (title.size, title.bold, title.color.as_deref()),
        (Some(28), Some(true), Some("365F91"))
    );
    assert_eq!(
        (title.fonts.ascii, title.fonts.east_asia),
        (name("Calibri"), name("宋体"))
    );
    // The "Strong" character style is bold, and so is Heading 1: they cancel.
    assert_eq!(run(0, 1).bold, Some(false));

    // Body text falls back to document defaults and the minor (body) fonts.
    let body = run(1, 0);
    assert_eq!(
        (body.size, body.bold, body.fonts.ascii),
        (Some(22), None, name("Cambria"))
    );
    assert_eq!(run(1, 1).bold, Some(true));
    assert_eq!(run(1, 2).italic, Some(true));
    assert_eq!(
        (run(1, 3).size, run(1, 3).color.as_deref()),
        (Some(32), Some("C00000"))
    );

    assert_eq!(
        resolver
            .paragraph(&paragraphs[2].props)
            .indent
            .first_line_chars,
        Some(200)
    );

    assert!(
        matches!(&docx.document.body[3], Block::Table(t) if t.rows.len() == 2 && t.rows[1].cells.len() == 2)
    );
    assert_eq!(paragraphs[6].text(), "单元格 1,1");

    let tracked = paragraphs[7];
    assert_eq!(tracked.text(), "修订：保留新增");
    assert_eq!(
        tracked.runs[2].revision.as_ref().unwrap().author.as_deref(),
        Some("张三")
    );
}

#[test]
fn libreoffice_fixture() {
    let docx = fixture("libreoffice.docx");
    let resolver = docx.resolver();
    let paragraphs = docx.document.paragraphs();
    assert_eq!(paragraphs[0].text(), "LibreOffice 导出");
    assert_eq!(
        resolver.paragraph(&paragraphs[0].props).style_id.as_deref(),
        Some("Heading1")
    );
    assert_eq!(
        resolver
            .paragraph(&paragraphs[2].props)
            .justification
            .as_deref(),
        Some("center")
    );

    let flagged = |get: fn(&RunProps) -> bool| -> String {
        paragraphs[1]
            .runs
            .iter()
            .filter(|r| get(&resolver.run(paragraphs[1].props.style_id.as_deref(), &r.props)))
            .map(|r| r.text())
            .collect()
    };
    assert_eq!(flagged(|p| p.bold == Some(true)), "粗体");
    assert_eq!(flagged(|p| p.italic == Some(true)), "斜体");
    assert_eq!(
        flagged(|p| p.underline.as_deref() == Some("single")),
        "下划线"
    );

    let cells: Vec<_> = paragraphs[3..7].iter().map(|p| p.text()).collect();
    assert_eq!(cells, ["甲", "乙", "丙", "丁"]);
}
