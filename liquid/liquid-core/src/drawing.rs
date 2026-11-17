//! Drawings: the pen strokes, shapes and text of a Whiteboard Block (ADR-0306) or of an
//! annotated Media Block's pages (ADR-0307), each an element of the Block's entry in its
//! home Page's document, so edits on two devices merge per element. A stroke's points never
//! change once drawn; erasing part of it replaces it by its pieces.
//!
//! ```text
//! blocks: { <block>: { kind: "whiteboard" or "media", style, deleted,
//!                      elements: { <element>: { sort, deleted, kind, x, y, w, h, rotation,
//!                                               color, width, doc-page?,
//!                                               (stroke) pen, points, count,
//!                                               (shape) shape, seed, arrowhead?, corners?,
//!                                               (text) text } } } }
//! ```

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

/// The most live elements a Whiteboard, or one page of an annotated document, holds.
pub const MAX_ELEMENTS: usize = 10_000;
/// The most pages of a document that take ink.
pub const MAX_DOC_PAGES: u32 = 2_000;
/// The most points one stroke holds.
pub const MAX_POINTS: usize = 10_000;
/// The most points all of a Whiteboard's, or one document page's, live strokes hold
/// together, so a board or a page reads and crosses the bridge bounded, at 12 bytes a point
/// (client architecture rule 3).
pub const MAX_BOARD_POINTS: usize = 1_000_000;
/// The most characters one text element holds.
pub const MAX_ELEMENT_TEXT: usize = 10_000;
/// The farthest a coordinate lies from the board's origin.
const MAX_COORDINATE: f64 = 1e7;
/// A stored point: x, y and pressure as little-endian f32.
const POINT: usize = 12;

/// How a drawing shows its shapes and text; one setting for the whole Block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrawingStyle {
    /// As Excalidraw draws at its Bold width and Artist sloppiness, text in Excalifont.
    HandDrawn,
    /// Exact lines, text in Liquid's typeface.
    Clean,
}

/// A Block's drawing, or the part of it on the pages read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drawing {
    /// How it draws its shapes and text.
    pub style: DrawingStyle,
    /// Its live elements, bottom to top.
    pub elements: Vec<(ElementId, Element)>,
}

/// Where an element lies on the board: its box, turned by `rotation` radians clockwise
/// around its center. A line runs from (x, y) to (x + width, y + height), so its width and
/// height may be negative; a stroke's points, as drawn, are stretched to fill the box.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Frame {
    /// Left.
    pub x: f64,
    /// Top.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
    /// Rotation in radians, clockwise.
    pub rotation: f64,
}

// Numbers compare by their bits, so every frame equals itself.
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

/// An element's color, one of the design tokens' text and data colors, so a board reads in
/// both themes, or for the highlighter one of their light tag colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InkColor {
    /// The text color.
    Text,
    /// Data color 1.
    Data1,
    /// Data color 2.
    Data2,
    /// Data color 3.
    Data3,
    /// The yellow tag color.
    Yellow,
    /// The green tag color.
    Green,
    /// The blue tag color.
    Blue,
    /// The pink tag color.
    Pink,
}

/// The pen a stroke was drawn with, as GoodNotes offers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pen {
    /// An even line.
    Ball,
    /// A line whose width follows the pressure.
    Fountain,
    /// A line that follows the pressure most.
    Brush,
    /// Broad, translucent ink beneath the rest, as GoodNotes' highlighter.
    Highlighter,
}

/// A shape.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Shape {
    /// A straight line across its frame, with an arrowhead at its end or none.
    Line {
        /// Whether it ends in an arrowhead.
        arrowhead: bool,
    },
    /// A rectangle filling its frame.
    Rectangle,
    /// An ellipse filling its frame.
    Ellipse,
    /// A triangle whose corners lie in its frame: x, y of each, from 0 to 1 of its width
    /// and height.
    Triangle {
        /// The corners.
        corners: [f64; 6],
    },
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

/// What an element is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Content {
    /// Pen ink.
    Stroke {
        /// The pen.
        pen: Pen,
        /// Its points as drawn, 12 bytes each: x, y and pressure as little-endian f32.
        #[serde(serialize_with = "hex", deserialize_with = "unhex")]
        points: Vec<u8>,
    },
    /// A shape, drawn hand-drawn with `seed`'s wobble, the same on every device.
    Shape {
        /// The shape.
        shape: Shape,
        /// The seed of its hand-drawn lines.
        seed: u32,
    },
    /// A text box; its width wraps the text.
    Text {
        /// Its text.
        text: String,
    },
}

/// A stroke, shape or text on a Whiteboard or a document's page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Element {
    /// Where it lies, in its page's points.
    pub frame: Frame,
    /// Its color.
    pub color: InkColor,
    /// Its line width, or a text's size: 0, 1 or 2 for thin, medium and bold or S, M and L.
    pub width: u8,
    /// The page of an annotated document it lies on, from 0; always 0 on a Whiteboard.
    #[serde(default)]
    pub doc_page: u32,
    /// What it is.
    pub content: Content,
}

/// A change to a live element: where it lies and how it looks. A stroke's points never
/// change; a text element's text may, and a line's arrowhead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementChange {
    /// The element.
    pub id: ElementId,
    /// Where it lies now.
    pub frame: Frame,
    /// Its color now.
    pub color: InkColor,
    /// Its width or size now.
    pub width: u8,
    /// A text element's new text; none keeps it.
    pub text: Option<String>,
    /// A line's arrowhead now; none keeps it.
    pub arrowhead: Option<bool>,
}

/// Points travel in Actions as hex, half the size of a JSON array of bytes.
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

/// Adds an empty, hand-drawn Whiteboard's entry.
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

/// A Block that holds a drawing: a Whiteboard, or a Media Block, whose file's pages take
/// ink; and whether it has pages.
fn holder(doc: &AutoCommit, block: &BlockId) -> Result<(ObjId, bool), Error> {
    if let Some(entry) = model::of_kind(doc, block.as_str(), BlockKind::Whiteboard) {
        return Ok((entry, false));
    }
    let entry = model::of_kind(doc, block.as_str(), BlockKind::Media).ok_or(Error::NotFound)?;
    Ok((entry, true))
}

/// The drawing's elements, made on a Media Block's first annotation.
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

/// A live drawing's style and its elements on the document pages `pages` (a Whiteboard's
/// are all on page 0); none if `block` holds no drawing. An element this version cannot
/// read, as a later one may write, is left out.
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
    let mut read: Vec<(Option<String>, ElementId, Element)> = Stored::all(doc, &elements)
        .into_iter()
        .filter(|stored| pages.contains(&stored.doc_page()))
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
    // Points, the bulk of a board, are read only here, so checks never copy ink.
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
    // A stroke whose points are no points, as no version of the core writes, is left out.
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

/// Adds, changes and removes elements as one write, after checking every one of them;
/// answers the new elements' identities in the order of `add`, each above all before.
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
    // Each page holds a board's caps.
    let mut pages: HashMap<u32, (usize, usize)> = HashMap::new();
    for stored in &live {
        let held = pages.entry(stored.doc_page()).or_default();
        held.0 += 1;
        held.1 += stored.count();
    }
    for element in add {
        let held = pages.entry(element.doc_page).or_default();
        held.0 += 1;
        held.1 += point_count(element);
        if held.0 > MAX_ELEMENTS || held.1 > MAX_BOARD_POINTS {
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

/// Refuses an element no drawing can hold.
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
    // Zero is one value: Automerge stores -0 as 0.
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

/// One live element's stored values, read in one pass over its map.
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
    /// The live elements of `elements`, in no order, without a stroke's points.
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
                // Points are the bulk of a board; only the reader of the element takes them.
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

    /// The document page it lies on.
    fn doc_page(&self) -> u32 {
        self.number(DOC_PAGE).map_or(0, |page| page as u32)
    }

    /// How many points it holds, if it is a stroke.
    fn count(&self) -> usize {
        match self.text(KIND) {
            Some("stroke") => self.number(COUNT).map_or(0, |count| count as usize),
            _ => 0,
        }
    }

    /// The element, if this version reads it.
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
