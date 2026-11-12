//! End-to-end tests of the reader: packages are built in memory so each test
//! states exactly the markup it depends on, plus two real-world fixtures.

use std::io::{Cursor, Write};

use oco_docx::{
    Block, BreakKind, Color, Docx, Error, FontRef, Highlight, OtherFormat, ParaProps, RevisionKind, RunContent,
    RunProps, Statistics, Underline, VertAlign, VerticalMerge,
};
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
        ("word/document.xml", format!("<w:document {W}><w:body>{body}</w:body></w:document>")),
        ("word/styles.xml", format!("<w:styles {W}>{styles}</w:styles>")),
    ]);
    Docx::from_bytes(&bytes).unwrap()
}

/// Effective properties of every run, in document order.
fn resolved_runs(docx: &Docx) -> Vec<RunProps> {
    let resolver = docx.resolver();
    docx.document
        .paragraphs()
        .into_iter()
        .flat_map(|p| p.runs.iter().map(|r| resolver.run(p.props.style_id.as_deref(), &r.props)).collect::<Vec<_>>())
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
    assert_eq!((para.spacing.before, para.spacing.after, para.spacing.line), (Some(240), Some(0), Some(259)));

    // A paragraph naming no style uses the default paragraph style.
    let plain = resolver.paragraph(&paragraphs[1].props);
    assert_eq!(plain.style_id.as_deref(), Some("Normal"));
    assert_eq!((plain.justification, plain.spacing.after), (None, Some(160)));

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
    let bold: Vec<_> = resolved_runs(&docx(&body, styles)).into_iter().map(|r| r.bold).collect();
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
fn document_default_toggles_apply_only_when_no_style_speaks() {
    // Read off Word's own PDF of these cases. As we read the standard, the
    // second and third would stay bold.
    let styles = r#"
        <w:docDefaults><w:rPrDefault><w:rPr><w:b/></w:rPr></w:rPrDefault></w:docDefaults>
        <w:style w:type="paragraph" w:default="1" w:styleId="Normal"/>
        <w:style w:type="paragraph" w:styleId="Bold"><w:rPr><w:b/></w:rPr></w:style>
        <w:style w:type="paragraph" w:styleId="Unbold"><w:rPr><w:b w:val="0"/></w:rPr></w:style>
        <w:style w:type="character" w:styleId="Colour"><w:rPr><w:color w:val="FF0000"/></w:rPr></w:style>"#;
    let paragraph = |style: &str, rpr: &str| {
        format!(r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:rPr>{rpr}</w:rPr><w:t>x</w:t></w:r></w:p>"#)
    };
    let body = [
        paragraph("Normal", ""),
        paragraph("Unbold", ""),
        paragraph("Unbold", r#"<w:rStyle w:val="Colour"/>"#),
        paragraph("Bold", ""),
    ]
    .concat();
    let bold: Vec<_> = resolved_runs(&docx(&body, styles)).into_iter().map(|r| r.bold).collect();
    assert_eq!(bold, [Some(true), Some(false), Some(false), Some(true)]);
}

#[test]
fn first_line_and_hanging_indents_replace_each_other() {
    let styles = r#"<w:style w:type="paragraph" w:styleId="List"><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:style>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="List"/><w:ind w:firstLine="420" w:firstLineChars="200"/></w:pPr></w:p>"#;
    let docx = docx(body, styles);
    let indent = docx.resolver().paragraph(&docx.document.paragraphs()[0].props).indent;
    assert_eq!((indent.left, indent.hanging, indent.first_line), (Some(720), None, Some(420)));
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
    let kinds: Vec<_> = paragraph.runs.iter().map(|r| r.revision.as_ref().map(|rev| rev.kind)).collect();
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
fn keeps_words_record_of_where_pages_began() {
    let body = r#"
        <w:p>
          <w:r><w:t>one</w:t></w:r>
          <w:r><w:lastRenderedPageBreak/><w:t>two</w:t></w:r>
        </w:p>"#;
    let docx = docx(body, "");
    let paragraph = docx.document.paragraphs()[0];
    assert_eq!(paragraph.runs[1].content, [RunContent::PageBreakHint, RunContent::Text("two".into())]);
    // It marks a place and is not content.
    assert_eq!(paragraph.text(), "onetwo");
    assert!(docx.coverage.unsupported.is_empty(), "{:?}", docx.coverage.unsupported);
}

#[test]
fn reads_the_statistics_the_saving_application_recorded() {
    let package = |app: Option<&str>| {
        let mut parts = vec![
            (
                "_rels/.rels",
                format!(
                    r#"<Relationships xmlns="{PKG_RELS}"><Relationship Id="rId1" Type="{DOC_RELS}/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="{DOC_RELS}/extended-properties" Target="docProps/app.xml"/></Relationships>"#
                ),
            ),
            ("word/document.xml", format!("<w:document {W}><w:body><w:p/></w:body></w:document>")),
        ];
        if let Some(app) = app {
            parts.push((
                "docProps/app.xml",
                format!(
                    r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">{app}</Properties>"#
                ),
            ));
        }
        Docx::from_bytes(&zip(&parts)).unwrap()
    };

    let word = package(Some(
        "<Application>Microsoft Office Word</Application><Pages>3</Pages><Words>410</Words>\
         <Characters>2337</Characters><Lines>19</Lines><Paragraphs>5</Paragraphs>\
         <CharactersWithSpaces>2742</CharactersWithSpaces><AppVersion>16.0000</AppVersion>",
    ));
    assert!(word.recorded.from_word());
    assert_eq!(word.recorded.pages, Some(3));
    assert_eq!(word.recorded.words, Some(410));
    assert_eq!(word.recorded.characters, Some(2337));
    assert_eq!(word.recorded.characters_with_spaces, Some(2742));
    assert_eq!(word.recorded.lines, Some(19));
    assert_eq!(word.recorded.paragraphs, Some(5));

    // Other applications write the same part; their figures are not Word's.
    let other = package(Some("<Application>LibreOffice/24.2</Application><Pages>3</Pages>"));
    assert_eq!(other.recorded.pages, Some(3));
    assert!(!other.recorded.from_word());

    // The part is optional, and a damaged one costs nothing but the figures.
    for docx in [package(None), package(Some("<Pages>3"))] {
        assert_eq!(docx.recorded, Statistics::default());
        assert!(docx.warnings.is_empty(), "{:?}", docx.warnings);
    }
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
    let texts: Vec<_> = docx.document.paragraphs().iter().map(|p| p.text()).collect();
    assert_eq!(texts, ["link-sdt-fallback-pic", "cell"]);
    assert!(matches!(&docx.document.body[1], Block::Table(t) if t.rows.len() == 1 && t.rows[0].cells.len() == 1));

    let skipped: Vec<_> = docx.coverage.ranked().into_iter().collect();
    assert_eq!(skipped, [("w:r/w:drawing", 1)]);
}

#[test]
fn reads_what_decides_where_a_tables_text_goes() {
    let body = r#"
        <w:tbl>
          <w:tblPr>
            <w:tblStyle w:val="Grid"/><w:tblW w:w="5000" w:type="dxa"/><w:tblInd w:w="108" w:type="dxa"/>
            <w:jc w:val="center"/><w:tblLayout w:type="fixed"/>
            <w:tblCellMar><w:left w:w="60" w:type="dxa"/><w:right w:w="70" w:type="dxa"/></w:tblCellMar>
            <w:tblBorders><w:top w:val="single" w:sz="4"/><w:insideH w:val="single" w:sz="12"/><w:left w:val="nil" w:sz="96"/></w:tblBorders>
            <w:tblLook w:val="04A0"/><w:tblOverlap w:val="never"/>
            <w:tblpPr w:vertAnchor="page" w:horzAnchor="page" w:tblpX="1362" w:tblpY="1135"/>
          </w:tblPr>
          <w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="3000"/></w:tblGrid>
          <w:tr>
            <w:trPr><w:trHeight w:val="400" w:hRule="exact"/><w:cantSplit/><w:tblHeader/></w:trPr>
            <w:tc><w:tcPr><w:tcW w:w="5000" w:type="dxa"/><w:gridSpan w:val="2"/><w:vMerge w:val="restart"/>
              <w:tcMar><w:top w:w="20" w:type="dxa"/></w:tcMar></w:tcPr><w:p/></w:tc>
          </w:tr>
          <w:tr>
            <w:tc><w:tcPr><w:tcW w:w="50" w:type="pct"/><w:vMerge/></w:tcPr><w:p/></w:tc>
            <w:tc><w:p/></w:tc>
          </w:tr>
        </w:tbl>"#;
    let docx = docx(body, "");
    let Block::Table(table) = &docx.document.body[0] else { panic!("a table") };
    assert_eq!(table.grid, [2000, 3000]);
    let props = &table.props;
    assert_eq!((props.style_id.as_deref(), props.width, props.indent), (Some("Grid"), Some(5000), Some(108)));
    assert_eq!((props.align.as_deref(), props.fixed_layout), (Some("center"), Some(true)));
    assert_eq!((props.cell_margins.left, props.cell_margins.right, props.cell_margins.top), (Some(60), Some(70), None));
    // The widest border that is drawn, in eighths of a point.
    assert_eq!(props.borders, Some(12));
    assert!(props.floating);

    let (first, second) = (&table.rows[0], &table.rows[1]);
    assert_eq!((first.props.height, first.props.exact_height), (Some(400), true));
    assert!(first.props.cant_split && first.props.header);
    assert_eq!((second.props.height, second.props.cant_split), (None, false));
    let spanning = &first.cells[0].props;
    assert_eq!((spanning.width, spanning.grid_span, spanning.merge), (Some(5000), 2, Some(VerticalMerge::Restart)));
    assert_eq!(spanning.margins.top, Some(20));
    // A width that is a percentage is not a length; a bare vMerge continues.
    assert_eq!((second.cells[0].props.width, second.cells[0].props.merge), (None, Some(VerticalMerge::Continue)));
    assert_eq!((second.cells[1].props.grid_span, second.cells[1].props.merge), (1, None));
    // What is not read is said; so is where the table floats, which is not.
    assert_eq!(
        docx.coverage.ranked().into_iter().collect::<Vec<_>>(),
        [("w:tblPr/w:tblOverlap", 1), ("w:tblPr/w:tblpPr", 1)]
    );
}

#[test]
fn reads_what_an_auto_fit_table_is_fitted_by() {
    let body = r#"
        <w:tbl>
          <w:tblPr><w:tblW w:w="2500" w:type="pct"/></w:tblPr>
          <w:tblGrid><w:gridCol w:w="1000"/><w:gridCol w:w="2000"/><w:gridCol w:w="3000"/></w:tblGrid>
          <w:tr>
            <w:trPr><w:gridBefore w:val="1"/><w:wBefore w:w="900" w:type="dxa"/><w:gridAfter w:val="1"/></w:trPr>
            <w:tc><w:tcPr><w:tcW w:w="40%" w:type="pct"/></w:tcPr>
              <w:p><w:r><w:t>text</w:t></w:r><w:r><w:drawing/></w:r></w:p></w:tc>
          </w:tr>
          <w:tr>
            <w:tc><w:tcPr><w:tcW w:w="1500" w:type="pct"/></w:tcPr><w:p><w:r><w:t>text</w:t></w:r></w:p></w:tc>
            <w:tc><w:tcPr><w:tcW w:w="0" w:type="auto"/></w:tcPr><w:p/></w:tc>
          </w:tr>
        </w:tbl>"#;
    let docx = docx(body, "");
    let Block::Table(table) = &docx.document.body[0] else { panic!("a table") };
    // A width in fiftieths of a percent is a share of the room, not a length.
    assert_eq!((table.props.width, table.props.width_share), (None, Some(2500)));
    let (first, second) = (&table.rows[0], &table.rows[1]);
    // The row leaves out a column at either end, and says how wide the first is.
    assert_eq!((first.props.grid_before, first.props.grid_after), (1, 1));
    assert_eq!((first.props.width_before, first.props.width_after), (Some(900), None));
    // A percentage may be written with its sign.
    assert_eq!((first.cells[0].props.width, first.cells[0].props.width_share), (None, Some(2000)));
    assert_eq!(second.cells[0].props.width_share, Some(1500));
    assert_eq!((second.cells[1].props.width, second.cells[1].props.width_share), (None, None));
    // A picture is passed over, and the cell says that not all of it was read.
    assert!(first.cells[0].unmeasured && !second.cells[0].unmeasured);
    assert_eq!(docx.coverage.ranked().into_iter().collect::<Vec<_>>(), [("w:r/w:drawing", 1)]);
}

#[test]
fn normal_table_is_built_in_and_table_styles_format_paragraphs() {
    // As measured in Word (docx4j's probe `styles-table-default`): the style
    // named Normal Table has Word's own cell margins whatever the file gives
    // it; a style that does not lead back to it has none.
    let styles = r#"
        <w:docDefaults><w:pPrDefault><w:pPr><w:spacing w:after="200" w:line="276"/></w:pPr></w:pPrDefault></w:docDefaults>
        <w:style w:type="paragraph" w:default="1" w:styleId="Normal"/>
        <w:style w:type="paragraph" w:styleId="Tight"><w:pPr><w:spacing w:after="40"/></w:pPr></w:style>
        <w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/>
          <w:tblPr><w:tblCellMar><w:left w:w="300" w:type="dxa"/><w:right w:w="300" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>
        <w:style w:type="table" w:styleId="Grid"><w:basedOn w:val="TableNormal"/>
          <w:pPr><w:spacing w:after="0" w:line="240"/></w:pPr>
          <w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4"/></w:tblBorders></w:tblPr></w:style>
        <w:style w:type="table" w:styleId="Alone"><w:tblPr><w:tblInd w:w="0" w:type="dxa"/></w:tblPr></w:style>"#;
    let table = |props: &str| format!("<w:tbl><w:tblPr>{props}</w:tblPr><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>");
    let body = [
        table(""),
        table(r#"<w:tblStyle w:val="Grid"/>"#),
        table(r#"<w:tblStyle w:val="Alone"/>"#),
        table(r#"<w:tblStyle w:val="Grid"/><w:tblCellMar><w:left w:w="20" w:type="dxa"/></w:tblCellMar>"#),
    ]
    .concat();
    let docx = docx(&body, styles);
    let resolver = docx.resolver();
    let resolved: Vec<_> = docx
        .document
        .body
        .iter()
        .map(|block| match block {
            Block::Table(table) => resolver.table(&table.props),
            Block::Paragraph(_) => panic!("only tables"),
        })
        .collect();
    let margins: Vec<_> = resolved.iter().map(|t| (t.cell_margins.left, t.cell_margins.right)).collect();
    assert_eq!(margins, [(Some(108), Some(108)), (Some(108), Some(108)), (Some(0), Some(0)), (Some(20), Some(108))]);
    assert_eq!(resolved.iter().map(|t| t.borders).collect::<Vec<_>>(), [None, Some(4), None, Some(4)]);

    // A table style's paragraph formatting sits between the document
    // defaults and the paragraph's own style.
    let in_grid = resolver.table_styles(Some("Grid"));
    let plain = resolver.paragraph_in(&ParaProps::default(), &in_grid);
    assert_eq!((plain.spacing.after, plain.spacing.line), (Some(0), Some(240)));
    let tight = ParaProps { style_id: Some("Tight".into()), ..ParaProps::default() };
    let styled = resolver.paragraph_in(&tight, &in_grid);
    assert_eq!((styled.spacing.after, styled.spacing.line), (Some(40), Some(240)));
    let outside = resolver.paragraph(&ParaProps::default());
    assert_eq!((outside.spacing.after, outside.spacing.line), (Some(200), Some(276)));
}

#[test]
fn what_the_file_gives_normal_table_is_not_passed_on_to_text() {
    // As measured in Word (docx4j's probes `tables-normal-table-text` and
    // `tables-named-normal-table`): the paragraph and character formatting
    // of the style named Normal Table reaches no text, whether the style is
    // the default or not; that of a default table style by another name does.
    let styles = |default_name: &str, other_name: &str| {
        let format = r#"<w:pPr><w:spacing w:after="600"/></w:pPr><w:rPr><w:sz w:val="40"/></w:rPr>"#;
        format!(
            r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"/>
               <w:style w:type="table" w:default="1" w:styleId="A"><w:name w:val="{default_name}"/>{format}</w:style>
               <w:style w:type="table" w:styleId="B"><w:name w:val="{other_name}"/>{format}</w:style>
               <w:style w:type="table" w:styleId="C"><w:name w:val="Based"/><w:basedOn w:val="A"/></w:style>"#
        )
    };
    let got = |styles: &str, table: Option<&str>| {
        let docx = docx("", styles);
        let resolver = docx.resolver();
        let chain = resolver.table_styles(table);
        let paragraph = resolver.paragraph_in(&ParaProps::default(), &chain);
        let run = resolver.run_in(None, &RunProps::default(), &chain);
        (paragraph.spacing.after, run.size)
    };
    let (built_in, renamed) = (styles("Normal Table", "Other"), styles("My Default", "Normal Table"));
    // No style named, one based on the default, and the default itself.
    for table in [None, Some("C"), Some("A")] {
        assert_eq!(got(&built_in, table), (None, None), "{table:?}");
        assert_eq!(got(&renamed, table), (Some(600), Some(40)), "{table:?}");
    }
    assert_eq!(got(&built_in, Some("B")), (Some(600), Some(40)));
    assert_eq!(got(&renamed, Some("B")), (None, None));
}

#[test]
fn page_number_fields_are_told_from_the_results_stored_for_them() {
    let field = |kind: &str| format!(r#"<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>"#);
    let code = |text: &str| format!(r#"<w:r><w:instrText xml:space="preserve">{text}</w:instrText></w:r>"#);
    let text = |text: &str| format!(r#"<w:r><w:t xml:space="preserve">{text}</w:t></w:r>"#);
    let body = [
        "<w:p>".to_owned(),
        text("Page "),
        // A complex field, its instruction in two pieces and its result in two runs.
        field("begin")
            + &code(" PA")
            + &code("GE  \\* MERGEFORMAT ")
            + &field("separate")
            + &text("1")
            + &text("2")
            + &field("end"),
        text(" of "),
        // The same thing written in one piece.
        r#"<w:fldSimple w:instr=" NUMPAGES "><w:r><w:rPr><w:b/></w:rPr><w:t>40</w:t></w:r></w:fldSimple>"#.to_owned(),
        text(", "),
        // Any other field shows the result stored for it.
        field("begin") + &code(" DATE ") + &field("separate") + &text("today") + &field("end"),
        // A page number inside another field is that field's business.
        field("begin")
            + &code(" IF ")
            + &field("begin")
            + &code(" PAGE ")
            + &field("separate")
            + &text("7")
            + &field("end")
            + &code(" > 1 yes no ")
            + &field("separate")
            + &text(" yes")
            + &field("end"),
        // A field with no result stored at all.
        field("begin") + &code("PAGE") + &field("end"),
        "</w:p>".to_owned(),
    ]
    .concat();
    let docx = docx(&body, "");
    let paragraph = docx.document.paragraphs()[0];
    let content: Vec<_> = paragraph.runs.iter().flat_map(|run| run.content.iter().cloned()).collect();
    assert_eq!(
        content,
        [
            RunContent::Text("Page ".into()),
            RunContent::PageNumber("12".into()),
            RunContent::Text(" of ".into()),
            RunContent::PageCount("40".into()),
            RunContent::Text(", ".into()),
            RunContent::Text("today".into()),
            RunContent::Text(" yes".into()),
            RunContent::PageNumber(String::new()),
        ]
    );
    // A reader that does not lay the document out shows what Word last showed.
    assert_eq!(paragraph.text(), "Page 12 of 40, today yes");
    // The formatting of the result stays with the number.
    let count = paragraph.runs.iter().find(|run| matches!(run.content[..], [RunContent::PageCount(_)])).unwrap();
    assert_eq!(count.props.bold, Some(true));
    assert!(docx.coverage.unsupported.is_empty(), "{:?}", docx.coverage.unsupported);
}

#[test]
fn reads_headers_and_footers_and_which_pages_they_are_for() {
    let relationship = |id: &str, kind: &str, target: &str| {
        format!(r#"<Relationship Id="{id}" Type="{DOC_RELS}/{kind}" Target="{target}"/>"#)
    };
    let r = r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    let body = r#"<w:p><w:pPr><w:sectPr>
             <w:headerReference w:type="default" r:id="rId1"/><w:footerReference w:type="even" r:id="rId3"/>
             <w:pgMar w:top="-1440" w:bottom="1134" w:left="1440" w:right="1440" w:header="567" w:footer="850"/>
           </w:sectPr></w:pPr></w:p>
           <w:sectPr>
             <w:headerReference w:type="first" r:id="rId2"/><w:footerReference r:id="rId3"/>
             <w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440" w:header="567" w:footer="850"/>
             <w:titlePg/><w:pgNumType w:start="5" w:fmt="lowerRoman"/>
           </w:sectPr>"#;
    let bytes = zip(&[
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<Relationships xmlns="{PKG_RELS}">{}{}{}{}{}</Relationships>"#,
                relationship("rId1", "header", "header1.xml"),
                relationship("rId2", "header", "header2.xml"),
                relationship("rId3", "footer", "footer1.xml"),
                relationship("rId4", "settings", "settings.xml"),
                relationship("rId5", "header", "gone.xml"),
            ),
        ),
        ("word/document.xml", format!("<w:document {W} {r}><w:body>{body}</w:body></w:document>")),
        ("word/header1.xml", format!("<w:hdr {W}><w:p><w:r><w:t>running head</w:t></w:r></w:p></w:hdr>")),
        ("word/header2.xml", format!("<w:hdr {W}><w:p><w:r><w:t>title page</w:t></w:r></w:p></w:hdr>")),
        (
            "word/footer1.xml",
            format!(
                r#"<w:ftr {W}><w:p><w:fldSimple w:instr="PAGE"><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:ftr>"#
            ),
        ),
        ("word/settings.xml", format!("<w:settings {W}><w:evenAndOddHeaders/></w:settings>")),
    ]);
    let docx = Docx::from_bytes(&bytes).unwrap();
    let story = |id: &str| {
        docx.stories[id]
            .iter()
            .map(|b| match b {
                Block::Paragraph(p) => p.text(),
                Block::Table(_) => "table".to_owned(),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(docx.stories.keys().collect::<Vec<_>>(), ["rId1", "rId2", "rId3"]);
    assert_eq!(
        (story("rId1"), story("rId2"), story("rId3")),
        (vec!["running head".to_owned()], vec!["title page".to_owned()], vec!["1".to_owned()])
    );
    // The part that is not there costs a warning, not the document.
    assert_eq!(docx.warnings.len(), 1, "{:?}", docx.warnings);
    assert!(docx.settings.even_and_odd_headers);

    let first = docx.document.paragraphs()[0].section_break.as_ref().unwrap();
    assert_eq!((first.headers.default.as_deref(), first.headers.first.as_deref()), (Some("rId1"), None));
    assert_eq!((first.footers.even.as_deref(), first.footers.default.as_deref()), (Some("rId3"), None));
    assert!(!first.title_page);
    // A margin written as a negative number is that far all the same, and
    // is not to give way to a header that reaches further.
    assert_eq!((first.margin_top, first.margin_top_fixed), (1440, true));
    assert_eq!((first.margin_bottom, first.margin_bottom_fixed), (1134, false));
    let last = &docx.document.section;
    assert_eq!((last.headers.first.as_deref(), last.footers.default.as_deref()), (Some("rId2"), Some("rId3")));
    assert_eq!((last.header, last.footer, last.title_page), (567, 850, true));
    assert_eq!((last.margin_top, last.margin_top_fixed), (1440, false));
    assert_eq!((last.page_number_start, last.page_number_format.as_deref()), (Some(5), Some("lowerRoman")));
    assert!(docx.coverage.unsupported.is_empty(), "{:?}", docx.coverage.unsupported);
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
    assert!(matches!(Docx::from_bytes(b"plain text"), Err(Error::Zip(_))));
    let no_document = zip(&[("unrelated.txt", "hello".to_owned())]);
    assert!(matches!(Docx::from_bytes(&no_document), Err(Error::NoMainDocument)));
    let broken = zip(&[("word/document.xml", "<w:document".to_owned())]);
    assert!(matches!(Docx::from_bytes(&broken), Err(Error::Xml { part, .. }) if part == "word/document.xml"));
}

// ------------------------------------------------------ how Word reads things
// Each test below pins a behaviour of Word 365 that differs from what the
// schema alone suggests. Sources are listed in docs/word-behaviour.md.

/// One run per given `w:rPr` content, each in a paragraph of its own that
/// uses paragraph style `Base`.
fn runs_over_base_style(styles: &str, run_props: &[&str]) -> Vec<RunProps> {
    let body: String = run_props
        .iter()
        .map(|rpr| {
            format!(r#"<w:p><w:pPr><w:pStyle w:val="Base"/></w:pPr><w:r><w:rPr>{rpr}</w:rPr><w:t>x</w:t></w:r></w:p>"#)
        })
        .collect();
    resolved_runs(&docx(&body, styles))
}

#[test]
fn invalid_property_values_leave_the_style_value_in_place() {
    let styles = r#"<w:style w:type="paragraph" w:styleId="Base"><w:rPr>
        <w:color w:val="00FF00"/><w:u w:val="double"/><w:highlight w:val="yellow"/><w:vertAlign w:val="superscript"/>
    </w:rPr></w:style>"#;
    let green = Some(Color::Rgb(0, 0xFF, 0));

    let colors: Vec<_> = runs_over_base_style(
        styles,
        &[
            r#"<w:color w:val="ff0000"/>"#,
            r##"<w:color w:val="#FF0000"/>"##,
            r#"<w:color w:val=" FF0000 "/>"#,
            r#"<w:color w:val="auto"/>"#,
            r#"<w:color w:val="Auto"/>"#,
            r#"<w:color w:val=" auto"/>"#,
            r#"<w:color w:val="F00"/>"#,
            r#"<w:color w:val="0xFF0000"/>"#,
            r#"<w:color/>"#,
        ],
    )
    .into_iter()
    .map(|r| r.color)
    .collect();
    let red = Some(Color::Rgb(0xFF, 0, 0));
    assert_eq!(colors, [red, red, red, Some(Color::Auto), green, green, green, green, green]);

    let underlines: Vec<_> = runs_over_base_style(
        styles,
        &[
            r#"<w:u w:val="single"/>"#,
            r#"<w:u w:val="none"/>"#,
            // Not in the enumeration, or in the wrong case: as if absent.
            r#"<w:u w:val="Single"/>"#,
            r#"<w:u w:val="bogus"/>"#,
            r#"<w:u w:val=""/>"#,
            r#"<w:u/>"#,
            r#"<w:u w:color="FF0000"/>"#,
            // The on/off words, even the "on" ones, mean no underline.
            r#"<w:u w:val="true"/>"#,
            r#"<w:u w:val="off"/>"#,
            r#"<w:u w:val="True"/>"#,
        ],
    )
    .into_iter()
    .map(|r| r.underline)
    .collect();
    let (double, none) = (Some(Underline::Double), Some(Underline::None));
    assert_eq!(underlines, [Some(Underline::Single), none, double, double, double, double, double, none, none, double]);

    let others = runs_over_base_style(
        styles,
        &[
            r#"<w:highlight w:val="YELLOW"/><w:vertAlign w:val="Superscript"/>"#,
            r#"<w:highlight w:val="green"/><w:vertAlign w:val="baseline"/>"#,
        ],
    );
    assert_eq!((others[0].highlight, others[0].vert_align), (Some(Highlight::Yellow), Some(VertAlign::Superscript)));
    assert_eq!((others[1].highlight, others[1].vert_align), (Some(Highlight::Green), Some(VertAlign::Baseline)));
}

#[test]
fn on_off_is_off_only_for_three_exact_spellings() {
    let run = |value: &str| format!(r#"<w:r><w:rPr><w:b w:val="{value}"/></w:rPr><w:t>x</w:t></w:r>"#);
    let values = ["false", "off", "0", "true", "on", "1", "", "FALSE", "Off", " false", "no"];
    let body = format!("<w:p>{}</w:p>", values.iter().map(|v| run(v)).collect::<String>());
    let bold: Vec<_> = resolved_runs(&docx(&body, "")).into_iter().map(|r| r.bold == Some(true)).collect();
    assert_eq!(bold, [false, false, false, true, true, true, true, true, true, true, true]);
}

#[test]
fn a_character_style_contributes_no_highlight() {
    let styles = r#"
        <w:style w:type="character" w:styleId="Marked"><w:rPr><w:b/><w:highlight w:val="yellow"/></w:rPr></w:style>
        <w:style w:type="paragraph" w:styleId="Noted"><w:rPr><w:highlight w:val="cyan"/></w:rPr></w:style>"#;
    let body = r#"
        <w:p><w:r><w:rPr><w:rStyle w:val="Marked"/></w:rPr><w:t>a</w:t></w:r></w:p>
        <w:p><w:pPr><w:pStyle w:val="Noted"/></w:pPr><w:r><w:t>b</w:t></w:r></w:p>
        <w:p><w:r><w:rPr><w:rStyle w:val="Marked"/><w:highlight w:val="green"/></w:rPr><w:t>c</w:t></w:r></w:p>"#;
    let runs = resolved_runs(&docx(body, styles));
    assert_eq!((runs[0].bold, runs[0].highlight), (Some(true), None));
    assert_eq!(runs[1].highlight, Some(Highlight::Cyan));
    assert_eq!(runs[2].highlight, Some(Highlight::Green));
}

#[test]
fn reads_lengths_written_with_units() {
    // ISO Strict documents saved by Word write "36pt" where Transitional has 720.
    let body = r#"<w:p>
        <w:pPr><w:ind w:start="36pt" w:hanging="0.25in"/><w:spacing w:before="10pt" w:after="2.54cm" w:line="276"/></w:pPr>
        <w:r><w:rPr><w:sz w:val="12pt"/></w:rPr><w:t>x</w:t></w:r>
    </w:p>"#;
    let docx = docx(body, "");
    let props = &docx.document.paragraphs()[0].props;
    assert_eq!((props.indent.left, props.indent.hanging), (Some(720), Some(360)));
    assert_eq!((props.spacing.before, props.spacing.after, props.spacing.line), (Some(200), Some(1440), Some(276)));
    assert_eq!(resolved_runs(&docx)[0].size, Some(24));
}

#[test]
fn a_line_height_without_a_rule_resets_the_rule() {
    let styles = r#"<w:style w:type="paragraph" w:styleId="Exact"><w:pPr><w:spacing w:line="480" w:lineRule="exact"/></w:pPr></w:style>"#;
    let paragraph = |spacing: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="Exact"/>{spacing}</w:pPr></w:p>"#);
    let body = [paragraph(r#"<w:spacing w:after="0"/>"#), paragraph(r#"<w:spacing w:line="240"/>"#)].concat();
    let docx = docx(&body, styles);
    let spacing: Vec<_> = docx
        .document
        .paragraphs()
        .iter()
        .map(|p| docx.resolver().paragraph(&p.props).spacing)
        .map(|s| (s.line, s.line_rule))
        .collect();
    // Stating only `after` keeps the style's exact line height; stating a
    // line height alone makes it a multiple of single spacing again.
    assert_eq!(spacing, [(Some(480), Some("exact".to_owned())), (Some(240), None)]);
}

#[test]
fn reads_integers_as_word_does() {
    let ind = |value: &str| format!(r#"<w:p><w:pPr><w:ind w:left="{value}"/></w:pPr></w:p>"#);
    let values = ["240", "+240", "-240", "120.5", "-121.5", "120.", "", "-", ".5", "1e3", "0x10", "12 0"];
    let body: String = values.iter().map(|v| ind(v)).collect();
    let left: Vec<_> = docx(&body, "").document.paragraphs().iter().map(|p| p.props.indent.left).collect();
    assert_eq!(
        left,
        [
            Some(240),
            Some(240),
            Some(-240),
            Some(120),
            Some(-121),
            Some(120),
            Some(0),
            Some(0),
            Some(0),
            None,
            None,
            None
        ]
    );
}

#[test]
fn reads_automatic_paragraph_spacing() {
    let body =
        r#"<w:p><w:pPr><w:spacing w:before="100" w:beforeAutospacing="1" w:afterAutospacing="0"/></w:pPr></w:p>"#;
    let spacing = &docx(body, "").document.paragraphs()[0].props.spacing.clone();
    assert_eq!((spacing.before, spacing.before_auto, spacing.after_auto), (Some(100), Some(true), Some(false)));
}

#[test]
fn content_controls_can_wrap_rows_and_cells() {
    // Word stores a form field that fills a table cell as a control around the cell.
    let cell = |text: &str| format!("<w:tc><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>");
    let body = format!(
        r#"<w:tbl>
             <w:tr>{}<w:sdt><w:sdtPr/><w:sdtContent>{}</w:sdtContent></w:sdt>{}</w:tr>
             <w:sdt><w:sdtContent><w:tr>{}</w:tr></w:sdtContent></w:sdt>
             <w:customXml><w:tr><w:customXml>{}</w:customXml></w:tr></w:customXml>
           </w:tbl>"#,
        cell("a"),
        cell("b"),
        cell("c"),
        cell("d"),
        cell("e"),
    );
    let docx = docx(&body, "");
    let Block::Table(table) = &docx.document.body[0] else { panic!("expected a table") };
    let shape: Vec<_> = table.rows.iter().map(|r| r.cells.len()).collect();
    assert_eq!(shape, [3, 1, 1]);
    let texts: Vec<_> = docx.document.paragraphs().iter().map(|p| p.text()).collect();
    assert_eq!(texts, ["a", "b", "c", "d", "e"]);
    assert!(docx.coverage.unsupported.is_empty(), "{:?}", docx.coverage.unsupported);
}

#[test]
fn takes_the_first_compatibility_choice_word_understands() {
    let ns = r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"
                xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"
                xmlns:future="urn:example:not-invented-yet""#;
    let run = |text: &str| format!("<w:r><w:t>{text}</w:t></w:r>");
    let body = format!(
        r#"<w:p>
             <mc:AlternateContent {ns}>
               <mc:Choice Requires="w14">{}</mc:Choice><mc:Fallback>{}</mc:Fallback>
             </mc:AlternateContent>
             <mc:AlternateContent {ns}>
               <mc:Choice Requires="future">{}</mc:Choice>
               <mc:Choice Requires="w14">{}</mc:Choice>
               <mc:Fallback>{}</mc:Fallback>
             </mc:AlternateContent>
             <mc:AlternateContent {ns}>
               <mc:Choice Requires="w14 future">{}</mc:Choice><mc:Fallback>{}</mc:Fallback>
             </mc:AlternateContent>
             <mc:AlternateContent {ns}><mc:Choice Requires="future">{}</mc:Choice></mc:AlternateContent>
           </w:p>
           <mc:AlternateContent {ns}>
             <mc:Choice Requires="w14"><w:p>{}</w:p></mc:Choice><mc:Fallback><w:p>{}</w:p></mc:Fallback>
           </mc:AlternateContent>"#,
        run("[new]"),
        run("[old]"),
        run("[unknown]"),
        run("[second]"),
        run("[old]"),
        run("[partly unknown]"),
        run("[fallback]"),
        run("[nothing]"),
        run("block choice"),
        run("block fallback"),
    );
    let texts: Vec<_> = docx(&body, "").document.paragraphs().iter().map(|p| p.text()).collect();
    assert_eq!(texts, ["[new][second][fallback]", "block choice"]);
}

#[test]
fn reads_through_unknown_elements_of_the_main_namespace() {
    let body = r#"
        <w:p><w:r><w:t>a</w:t></w:r><w:madeUp><w:r><w:t>b</w:t></w:r><w:other><w:r><w:t>c</w:t></w:r></w:other></w:madeUp></w:p>
        <w:madeUp><w:p><w:r><w:t>d</w:t></w:r></w:p></w:madeUp>
        <w:tbl><w:madeUp><w:tr><w:madeUp><w:tc><w:p><w:r><w:madeUp><w:t>e</w:t></w:madeUp></w:r></w:p></w:tc></w:madeUp></w:tr></w:madeUp></w:tbl>
        <w:p><w:emptyMadeUp/><x:foreign xmlns:x="urn:example"><w:r><w:t>hidden</w:t></w:r></x:foreign></w:p>"#;
    let docx = docx(body, "");
    let texts: Vec<_> = docx.document.paragraphs().iter().map(|p| p.text()).collect();
    assert_eq!(texts, ["abc", "d", "e", ""]);
    // What held no content is still reported; what was read through is not.
    assert_eq!(docx.coverage.ranked(), [("w:p/w:emptyMadeUp", 1), ("w:p/{urn:example}foreign", 1)]);
}

#[test]
fn merges_what_should_appear_once_but_appears_twice() {
    let bytes = zip(&[(
        "word/document.xml",
        format!(
            r#"<w:document {W}>
                 <w:body><w:p>
                   <w:pPr><w:jc w:val="center"/></w:pPr><w:pPr><w:keepNext/></w:pPr>
                   <w:r><w:rPr><w:b/></w:rPr><w:rPr><w:i/></w:rPr><w:t>first</w:t></w:r>
                   <w:sdt><w:sdtContent><w:r><w:t>+one</w:t></w:r></w:sdtContent><w:sdtContent><w:r><w:t>+two</w:t></w:r></w:sdtContent></w:sdt>
                 </w:p></w:body>
                 <w:body><w:p><w:r><w:t>second body</w:t></w:r></w:p></w:body>
               </w:document>"#
        ),
    )]);
    let docx = Docx::from_bytes(&bytes).unwrap();
    let paragraphs = docx.document.paragraphs();
    assert_eq!(paragraphs[0].text(), "first+one+two");
    assert_eq!(paragraphs[1].text(), "second body");
    assert_eq!(
        (paragraphs[0].props.justification.as_deref(), paragraphs[0].props.keep_next),
        (Some("center"), Some(true))
    );
    let run = &paragraphs[0].runs[0].props;
    assert_eq!((run.bold, run.italic), (Some(true), Some(true)));
}

#[test]
fn reads_run_content_placed_outside_a_run() {
    let body = r#"<w:p><w:r><w:t>before</w:t></w:r><w:br/><w:hyperlink><w:tab/><w:r><w:t>after</w:t></w:r></w:hyperlink></w:p>"#;
    assert_eq!(docx(body, "").document.paragraphs()[0].text(), "before\n\tafter");
}

#[test]
fn reads_breaks_and_text_nested_inside_text() {
    let body = r#"<w:p><w:r><w:t xml:space="preserve">a <w:br/>b<w:t>c</w:t><w:tab/><w:madeUp>dropped</w:madeUp></w:t></w:r></w:p>"#;
    let docx = docx(body, "");
    assert_eq!(
        docx.document.paragraphs()[0].runs[0].content,
        [RunContent::Text("a ".into()), RunContent::Break(BreakKind::Line), RunContent::Text("bc".into())]
    );
}

#[test]
fn reads_colour_names() {
    let run = |value: &str| format!(r#"<w:r><w:rPr><w:color w:val="{value}"/></w:rPr><w:t>x</w:t></w:r>"#);
    let values = ["red", "DarkRed", " ORANGE ", "lightgrey", "lightgray", "grey", "dark red"];
    let body = format!("<w:p>{}</w:p>", values.iter().map(|v| run(v)).collect::<String>());
    let colors: Vec<_> = resolved_runs(&docx(&body, "")).into_iter().map(|r| r.color.map(|c| c.to_string())).collect();
    let hex = |value: &str| Some(value.to_owned());
    assert_eq!(colors, [hex("FF0000"), hex("8B0000"), hex("FFA500"), hex("D3D3D3"), None, None, None]);
}

#[test]
fn reads_main_parts_with_an_unusual_root() {
    let mc = r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;
    let paragraph = |text: &str| format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>");
    let read = |root: String| {
        let docx = Docx::from_bytes(&zip(&[("word/document.xml", root)])).unwrap();
        docx.document.paragraphs().iter().map(|p| p.text()).collect::<Vec<_>>().join("|")
    };
    // Compatibility markup around the document element, with two documents in the branch taken.
    let choice = format!("<w:document><w:body>{}</w:body></w:document>", paragraph("new"));
    let second = format!("<w:document><w:body>{}</w:body></w:document>", paragraph("more"));
    let fallback = format!("<w:document><w:body>{}</w:body></w:document>", paragraph("old"));
    assert_eq!(
        read(format!(
            r#"<mc:AlternateContent {W} {mc}><mc:Choice Requires="w14">{choice}{second}</mc:Choice><mc:Fallback>{fallback}</mc:Fallback></mc:AlternateContent>"#
        )),
        "new|more"
    );
    // A body, a paragraph, or an unknown element standing in for the document element.
    assert_eq!(read(format!("<w:body {W}>{}{}</w:body>", paragraph("a"), paragraph("b"))), "a|b");
    assert_eq!(read(format!("<w:p {W}><w:r><w:t>alone</w:t></w:r></w:p>")), "alone");
    assert_eq!(
        read(format!("<w:madeUp {W}><w:body>{}</w:body>{}</w:madeUp>", paragraph("in"), paragraph("out"))),
        "in|out"
    );
    // A body inside a body reads through.
    assert_eq!(
        read(format!("<w:document {W}><w:body><w:body>{}</w:body></w:body></w:document>", paragraph("nested"))),
        "nested"
    );
}

#[test]
fn reads_symbol_characters() {
    let body = r#"<w:p><w:r><w:t>tick </w:t><w:sym w:font="Wingdings" w:char="F0FC"/><w:sym w:char="zz"/></w:r></w:p>"#;
    let docx = docx(body, "");
    let run = &docx.document.paragraphs()[0].runs[0];
    assert_eq!(run.content[1], RunContent::Symbol { font: Some("Wingdings".into()), character: '\u{F0FC}' });
    assert_eq!(run.text(), "tick\u{F0FC}");
    assert_eq!(docx.coverage.ranked(), [("w:r/w:sym", 1)]);
}

#[test]
fn the_last_main_document_relationship_wins() {
    let part =
        |text: &str| format!("<w:document {W}><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>");
    let bytes = zip(&[
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="{PKG_RELS}">
                     <Relationship Id="rId1" Type="{DOC_RELS}/officeDocument" Target="word/document.xml"/>
                     <Relationship Id="rId2" Type="{DOC_RELS}/officeDocument" Target="word/other.xml"/>
                   </Relationships>"#
            ),
        ),
        ("word/document.xml", part("first")),
        ("word/other.xml", part("last")),
    ]);
    assert_eq!(Docx::from_bytes(&bytes).unwrap().document.paragraphs()[0].text(), "last");
}

// --------------------------------------------------- numbering and page setup

/// A package with a numbering part beside the document and styles.
fn numbered(body: &str, styles: &str, numbering: &str) -> Docx {
    let relationship = |id: u8, kind: &str, target: &str| {
        format!(r#"<Relationship Id="rId{id}" Type="{DOC_RELS}/{kind}" Target="{target}"/>"#)
    };
    let bytes = zip(&[
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<Relationships xmlns="{PKG_RELS}">{}{}</Relationships>"#,
                relationship(1, "styles", "styles.xml"),
                relationship(2, "numbering", "numbering.xml")
            ),
        ),
        ("word/document.xml", format!("<w:document {W}><w:body>{body}</w:body></w:document>")),
        ("word/styles.xml", format!("<w:styles {W}>{styles}</w:styles>")),
        ("word/numbering.xml", format!("<w:numbering {W}>{numbering}</w:numbering>")),
    ]);
    Docx::from_bytes(&bytes).unwrap()
}

/// The label of every paragraph, in order; `-` for an unnumbered one.
fn labels(docx: &Docx) -> Vec<String> {
    let resolver = docx.resolver();
    let mut counters = oco_docx::Counters::default();
    docx.document
        .paragraphs()
        .iter()
        .map(|p| {
            let props = resolver.paragraph(&p.props);
            resolver
                .list_level(&props)
                .and_then(|(num_id, ilvl, _)| counters.next(&docx.numbering, num_id, ilvl))
                .map_or("-".to_owned(), |label| label.text)
        })
        .collect()
}

fn item(num_id: i32, ilvl: i32) -> String {
    format!(r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="{ilvl}"/><w:numId w:val="{num_id}"/></w:numPr></w:pPr></w:p>"#)
}

fn level(ilvl: u8, format: &str, text: &str, extra: &str) -> String {
    format!(
        r#"<w:lvl w:ilvl="{ilvl}"><w:start w:val="1"/><w:numFmt w:val="{format}"/>{extra}<w:lvlText w:val="{text}"/></w:lvl>"#
    )
}

#[test]
fn lists_on_one_definition_share_a_count_until_restarted() {
    // As measured in Word: every instance of a definition continues the same
    // count; a start override takes effect once, when its instance is first used.
    let numbering = format!(
        r#"<w:abstractNum w:abstractNumId="0">{}</w:abstractNum>
           <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
           <w:num w:numId="2"><w:abstractNumId w:val="0"/></w:num>
           <w:num w:numId="3"><w:abstractNumId w:val="0"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="1"/></w:lvlOverride></w:num>"#,
        level(0, "decimal", "（%1）", "")
    );
    let body = [item(1, 0), item(1, 0), item(2, 0), item(1, 0), item(3, 0), item(3, 0), item(2, 0)].concat();
    assert_eq!(
        labels(&numbered(&body, "", &numbering)),
        ["（1）", "（2）", "（3）", "（4）", "（1）", "（2）", "（3）"]
    );
}

#[test]
fn deeper_levels_restart_unless_told_otherwise() {
    // 第一章 / 第一条: articles keep counting through the chapters, because
    // their level never restarts (`lvlRestart` 0). The third level restarts
    // under each article as usual.
    let numbering = format!(
        r#"<w:abstractNum w:abstractNumId="0">{}{}{}</w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#,
        level(0, "chineseCounting", "第%1章", ""),
        level(1, "chineseCounting", "第%2条", r#"<w:lvlRestart w:val="0"/>"#),
        level(2, "chineseCounting", "（%3）", ""),
    );
    let body: String = [0, 1, 1, 2, 2, 0, 1, 2, 1].iter().map(|ilvl| item(1, *ilvl)).collect();
    assert_eq!(
        labels(&numbered(&body, "", &numbering)),
        ["第一章", "第一条", "第二条", "（一）", "（二）", "第二章", "第三条", "（一）", "第四条"]
    );
}

#[test]
fn legal_numbering_prints_inherited_numbers_as_decimals() {
    let numbering = format!(
        r#"<w:abstractNum w:abstractNumId="0">{}{}{}</w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#,
        level(0, "upperRoman", "%1", ""),
        level(1, "decimal", "%1.%2", "<w:isLgl/>"),
        level(2, "lowerLetter", "%1.%2.%3", ""),
    );
    let body: String = [0, 0, 1, 2, 1].iter().map(|ilvl| item(1, *ilvl)).collect();
    assert_eq!(labels(&numbered(&body, "", &numbering)), ["I", "II", "2.1", "II.1.a", "2.2"]);
}

#[test]
fn a_numbered_heading_takes_its_level_and_indent_from_the_list() {
    let numbering = format!(
        r#"<w:abstractNum w:abstractNumId="0">{}{}</w:abstractNum><w:num w:numId="5"><w:abstractNumId w:val="0"/></w:num>"#,
        r#"<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:pStyle w:val="H1"/><w:lvlText w:val="%1"/><w:pPr><w:ind w:left="432" w:hanging="432"/></w:pPr></w:lvl>"#,
        r#"<w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:pStyle w:val="H2"/><w:lvlText w:val="%1.%2"/><w:pPr><w:ind w:left="576" w:hanging="576"/></w:pPr></w:lvl>"#,
    );
    // Both styles name the list, as Word writes numbered headings: the
    // first level goes unsaid, any other is stated.
    let styles = r#"
        <w:style w:type="paragraph" w:default="1" w:styleId="Normal"/>
        <w:style w:type="paragraph" w:styleId="H1"><w:pPr><w:numPr><w:numId w:val="5"/></w:numPr><w:ind w:left="100"/></w:pPr></w:style>
        <w:style w:type="paragraph" w:styleId="H2"><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="5"/></w:numPr></w:pPr></w:style>"#;
    let heading = |style: &str, extra: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="{style}"/>{extra}</w:pPr></w:p>"#);
    let body = [
        heading("H1", ""),
        heading("H2", ""),
        heading("H2", r#"<w:ind w:left="720"/>"#),
        "<w:p/>".to_owned(),
        // `numId` 0 takes a paragraph out of its style's list.
        heading("H1", r#"<w:numPr><w:numId w:val="0"/></w:numPr>"#),
        heading("H1", ""),
        heading("H2", ""),
    ]
    .concat();
    let docx = numbered(&body, styles, &numbering);
    assert_eq!(labels(&docx), ["1", "1.1", "1.2", "-", "-", "2", "2.1"]);

    let resolver = docx.resolver();
    let indent = |index: usize| {
        let indent = resolver.paragraph(&docx.document.paragraphs()[index].props).indent;
        (indent.left, indent.hanging)
    };
    // The level's indent beats the style's; direct formatting beats both.
    assert_eq!(indent(0), (Some(432), Some(432)));
    assert_eq!(indent(1), (Some(576), Some(576)));
    assert_eq!(indent(2), (Some(720), Some(576)));
    assert_eq!(indent(4), (None, None));
}

#[test]
fn a_list_level_numbers_the_paragraphs_of_one_style_only() {
    // As measured in Word (docx4j's probes `numbering-linked-level-other-style`,
    // `numbering-unlinked-level-claim` and `numbering-shared-abstract-styles`).
    let numbering = format!(
        r#"<w:abstractNum w:abstractNumId="0">{}{}{}</w:abstractNum>
           <w:num w:numId="100"><w:abstractNumId w:val="0"/></w:num>
           <w:num w:numId="101"><w:abstractNumId w:val="0"/></w:num>"#,
        level(0, "decimal", "A%1", r#"<w:pStyle w:val="Owner"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr>"#),
        level(1, "decimal", "B%2", ""),
        level(2, "decimal", "C%3", r#"<w:pStyle w:val="NamedByTwo"/>"#),
    );
    let style = |id: &str, based_on: &str, num: &str| {
        format!(
            r#"<w:style w:type="paragraph" w:styleId="{id}"><w:basedOn w:val="{based_on}"/><w:pPr>{num}</w:pPr></w:style>"#
        )
    };
    let list = |num_id: u8| format!(r#"<w:numPr><w:numId w:val="{num_id}"/></w:numPr>"#);
    let at_one = r#"<w:numPr><w:ilvl w:val="1"/><w:numId w:val="100"/></w:numPr>"#;
    let styles = [
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"/>"#.to_owned(),
        style("Owner", "Normal", &list(100)),
        style("Other", "Normal", &list(100)),
        style("Derived", "Owner", ""),
        style("OtherInstance", "Normal", &list(101)),
        // Level 2 names this style, but its own numPr reads the first level.
        style("NamedByTwo", "Normal", &list(100)),
        // No style is named by level 1: the first to claim it in this part owns it.
        style("FirstAtOne", "Normal", at_one),
        style("SecondAtOne", "Normal", at_one),
    ]
    .concat();
    let of = |style: &str, extra: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="{style}"/>{extra}</w:pPr></w:p>"#);
    let direct = r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="100"/></w:numPr>"#;
    let cases = [
        (of("Other", ""), "-"),
        (of("Owner", ""), "A1"),
        (of("Derived", ""), "A2"),
        (of("OtherInstance", ""), "-"),
        (of("NamedByTwo", ""), "-"),
        (of("SecondAtOne", ""), "-"),
        (of("FirstAtOne", ""), "B1"),
        // Numbering on the paragraph itself always numbers it.
        (of("Other", direct), "A3"),
        (of("Owner", ""), "A4"),
    ];
    let body: String = cases.iter().map(|(paragraph, _)| paragraph.as_str()).collect();
    let docx = numbered(&body, &styles, &numbering);
    assert_eq!(labels(&docx), cases.map(|(_, label)| label));

    // A paragraph the level does not number still takes the level's indents.
    let other = docx.resolver().paragraph(&docx.document.paragraphs()[0].props);
    assert_eq!((other.numbering, other.indent.left, other.indent.hanging), (None, Some(720), Some(360)));
}

#[test]
fn theme_fonts_fall_back_to_the_office_theme_when_there_is_no_theme_part() {
    // As measured in Word (docx4j's probe `theme-fonts-no-theme-part`).
    let run = |fonts: &str| format!("<w:r><w:rPr><w:rFonts {fonts}/></w:rPr><w:t>x</w:t></w:r>");
    let body = format!(
        "<w:p>{}{}{}</w:p>",
        run(r#"w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi""#),
        run(r#"w:asciiTheme="majorHAnsi" w:hAnsiTheme="majorAscii""#),
        run(r#"w:eastAsiaTheme="minorEastAsia" w:ascii="Arial""#),
    );
    let runs = resolved_runs(&docx(&body, ""));
    assert_eq!((runs[0].fonts.ascii.clone(), runs[0].fonts.h_ansi.clone()), (name("Calibri"), name("Calibri")));
    assert_eq!((runs[1].fonts.ascii.clone(), runs[1].fonts.h_ansi.clone()), (name("Cambria"), name("Cambria")));
    // Only the Latin slots: the East Asian one depends on the language.
    assert_eq!(runs[2].fonts.east_asia, Some(FontRef::Theme("minorEastAsia".into())));
    assert_eq!(runs[2].fonts.ascii, name("Arial"));
}

#[test]
fn reads_page_setup_and_section_breaks() {
    let body = r#"
        <w:p><w:pPr><w:sectPr><w:pgSz w:w="16838" w:h="11906"/><w:type w:val="continuous"/></w:sectPr></w:pPr></w:p>
        <w:p/>
        <w:sectPr>
          <w:pgSz w:w="11906" w:h="16838"/>
          <w:pgMar w:top="2098" w:right="1474" w:bottom="1984" w:left="1588" w:header="851" w:footer="992" w:gutter="0"/>
          <w:docGrid w:type="lines" w:linePitch="312"/>
        </w:sectPr>"#;
    let docx = docx(body, "");
    let last = &docx.document.section;
    assert_eq!((last.page_width, last.page_height), (11906, 16838));
    assert_eq!((last.margin_top, last.margin_right, last.margin_bottom, last.margin_left), (2098, 1474, 1984, 1588));
    assert_eq!((last.grid.kind, last.grid.line_pitch), (oco_docx::GridKind::Lines, 312));
    assert_eq!(last.text_width(), 11906 - 1588 - 1474);

    let first = docx.document.paragraphs()[0].section_break.as_ref().expect("the first paragraph ends a section");
    assert_eq!((first.page_width, first.start), (16838, oco_docx::SectionStart::Continuous));
    assert_eq!(first.grid.kind, oco_docx::GridKind::None);
    assert!(docx.document.paragraphs()[1].section_break.is_none());
    assert!(docx.coverage.unsupported.is_empty(), "{:?}", docx.coverage.unsupported);
}

#[test]
fn tab_stops_accumulate_through_styles_and_can_be_cleared() {
    let styles = r#"<w:style w:type="paragraph" w:styleId="Tabbed"><w:pPr><w:tabs>
        <w:tab w:val="left" w:pos="2000"/><w:tab w:val="right" w:pos="8000" w:leader="dot"/></w:tabs></w:pPr></w:style>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="Tabbed"/><w:tabs>
        <w:tab w:val="clear" w:pos="2000"/><w:tab w:val="center" w:pos="4000"/></w:tabs></w:pPr></w:p>"#;
    let docx = docx(body, styles);
    let tabs = docx.resolver().paragraph(&docx.document.paragraphs()[0].props).tabs;
    let summary: Vec<_> = tabs.iter().map(|t| (t.position, t.kind, t.leader.as_deref())).collect();
    assert_eq!(summary, [(4000, oco_docx::TabKind::Center, None), (8000, oco_docx::TabKind::Right, Some("dot"))]);
}

#[test]
fn reads_character_effects() {
    let body = r#"<w:p><w:r><w:rPr>
        <w:dstrike/><w:em w:val="dot"/><w:spacing w:val="40"/><w:w w:val="150"/><w:position w:val="6"/>
        <w:u w:val="wave" w:color="FF0000"/><w:shd w:val="clear" w:fill="D9D9D9"/><w:bdr w:val="single" w:sz="4"/>
        <w:szCs w:val="28"/><w:lang w:val="en-US"/><w:em w:val="bogus"/>
    </w:rPr><w:t>x</w:t></w:r></w:p>"#;
    let docx = docx(body, "");
    let run = &resolved_runs(&docx)[0];
    assert_eq!((run.double_strike, run.emphasis), (Some(true), Some(oco_docx::Emphasis::Dot)));
    assert_eq!((run.char_spacing, run.scale, run.position, run.size_cs), (Some(40), Some(150), Some(6), Some(28)));
    assert_eq!((run.underline, run.underline_color), (Some(Underline::Wave), Some(Color::Rgb(0xFF, 0, 0))));
    assert_eq!(run.shade, Some(Color::Rgb(0xD9, 0xD9, 0xD9)));
    assert!(run.border.as_ref().is_some_and(|b| b.is_visible()));
    assert!(docx.coverage.unsupported.is_empty(), "{:?}", docx.coverage.unsupported);
}

// ------------------------------------------------------------ damaged input

#[test]
fn opens_with_a_warning_when_an_optional_part_is_damaged() {
    let rels = |target_type: &str, target: &str| {
        format!(
            r#"<Relationships xmlns="{PKG_RELS}"><Relationship Id="rId1" Type="{DOC_RELS}/{target_type}" Target="{target}"/></Relationships>"#
        )
    };
    let bytes = zip(&[
        ("_rels/.rels", rels("officeDocument", "word/document.xml")),
        ("word/_rels/document.xml.rels", rels("styles", "styles.xml")),
        (
            "word/document.xml",
            format!(
                r#"<w:document {W}><w:body><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>still here</w:t></w:r></w:p></w:body></w:document>"#
            ),
        ),
        (
            "word/styles.xml",
            format!(r#"<w:styles {W}><w:style w:type="paragraph" w:styleId="Normal"><w:rPr><w:rPrChange"#),
        ),
    ]);
    let docx = Docx::from_bytes(&bytes).unwrap();
    assert_eq!(docx.document.paragraphs()[0].text(), "still here");
    assert_eq!(resolved_runs(&docx)[0].bold, Some(true));
    assert!(docx.styles.is_empty());
    assert_eq!(docx.warnings.len(), 1);
    assert!(docx.warnings[0].contains("styles"), "{}", docx.warnings[0]);
    // Nothing half-read from the damaged part is left in the report.
    assert!(docx.coverage.unsupported.is_empty());
}

#[test]
fn says_what_a_file_is_when_it_is_not_a_docx() {
    let ole = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0];
    assert!(matches!(Docx::from_bytes(&ole), Err(Error::OtherFormat(OtherFormat::OleCompoundFile))));
    let odt = zip(&[
        ("mimetype", "application/vnd.oasis.opendocument.text".to_owned()),
        ("content.xml", "<office:document-content/>".to_owned()),
    ]);
    assert!(matches!(Docx::from_bytes(&odt), Err(Error::OtherFormat(OtherFormat::OpenDocument))));
}

#[test]
fn reports_tables_nested_beyond_the_limit_instead_of_overflowing() {
    let depth = 200;
    let body = format!(
        "{}<w:p><w:r><w:t>deep</w:t></w:r></w:p>{}",
        "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>level</w:t></w:r></w:p>".repeat(depth),
        "</w:tc></w:tr></w:tbl>".repeat(depth)
    );
    let docx = docx(&body, "");
    assert_eq!(docx.document.paragraphs().len(), 64);
    assert_eq!(docx.coverage.unsupported.get("w:tbl (nested too deeply, dropped)"), Some(&1));
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
    let run = |p: usize, r: usize| resolver.run(paragraphs[p].props.style_id.as_deref(), &paragraphs[p].runs[r].props);

    // Heading 1 takes its fonts from the theme's major (heading) slots; the
    // East Asian one is picked by the document's `zh-CN` theme language.
    let heading = resolver.paragraph(&paragraphs[0].props);
    assert_eq!((heading.style_id.as_deref(), heading.outline_level), (Some("Heading1"), Some(0)));
    assert_eq!(docx.styles.get("Heading1").unwrap().name.as_deref(), Some("heading 1"));
    let title = run(0, 0);
    assert_eq!((title.size, title.bold, title.color), (Some(28), Some(true), Some(Color::Rgb(0x36, 0x5F, 0x91))));
    assert_eq!((title.fonts.ascii, title.fonts.east_asia), (name("Calibri"), name("宋体")));
    // The "Strong" character style is bold, and so is Heading 1: they cancel.
    assert_eq!(run(0, 1).bold, Some(false));

    // Body text falls back to document defaults and the minor (body) fonts.
    let body = run(1, 0);
    assert_eq!((body.size, body.bold, body.fonts.ascii), (Some(22), None, name("Cambria")));
    assert_eq!(run(1, 1).bold, Some(true));
    assert_eq!(run(1, 2).italic, Some(true));
    assert_eq!((run(1, 3).size, run(1, 3).color), (Some(32), Some(Color::Rgb(0xC0, 0, 0))));

    assert_eq!(resolver.paragraph(&paragraphs[2].props).indent.first_line_chars, Some(200));

    assert!(matches!(&docx.document.body[3], Block::Table(t) if t.rows.len() == 2 && t.rows[1].cells.len() == 2));
    assert_eq!(paragraphs[6].text(), "单元格 1,1");

    let tracked = paragraphs[7];
    assert_eq!(tracked.text(), "修订：保留新增");
    assert_eq!(tracked.runs[2].revision.as_ref().unwrap().author.as_deref(), Some("张三"));
}

#[test]
fn libreoffice_fixture() {
    let docx = fixture("libreoffice.docx");
    let resolver = docx.resolver();
    let paragraphs = docx.document.paragraphs();
    assert_eq!(paragraphs[0].text(), "LibreOffice 导出");
    assert_eq!(resolver.paragraph(&paragraphs[0].props).style_id.as_deref(), Some("Heading1"));
    assert_eq!(resolver.paragraph(&paragraphs[2].props).justification.as_deref(), Some("center"));

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
    assert_eq!(flagged(|p| p.underline == Some(Underline::Single)), "下划线");

    let cells: Vec<_> = paragraphs[3..7].iter().map(|p| p.text()).collect();
    assert_eq!(cells, ["甲", "乙", "丙", "丁"]);
}
