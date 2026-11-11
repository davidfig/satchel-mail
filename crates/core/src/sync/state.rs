//! The merge: each item is a last-writer-wins register, and a computer's
//! own changes are found by comparing with what it last recorded (0080).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One item as the computers agree on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// The value; null once removed (a tombstone).
    pub v: Value,
    /// When it changed (ms) and on which computer.
    pub at: i64,
    pub by: String,
}

impl Entry {
    fn newer_than(&self, other: &Entry) -> bool {
        (self.at, &self.by) > (other.at, &other.by)
    }
}

/// An entry and this computer's form of it when last synced (canonical JSON,
/// "null" when it isn't here), which is what local changes are found against.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Known {
    pub items: BTreeMap<String, (Entry, String)>,
}

/// Proposals are never deleted, only forgotten by the sync after a while
/// (`proposals::KEEP_DAYS`), so one missing here isn't a removal.
pub fn removable(key: &str) -> bool {
    !key.starts_with("proposal:")
}

pub fn canon(v: Option<&Value>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "null".into())
}

impl Known {
    /// Records what changed here since the last pass, stamped `now` by `me`.
    /// Returns the keys that changed.
    pub fn take_local(&mut self, local: &BTreeMap<String, Value>, now: i64, me: &str) -> Vec<String> {
        let mut changed = vec![];
        for (k, v) in local {
            let c = canon(Some(v));
            if self.items.get(k).is_none_or(|(_, seen)| *seen != c) {
                self.items.insert(k.clone(), (Entry { v: v.clone(), at: now, by: me.into() }, c));
                changed.push(k.clone());
            }
        }
        for (k, (e, seen)) in self.items.iter_mut() {
            // Gone from here since it was last seen here: removed here.
            if !local.contains_key(k) && seen != "null" && removable(k) {
                *e = Entry { v: Value::Null, at: now, by: me.into() };
                *seen = "null".into();
                changed.push(k.clone());
            }
        }
        changed
    }

    /// Takes in another computer's items. Returns the ones that are newer
    /// than what's known here, which the caller applies to this computer and
    /// then records with `seen`.
    pub fn merge(&mut self, theirs: &BTreeMap<String, Entry>) -> Vec<(String, Entry)> {
        let mut newer = vec![];
        for (k, e) in theirs {
            let take = match self.items.get(k) {
                Some((mine, _)) => e.newer_than(mine),
                None => true,
            };
            if take {
                let seen = self.items.get(k).map(|(_, s)| s.clone()).unwrap_or_else(|| "null".into());
                self.items.insert(k.clone(), (e.clone(), seen));
                newer.push((k.clone(), e.clone()));
            }
        }
        newer
    }

    /// Drops what the sync no longer carries: proposals older than `before`
    /// (ms) that this computer doesn't list any more.
    pub fn forget_old(&mut self, before: i64, local: &BTreeMap<String, Value>) {
        self.items.retain(|k, (e, _)| removable(k) || e.at >= before || local.contains_key(k));
    }

    /// After applying, what this computer now has for `key`.
    pub fn seen(&mut self, key: &str, local: Option<&Value>) {
        if let Some((_, s)) = self.items.get_mut(key) {
            *s = canon(local);
        }
    }

    pub fn entries(&self) -> BTreeMap<String, Entry> {
        self.items.iter().map(|(k, (e, _))| (k.clone(), e.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn local_edits_and_removals_are_stamped() {
        let mut k = Known::default();
        assert_eq!(k.take_local(&map(&[("a", json!(1)), ("b", json!(2))]), 10, "me").len(), 2);
        assert!(k.take_local(&map(&[("a", json!(1)), ("b", json!(2))]), 20, "me").is_empty());
        assert_eq!(k.take_local(&map(&[("a", json!(3))]), 30, "me"), vec!["a", "b"]);
        assert_eq!(k.items["b"].0, Entry { v: Value::Null, at: 30, by: "me".into() });
        // A tombstone isn't removed again.
        assert!(k.take_local(&map(&[("a", json!(3))]), 40, "me").is_empty());
    }

    #[test]
    fn newest_wins_and_ties_go_to_the_larger_device() {
        let mut k = Known::default();
        k.take_local(&map(&[("a", json!(1))]), 10, "m");
        let older: BTreeMap<_, _> = [("a".to_string(), Entry { v: json!(0), at: 5, by: "z".into() })].into();
        assert!(k.merge(&older).is_empty());
        let tie: BTreeMap<_, _> = [("a".to_string(), Entry { v: json!(2), at: 10, by: "z".into() })].into();
        assert_eq!(k.merge(&tie).len(), 1);
        k.seen("a", Some(&json!(2)));
        // Applied here, so not mistaken for a local change.
        assert!(k.take_local(&map(&[("a", json!(2))]), 50, "m").is_empty());
    }

    #[test]
    fn an_item_this_computer_cant_hold_isnt_a_removal() {
        let mut k = Known::default();
        let theirs: BTreeMap<_, _> = [("account:x".to_string(), Entry { v: json!({}), at: 5, by: "z".into() })].into();
        k.merge(&theirs);
        k.seen("account:x", None);
        assert!(k.take_local(&BTreeMap::new(), 50, "m").is_empty());
        assert_eq!(k.items["account:x"].0.v, json!({}));
    }
}
