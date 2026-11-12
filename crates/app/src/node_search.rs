//! The node search popover. A wire dropped on empty canvas, a drag from an
//! input end, or a double click on empty canvas opens it at the pointer;
//! picking a feature adds and connects the node in one undo step.

use std::collections::BTreeSet;
use std::time::Instant;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, DispatchPhase, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, Pixels, Point, SharedString, Styled as _,
    Window, canvas, div, px,
};
use graph::{
    FeatureDef, Point as CanvasPoint, TypeName, card_width, features_in, groups, try_connect,
};

use crate::workspace::Workspace;

/// Height of a card above its first port row, so a picked node lands with the
/// compatible port near the drop point.
const PORT_ANCHOR_Y: f32 = 63.0;
/// The fade and slide of the popover entrance.
const ENTRANCE_SECS: f32 = 0.14;

/// Live state of the open popover. `None` when it is closed.
pub struct NodeSearch {
    /// The output side the dropped wire came from.
    pub from: Option<(String, String)>,
    /// The input side the drag ended on.
    pub to: Option<(String, String)>,
    /// The type the new node must accept or produce.
    pub ty: Option<TypeName>,
    /// Canvas point of the drop.
    pub at: CanvasPoint,
    /// Window point to anchor the popover at.
    pub screen: Point<Pixels>,
    pub query: String,
    pub highlight: usize,
    pub opened: Instant,
}

/// Features the popover lists, grouped in catalog order, with the flat order
/// the keyboard highlight walks.
pub struct Matches {
    pub groups: Vec<(&'static str, Vec<&'static FeatureDef>)>,
    pub flat: Vec<(&'static FeatureDef, &'static str)>,
}

impl Workspace {
    pub(crate) fn open_node_search(
        &mut self,
        state: NodeSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.node_search_input.clone();
        input.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.node_search = Some(state);
        cx.notify();
    }

    fn close_node_search(&mut self, cx: &mut Context<Self>) {
        if self.node_search.take().is_some() {
            cx.notify();
        }
    }

    /// Features allowed by the port filter, when there is one.
    fn allowed_features(&self, cx: &App) -> Option<BTreeSet<&'static str>> {
        let ty = self.node_search.as_ref()?.ty?;
        let doc = self.session.read(cx).canvas.read(cx).editor();
        let picked = if self
            .node_search
            .as_ref()
            .and_then(|s| s.to.as_ref())
            .is_some()
        {
            doc.features_producing(ty)
        } else {
            doc.features_accepting(ty)
        };
        Some(picked.into_iter().map(|def| def.id).collect())
    }

    fn search_matches(&self, cx: &App) -> Matches {
        let query = self
            .node_search
            .as_ref()
            .map(|state| state.query.to_lowercase())
            .unwrap_or_default();
        let allowed = self.allowed_features(cx);
        let mut matches = Matches {
            groups: Vec::new(),
            flat: Vec::new(),
        };
        for group in groups() {
            let items: Vec<&'static FeatureDef> = features_in(group)
                .filter(|def| allowed.as_ref().is_none_or(|set| set.contains(def.id)))
                .filter(|def| {
                    query.is_empty()
                        || def.title.to_lowercase().contains(&query)
                        || def.id.contains(&query)
                })
                .collect();
            if items.is_empty() {
                continue;
            }
            for def in &items {
                matches.flat.push((def, group));
            }
            matches.groups.push((group, items));
        }
        matches
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        let total = self.search_matches(cx).flat.len();
        if total == 0 {
            return;
        }
        if let Some(state) = &mut self.node_search {
            let current = state.highlight as isize;
            state.highlight = (current + delta).rem_euclid(total as isize) as usize;
        }
        cx.notify();
    }

    fn pick_highlight(&mut self, cx: &mut Context<Self>) {
        let index = self.node_search.as_ref().map(|state| state.highlight);
        let Some(index) = index else {
            return;
        };
        let matches = self.search_matches(cx);
        let Some((def, _)) = matches.flat.get(index) else {
            return;
        };
        let id = def.id.to_string();
        self.pick_node(&id, cx);
    }

    /// Add the picked feature at the drop point and connect it through its
    /// first compatible port, all as one undo step.
    fn pick_node(&mut self, feature: &str, cx: &mut Context<Self>) {
        let Some(state) = self.node_search.take() else {
            return;
        };
        let (from, to, at) = (state.from.clone(), state.to.clone(), state.at);
        self.session.update(cx, |session, cx| {
            session.canvas.update(cx, |canvas, cx| {
                canvas.begin_edit();
                let position = if to.is_some() {
                    // The new output port should land near the drop point.
                    CanvasPoint::new(at.x - card_width(feature), at.y - PORT_ANCHOR_Y)
                } else if from.is_some() {
                    CanvasPoint::new(at.x, at.y - PORT_ANCHOR_Y)
                } else {
                    at
                };
                let id = if feature == "loop_start" {
                    match canvas.editor_mut().add_loop(position) {
                        Some((start, _)) => start,
                        None => {
                            canvas.editor_mut().cancel_gesture();
                            return;
                        }
                    }
                } else {
                    let Some(id) = canvas.editor_mut().add_node(feature, position) else {
                        canvas.editor_mut().cancel_gesture();
                        return;
                    };
                    id
                };
                if let Some((from_node, from_port)) = &from {
                    let ports = canvas
                        .editor()
                        .graph()
                        .node(&id)
                        .map(|node| node.input_ports())
                        .unwrap_or_default();
                    for port in ports {
                        if try_connect(
                            canvas.editor().graph(),
                            from_node,
                            from_port,
                            &id,
                            &port.name,
                        )
                        .is_ok()
                        {
                            let _ = canvas
                                .editor_mut()
                                .connect(from_node, from_port, &id, &port.name);
                            break;
                        }
                    }
                } else if let Some((to_node, to_port)) = &to
                    && let Some(output) = canvas
                        .editor()
                        .graph()
                        .node(&id)
                        .and_then(|node| node.output_port())
                    && try_connect(canvas.editor().graph(), &id, &output.name, to_node, to_port)
                        .is_ok()
                {
                    let _ = canvas
                        .editor_mut()
                        .connect(&id, &output.name, to_node, to_port);
                }
                canvas.commit_edit(cx);
                canvas.select_only(id, cx);
                cx.emit(canvas::CanvasEvent::FocusInspector);
            });
        });
        cx.notify();
    }

    /// The popover element, when it is open. Rendered above the dock.
    pub(crate) fn render_node_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(state) = self.node_search.as_ref() else {
            return div().into_any_element();
        };
        let reduced = self.session.read(cx).reduced_motion;
        let elapsed = state.opened.elapsed().as_secs_f32();
        let t = if reduced {
            1.0
        } else {
            (elapsed / ENTRANCE_SECS).clamp(0.0, 1.0)
        };
        if t < 1.0 {
            // Keep animating the entrance for a few more frames.
            let entity = cx.entity().downgrade();
            window.on_next_frame(move |_window, cx| {
                let _ = entity.update(cx, |_, cx| cx.notify());
            });
        }
        let screen = state.screen;
        // Clamp the popover inside the canvas panel: flip to the left or
        // above the pointer when the window has no room for it, so the
        // 300x360 sheet never clips at an edge.
        const POPOVER_W: f32 = 300.0;
        const POPOVER_H: f32 = 360.0;
        const INSET: f32 = 8.0;
        let mut left = screen.x.as_f32();
        let mut top = screen.y.as_f32();
        let panel = self.session.read(cx).canvas.read(cx).panel_bounds();
        if let Some(panel) = panel {
            let panel_left = panel.origin.x.as_f32();
            let panel_top = panel.origin.y.as_f32();
            let panel_right = panel_left + panel.size.width.as_f32();
            let panel_bottom = panel_top + panel.size.height.as_f32();
            if left + POPOVER_W > panel_right {
                // No room to the right: open to the left of the pointer.
                left = (screen.x.as_f32() - POPOVER_W).min(panel_right - POPOVER_W);
            }
            if top + POPOVER_H > panel_bottom {
                // No room below: open above the pointer.
                top = (screen.y.as_f32() - POPOVER_H).min(panel_bottom - POPOVER_H);
            }
            left = left.max(panel_left + INSET);
            top = top.max(panel_top + INSET);
        }
        let slide = 2.0 * (1.0 - t);
        let matches = self.search_matches(cx);
        let highlight = self.node_search.as_ref().map(|s| s.highlight).unwrap_or(0);
        let input = self.node_search_input.clone();
        let query_empty = input.read(cx).value().is_empty();
        let theme_fg = cx.theme().popover_foreground;
        let theme_muted = cx.theme().muted_foreground;
        let theme_hover = cx.theme().secondary;
        let theme_accent = cx.theme().accent;
        let theme_accent_fg = cx.theme().accent_foreground;
        let close = cx.entity();

        // Keys reach the popover before the focused input, through a capture
        // phase listener painted alongside the popover itself.
        let keys = cx.entity();
        let catcher =
            div()
                .absolute()
                .size_full()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    close.update(cx, |this, cx| this.close_node_search(cx));
                });
        let key_frame = canvas(
            move |_, _, _| {},
            move |_, _, window, _| {
                window.on_key_event(move |event: &KeyDownEvent, phase, _window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let key = event.keystroke.key.as_ref();
                    match key {
                        "up" | "down" | "escape" => {}
                        // Plain Enter picks; a chord such as ctrl-enter belongs
                        // to the window.
                        "enter"
                            if !event.keystroke.modifiers.control
                                && !event.keystroke.modifiers.platform
                                && !event.keystroke.modifiers.alt
                                && !event.keystroke.modifiers.shift => {}
                        _ => return,
                    }
                    keys.update(cx, |this, cx| match key {
                        "up" => this.move_highlight(-1, cx),
                        "down" => this.move_highlight(1, cx),
                        "enter" => this.pick_highlight(cx),
                        _ => this.close_node_search(cx),
                    });
                    cx.stop_propagation();
                });
            },
        )
        .absolute()
        .size_full();

        let mut list = div()
            .id("node-search-list")
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .px(px(6.0))
            .pb(px(6.0))
            .overflow_y_scrollbar();
        let mut index = 0usize;
        for (group, items) in &matches.groups {
            list = list.child(
                div()
                    .px(px(8.0))
                    .pt(px(6.0))
                    .pb(px(2.0))
                    .text_xs()
                    .text_color(theme_muted)
                    .child(*group),
            );
            for def in items {
                let selected = index == highlight;
                let id = def.id.to_string();
                let entity = cx.entity();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("search-{}", def.id)))
                        .h(px(30.0))
                        .w_full()
                        .px(px(8.0))
                        .rounded(px(6.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .text_sm()
                        .cursor_pointer()
                        .when(selected, |row| {
                            row.bg(theme_accent).text_color(theme_accent_fg)
                        })
                        .hover(move |style| style.bg(theme_hover))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            entity.update(cx, |this, cx| this.pick_node(&id, cx));
                        })
                        .child(div().flex_1().truncate().child(def.title.to_string()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(if selected {
                                    theme_accent_fg
                                } else {
                                    theme_muted
                                })
                                .child(*group),
                        ),
                );
                index += 1;
            }
        }
        if matches.flat.is_empty() {
            list = list.child(
                div()
                    .p(px(10.0))
                    .text_xs()
                    .text_color(theme_muted)
                    .child("No matching feature"),
            );
        }

        div()
            .absolute()
            .size_full()
            .child(catcher)
            .child(key_frame)
            .child(
                div()
                    .absolute()
                    .left(px(left))
                    .top(px((top + slide).max(8.0)))
                    .w(px(300.0))
                    .max_h(px(360.0))
                    .flex()
                    .flex_col()
                    .rounded(px(8.0))
                    .bg(cx.theme().popover)
                    .text_color(theme_fg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .shadow_md()
                    .opacity(t)
                    .overflow_hidden()
                    .child(
                        div()
                            .relative()
                            .p(px(6.0))
                            .pb(px(2.0))
                            .child(Input::new(&input).small())
                            .when(query_empty, |row| {
                                row.child(
                                    div()
                                        .absolute()
                                        .left(px(16.0))
                                        .top(px(12.0))
                                        .text_xs()
                                        .text_color(theme_muted)
                                        .child("Add node..."),
                                )
                            }),
                    )
                    .child(list),
            )
            .into_any_element()
    }
}

/// Keep the popover query in step with the search input.
pub fn watch_node_search_input(
    this: &mut Workspace,
    input: &Entity<InputState>,
    event: &InputEvent,
    cx: &mut Context<Workspace>,
) {
    if !matches!(event, InputEvent::Change) {
        return;
    }
    let query = input.read(cx).value().to_string();
    if let Some(state) = &mut this.node_search {
        state.query = query;
        state.highlight = 0;
        cx.notify();
    }
}
