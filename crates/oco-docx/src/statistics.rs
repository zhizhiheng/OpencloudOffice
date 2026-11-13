//! Document statistics from the extended properties part (`docProps/app.xml`).

#[cfg(feature = "serde")]
use serde::Serialize;

/// What the application that last saved the document recorded about it.
///
/// For a document saved by Word these are Word's own figures for its own
/// layout, so they are ground truth for pagination and line breaking that
/// needs no copy of Word to obtain. They describe the document as it was
/// when the statistics were last updated, so check `characters` against the
/// text actually present before trusting the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Statistics {
    pub application: Option<String>,
    pub pages: Option<u32>,
    pub words: Option<u32>,
    /// Characters, not counting spaces.
    pub characters: Option<u32>,
    pub characters_with_spaces: Option<u32>,
    pub lines: Option<u32>,
    pub paragraphs: Option<u32>,
}

impl Statistics {
    pub(crate) fn parse(xml: &str) -> Result<Self, crate::xmltree::Error> {
        let doc = crate::xmltree::Document::parse(xml)?;
        let mut statistics = Statistics::default();
        for node in doc.root_element().children().filter(|n| n.is_element()) {
            let text = node.text().unwrap_or_default().trim();
            let number = || text.parse::<u32>().ok();
            match node.tag_name().name() {
                "Application" => statistics.application = Some(text.to_owned()),
                "Pages" => statistics.pages = number(),
                "Words" => statistics.words = number(),
                "Characters" => statistics.characters = number(),
                "CharactersWithSpaces" => statistics.characters_with_spaces = number(),
                "Lines" => statistics.lines = number(),
                "Paragraphs" => statistics.paragraphs = number(),
                _ => {}
            }
        }
        Ok(statistics)
    }

    /// Whether Word produced these figures.
    pub fn from_word(&self) -> bool {
        self.application
            .as_deref()
            .is_some_and(|name| name.starts_with("Microsoft Office Word") || name.starts_with("Microsoft Word"))
    }
}
