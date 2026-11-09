//! Document window. Dock holds the palette, the canvas, the chart, the inspector, and the preview.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use canvas::{CanvasEvent, CanvasModel, Dot, Tool};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::{
    BasePanel, DockArea, DockLayout, DockSkin, Panel, PanelEvent, panel_handle,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::table::{
    Column, ColumnFixed, ColumnSort, DataTable, TableDelegate, TableState,
};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _};
use gpui_kit::component::{TitleBar, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, actions, div, px,
};
use graph::{
    CHART_POINT_CAP, Job, JobOp, PREVIEW_ROW_CAP, RunMode, TABLE_WINDOW, ancestors, cell_text,
    duplicate_headers, header_pairs, run_fingerprint, sinks, status_line,
};
use serde_json::{Value, json};

use crate::chart_plot::{Datum, FeaturePlot};
use crate::worker::Worker;

actions!(
    ompp,
    [
        OpenFile,
        SaveFile,
        SaveFileAs,
        RunPreview,
        UndoEdit,
        RedoEdit,
        DeleteSelection,
        DuplicateSelection,
        FitView,
        ReduceMotion,
        ToggleTheme,
        QuitApp
    ]
);

#[derive(Default)]
pub struct GuiOptions {
    pub open: Option<PathBuf>,
    pub demo: Option<String>,
    pub ready_file: Option<PathBuf>,
}

pub struct Session {
    pub canvas: Entity<CanvasModel>,
    pub columns: Vec<String>,
    pub dtypes: Vec<String>,
    pub index: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub n: u64,
    pub mode: RunMode,
    pub status: String,
    pub full_enabled: bool,
    pub preview_fp: Option<u64>,
    pub generation: u64,
    pub running: bool,
    pub error: Option<String>,
    pub search: String,
    #[allow(dead_code)]
    pub sort: Option<(usize, bool)>,
    pub reduced_motion: bool,
    pub worker: Worker,
    pub path_prompt: Option<PathKind>,
}

pub enum PathKind {
    Open,
    SaveAs,
}

impl Session {
    fn target_node(&self, cx: &App) -> Option<String> {
        let canvas = self.canvas.read(cx);
        if let Some(node) = canvas.selected_node() {
            return Some(node.id.clone());
        }
        let sinks = sinks(canvas.editor().graph());
        if sinks.len() == 1 {
            Some(sinks[0].clone())
        } else {
            None
        }
    }

    fn recompute_dots(&self, cx: &mut App) {
        let canvas = self.canvas.read(cx);
        let graph = canvas.editor().graph();
        let mut dots = BTreeMap::new();
        for node in &graph.nodes {
            let dot = if node.unknown {
                Dot::Danger
            } else if self.running && self.target_node(cx).as_deref() == Some(node.id.as_str()) {
                Dot::Running
            } else if self.error.is_some()
                && self.target_node(cx).as_deref() == Some(node.id.as_str())
            {
                Dot::Danger
            } else {
                Dot::Muted
            };
            dots.insert(node.id.clone(), dot);
        }
        let reduced = self.reduced_motion;
        self.canvas.update(cx, |canvas, cx| {
            canvas.set_reduced_motion(reduced, cx);
            canvas.set_dots(dots, cx);
        });
    }
}

pub struct Workspace {
    session: Entity<Session>,
    dock: Entity<DockArea>,
    focus: FocusHandle,
    polling: bool,
    path_input: Entity<InputState>,
    #[allow(dead_code)]
    search_input: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl Workspace {
    pub fn new(options: GuiOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let canvas = cx.new(|cx| CanvasModel::new(window, cx));
        if let Some(Ok(doc)) = options.open.as_deref().map(graph::Document::load) {
            canvas.update(cx, |canvas, cx| canvas.load_document(doc, cx));
        }
        load_demo(&options.demo, &canvas, window, cx);
        let reduced = load_reduced_motion();
        canvas.update(cx, |canvas, cx| canvas.set_reduced_motion(reduced, cx));
        let worker = Worker::spawn();
        let session = cx.new(|_cx| Session {
            canvas: canvas.clone(),
            columns: Vec::new(),
            dtypes: Vec::new(),
            index: Vec::new(),
            rows: Vec::new(),
            n: 0,
            mode: RunMode::Preview,
            status: "preview · at most 10,000 source rows · not the file".into(),
            full_enabled: false,
            preview_fp: None,
            generation: 0,
            running: false,
            error: None,
            search: String::new(),
            sort: None,
            reduced_motion: reduced,
            worker,
            path_prompt: None,
        });
        let path_input = cx.new(|cx| InputState::new(window, cx));
        let search_input = cx.new(|cx| InputState::new(window, cx));
        let palette = cx.new(|cx| PalettePanel::new(session.clone(), cx));
        let graph_panel = cx.new(|cx| GraphPanel::new(canvas.clone(), cx));
        let chart = cx.new(|cx| ChartPanel::new(session.clone(), cx));
        let inspector = cx.new(|cx| InspectorPanel::new(session.clone(), window, cx));
        let preview =
            cx.new(|cx| PreviewPanel::new(session.clone(), search_input.clone(), window, cx));
        let (dock, _) = DockSkin::dock_area("ohmypipeline", Some(1), window, cx);
        dock.update(cx, |area, cx| {
            let layout = DockLayout::v_split()
                .child(
                    DockLayout::h_split()
                        .child(
                            DockLayout::tabs().panel_view(panel_handle(palette), cx),
                            Some(px(220.)),
                        )
                        .child(
                            DockLayout::tabs()
                                .panel_view(panel_handle(graph_panel.clone()), cx)
                                .panel_view(panel_handle(chart.clone()), cx),
                            None,
                        )
                        .child(
                            DockLayout::tabs().panel_view(panel_handle(inspector), cx),
                            Some(px(300.)),
                        ),
                    None,
                )
                .child(
                    DockLayout::tabs().panel_view(panel_handle(preview), cx),
                    Some(px(240.)),
                );
            area.set_center(layout, window, cx);
        });
        let chart_for_select = chart.clone();
        let dock_for_select = dock.clone();
        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(
            &canvas,
            window,
            move |this, _, event, window, cx| match event {
                CanvasEvent::Edited => this.on_graph_edit(cx),
                CanvasEvent::SelectionChanged => {
                    this.on_graph_edit(cx);
                    let chart_selected = this
                        .session
                        .read(cx)
                        .canvas
                        .read(cx)
                        .selected_node()
                        .is_some_and(|node| node.feature == "chart");
                    if chart_selected {
                        let id =
                            gpui_kit::component::dock::PanelId::from(chart_for_select.entity_id());
                        dock_for_select.update(cx, |area, cx| area.select_panel(id, window, cx));
                    }
                    if this.session.read(cx).running {
                        this.session.update(cx, |session, _| {
                            session.generation = session.generation.wrapping_add(1);
                            session.running = false;
                            session.worker.cancel();
                        });
                    }
                }
                CanvasEvent::FocusInspector | CanvasEvent::OpenCatalog { .. } => {}
            },
        ));
        // Replies are applied on the next frames while a run is in progress.
        if let Some(ready) = options.ready_file.clone() {
            cx.spawn(async move |_this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(2500))
                    .await;
                let _ = std::fs::write(ready, "ready\n");
            })
            .detach();
        }
        let focus = cx.focus_handle();
        let handle = canvas.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        let mut workspace = Self {
            session,
            dock: dock.clone(),
            focus,
            polling: false,
            path_input,
            search_input,
            _subs: subs,
        };
        let chart_selected = workspace
            .session
            .read(cx)
            .canvas
            .read(cx)
            .selected_node()
            .is_some_and(|node| node.feature == "chart");
        if chart_selected {
            let id = gpui_kit::component::dock::PanelId::from(chart.entity_id());
            dock.update(cx, |area, cx| area.select_panel(id, window, cx));
        }
        if matches!(
            options.demo.as_deref(),
            Some("preview") | Some("chart") | Some("dark") | Some("light")
        ) {
            workspace.run(RunMode::Preview, window, cx);
        }
        workspace
    }

    fn on_graph_edit(&mut self, cx: &mut Context<Self>) {
        let fp = {
            let session = self.session.read(cx);
            run_fingerprint(session.canvas.read(cx).editor().graph())
        };
        self.session.update(cx, |session, cx| {
            if session.preview_fp != Some(fp) {
                session.full_enabled = false;
            }
            session.recompute_dots(cx);
            cx.notify();
        });
    }

    fn run(&mut self, mode: RunMode, win: &mut Window, cx: &mut Context<Self>) {
        if mode == RunMode::Full && !self.session.read(cx).full_enabled {
            return;
        }
        let Some(node) = self.session.read(cx).target_node(cx) else {
            self.session.update(cx, |session, cx| {
                session.status = "select a node to run".into();
                cx.notify();
            });
            return;
        };
        let (graph, base) = {
            let session = self.session.read(cx);
            let canvas = session.canvas.read(cx);
            let graph = canvas.editor().graph().clone();
            let base = canvas
                .editor()
                .path()
                .and_then(|path| path.parent().map(|path| path.display().to_string()));
            (graph, base)
        };
        if let Some(python) = ancestors(&graph, &node).ok().and_then(|order| {
            order
                .into_iter()
                .find(|id| graph.node(id).is_some_and(|node| node.feature == "python"))
        }) {
            self.session.update(cx, |session, cx| {
                session.canvas.update(cx, |canvas, cx| {
                    canvas.select_only(python, cx);
                });
            });
        }
        let generation = self.session.read(cx).generation.wrapping_add(1);
        let window = if mode == RunMode::Full {
            TABLE_WINDOW
        } else {
            PREVIEW_ROW_CAP
        };
        let job = Job {
            generation,
            mode,
            graph,
            node,
            base_dir: base,
            inputs: BTreeMap::new(),
            op: JobOp::Preview,
            output: None,
            window: Some(window),
        };
        self.session.update(cx, |session, cx| {
            session.generation = generation;
            session.mode = mode;
            session.running = true;
            session.error = None;
            session.worker.submit(job);
            session.recompute_dots(cx);
            cx.notify();
        });
        self.arm_poll(win, cx);
    }

    fn arm_poll(&mut self, win: &mut Window, cx: &mut Context<Self>) {
        if self.polling {
            return;
        }
        self.polling = true;
        let this = cx.entity().downgrade();
        win.on_next_frame(move |win, cx| Self::pump(this, win, cx));
    }

    fn pump(this: gpui_kit::WeakEntity<Self>, win: &mut Window, cx: &mut App) {
        let again = this
            .update(cx, |this, cx| {
                this.poll_worker(cx);
                let running = this.session.read(cx).running;
                if !running {
                    this.polling = false;
                }
                running
            })
            .unwrap_or(false);
        if again {
            win.on_next_frame(move |win, cx| Self::pump(this, win, cx));
        }
    }

    fn poll_worker(&mut self, cx: &mut Context<Self>) {
        let message = self.session.read(cx).worker.try_recv();
        let Some((generation, result)) = message else {
            return;
        };
        if generation != self.session.read(cx).generation {
            return;
        }
        match result {
            Ok(reply) => {
                let fp = run_fingerprint(self.session.read(cx).canvas.read(cx).editor().graph());
                self.session.update(cx, |session, cx| {
                    session.running = false;
                    if let Some(error) = reply.error {
                        session.error = Some(error.message);
                        session.full_enabled = false;
                        session.status = session.error.clone().unwrap_or_default();
                    } else {
                        session.error = None;
                        session.columns = reply.columns;
                        session.dtypes = reply.dtypes;
                        session.index = reply.index;
                        session.rows = reply.rows;
                        session.n = reply.n;
                        let shown = session.rows.len();
                        session.status = status_line(reply.mode, reply.n, shown);
                        if reply.mode == RunMode::Preview {
                            session.full_enabled = true;
                            session.preview_fp = Some(fp);
                        }
                    }
                    session.recompute_dots(cx);
                    cx.notify();
                });
            }
            Err(err) => {
                self.session.update(cx, |session, cx| {
                    session.running = false;
                    if !err.contains("broken") && !err.contains("EOF") && !err.contains("os error")
                    {
                        session.error = Some(err);
                    }
                    session.recompute_dots(cx);
                    cx.notify();
                });
            }
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let result: Result<(), String> = self.session.update(cx, |session, cx| {
            session.canvas.update(cx, |canvas, cx| {
                let editor = canvas.editor_mut();
                if editor.path().is_none() {
                    return Err("save-as".into());
                }
                editor.save().map_err(|err| err.to_string())?;
                cx.emit(CanvasEvent::Edited);
                Ok(())
            })
        });
        if result.is_err() {
            self.session.update(cx, |session, cx| {
                session.path_prompt = Some(PathKind::SaveAs);
                cx.notify();
            });
        }
    }

    fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match graph::Document::load(&path) {
            Ok(doc) => {
                self.session.update(cx, |session, cx| {
                    session
                        .canvas
                        .update(cx, |canvas, cx| canvas.load_document(doc, cx));
                    session.path_prompt = None;
                    session.rows.clear();
                    session.full_enabled = false;
                    cx.notify();
                });
            }
            Err(err) => {
                self.session.update(cx, |session, cx| {
                    session.error = Some(err.to_string());
                    cx.notify();
                });
            }
        }
    }

    fn confirm_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.path_input.read(cx).value().to_string();
        let kind = self.session.read(cx).path_prompt.is_some();
        if !kind || text.is_empty() {
            return;
        }
        let path = PathBuf::from(text);
        let save = matches!(self.session.read(cx).path_prompt, Some(PathKind::SaveAs));
        if save {
            let result = self.session.update(cx, |session, cx| {
                session.canvas.update(cx, |canvas, _| {
                    canvas
                        .editor_mut()
                        .save_as(&path)
                        .map_err(|err| err.to_string())
                })
            });
            if let Err(err) = result {
                self.session.update(cx, |session, cx| {
                    session.error = Some(err);
                    cx.notify();
                });
            } else {
                self.session.update(cx, |session, cx| {
                    session.path_prompt = None;
                    cx.notify();
                });
            }
        } else {
            self.open_path(path, cx);
        }
        let _ = window;
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.poll_worker(cx);
        let (title, dirty, readonly, full_enabled, prompt, narrow) = {
            let session = self.session.read(cx);
            let canvas = session.canvas.read(cx);
            let title = canvas
                .editor()
                .path()
                .and_then(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "untitled.pp".into());
            let dirty = if canvas.editor().is_dirty() {
                " ●"
            } else {
                ""
            };
            let readonly = if canvas.editor().is_readonly() {
                "  read-only"
            } else {
                ""
            };
            (
                title,
                dirty,
                readonly,
                session.full_enabled,
                session.path_prompt.is_some(),
                window.viewport_size().width < px(960.0),
            )
        };

        div()
            .id("workspace")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|this, _: &RunPreview, win, cx| this.run(RunMode::Preview, win, cx)),
            )
            .on_action(cx.listener(|this, _: &SaveFile, _, cx| this.save(cx)))
            .on_action(cx.listener(|this, _: &OpenFile, _, cx| {
                this.session.update(cx, |session, cx| {
                    session.path_prompt = Some(PathKind::Open);
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|this, _: &UndoEdit, window, cx| {
                if window.focused_input(cx).is_some() {
                    return;
                }
                this.session.update(cx, |session, cx| {
                    session.canvas.update(cx, |canvas, cx| canvas.undo(cx));
                });
            }))
            .on_action(cx.listener(|this, _: &RedoEdit, window, cx| {
                if window.focused_input(cx).is_some() {
                    return;
                }
                this.session.update(cx, |session, cx| {
                    session.canvas.update(cx, |canvas, cx| canvas.redo(cx));
                });
            }))
            .on_action(cx.listener(|this, _: &DeleteSelection, window, cx| {
                if window.focused_input(cx).is_some() {
                    return;
                }
                this.session.update(cx, |session, cx| {
                    session
                        .canvas
                        .update(cx, |canvas, cx| canvas.delete_selection(cx));
                });
            }))
            .on_action(cx.listener(|this, _: &DuplicateSelection, window, cx| {
                if window.focused_input(cx).is_some() {
                    return;
                }
                this.session.update(cx, |session, cx| {
                    session
                        .canvas
                        .update(cx, |canvas, cx| canvas.duplicate_selection(cx));
                });
            }))
            .on_action(cx.listener(|this, _: &FitView, _, cx| {
                this.session.update(cx, |session, cx| {
                    session.canvas.update(cx, |canvas, cx| canvas.fit(cx));
                });
            }))
            .on_action(cx.listener(|this, _: &ReduceMotion, _, cx| {
                this.session.update(cx, |session, cx| {
                    session.reduced_motion = !session.reduced_motion;
                    let reduced = session.reduced_motion;
                    session
                        .canvas
                        .update(cx, |canvas, cx| canvas.set_reduced_motion(reduced, cx));
                    save_reduced_motion(reduced);
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|_this, _: &ToggleTheme, window, cx| {
                let dark = cx.theme().mode.is_dark();
                let next = if dark {
                    gpui_kit::component::theme::ThemeMode::Light
                } else {
                    gpui_kit::component::theme::ThemeMode::Dark
                };
                gpui_kit::component::theme::Theme::change(next, Some(window), cx);
            }))
            .on_action(cx.listener(|this, _: &QuitApp, window, cx| {
                let dirty = this.session.read(cx).canvas.read(cx).editor().is_dirty();
                let name = this
                    .session
                    .read(cx)
                    .canvas
                    .read(cx)
                    .editor()
                    .path()
                    .and_then(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| "untitled.pp".into());
                if dirty {
                    let session = this.session.clone();
                    window.open_alert_dialog(cx, move |alert, _, _| {
                        alert
                            .title("Save changes?")
                            .description(format!("{name} has unsaved changes."))
                            .ok_text("Save")
                            .cancel_text("Don't save")
                            .show_cancel(true)
                            .on_ok({
                                let session = session.clone();
                                move |_, _, cx| {
                                    session.update(cx, |session, cx| {
                                        let _ = session
                                            .canvas
                                            .update(cx, |canvas, _| canvas.editor_mut().save());
                                    });
                                    cx.quit();
                                    true
                                }
                            })
                            .on_cancel(|_, _, cx| {
                                cx.quit();
                                true
                            })
                    });
                } else {
                    cx.quit();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new().child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .w_full()
                        .px_3()
                        .child(
                            div()
                                .text_sm()
                                .child(format!("Ohmypipeline — {title}{dirty}{readonly}")),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_2()
                                .child(
                                    Button::new("run")
                                        .small()
                                        .primary()
                                        .label("Preview")
                                        .on_click(cx.listener(|this, _, win, cx| {
                                            this.run(RunMode::Preview, win, cx)
                                        })),
                                )
                                .child(
                                    Button::new("full")
                                        .small()
                                        .label("Full dataset")
                                        .disabled(!full_enabled)
                                        .on_click(cx.listener(|this, _, win, cx| {
                                            this.run(RunMode::Full, win, cx)
                                        })),
                                ),
                        ),
                ),
            )
            .child(self.menu_row(narrow, cx))
            .child(div().flex_1().min_h(px(0.)).child(self.dock.clone()))
            .when(prompt, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(120.))
                        .left(px(280.))
                        .w(px(420.))
                        .p_4()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().background)
                        .border_1()
                        .border_color(cx.theme().border)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child("Path")
                        .child(Input::new(&self.path_input))
                        .child(
                            Button::new("confirm-path")
                                .primary()
                                .label("Open")
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.confirm_path(window, cx)
                                    }),
                                ),
                        ),
                )
            })
    }
}

impl Workspace {
    fn menu_row(&self, narrow: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let _ = narrow;
        div()
            .flex()
            .flex_row()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("file-open")
                    .ghost()
                    .small()
                    .label("Open")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.path_prompt = Some(PathKind::Open);
                            cx.notify();
                        });
                    })),
            )
            .child(
                Button::new("file-save")
                    .ghost()
                    .small()
                    .label("Save")
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
            .child(
                Button::new("edit-undo")
                    .ghost()
                    .small()
                    .label("Undo")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.canvas.update(cx, |canvas, cx| canvas.undo(cx))
                        });
                    })),
            )
            .child(
                Button::new("edit-redo")
                    .ghost()
                    .small()
                    .label("Redo")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.canvas.update(cx, |canvas, cx| canvas.redo(cx))
                        });
                    })),
            )
            .child(
                Button::new("node-dup")
                    .ghost()
                    .small()
                    .label("Duplicate")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session
                                .canvas
                                .update(cx, |canvas, cx| canvas.duplicate_selection(cx))
                        });
                    })),
            )
            .child(
                Button::new("node-del")
                    .ghost()
                    .small()
                    .label("Delete")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session
                                .canvas
                                .update(cx, |canvas, cx| canvas.delete_selection(cx))
                        });
                    })),
            )
            .child(
                Button::new("view-fit")
                    .ghost()
                    .small()
                    .label("Fit")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.canvas.update(cx, |canvas, cx| canvas.fit(cx))
                        });
                    })),
            )
            .child(
                Button::new("view-motion")
                    .ghost()
                    .small()
                    .label("Reduce motion")
                    .on_click(cx.listener(|_this, _, _, cx| {
                        cx.dispatch_action(&ReduceMotion);
                    })),
            )
            .child(
                Button::new("view-theme")
                    .ghost()
                    .small()
                    .label("Theme")
                    .on_click(cx.listener(|_this, _, window, cx| {
                        cx.dispatch_action(&ToggleTheme);
                        let _ = window;
                    })),
            )
            .child(
                Button::new("tool-select")
                    .ghost()
                    .small()
                    .label("Select")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session
                                .canvas
                                .update(cx, |canvas, cx| canvas.set_tool(Tool::Select, cx))
                        });
                    })),
            )
            .child(
                Button::new("tool-hand")
                    .ghost()
                    .small()
                    .label("Hand")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session
                                .canvas
                                .update(cx, |canvas, cx| canvas.set_tool(Tool::Hand, cx))
                        });
                    })),
            )
            .child(
                Button::new("tool-note")
                    .ghost()
                    .small()
                    .label("Note")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session
                                .canvas
                                .update(cx, |canvas, cx| canvas.set_tool(Tool::Note, cx))
                        });
                    })),
            )
            .into_any_element()
    }
}

fn load_demo(
    demo: &Option<String>,
    canvas: &Entity<CanvasModel>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(demo) = demo else {
        return;
    };
    if demo == "dark" {
        gpui_kit::component::theme::Theme::change(
            gpui_kit::component::theme::ThemeMode::Dark,
            Some(window),
            cx,
        );
    }
    if demo == "light" {
        gpui_kit::component::theme::Theme::change(
            gpui_kit::component::theme::ThemeMode::Light,
            Some(window),
            cx,
        );
    }
    if demo == "empty" {
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let csv = root.join("examples/orders.csv");
    let csv = csv.canonicalize().unwrap_or(csv);
    canvas.update(cx, |canvas, cx| {
        let mut doc = graph::Document::new();
        let read = doc
            .add_node("read_csv", graph::Point::new(48.0, 64.0))
            .unwrap();
        doc.set_param(&read, "path", json!(csv.display().to_string()));
        doc.set_label(&read, "orders.csv".into());
        let head = doc
            .add_node("head", graph::Point::new(320.0, 64.0))
            .unwrap();
        doc.set_param(&head, "n", json!(5));
        if demo != "link-snap" {
            let _ = doc.connect(&read, "out", &head, "in");
        }
        let headers = doc
            .add_node("change_headers", graph::Point::new(592.0, 64.0))
            .unwrap();
        doc.set_param(
            &headers,
            "mapping",
            json!([{"from": "amount", "to": "revenue"}, {"from": "region", "to": ""}]),
        );
        let _ = doc.connect(&head, "out", &headers, "in");
        if demo == "chart" || demo == "dark" || demo == "light" {
            let chart = doc
                .add_node("chart", graph::Point::new(48.0, 280.0))
                .unwrap();
            doc.set_param(&chart, "mark", json!("bar"));
            doc.set_param(&chart, "x", json!("month"));
            doc.set_param(&chart, "y", json!("amount"));
            let _ = doc.connect(&read, "out", &chart, "in");
            canvas.load_document(doc, cx);
            canvas.select_only(chart, cx);
        } else if demo == "inspector" {
            canvas.load_document(doc, cx);
            canvas.select_only(headers, cx);
        } else if demo == "link-snap" {
            canvas.load_document(doc, cx);
            canvas.seed_link_drag(&read, "out", (300.0, 130.0), cx);
        } else if demo == "link-reject" {
            let column = doc
                .add_node("column", graph::Point::new(48.0, 280.0))
                .unwrap();
            doc.set_param(&column, "name", json!("amount"));
            let _ = doc.connect(&read, "out", &column, "in");
            canvas.load_document(doc, cx);
            canvas.seed_link_drag(&column, "out", (592.0, 140.0), cx);
        } else {
            canvas.load_document(doc, cx);
            canvas.select_only(&headers, cx);
        }
    });
}

fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|dir| !dir.is_empty())
    {
        return PathBuf::from(dir).join("ohmypipeline");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config").join("ohmypipeline")
}

fn load_reduced_motion() -> bool {
    let path = config_dir().join("settings.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.get("reduced_motion").and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

fn save_reduced_motion(reduced: bool) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(
        dir.join("settings.json"),
        json!({"reduced_motion": reduced}).to_string(),
    );
}

struct PalettePanel {
    focus: FocusHandle,
    session: Entity<Session>,
}

impl PalettePanel {
    fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            session,
        }
    }
}

impl EventEmitter<PanelEvent> for PalettePanel {}
impl Focusable for PalettePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl BasePanel for PalettePanel {
    fn panel_name(&self) -> &'static str {
        "Palette"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
}
impl Panel for PalettePanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Features"
    }
}
impl Render for PalettePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let narrow = window.viewport_size().width < px(960.0);
        let mut column = div()
            .id("palette")
            .size_full()
            .flex()
            .flex_col()
            .p_2()
            .gap_2()
            .overflow_hidden();
        for group in graph::groups() {
            column = column.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(*group),
            );
            for feature in graph::features_in(group) {
                let id = feature.id.to_string();
                let title = if narrow {
                    feature.title.chars().next().unwrap_or('·').to_string()
                } else {
                    feature.title.to_string()
                };
                let session = self.session.clone();
                column = column.child(
                    Button::new(SharedString::from(format!("feature-{id}")))
                        .ghost()
                        .small()
                        .label(title)
                        .on_click(move |_, _, cx| {
                            session.update(cx, |session, cx| {
                                session.canvas.update(cx, |canvas, cx| {
                                    let _ = canvas.add_feature(&id, cx);
                                });
                            });
                        }),
                );
            }
        }
        column
    }
}

struct GraphPanel {
    focus: FocusHandle,
    canvas: Entity<CanvasModel>,
}
impl GraphPanel {
    fn new(canvas: Entity<CanvasModel>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            canvas,
        }
    }
}
impl EventEmitter<PanelEvent> for GraphPanel {}
impl Focusable for GraphPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl BasePanel for GraphPanel {
    fn panel_name(&self) -> &'static str {
        "Graph"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
}
impl Panel for GraphPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Graph"
    }
}
impl Render for GraphPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.canvas.clone())
    }
}

struct ChartPanel {
    focus: FocusHandle,
    session: Entity<Session>,
    drag: Option<(f32, f32)>,
}
impl ChartPanel {
    fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            session,
            drag: None,
        }
    }
}
impl EventEmitter<PanelEvent> for ChartPanel {}
impl Focusable for ChartPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl BasePanel for ChartPanel {
    fn panel_name(&self) -> &'static str {
        "Chart"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
}
impl Panel for ChartPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Chart"
    }
}
impl Render for ChartPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (node, rows, columns, n, reduced, error) = {
            let session = self.session.read(cx);
            let canvas = session.canvas.read(cx);
            (
                canvas
                    .selected_node()
                    .filter(|node| node.feature == "chart")
                    .cloned(),
                session.rows.clone(),
                session.columns.clone(),
                session.n,
                session.reduced_motion,
                session.error.clone(),
            )
        };
        let Some(node) = node else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child("Select a chart feature")
                .into_any_element();
        };
        let x_name = node.param_str("x");
        let y_name = node.param_str("y");
        let mark = node.param_str("mark");
        let y_index = columns.iter().position(|col| col == &y_name);
        let x_index = columns.iter().position(|col| col == &x_name);
        let mut data = Vec::new();
        let mut numeric = true;
        for row in &rows {
            let x = x_index
                .and_then(|ix| row.get(ix))
                .map(cell_text)
                .unwrap_or_else(|| data.len().to_string());
            let y = y_index
                .and_then(|ix| row.get(ix))
                .and_then(|value| match value {
                    Value::Number(n) => n.as_f64().map(|n| n as f32),
                    Value::String(text) => text.parse().ok(),
                    _ => None,
                });
            match y {
                Some(y) => data.push(Datum { x, y }),
                None => numeric = false,
            }
        }
        if !numeric && y_index.is_some() {
            return div()
                .p_4()
                .text_color(cx.theme().danger)
                .child(format!("{y_name} is not numeric"))
                .into_any_element();
        }
        if n > CHART_POINT_CAP as u64 {
            return div()
                .p_4()
                .child(format!("{n} points. Aggregate or sample before charting."))
                .into_any_element();
        }
        let title = node
            .presentation
            .as_ref()
            .and_then(|item| item.title.text.clone())
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "Chart".into());
        let color = cx.theme().chart_1;
        let note = if error.is_some() {
            error.unwrap_or_default()
        } else {
            "preview · the chart draws this window".into()
        };
        let _ = self.drag;
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_3()
            .gap_2()
            .child(div().text_sm().child(title))
            .child(div().flex_1().min_h(px(0.)).child(FeaturePlot {
                mark,
                data,
                color,
                reduced_motion: reduced,
            }))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(note),
            )
            .into_any_element()
    }
}

struct InspectorPanel {
    focus: FocusHandle,
    session: Entity<Session>,
    fields: Vec<InspectorField>,
    bound: Option<String>,
}
struct InspectorField {
    name: String,
    input: Entity<InputState>,
    _sub: Subscription,
}
impl InspectorPanel {
    fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            session,
            fields: Vec::new(),
            bound: None,
        }
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let node = self
            .session
            .read(cx)
            .canvas
            .read(cx)
            .selected_node()
            .cloned();
        let id = node.as_ref().map(|node| node.id.clone());
        if id == self.bound && node.is_some() {
            return;
        }
        self.bound = id.clone();
        self.fields.clear();
        let Some(node) = node else {
            return;
        };
        let Some(def) = node.def() else {
            return;
        };
        for param in def.params {
            let name = param.name.to_string();
            let text = node.param_str(&name);
            let input = cx.new(|cx| InputState::new(window, cx));
            input.update(cx, |state, cx| state.set_value(text, window, cx));
            let session = self.session.clone();
            let node_id = node.id.clone();
            let param_name = name.clone();
            let sub = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Focus) {
                    this.session.update(cx, |session, cx| {
                        session.canvas.update(cx, |canvas, _| canvas.begin_edit());
                    });
                }
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    let value = coerce(&param_name, &text);
                    session.update(cx, |session, cx| {
                        session.canvas.update(cx, |canvas, cx| {
                            canvas.set_param(&node_id, &param_name, value, cx);
                        });
                    });
                }
                if matches!(event, InputEvent::Blur) {
                    this.session.update(cx, |session, cx| {
                        session
                            .canvas
                            .update(cx, |canvas, cx| canvas.commit_edit(cx));
                    });
                }
            });
            self.fields.push(InspectorField {
                name,
                input,
                _sub: sub,
            });
        }
        cx.notify();
    }
}
impl EventEmitter<PanelEvent> for InspectorPanel {}
impl Focusable for InspectorPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl BasePanel for InspectorPanel {
    fn panel_name(&self) -> &'static str {
        "Inspector"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
}
impl Panel for InspectorPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Inspector"
    }
}
impl Render for InspectorPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(window, cx);
        let node = self
            .session
            .read(cx)
            .canvas
            .read(cx)
            .selected_node()
            .cloned();
        let Some(node) = node else {
            return div()
                .p_3()
                .text_color(cx.theme().muted_foreground)
                .child("Nothing selected")
                .into_any_element();
        };
        let mut body = div()
            .id("inspector")
            .size_full()
            .flex()
            .flex_col()
            .p_3()
            .gap_2()
            .overflow_hidden();
        body = body.child(div().text_sm().child(node.label.clone()));
        body = body.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(node.feature.clone()),
        );
        if node.feature == "python" {
            body = body.child(div().text_xs().child("This block runs with the pipeline."));
        }
        for field in &self.fields {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(field.name.clone()),
            );
            body = body.child(Input::new(&field.input).small());
        }
        if node.feature == "change_headers"
            && let Some(dup) = node
                .params
                .get("mapping")
                .map(header_pairs)
                .as_deref()
                .and_then(duplicate_headers)
        {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(format!("duplicate header {dup}")),
            );
        }
        if let Some(error) = &self.session.read(cx).error {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        let _ = node.summary();
        body.into_any_element()
    }
}

fn coerce(name: &str, text: &str) -> Value {
    if name == "n" || name == "bins" {
        json!(text.parse::<i64>().unwrap_or(0))
    } else if name == "ascending" {
        json!(text == "true")
    } else if name == "mapping" {
        serde_json::from_str(text).unwrap_or_else(|_| json!(text))
    } else {
        json!(text)
    }
}

struct PreviewPanel {
    focus: FocusHandle,
    session: Entity<Session>,
    table: Entity<TableState<PreviewTable>>,
    search: Entity<InputState>,
    _sub: Subscription,
}
impl PreviewPanel {
    fn new(
        session: Entity<Session>,
        search: Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let table = cx.new(|cx| {
            TableState::new(
                PreviewTable {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    index: Vec::new(),
                },
                window,
                cx,
            )
        });
        let table_for_sub = table.clone();
        let sub = cx.observe(&session, move |_, session, cx| {
            let session = session.read(cx);
            let columns = session.columns.clone();
            let rows = session.rows.clone();
            let index = session.index.clone();
            let search = session.search.to_lowercase();
            table_for_sub.update(cx, |table, cx| {
                let delegate = table.delegate_mut();
                delegate.columns = columns;
                delegate.index = index;
                delegate.rows = if search.is_empty() {
                    rows
                } else {
                    rows.into_iter()
                        .filter(|row| {
                            row.iter()
                                .any(|cell| cell_text(cell).to_lowercase().contains(&search))
                        })
                        .collect()
                };
                table.refresh(cx);
            });
        });
        Self {
            focus: cx.focus_handle(),
            session,
            table,
            search,
            _sub: sub,
        }
    }
}
impl EventEmitter<PanelEvent> for PreviewPanel {}
impl Focusable for PreviewPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl BasePanel for PreviewPanel {
    fn panel_name(&self) -> &'static str {
        "Preview"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
}
impl Panel for PreviewPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Preview"
    }
}
impl Render for PreviewPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.session.read(cx).status.clone();
        let session = self.session.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(status),
            )
            .child(
                div()
                    .px_3()
                    .py_1()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .child(Input::new(&self.search).small())
                    .child(
                        Button::new("add-search")
                            .small()
                            .ghost()
                            .label("Add as node")
                            .on_click(move |_, _, cx| {
                                session.update(cx, |session, cx| {
                                    let expr = session.search.clone();
                                    if expr.is_empty() {
                                        return;
                                    }
                                    session.canvas.update(cx, |canvas, cx| {
                                        if let Some(id) = canvas.add_feature("query", cx) {
                                            canvas.set_param(&id, "expr", json!(expr), cx);
                                        }
                                    });
                                });
                            }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .child(DataTable::new(&self.table)),
            )
    }
}

struct PreviewTable {
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
    index: Vec<String>,
}
impl TableDelegate for PreviewTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len() + 1
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }
    fn column(&self, col_ix: usize, _: &App) -> Column {
        if col_ix == 0 {
            Column::new("index", "index")
                .width(72.)
                .fixed(ColumnFixed::Left)
                .resizable(false)
        } else {
            let name = self
                .columns
                .get(col_ix - 1)
                .map(String::as_str)
                .unwrap_or("");
            let numeric = self
                .rows
                .first()
                .and_then(|row| row.get(col_ix - 1))
                .is_some_and(|cell| cell.is_number());
            let column = Column::new(name, name).width(140.).sortable();
            if numeric { column.text_right() } else { column }
        }
    }
    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let mono = cx.theme().mono_font_family.clone();
        let text = if col_ix == 0 {
            self.index.get(row_ix).cloned().unwrap_or_default()
        } else {
            self.rows
                .get(row_ix)
                .and_then(|row| row.get(col_ix - 1))
                .map(cell_text)
                .unwrap_or_else(|| "—".into())
        };
        div().font_family(mono).child(text)
    }
    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        if col_ix == 0 {
            return;
        }
        let ix = col_ix - 1;
        let desc = matches!(sort, ColumnSort::Descending);
        self.rows.sort_by(|a, b| {
            let left = a.get(ix).map(cell_text).unwrap_or_default();
            let right = b.get(ix).map(cell_text).unwrap_or_default();
            if desc {
                right.cmp(&left)
            } else {
                left.cmp(&right)
            }
        });
    }
}

pub fn run_gui(options: GuiOptions) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.bind_keys(vec![
                gpui_kit::KeyBinding::new("ctrl-o", OpenFile, None),
                gpui_kit::KeyBinding::new("ctrl-s", SaveFile, None),
                gpui_kit::KeyBinding::new("ctrl-enter", RunPreview, None),
                gpui_kit::KeyBinding::new("ctrl-z", UndoEdit, None),
                gpui_kit::KeyBinding::new("ctrl-shift-z", RedoEdit, None),
                gpui_kit::KeyBinding::new("backspace", DeleteSelection, None),
                gpui_kit::KeyBinding::new("ctrl-d", DuplicateSelection, None),
                gpui_kit::KeyBinding::new("ctrl-0", FitView, None),
            ]);
            let mut window_options = TitleBar::window_options();
            window_options.window_min_size = Some(gpui_kit::size(px(960.0), px(640.0)));
            window_options.window_bounds = Some(gpui_kit::WindowBounds::Windowed(
                gpui_kit::Bounds::centered(None, gpui_kit::size(px(1280.0), px(800.0)), cx),
            ));
            #[cfg(target_os = "linux")]
            {
                window_options.window_decorations = Some(gpui_kit::WindowDecorations::Client);
            }
            let options = options;
            let _ = gpui_kit::open_window(window_options, cx, move |window, cx| {
                cx.new(|cx| Workspace::new(options, window, cx))
            });
            cx.activate(true);
        });
}
