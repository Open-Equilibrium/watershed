use std::collections::HashMap;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType, Patch, PatchAction, Prop, ROOT, ReadDoc, Value};

use crate::Error;

#[derive(Default)]
struct Ids(HashMap<ObjId, ObjId>);

impl Ids {
    fn of(&self, obj: &ObjId) -> ObjId {
        self.0.get(obj).cloned().unwrap_or_else(|| obj.clone())
    }
}

fn put_value(
    doc: &mut AutoCommit,
    ids: &mut Ids,
    obj: &ObjId,
    prop: Prop,
    value: &Value<'static>,
    source: &ObjId,
) -> Result<(), Error> {
    match value {
        Value::Object(kind) => {
            let made = match prop {
                Prop::Map(key) => doc.put_object(obj, key, *kind)?,
                Prop::Seq(index) => doc.put_object(obj, index, *kind)?,
            };
            ids.0.insert(source.clone(), made);
        }
        Value::Scalar(scalar) => match prop {
            Prop::Map(key) => doc.put(obj, key, scalar.as_ref().clone())?,
            Prop::Seq(index) => doc.put(obj, index, scalar.as_ref().clone())?,
        },
    }
    Ok(())
}

pub(crate) fn apply(doc: &mut AutoCommit, patches: Vec<Patch>) -> Result<(), Error> {
    let mut ids = Ids::default();
    for patch in patches {
        let obj = ids.of(&patch.obj);
        match patch.action {
            PatchAction::PutMap { key, value, .. } => {
                put_value(doc, &mut ids, &obj, Prop::Map(key), &value.0, &value.1)?;
            }
            PatchAction::PutSeq { index, value, .. } => {
                put_value(doc, &mut ids, &obj, Prop::Seq(index), &value.0, &value.1)?;
            }
            PatchAction::Insert { index, values } => {
                for (at, (value, source, _)) in values.iter().enumerate() {
                    match value {
                        Value::Object(kind) => {
                            let made = doc.insert_object(&obj, index + at, *kind)?;
                            ids.0.insert(source.clone(), made);
                        }
                        Value::Scalar(scalar) => {
                            doc.insert(&obj, index + at, scalar.as_ref().clone())?;
                        }
                    }
                }
            }
            PatchAction::SpliceText { index, value, .. } => {
                doc.splice_text(&obj, index, 0, &value.make_string())?;
            }
            PatchAction::DeleteMap { key } => doc.delete(&obj, key)?,
            PatchAction::DeleteSeq { index, length } => {
                if doc.object_type(&obj)? == ObjType::Text {
                    doc.splice_text(&obj, index, length as isize, "")?;
                } else {
                    for _ in 0..length {
                        doc.delete(&obj, index)?;
                    }
                }
            }
            PatchAction::Increment { prop, value } => doc.increment(&obj, prop, value)?,
            PatchAction::Conflict { .. } | PatchAction::Mark { .. } => {}
        }
    }
    Ok(())
}

pub(crate) fn touched(patches: &[Patch]) -> Vec<(ObjId, String)> {
    patches
        .iter()
        .filter_map(|patch| match &patch.action {
            PatchAction::PutMap { key, .. } | PatchAction::DeleteMap { key } => {
                Some((patch.obj.clone(), key.clone()))
            }
            _ => None,
        })
        .collect()
}

fn copy(
    source: &AutoCommit,
    from: &ObjId,
    target: &mut AutoCommit,
    to: &ObjId,
) -> Result<(), Error> {
    match source.object_type(from)? {
        ObjType::Map | ObjType::Table => {
            for item in source.map_range(from, ..) {
                let key = item.key.to_string();
                let child = item.id();
                match item.value.into_value() {
                    Value::Object(kind) => {
                        let made = target.put_object(to, key.as_str(), kind)?;
                        copy(source, &child, target, &made)?;
                    }
                    Value::Scalar(scalar) => target.put(to, key.as_str(), scalar.into_owned())?,
                }
            }
        }
        ObjType::List => {
            for (at, item) in source.list_range(from, ..).enumerate() {
                let child = item.id();
                match item.value.into_value() {
                    Value::Object(kind) => {
                        let made = target.insert_object(to, at, kind)?;
                        copy(source, &child, target, &made)?;
                    }
                    Value::Scalar(scalar) => target.insert(to, at, scalar.into_owned())?,
                }
            }
        }
        ObjType::Text => {
            let text = source.text(from)?;
            target.splice_text(to, 0, 0, &text)?;
        }
    }
    Ok(())
}

pub(crate) fn rebuild(source: &AutoCommit, target: &mut AutoCommit) -> Result<(), Error> {
    copy(source, &ROOT, target, &ROOT)
}
