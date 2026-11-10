//! Performance checks for the canvas model. The culling helpers are pure, so
//! the bench times them without GPUI. Run with:
//!
//! ```text
//! cargo test -p canvas --release --test perf -- --ignored --nocapture
//! ```

use canvas::perf;
use canvas::{edge_wire_path, wire_hit};
use graph::{Camera, Graph, Point, Rect};

const VIEW: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 1400.0,
    h: 900.0,
};

fn big_graph() -> Graph {
    perf::synthetic_graph(500, 3)
}

#[test]
fn a_500_node_graph_culls_to_a_small_fraction() {
    let graph = big_graph();
    assert_eq!(graph.nodes.len(), 500);
    let camera = Camera::default();
    let nodes = perf::visible_nodes(&graph, &camera, VIEW, perf::MARGIN_PX);
    assert!(
        nodes.len() < 250,
        "expected far fewer than 500 nodes, got {}",
        nodes.len()
    );
    assert!(!nodes.is_empty());
    // Every reported node really is in the viewport with the margin added.
    let view = perf::grow(VIEW, perf::MARGIN_PX);
    for index in &nodes {
        let rect = perf::node_window_rect(&graph.nodes[*index], camera);
        assert!(perf::rects_intersect(rect, view));
    }
    // Culling is exact: no skipped node belongs in the viewport.
    for (index, node) in graph.nodes.iter().enumerate() {
        let rect = perf::node_window_rect(node, camera);
        if perf::rects_intersect(rect, view) {
            assert!(nodes.contains(&index));
        }
    }
}

#[test]
fn visible_edges_are_the_ones_meeting_the_view() {
    let graph = big_graph();
    let camera = Camera::default();
    let edges = perf::visible_edges(&graph, &camera, VIEW, perf::MARGIN_PX);
    let slots = perf::slots_of(&graph);
    // Every visible edge has at least one endpoint or its control box in view.
    for index in &edges {
        let edge = &graph.edges[*index];
        let box_ = perf::edge_control_box(edge, &slots)
            .map(|box_| perf::projected(box_, camera))
            .expect("visible edge has both endpoints");
        assert!(perf::rects_intersect(
            box_,
            perf::grow(VIEW, perf::MARGIN_PX)
        ));
    }
}

#[test]
fn synthetic_graph_is_deterministic_and_layered() {
    let first = perf::synthetic_graph(200, 11);
    let second = perf::synthetic_graph(200, 11);
    assert_eq!(first, second);
    assert_eq!(first.nodes.len(), 200);
    // About 1.2 edges per node: one wire into every body node plus a second
    // thread for the merge nodes.
    let ratio = first.edges.len() as f32 / first.nodes.len() as f32;
    assert!(ratio > 0.9 && ratio < 1.5, "edge ratio {ratio}");
    // Grid spacing 260 x 150 from the second row on, layer by layer.
    let sources: Vec<&_> = first
        .nodes
        .iter()
        .take((first.nodes.len() / 10).clamp(2, 24))
        .collect();
    for pair in sources.windows(2) {
        assert_eq!(pair[1].position[0] - pair[0].position[0], 260.0);
    }
}

#[test]
#[ignore]
fn bench_500() {
    let graph = big_graph();
    let camera = Camera::default();
    let iterations = 1000;
    let mut total_ns = 0u128;
    for _ in 0..iterations {
        let started = std::time::Instant::now();
        let nodes = perf::visible_nodes(&graph, &camera, VIEW, perf::MARGIN_PX);
        let edges = perf::visible_edges(&graph, &camera, VIEW, perf::MARGIN_PX);
        // The chains behind the visible wires, straight from the layout
        // rules (as the uncached first frame would build them).
        let mut hit = None;
        for index in edges {
            let Some(path) = edge_wire_path(&graph.nodes, &graph.edges[index]) else {
                continue;
            };
            if hit.is_none() {
                hit = wire_hit(&path, Point::new(700.0, 450.0), 8.0);
            }
        }
        total_ns += started.elapsed().as_nanos();
        std::hint::black_box((nodes.len(), hit.is_some()));
    }
    let mean_us = total_ns as f64 / iterations as f64 / 1000.0;
    println!("bench_500: {mean_us:.1} us per frame ({iterations} frames)");
    assert!(
        mean_us < 4000.0,
        "mean {mean_us:.1} us per frame exceeds the 4000 us budget"
    );
}
