use liquid_core::{
    Content, Drawing, DrawingStyle, Element, ElementChange, ElementId, Frame, InkColor, Pen, Shape,
};

use super::workspace::id;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeDrawingStyle {
    HandDrawn,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeElementKind {
    Stroke,
    Shape,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePen {
    Ball,
    Fountain,
    Brush,
    Highlighter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeShape {
    Line,
    Rectangle,
    Ellipse,
    Triangle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeInkColor {
    Text,
    Data1,
    Data2,
    Data3,
    Yellow,
    Green,
    Blue,
    Pink,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeElement {
    pub id: Option<String>,
    pub kind: BridgeElementKind,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rotation: f64,
    pub color: BridgeInkColor,
    pub width: i64,
    pub doc_page: i64,
    pub pen: BridgePen,
    pub points: Vec<u8>,
    pub shape: BridgeShape,
    pub seed: i64,
    pub arrowhead: bool,
    pub corners: Vec<f64>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeElementChange {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rotation: f64,
    pub color: BridgeInkColor,
    pub width: i64,
    pub text: Option<String>,
    pub arrowhead: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BridgeDrawingEdit {
    pub add: Vec<BridgeElement>,
    pub change: Vec<BridgeElementChange>,
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeDrawing {
    pub style: BridgeDrawingStyle,
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

type Edit = (Vec<Element>, Vec<ElementChange>, Vec<ElementId>);

impl BridgeDrawingEdit {
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
