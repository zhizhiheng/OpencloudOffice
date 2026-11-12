use std::collections::BTreeMap;

use roxmltree::Node;
use serde::Serialize;

use crate::xml::qualified_name;

/// Tally of markup the reader met but does not model yet.
///
/// Running this over a corpus of real documents shows which features matter
/// most, so it doubles as the to-do list for the document model.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Coverage {
    /// Element name (or `parent/child` for properties) -> number of occurrences.
    pub unsupported: BTreeMap<String, usize>,
}

impl Coverage {
    pub(crate) fn miss(&mut self, context: Option<&str>, node: Node) {
        let name = qualified_name(node);
        let key = match context {
            Some(context) => format!("{context}/{name}"),
            None => name,
        };
        *self.unsupported.entry(key).or_default() += 1;
    }

    /// Records something other than a single unmodelled element.
    pub(crate) fn note(&mut self, what: &str) {
        *self.unsupported.entry(what.to_owned()).or_default() += 1;
    }

    pub(crate) fn merge(&mut self, other: Coverage) {
        for (name, count) in other.unsupported {
            *self.unsupported.entry(name).or_default() += count;
        }
    }

    /// Entries sorted by descending count, then by name.
    pub fn ranked(&self) -> Vec<(&str, usize)> {
        let mut entries: Vec<_> = self.unsupported.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        entries
    }
}
