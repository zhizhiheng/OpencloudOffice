//! Small helpers for namespace-aware access to WordprocessingML.
//!
//! Documents exist in two flavours with different namespace URIs: Transitional
//! (what Word writes by default) and ISO Strict. Everything here accepts both.

use crate::xmltree::Node;

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
    ("http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing", "wp"),
    ("http://schemas.openxmlformats.org/officeDocument/2006/math", "m"),
    ("http://schemas.microsoft.com/office/word/2010/wordml", "w14"),
    ("http://schemas.microsoft.com/office/word/2012/wordml", "w15"),
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
    node.attribute((W_NS, name)).or_else(|| node.attribute((W_NS_STRICT, name))).or_else(|| node.attribute(name))
}

pub(crate) fn val<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    w_attr(node, "val")
}

/// Value of an on/off element such as `<w:b/>`: present without `w:val` means on.
pub(crate) fn on_off(node: Node) -> bool {
    !matches!(val(node), Some("0" | "false" | "off"))
}

/// An on/off attribute, read by the same rule as [`on_off`].
pub(crate) fn on_off_attr(node: Node, name: &str) -> Option<bool> {
    w_attr(node, name).map(|value| !matches!(value, "0" | "false" | "off"))
}

/// Parses an integer attribute the way Word does: an optional sign, digits,
/// and an optional fraction that is cut off, not rounded (`120.5` is 120).
/// With no digits at all (`""`, `-`, `.5`) the value is 0. Anything else is
/// not a number; Word refuses such a file, we treat the attribute as absent.
pub(crate) fn int(value: Option<&str>) -> Option<i32> {
    let s = value?.trim();
    let (negative, unsigned) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if !whole.bytes().chain(fraction.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let magnitude = if whole.is_empty() { 0 } else { whole.parse::<i64>().ok()? };
    i32::try_from(if negative { -magnitude } else { magnitude }).ok()
}

/// Parses a length in twips (1/20 pt): either a bare number of twips or a
/// "universal measure" such as `36pt` or `0.5in`. Word writes the latter in
/// ISO Strict documents, so reading only integers loses every indent,
/// spacing and page dimension in them.
pub(crate) fn twips(value: Option<&str>) -> Option<i32> {
    measure(value?, 1.0)
}

/// Parses a font size in half-points, which also allows a universal measure.
pub(crate) fn half_points(value: Option<&str>) -> Option<u32> {
    measure(value?, 10.0).and_then(|v| u32::try_from(v).ok())
}

/// A bare number is already in the result unit. A number with a unit suffix
/// is converted through twips, `twips_per_result_unit` being 1 for a result
/// in twips and 10 for one in half-points.
fn measure(value: &str, twips_per_result_unit: f64) -> Option<i32> {
    let s = value.trim();
    const UNITS: [(&str, f64); 6] =
        [("mm", 1440.0 / 25.4), ("cm", 1440.0 / 2.54), ("in", 1440.0), ("pt", 20.0), ("pc", 240.0), ("pi", 240.0)];
    for (suffix, twips_per_unit) in UNITS {
        if let Some(number) = s.strip_suffix(suffix) {
            let amount = number.parse::<f64>().ok()?;
            return rounded(amount * twips_per_unit / twips_per_result_unit);
        }
    }
    int(Some(s))
}

fn rounded(value: f64) -> Option<i32> {
    (value.is_finite() && value.abs() < i32::MAX as f64).then(|| value.round() as i32)
}

/// For an `mc:AlternateContent`, the branch to read: `Some(Some(branch))`,
/// or `Some(None)` when no branch applies. `None` for any other element.
///
/// A producer offers `mc:Choice` branches that each require some namespaces,
/// and an `mc:Fallback` for consumers that understand none of them. Word
/// takes the first choice it understands, and so do we, judging by Word's
/// vocabulary rather than our own: what a choice holds that we do not model
/// is then reported as unmodelled, instead of being silently replaced by an
/// older rendition of the same content.
pub(crate) fn mc_branch<'a, 'i>(node: Node<'a, 'i>) -> Option<Option<Node<'a, 'i>>> {
    if !node.has_tag_name((MC_NS, "AlternateContent")) {
        return None;
    }
    let choice = elements(node).filter(|n| n.has_tag_name((MC_NS, "Choice"))).find(|choice| {
        let requires = choice.attribute("Requires").unwrap_or_default();
        !requires.trim().is_empty()
            && requires
                .split_whitespace()
                .all(|prefix| choice.lookup_namespace_uri(Some(prefix)).is_some_and(is_understood))
    });
    Some(choice.or_else(|| elements(node).find(|n| n.has_tag_name((MC_NS, "Fallback")))))
}

/// Whether a namespace belongs to the vocabulary current Word understands.
/// Microsoft's extension namespaces all live under these two roots.
fn is_understood(namespace: &str) -> bool {
    namespace.starts_with("http://schemas.microsoft.com/office/")
        || namespace.starts_with("http://schemas.openxmlformats.org/")
}

pub(crate) fn preserves_space(node: Node) -> bool {
    node.attribute((XML_NS, "space")) == Some("preserve")
}

pub(crate) fn text_of(node: Node) -> String {
    node.children().filter(|n| n.is_text()).filter_map(|n| n.text()).collect()
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
