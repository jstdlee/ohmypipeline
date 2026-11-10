//! Document window. Dock holds the palette, the canvas, the chart, the inspector, and the preview.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use canvas::{CanvasEvent, CanvasModel, Dot, Side, Tool, feature_chart, points_from, port_slots};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::{
    BasePanel, DockArea, DockLayout, DockSkin, Panel, PanelEvent, panel_handle,
};
use gpui_kit::component::empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::table::{
    Column, ColumnFixed, ColumnSort, DataTable, TableDelegate, TableState,
};
use gpui_kit::component::toolbar::{Toolbar, ToolbarGroup};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _};
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

use crate::chart_plot::FeaturePlot;
use crate::worker::Worker;

actions!(
    ompp,
    [
        OpenFile,
        SaveFile,
        SaveFileAs,
        RunPreview,
        RunFull,
        UndoEdit,
        RedoEdit,
        DeleteSelection,
        DuplicateSelection,
        FitView,
        ReduceMotion,
        ToggleTheme,
        OpenModelSettings,
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
    pub ran_node: Option<String>,
    pub error: Option<String>,
    pub search: String,
    #[allow(dead_code)]
    pub sort: Option<(usize, bool)>,
    pub reduced_motion: bool,
    pub worker: Worker,
    pub path_prompt: Option<PathKind>,
    pub assist_note: Option<String>,
    pub assist_fill: Option<(String, String, String)>,
    pub assist_rx: Option<std::sync::mpsc::Receiver<crate::assist::AssistReply>>,
    frames: BTreeMap<String, CachedFrame>,
}

#[derive(Clone)]
struct CachedFrame {
    columns: Vec<String>,
    dtypes: Vec<String>,
    index: Vec<String>,
    rows: Vec<Vec<Value>>,
    n: u64,
    status: String,
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

    fn show_cached(&mut self, id: &str) -> bool {
        let Some(frame) = self.frames.get(id).cloned() else {
            return false;
        };
        self.columns = frame.columns;
        self.dtypes = frame.dtypes;
        self.index = frame.index;
        self.rows = frame.rows;
        self.n = frame.n;
        self.status = frame.status;
        self.error = None;
        true
    }

    fn recompute_dots(&self, cx: &mut App) {
        let canvas = self.canvas.read(cx);
        let graph = canvas.editor().graph();
        let mut dots = BTreeMap::new();
        let live = self
            .ran_node
            .as_deref()
            .and_then(|id| ancestors(graph, id).ok())
            .unwrap_or_default();
        let done = !self.running && self.error.is_none() && self.preview_fp.is_some();
        for node in &graph.nodes {
            let dot = if node.unknown {
                Dot::Danger
            } else if self.running && live.iter().any(|id| id == &node.id) {
                Dot::Running
            } else if self.error.is_some() && self.ran_node.as_deref() == Some(node.id.as_str()) {
                Dot::Danger
            } else if done && live.iter().any(|id| id == &node.id) {
                Dot::Success
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
    model_endpoint: Entity<InputState>,
    model_name: Entity<InputState>,
    model_key: Entity<InputState>,
    model_open: bool,
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
        let stored_motion = load_reduced_motion();
        let reduced = stored_motion.unwrap_or_else(|| cx.reduce_motion());
        if stored_motion.is_some() {
            cx.set_reduce_motion(reduced);
        }
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
            ran_node: None,
            error: None,
            search: String::new(),
            sort: None,
            reduced_motion: reduced,
            worker,
            path_prompt: None,
            assist_note: None,
            assist_fill: None,
            assist_rx: None,
            frames: BTreeMap::new(),
        });
        let path_input = cx.new(|cx| InputState::new(window, cx));
        let stored_model = load_model_settings();
        let model_endpoint = cx.new(|cx| InputState::new(window, cx));
        let model_name = cx.new(|cx| InputState::new(window, cx));
        let model_key = cx.new(|cx| InputState::new(window, cx));
        model_endpoint.update(cx, |state, cx| {
            state.set_value(stored_model.endpoint, window, cx);
        });
        model_name.update(cx, |state, cx| {
            state.set_value(stored_model.model, window, cx);
        });
        model_key.update(cx, |state, cx| {
            state.set_value(stored_model.key, window, cx);
        });
        let search_input = cx.new(|cx| InputState::new(window, cx));
        let palette = cx.new(|cx| PalettePanel::new(session.clone(), window, cx));
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
                    this.session.update(cx, |session, cx| {
                        let selected = session
                            .canvas
                            .read(cx)
                            .selected_node()
                            .map(|node| (node.id.clone(), node.feature.clone()));
                        if let Some((id, feature)) = selected
                            && !session.show_cached(&id)
                            && (feature == "chart" || feature == "preview_table")
                        {
                            session.columns.clear();
                            session.dtypes.clear();
                            session.index.clear();
                            session.rows.clear();
                            session.n = 0;
                        }
                        cx.notify();
                    });
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
            model_endpoint,
            model_name,
            model_key,
            model_open: options.demo.as_deref() == Some("assist"),
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
            Some("preview")
                | Some("chart")
                | Some("dark")
                | Some("light")
                | Some("outputs")
                | Some("skip")
                | Some("pie")
                | Some("line")
        ) {
            workspace.run(RunMode::Preview, window, cx);
        }
        let session = workspace.session.clone();
        window.on_next_frame(move |window, _cx| {
            window.on_next_frame(move |_window, cx| {
                session.update(cx, |session, cx| {
                    let empty = session.canvas.read(cx).editor().graph().nodes.is_empty();
                    if !empty {
                        session.canvas.update(cx, |canvas, cx| canvas.fit_all(cx));
                    }
                });
            });
        });
        if options.demo.as_deref() == Some("assist") {
            workspace.session.update(cx, |session, cx| {
                session.assist_note = Some(
                    "Add a model endpoint and key in Settings. They stay on this computer and are not saved in the pipeline."
                        .into(),
                );
                cx.notify();
            });
        }
        if matches!(
            options.demo.as_deref(),
            Some("reel-link" | "reel-run" | "reel-python")
        ) {
            let session = workspace.session.clone();
            let demo = options.demo.clone().unwrap_or_default();
            let started = std::time::Instant::now();
            window.on_next_frame(move |window, cx| {
                play_reel(demo, session, started, window, cx);
            });
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
        let ran = node.clone();
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
            session.ran_node = Some(ran);
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
                let waiting = this.session.read(cx).assist_rx.is_some();
                if !running && !waiting {
                    this.polling = false;
                }
                running || waiting
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
                        let node_id = session.ran_node.clone().unwrap_or_default();
                        let cached = CachedFrame {
                            columns: session.columns.clone(),
                            dtypes: session.dtypes.clone(),
                            index: session.index.clone(),
                            rows: session.rows.clone(),
                            n: session.n,
                            status: session.status.clone(),
                        };
                        let view_rows: Vec<_> = cached.rows.iter().take(64).cloned().collect();
                        let view_columns = cached.columns.clone();
                        let ids = output_views(session.canvas.read(cx).editor().graph(), &node_id);
                        session.frames.insert(node_id.clone(), cached.clone());
                        for id in &ids {
                            session.frames.insert(id.clone(), cached.clone());
                        }
                        session.canvas.update(cx, |canvas, cx| {
                            canvas.set_frames(&ids, view_columns, view_rows, cx);
                        });
                        if reply.mode == RunMode::Preview {
                            session.full_enabled = true;
                            session.preview_fp = Some(fp);
                        }
                    }
                    session.recompute_dots(cx);
                    cx.notify();
                });
                cx.notify();
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
                cx.notify();
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
        let (title, dirty, readonly, full_enabled, prompt, narrow, running, mode, reduced) = {
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
                session.running,
                session.mode,
                session.reduced_motion,
            )
        };

        div()
            .id("workspace")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|this, _: &RunPreview, win, cx| this.run(RunMode::Preview, win, cx)),
            )
            .on_action(cx.listener(|this, _: &RunFull, win, cx| this.run(RunMode::Full, win, cx)))
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
                    cx.set_reduce_motion(reduced);
                    session
                        .canvas
                        .update(cx, |canvas, cx| canvas.set_reduced_motion(reduced, cx));
                    save_reduced_motion(reduced);
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|this, _: &OpenModelSettings, _, cx| {
                this.model_open = !this.model_open;
                cx.notify();
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
            .child(TitleBar::new().child(self.command_bar(
                &title,
                dirty,
                readonly,
                full_enabled,
                running,
                mode,
                reduced,
                narrow,
                cx,
            )))
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
            .when(self.model_open, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(64.))
                        .left(px(248.))
                        .w(px(320.))
                        .p_4()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().background)
                        .border_1()
                        .border_color(cx.theme().border)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child("Model")
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Endpoint, model name, and key stay on this computer."),
                        )
                        .child(div().text_xs().child("Endpoint"))
                        .child(Input::new(&self.model_endpoint).small())
                        .child(div().text_xs().child("Model"))
                        .child(Input::new(&self.model_name).small())
                        .child(div().text_xs().child("Key"))
                        .child(Input::new(&self.model_key).small())
                        .child(
                            Button::new("save-model")
                                .primary()
                                .small()
                                .label("Save")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    save_model_settings(&crate::assist::ModelSettings {
                                        endpoint: this.model_endpoint.read(cx).value().to_string(),
                                        model: this.model_name.read(cx).value().to_string(),
                                        key: this.model_key.read(cx).value().to_string(),
                                    });
                                    this.model_open = false;
                                    cx.notify();
                                })),
                        ),
                )
            })
    }
}

impl Workspace {
    #[allow(clippy::too_many_arguments)]
    fn command_bar(
        &self,
        title: &str,
        dirty: &str,
        readonly: &str,
        full_enabled: bool,
        running: bool,
        mode: RunMode,
        reduced: bool,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let dark = cx.theme().mode.is_dark();
        let preview_label = if running && mode == RunMode::Preview {
            "Running"
        } else {
            "Preview"
        };
        let file_label = format!("{title}{dirty}{readonly}");
        Toolbar::new("command-bar")
            .w_full()
            .child(
                icon_button("file-open", IconName::FolderOpen, "Open", &OpenFile, cx).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.path_prompt = Some(PathKind::Open);
                            cx.notify();
                        });
                    }),
                ),
            )
            .child(
                icon_button("file-save", IconName::Save, "Save", &SaveFile, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
            .child(
                icon_button("edit-undo", IconName::Undo2, "Undo", &UndoEdit, cx).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.canvas.update(cx, |canvas, cx| canvas.undo(cx));
                        });
                    }),
                ),
            )
            .child(
                icon_button("edit-redo", IconName::Redo2, "Redo", &RedoEdit, cx).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.canvas.update(cx, |canvas, cx| canvas.redo(cx));
                        });
                    }),
                ),
            )
            .content(
                ToolbarGroup::new("edit").child(
                    icon_button(
                        "node-dup",
                        IconName::Copy,
                        "Duplicate",
                        &DuplicateSelection,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session
                                .canvas
                                .update(cx, |canvas, cx| canvas.duplicate_selection(cx));
                        });
                    })),
                ),
            )
            .child(
                icon_button("node-del", IconName::Trash, "Delete", &DeleteSelection, cx).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session
                                .canvas
                                .update(cx, |canvas, cx| canvas.delete_selection(cx));
                        });
                    }),
                ),
            )
            .child(
                icon_button("view-fit", IconName::Focus, "Fit", &FitView, cx).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.canvas.update(cx, |canvas, cx| canvas.fit(cx));
                        });
                    }),
                ),
            )
            .content(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .px_3()
                    .overflow_hidden()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(if narrow {
                        title.to_string()
                    } else {
                        file_label
                    }),
            )
            .content(
                Button::new("settings")
                    .ghost()
                    .compact()
                    .icon(IconName::Settings)
                    .tooltip("Settings")
                    .accessibility_label("Settings")
                    .dropdown_menu(move |menu, _, _| {
                        menu.menu_with_check("Dark theme", dark, Box::new(ToggleTheme))
                            .menu_with_check("Reduce motion", reduced, Box::new(ReduceMotion))
                            .separator()
                            .menu("Model settings", Box::new(OpenModelSettings))
                    }),
            )
            .content(
                Button::new("run")
                    .small()
                    .primary()
                    .compact()
                    .icon(IconName::Play)
                    .label(preview_label)
                    .loading(running && mode == RunMode::Preview)
                    .tooltip_with_action("Run preview", &RunPreview, None)
                    .on_click(cx.listener(|this, _, win, cx| this.run(RunMode::Preview, win, cx))),
            )
            .content(
                Button::new("full")
                    .small()
                    .compact()
                    .label(if running && mode == RunMode::Full {
                        "Running"
                    } else {
                        "Full dataset"
                    })
                    .loading(running && mode == RunMode::Full)
                    .disabled(!full_enabled || running)
                    .tooltip("Run the full file. Enabled after a preview of this graph.")
                    .on_click(cx.listener(|this, _, win, cx| this.run(RunMode::Full, win, cx))),
            )
    }
}

fn icon_button(
    id: &str,
    icon: IconName,
    label: &str,
    action: &dyn gpui_kit::Action,
    _cx: &mut gpui_kit::App,
) -> Button {
    Button::new(SharedString::from(id.to_string()))
        .ghost()
        .compact()
        .icon(icon)
        .tooltip_with_action(label.to_string(), action, None)
        .accessibility_label(label.to_string())
}

fn play_reel(
    demo: String,
    session: Entity<Session>,
    started: std::time::Instant,
    window: &mut Window,
    cx: &mut App,
) {
    let t = started.elapsed().as_secs_f32();
    let keep = if demo == "reel-link" {
        session.update(cx, |session, cx| reel_link(session, t, cx));
        t < 3.8
    } else if demo == "reel-python" {
        let ready = session.update(cx, |session, cx| {
            reel_python(session, t, cx);
            t > 2.2
                && !session.running
                && session.ran_node.is_none()
                && session
                    .canvas
                    .read(cx)
                    .editor()
                    .graph()
                    .nodes
                    .iter()
                    .any(|node| node.feature == "python")
        });
        if ready {
            window.dispatch_action(Box::new(RunPreview), cx);
        }
        t < 6.5
    } else {
        let ready = session.update(cx, |session, cx| {
            reel_run(session, t, cx);
            t > 1.9
                && !session.running
                && session.ran_node.is_none()
                && session.canvas.read(cx).editor().graph().nodes.len() >= 3
        });
        if ready {
            window.dispatch_action(Box::new(RunPreview), cx);
        }
        t < 5.2
    };
    if keep {
        let session = session.clone();
        let demo = demo.clone();
        window.on_next_frame(move |window, cx| play_reel(demo, session, started, window, cx));
    }
}

fn node_id(graph: &graph::Graph, feature: &str) -> Option<String> {
    graph
        .nodes
        .iter()
        .find(|node| node.feature == feature)
        .map(|node| node.id.clone())
}

fn slot_at(graph: &graph::Graph, id: &str, name: &str, side: Side) -> Option<graph::Point> {
    let node = graph.node(id)?;
    port_slots(node)
        .into_iter()
        .find(|slot| slot.name == name && slot.side == side)
        .map(|slot| slot.center)
}

fn mix(from: graph::Point, to: graph::Point, t: f32) -> graph::Point {
    let t = t.clamp(0.0, 1.0);
    graph::Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t)
}

fn reel_link(session: &mut Session, t: f32, cx: &mut gpui_kit::Context<Session>) {
    session.canvas.update(cx, |canvas, cx| {
        let graph = canvas.editor().graph().clone();
        let Some(read) = node_id(&graph, "read_csv") else {
            return;
        };
        let Some(head) = node_id(&graph, "head") else {
            return;
        };
        let Some(column) = node_id(&graph, "column") else {
            return;
        };
        let Some(headers) = node_id(&graph, "change_headers") else {
            return;
        };
        if t < 1.8 {
            let Some(from) = slot_at(&graph, &read, "out", Side::Out) else {
                return;
            };
            let Some(to) = slot_at(&graph, &head, "in", Side::In) else {
                return;
            };
            canvas.show_link(&read, "out", mix(from, to, t / 1.8), cx);
        } else {
            let Some(from) = slot_at(&graph, &column, "out", Side::Out) else {
                return;
            };
            let Some(to) = slot_at(&graph, &headers, "in", Side::In) else {
                return;
            };
            canvas.show_link(&column, "out", mix(from, to, (t - 1.8) / 1.5), cx);
        }
    });
}

fn reel_python(session: &mut Session, t: f32, cx: &mut gpui_kit::Context<Session>) {
    let has = session
        .canvas
        .read(cx)
        .editor()
        .graph()
        .nodes
        .iter()
        .any(|node| node.feature == "python");
    if t > 0.4 && !has {
        session.canvas.update(cx, |canvas, cx| {
            if let Some(id) = canvas.add_feature("python", cx) {
                canvas.set_label(&id, "Keep the north rows".into(), cx);
                canvas.set_param(
                    &id,
                    "code",
                    json!("def run(frame):\n    return frame.head(4)\n"),
                    cx,
                );
                canvas.set_python_inputs(
                    &id,
                    vec![graph::PortRecord {
                        name: "frame".into(),
                        accepts: vec![graph::TypeName::Dataframe],
                        multi: false,
                    }],
                    cx,
                );
                if let Some(read) = canvas
                    .editor()
                    .graph()
                    .nodes
                    .iter()
                    .find(|node| node.feature == "read_csv")
                    .map(|node| node.id.clone())
                {
                    let _ = canvas.editor_mut().connect(&read, "out", &id, "frame");
                }
                canvas.fit_all(cx);
            }
        });
    }
}

fn reel_run(session: &mut Session, t: f32, cx: &mut gpui_kit::Context<Session>) {
    let has = |session: &Session, cx: &mut gpui_kit::Context<Session>, feature: &str| {
        session
            .canvas
            .read(cx)
            .editor()
            .graph()
            .nodes
            .iter()
            .any(|node| node.feature == feature)
    };
    if t > 0.45 && !has(session, cx, "head") {
        session.canvas.update(cx, |canvas, cx| {
            if let Some(id) = canvas.add_feature("head", cx) {
                canvas.set_param(&id, "n", json!(5), cx);
                canvas.fit_all(cx);
            }
        });
    }
    if t > 1.15 && !has(session, cx, "change_headers") {
        session.canvas.update(cx, |canvas, cx| {
            if let Some(id) = canvas.add_feature("change_headers", cx) {
                canvas.set_param(
                    &id,
                    "mapping",
                    json!([{"from": "amount", "to": "revenue"}]),
                    cx,
                );
                canvas.fit_all(cx);
            }
        });
    }
}

fn load_sales_demo(demo: &str, canvas: &Entity<CanvasModel>, cx: &mut App) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let csv = root.join("examples/sales.csv");
    let csv = csv.canonicalize().unwrap_or(csv);
    canvas.update(cx, |canvas, cx| {
        let mut doc = graph::Document::new();
        let read = doc
            .add_node("read_csv", graph::Point::new(48.0, 72.0))
            .unwrap();
        doc.set_param(&read, "path", json!(csv.display().to_string()));
        doc.set_param(&read, "skip_rows", json!(1));
        doc.set_param(&read, "header", json!("header row"));
        doc.set_param(&read, "header_row", json!(0));
        doc.set_label(&read, "sales.csv".into());
        let pivot = doc
            .add_node("pivot", graph::Point::new(360.0, 72.0))
            .unwrap();
        doc.set_label(&pivot, "Qty by region".into());
        doc.set_param(&pivot, "index", json!("region"));
        doc.set_param(&pivot, "columns", json!("product"));
        doc.set_param(&pivot, "values", json!("qty"));
        doc.set_param(&pivot, "agg", json!("sum"));
        let table = doc
            .add_node("preview_table", graph::Point::new(360.0, 300.0))
            .unwrap();
        let line = doc
            .add_node("chart", graph::Point::new(760.0, 32.0))
            .unwrap();
        doc.set_label(&line, "Pen by region".into());
        doc.set_param(&line, "mark", json!("line"));
        doc.set_param(&line, "x", json!("region"));
        doc.set_param(&line, "y", json!("Pen"));
        let pie = doc
            .add_node("chart", graph::Point::new(760.0, 300.0))
            .unwrap();
        doc.set_label(&pie, "Book by region".into());
        doc.set_param(&pie, "mark", json!("pie"));
        doc.set_param(&pie, "x", json!("region"));
        doc.set_param(&pie, "y", json!("Book"));
        let _ = doc.connect(&read, "out", &pivot, "in");
        let _ = doc.connect(&pivot, "out", &table, "in");
        let _ = doc.connect(&pivot, "out", &line, "in");
        let _ = doc.connect(&pivot, "out", &pie, "in");
        canvas.load_document(doc, cx);
        let selected = match demo {
            "skip" => read,
            "pie" => pie,
            "line" => line,
            _ => pivot,
        };
        canvas.select_only(selected, cx);
    });
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
    if matches!(demo.as_str(), "outputs" | "skip" | "pie" | "line") {
        load_sales_demo(demo, canvas, cx);
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
        if demo != "link-snap" && demo != "reel-link" {
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
        if demo == "marks" {
            let mut doc = graph::Document::new();
            let _ = doc.add_mark(
                graph::MarkKind::Rectangle,
                graph::Point::new(48.0, 48.0),
                Some([180.0, 100.0]),
            );
            let _ = doc.add_mark(
                graph::MarkKind::Circle,
                graph::Point::new(280.0, 64.0),
                Some([96.0, 96.0]),
            );
            if let Some(id) = doc.add_mark(
                graph::MarkKind::Label,
                graph::Point::new(48.0, 180.0),
                Some([200.0, 36.0]),
            ) {
                doc.set_mark_text(&id, "Revenue is the amount column".into());
            }
            canvas.load_document(doc, cx);
            return;
        }
        if demo == "python" || demo == "textarea" {
            let mut doc = graph::Document::new();
            let read = doc
                .add_node("read_csv", graph::Point::new(48.0, 48.0))
                .unwrap();
            doc.set_param(&read, "path", json!(csv.display().to_string()));
            doc.set_label(&read, "orders.csv".into());
            let code = doc
                .add_node("python", graph::Point::new(320.0, 48.0))
                .unwrap();
            doc.set_label(&code, "Keep the north rows".into());
            doc.set_param(
                &code,
                "code",
                json!("def run(frame):\n    return frame.head(4)\n"),
            );
            doc.set_python_inputs(
                &code,
                vec![graph::PortRecord {
                    name: "frame".into(),
                    accepts: vec![graph::TypeName::Dataframe],
                    multi: false,
                }],
            );
            let _ = doc.connect(&read, "out", &code, "frame");
            canvas.load_document(doc, cx);
            canvas.select_only(&code, cx);
            return;
        }
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
        } else if demo == "reel-run" {
            let mut doc = graph::Document::new();
            let read = doc
                .add_node("read_csv", graph::Point::new(80.0, 80.0))
                .unwrap();
            doc.set_param(&read, "path", json!(csv.display().to_string()));
            doc.set_label(&read, "orders.csv".into());
            canvas.load_document(doc, cx);
            canvas.select_only(read, cx);
        } else if demo == "link-reject" || demo == "reel-link" {
            let column = doc
                .add_node("column", graph::Point::new(48.0, 280.0))
                .unwrap();
            doc.set_param(&column, "name", json!("amount"));
            let _ = doc.connect(&read, "out", &column, "in");
            canvas.load_document(doc, cx);
            if demo != "reel-link" {
                canvas.seed_link_drag(&column, "out", (592.0, 140.0), cx);
            }
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

fn load_reduced_motion() -> Option<bool> {
    let path = config_dir().join("settings.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.get("reduced_motion").and_then(|v| v.as_bool()))
}

fn read_settings() -> Value {
    let path = config_dir().join("settings.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_settings(mut body: impl FnMut(&mut serde_json::Map<String, Value>)) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let mut value = read_settings();
    if let Some(map) = value.as_object_mut() {
        body(map);
    }
    let _ = std::fs::write(dir.join("settings.json"), value.to_string());
}

fn save_reduced_motion(reduced: bool) {
    write_settings(|map| {
        map.insert("reduced_motion".into(), json!(reduced));
    });
}

fn load_model_settings() -> crate::assist::ModelSettings {
    let value = read_settings();
    crate::assist::ModelSettings {
        endpoint: value
            .get("model_endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        model: value
            .get("model_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        key: value
            .get("model_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

fn save_model_settings(settings: &crate::assist::ModelSettings) {
    write_settings(|map| {
        map.insert("model_endpoint".into(), json!(settings.endpoint));
        map.insert("model_name".into(), json!(settings.model));
        map.insert("model_key".into(), json!(settings.key));
    });
}

struct PalettePanel {
    focus: FocusHandle,
    session: Entity<Session>,
    search: Entity<InputState>,
    query: String,
    _sub: Subscription,
}

impl PalettePanel {
    fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx));
        let sub = cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = input.read(cx).value().to_string();
                cx.notify();
            }
        });
        Self {
            focus: cx.focus_handle(),
            session,
            search,
            query: String::new(),
            _sub: sub,
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
        let narrow = window.viewport_size().width < px(1100.0);
        let query = self.query.to_lowercase();
        let mut column = div()
            .id("palette")
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden();
        column = column.child(
            div()
                .px_3()
                .pt_3()
                .pb_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Search"),
                )
                .child(Input::new(&self.search).small()),
        );
        let mut list = div()
            .id("palette-list")
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .px_2()
            .pb_2()
            .gap_1()
            .overflow_y_scrollbar();
        for group in graph::groups() {
            let features: Vec<_> = graph::features_in(group)
                .filter(|feature| query.is_empty() || feature.title.to_lowercase().contains(&query))
                .collect();
            if features.is_empty() {
                continue;
            }
            list = list.child(
                div()
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(*group),
            );
            let row_hover = cx.theme().secondary;
            for feature in features {
                let id = feature.id.to_string();
                let title = if narrow {
                    feature.title.chars().take(14).collect::<String>()
                } else {
                    feature.title.to_string()
                };
                let session = self.session.clone();
                let icon = feature_icon(feature.id);
                list = list.child(
                    div()
                        .id(SharedString::from(format!("feature-{id}")))
                        .h(px(36.0))
                        .w_full()
                        .px_2()
                        .rounded(px(6.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .cursor_pointer()
                        .hover(move |style| style.bg(row_hover))
                        .child(icon)
                        .child(title)
                        .on_mouse_down(gpui_kit::MouseButton::Left, move |_, _, cx| {
                            session.update(cx, |session, cx| {
                                session.canvas.update(cx, |canvas, cx| {
                                    let _ = canvas.add_feature(&id, cx);
                                });
                            });
                        }),
                );
            }
        }
        column.child(list)
    }
}

fn feature_icon(id: &str) -> IconName {
    match id {
        "read_csv" | "read_excel" => IconName::FileSpreadsheet,
        "read_sql" => IconName::Database,
        "head" => IconName::ListFilter,
        "change_headers" => IconName::Type,
        "query" => IconName::Search,
        "column" => IconName::Columns3,
        "to_frame" => IconName::Table,
        "sort_values" => IconName::ArrowDownWideNarrow,
        "merge" => IconName::GitMerge,
        "concat" => IconName::Layers,
        "literal" => IconName::Hash,
        "python" => IconName::Code,
        "chart" => IconName::ChartColumn,
        "preview_table" => IconName::Table,
        "read_json" => IconName::Code,
        "pivot" | "groupby" | "melt" => IconName::Table,
        "assign" | "astype" => IconName::Type,
        "sum" | "mean" | "min" | "max" | "count" | "describe" | "value_counts" => IconName::Hash,
        _ => IconName::Square,
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tool = self.canvas.read(cx).tool();
        let canvas = self.canvas.clone();
        div()
            .size_full()
            .relative()
            .child(self.canvas.clone())
            .child(
                div()
                    .absolute()
                    .bottom(px(12.0))
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .px_1()
                            .py(px(2.0))
                            .rounded(px(8.0))
                            .bg(cx.theme().background)
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(canvas_tool(
                                &canvas,
                                "tool-select",
                                IconName::MousePointer2,
                                "Select",
                                tool == Tool::Select,
                                Tool::Select,
                            ))
                            .child(canvas_tool(
                                &canvas,
                                "tool-hand",
                                IconName::Hand,
                                "Hand",
                                tool == Tool::Hand,
                                Tool::Hand,
                            ))
                            .child(canvas_tool(
                                &canvas,
                                "tool-rect",
                                IconName::Square,
                                "Rectangle",
                                tool == Tool::Rectangle,
                                Tool::Rectangle,
                            ))
                            .child(canvas_tool(
                                &canvas,
                                "tool-circle",
                                IconName::Circle,
                                "Circle",
                                tool == Tool::Circle,
                                Tool::Circle,
                            ))
                            .child(canvas_tool(
                                &canvas,
                                "tool-label",
                                IconName::Type,
                                "Label",
                                tool == Tool::Label,
                                Tool::Label,
                            ))
                            .child(canvas_tool(
                                &canvas,
                                "tool-note",
                                IconName::StickyNote,
                                "Note",
                                tool == Tool::Note,
                                Tool::Note,
                            )),
                    ),
            )
    }
}

fn canvas_tool(
    canvas: &Entity<CanvasModel>,
    id: &str,
    icon: IconName,
    label: &str,
    selected: bool,
    tool: Tool,
) -> gpui_kit::component::button::Button {
    let canvas = canvas.clone();
    Button::new(SharedString::from(id.to_string()))
        .ghost()
        .compact()
        .icon(icon)
        .selected(selected)
        .tooltip(label.to_string())
        .accessibility_label(label.to_string())
        .on_click(move |_, _, cx| {
            canvas.update(cx, |canvas, cx| canvas.set_tool(tool, cx));
        })
}

struct ChartPanel {
    focus: FocusHandle,
    session: Entity<Session>,
    drag: Option<(f32, f32)>,
    _sub: Subscription,
}
impl ChartPanel {
    fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&session, |_, _, cx| cx.notify());
        Self {
            focus: cx.focus_handle(),
            session,
            drag: None,
            _sub: sub,
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
        let series_name = node.param_str("series");
        let mark = node.param_str("mark");
        if rows.is_empty() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child("Run preview to draw this chart")
                .into_any_element();
        }
        if n > CHART_POINT_CAP as u64 {
            return div()
                .p_4()
                .child(format!("{n} points. Aggregate or sample before charting."))
                .into_any_element();
        }
        let points = match points_from(&columns, &rows, &x_name, &y_name, &series_name) {
            Ok(points) => points,
            Err(message) => {
                return div()
                    .p_4()
                    .text_color(cx.theme().danger)
                    .child(message)
                    .into_any_element();
            }
        };
        let title = node
            .presentation
            .as_ref()
            .and_then(|item| item.title.text.clone())
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| node.label.clone());
        let colors = [
            cx.theme().chart_1,
            cx.theme().chart_2,
            cx.theme().chart_3,
            cx.theme().chart_4,
            cx.theme().chart_5,
        ];
        let note = if error.is_some() {
            error.unwrap_or_default()
        } else {
            "preview · the chart draws this window".into()
        };
        let _ = self.drag;
        let plot = if matches!(mark.as_str(), "area" | "scatter") {
            FeaturePlot {
                mark: mark.clone(),
                data: points,
                color: colors[0],
                reduced_motion: reduced,
            }
            .into_any_element()
        } else {
            feature_chart(
                &mark,
                points,
                colors,
                cx.theme().foreground,
                reduced,
                true,
                &node.id,
            )
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_2()
            .child(div().text_sm().child(title))
            .child(div().flex_1().min_h(px(160.)).child(plot))
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
    mapping: Vec<MapRow>,
    python_rows: Vec<PyRow>,
    python_count: usize,
    bound: Option<String>,
}
struct PyRow {
    input: Entity<InputState>,
    _sub: Subscription,
}
struct InspectorField {
    name: String,
    line: Option<Entity<InputState>>,
    area: Option<Entity<TextareaState>>,
    _sub: Subscription,
}
struct MapRow {
    from: Entity<InputState>,
    to: Entity<InputState>,
    _subs: Vec<Subscription>,
}
impl InspectorPanel {
    fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            focus: cx.focus_handle(),
            session,
            fields: Vec::new(),
            mapping: Vec::new(),
            python_rows: Vec::new(),
            python_count: 0,
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
        self.mapping.clear();
        self.python_rows.clear();
        self.python_count = 0;
        let Some(node) = node else {
            return;
        };
        let Some(def) = node.def() else {
            return;
        };
        for param in def.params {
            if param.name == "mapping" {
                self.bind_mapping(&node, window, cx);
                continue;
            }
            let name = param.name.to_string();
            let text = node.param_str(&name);
            let long = matches!(name.as_str(), "code" | "expr" | "value");
            let (line, area, sub) = if long {
                let area = cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 12));
                area.update(cx, |state, cx| state.set_value(text, window, cx));
                let session = self.session.clone();
                let node_id = node.id.clone();
                let param_name = name.clone();
                let sub = cx.subscribe(&area, move |this, input, event: &InputEvent, cx| {
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
                        let mut text = input.read(cx).value().to_string();
                        let mut rewrite = false;
                        if param_name == "code" {
                            let pretty = crate::assist::pretty_python(&text);
                            if pretty != text {
                                text = pretty;
                                rewrite = true;
                            }
                        } else if let Some(pretty) = crate::assist::pretty_json(&text) {
                            if pretty.trim() != text.trim() {
                                text = pretty;
                                rewrite = true;
                            }
                        } else if text.trim().starts_with('{') || text.trim().starts_with('[') {
                            this.session.update(cx, |session, cx| {
                                session.assist_note =
                                    Some("Not valid JSON. The text was left as typed.".into());
                                cx.notify();
                            });
                        }
                        let value = coerce(&param_name, &text);
                        let show = if rewrite { Some(text.clone()) } else { None };
                        session.update(cx, |session, cx| {
                            if let Some(text) = show {
                                session.assist_fill =
                                    Some((node_id.clone(), param_name.clone(), text));
                            }
                            session.canvas.update(cx, |canvas, cx| {
                                canvas.set_param(&node_id, &param_name, value, cx);
                                canvas.commit_edit(cx);
                            });
                        });
                    }
                });
                (None, Some(area), sub)
            } else {
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
                (Some(input), None, sub)
            };
            self.fields.push(InspectorField {
                name,
                line,
                area,
                _sub: sub,
            });
        }
        cx.notify();
    }

    fn bind_mapping(&mut self, node: &graph::Node, window: &mut Window, cx: &mut Context<Self>) {
        let pairs = node
            .params
            .get(graph::HEADER_PAIRS)
            .map(header_pairs)
            .unwrap_or_default();
        let pairs = if pairs.is_empty() {
            vec![("".into(), "".into())]
        } else {
            pairs
        };
        for (from_text, to_text) in pairs {
            let from = cx.new(|cx| InputState::new(window, cx));
            let to = cx.new(|cx| InputState::new(window, cx));
            from.update(cx, |state, cx| state.set_value(from_text, window, cx));
            to.update(cx, |state, cx| state.set_value(to_text, window, cx));
            let mut subs = Vec::new();
            for input in [&from, &to] {
                let session = self.session.clone();
                let node_id = node.id.clone();
                subs.push(cx.subscribe(input, move |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Focus) {
                        this.session.update(cx, |session, cx| {
                            session.canvas.update(cx, |canvas, _| canvas.begin_edit());
                        });
                    }
                    if matches!(event, InputEvent::Change) {
                        this.write_mapping(&node_id, &session, cx);
                    }
                    if matches!(event, InputEvent::Blur) {
                        this.session.update(cx, |session, cx| {
                            session
                                .canvas
                                .update(cx, |canvas, cx| canvas.commit_edit(cx));
                        });
                    }
                }));
            }
            self.mapping.push(MapRow {
                from,
                to,
                _subs: subs,
            });
        }
    }

    fn write_mapping(&self, node_id: &str, session: &Entity<Session>, cx: &mut Context<Self>) {
        let pairs: Vec<Value> = self
            .mapping
            .iter()
            .map(|row| {
                json!({
                    "from": row.from.read(cx).value().to_string(),
                    "to": row.to.read(cx).value().to_string(),
                })
            })
            .collect();
        let node_id = node_id.to_string();
        session.update(cx, |session, cx| {
            session.canvas.update(cx, |canvas, cx| {
                canvas.set_param(&node_id, "mapping", json!(pairs), cx);
            });
        });
    }

    fn sync_python(&mut self, node: &graph::Node, window: &mut Window, cx: &mut Context<Self>) {
        let ports = node.input_ports();
        if ports.len() == self.python_count && self.python_rows.len() == ports.len() {
            return;
        }
        self.python_rows.clear();
        let node_id = node.id.clone();
        for port in &ports {
            let input = cx.new(|cx| InputState::new(window, cx));
            input.update(cx, |state, cx| {
                state.set_value(port.name.clone(), window, cx)
            });
            let node_id = node_id.clone();
            let original = port.name.clone();
            let sub = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Blur) {
                    return;
                }
                let renamed = input.read(cx).value().to_string();
                if renamed.trim().is_empty() || renamed == original {
                    return;
                }
                let ports = this.session.update(cx, |session, cx| {
                    session
                        .canvas
                        .read(cx)
                        .editor()
                        .graph()
                        .node(&node_id)
                        .map(|node| node.input_ports())
                        .unwrap_or_default()
                });
                let mut next = ports;
                if let Some(found) = next.iter_mut().find(|port| port.name == original) {
                    found.name = renamed.trim().to_string();
                }
                this.session.update(cx, |session, cx| {
                    session.canvas.update(cx, |canvas, cx| {
                        canvas.set_python_inputs(&node_id, next, cx);
                    });
                });
                this.python_count = 0;
            });
            self.python_rows.push(PyRow { input, _sub: sub });
        }
        self.python_count = self.python_rows.len();
    }

    fn python_block(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let node_id = self.bound.clone().unwrap_or_default();
        let ports = self.session.update(cx, |session, cx| {
            session
                .canvas
                .read(cx)
                .editor()
                .graph()
                .node(&node_id)
                .map(|node| node.input_ports())
                .unwrap_or_default()
        });
        let mut column = div().flex().flex_col().gap_2().child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Inputs. Each name is an argument of run."),
        );
        for (index, row) in self.python_rows.iter().enumerate() {
            let accepts = ports
                .get(index)
                .map(|port| {
                    port.accepts
                        .first()
                        .copied()
                        .unwrap_or(graph::TypeName::Dataframe)
                })
                .unwrap_or(graph::TypeName::Dataframe);
            let mut types = div().flex().flex_row().gap_1().flex_wrap();
            for (label, ty) in [
                ("table", graph::TypeName::Dataframe),
                ("column", graph::TypeName::Series),
                ("value", graph::TypeName::Scalar),
                ("list", graph::TypeName::List),
                ("map", graph::TypeName::Dict),
                ("code", graph::TypeName::Code),
            ] {
                let session = self.session.clone();
                let node_id = node_id.clone();
                let selected = accepts == ty;
                types = types.child(
                    Button::new(SharedString::from(format!("py-type-{index}-{label}")))
                        .small()
                        .compact()
                        .selected(selected)
                        .label(label)
                        .on_click(move |_, _, cx| {
                            let mut next = session.update(cx, |session, cx| {
                                session
                                    .canvas
                                    .read(cx)
                                    .editor()
                                    .graph()
                                    .node(&node_id)
                                    .map(|node| node.input_ports())
                                    .unwrap_or_default()
                            });
                            if let Some(port) = next.get_mut(index) {
                                port.accepts = vec![ty];
                            }
                            session.update(cx, |session, cx| {
                                session.canvas.update(cx, |canvas, cx| {
                                    canvas.set_python_inputs(&node_id, next, cx);
                                });
                            });
                        }),
                );
            }
            let session = self.session.clone();
            let node_id_remove = node_id.clone();
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Input::new(&row.input).small())
                    .child(types)
                    .child(
                        Button::new(SharedString::from(format!("py-remove-{index}")))
                            .ghost()
                            .small()
                            .label("Remove")
                            .on_click(move |_, _, cx| {
                                let mut next = session.update(cx, |session, cx| {
                                    session
                                        .canvas
                                        .read(cx)
                                        .editor()
                                        .graph()
                                        .node(&node_id_remove)
                                        .map(|node| node.input_ports())
                                        .unwrap_or_default()
                                });
                                if index < next.len() {
                                    next.remove(index);
                                }
                                session.update(cx, |session, cx| {
                                    session.canvas.update(cx, |canvas, cx| {
                                        canvas.set_python_inputs(&node_id_remove, next, cx);
                                    });
                                });
                            }),
                    ),
            );
        }
        let session = self.session.clone();
        let node_id_add = node_id.clone();
        column
            .child(
                Button::new("py-add")
                    .small()
                    .label("Add input")
                    .on_click(move |_, _, cx| {
                        let mut next = session.update(cx, |session, cx| {
                            session
                                .canvas
                                .read(cx)
                                .editor()
                                .graph()
                                .node(&node_id_add)
                                .map(|node| node.input_ports())
                                .unwrap_or_default()
                        });
                        let name = format!("in{}", next.len() + 1);
                        next.push(graph::PortRecord {
                            name,
                            accepts: vec![graph::TypeName::Dataframe],
                            multi: false,
                        });
                        session.update(cx, |session, cx| {
                            session.canvas.update(cx, |canvas, cx| {
                                canvas.set_python_inputs(&node_id_add, next, cx);
                            });
                        });
                    }),
            )
            .into_any_element()
    }

    fn apply_fill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fill = self.session.read(cx).assist_fill.clone();
        let Some((node, field, text)) = fill else {
            return;
        };
        if self.bound.as_deref() != Some(node.as_str()) {
            return;
        }
        for item in &self.fields {
            if item.name != field {
                continue;
            }
            if let Some(area) = &item.area {
                area.update(cx, |state, cx| state.set_value(text.clone(), window, cx));
            }
            if let Some(line) = &item.line {
                line.update(cx, |state, cx| state.set_value(text.clone(), window, cx));
            }
        }
        self.session.update(cx, |session, _| {
            session.assist_fill = None;
        });
    }

    fn ask_model(
        &mut self,
        node_id: &str,
        field: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = load_model_settings();
        let (feature, purpose, current, neighbors) = self.session.update(cx, |session, cx| {
            let canvas = session.canvas.read(cx);
            let node = canvas.editor().graph().node(node_id);
            let feature = node.map(|node| node.feature.clone()).unwrap_or_default();
            let purpose = node
                .and_then(|node| node.def())
                .map(|def| def.title.to_string())
                .unwrap_or_else(|| feature.clone());
            let current = node.map(|node| node.param_str(field)).unwrap_or_default();
            let mut neighbors = std::collections::BTreeMap::new();
            if let Some(node) = node {
                for (key, value) in &node.params {
                    neighbors.insert(key.clone(), value.to_string());
                }
            }
            (feature, purpose, current, neighbors)
        });
        let columns = self.session.update(cx, |session, cx| {
            column_choices(
                session.canvas.read(cx).editor().graph(),
                node_id,
                &session.columns,
            )
        });
        let sample = self
            .session
            .read(cx)
            .rows
            .iter()
            .take(3)
            .map(|row| row.iter().map(cell_text).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let job = crate::assist::AssistJob {
            endpoint: settings.endpoint,
            model: settings.model,
            key: settings.key,
            field: field.to_string(),
            feature,
            purpose,
            current,
            columns,
            sample,
            neighbors,
        };
        if let Some(question) = crate::assist::local_question(&job) {
            self.session.update(cx, |session, cx| {
                session.assist_note = Some(question);
                cx.notify();
            });
            cx.notify();
            return;
        }
        let rx = crate::assist::spawn(job);
        let node_id = node_id.to_string();
        let field = field.to_string();
        self.session.update(cx, |session, cx| {
            session.assist_note = Some("Asking the model…".into());
            session.assist_fill = Some((node_id, field, String::new()));
            session.assist_rx = Some(rx);
            cx.notify();
        });
        let session = self.session.clone();
        window.on_next_frame(move |window, cx| watch_assist(session, window, cx));
        cx.notify();
    }
}

fn watch_assist(session: Entity<Session>, window: &mut Window, cx: &mut App) {
    let reply = session.update(cx, |session, _| {
        let ready = session.assist_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if ready.is_some() {
            session.assist_rx = None;
        }
        ready
    });
    if let Some(reply) = reply {
        session.update(cx, |session, cx| {
            let target = session.assist_fill.clone();
            match reply {
                crate::assist::AssistReply::Value(text) => {
                    if let Some((node, field, _)) = target {
                        session.canvas.update(cx, |canvas, cx| {
                            canvas.set_param(&node, &field, json!(text.clone()), cx);
                        });
                        session.assist_fill = Some((node, field, text));
                        session.assist_note = None;
                    }
                }
                crate::assist::AssistReply::Ask(text)
                | crate::assist::AssistReply::Failed(text) => {
                    session.assist_note = Some(text);
                    session.assist_fill = None;
                }
            }
            cx.notify();
        });
    } else if session.read(cx).assist_rx.is_some() {
        let session = session.clone();
        window.on_next_frame(move |window, cx| watch_assist(session, window, cx));
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
        self.apply_fill(window, cx);
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
            .overflow_y_scrollbar();
        body = body.child(div().text_sm().child(node.label.clone()));
        body = body.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(node.feature.clone()),
        );
        if node.feature == "python" {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("This block runs with the pipeline."),
            );
        }
        let choices = column_choices(
            self.session.read(cx).canvas.read(cx).editor().graph(),
            &node.id,
            &self.session.read(cx).columns,
        );
        let def = node.def();
        let mark = node.param_str("mark");
        for field in &self.fields {
            if node.feature == "chart" && field.name == "bins" && mark != "histogram" {
                continue;
            }
            if node.feature == "chart" && field.name == "series" && mark == "pie" {
                continue;
            }
            let kind = def
                .and_then(|def| def.params.iter().find(|param| param.name == field.name))
                .map(|param| param.kind);
            let options = def
                .and_then(|def| def.params.iter().find(|param| param.name == field.name))
                .map(|param| param.options)
                .unwrap_or(&[]);
            let label = if node.feature == "chart" && mark == "pie" && field.name == "x" {
                "Category".to_string()
            } else if node.feature == "chart" && mark == "pie" && field.name == "y" {
                "Value".to_string()
            } else if node.feature == "groupby" && field.name == "by" {
                "Group by".to_string()
            } else if node.feature == "sort_values" && field.name == "by" {
                "Sort by".to_string()
            } else if node.feature == "pivot" && field.name == "columns" {
                "Columns".to_string()
            } else {
                field_label(&field.name)
            };
            let help = field_help(&field.name);
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            );
            if matches!(kind, Some(graph::ParamKind::Enum)) {
                let mut row = div().flex().flex_row().flex_wrap().gap_1();
                for option in options {
                    let option = (*option).to_string();
                    let current = field
                        .line
                        .as_ref()
                        .map(|input| input.read(cx).value().to_string())
                        .unwrap_or_default();
                    let selected = current == option;
                    let Some(input) = field.line.clone() else {
                        continue;
                    };
                    let session = self.session.clone();
                    let node_id = node.id.clone();
                    let param = field.name.clone();
                    row = row.child(
                        Button::new(SharedString::from(format!("opt-{}-{option}", field.name)))
                            .small()
                            .compact()
                            .selected(selected)
                            .label(option.clone())
                            .on_click(move |_, window, cx| {
                                input.update(cx, |state, cx| {
                                    state.set_value(option.clone(), window, cx);
                                });
                                session.update(cx, |session, cx| {
                                    session.canvas.update(cx, |canvas, cx| {
                                        canvas.set_param(&node_id, &param, json!(option), cx);
                                    });
                                });
                            }),
                    );
                }
                body = body.child(row);
            } else if let Some(area) = &field.area {
                body = body.child(Textarea::new(area));
                let session = self.session.clone();
                let node_id = node.id.clone();
                let param = field.name.clone();
                let area = area.clone();
                body = body.child(
                    Button::new(SharedString::from(format!("pretty-{}", field.name)))
                        .ghost()
                        .small()
                        .label("Pretty")
                        .on_click(move |_, window, cx| {
                            let text = area.read(cx).value().to_string();
                            let pretty = if param == "code" {
                                Some(crate::assist::pretty_python(&text))
                            } else {
                                crate::assist::pretty_json(&text)
                            };
                            let Some(pretty) = pretty else {
                                session.update(cx, |session, cx| {
                                    session.assist_note =
                                        Some("Not valid JSON. The text was left as typed.".into());
                                    cx.notify();
                                });
                                return;
                            };
                            area.update(cx, |state, cx| {
                                state.set_value(pretty.clone(), window, cx);
                            });
                            session.update(cx, |session, cx| {
                                session.canvas.update(cx, |canvas, cx| {
                                    canvas.set_param(&node_id, &param, json!(pretty), cx);
                                });
                            });
                        }),
                );
            } else if let Some(input) = &field.line {
                body = body.child(Input::new(input).small());
                if matches!(kind, Some(graph::ParamKind::Column)) && !choices.is_empty() {
                    let mut chips = div().flex().flex_row().flex_wrap().gap_1();
                    for choice in &choices {
                        let choice = choice.clone();
                        let input = input.clone();
                        let session = self.session.clone();
                        let node_id = node.id.clone();
                        let param = field.name.clone();
                        chips = chips.child(
                            Button::new(SharedString::from(format!("col-{}-{choice}", field.name)))
                                .ghost()
                                .small()
                                .compact()
                                .label(choice.clone())
                                .on_click(move |_, window, cx| {
                                    input.update(cx, |state, cx| {
                                        state.set_value(choice.clone(), window, cx);
                                    });
                                    session.update(cx, |session, cx| {
                                        session.canvas.update(cx, |canvas, cx| {
                                            canvas.set_param(&node_id, &param, json!(choice), cx);
                                        });
                                    });
                                }),
                        );
                    }
                    body = body.child(chips);
                }
            }
            if !help.is_empty() {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(help),
                );
            }
            let node_id = node.id.clone();
            let param = field.name.clone();
            body = body.child(
                Button::new(SharedString::from(format!("assist-{param}")))
                    .ghost()
                    .compact()
                    .icon(IconName::Pencil)
                    .tooltip("Fill from context")
                    .accessibility_label(format!("Fill {param} from context"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.ask_model(&node_id, &param, window, cx);
                    })),
            );
        }
        if node.feature == "python" {
            self.sync_python(&node, window, cx);
            body = body.child(self.python_block(cx));
        }
        if let Some(note) = self.session.read(cx).assist_note.clone() {
            body = body.child(
                div()
                    .p_2()
                    .rounded(px(8.0))
                    .bg(cx.theme().secondary)
                    .text_xs()
                    .child(note),
            );
        }
        if node.feature == "change_headers" {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Rename"),
            );
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Each row renames a column. Leave the new name empty to drop it."),
            );
            for (index, row) in self.mapping.iter().enumerate() {
                body = body.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(88.)).child(Input::new(&row.from).small()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("to"),
                        )
                        .child(div().flex_1().child(Input::new(&row.to).small())),
                );
                let _ = index;
            }
            if !choices.is_empty() {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Columns in the file"),
                );
                let mut chips = div().flex().flex_row().flex_wrap().gap_1();
                for choice in &choices {
                    let choice = choice.clone();
                    chips = chips.child(
                        Button::new(SharedString::from(format!("map-col-{choice}")))
                            .ghost()
                            .small()
                            .label(choice.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                for row in &this.mapping {
                                    if row.from.read(cx).value().is_empty() {
                                        row.from.update(cx, |state, cx| {
                                            state.set_value(choice.clone(), window, cx);
                                        });
                                        break;
                                    }
                                }
                                if let Some(node_id) = this.bound.clone() {
                                    let session = this.session.clone();
                                    this.write_mapping(&node_id, &session, cx);
                                }
                            })),
                    );
                }
                body = body.child(chips);
            }
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

fn column_choices(graph: &graph::Graph, node: &str, preview: &[String]) -> Vec<String> {
    let feature = graph
        .node(node)
        .map(|node| node.feature.as_str())
        .unwrap_or("");
    if matches!(feature, "chart" | "preview_table") && !preview.is_empty() {
        return preview.to_vec();
    }
    if let Ok(order) = ancestors(graph, node) {
        for id in order.iter().rev() {
            let Some(reader) = graph.node(id) else {
                continue;
            };
            if reader.feature != "read_csv" {
                continue;
            }
            if reader.param_str("header") == "no header" {
                break;
            }
            let skip = reader.param_i64("skip_rows").unwrap_or(0).max(0) as usize;
            let header = reader.param_i64("header_row").unwrap_or(0).max(0) as usize;
            let path = reader.param_str("path");
            if let Ok(headers) =
                graph::csv_headers_after(std::path::Path::new(&path), skip.saturating_add(header))
            {
                return headers;
            }
        }
    }
    preview.to_vec()
}

fn output_views(graph: &graph::Graph, ran: &str) -> Vec<String> {
    let output = |id: &str| {
        graph
            .node(id)
            .is_some_and(|node| node.feature == "chart" || node.feature == "preview_table")
    };
    let mut ids = Vec::new();
    if output(ran) {
        ids.push(ran.to_string());
        let parents: Vec<String> = graph
            .edges
            .iter()
            .filter(|edge| edge.to == ran)
            .map(|edge| edge.from.clone())
            .collect();
        for parent in parents {
            for edge in &graph.edges {
                if edge.from == parent && output(&edge.to) {
                    ids.push(edge.to.clone());
                }
            }
        }
    } else {
        for edge in &graph.edges {
            if edge.from == ran && output(&edge.to) {
                ids.push(edge.to.clone());
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn field_label(name: &str) -> String {
    match name {
        "n" => "Rows".into(),
        "path" => "File".into(),
        "expr" => "Expression".into(),
        "name" => "Column".into(),
        "mark" => "Mark".into(),
        "x" => "X".into(),
        "y" => "Y".into(),
        "series" => "Series".into(),
        "bins" => "Bins".into(),
        "by" => "By".into(),
        "ascending" => "Ascending".into(),
        "how" => "Join".into(),
        "on" => "Key".into(),
        "kind" => "Kind".into(),
        "value" => "Value".into(),
        "code" => "Code".into(),
        "skip_rows" => "Skip rows".into(),
        "header" => "Header".into(),
        "header_row" => "Header row".into(),
        "columns" => "Columns".into(),
        "agg" => "Aggregation".into(),
        "index" => "Index".into(),
        "values" => "Values".into(),
        "id_vars" => "Id columns".into(),
        "dtype" => "Type".into(),
        "decimals" => "Decimals".into(),
        "old" => "Find".into(),
        "new" => "Replace with".into(),
        other => other.into(),
    }
}

fn field_help(name: &str) -> &'static str {
    match name {
        "skip_rows" => "Lines to discard before the header. For SQLite, rows to skip.",
        "header_row" => "Which remaining row is the header. 0 is the first.",
        "header" => "Use a header row, or treat every row as data.",
        "index" => "Row labels of the pivot.",
        "columns" => "Which columns to keep, or the pivot column.",
        "values" => "Numbers to aggregate.",
        "agg" => "How the values combine.",
        "series" => "One line for each value in this column.",
        "x" => "Horizontal axis. On a pie chart this is the category.",
        "y" => "Vertical axis. On a pie chart this is the slice size.",
        "n" => "How many rows to keep.",
        "expr" => "A pandas query expression.",
        "code" => "def run(...) is the block the pipeline calls.",
        "bins" => "Histogram bins.",
        _ => "",
    }
}

fn coerce(name: &str, text: &str) -> Value {
    if matches!(name, "n" | "bins" | "skip_rows" | "header_row" | "decimals") {
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
    _search_sub: Subscription,
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
                    dtypes: Vec::new(),
                    rows: Vec::new(),
                    index: Vec::new(),
                },
                window,
                cx,
            )
        });
        let table_for_sub = table.clone();
        let search_for_sub = search.clone();
        let session_for_search = session.clone();
        let search_sub = cx.subscribe(&search_for_sub, move |_, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let value = input.read(cx).value().to_string();
                session_for_search.update(cx, |session, cx| {
                    session.search = value;
                    cx.notify();
                });
            }
        });
        let sub = cx.observe(&session, move |_, session, cx| {
            let session = session.read(cx);
            let columns = session.columns.clone();
            let dtypes = session.dtypes.clone();
            let rows = session.rows.clone();
            let index = session.index.clone();
            let search = session.search.to_lowercase();
            table_for_sub.update(cx, |table, cx| {
                let delegate = table.delegate_mut();
                delegate.columns = columns;
                delegate.dtypes = dtypes;
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
            cx.notify();
        });
        Self {
            focus: cx.focus_handle(),
            session,
            table,
            search,
            _sub: sub,
            _search_sub: search_sub,
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
        let running = self.session.read(cx).running;
        let has_rows = !self.session.read(cx).rows.is_empty();
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
                    .when(!self.search.read(cx).value().is_empty(), |row| {
                        let search = self.search.clone();
                        row.child(
                            Button::new("add-search")
                                .small()
                                .ghost()
                                .label("Add as query")
                                .on_click(move |_, _, cx| {
                                    let expr = search.read(cx).value().to_string();
                                    if expr.is_empty() {
                                        return;
                                    }
                                    session.update(cx, |session, cx| {
                                        session.search = expr.clone();
                                        session.canvas.update(cx, |canvas, cx| {
                                            if let Some(id) = canvas.add_feature("query", cx) {
                                                canvas.set_param(&id, "expr", json!(expr), cx);
                                            }
                                        });
                                    });
                                }),
                        )
                    }),
            )
            .child(if has_rows {
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .child(DataTable::new(&self.table))
                    .into_any_element()
            } else if running {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Running preview")
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .p_3()
                    .child(
                        Empty::new()
                            .header(
                                EmptyHeader::new()
                                    .title(EmptyTitle::new().child("No preview yet"))
                                    .description(
                                        EmptyDescription::new().child(
                                            "Wire a reader, then run Preview. The table shows at most 10,000 source rows, not the whole file.",
                                        ),
                                    ),
                            )
                            .child(
                                Button::new("empty-preview")
                                    .primary()
                                    .small()
                                    .label("Run preview")
                                    .on_click(|_, _, cx| {
                                        cx.dispatch_action(&RunPreview);
                                    }),
                            ),
                    )
                    .into_any_element()
            })
    }
}

struct PreviewTable {
    columns: Vec<String>,
    dtypes: Vec<String>,
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
    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let (name, dtype) = if col_ix == 0 {
            ("index".to_string(), "row".to_string())
        } else {
            (
                self.columns.get(col_ix - 1).cloned().unwrap_or_default(),
                self.dtypes.get(col_ix - 1).cloned().unwrap_or_default(),
            )
        };
        div()
            .flex()
            .flex_col()
            .justify_center()
            .child(div().text_xs().child(name))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(dtype),
            )
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
                gpui_kit::KeyBinding::new("ctrl-shift-enter", RunFull, None),
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
