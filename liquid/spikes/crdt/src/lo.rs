//! The Page model on Loro.

use std::borrow::Cow;

use loro::{
    Container, ContainerID, ExpandType, ExportMode, ID, IdSpan, LoroDoc, LoroMap, LoroText,
    LoroValue, StyleConfig, ValueOrContainer,
};

use crate::{Policy, Rejected, Replica, sorted_order};

pub struct Lo {
    pub doc: LoroDoc,
}

/// Formatting expands like Automerge's, and every commit stays its own change: by default
/// Loro merges a peer's commits within 1000 s into one change, which would let one denied
/// edit take the allowed edits around it down with it.
fn configure(doc: &LoroDoc) {
    doc.config_default_text_style(Some(StyleConfig {
        expand: ExpandType::After,
    }));
    doc.set_change_merge_interval(-1);
}

fn value(value: &LoroValue) -> String {
    match value {
        LoroValue::String(text) => text.to_string(),
        LoroValue::Map(map) => {
            let mut keys: Vec<_> = map.iter().collect();
            keys.sort_by(|a, b| a.0.cmp(b.0));
            let inner: Vec<String> = keys
                .iter()
                .map(|(k, v)| format!("{k}:{}", self::value(v)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        LoroValue::List(list) => {
            let inner: Vec<String> = list.iter().map(self::value).collect();
            format!("[{}]", inner.join(","))
        }
        LoroValue::Bool(b) => b.to_string(),
        LoroValue::I64(n) => n.to_string(),
        LoroValue::Double(n) => n.to_string(),
        other => format!("{other:?}"),
    }
}

impl Lo {
    fn root(&self, key: &str) -> LoroMap {
        self.doc.get_map(key)
    }

    fn child_map(map: &LoroMap, key: &str) -> Option<LoroMap> {
        match map.get(key)? {
            ValueOrContainer::Container(Container::Map(child)) => Some(child),
            _ => None,
        }
    }

    fn get_str(map: &LoroMap, key: &str) -> Option<String> {
        match map.get(key)? {
            ValueOrContainer::Value(v) => Some(value(&v)),
            ValueOrContainer::Container(_) => None,
        }
    }

    fn text_obj(&self, id: &str) -> Option<LoroText> {
        let block = Self::child_map(&self.root("blocks"), id)?;
        match block.get("text")? {
            ValueOrContainer::Container(Container::Text(text)) => Some(text),
            _ => None,
        }
    }

    fn walk(map: &LoroMap) -> String {
        let mut keys: Vec<String> = map.keys().map(|k| k.to_string()).collect();
        keys.sort();
        let mut out = String::from("{");
        for key in keys {
            out.push_str(&key);
            out.push(':');
            match map.get(&key) {
                Some(ValueOrContainer::Container(Container::Map(child))) => {
                    out.push_str(&Self::walk(&child))
                }
                Some(ValueOrContainer::Container(Container::Text(text))) => {
                    out.push_str(&value(&text.get_richtext_value()))
                }
                Some(ValueOrContainer::Value(v)) => out.push_str(&value(&v)),
                _ => {}
            }
            out.push(',');
        }
        out.push('}');
        out
    }

    fn touches_rows(doc: &LoroDoc, cid: &ContainerID) -> bool {
        let is_rows = |id: &ContainerID| matches!(id, ContainerID::Root { name, .. } if name.as_str() == "rows");
        is_rows(cid)
            || doc
                .get_path_to_container(cid)
                .is_some_and(|path| path.iter().any(|(id, _)| is_rows(id)))
    }
}

impl Replica for Lo {
    fn seed(peer: u64) -> Self {
        let doc = LoroDoc::new();
        doc.set_peer_id(peer).unwrap();
        configure(&doc);
        Lo { doc }
    }

    fn fork_as(&mut self, peer: u64) -> Self {
        self.doc.commit();
        let doc = self.doc.fork();
        doc.set_peer_id(peer).unwrap();
        configure(&doc);
        Lo { doc }
    }

    fn add_text_block(&mut self, id: &str, text: &str, sort_key: &str) {
        let block = self
            .root("blocks")
            .insert_container(id, LoroMap::new())
            .unwrap();
        block.insert("kind", "text").unwrap();
        let obj = block.insert_container("text", LoroText::new()).unwrap();
        obj.insert(0, text).unwrap();
        self.root("order").insert(id, sort_key).unwrap();
    }

    fn edit_text(&mut self, id: &str, pos: usize, delete: usize, insert: &str) {
        let obj = self.text_obj(id).expect("text block");
        if delete > 0 {
            obj.delete(pos, delete).unwrap();
        }
        obj.insert(pos, insert).unwrap();
    }

    fn bold(&mut self, id: &str, start: usize, end: usize) {
        let obj = self.text_obj(id).expect("text block");
        obj.mark(start..end, "bold", true).unwrap();
    }

    fn put_cell(&mut self, row: &str, field: &str, value: &str) {
        let rows = self.root("rows");
        let row_obj = match Self::child_map(&rows, row) {
            Some(obj) => obj,
            None => rows.insert_container(row, LoroMap::new()).unwrap(),
        };
        row_obj.insert(field, value).unwrap();
    }

    fn move_view(&mut self, id: &str, sort_key: &str) {
        self.root("order").insert(id, sort_key).unwrap();
    }

    fn add_connection(&mut self, id: &str, from: &str, to: &str) {
        let obj = self
            .root("connections")
            .insert_container(id, LoroMap::new())
            .unwrap();
        obj.insert("from", from).unwrap();
        obj.insert("to", to).unwrap();
    }

    fn add_automation(&mut self, id: &str, owner: Option<&str>, uses: &str) {
        let obj = self
            .root("automations")
            .insert_container(id, LoroMap::new())
            .unwrap();
        if let Some(owner) = owner {
            obj.insert("owner", owner).unwrap();
        }
        obj.insert("uses", uses).unwrap();
        obj.insert("enabled", true).unwrap();
    }

    fn delete_block(&mut self, id: &str) {
        self.root("blocks").delete(id).unwrap();
        self.root("order").delete(id).unwrap();
        let connections = self.root("connections");
        let keys: Vec<String> = connections.keys().map(|k| k.to_string()).collect();
        for key in keys {
            let obj = Self::child_map(&connections, &key).unwrap();
            let ends = [Self::get_str(&obj, "from"), Self::get_str(&obj, "to")];
            if ends.iter().any(|end| end.as_deref() == Some(id)) {
                connections.delete(&key).unwrap();
            }
        }
        let automations = self.root("automations");
        let keys: Vec<String> = automations.keys().map(|k| k.to_string()).collect();
        for key in keys {
            let obj = Self::child_map(&automations, &key).unwrap();
            if Self::get_str(&obj, "owner").as_deref() == Some(id) {
                automations.delete(&key).unwrap();
            } else if Self::get_str(&obj, "uses").as_deref() == Some(id) {
                obj.insert("enabled", false).unwrap();
            }
        }
        self.doc.commit();
    }

    fn commit(&mut self) {
        self.doc.commit();
    }

    fn pull(&mut self, other: &mut Self) {
        other.doc.commit();
        let bytes = other
            .doc
            .export(ExportMode::updates(&self.doc.oplog_vv()))
            .unwrap();
        self.doc.import(&bytes).unwrap();
    }

    fn pull_checked(&mut self, other: &mut Self, policy: Policy) -> Rejected {
        other.doc.commit();
        self.doc.commit();
        let mine = self.doc.oplog_vv();
        let mut changes = Vec::new();
        for (peer, end) in other.doc.oplog_vv().iter() {
            let mut counter = mine.get(peer).copied().unwrap_or(0);
            while counter < *end {
                let meta = other.doc.get_change(ID::new(*peer, counter)).unwrap();
                let span = IdSpan::new(*peer, meta.id.counter, meta.id.counter + meta.len as i32);
                counter = span.counter.end;
                changes.push((meta, span));
            }
        }
        changes.sort_by_key(|(meta, _)| (meta.lamport, meta.id.peer));
        let mut refused: Vec<IdSpan> = Vec::new();
        let mut rejected = Vec::new();
        for (meta, span) in changes {
            let depends_on_refused = meta.deps.iter().any(|dep| {
                refused
                    .iter()
                    .any(|r| r.peer == dep.peer && r.counter.contains(dep.counter))
            });
            let bytes = other
                .doc
                .export(ExportMode::UpdatesInRange {
                    spans: Cow::Owned(vec![span]),
                })
                .unwrap();
            let denied = depends_on_refused || {
                let trial = self.doc.fork();
                let before = self.doc.oplog_frontiers();
                let status = trial.import(&bytes).unwrap();
                status.pending.is_some()
                    || (meta.id.peer == policy.denied_rows_peer
                        && trial
                            .diff(&before, &trial.oplog_frontiers())
                            .unwrap()
                            .iter()
                            .any(|(cid, _)| Self::touches_rows(&trial, cid)))
            };
            if denied {
                rejected.push(format!(
                    "{}@{}..{}",
                    span.peer, span.counter.start, span.counter.end
                ));
                refused.push(span);
            } else {
                self.doc.import(&bytes).unwrap();
            }
        }
        rejected.sort();
        rejected
    }

    fn cell_values(&mut self, row: &str, field: &str) -> Vec<String> {
        Self::child_map(&self.root("rows"), row)
            .and_then(|obj| Self::get_str(&obj, field))
            .into_iter()
            .collect()
    }

    fn order(&mut self) -> Vec<String> {
        let order = self.root("order");
        let keys = order
            .keys()
            .map(|id| {
                let key = Self::get_str(&order, &id).unwrap_or_default();
                (id.to_string(), key)
            })
            .collect();
        sorted_order(keys)
    }

    fn text(&mut self, id: &str) -> Option<String> {
        self.text_obj(id).map(|t| t.to_string())
    }

    fn canonical(&mut self) -> String {
        self.doc.commit();
        ["automations", "blocks", "connections", "order", "rows"]
            .iter()
            .map(|key| format!("{key}:{}", Self::walk(&self.root(key))))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn save(&mut self) -> Vec<u8> {
        self.doc.commit();
        self.doc.export(ExportMode::Snapshot).unwrap()
    }

    fn load(bytes: &[u8]) -> Self {
        let doc = LoroDoc::new();
        doc.import(bytes).unwrap();
        configure(&doc);
        Lo { doc }
    }
}
