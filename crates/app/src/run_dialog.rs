//! "Run with…" : edit variable values for a single run. The document stays.

use std::collections::BTreeMap;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, px,
};
use graph::{RunMode, parse_value, value_text_of};

use crate::workspace::{Session, Workspace};

struct DraftRow {
    name: String,
    kind: graph::VarKind,
    input: Entity<InputState>,
    _sub: Subscription,
}

pub struct RunDialog {
    session: Entity<Session>,
    rows: Vec<DraftRow>,
}

impl RunDialog {
    fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let variables = session
            .read(cx)
            .canvas
            .read(cx)
            .editor()
            .graph()
            .variables
            .clone();
        let mut rows = Vec::new();
        for variable in variables {
            let input = cx.new(|cx| InputState::new(window, cx));
            let text = value_text_of(&variable.value);
            input.update(cx, |state, cx| state.set_value(text, window, cx));
            let sub = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
            rows.push(DraftRow {
                name: variable.name,
                kind: variable.kind,
                input,
                _sub: sub,
            });
        }
        Self { session, rows }
    }

    fn problems(&self, cx: &App) -> Option<String> {
        for row in &self.rows {
            let text = row.input.read(cx).value().to_string();
            if let Err(message) = parse_value(row.kind, &text) {
                return Some(format!("{}: {message}", row.name));
            }
        }
        let session = self.session.read(cx);
        let target = session
            .canvas
            .read(cx)
            .selected_node()
            .map(|node| node.id.clone());
        let sinks = graph::sinks(session.canvas.read(cx).editor().graph());
        if target.is_none() && sinks.len() != 1 {
            return Some("Select a node to run.".into());
        }
        None
    }

    fn full_blocked(&self, cx: &App) -> bool {
        !self.session.read(cx).full_enabled
    }

    fn overrides(&self, cx: &App) -> BTreeMap<String, String> {
        self.rows
            .iter()
            .map(|row| (row.name.clone(), row.input.read(cx).value().to_string()))
            .collect()
    }

    fn run(&mut self, mode: RunMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.problems(cx).is_some() {
            cx.notify();
            return;
        }
        let overrides = self.overrides(cx);
        self.session.update(cx, |session, cx| {
            session.run_overridden(mode, overrides, cx);
            session.arm_poll(window, cx);
        });
        window.close_dialog(cx);
    }
}

impl Render for RunDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let problem = self.problems(cx);
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let mut body = div().flex().flex_col().gap_3();
        if self.rows.is_empty() {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("This pipeline has no variables. The run uses the file as it is."),
            );
        }
        for row in &self.rows {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().child(row.name.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(kind_label(row.kind)),
                            ),
                    )
                    .child(Input::new(&row.input).small()),
            );
        }
        if let Some(message) = problem.clone() {
            body = body.child(div().text_xs().text_color(danger).child(message));
        }
        let blocked = problem.is_some();
        let full_blocked = blocked || self.full_blocked(cx);
        body.child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap_2()
                .pt_1()
                .child(
                    Button::new("run-full-with")
                        .small()
                        .label("Run full dataset")
                        .disabled(full_blocked)
                        .tooltip("Enabled after a preview of this graph.")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run(RunMode::Full, window, cx);
                        })),
                )
                .child(
                    Button::new("run-preview-with")
                        .small()
                        .primary()
                        .label("Run preview")
                        .disabled(blocked)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run(RunMode::Preview, window, cx);
                        })),
                ),
        )
    }
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

pub fn open(session: Entity<Session>, window: &mut Window, cx: &mut Context<Workspace>) {
    let dialog = cx.new(|cx| RunDialog::new(session, window, cx));
    window.open_dialog(cx, move |dialog_builder, _, _| {
        dialog_builder
            .title("Run with…")
            .w(px(420.0))
            .child(dialog.clone())
    });
}
