//! Chart marks drawn with GPUI Kit. The worker never sees these pixels.

use gpui_kit::component::chart::{BarChart, LineChart, PieChart};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Corners, Hsla, IntoElement, ParentElement as _, SharedString, Styled as _, div, px,
};
use serde_json::Value;

use graph::cell_text;

#[derive(Clone, Debug)]
pub struct PlotPoint {
    pub x: String,
    pub y: f32,
    pub series: String,
    pub index: usize,
    /// "left" or "right". GPUI Chart draws one y axis, so right is a second plot.
    pub axis: String,
}

pub fn points_from(
    columns: &[String],
    rows: &[Vec<Value>],
    x_name: &str,
    y_name: &str,
    series_name: &str,
) -> Result<Vec<PlotPoint>, String> {
    if y_name.is_empty() {
        return Err("Choose a value column".into());
    }
    let y_index = columns
        .iter()
        .position(|column| column == y_name)
        .ok_or_else(|| format!("{y_name} is not a column"))?;
    let x_index = columns.iter().position(|column| column == x_name);
    let series_index = if series_name.is_empty() {
        None
    } else {
        Some(
            columns
                .iter()
                .position(|column| column == series_name)
                .ok_or_else(|| format!("{series_name} is not a column"))?,
        )
    };
    let mut points = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let Some(raw) = row.get(y_index) else {
            continue;
        };
        if raw.is_null() {
            continue;
        }
        let y = match raw {
            Value::Number(number) => number.as_f64().map(|number| number as f32),
            Value::String(text) => text.parse().ok(),
            _ => None,
        };
        let Some(y) = y else {
            return Err(format!("{y_name} is not numeric"));
        };
        let x = x_index
            .and_then(|index| row.get(index))
            .map(cell_text)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| index.to_string());
        let series = series_index
            .and_then(|index| row.get(index))
            .map(cell_text)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| y_name.to_string());
        points.push(PlotPoint {
            x,
            y,
            series,
            index,
            axis: "left".into(),
        });
    }
    if points.is_empty() {
        return Err(format!("{y_name} has no numbers"));
    }
    Ok(points)
}

pub fn points_for_encodings(
    columns: &[String],
    rows: &[Vec<Value>],
    x_name: &str,
    y_names: &[String],
    series_name: &str,
    axes: &[(String, String)],
) -> Result<Vec<PlotPoint>, String> {
    let names = if y_names.is_empty() {
        vec![String::new()]
    } else {
        y_names.to_vec()
    };
    let mut all = Vec::new();
    for y_name in &names {
        let mut part = points_from(columns, rows, x_name, y_name, series_name)?;
        let axis = axes
            .iter()
            .find(|(name, _)| name == y_name)
            .map(|(_, axis)| axis.clone())
            .unwrap_or_else(|| "left".into());
        for point in &mut part {
            if names.len() > 1 {
                point.series = if series_name.is_empty() {
                    y_name.clone()
                } else {
                    format!("{y_name} · {}", point.series)
                };
            }
            point.axis = axis.clone();
        }
        all.extend(part);
    }
    Ok(all)
}

#[allow(clippy::too_many_arguments)]
pub fn feature_chart(
    mark: &str,
    points: Vec<PlotPoint>,
    colors: [Hsla; 5],
    foreground: Hsla,
    reduced_motion: bool,
    interactive: bool,
    id_prefix: &str,
    limitation: &str,
) -> AnyElement {
    if points.is_empty() {
        return div()
            .p_3()
            .text_xs()
            .text_color(foreground)
            .child("Run preview to draw this chart")
            .into_any_element();
    }
    let appear = !reduced_motion;
    let limitation_row = (!limitation.is_empty()).then(|| {
        div()
            .text_xs()
            .text_color(foreground)
            .child(limitation.to_string())
    });
    if mark == "pie" {
        let legend = slice_legend(&points, &colors, foreground);
        let prefix = id_prefix.to_string();
        let pie = PieChart::new(points)
            .id(SharedString::from(format!("{prefix}-pie")))
            .value(|point: &PlotPoint| point.y)
            .label(|point: &PlotPoint| point.x.clone().into())
            .color(move |point: &PlotPoint| colors[point.index % colors.len()])
            .name("value")
            .interactive(interactive)
            .appear(appear);
        return div()
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().flex_1().min_h(px(96.)).child(pie))
            .child(legend)
            .children(limitation_row)
            .into_any_element();
    }
    let right: Vec<PlotPoint> = points
        .iter()
        .filter(|point| point.axis == "right")
        .cloned()
        .collect();
    let points: Vec<PlotPoint> = points
        .into_iter()
        .filter(|point| point.axis != "right")
        .collect();
    if matches!(mark, "bar" | "histogram") {
        let names = series_names(&points);
        let legend = series_legend(&names, &colors, foreground);
        let names_for_fill = names.clone();
        let prefix = id_prefix.to_string();
        let bars = BarChart::new(points)
            .id(SharedString::from(format!("{prefix}-bars")))
            .band(|point: &PlotPoint| point.x.clone())
            .value(|point: &PlotPoint| point.y)
            .value_axis(true)
            .value_tick_count(4)
            .grid(true)
            .padding_inner(0.32)
            .corner_radii(Corners::all(px(3.0)))
            .name("value")
            .fill(move |point: &PlotPoint, _, _, _| {
                let index = names_for_fill
                    .iter()
                    .position(|name| name == &point.series)
                    .unwrap_or(0);
                colors[index % colors.len()]
            })
            .interactive(interactive)
            .appear(appear);
        return div()
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().flex_1().min_h(px(96.)).child(bars))
            .child(legend)
            .when(!right.is_empty(), |row| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(foreground)
                        .child("GPUI Chart has one y axis. Right-axis series need a line mark, drawn in a second plot."),
                )
            })
            .when(!limitation.is_empty(), |row| {
                row.child(div().text_xs().text_color(foreground).child(limitation.to_string()))
            })
            .into_any_element();
    }
    let names = series_names(&points);
    let legend = series_legend(&names, &colors, foreground);
    let (min_y, max_y) = y_span(&points);
    let mut plot = div().relative().flex_1().min_h(px(96.));
    for (index, name) in names.iter().enumerate() {
        let rows: Vec<PlotPoint> = points
            .iter()
            .filter(|point| &point.series == name)
            .cloned()
            .collect();
        let color = colors[index % colors.len()];
        let prefix = id_prefix.to_string();
        let series_name = name.clone();
        let line = LineChart::new(rows)
            .id(SharedString::from(format!("{prefix}-line-{index}")))
            .x(|point: &PlotPoint| point.x.clone())
            .y(|point: &PlotPoint| point.y)
            .y_domain(min_y, max_y)
            .x_axis(true)
            .y_axis(true)
            .y_tick_count(4)
            .grid(index == 0)
            .dot()
            .name(series_name.clone())
            .stroke(color)
            .tooltip_title(|point: &PlotPoint| point.x.clone().into())
            .tooltip_value(move |point: &PlotPoint, _| format!("{series_name} {}", point.y).into())
            .interactive(interactive && index + 1 == names.len())
            .appear(appear);
        plot = plot.child(div().absolute().size_full().child(line));
    }
    let right_names = series_names(&right);
    let right_legend = series_legend(&right_names, &colors, foreground);
    let (right_min, right_max) = if right.is_empty() {
        (0.0, 1.0)
    } else {
        y_span(&right)
    };
    let mut right_plot = div().relative().h(px(120.));
    for (index, name) in right_names.iter().enumerate() {
        let rows: Vec<PlotPoint> = right
            .iter()
            .filter(|point| &point.series == name)
            .cloned()
            .collect();
        let color = colors[(index + 2) % colors.len()];
        let prefix = id_prefix.to_string();
        let series_name = name.clone();
        let line = LineChart::new(rows)
            .id(SharedString::from(format!("{prefix}-right-{index}")))
            .x(|point: &PlotPoint| point.x.clone())
            .y(|point: &PlotPoint| point.y)
            .y_domain(right_min, right_max)
            .x_axis(true)
            .y_axis(true)
            .y_tick_count(3)
            .grid(index == 0)
            .dot()
            .name(series_name.clone())
            .stroke(color)
            .interactive(false)
            .appear(appear);
        right_plot = right_plot.child(div().absolute().size_full().child(line));
    }
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap_1()
        .child(plot)
        .child(legend)
        .when(!right.is_empty(), |column| {
            column
                .child(
                    div().text_xs().text_color(foreground).child(
                        "Right axis. GPUI Chart has one y axis, so this is a separate plot.",
                    ),
                )
                .child(right_plot)
                .child(right_legend)
        })
        .when(!limitation.is_empty(), |column| {
            column.child(
                div()
                    .text_xs()
                    .text_color(foreground)
                    .child(limitation.to_string()),
            )
        })
        .into_any_element()
}

fn series_names(points: &[PlotPoint]) -> Vec<String> {
    let mut names = Vec::new();
    for point in points {
        if !names.iter().any(|name| name == &point.series) {
            names.push(point.series.clone());
        }
    }
    names
}

fn y_span(points: &[PlotPoint]) -> (f32, f32) {
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for point in points {
        min_y = min_y.min(point.y);
        max_y = max_y.max(point.y);
    }
    if !min_y.is_finite() || (max_y - min_y).abs() < f32::EPSILON {
        (min_y - 1.0, max_y + 1.0)
    } else {
        (min_y, max_y)
    }
}

fn series_legend(names: &[String], colors: &[Hsla; 5], foreground: Hsla) -> gpui_kit::Div {
    let mut row = div().flex().flex_row().flex_wrap().gap_2().px_1();
    for (index, name) in names.iter().enumerate() {
        row = row.child(swatch(name, colors[index % colors.len()], foreground));
    }
    row
}

fn slice_legend(points: &[PlotPoint], colors: &[Hsla; 5], foreground: Hsla) -> gpui_kit::Div {
    let mut row = div().flex().flex_row().flex_wrap().gap_2().px_1();
    for point in points.iter().take(8) {
        row = row.child(swatch(
            &point.x,
            colors[point.index % colors.len()],
            foreground,
        ));
    }
    if points.len() > 8 {
        row = row.child(
            div()
                .text_xs()
                .text_color(foreground)
                .child(format!("+{}", points.len() - 8)),
        );
    }
    row
}

fn swatch(name: &str, color: Hsla, foreground: Hsla) -> gpui_kit::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .child(div().size(px(8.)).rounded_full().bg(color))
        .child(
            div()
                .text_xs()
                .text_color(foreground)
                .child(name.to_string()),
        )
}
