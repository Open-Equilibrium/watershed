//! The Page model on Loro.

use std::borrow::Cow;
use std::collections::HashMap;

use loro::{
    Container, ContainerID, ExpandType, ExportMode, IdSpan, JsonChange, JsonOpContent, LoroDoc,
    LoroMap, LoroText, LoroValue, StyleConfig, ValueOrContainer, VersionVector, json::MapOp,
};

use crate::{Automation, Policy, Rejected, Replica, sorted_order};

pub struct Lo {
    pub doc: LoroDoc,
    /// Where every container was created: its parent and key, from the map insertions of
    /// all changes, independent of deletions and of which concurrent write won.
    created: HashMap<ContainerID, ContainerID>,
    indexed: VersionVector,
}

fn record_created(created: &mut HashMap<ContainerID, ContainerID>, change: &JsonChange) {
    for op in &change.ops {
        if let JsonOpContent::Map(MapOp::Insert {
            value: LoroValue::Container(child),
            ..
        }) = &op.content
        {
            created.insert(child.clone(), op.container.clone());
        }
    }
}

/// Whether `container` lies under the root container `root`, walking creation parents.
fn under(
    created: &HashMap<ContainerID, ContainerID>,
    local: &HashMap<ContainerID, ContainerID>,
    container: &ContainerID,
    root: &str,
) -> bool {
    let mut current = container.clone();
    loop {
        if let ContainerID::Root { name, .. } = &current {
            return name.as_str() == root;
        }
        match local.get(&current).or_else(|| created.get(&current)) {
            Some(parent) => current = parent.clone(),
            None => return false,
        }
    }
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

    fn new(doc: LoroDoc) -> Self {
        configure(&doc);
        Lo {
            doc,
            created: HashMap::new(),
            indexed: VersionVector::default(),
        }
    }

    fn changes_between(doc: &LoroDoc, from: &VersionVector, to: &VersionVector) -> Vec<JsonChange> {
        doc.export_json_updates_without_peer_compression(from, to)
            .unwrap()
            .changes
    }

    fn index_new_changes(&mut self) {
        let now = self.doc.oplog_vv();
        for change in Self::changes_between(&self.doc, &self.indexed, &now) {
            record_created(&mut self.created, &change);
        }
        self.indexed = now;
    }

    /// Whether a received change writes under the root container `root`, read from its
    /// operations and the creation index alone; a visible diff would depend on which
    /// concurrent write already won.
    fn touches(&self, change: &JsonChange, root: &str) -> bool {
        let mut local = HashMap::new();
        for op in &change.ops {
            if under(&self.created, &local, &op.container, root) {
                return true;
            }
            if let JsonOpContent::Map(MapOp::Insert {
                value: LoroValue::Container(child),
                ..
            }) = &op.content
            {
                local.insert(child.clone(), op.container.clone());
            }
        }
        false
    }
}

impl Replica for Lo {
    fn seed(peer: u64) -> Self {
        let doc = LoroDoc::new();
        doc.set_peer_id(peer).unwrap();
        Lo::new(doc)
    }

    fn fork_as(&mut self, peer: u64) -> Self {
        self.doc.commit();
        let doc = self.doc.fork();
        doc.set_peer_id(peer).unwrap();
        Lo::new(doc)
    }

    fn add_text_block(&mut self, id: &str, text: &str, sort_key: &str) {
        let block = self
            .root("blocks")
            .insert_container(id, LoroMap::new())
            .unwrap();
        block.insert("kind", "text").unwrap();
        let obj = block.insert_container("text", LoroText::new()).unwrap();
        obj.insert(0, text).unwrap();
        let view = self
            .root("order")
            .insert_container(id, LoroMap::new())
            .unwrap();
        view.insert("key", sort_key).unwrap();
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

    /// Writes inside the View's own container, so a concurrent deletion of the View wins.
    fn move_view(&mut self, id: &str, sort_key: &str) {
        if let Some(view) = Self::child_map(&self.root("order"), id) {
            view.insert("key", sort_key).unwrap();
        }
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
        self.index_new_changes();
        let mine = self.doc.oplog_vv();
        let mut changes = Self::changes_between(&other.doc, &mine, &other.doc.oplog_vv());
        changes.sort_by_key(|change| (change.lamport, change.id.peer));
        let mut refused: Vec<IdSpan> = Vec::new();
        let mut rejected = Vec::new();
        for change in changes {
            let len = other.doc.get_change(change.id).unwrap().len as i32;
            let span = IdSpan::new(change.id.peer, change.id.counter, change.id.counter + len);
            let depends_on_refused = change.deps.iter().any(|dep| {
                refused
                    .iter()
                    .any(|r| r.peer == dep.peer && r.counter.contains(dep.counter))
            });
            let denied = depends_on_refused
                || (change.id.peer == policy.denied_rows_peer && self.touches(&change, "rows"));
            if denied {
                rejected.push(format!(
                    "{}@{}..{}",
                    span.peer, span.counter.start, span.counter.end
                ));
                refused.push(span);
            } else {
                record_created(&mut self.created, &change);
                let bytes = other
                    .doc
                    .export(ExportMode::UpdatesInRange {
                        spans: Cow::Owned(vec![span]),
                    })
                    .unwrap();
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
                let key = Self::child_map(&order, &id)
                    .and_then(|view| Self::get_str(&view, "key"))
                    .unwrap_or_default();
                (id.to_string(), key)
            })
            .collect();
        sorted_order(keys)
    }

    fn connections(&mut self) -> Vec<(String, String, String)> {
        let connections = self.root("connections");
        let keys: Vec<String> = connections.keys().map(|k| k.to_string()).collect();
        keys.into_iter()
            .filter_map(|id| {
                let obj = Self::child_map(&connections, &id)?;
                Some((id, Self::get_str(&obj, "from")?, Self::get_str(&obj, "to")?))
            })
            .collect()
    }

    fn automations(&mut self) -> Vec<Automation> {
        let automations = self.root("automations");
        let keys: Vec<String> = automations.keys().map(|k| k.to_string()).collect();
        keys.into_iter()
            .filter_map(|id| {
                let obj = Self::child_map(&automations, &id)?;
                Some(Automation {
                    owner: Self::get_str(&obj, "owner"),
                    uses: Self::get_str(&obj, "uses")?,
                    enabled: Self::get_str(&obj, "enabled").as_deref() == Some("true"),
                    id,
                })
            })
            .collect()
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
        Lo::new(doc)
    }
}
