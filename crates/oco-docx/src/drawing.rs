//! Drawings placed in the text (`w:drawing`): pictures, and the frame of
//! objects whose content is not read yet.
//!
//! Lengths are in EMU, the unit of DrawingML: 12700 to the point.

#[cfg(feature = "serde")]
use serde::Serialize;

use crate::xml::{elements, text_of};
use crate::xmltree::Node;

pub const EMU_PER_POINT: i64 = 12700;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Drawing {
    pub width: i64,
    pub height: i64,
    /// `None` for an object that stands in the line like a character.
    pub anchor: Option<Anchor>,
    /// The picture shown, when the object is one.
    pub picture: Option<Picture>,
    /// The words that stand for the object where it cannot be seen.
    pub description: Option<String>,
    /// The text box the object is, among those of its run
    /// ([`crate::Run::text_boxes`]).
    pub text_box: Option<usize>,
}

/// A shape with text in it.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct TextBox {
    pub blocks: Vec<crate::Block>,
    /// The room between the edge of the box and its text: left, top, right, bottom.
    pub insets: [i64; 4],
    /// Where the text stands in a box taller than it: at the top, in the middle or at the bottom.
    pub anchor: Alignment,
    /// The colour the box is filled with.
    pub fill: Option<[u8; 3]>,
    /// The colour and the width of the line around the box.
    pub outline: Option<([u8; 3], i64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Picture {
    /// The package part that holds the image, such as `word/media/image1.png`.
    /// `None` for an image that is linked rather than embedded, or missing.
    pub part: Option<String>,
    /// How much of the image is cut away at the left, top, right and bottom,
    /// in thousandths of a percent of its width or height.
    pub crop: [i32; 4],
}

/// How an object that is not in the line is placed, and how text meets it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Anchor {
    pub horizontal: Placement,
    pub vertical: Placement,
    pub wrap: Wrap,
    pub wrap_side: WrapSide,
    /// The room text keeps from the object at its top, bottom, left and right.
    pub distance: [i64; 4],
    /// Drawn under the text rather than over it. Only matters with [`Wrap::None`].
    pub behind_text: bool,
    /// In a table cell, placed against the cell rather than the page.
    pub in_cell: bool,
    pub allow_overlap: bool,
    /// Which of two overlapping objects is in front: the one with the larger number.
    pub order: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Placement {
    pub relative_to: RelativeTo,
    pub offset: Offset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Offset {
    /// A distance from the start of what the placement is relative to.
    By(i64),
    Align(Alignment),
}

/// `Start` is the left or the top, `End` the right or the bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Alignment {
    Start,
    Center,
    End,
    Inside,
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum RelativeTo {
    Page,
    Margin,
    Column,
    Paragraph,
    Line,
    Character,
    LeftMargin,
    RightMargin,
    TopMargin,
    BottomMargin,
    InsideMargin,
    OutsideMargin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Wrap {
    /// Text takes no notice of the object, which lies over or under it.
    None,
    /// Text keeps to the sides of the object's box.
    Square,
    /// Text follows the object's outline.
    Tight,
    /// As `Tight`, and also fills the hollows of the outline.
    Through,
    /// Text stops above the object and goes on below it.
    TopAndBottom,
}

/// Which sides of an object text may stand on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum WrapSide {
    Both,
    Left,
    Right,
    /// Only the wider of the two.
    Largest,
}

/// DrawingML sits in several namespaces, and in others again in ISO Strict
/// documents. Its element names are told apart well enough without them.
fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    elements(node).find(|n| n.tag_name().name() == name)
}

fn emu(value: Option<&str>) -> i64 {
    value.and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

fn flag(node: Node, name: &str) -> bool {
    matches!(node.attribute(name), Some("1" | "true"))
}

/// Reads a `w:drawing`. `None` when it holds neither an inline nor an anchored object.
pub(crate) fn read(drawing: Node) -> Option<Drawing> {
    let frame = elements(drawing).find(|n| matches!(n.tag_name().name(), "inline" | "anchor"))?;
    let extent = child(frame, "extent");
    let properties = child(frame, "docPr");
    let picture = child(frame, "graphic")
        .and_then(|graphic| child(graphic, "graphicData"))
        .and_then(|data| child(data, "pic"))
        .and_then(|picture| child(picture, "blipFill"))
        .map(|fill| {
            let embedded = child(fill, "blip")
                .and_then(|blip| blip.attributes().find(|a| a.name() == "embed" && a.namespace().is_some()))
                .map(|id| id.value().to_owned())
                .filter(|id| !id.is_empty());
            let crop = child(fill, "srcRect").map_or([0; 4], |rect| {
                ["l", "t", "r", "b"].map(|side| rect.attribute(side).and_then(|v| v.parse().ok()).unwrap_or(0))
            });
            // The relationship id for now; `resolve` turns it into the part's name.
            Picture { part: embedded, crop }
        });
    Some(Drawing {
        width: emu(extent.and_then(|e| e.attribute("cx"))),
        height: emu(extent.and_then(|e| e.attribute("cy"))),
        anchor: (frame.tag_name().name() == "anchor").then(|| anchor(frame)),
        picture,
        description: properties
            .and_then(|p| p.attribute("descr").or(p.attribute("title")))
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        text_box: None,
    })
}

/// For a `w:drawing` that is a shape with text: the element that holds the
/// text (`w:txbxContent`) and the box, its `blocks` still to be read.
pub(crate) fn text_box<'a, 'i>(drawing: Node<'a, 'i>) -> Option<(Node<'a, 'i>, TextBox)> {
    let frame = elements(drawing).find(|n| matches!(n.tag_name().name(), "inline" | "anchor"))?;
    let shape = child(child(child(frame, "graphic")?, "graphicData")?, "wsp")?;
    let content = child(child(shape, "txbx")?, "txbxContent")?;
    let colour = |holder: Node| -> Option<[u8; 3]> {
        let value = child(child(holder, "solidFill")?, "srgbClr")?.attribute("val")?;
        let number = u32::from_str_radix(value, 16).ok().filter(|_| value.len() == 6)?;
        Some([(number >> 16) as u8, (number >> 8) as u8, number as u8])
    };
    let properties = child(shape, "spPr");
    let line = properties.and_then(|p| child(p, "ln"));
    let body = child(shape, "bodyPr");
    // What a box has when it does not say: a tenth of an inch at the sides, half of that above and below.
    let inset = |name: &str, default: i64| {
        body.and_then(|b| b.attribute(name)).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
    };
    Some((
        content,
        TextBox {
            blocks: Vec::new(),
            insets: [inset("lIns", 91440), inset("tIns", 45720), inset("rIns", 91440), inset("bIns", 45720)],
            anchor: match body.and_then(|b| b.attribute("anchor")) {
                Some("ctr") => Alignment::Center,
                Some("b") => Alignment::End,
                _ => Alignment::Start,
            },
            fill: properties.and_then(colour),
            outline: line.and_then(|line| Some((colour(line)?, line.attribute("w").map_or(9525, |w| emu(Some(w)))))),
        },
    ))
}

fn anchor(frame: Node) -> Anchor {
    let placement = |name: &str, fallback: RelativeTo| {
        let Some(node) = child(frame, name) else {
            return Placement { relative_to: fallback, offset: Offset::By(0) };
        };
        let relative_to = match node.attribute("relativeFrom") {
            Some("page") => RelativeTo::Page,
            Some("margin") => RelativeTo::Margin,
            Some("column") => RelativeTo::Column,
            Some("paragraph") => RelativeTo::Paragraph,
            Some("line") => RelativeTo::Line,
            Some("character") => RelativeTo::Character,
            Some("leftMargin") => RelativeTo::LeftMargin,
            Some("rightMargin") => RelativeTo::RightMargin,
            Some("topMargin") => RelativeTo::TopMargin,
            Some("bottomMargin") => RelativeTo::BottomMargin,
            Some("insideMargin") => RelativeTo::InsideMargin,
            Some("outsideMargin") => RelativeTo::OutsideMargin,
            _ => fallback,
        };
        let offset = match child(node, "align").map(text_of).as_deref().map(str::trim) {
            Some("left" | "top") => Offset::Align(Alignment::Start),
            Some("center") => Offset::Align(Alignment::Center),
            Some("right" | "bottom") => Offset::Align(Alignment::End),
            Some("inside") => Offset::Align(Alignment::Inside),
            Some("outside") => Offset::Align(Alignment::Outside),
            _ => Offset::By(emu(child(node, "posOffset").map(text_of).as_deref())),
        };
        Placement { relative_to, offset }
    };
    let (wrap, wrap_node) = elements(frame)
        .find_map(|node| {
            let wrap = match node.tag_name().name() {
                "wrapNone" => Wrap::None,
                "wrapSquare" => Wrap::Square,
                "wrapTight" => Wrap::Tight,
                "wrapThrough" => Wrap::Through,
                "wrapTopAndBottom" => Wrap::TopAndBottom,
                _ => return None,
            };
            Some((wrap, Some(node)))
        })
        .unwrap_or((Wrap::None, None));
    Anchor {
        horizontal: placement("positionH", RelativeTo::Column),
        vertical: placement("positionV", RelativeTo::Paragraph),
        wrap,
        wrap_side: match wrap_node.and_then(|node| node.attribute("wrapText")) {
            Some("left") => WrapSide::Left,
            Some("right") => WrapSide::Right,
            Some("largest") => WrapSide::Largest,
            _ => WrapSide::Both,
        },
        distance: ["distT", "distB", "distL", "distR"].map(|side| emu(frame.attribute(side))),
        behind_text: flag(frame, "behindDoc"),
        in_cell: flag(frame, "layoutInCell"),
        allow_overlap: flag(frame, "allowOverlap"),
        order: emu(frame.attribute("relativeHeight")),
    }
}
