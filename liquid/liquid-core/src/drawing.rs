use std::collections::HashMap;
use std::ops::Range;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, ScalarValue, ScalarValueRef, ValueRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::{self, BLOCKS, DELETED, KIND, SORT, STYLE};
use crate::{BlockId, BlockKind, ElementId, Error, Place};

const ELEMENTS: &str = "elements";
const X: &str = "x";
const Y: &str = "y";
const W: &str = "w";
const H: &str = "h";
const ROTATION: &str = "rotation";
const COLOR: &str = "color";
const WIDTH: &str = "width";
const PEN: &str = "pen";
const POINTS: &str = "points";
const COUNT: &str = "count";
const SHAPE: &str = "shape";
const SEED: &str = "seed";
const ARROWHEAD: &str = "arrowhead";
const CORNERS: &str = "corners";
const TEXT: &str = "text";
const DOC_PAGE: &str = "doc-page";

pub const MAX_ELEMENTS: usize = 10_000;
pub const MAX_DOC_PAGES: u32 = 2_000;
pub const MAX_POINTS: usize = 10_000;
pub const MAX_BOARD_POINTS: usize = 1_000_000;
pub const MAX_DRAWING_ELEMENTS: usize = 10 * MAX_ELEMENTS;
pub const MAX_DRAWING_POINTS: usize = 10 * MAX_BOARD_POINTS;
pub const MAX_ELEMENT_TEXT: usize = 10_000;
const MAX_COORDINATE: f64 = 1e7;
const POINT: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrawingStyle {
    HandDrawn,
    Clean,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drawing {
    pub style: DrawingStyle,
    pub elements: Vec<(ElementId, Element)>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
}

impl PartialEq for Frame {
    fn eq(&self, other: &Self) -> bool {
        self.values()
            .iter()
            .zip(other.values())
            .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

impl Eq for Frame {}

impl Frame {
    fn values(&self) -> [f64; 5] {
        [self.x, self.y, self.width, self.height, self.rotation]
    }

    fn valid(&self) -> bool {
        self.values()
            .iter()
            .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InkColor {
    Text,
    Data1,
    Data2,
    Data3,
    Yellow,
    Green,
    Blue,
    Pink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pen {
    Ball,
    Fountain,
    Brush,
    Highlighter,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Shape {
    Line { arrowhead: bool },
    Rectangle,
    Ellipse,
    Triangle { corners: [f64; 6] },
}

impl PartialEq for Shape {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Line { arrowhead: a }, Self::Line { arrowhead: b }) => a == b,
            (Self::Rectangle, Self::Rectangle) | (Self::Ellipse, Self::Ellipse) => true,
            (Self::Triangle { corners: a }, Self::Triangle { corners: b }) => {
                a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
            }
            _ => false,
        }
    }
}

impl Eq for Shape {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Content {
    Stroke {
        pen: Pen,
        #[serde(serialize_with = "hex", deserialize_with = "unhex")]
        points: Vec<u8>,
    },
    Shape {
        shape: Shape,
        seed: u32,
    },
    Text {
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Element {
    pub frame: Frame,
    pub color: InkColor,
    pub width: u8,
    #[serde(default)]
    pub doc_page: u32,
    pub content: Content,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementChange {
    pub id: ElementId,
    pub frame: Frame,
    pub color: InkColor,
    pub width: u8,
    pub text: Option<String>,
    pub arrowhead: Option<bool>,
}

fn hex<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    let text: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    serializer.serialize_str(&text)
}

fn unhex<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let text = String::deserialize(deserializer)?;
    (0..text.len())
        .step_by(2)
        .map(|at| {
            text.get(at..at + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| serde::de::Error::custom("points are no hex"))
        })
        .collect()
}

fn style_name(style: DrawingStyle) -> &'static str {
    match style {
        DrawingStyle::HandDrawn => "hand-drawn",
        DrawingStyle::Clean => "clean",
    }
}

fn color_name(color: InkColor) -> &'static str {
    match color {
        InkColor::Text => "text",
        InkColor::Data1 => "data-1",
        InkColor::Data2 => "data-2",
        InkColor::Data3 => "data-3",
        InkColor::Yellow => "yellow",
        InkColor::Green => "green",
        InkColor::Blue => "blue",
        InkColor::Pink => "pink",
    }
}

fn pen_name(pen: Pen) -> &'static str {
    match pen {
        Pen::Ball => "ball",
        Pen::Fountain => "fountain",
        Pen::Brush => "brush",
        Pen::Highlighter => "highlighter",
    }
}

fn shape_name(shape: &Shape) -> &'static str {
    match shape {
        Shape::Line { .. } => "line",
        Shape::Rectangle => "rectangle",
        Shape::Ellipse => "ellipse",
        Shape::Triangle { .. } => "triangle",
    }
}

pub(crate) fn create(doc: &mut AutoCommit, block: &BlockId) -> Result<(), Error> {
    let entry = model::insert(
        doc,
        BLOCKS,
        block.as_str(),
        &[
            (KIND, "whiteboard"),
            (STYLE, style_name(DrawingStyle::HandDrawn)),
        ],
    )?;
    doc.put_object(&entry, ELEMENTS, ObjType::Map)?;
    Ok(())
}

fn holder(doc: &AutoCommit, block: &BlockId) -> Result<(ObjId, bool), Error> {
    if let Some(entry) = model::of_kind(doc, block.as_str(), BlockKind::Whiteboard) {
        return Ok((entry, false));
    }
    let entry = model::of_kind(doc, block.as_str(), BlockKind::Media).ok_or(Error::NotFound)?;
    Ok((entry, true))
}

fn elements_for_writing(doc: &mut AutoCommit, block: &BlockId) -> Result<(ObjId, ObjId), Error> {
    let (entry, _) = holder(doc, block)?;
    let elements = match model::child(doc, &entry, ELEMENTS) {
        Some(elements) => elements,
        None => doc.put_object(&entry, ELEMENTS, ObjType::Map)?,
    };
    Ok((entry, elements))
}

pub(crate) fn set_style(
    doc: &mut AutoCommit,
    block: &BlockId,
    style: DrawingStyle,
) -> Result<(), Error> {
    let (entry, _) = holder(doc, block)?;
    doc.put(&entry, STYLE, style_name(style))?;
    Ok(())
}

pub(crate) fn read(
    doc: &AutoCommit,
    block: &BlockId,
    pages: Range<u32>,
) -> Result<Option<Drawing>, Error> {
    let Ok((entry, _)) = holder(doc, block) else {
        return Ok(None);
    };
    let style = match model::string(doc, &entry, STYLE).as_deref() {
        Some("clean") => DrawingStyle::Clean,
        _ => DrawingStyle::HandDrawn,
    };
    let Some(elements) = model::child(doc, &entry, ELEMENTS) else {
        return Ok(Some(Drawing {
            style,
            elements: Vec::new(),
        }));
    };
    let shown: Vec<Stored> = Stored::all(doc, &elements)
        .into_iter()
        .filter(|stored| pages.contains(&stored.doc_page()))
        .collect();
    let mut per_page: Vec<(u32, usize)> = Vec::new();
    for stored in &shown {
        match per_page
            .iter_mut()
            .find(|(page, _)| *page == stored.doc_page())
        {
            Some((_, points)) => *points += stored.count(),
            None => per_page.push((stored.doc_page(), stored.count())),
        }
    }
    per_page.sort_unstable();
    let mut budget = MAX_BOARD_POINTS;
    let mut last = None;
    for (page, points) in per_page {
        if points > budget {
            break;
        }
        budget -= points;
        last = Some(page);
    }
    let mut read: Vec<(Option<String>, ElementId, Element)> = shown
        .into_iter()
        .filter(|stored| last.is_some_and(|last| stored.doc_page() <= last))
        .filter_map(|stored| {
            let element = stored.element()?;
            Some((stored.sort, ElementId::stored(&stored.id), element))
        })
        .collect();
    read.sort_by(|a, b| (usable(&a.0), &a.1).cmp(&(usable(&b.0), &b.1)));
    let mut elements_read: Vec<(ElementId, Element)> = read
        .into_iter()
        .map(|(_, id, element)| (id, element))
        .collect();
    for (id, element) in &mut elements_read {
        if let Content::Stroke { points, .. } = &mut element.content {
            let obj = model::child(doc, &elements, id.as_str()).ok_or(Error::NotFound)?;
            if let Some((automerge::Value::Scalar(value), _)) = doc.get(&obj, POINTS)?
                && let ScalarValue::Bytes(bytes) = value.as_ref()
            {
                points.clone_from(bytes);
            }
        }
    }
    elements_read.retain(|(_, element)| match &element.content {
        Content::Stroke { points, .. } => !points.is_empty() && points.len() % POINT == 0,
        _ => true,
    });
    Ok(Some(Drawing {
        style,
        elements: elements_read,
    }))
}

fn usable(sort: &Option<String>) -> Option<&str> {
    crate::sort_key::usable(sort.as_deref())
}

pub(crate) fn edit(
    doc: &mut AutoCommit,
    block: &BlockId,
    add: &[Element],
    change: &[ElementChange],
    remove: &[ElementId],
) -> Result<Vec<ElementId>, Error> {
    let (_, paged) = holder(doc, block)?;
    for element in add {
        check(element)?;
        let last = if paged { MAX_DOC_PAGES - 1 } else { 0 };
        if element.doc_page > last {
            return Err(Error::WrongKind);
        }
    }
    for changed in change {
        if !changed.frame.valid()
            || changed.width > 2
            || changed
                .text
                .as_ref()
                .is_some_and(|text| text.chars().count() > MAX_ELEMENT_TEXT)
        {
            return Err(Error::WrongKind);
        }
    }
    let (_, elements) = elements_for_writing(doc, block)?;
    let mut live = Stored::all(doc, &elements);
    for id in remove {
        let at = live
            .iter()
            .position(|stored| stored.id == id.as_str())
            .ok_or(Error::NotFound)?;
        let gone = live.remove(at);
        model::mark_deleted(doc, &gone.obj)?;
    }
    for changed in change {
        let stored = live
            .iter()
            .find(|stored| stored.id == changed.id.as_str())
            .ok_or(Error::NotFound)?;
        let kind = stored.text(KIND);
        if changed.text.is_some() && kind != Some(TEXT) {
            return Err(Error::WrongKind);
        }
        if changed.arrowhead.is_some() && stored.text(SHAPE) != Some("line") {
            return Err(Error::WrongKind);
        }
        let obj = stored.obj.clone();
        put_frame(doc, &obj, &changed.frame)?;
        doc.put(&obj, COLOR, color_name(changed.color))?;
        doc.put(&obj, WIDTH, u64::from(changed.width))?;
        if let Some(text) = &changed.text {
            doc.put(&obj, TEXT, text.as_str())?;
        }
        if let Some(arrowhead) = changed.arrowhead {
            doc.put(&obj, ARROWHEAD, arrowhead)?;
        }
    }
    let mut pages: HashMap<u32, (usize, usize)> = HashMap::new();
    let mut all = (0, 0);
    for stored in &live {
        let held = pages.entry(stored.doc_page()).or_default();
        held.0 += 1;
        held.1 += stored.count();
        all.0 += 1;
        all.1 += stored.count();
    }
    for element in add {
        let points = point_count(element);
        let held = pages.entry(element.doc_page).or_default();
        held.0 += 1;
        held.1 += points;
        all.0 += 1;
        all.1 += points;
        if held.0 > MAX_ELEMENTS
            || held.1 > MAX_BOARD_POINTS
            || all.0 > MAX_DRAWING_ELEMENTS
            || all.1 > MAX_DRAWING_POINTS
        {
            return Err(Error::TooMany);
        }
    }
    let mut last = live
        .iter()
        .filter_map(|stored| crate::sort_key::usable(stored.sort.as_deref()))
        .max()
        .map(str::to_string);
    let mut ids = Vec::with_capacity(add.len());
    for element in add {
        let id = ElementId::new()?;
        let below = last
            .iter()
            .map(|sort| ("", sort.as_str()))
            .collect::<Vec<_>>();
        let sort = crate::sort_key::at(&below, Place::Last, |item| item.0, |item| Some(item.1))?;
        write(doc, &elements, &id, &sort, element)?;
        last = Some(sort);
        ids.push(id);
    }
    Ok(ids)
}

fn point_count(element: &Element) -> usize {
    match &element.content {
        Content::Stroke { points, .. } => points.len() / POINT,
        _ => 0,
    }
}

fn check(element: &Element) -> Result<(), Error> {
    if !element.frame.valid() || element.width > 2 {
        return Err(Error::WrongKind);
    }
    match &element.content {
        Content::Stroke { points, .. } => {
            if points.is_empty() || points.len() % POINT != 0 {
                return Err(Error::WrongKind);
            }
            if points.len() / POINT > MAX_POINTS {
                return Err(Error::TooMany);
            }
            let finite = points
                .as_chunks::<4>()
                .0
                .iter()
                .all(|value| f32::from_le_bytes(*value).is_finite());
            if !finite {
                return Err(Error::WrongKind);
            }
        }
        Content::Shape {
            shape: Shape::Triangle { corners },
            ..
        } => {
            if !corners.iter().all(|corner| (0.0..=1.0).contains(corner)) {
                return Err(Error::WrongKind);
            }
        }
        Content::Shape { .. } => {}
        Content::Text { text } => {
            if text.chars().count() > MAX_ELEMENT_TEXT {
                return Err(Error::WrongKind);
            }
        }
    }
    Ok(())
}

fn put_frame(doc: &mut AutoCommit, obj: &ObjId, frame: &Frame) -> Result<(), Error> {
    for (key, value) in [X, Y, W, H, ROTATION].into_iter().zip(frame.values()) {
        doc.put(obj, key, value + 0.0)?;
    }
    Ok(())
}

fn write(
    doc: &mut AutoCommit,
    elements: &ObjId,
    id: &ElementId,
    sort: &str,
    element: &Element,
) -> Result<(), Error> {
    let obj = doc.put_object(elements, id.as_str(), ObjType::Map)?;
    doc.put(&obj, SORT, sort)?;
    doc.put(&obj, DELETED, false)?;
    put_frame(doc, &obj, &element.frame)?;
    doc.put(&obj, COLOR, color_name(element.color))?;
    doc.put(&obj, WIDTH, u64::from(element.width))?;
    if element.doc_page > 0 {
        doc.put(&obj, DOC_PAGE, u64::from(element.doc_page))?;
    }
    match &element.content {
        Content::Stroke { pen, points } => {
            doc.put(&obj, KIND, "stroke")?;
            doc.put(&obj, PEN, pen_name(*pen))?;
            doc.put(&obj, POINTS, ScalarValue::Bytes(points.clone()))?;
            doc.put(&obj, COUNT, (points.len() / POINT) as u64)?;
        }
        Content::Shape { shape, seed } => {
            doc.put(&obj, KIND, "shape")?;
            doc.put(&obj, SHAPE, shape_name(shape))?;
            doc.put(&obj, SEED, u64::from(*seed))?;
            match shape {
                Shape::Line { arrowhead } => doc.put(&obj, ARROWHEAD, *arrowhead)?,
                Shape::Triangle { corners } => {
                    let bytes = corners.iter().flat_map(|c| c.to_le_bytes()).collect();
                    doc.put(&obj, CORNERS, ScalarValue::Bytes(bytes))?;
                }
                Shape::Rectangle | Shape::Ellipse => {}
            }
        }
        Content::Text { text } => {
            doc.put(&obj, KIND, TEXT)?;
            doc.put(&obj, TEXT, text.as_str())?;
        }
    }
    Ok(())
}

struct Stored {
    id: String,
    obj: ObjId,
    sort: Option<String>,
    texts: Vec<(String, String)>,
    numbers: Vec<(String, f64)>,
    flags: Vec<(String, bool)>,
    bytes: Vec<(String, Vec<u8>)>,
}

impl Stored {
    fn all(doc: &AutoCommit, elements: &ObjId) -> Vec<Stored> {
        doc.map_range(elements, ..)
            .filter(|item| matches!(item.value, ValueRef::Object(ObjType::Map)))
            .filter_map(|item| Self::read(doc, item.key.to_string(), item.id()))
            .collect()
    }

    fn read(doc: &AutoCommit, id: String, obj: ObjId) -> Option<Self> {
        let mut stored = Self {
            id,
            obj,
            sort: None,
            texts: Vec::new(),
            numbers: Vec::new(),
            flags: Vec::new(),
            bytes: Vec::new(),
        };
        let mut live = false;
        for field in doc.map_range(&stored.obj, ..) {
            let key = field.key.to_string();
            match field.value {
                ValueRef::Scalar(ScalarValueRef::Boolean(deleted)) if key == DELETED => {
                    live = !deleted;
                }
                ValueRef::Scalar(ScalarValueRef::Str(text)) if key == SORT => {
                    stored.sort = Some(text.to_string());
                }
                ValueRef::Scalar(ScalarValueRef::Str(text)) => {
                    stored.texts.push((key, text.to_string()));
                }
                ValueRef::Scalar(ScalarValueRef::F64(number)) => stored.numbers.push((key, number)),
                ValueRef::Scalar(ScalarValueRef::Uint(number)) => {
                    stored.numbers.push((key, number as f64));
                }
                ValueRef::Scalar(ScalarValueRef::Boolean(flag)) => stored.flags.push((key, flag)),
                ValueRef::Scalar(ScalarValueRef::Bytes(bytes)) if key == CORNERS => {
                    stored.bytes.push((key, bytes.to_vec()));
                }
                _ => {}
            }
        }
        live.then_some(stored)
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.texts
            .iter()
            .find(|(field, _)| field == key)
            .map(|(_, value)| value.as_str())
    }

    fn number(&self, key: &str) -> Option<f64> {
        self.numbers
            .iter()
            .find(|(field, _)| field == key)
            .map(|(_, value)| *value)
    }

    fn flag(&self, key: &str) -> Option<bool> {
        self.flags
            .iter()
            .find(|(field, _)| field == key)
            .map(|(_, value)| *value)
    }

    fn doc_page(&self) -> u32 {
        self.number(DOC_PAGE).map_or(0, |page| page as u32)
    }

    fn count(&self) -> usize {
        match self.text(KIND) {
            Some("stroke") => self.number(COUNT).map_or(0, |count| count as usize),
            _ => 0,
        }
    }

    fn element(&self) -> Option<Element> {
        let frame = Frame {
            x: self.number(X)?,
            y: self.number(Y)?,
            width: self.number(W)?,
            height: self.number(H)?,
            rotation: self.number(ROTATION)?,
        };
        let color = match self.text(COLOR)? {
            "text" => InkColor::Text,
            "data-1" => InkColor::Data1,
            "data-2" => InkColor::Data2,
            "data-3" => InkColor::Data3,
            "yellow" => InkColor::Yellow,
            "green" => InkColor::Green,
            "blue" => InkColor::Blue,
            "pink" => InkColor::Pink,
            _ => return None,
        };
        let width = self.number(WIDTH).filter(|width| *width <= 2.0)? as u8;
        let content = match self.text(KIND)? {
            "stroke" => Content::Stroke {
                pen: match self.text(PEN)? {
                    "ball" => Pen::Ball,
                    "fountain" => Pen::Fountain,
                    "brush" => Pen::Brush,
                    "highlighter" => Pen::Highlighter,
                    _ => return None,
                },
                points: Vec::new(),
            },
            "shape" => Content::Shape {
                shape: match self.text(SHAPE)? {
                    "line" => Shape::Line {
                        arrowhead: self.flag(ARROWHEAD).unwrap_or(false),
                    },
                    "rectangle" => Shape::Rectangle,
                    "ellipse" => Shape::Ellipse,
                    "triangle" => Shape::Triangle {
                        corners: self.corners()?,
                    },
                    _ => return None,
                },
                seed: self.number(SEED)? as u32,
            },
            TEXT => Content::Text {
                text: self.text(TEXT)?.to_string(),
            },
            _ => return None,
        };
        Some(Element {
            frame,
            color,
            width,
            doc_page: self.doc_page(),
            content,
        })
    }

    fn corners(&self) -> Option<[f64; 6]> {
        let (_, bytes) = self.bytes.iter().find(|(field, _)| field == CORNERS)?;
        let values: Vec<f64> = bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|value| f64::from_le_bytes(*value))
            .collect();
        values.try_into().ok()
    }
}
