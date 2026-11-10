//! The Ohmypipeline window. The graph crate decides what a wire means.
//! This crate draws it, and asks the pandas worker to run it.

mod assist;
mod chart_plot;
mod headless;
mod node_search;
mod run_dialog;
mod stats_view;
mod variables_panel;
mod worker;
mod workspace;

pub use headless::{CliRequest, run_pipeline, run_usage, usage, version_line};
pub use workspace::{GuiOptions, run_gui};
