//! The variables inspector. Shown when nothing is selected.
//!
//! Edits commit on Enter or blur, one undo step each, through the document.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};
use graph::{
    TextPiece, Variable, is_variable_name, parse_value, referenced_vars, split_references,
    value_text_of,
};

use crate::workspace::Session;

const KINDS: &[(&str, graph::VarKind)] = &[
    ("string", graph::VarKind::String),
    ("int", graph::VarKind::Int),
    ("float", graph::VarKind::Float),
    ("bool", graph::VarKind::Bool),
    ("path", graph::VarKind::Path),
];

struct VarRow {
    key: String,
    name: Entity<InputState>,
    value: Entity<InputState>,
    kind: graph::VarKind,
    committed_value: String,
    name_error: Option<String>,
    value_error: Option<String>,
    _subs: Vec<Subscription>,
}

pub struct VariablesPanel {
    focus: FocusHandle,
    session: Entity<Session>,
    rows: Vec<VarRow>,
    /// Which row is showing its kind list.
    kind_open: Option<usize>,
}

impl VariablesPanel {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            focus: cx.focus_handle(),
            session,
            rows: Vec::new(),
            kind_open: None,
        }
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let variables = self
            .session
            .read(cx)
            .canvas
            .read(cx)
            .editor()
            .graph()
            .variables
            .clone();
        let same = variables.len() == self.rows.len()
            && variables
                .iter()
                .zip(self.rows.iter())
                .all(|(variable, row)| variable.name == row.key && variable.kind == row.kind);
        if !same {
            self.rows.clear();
            self.kind_open = None;
            for variable in &variables {
                self.rows.push(self.bind(variable, window, cx));
            }
            return;
        }
        for (variable, row) in variables.iter().zip(self.rows.iter_mut()) {
            let text = value_text_of(&variable.value);
            if text != row.committed_value && row.value_error.is_none() {
                let input = row.value.clone();
                input.update(cx, |state, cx| state.set_value(text.clone(), window, cx));
                row.committed_value = text;
            }
        }
    }

    fn bind(&self, variable: &Variable, window: &mut Window, cx: &mut Context<Self>) -> VarRow {
        let name = cx.new(|cx| InputState::new(window, cx));
        let value = cx.new(|cx| InputState::new(window, cx));
        let name_text = variable.name.clone();
        let value_text = value_text_of(&variable.value);
        name.update(cx, |state, cx| {
            state.set_value(name_text.clone(), window, cx);
        });
        value.update(cx, |state, cx| {
            state.set_value(value_text.clone(), window, cx);
        });
        let key = variable.name.clone();
        let name_key = key.clone();
        let value_key = key.clone();
        let name_sub = cx.subscribe(&name, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.review_name(&name_key, cx);
            }
            if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                this.commit_name(&name_key, cx);
            }
        });
        let value_sub = cx.subscribe(&value, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.review_value(&value_key, cx);
            }
            if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                this.commit_value(&value_key, cx);
            }
        });
        VarRow {
            key,
            name,
            value,
            kind: variable.kind,
            committed_value: value_text,
            name_error: None,
            value_error: None,
            _subs: vec![name_sub, value_sub],
        }
    }

    fn review_name(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(index) = self.rows.iter().position(|row| row.key == key) else {
            return;
        };
        let text = self.rows[index].name.read(cx).value().to_string();
        self.rows[index].name_error = name_problem(&text, key, &self.names(cx));
        cx.notify();
    }

    fn review_value(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(index) = self.rows.iter().position(|row| row.key == key) else {
            return;
        };
        let kind = self.rows[index].kind;
        let text = self.rows[index].value.read(cx).value().to_string();
        self.rows[index].value_error = parse_value(kind, &text).err();
        cx.notify();
    }

    fn commit_name(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(index) = self.rows.iter().position(|row| row.key == key) else {
            return;
        };
        let text = self.rows[index].name.read(cx).value().trim().to_string();
        if text == key {
            self.rows[index].name_error = None;
            cx.notify();
            return;
        }
        if let Some(problem) = name_problem(&text, key, &self.names(cx)) {
            self.rows[index].name_error = Some(problem);
            cx.notify();
            return;
        }
        let renamed = self.session.update(cx, |session, cx| {
            session.canvas.update(cx, |canvas, cx| {
                let ok = canvas.editor_mut().rename_variable(key, &text);
                if ok {
                    cx.emit(canvas::CanvasEvent::Edited);
                }
                ok
            })
        });
        if renamed {
            self.rows[index].key = text;
            self.rows[index].name_error = None;
        }
        cx.notify();
    }

    fn commit_value(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(index) = self.rows.iter().position(|row| row.key == key) else {
            return;
        };
        let kind = self.rows[index].kind;
        let text = self.rows[index].value.read(cx).value().to_string();
        match parse_value(kind, &text) {
            Ok(value) => {
                self.rows[index].value_error = None;
                self.rows[index].committed_value = text;
                let variable = Variable {
                    name: key.to_string(),
                    kind,
                    value,
                };
                self.session.update(cx, |session, cx| {
                    session.canvas.update(cx, |canvas, cx| {
                        canvas.editor_mut().set_variable(variable);
                        cx.emit(canvas::CanvasEvent::Edited);
                    });
                });
            }
            Err(message) => self.rows[index].value_error = Some(message),
        }
        cx.notify();
    }

    fn set_kind(&mut self, index: usize, kind: graph::VarKind, cx: &mut Context<Self>) {
        self.kind_open = None;
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let text = row.value.read(cx).value().to_string();
        let key = row.key.clone();
        match parse_value(kind, &text) {
            Ok(value) => {
                if let Some(row) = self.rows.get_mut(index) {
                    row.kind = kind;
                    row.value_error = None;
                    row.committed_value = text;
                }
                self.session.update(cx, |session, cx| {
                    session.canvas.update(cx, |canvas, cx| {
                        canvas.editor_mut().set_variable(Variable {
                            name: key,
                            kind,
                            value,
                        });
                        cx.emit(canvas::CanvasEvent::Edited);
                    });
                });
            }
            Err(message) => {
                if let Some(row) = self.rows.get_mut(index) {
                    row.value_error = Some(message);
                }
            }
        }
        cx.notify();
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        let names = self.names(cx);
        let mut name = "var".to_string();
        let mut n = 2;
        while names.iter().any(|found| found == &name) {
            name = format!("var_{n}");
            n += 1;
        }
        self.session.update(cx, |session, cx| {
            session.canvas.update(cx, |canvas, cx| {
                canvas.editor_mut().set_variable(Variable {
                    name,
                    kind: graph::VarKind::String,
                    value: serde_json::json!(""),
                });
                cx.emit(canvas::CanvasEvent::Edited);
            });
        });
        cx.notify();
    }

    fn remove(&mut self, key: &str, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.canvas.update(cx, |canvas, cx| {
                canvas.editor_mut().remove_variable(key);
                cx.emit(canvas::CanvasEvent::Edited);
            });
        });
        cx.notify();
    }

    fn names(&self, cx: &App) -> Vec<String> {
        self.session
            .read(cx)
            .canvas
            .read(cx)
            .editor()
            .graph()
            .variables
            .iter()
            .map(|variable| variable.name.clone())
            .collect()
    }
}

impl Focusable for VariablesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for VariablesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(window, cx);
        let usage = usage_counts(cx, &self.session);
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let mut column = div()
            .id("variables")
            .size_full()
            .flex()
            .flex_col()
            .p_3()
            .gap_3()
            .overflow_y_scrollbar()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().child("Variables"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("A ${name} in a setting reads the value here."),
                    ),
            );
        if self.rows.is_empty() {
            column = column.child(div().text_xs().text_color(muted).child("No variables yet."));
        }
        for (index, row) in self.rows.iter().enumerate() {
            let used = usage.get(&row.key).copied().unwrap_or(0);
            let kind_label = kind_label(row.kind);
            let name_bad = row.name_error.is_some();
            let value_bad = row.value_error.is_some();
            let mut block = div().flex().flex_col().gap_1().child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .w(px(88.0))
                            .when(name_bad, |el| {
                                el.border_1().border_color(danger).rounded(px(6.0))
                            })
                            .child(Input::new(&row.name).small()),
                    )
                    .child({
                        let open = self.kind_open == Some(index);
                        Button::new(SharedString::from(format!("kind-{index}")))
                            .small()
                            .compact()
                            .icon(gpui_kit::assets::IconName::ChevronDown)
                            .label(kind_label.to_string())
                            .selected(open)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.kind_open = if this.kind_open == Some(index) {
                                    None
                                } else {
                                    Some(index)
                                };
                                cx.notify();
                            }))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .when(value_bad, |el| {
                                el.border_1().border_color(danger).rounded(px(6.0))
                            })
                            .child(Input::new(&row.value).small()),
                    )
                    .child(
                        Button::new(SharedString::from(format!("var-remove-{index}")))
                            .ghost()
                            .compact()
                            .icon(gpui_kit::assets::IconName::X)
                            .tooltip("Remove")
                            .accessibility_label(format!("Remove {}", row.key))
                            .on_click({
                                let key = row.key.clone();
                                cx.listener(move |this, _, _, cx| this.remove(&key, cx))
                            }),
                    ),
            );
            if self.kind_open == Some(index) {
                let mut kinds = div().flex().flex_row().flex_wrap().gap_1();
                for (label, kind) in KINDS {
                    let kind = *kind;
                    kinds = kinds.child(
                        Button::new(SharedString::from(format!("kind-{index}-{label}")))
                            .small()
                            .compact()
                            .selected(row.kind == kind)
                            .label((*label).to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_kind(index, kind, cx);
                            })),
                    );
                }
                block = block.child(kinds);
            }
            if let Some(message) = row.name_error.as_ref().or(row.value_error.as_ref()) {
                block = block.child(div().text_xs().text_color(danger).child(message.clone()));
            }
            block = block.child(div().text_xs().text_color(muted).child(used_label(used)));
            column = column.child(block);
        }
        column.child(
            Button::new("add-variable")
                .small()
                .label("Add variable")
                .on_click(cx.listener(|this, _, _, cx| this.add(cx))),
        )
    }
}

fn name_problem(text: &str, current: &str, names: &[String]) -> Option<String> {
    if text == current {
        return None;
    }
    if !is_variable_name(text) {
        return Some("Use a letter, then letters, digits, or _.".into());
    }
    if names.iter().any(|name| name == text) {
        return Some("That name is already used.".into());
    }
    None
}

fn kind_label(kind: graph::VarKind) -> &'static str {
    match kind {
        graph::VarKind::String => "string",
        graph::VarKind::Int => "int",
        graph::VarKind::Float => "float",
        graph::VarKind::Bool => "bool",
        graph::VarKind::Path => "path",
    }
}

fn used_label(count: usize) -> String {
    match count {
        1 => "used by 1 node".into(),
        n => format!("used by {n} nodes"),
    }
}

fn usage_counts(cx: &App, session: &Entity<Session>) -> std::collections::BTreeMap<String, usize> {
    let graph = session.read(cx).canvas.read(cx).editor().graph().clone();
    let mut counts = std::collections::BTreeMap::new();
    for node in &graph.nodes {
        for name in referenced_vars(node) {
            *counts.entry(name).or_insert(0) += 1;
        }
    }
    counts
}

/// Colored `${name}` runs for an inspector field. Unknown names are danger
/// colored and carry the "unknown variable" tooltip.
pub fn highlighted_refs(
    text: &str,
    known: &std::collections::BTreeSet<String>,
    accent: gpui_kit::Hsla,
    danger: gpui_kit::Hsla,
    muted: gpui_kit::Hsla,
) -> gpui_kit::AnyElement {
    let mut row = div().flex().flex_row().flex_wrap().text_xs();
    for (index, piece) in split_references(text).into_iter().enumerate() {
        row = row.child(piece_el(index, piece, known, accent, danger, muted));
    }
    row.into_any_element()
}

fn piece_el(
    index: usize,
    piece: TextPiece,
    known: &std::collections::BTreeSet<String>,
    accent: gpui_kit::Hsla,
    danger: gpui_kit::Hsla,
    muted: gpui_kit::Hsla,
) -> gpui_kit::AnyElement {
    if let Some(name) = piece.variable.clone() {
        let known_name = known.contains(&name);
        let color = if known_name { accent } else { danger };
        return div()
            .id(SharedString::from(format!("insp-ref-{index}-{name}")))
            .text_color(color)
            .child(piece.text)
            .when(!known_name, |el| {
                el.tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new("unknown variable").build(window, cx)
                })
            })
            .into_any_element();
    }
    div().text_color(muted).child(piece.text).into_any_element()
}

/// Which output of an if_switch is the one the last run kept.
pub fn active_branch(
    graph: &graph::Graph,
    switch: &str,
    inactive: impl Fn(&str) -> bool,
) -> Option<&'static str> {
    let mut true_off = false;
    let mut false_off = false;
    let mut saw_true = false;
    let mut saw_false = false;
    for edge in &graph.edges {
        if edge.from != switch {
            continue;
        }
        if edge.from_port == "true" {
            saw_true = true;
            true_off |= inactive(&edge.to);
        } else if edge.from_port == "false" {
            saw_false = true;
            false_off |= inactive(&edge.to);
        }
    }
    match (saw_true && true_off, saw_false && false_off) {
        (true, false) => Some("false"),
        (false, true) => Some("true"),
        _ => None,
    }
}
