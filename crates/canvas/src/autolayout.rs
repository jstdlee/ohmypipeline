//! Layered auto-layout over the induced subgraph of a selection.
//!
//! A pure function: no GPUI types, no camera, no document. The caller
//! commits the result through `Document::place_many` as one undo step.

use std::collections::{BTreeMap, BTreeSet};

use graph::{Node, Point};

pub const COLUMN_GAP: f32 = 80.0;
pub const ROW_GAP: f32 = 32.0;

/// Row height of a card, for tests and callers outside the canvas crate.
pub fn card_height_of(node: &Node) -> f32 {
    crate::card_height(node)
}

/// Left-to-right layered layout.
///
/// Layers follow the longest path from a source of the induced subgraph of
/// `ids`. Four barycenter passes reduce crossings inside each layer. Columns
/// start at the selection's minimum x and advance by the widest card seen so
/// far plus a gap. Rows stack by real card heights and each layer is centered
/// on the selection's original vertical center. Nodes with no edges in the
/// subgraph land in a final column after the layered ones. The same input
/// always produces the same output.
pub fn layered_layout(graph: &graph::Graph, ids: &[String]) -> BTreeMap<String, Point> {
    let mut out = BTreeMap::new();
    let picked: BTreeSet<&str> = ids.iter().map(|id| id.as_str()).collect();
    let nodes: Vec<&Node> = graph
        .nodes
        .iter()
        .filter(|node| picked.contains(node.id.as_str()))
        .collect();
    if nodes.is_empty() {
        return out;
    }
    let min_x = nodes
        .iter()
        .map(|node| node.position[0])
        .fold(f32::MAX, f32::min);
    let heights: BTreeMap<&str, f32> = nodes
        .iter()
        .map(|node| (node.id.as_str(), card_height_of(node)))
        .collect();
    let original_center = center_y(graph, &picked, &heights);

    // Edges of the induced subgraph, sorted so ties resolve by endpoint id.
    let links: Vec<(&str, &str)> = graph
        .edges
        .iter()
        .filter(|edge| picked.contains(edge.from.as_str()) && picked.contains(edge.to.as_str()))
        .map(|edge| (edge.from.as_str(), edge.to.as_str()))
        .collect();
    let connected: BTreeSet<&str> = links.iter().flat_map(|(from, to)| [*from, *to]).collect();

    // Longest-path layering with a worklist. A node's layer is one past the
    // deepest parent; the loop repeats until stable, which also absorbs any
    // cycle the caller somehow built (layers just stop growing).
    let order: Vec<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    let mut layer: BTreeMap<&str, usize> = BTreeMap::new();
    for id in &order {
        layer.insert(id, 0);
    }
    for _ in 0..order.len().max(1) {
        let mut changed = false;
        for &(from, to) in links.iter() {
            let next = layer[from] + 1;
            if next > layer[to] {
                layer.insert(to, next);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let layer_count = layer.values().copied().max().unwrap_or(0) + 1;
    let mut members: Vec<Vec<&str>> = vec![Vec::new(); layer_count];
    for id in &order {
        if !connected.contains(id) {
            continue;
        }
        members[layer[id]].push(id);
    }
    // Detached nodes share one final column after the layered ones.
    let detached: Vec<&str> = order
        .iter()
        .copied()
        .filter(|id| !connected.contains(id))
        .collect();
    if !detached.is_empty() {
        members.push(detached);
    }

    // Barycenter ordering within layers, four alternating passes.
    let mut column_of: BTreeMap<&str, usize> = BTreeMap::new();
    for layer_ids in members.iter() {
        for (row, id) in layer_ids.iter().enumerate() {
            column_of.insert(*id, row);
        }
    }
    for pass in 0..4 {
        let forward = pass % 2 == 0;
        let indices: Vec<usize> = if forward {
            (0..members.len()).collect()
        } else {
            (0..members.len()).rev().collect()
        };
        for layer_index in indices {
            let layer_ids = members[layer_index].clone();
            let mut keyed: Vec<(f32, &str)> = layer_ids
                .iter()
                .copied()
                .map(|id| {
                    let neighbors: Vec<&str> = links
                        .iter()
                        .copied()
                        .filter(|&(from, to)| if forward { to == id } else { from == id })
                        .map(|(from, to)| if forward { from } else { to })
                        .collect();
                    let barycenter = if neighbors.is_empty() {
                        column_of[id] as f32
                    } else {
                        neighbors
                            .iter()
                            .map(|other| column_of[other] as f32)
                            .sum::<f32>()
                            / neighbors.len() as f32
                    };
                    (barycenter, id)
                })
                .collect();
            keyed.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for (row, (_, id)) in keyed.into_iter().enumerate() {
                members[layer_index][row] = id;
                column_of.insert(id, row);
            }
        }
    }

    // Columns advance by the widest card seen so far plus a gap; rows stack
    // by real heights and center on the original selection center.
    let mut widest = 0.0f32;
    for layer_ids in &members {
        let total: f32 = layer_ids
            .iter()
            .map(|id| heights[*id])
            .fold(0.0, |sum, height| sum + height);
        let gaps = ROW_GAP * layer_ids.len().saturating_sub(1) as f32;
        let mut y = original_center - (total + gaps) * 0.5;
        for id in layer_ids.iter().copied() {
            let _node = graph.node(id).expect("picked node");
            out.insert(id.to_string(), Point::new(min_x + widest, snap(y)));
            y += heights[id] + ROW_GAP;
        }
        let layer_width = layer_ids
            .iter()
            .filter_map(|id| graph.node(id))
            .map(|node| graph::card_width(&node.feature))
            .fold(0.0f32, f32::max);
        widest += layer_width + COLUMN_GAP;
    }
    out
}
/// Align to the grid so a layout commit lands on the same lattice a drag
/// would.
fn snap(y: f32) -> f32 {
    (y / graph::GRID_PX).round() * graph::GRID_PX
}

fn center_y(graph: &graph::Graph, picked: &BTreeSet<&str>, heights: &BTreeMap<&str, f32>) -> f32 {
    let mut top = f32::MAX;
    let mut bottom = f32::MIN;
    for node in &graph.nodes {
        if !picked.contains(node.id.as_str()) {
            continue;
        }
        let height = heights[node.id.as_str()];
        top = top.min(node.position[1]);
        bottom = bottom.max(node.position[1] + height);
    }
    if top > bottom {
        0.0
    } else {
        (top + bottom) * 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph::Document;

    fn chain_doc() -> (Document, Vec<String>) {
        let mut doc = Document::new();
        let a = doc.add_node("read_csv", Point::new(400.0, 300.0)).unwrap();
        let b = doc.add_node("head", Point::new(40.0, 400.0)).unwrap();
        let c = doc.add_node("query", Point::new(900.0, 200.0)).unwrap();
        doc.connect(&a, "out", &b, "in").unwrap();
        doc.connect(&b, "out", &c, "in").unwrap();
        (doc, vec![a, b, c])
    }

    fn rects(
        doc: &Document,
        placed: &BTreeMap<String, Point>,
    ) -> Vec<(String, f32, f32, f32, f32)> {
        doc.graph()
            .nodes
            .iter()
            .filter_map(|node| {
                placed.get(&node.id).map(|at| {
                    (
                        node.id.clone(),
                        at.x,
                        at.y,
                        graph::card_width(&node.feature),
                        card_height_of(node),
                    )
                })
            })
            .collect()
    }

    fn overlap(a: &(String, f32, f32, f32, f32), b: &(String, f32, f32, f32, f32)) -> bool {
        a.1 < b.1 + b.3 && a.1 + a.3 > b.1 && a.2 < b.2 + b.4 && a.2 + a.4 > b.2
    }

    #[test]
    fn a_chain_lays_out_left_to_right_on_one_row() {
        let (doc, ids) = chain_doc();
        let placed = layered_layout(doc.graph(), &ids);
        assert_eq!(placed.len(), 3);
        let a = placed[&ids[0]];
        let b = placed[&ids[1]];
        let c = placed[&ids[2]];
        assert!(a.x < b.x && b.x < c.x, "layers must advance to the right");
        assert_eq!(a.y, b.y);
        assert_eq!(b.y, c.y);
    }

    #[test]
    fn a_diamond_shares_a_column_without_overlap() {
        let mut doc = Document::new();
        let a = doc.add_node("read_csv", Point::new(0.0, 0.0)).unwrap();
        let b = doc.add_node("head", Point::new(400.0, 0.0)).unwrap();
        let c = doc.add_node("query", Point::new(400.0, 300.0)).unwrap();
        // The sink takes two inputs, so the diamond is a legal graph.
        let d = doc.add_node("concat", Point::new(900.0, 0.0)).unwrap();
        doc.connect(&a, "out", &b, "in").unwrap();
        doc.connect(&a, "out", &c, "in").unwrap();
        doc.connect(&b, "out", &d, "in").unwrap();
        doc.connect(&c, "out", &d, "in").unwrap();
        let ids = vec![a, b, c, d];
        let placed = layered_layout(doc.graph(), &ids);
        let b_at = placed[&ids[1]];
        let c_at = placed[&ids[2]];
        assert_eq!(b_at.x, c_at.x, "middle layers share a column");
        let rects = rects(&doc, &placed);
        for (index, left) in rects.iter().enumerate() {
            for right in rects.iter().skip(index + 1) {
                assert!(!overlap(left, right), "{} and {} overlap", left.0, right.0);
            }
        }
    }

    #[test]
    fn no_two_output_rects_overlap() {
        let mut doc = Document::new();
        let a = doc.add_node("read_csv", Point::new(0.0, 0.0)).unwrap();
        let b = doc.add_node("head", Point::new(500.0, 40.0)).unwrap();
        let c = doc.add_node("query", Point::new(60.0, 420.0)).unwrap();
        // The sink takes two inputs, so the fan-in is a legal graph.
        let d = doc.add_node("concat", Point::new(700.0, 380.0)).unwrap();
        let e = doc
            .add_node("sort_values", Point::new(120.0, 900.0))
            .unwrap();
        doc.connect(&a, "out", &b, "in").unwrap();
        doc.connect(&a, "out", &c, "in").unwrap();
        doc.connect(&b, "out", &d, "in").unwrap();
        doc.connect(&c, "out", &d, "in").unwrap();
        doc.connect(&d, "out", &e, "in").unwrap();
        let ids = vec![a, b, c, d, e];
        let placed = layered_layout(doc.graph(), &ids);
        let rects = rects(&doc, &placed);
        for (index, left) in rects.iter().enumerate() {
            for right in rects.iter().skip(index + 1) {
                assert!(!overlap(left, right));
            }
        }
    }

    #[test]
    fn the_same_input_layouts_the_same_way() {
        let (doc, ids) = chain_doc();
        let first = layered_layout(doc.graph(), &ids);
        let second = layered_layout(doc.graph(), &ids);
        assert_eq!(first, second);
    }

    #[test]
    fn disconnected_nodes_land_in_a_final_column() {
        let (mut doc, ids) = chain_doc();
        let solo = doc.add_node("read_csv", Point::new(2000.0, 0.0)).unwrap();
        let mut all = ids.clone();
        all.push(solo.clone());
        let placed = layered_layout(doc.graph(), &all);
        let solo_at = placed[&solo];
        for id in &ids {
            assert!(solo_at.x > placed[id].x, "the solo node sits last");
        }
    }

    #[test]
    fn an_empty_selection_layouts_nothing() {
        let (doc, _) = chain_doc();
        assert!(layered_layout(doc.graph(), &[]).is_empty());
    }
}
