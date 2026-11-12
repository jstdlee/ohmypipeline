//! The Stats tab of the preview panel: one row per column of the frame the
//! worker just computed, with a missing-value overview at the top.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{App, IntoElement, ParentElement as _, SharedString, Styled as _, div, px};
use graph::ColumnStats;
use graph::cell_text;

/// One full stats view. Scrollable; sized by the panel around it.
pub fn stats_view(stats: &[ColumnStats], cx: &mut App) -> gpui_kit::AnyElement {
    let theme = cx.theme();
    let mono = theme.mono_font_family.clone();
    let fg = theme.foreground;
    let muted = theme.muted_foreground;
    let border = theme.border;
    let danger = theme.danger;
    let accent = theme.primary;

    let mut rows: Vec<gpui_kit::AnyElement> = Vec::with_capacity(stats.len() + 1);
    rows.push(
        header_row(fg)
            .border_b_1()
            .border_color(border)
            .into_any_element(),
    );
    for stat in stats {
        let present = stat.count - stat.nulls;
        let name = div().truncate().child(stat.name.clone());
        let dtype = mono_cell(&stat.dtype, &mono, muted);
        let non_null = number_cell(present.to_string(), &mono);
        let missing = div()
            .w(px(112.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .child(
                div()
                    .w(px(48.0))
                    .h(px(4.0))
                    .rounded(px(2.0))
                    .bg(muted.opacity(0.2))
                    .child(
                        div()
                            .w(px(48.0 * (stat.null_pct / 100.0) as f32))
                            .h(px(4.0))
                            .rounded(px(2.0))
                            .bg(danger),
                    ),
            )
            .child(percent_cell(stat.null_pct, &mono, muted));
        let unique = number_cell(optional_text(stat.unique.map(|u| u.to_string())), &mono);
        let min = value_cell(stat.min.as_ref(), &mono);
        let max = value_cell(stat.max.as_ref(), &mono);
        let mean = number_cell(
            match stat.mean {
                Some(value) => sig4(value),
                None => "—".to_string(),
            },
            &mono,
        );
        rows.push(
            div()
                .h(px(28.0))
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(label_cell(name.into_any_element()))
                .child(width_cell(dtype, 84.0))
                .child(width_cell(non_null, 64.0))
                .child(missing)
                .child(width_cell(unique, 56.0))
                .child(width_cell(min, 70.0))
                .child(width_cell(max, 70.0))
                .child(width_cell(mean, 70.0))
                .child(spread_cell(stat, &mono, accent, muted))
                .into_any_element(),
        );
    }

    div()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .px(px(12.0))
        .overflow_y_scrollbar()
        .child(overview_row(stats, muted))
        .children(rows)
        .into_any_element()
}

fn overview_row(stats: &[ColumnStats], muted: gpui_kit::Hsla) -> gpui_kit::Div {
    let missing = stats.iter().filter(|stat| stat.nulls > 0).count();
    let total: u64 = stats.iter().map(|stat| stat.nulls).sum();
    div()
        .w_full()
        .py(px(6.0))
        .text_xs()
        .text_color(muted)
        .child(format!(
            "{missing} of {} columns have missing values · {total} nulls",
            stats.len()
        ))
}

fn header_row(fg: gpui_kit::Hsla) -> gpui_kit::Div {
    let label = |name: &'static str, width: Option<f32>| -> gpui_kit::AnyElement {
        let cell = div().text_xs().text_color(fg.opacity(0.85)).child(name);
        match width {
            Some(width) => width_cell(cell.into_any_element(), width).into_any_element(),
            None => label_cell(cell.into_any_element()).into_any_element(),
        }
    };
    div()
        .h(px(28.0))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(label("Column", None))
        .child(label("Type", Some(84.0)))
        .child(label("Non-null", Some(64.0)))
        .child(label("Missing", Some(112.0)))
        .child(label("Unique", Some(56.0)))
        .child(label("Min", Some(70.0)))
        .child(label("Max", Some(70.0)))
        .child(label("Mean", Some(70.0)))
        .child(label("Top", Some(64.0)))
}

// Cells ---------------------------------------------------------------------

/// The name column takes whatever width is left.
fn label_cell(cell: gpui_kit::AnyElement) -> gpui_kit::Div {
    div().flex_1().min_w(px(0.0)).child(cell)
}

fn width_cell(cell: gpui_kit::AnyElement, width: f32) -> gpui_kit::Div {
    div().w(px(width)).flex_shrink_0().child(cell)
}

fn mono_cell(text: &str, mono: &SharedString, color: gpui_kit::Hsla) -> gpui_kit::AnyElement {
    div()
        .truncate()
        .text_xs()
        .font_family(mono.clone())
        .text_color(color)
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

fn number_cell(text: String, mono: &SharedString) -> gpui_kit::AnyElement {
    div()
        .truncate()
        .w_full()
        .text_xs()
        .text_right()
        .font_family(mono.clone())
        .child(SharedString::from(text))
        .into_any_element()
}

/// Bar, percent, histogram, or the most common value, whichever the column has.
fn spread_cell(
    stat: &ColumnStats,
    mono: &SharedString,
    accent: gpui_kit::Hsla,
    muted: gpui_kit::Hsla,
) -> gpui_kit::AnyElement {
    if !stat.hist.is_empty() {
        let top = stat.hist.iter().copied().max().unwrap_or(0).max(1);
        let mut bar = div()
            .w(px(62.0))
            .h(px(18.0))
            .flex()
            .flex_row()
            .items_end()
            .gap(px(1.0));
        for bin in &stat.hist {
            let height = (18.0 * (*bin as f32 / top as f32)).max(if *bin > 0 { 2.0 } else { 1.0 });
            bar = bar.child(
                div()
                    .w(px(4.0))
                    .rounded(px(1.0))
                    .bg(accent.opacity(0.7))
                    .h(px(height)),
            );
        }
        return width_cell(bar.into_any_element(), 64.0).into_any_element();
    }
    if let Some(count) = stat.top.first() {
        let text = div()
            .truncate()
            .text_xs()
            .font_family(mono.clone())
            .text_color(muted)
            .child(cell_text(&count.0));
        return width_cell(text.into_any_element(), 64.0).into_any_element();
    }
    empty_cell()
}

fn empty_cell() -> gpui_kit::AnyElement {
    div().into_any_element()
}

fn value_cell(value: Option<&serde_json::Value>, mono: &SharedString) -> gpui_kit::AnyElement {
    match value {
        None => number_cell("—".to_string(), mono),
        Some(value) => match value.as_f64().filter(|v| v.is_finite()) {
            Some(number) => number_cell(sig4(number), mono),
            None => number_cell(cell_text(value), mono),
        },
    }
}

fn optional_text(text: Option<String>) -> String {
    text.unwrap_or_else(|| "—".into())
}

fn percent_cell(pct: f64, mono: &SharedString, color: gpui_kit::Hsla) -> gpui_kit::AnyElement {
    let text = if pct >= 100.0 {
        "100%".to_string()
    } else if (pct * 10.0).round() as i64 % 10 == 0 {
        format!("{pct:.0}%")
    } else {
        format!("{pct:.1}%")
    };
    div()
        .w(px(34.0))
        .truncate()
        .text_xs()
        .text_right()
        .font_family(mono.clone())
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// Four significant digits, keeping integers integers.
pub fn sig4(value: f64) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value).trim_end_matches(".0").to_string();
    }
    let exponent = if value == 0.0 {
        0
    } else {
        value.abs().log10().floor() as i32
    };
    let decimals = (4 - 1 - exponent).clamp(0, 8);
    format!("{value:.*}", decimals as usize)
}
