//! Small helpers over `roxmltree` for namespace-aware access to WordprocessingML.
//!
//! Documents exist in two flavours with different namespace URIs: Transitional
//! (what Word writes by default) and ISO Strict. Everything here accepts both.

use roxmltree::Node;

pub(crate) const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub(crate) const W_NS_STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
pub(crate) const MC_NS: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// Conventional prefixes, used only to print readable names in coverage reports.
const KNOWN_PREFIXES: &[(&str, &str)] = &[
    (W_NS, "w"),
    (W_NS_STRICT, "w"),
    (MC_NS, "mc"),
    ("http://schemas.openxmlformats.org/drawingml/2006/main", "a"),
    (
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
        "wp",
    ),
    (
        "http://schemas.openxmlformats.org/officeDocument/2006/math",
        "m",
    ),
    (
        "http://schemas.microsoft.com/office/word/2010/wordml",
        "w14",
    ),
    (
        "http://schemas.microsoft.com/office/word/2012/wordml",
        "w15",
    ),
    ("urn:schemas-microsoft-com:vml", "v"),
    ("urn:schemas-microsoft-com:office:office", "o"),
];

pub(crate) fn elements<'a, 'i>(node: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(|n| n.is_element())
}

/// Local name of `node` if it is an element in a WordprocessingML namespace.
pub(crate) fn w_name<'i>(node: Node<'_, 'i>) -> Option<&'i str> {
    let tag = node.tag_name();
    let in_w = matches!(tag.namespace(), Some(W_NS) | Some(W_NS_STRICT));
    (node.is_element() && in_w).then(|| tag.name())
}

pub(crate) fn w_child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    elements(node).find(|n| w_name(*n) == Some(name))
}

/// A `w:`-qualified attribute. Falls back to the unqualified name, which some
/// third-party producers write.
pub(crate) fn w_attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attribute((W_NS, name))
        .or_else(|| node.attribute((W_NS_STRICT, name)))
        .or_else(|| node.attribute(name))
}

pub(crate) fn val<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    w_attr(node, "val")
}

/// Value of an on/off element such as `<w:b/>`: present without `w:val` means on.
pub(crate) fn on_off(node: Node) -> bool {
    !matches!(val(node), Some("0" | "false" | "off"))
}

/// Parses an integer measure. Tolerates decimal values, which are not valid
/// per the schema but do occur in the wild.
pub(crate) fn int(value: Option<&str>) -> Option<i32> {
    let s = value?.trim();
    s.parse::<i32>().ok().or_else(|| {
        let f = s.parse::<f64>().ok()?;
        (f.is_finite() && f.abs() < i32::MAX as f64).then(|| f.round() as i32)
    })
}

pub(crate) fn is_mc_alternate_content(node: Node) -> bool {
    node.has_tag_name((MC_NS, "AlternateContent"))
}

/// The `mc:Fallback` branch of an `mc:AlternateContent`: the content meant for
/// consumers that do not understand the namespaces required by `mc:Choice`.
pub(crate) fn mc_fallback<'a, 'i>(node: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    elements(node).find(|n| n.has_tag_name((MC_NS, "Fallback")))
}

pub(crate) fn preserves_space(node: Node) -> bool {
    node.attribute((XML_NS, "space")) == Some("preserve")
}

pub(crate) fn text_of(node: Node) -> String {
    node.children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect()
}

/// `prefix:local` using conventional prefixes, or `{namespace}local` when unknown.
pub(crate) fn qualified_name(node: Node) -> String {
    let tag = node.tag_name();
    match tag.namespace() {
        None => tag.name().to_owned(),
        Some(ns) => match KNOWN_PREFIXES.iter().find(|(uri, _)| *uri == ns) {
            Some((_, prefix)) => format!("{prefix}:{}", tag.name()),
            None => format!("{{{ns}}}{}", tag.name()),
        },
    }
}
