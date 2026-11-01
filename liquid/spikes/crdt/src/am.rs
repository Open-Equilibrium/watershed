//! The Page model on Automerge.

use std::collections::HashSet;

use automerge::legacy;
use automerge::marks::{ExpandMark, Mark};
use automerge::transaction::Transactable;
use automerge::{ActorId, AutoCommit, ObjId, ObjType, ROOT, ReadDoc, ScalarValue, Value};

use crate::{Policy, Rejected, Replica, sorted_order};

pub struct Am {
    pub doc: AutoCommit,
}

fn actor(peer: u64) -> ActorId {
    ActorId::from(peer.to_be_bytes().to_vec())
}

fn peer_of(actor: &ActorId) -> u64 {
    u64::from_be_bytes(actor.to_bytes().try_into().unwrap_or([0; 8]))
}

fn scalar(value: &Value<'_>) -> String {
    match value {
        Value::Scalar(s) => match s.as_ref() {
            ScalarValue::Str(text) => text.to_string(),
            other => format!("{other}"),
        },
        Value::Object(kind) => format!("<{kind:?}>"),
    }
}

impl Am {
    fn root(&self, key: &str) -> ObjId {
        self.child(&ROOT, key).expect("seeded root map")
    }

    fn child(&self, parent: &ObjId, key: &str) -> Option<ObjId> {
        match self.doc.get(parent, key).ok()? {
            Some((Value::Object(_), id)) => Some(id),
            _ => None,
        }
    }

    fn get_str(&self, obj: &ObjId, key: &str) -> Option<String> {
        self.doc
            .get(obj, key)
            .ok()?
            .map(|(value, _)| scalar(&value))
    }

    fn text_obj(&self, id: &str) -> Option<ObjId> {
        let block = self.child(&self.root("blocks"), id)?;
        self.child(&block, "text")
    }

    /// Whether a received change writes inside `target`, read from its operations alone. An
    /// object the change itself creates is not in the Page yet; it is inside only if the
    /// operation creating it is, which already answers yes. Trying the change on a fork and
    /// diffing gives the same answer but copies the Page for every change.
    fn touches(&self, change: &automerge::Change, target: &ObjId) -> bool {
        change.decode().operations.iter().any(|op| match &op.obj {
            legacy::ObjectId::Root => {
                matches!(&op.key, legacy::Key::Map(key) if *target == self.root(key))
            }
            legacy::ObjectId::Id(id) => self.doc.import_obj(&id.to_string()).is_ok_and(|obj| {
                obj == *target
                    || self
                        .doc
                        .parents(&obj)
                        .is_ok_and(|mut parents| parents.any(|parent| parent.obj == *target))
            }),
        })
    }

    fn walk(&self, obj: &ObjId, out: &mut String) {
        match self.doc.object_type(obj) {
            Ok(ObjType::Text) => {
                out.push_str(&format!("{:?}", self.doc.text(obj).unwrap_or_default()));
                let mut marks: Vec<String> = self
                    .doc
                    .marks(obj)
                    .unwrap_or_default()
                    .iter()
                    .map(|m| format!("{}@{}..{}={}", m.name, m.start, m.end, m.value))
                    .collect();
                marks.sort();
                out.push_str(&format!("{marks:?}"));
            }
            _ => {
                out.push('{');
                for key in self.doc.keys(obj) {
                    out.push_str(&key);
                    out.push(':');
                    match self.doc.get(obj, key.as_str()) {
                        Ok(Some((Value::Object(_), child))) => self.walk(&child, out),
                        Ok(Some((value, _))) => out.push_str(&scalar(&value)),
                        _ => {}
                    }
                    out.push(',');
                }
                out.push('}');
            }
        }
    }
}

impl Replica for Am {
    fn seed(peer: u64) -> Self {
        let mut doc = AutoCommit::new().with_actor(actor(peer));
        for key in ["blocks", "order", "rows", "connections", "automations"] {
            doc.put_object(ROOT, key, ObjType::Map).unwrap();
        }
        doc.commit();
        Am { doc }
    }

    fn fork_as(&mut self, peer: u64) -> Self {
        Am {
            doc: self.doc.fork().with_actor(actor(peer)),
        }
    }

    fn add_text_block(&mut self, id: &str, text: &str, sort_key: &str) {
        let blocks = self.root("blocks");
        let block = self.doc.put_object(&blocks, id, ObjType::Map).unwrap();
        self.doc.put(&block, "kind", "text").unwrap();
        let obj = self.doc.put_object(&block, "text", ObjType::Text).unwrap();
        self.doc.splice_text(&obj, 0, 0, text).unwrap();
        let order = self.root("order");
        self.doc.put(&order, id, sort_key).unwrap();
    }

    fn edit_text(&mut self, id: &str, pos: usize, delete: usize, insert: &str) {
        let obj = self.text_obj(id).expect("text block");
        self.doc
            .splice_text(&obj, pos, delete as isize, insert)
            .unwrap();
    }

    fn bold(&mut self, id: &str, start: usize, end: usize) {
        let obj = self.text_obj(id).expect("text block");
        self.doc
            .mark(
                &obj,
                Mark::new("bold".into(), true, start, end),
                ExpandMark::After,
            )
            .unwrap();
    }

    fn put_cell(&mut self, row: &str, field: &str, value: &str) {
        let rows = self.root("rows");
        let row_obj = match self.child(&rows, row) {
            Some(obj) => obj,
            None => self.doc.put_object(&rows, row, ObjType::Map).unwrap(),
        };
        self.doc.put(&row_obj, field, value).unwrap();
    }

    fn move_view(&mut self, id: &str, sort_key: &str) {
        let order = self.root("order");
        self.doc.put(&order, id, sort_key).unwrap();
    }

    fn add_connection(&mut self, id: &str, from: &str, to: &str) {
        let connections = self.root("connections");
        let obj = self.doc.put_object(&connections, id, ObjType::Map).unwrap();
        self.doc.put(&obj, "from", from).unwrap();
        self.doc.put(&obj, "to", to).unwrap();
    }

    fn add_automation(&mut self, id: &str, owner: Option<&str>, uses: &str) {
        let automations = self.root("automations");
        let obj = self.doc.put_object(&automations, id, ObjType::Map).unwrap();
        if let Some(owner) = owner {
            self.doc.put(&obj, "owner", owner).unwrap();
        }
        self.doc.put(&obj, "uses", uses).unwrap();
        self.doc.put(&obj, "enabled", true).unwrap();
    }

    fn delete_block(&mut self, id: &str) {
        let (blocks, order) = (self.root("blocks"), self.root("order"));
        self.doc.delete(&blocks, id).unwrap();
        self.doc.delete(&order, id).unwrap();
        let connections = self.root("connections");
        let keys: Vec<String> = self.doc.keys(&connections).collect();
        for key in keys {
            let obj = self.child(&connections, &key).unwrap();
            let ends = [self.get_str(&obj, "from"), self.get_str(&obj, "to")];
            if ends.iter().any(|end| end.as_deref() == Some(id)) {
                self.doc.delete(&connections, key.as_str()).unwrap();
            }
        }
        let automations = self.root("automations");
        let keys: Vec<String> = self.doc.keys(&automations).collect();
        for key in keys {
            let obj = self.child(&automations, &key).unwrap();
            if self.get_str(&obj, "owner").as_deref() == Some(id) {
                self.doc.delete(&automations, key.as_str()).unwrap();
            } else if self.get_str(&obj, "uses").as_deref() == Some(id) {
                self.doc.put(&obj, "enabled", false).unwrap();
            }
        }
        self.doc.commit();
    }

    fn commit(&mut self) {
        self.doc.commit();
    }

    fn pull(&mut self, other: &mut Self) {
        self.doc.merge(&mut other.doc).unwrap();
    }

    fn pull_checked(&mut self, other: &mut Self, policy: Policy) -> Rejected {
        let rows = self.root("rows");
        let mut rejected = Vec::new();
        let mut refused = HashSet::new();
        for change in other.doc.get_changes(&self.doc.get_heads()) {
            if self.doc.get_change_by_hash(&change.hash()).is_some() {
                continue;
            }
            let depends_on_refused = change.deps().iter().any(|dep| refused.contains(dep));
            let denied = depends_on_refused
                || (peer_of(change.actor_id()) == policy.denied_rows_peer
                    && self.touches(&change, &rows));
            if denied {
                refused.insert(change.hash());
                rejected.push(change.hash().to_string());
            } else {
                self.doc.apply_changes([change]).unwrap();
            }
        }
        rejected.sort();
        rejected
    }

    fn cell_values(&mut self, row: &str, field: &str) -> Vec<String> {
        let Some(row_obj) = self.child(&self.root("rows"), row) else {
            return Vec::new();
        };
        let mut values: Vec<String> = self
            .doc
            .get_all(&row_obj, field)
            .unwrap()
            .iter()
            .map(|(value, _)| scalar(value))
            .collect();
        values.sort();
        values
    }

    fn order(&mut self) -> Vec<String> {
        let order = self.root("order");
        let keys = self
            .doc
            .keys(&order)
            .map(|id| {
                let key = self.get_str(&order, &id).unwrap_or_default();
                (id, key)
            })
            .collect();
        sorted_order(keys)
    }

    fn text(&mut self, id: &str) -> Option<String> {
        let obj = self.text_obj(id)?;
        self.doc.text(&obj).ok()
    }

    fn canonical(&mut self) -> String {
        let mut out = String::new();
        self.walk(&ROOT, &mut out);
        out
    }

    fn save(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    fn load(bytes: &[u8]) -> Self {
        Am {
            doc: AutoCommit::load(bytes).unwrap(),
        }
    }
}
