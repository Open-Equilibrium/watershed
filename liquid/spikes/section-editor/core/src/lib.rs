use std::ffi::{CStr, CString, c_char};

use automerge::marks::{ExpandMark, Mark};
use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjType, ROOT, ReadDoc, ScalarValue, Span, TextEncoding, Value};
use serde_json::{Value as Json, json};

pub struct Section {
    doc: AutoCommit,
    text: automerge::ObjId,
}

fn expand(name: &str) -> ExpandMark {
    match name {
        "bold" | "italic" | "strike" => ExpandMark::After,
        _ => ExpandMark::None,
    }
}

fn width(text: &str) -> usize {
    text.encode_utf16().count()
}

impl Section {
    pub fn new() -> Self {
        let mut doc = AutoCommit::new_with_encoding(TextEncoding::Utf16CodeUnit);
        let text = doc.put_object(ROOT, "text", ObjType::Text).unwrap();
        let first = doc.split_block(&text, 0).unwrap();
        doc.put(&first, "type", "paragraph").unwrap();
        Section { doc, text }
    }

    pub fn fork(&mut self) -> Self {
        let doc = self.doc.fork();
        let text = self.text.clone();
        Section { doc, text }
    }

    pub fn merge(&mut self, other: &mut Section) {
        self.doc.merge(&mut other.doc).unwrap();
    }

    pub fn apply(&mut self, op: &Json) -> Result<Json, String> {
        let at = |key: &str| op[key].as_u64().map(|v| v as usize);
        let fail = |e: automerge::AutomergeError| e.to_string();
        match op["op"].as_str() {
            Some("splice") => {
                let del = at("del").unwrap_or(0) as isize;
                let ins = op["ins"].as_str().unwrap_or("");
                self.doc
                    .splice_text(&self.text, at("at").ok_or("at")?, del, ins)
                    .map_err(fail)?;
                Ok(Json::Null)
            }
            Some("split") => {
                let id = self
                    .doc
                    .split_block(&self.text, at("at").ok_or("at")?)
                    .map_err(fail)?;
                self.write_block(&id, op)?;
                Ok(json!(id.to_string()))
            }
            Some("block") => {
                let id = self.marker(op["id"].as_str().ok_or("id")?)?;
                self.write_block(&id, op)?;
                Ok(Json::Null)
            }
            Some("mark") => {
                let name = op["name"].as_str().ok_or("name")?;
                let value = match &op["value"] {
                    Json::String(s) => ScalarValue::Str(s.as_str().into()),
                    _ => ScalarValue::Boolean(true),
                };
                let mark = Mark::new(
                    name.into(),
                    value,
                    at("start").ok_or("start")?,
                    at("end").ok_or("end")?,
                );
                self.doc
                    .mark(&self.text, mark, expand(name))
                    .map_err(fail)?;
                Ok(Json::Null)
            }
            Some("unmark") => {
                let name = op["name"].as_str().ok_or("name")?;
                self.doc
                    .unmark(
                        &self.text,
                        name,
                        at("start").ok_or("start")?,
                        at("end").ok_or("end")?,
                        expand(name),
                    )
                    .map_err(fail)?;
                Ok(Json::Null)
            }
            other => Err(format!("unknown op {other:?}")),
        }
    }

    fn marker(&self, id: &str) -> Result<automerge::ObjId, String> {
        self.doc
            .import(id)
            .map(|(id, _)| id)
            .map_err(|e| e.to_string())
    }

    fn write_block(&mut self, id: &automerge::ObjId, op: &Json) -> Result<(), String> {
        let fail = |e: automerge::AutomergeError| e.to_string();
        if let Some(kind) = op["type"].as_str() {
            self.doc.put(id, "type", kind).map_err(fail)?;
        }
        if let Some(indent) = op["indent"].as_i64() {
            self.doc.put(id, "indent", indent).map_err(fail)?;
        }
        if let Some(done) = op["done"].as_bool() {
            self.doc.put(id, "done", done).map_err(fail)?;
        }
        Ok(())
    }

    fn block_field(&self, id: &automerge::ObjId, key: &str) -> Json {
        match self.doc.get(id, key).ok().flatten() {
            Some((Value::Scalar(s), _)) => match s.as_ref() {
                ScalarValue::Str(v) => json!(v.as_str()),
                ScalarValue::Int(v) => json!(v),
                ScalarValue::Uint(v) => json!(v),
                ScalarValue::Boolean(v) => json!(v),
                _ => Json::Null,
            },
            _ => Json::Null,
        }
    }

    pub fn spans(&self) -> Json {
        let mut out = Vec::new();
        let mut index = 0usize;
        for span in self.doc.spans(&self.text).unwrap() {
            match span {
                Span::Block(_) => {
                    let id = match self.doc.get(&self.text, index).ok().flatten() {
                        Some((Value::Object(_), id)) => id,
                        _ => panic!("no marker at {index}"),
                    };
                    out.push(json!({
                        "block": id.to_string(),
                        "type": self.block_field(&id, "type"),
                        "indent": self.block_field(&id, "indent"),
                        "done": self.block_field(&id, "done"),
                    }));
                    index += 1;
                }
                Span::Text { text, marks } => {
                    let mut set = serde_json::Map::new();
                    if let Some(marks) = marks {
                        for (name, value) in marks.iter() {
                            let value = match value {
                                ScalarValue::Str(s) => json!(s.as_str()),
                                ScalarValue::Null => continue,
                                ScalarValue::Boolean(false) => continue,
                                _ => json!(true),
                            };
                            set.insert(name.to_string(), value);
                        }
                    }
                    index += width(&text);
                    out.push(json!({ "text": text, "marks": set }));
                }
            }
        }
        Json::Array(out)
    }
}

impl Default for Section {
    fn default() -> Self {
        Self::new()
    }
}

fn into_c(value: Json) -> *mut c_char {
    CString::new(value.to_string()).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub extern "C" fn sec_new() -> *mut Section {
    Box::into_raw(Box::new(Section::new()))
}

/// # Safety
/// `section` comes from `sec_new` or `sec_fork` and is freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sec_free(section: *mut Section) {
    drop(unsafe { Box::from_raw(section) });
}

/// # Safety
/// `section` comes from `sec_new` or `sec_fork`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sec_fork(section: *mut Section) -> *mut Section {
    Box::into_raw(Box::new(unsafe { &mut *section }.fork()))
}

/// # Safety
/// Both come from `sec_new` or `sec_fork` and differ.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sec_merge(into: *mut Section, from: *mut Section) {
    unsafe { &mut *into }.merge(unsafe { &mut *from });
}

/// Applies a JSON array of ops; returns a JSON array of results, or `{"error": …}`.
///
/// # Safety
/// `section` comes from `sec_new` or `sec_fork`; `ops` is a NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sec_apply(section: *mut Section, ops: *const c_char) -> *mut c_char {
    let section = unsafe { &mut *section };
    let ops: Json = match serde_json::from_slice(unsafe { CStr::from_ptr(ops) }.to_bytes()) {
        Ok(ops) => ops,
        Err(e) => return into_c(json!({ "error": e.to_string() })),
    };
    let mut results = Vec::new();
    for op in ops.as_array().into_iter().flatten() {
        match section.apply(op) {
            Ok(result) => results.push(result),
            Err(error) => return into_c(json!({ "error": error, "op": op })),
        }
    }
    into_c(Json::Array(results))
}

/// # Safety
/// `section` comes from `sec_new` or `sec_fork`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sec_spans(section: *mut Section) -> *mut c_char {
    into_c(unsafe { &*section }.spans())
}

/// # Safety
/// `s` comes from this library and is freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sec_str_free(s: *mut c_char) {
    drop(unsafe { CString::from_raw(s) });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(section: &Section) -> Vec<String> {
        let mut paragraphs: Vec<String> = Vec::new();
        for span in section.spans().as_array().unwrap() {
            if span.get("block").is_some() {
                paragraphs.push(String::new());
            } else {
                paragraphs
                    .last_mut()
                    .unwrap()
                    .push_str(span["text"].as_str().unwrap());
            }
        }
        paragraphs
    }

    fn op(section: &mut Section, op: Json) -> Json {
        section.apply(&op).unwrap()
    }

    #[test]
    fn a_concurrent_edit_stays_in_its_paragraph_when_another_device_splits() {
        let mut a = Section::new();
        op(
            &mut a,
            json!({"op": "splice", "at": 1, "ins": "Hello world"}),
        );
        let mut b = a.fork();
        op(&mut a, json!({"op": "split", "at": 6, "type": "paragraph"}));
        op(
            &mut b,
            json!({"op": "splice", "at": 7, "del": 1, "ins": "W"}),
        );
        op(&mut b, json!({"op": "splice", "at": 12, "ins": "!"}));
        a.merge(&mut b);
        assert_eq!(text_of(&a), ["Hello", " World!"]);
    }

    #[test]
    fn a_concurrent_edit_survives_when_another_device_joins() {
        let mut a = Section::new();
        op(&mut a, json!({"op": "splice", "at": 1, "ins": "First."}));
        op(&mut a, json!({"op": "split", "at": 7, "type": "paragraph"}));
        op(&mut a, json!({"op": "splice", "at": 8, "ins": " Second."}));
        let mut b = a.fork();
        op(&mut a, json!({"op": "splice", "at": 7, "del": 1}));
        op(&mut b, json!({"op": "splice", "at": 16, "ins": " Third."}));
        a.merge(&mut b);
        assert_eq!(text_of(&a), ["First. Second. Third."]);
    }

    #[test]
    fn a_split_keeps_the_marker_of_the_upper_paragraph_and_names_the_new_one() {
        let mut a = Section::new();
        let first = a.spans()[0]["block"].clone();
        op(&mut a, json!({"op": "splice", "at": 1, "ins": "ab"}));
        let id = op(&mut a, json!({"op": "split", "at": 2, "type": "heading1"}));
        let spans = a.spans();
        assert_eq!(spans[0]["block"], first);
        assert_eq!(spans[2]["block"], id);
        assert_eq!(spans[2]["type"], "heading1");
    }

    #[test]
    fn bold_grows_at_its_end_and_a_link_does_not() {
        let mut a = Section::new();
        op(&mut a, json!({"op": "splice", "at": 1, "ins": "bold link"}));
        op(
            &mut a,
            json!({"op": "mark", "start": 1, "end": 5, "name": "bold"}),
        );
        op(
            &mut a,
            json!({"op": "mark", "start": 6, "end": 10, "name": "link", "value": "https://example.com"}),
        );
        op(&mut a, json!({"op": "splice", "at": 10, "ins": "!"}));
        op(&mut a, json!({"op": "splice", "at": 5, "ins": "er"}));
        let spans = a.spans();
        let runs: Vec<(String, Json)> = spans.as_array().unwrap()[1..]
            .iter()
            .map(|s| (s["text"].as_str().unwrap().to_string(), s["marks"].clone()))
            .collect();
        assert_eq!(runs[0], ("bolder".into(), json!({"bold": true})));
        assert_eq!(runs[1], (" ".into(), json!({})));
        assert_eq!(
            runs[2],
            ("link".into(), json!({"link": "https://example.com"}))
        );
        assert_eq!(runs[3], ("!".into(), json!({})));
    }

    #[test]
    fn utf16_offsets_match_dart_strings() {
        let mut a = Section::new();
        op(&mut a, json!({"op": "splice", "at": 1, "ins": "😀x"}));
        op(&mut a, json!({"op": "split", "at": 3, "type": "paragraph"}));
        assert_eq!(text_of(&a), ["😀", "x"]);
    }
}

#[cfg(test)]
mod replay {
    use super::*;

    #[test]
    #[ignore = "replays a dumped op log from SECTION_REPLAY"]
    fn replay_a_dumped_log() {
        let path = std::env::var("SECTION_REPLAY").unwrap();
        let log: Json = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut section = Section::new();
        let mut at = 1;
        for (i, text) in ["alpha beta", "gamma delta", "epsilon"].iter().enumerate() {
            if i > 0 {
                section
                    .apply(&json!({"op": "split", "at": at, "type": "paragraph"}))
                    .unwrap();
                at += 1;
            }
            section
                .apply(&json!({"op": "splice", "at": at, "ins": text}))
                .unwrap();
            at += width(text);
        }
        let steps = log.as_array().unwrap();
        for (n, step) in steps.iter().enumerate() {
            for op in step.as_array().unwrap() {
                section.apply(op).unwrap();
            }
            if n + 3 >= steps.len() {
                println!("after step {n}: {step}\n  {}", section.spans());
            }
        }
    }
}
