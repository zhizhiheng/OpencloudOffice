//! Markdown turned into the document form.
//!
//! A Markdown text becomes a WordprocessingML package, the same thing a
//! `.docx` file is, so that it is read, laid out and drawn by the code that
//! shows Word documents, and can be saved as a Word document.
//!
//! This is the skeleton: every line becomes a paragraph. The parser and the
//! styles are still to be written.

use oco_docx::opc::{content_type, relationship, Builder};
use oco_docx::xmltree::escape;

/// The images a Markdown text shows, by the address written for each, in
/// the order they first appear. A host that can fetch them does, and hands
/// their bytes to [`to_docx`].
pub fn images(_markdown: &str) -> Vec<String> {
    Vec::new()
}

/// Turns Markdown into a `.docx` package. `image` is asked for the bytes of
/// each image by its address as written; one it does not have is replaced
/// by its description.
pub fn to_docx(markdown: &str, _image: &dyn Fn(&str) -> Option<Vec<u8>>) -> Vec<u8> {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let mut body = String::new();
    for line in markdown.lines() {
        body.push_str(&format!(r#"<w:p><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#, escape(line)));
    }
    let document = format!(r#"<w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>"#);
    let mut package = Builder::new();
    package.part("word/document.xml", content_type::DOCUMENT, document.as_bytes());
    package.relate(None, relationship::DOCUMENT, "word/document.xml");
    package.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_package_reads_back_as_a_document() {
        let docx = oco_docx::Docx::from_bytes(&to_docx("first & <second>\nthird", &|_| None)).unwrap();
        let text: Vec<String> = docx.document.paragraphs().iter().map(|p| p.text()).collect();
        assert_eq!(text, ["first & <second>", "third"]);
    }
}
