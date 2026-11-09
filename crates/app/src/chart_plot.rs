//! A chart feature drawn with GPUI Kit Plot. Cards and wires are not plotted.

use gpui_kit::base::plot::scale::{Scale, ScaleBand, ScaleLinear};
use gpui_kit::base::plot::shape::{Arc, Area, Bar, Line, Pie};
use gpui_kit::base::plot::{Plot, PlotElement};
use gpui_kit::{App, Bounds, ElementId, Hsla, IntoElement, Pixels, Window, point, px};

#[derive(Clone)]
pub struct Datum {
    pub x: String,
    pub y: f32,
}

pub struct FeaturePlot {
    pub mark: String,
    pub data: Vec<Datum>,
    pub color: Hsla,
    pub reduced_motion: bool,
}

impl FeaturePlot {
    fn numbers(&self) -> Vec<(f32, f32)> {
        self.data
            .iter()
            .enumerate()
            .map(|(index, datum)| (index as f32, datum.y))
            .collect()
    }
}

impl Plot for FeaturePlot {
    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Name("feature-plot".into()))
    }

    fn appear_generation(&self) -> Option<u64> {
        if self.reduced_motion { None } else { Some(1) }
    }

    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let width = bounds.size.width.as_f32().max(1.0);
        let height = bounds.size.height.as_f32().max(1.0);
        let color = self.color;
        match self.mark.as_str() {
            "bar" | "histogram" => {
                let labels: Vec<String> = self.data.iter().map(|datum| datum.x.clone()).collect();
                let max = self
                    .data
                    .iter()
                    .map(|datum| datum.y)
                    .fold(0.0_f32, f32::max)
                    .max(1.0);
                let x = ScaleBand::new(labels.clone(), [0.0, width]).padding_inner(0.15);
                let y = ScaleLinear::new([0.0, max], [height, 8.0]);
                let baseline = y.tick(&0.0).unwrap_or(height);
                let band = x.band_width().max(1.0);
                let rows = self.data.clone();
                Bar::new()
                    .data(rows)
                    .band_width(band)
                    .cross(move |datum: &Datum| x.tick(&datum.x))
                    .base(move |_| baseline)
                    .value(move |datum: &Datum| y.tick(&datum.y))
                    .fill(move |_, _, _| color)
                    .paint(&bounds, window, cx);
            }
            "area" => {
                let rows = self.numbers();
                let max_x = (rows.len().saturating_sub(1) as f32).max(1.0);
                let max_y = rows.iter().map(|row| row.1).fold(0.0, f32::max).max(1.0);
                let x = ScaleLinear::new([0.0, max_x], [0.0, width]);
                let y = ScaleLinear::new([0.0, max_y], [height, 8.0]);
                Area::new()
                    .data(rows)
                    .x(move |row: &(f32, f32)| x.tick(&row.0))
                    .y0(height)
                    .y1(move |row: &(f32, f32)| y.tick(&row.1))
                    .fill(color.opacity(0.28))
                    .stroke(color)
                    .paint(&bounds, window);
            }
            "pie" => {
                let rows = self.data.clone();
                let pie = Pie::new().value(|datum: &Datum| Some(datum.y));
                let arcs = pie.arcs(&rows);
                let radius = width.min(height) * 0.38;
                let arc = Arc::new().inner_radius(radius * 0.45).outer_radius(radius);
                for (index, arc_data) in arcs.iter().enumerate() {
                    let shade = if index % 2 == 0 {
                        color
                    } else {
                        color.opacity(0.55)
                    };
                    arc.paint(arc_data, shade, &bounds, window);
                }
            }
            "scatter" => {
                let rows = self.numbers();
                let max_x = (rows.len().saturating_sub(1) as f32).max(1.0);
                let max_y = rows.iter().map(|row| row.1).fold(0.0, f32::max).max(1.0);
                let x = ScaleLinear::new([0.0, max_x], [0.0, width]);
                let y = ScaleLinear::new([0.0, max_y], [height, 8.0]);
                Line::new()
                    .data(rows)
                    .x(move |row: &(f32, f32)| x.tick(&row.0))
                    .y(move |row: &(f32, f32)| y.tick(&row.1))
                    .stroke(color.opacity(0.0))
                    .dot()
                    .dot_size(px(4.0))
                    .paint(&bounds, window);
            }
            _ => {
                let rows = self.numbers();
                let max_x = (rows.len().saturating_sub(1) as f32).max(1.0);
                let max_y = rows.iter().map(|row| row.1).fold(0.0, f32::max).max(1.0);
                let x = ScaleLinear::new([0.0, max_x], [0.0, width]);
                let y = ScaleLinear::new([0.0, max_y], [height, 8.0]);
                let _ = point(px(0.0), px(0.0));
                Line::new()
                    .data(rows)
                    .x(move |row: &(f32, f32)| x.tick(&row.0))
                    .y(move |row: &(f32, f32)| y.tick(&row.1))
                    .stroke(color)
                    .stroke_width(px(1.75))
                    .paint(&bounds, window);
            }
        }
    }
}

impl IntoElement for FeaturePlot {
    type Element = PlotElement<Self>;

    fn into_element(self) -> Self::Element {
        PlotElement::new(self)
    }
}
