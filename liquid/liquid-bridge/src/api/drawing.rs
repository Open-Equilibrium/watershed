//! The parts of the Workspace's messages that carry drawings, a Whiteboard's (ADR-0306) or
//! an annotated Media Block's (ADR-0307): elements, the changes to them, and what a drawing
//! read answers. Like the other messages they are
//! flat, and identities cross as text; a stroke's points cross as bytes, never as numbers.

use liquid_core::{
    Content, Drawing, DrawingStyle, Element, ElementChange, ElementId, Frame, InkColor, Pen, Shape,
};

use super::workspace::id;

/// How a drawing shows its shapes and text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeDrawingStyle {
    /// As Excalidraw draws, text in Excalifont.
    HandDrawn,
    /// Exact lines, text in Liquid's typeface.
    Clean,
}

impl BridgeDrawingStyle {
    pub(crate) fn to_core(self) -> DrawingStyle {
        match self {
            Self::HandDrawn => DrawingStyle::HandDrawn,
            Self::Clean => DrawingStyle::Clean,
        }
    }

    fn of(style: DrawingStyle) -> Self {
        match style {
            DrawingStyle::HandDrawn => Self::HandDrawn,
            DrawingStyle::Clean => Self::Clean,
        }
    }
}

/// What an element is; each kind names the parts it fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeElementKind {
    /// Ink: `pen` and `points`.
    Stroke,
    /// `shape`, with `seed`, `arrowhead` for a line and `corners` for a triangle.
    Shape,
    /// `text`.
    Text,
}

/// The pen a stroke was drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePen {
    /// An even line.
    Ball,
    /// A line that follows the pressure.
    Fountain,
    /// A line that follows the pressure most.
    Brush,
    /// Broad, translucent ink beneath the rest.
    Highlighter,
}

/// A shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeShape {
    /// A line across its frame.
    Line,
    /// A rectangle.
    Rectangle,
    /// An ellipse.
    Ellipse,
    /// A triangle with `corners`.
    Triangle,
}

/// An element's color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeInkColor {
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

/// A stroke, shape or text: where it lies, turned `rotation` radians clockwise around its
/// center, and how it looks.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeElement {
    /// Its identity; none in a new one.
    pub id: Option<String>,
    /// What it is.
    pub kind: BridgeElementKind,
    /// Left.
    pub x: f64,
    /// Top.
    pub y: f64,
    /// Its frame's width; a line's may be negative.
    pub w: f64,
    /// Its frame's height; a line's may be negative.
    pub h: f64,
    /// Rotation in radians, clockwise.
    pub rotation: f64,
    /// Its color.
    pub color: BridgeInkColor,
    /// Its line width, or a text's size: 0, 1 or 2.
    pub width: i64,
    /// The page of an annotated document it lies on, from 0; 0 on a Whiteboard.
    pub doc_page: i64,
    /// A stroke's pen.
    pub pen: BridgePen,
    /// A stroke's points: x, y and pressure as little-endian f32, 12 bytes a point.
    pub points: Vec<u8>,
    /// A shape.
    pub shape: BridgeShape,
    /// The seed of a shape's hand-drawn lines.
    pub seed: i64,
    /// Whether a line ends in an arrowhead.
    pub arrowhead: bool,
    /// A triangle's corners: x, y of each, from 0 to 1 of its frame.
    pub corners: Vec<f64>,
    /// A text element's text.
    pub text: String,
}

/// A change to a live element: its frame and look, a text's text, a line's arrowhead.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeElementChange {
    /// The element.
    pub id: String,
    /// Left.
    pub x: f64,
    /// Top.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
    /// Rotation in radians, clockwise.
    pub rotation: f64,
    /// Its color.
    pub color: BridgeInkColor,
    /// Its line width or text size.
    pub width: i64,
    /// A text element's new text; none keeps it.
    pub text: Option<String>,
    /// A line's arrowhead; none keeps it.
    pub arrowhead: Option<bool>,
}

/// Elements removed, changed and added in one Action.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BridgeDrawingEdit {
    /// New elements, each above all before.
    pub add: Vec<BridgeElement>,
    /// Changes to live elements.
    pub change: Vec<BridgeElementChange>,
    /// Elements to remove.
    pub remove: Vec<String>,
}

/// A drawing's style and elements, bottom to top.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeDrawing {
    /// How it draws its shapes and text.
    pub style: BridgeDrawingStyle,
    /// Its elements, bottom to top.
    pub elements: Vec<BridgeElement>,
}

impl BridgeDrawing {
    pub(crate) fn of(board: Drawing) -> Self {
        Self {
            style: BridgeDrawingStyle::of(board.style),
            elements: board
                .elements
                .into_iter()
                .map(|(id, element)| BridgeElement::of(id, element))
                .collect(),
        }
    }
}

fn color(color: BridgeInkColor) -> InkColor {
    match color {
        BridgeInkColor::Text => InkColor::Text,
        BridgeInkColor::Data1 => InkColor::Data1,
        BridgeInkColor::Data2 => InkColor::Data2,
        BridgeInkColor::Data3 => InkColor::Data3,
        BridgeInkColor::Yellow => InkColor::Yellow,
        BridgeInkColor::Green => InkColor::Green,
        BridgeInkColor::Blue => InkColor::Blue,
        BridgeInkColor::Pink => InkColor::Pink,
    }
}

fn width(width: i64) -> Result<u8, String> {
    u8::try_from(width).map_err(|_| "a width is 0, 1 or 2".to_string())
}

impl BridgeElement {
    fn of(id: ElementId, element: Element) -> Self {
        let mut read = Self {
            id: Some(id.to_string()),
            kind: BridgeElementKind::Text,
            x: element.frame.x,
            y: element.frame.y,
            w: element.frame.width,
            h: element.frame.height,
            rotation: element.frame.rotation,
            color: match element.color {
                InkColor::Text => BridgeInkColor::Text,
                InkColor::Data1 => BridgeInkColor::Data1,
                InkColor::Data2 => BridgeInkColor::Data2,
                InkColor::Data3 => BridgeInkColor::Data3,
                InkColor::Yellow => BridgeInkColor::Yellow,
                InkColor::Green => BridgeInkColor::Green,
                InkColor::Blue => BridgeInkColor::Blue,
                InkColor::Pink => BridgeInkColor::Pink,
            },
            width: i64::from(element.width),
            doc_page: i64::from(element.doc_page),
            pen: BridgePen::Ball,
            points: Vec::new(),
            shape: BridgeShape::Rectangle,
            seed: 0,
            arrowhead: false,
            corners: Vec::new(),
            text: String::new(),
        };
        match element.content {
            Content::Stroke { pen, points } => {
                read.kind = BridgeElementKind::Stroke;
                read.pen = match pen {
                    Pen::Ball => BridgePen::Ball,
                    Pen::Fountain => BridgePen::Fountain,
                    Pen::Brush => BridgePen::Brush,
                    Pen::Highlighter => BridgePen::Highlighter,
                };
                read.points = points;
            }
            Content::Shape { shape, seed } => {
                read.kind = BridgeElementKind::Shape;
                read.seed = i64::from(seed);
                read.shape = match shape {
                    Shape::Line { arrowhead } => {
                        read.arrowhead = arrowhead;
                        BridgeShape::Line
                    }
                    Shape::Rectangle => BridgeShape::Rectangle,
                    Shape::Ellipse => BridgeShape::Ellipse,
                    Shape::Triangle { corners } => {
                        read.corners = corners.to_vec();
                        BridgeShape::Triangle
                    }
                };
            }
            Content::Text { text } => read.text = text,
        }
        read
    }

    fn to_core(&self) -> Result<Element, String> {
        let content = match self.kind {
            BridgeElementKind::Stroke => Content::Stroke {
                pen: match self.pen {
                    BridgePen::Ball => Pen::Ball,
                    BridgePen::Fountain => Pen::Fountain,
                    BridgePen::Brush => Pen::Brush,
                    BridgePen::Highlighter => Pen::Highlighter,
                },
                points: self.points.clone(),
            },
            BridgeElementKind::Shape => Content::Shape {
                shape: match self.shape {
                    BridgeShape::Line => Shape::Line {
                        arrowhead: self.arrowhead,
                    },
                    BridgeShape::Rectangle => Shape::Rectangle,
                    BridgeShape::Ellipse => Shape::Ellipse,
                    BridgeShape::Triangle => Shape::Triangle {
                        corners: self
                            .corners
                            .clone()
                            .try_into()
                            .map_err(|_| "a triangle has six corner values".to_string())?,
                    },
                },
                seed: u32::try_from(self.seed).map_err(|_| "a seed is a u32".to_string())?,
            },
            BridgeElementKind::Text => Content::Text {
                text: self.text.clone(),
            },
        };
        Ok(Element {
            frame: Frame {
                x: self.x,
                y: self.y,
                width: self.w,
                height: self.h,
                rotation: self.rotation,
            },
            color: color(self.color),
            width: width(self.width)?,
            doc_page: u32::try_from(self.doc_page)
                .map_err(|_| "a document page is from 0".to_string())?,
            content,
        })
    }
}

/// An edit's added, changed and removed elements, as the core takes them.
type Edit = (Vec<Element>, Vec<ElementChange>, Vec<ElementId>);

impl BridgeDrawingEdit {
    /// The core's parts of the edit.
    pub(crate) fn to_core(&self) -> Result<Edit, String> {
        let add = self
            .add
            .iter()
            .map(BridgeElement::to_core)
            .collect::<Result<_, _>>()?;
        let change = self
            .change
            .iter()
            .map(|changed| {
                Ok(ElementChange {
                    id: id(&Some(changed.id.clone()), ElementId::parse, "element")?,
                    frame: Frame {
                        x: changed.x,
                        y: changed.y,
                        width: changed.w,
                        height: changed.h,
                        rotation: changed.rotation,
                    },
                    color: color(changed.color),
                    width: width(changed.width)?,
                    text: changed.text.clone(),
                    arrowhead: changed.arrowhead,
                })
            })
            .collect::<Result<_, String>>()?;
        let remove = self
            .remove
            .iter()
            .map(|removed| id(&Some(removed.clone()), ElementId::parse, "element"))
            .collect::<Result<_, _>>()?;
        Ok((add, change, remove))
    }
}
