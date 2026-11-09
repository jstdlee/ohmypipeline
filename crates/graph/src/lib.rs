//! The pipeline document and the rules that edit it.
//!
//! This crate does not draw. The window and the worker both call it, so a
//! wire that the canvas accepts is the wire the worker will run.

mod connect;
mod editor;
mod feature;
mod file;
mod geom;
mod model;
mod protocol;
mod sniff;
mod types;

pub use connect::{
    LinkEnd, Reject, SnapCandidate, edge_problem, output_types, pick_snap, produced_type,
    reject_message, try_connect, type_pair_label, would_cycle,
};
pub use editor::Document;
pub use feature::{CATALOG, FeatureDef, ParamDef, ParamKind, feature, features_in, groups};
pub use file::{FileError, load_path, load_str, recovery_path, save_atomic, to_json};
pub use geom::{
    Camera, GRID_PX, Point, Rect, SNAP_RADIUS_PX, fit_camera, point_in_rect, snap_grid, zoom_toward,
};
pub use model::{
    Anchor, Edge, Graph, HEADER_PAIRS, Mark, MarkKind, Node, Placed, PortRecord, Presentation,
    ancestors, bounds_of, dependency_key, duplicate_headers, empty_mapping, header_pairs,
    python_arg_names, run_fingerprint, sinks,
};
pub use protocol::{
    CHART_POINT_CAP, CellText, Job, JobOp, PREVIEW_ROW_CAP, Reply, RunMode, TABLE_WINDOW,
    WorkerError, cell_text, decode_frame, encode_frame, read_frame, status_line, write_frame,
};
pub use sniff::{csv_headers, sniff_separator};
pub use types::TypeName;

pub const FORMAT: &str = "ohmypipeline";
pub const VERSION: u32 = 1;
pub const CARD_WIDTH: f32 = 220.0;
