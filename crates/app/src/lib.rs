//! The Ohmypipeline window. The graph crate decides what a wire means.
//! This crate draws it, and asks the pandas worker to run it.

mod chart_plot;
mod headless;
mod worker;
mod workspace;

pub use headless::{CliRequest, run_pipeline, usage, version_line};
pub use workspace::{GuiOptions, run_gui};
