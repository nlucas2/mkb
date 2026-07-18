//! Dependency-free graph layout and export primitives shared by UI and CLI clients.

use crate::{GraphData, LinkKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

/// A positioned graph ready for a renderer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphScene {
    /// Output width in pixels.
    pub width: f64,
    /// Output height in pixels.
    pub height: f64,
    /// Graph-space rectangle mapped onto the output.
    pub view_box: GraphViewBox,
    /// Optional canvas/background fill; `None` keeps the SVG transparent.
    pub background: Option<String>,
    /// Positioned nodes.
    pub nodes: Vec<GraphSceneNode>,
    /// Positioned edges.
    pub edges: Vec<GraphSceneEdge>,
}

/// An SVG view box in graph coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GraphViewBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Shape of a positioned graph node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeShape {
    Circle,
    Diamond,
}

/// Label alignment relative to its x coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphTextAnchor {
    Start,
    Middle,
    End,
}

/// One positioned and styled node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphSceneNode {
    pub id: String,
    pub label: String,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub shape: GraphNodeShape,
    pub fill: String,
    pub opacity: f64,
    pub show_label: bool,
    pub label_x: f64,
    pub label_y: f64,
    pub label_fill: String,
    pub label_stroke: String,
    pub label_size: f64,
    pub label_weight: u16,
    pub label_anchor: GraphTextAnchor,
}

/// One positioned and styled directed edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphSceneEdge {
    pub source: String,
    pub target: String,
    pub stroke: String,
    pub opacity: f64,
    pub width: f64,
    pub dash: Vec<f64>,
    pub arrow: bool,
    pub arrow_size: f64,
}

/// Palette used by the deterministic CLI scene builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphTheme {
    Dark,
    Light,
}

/// Label policy used by the deterministic CLI scene builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphLabels {
    Full,
    Truncate,
    Off,
}

/// Options for the built-in deterministic radial layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphLayoutOptions {
    pub width: f64,
    pub height: f64,
    pub theme: GraphTheme,
    pub labels: GraphLabels,
    pub include_tags: bool,
    pub transparent: bool,
}

impl Default for GraphLayoutOptions {
    fn default() -> Self {
        Self {
            width: 1600.0,
            height: 900.0,
            theme: GraphTheme::Dark,
            labels: GraphLabels::Full,
            include_tags: true,
            transparent: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Palette {
    background: &'static str,
    foreground: &'static str,
    accent: &'static str,
    mauve: &'static str,
    tag: &'static str,
}

impl GraphTheme {
    fn palette(self) -> Palette {
        match self {
            Self::Dark => Palette {
                background: "#181825",
                foreground: "#cdd6f4",
                accent: "#89b4fa",
                mauve: "#cba6f7",
                tag: "#f9e2af",
            },
            Self::Light => Palette {
                background: "#fbf6ea",
                foreground: "#3a3327",
                accent: "#3b6ea5",
                mauve: "#8557c9",
                tag: "#df8e1d",
            },
        }
    }
}

/// Build a deterministic, dependency-free radial scene from graph data.
///
/// Nodes are ordered by their stable IDs and seeded on a golden-angle spiral. The layout is meant
/// to be reproducible and scriptable, not pixel-identical to the UI's interactive d3 simulation.
pub fn layout_graph_scene(graph: &GraphData, options: GraphLayoutOptions) -> GraphScene {
    let palette = options.theme.palette();
    let mut raw_nodes: Vec<(String, String, usize, bool, GraphNodeShape)> = graph
        .nodes
        .iter()
        .map(|node| {
            (
                node.id.to_string(),
                node.title.clone(),
                node.in_degree + node.out_degree,
                node.root,
                GraphNodeShape::Circle,
            )
        })
        .collect();

    let mut raw_edges: Vec<(String, String, LinkKind)> = graph
        .edges
        .iter()
        .map(|edge| (edge.source.to_string(), edge.target.to_string(), edge.kind))
        .collect();

    if options.include_tags {
        let tags: BTreeSet<String> = graph
            .nodes
            .iter()
            .flat_map(|node| node.tags.iter().cloned())
            .collect();
        for tag in tags {
            raw_nodes.push((
                format!("tag:{tag}"),
                format!("#{tag}"),
                graph
                    .nodes
                    .iter()
                    .filter(|node| node.tags.contains(&tag))
                    .count(),
                false,
                GraphNodeShape::Diamond,
            ));
        }
        for node in &graph.nodes {
            for tag in &node.tags {
                raw_edges.push((
                    node.id.to_string(),
                    format!("tag:{tag}"),
                    LinkKind::References,
                ));
            }
        }
    }

    raw_nodes.sort_by(|a, b| a.0.cmp(&b.0));
    raw_edges.sort_by(|a, b| {
        (&a.0, &a.1, link_kind_order(a.2)).cmp(&(&b.0, &b.1, link_kind_order(b.2)))
    });

    let padding = 72.0;
    let usable_width = (options.width - padding * 2.0).max(1.0);
    let usable_height = (options.height - padding * 2.0).max(1.0);
    let radius_x = usable_width * 0.46;
    let radius_y = usable_height * 0.46;
    let count = raw_nodes.len().max(1) as f64;
    let golden_angle = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    let center_x = options.width / 2.0;
    let center_y = options.height / 2.0;

    let mut nodes = Vec::with_capacity(raw_nodes.len());
    for (index, (id, title, degree, root, shape)) in raw_nodes.into_iter().enumerate() {
        let fraction = ((index as f64 + 0.5) / count).sqrt();
        let angle = index as f64 * golden_angle;
        let x = center_x + angle.cos() * radius_x * fraction;
        let y = center_y + angle.sin() * radius_y * fraction;
        let radius = ((degree as f64 + 1.0).cbrt() * 2.6).max(2.4);
        let label = match options.labels {
            GraphLabels::Full => title,
            GraphLabels::Truncate => truncate_label(&title),
            GraphLabels::Off => title,
        };
        let fill = match shape {
            GraphNodeShape::Diamond => palette.tag,
            GraphNodeShape::Circle if root => palette.accent,
            GraphNodeShape::Circle => palette.mauve,
        };
        nodes.push(GraphSceneNode {
            id,
            label,
            x,
            y,
            radius,
            shape,
            fill: fill.to_string(),
            opacity: 1.0,
            show_label: options.labels != GraphLabels::Off,
            label_x: x + radius + 3.0,
            label_y: y,
            label_fill: match shape {
                GraphNodeShape::Diamond => palette.tag,
                GraphNodeShape::Circle => palette.foreground,
            }
            .to_string(),
            label_stroke: palette.background.to_string(),
            label_size: 11.0,
            label_weight: 400,
            label_anchor: GraphTextAnchor::Start,
        });
    }

    let tag_ids: BTreeSet<&str> = nodes
        .iter()
        .filter(|node| node.shape == GraphNodeShape::Diamond)
        .map(|node| node.id.as_str())
        .collect();
    let edges = raw_edges
        .into_iter()
        .map(|(source, target, kind)| {
            let tag = tag_ids.contains(target.as_str());
            let (stroke, opacity, width, dash, arrow) = if tag {
                (palette.tag, 0.32, 1.0, vec![1.0, 3.0], false)
            } else {
                match kind {
                    LinkKind::Transcludes => (palette.mauve, 0.6, 1.6, Vec::new(), true),
                    LinkKind::References => (palette.accent, 0.45, 0.7, vec![3.0, 2.0], true),
                }
            };
            GraphSceneEdge {
                source,
                target,
                stroke: stroke.to_string(),
                opacity,
                width,
                dash,
                arrow,
                arrow_size: 3.0,
            }
        })
        .collect();

    GraphScene {
        width: options.width,
        height: options.height,
        view_box: GraphViewBox {
            x: 0.0,
            y: 0.0,
            width: options.width,
            height: options.height,
        },
        background: (!options.transparent).then(|| palette.background.to_string()),
        nodes,
        edges,
    }
}

/// Render a positioned graph scene to standalone SVG.
pub fn render_graph_svg(scene: &GraphScene) -> Result<String, String> {
    validate_scene(scene)?;
    let mut svg = String::new();
    writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="{} {} {} {}">"#,
        number(scene.width),
        number(scene.height),
        number(scene.view_box.x),
        number(scene.view_box.y),
        number(scene.view_box.width),
        number(scene.view_box.height)
    )
    .unwrap();
    if let Some(background) = &scene.background {
        writeln!(
            svg,
            r#"  <rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
            number(scene.view_box.x),
            number(scene.view_box.y),
            number(scene.view_box.width),
            number(scene.view_box.height),
            xml_escape(background)
        )
        .unwrap();
    }

    let positions: HashMap<&str, &GraphSceneNode> = scene
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    svg.push_str("  <g fill=\"none\" stroke-linecap=\"round\">\n");
    for edge in &scene.edges {
        let Some(source) = positions.get(edge.source.as_str()) else {
            continue;
        };
        let Some(target) = positions.get(edge.target.as_str()) else {
            continue;
        };
        let dash = if edge.dash.is_empty() {
            String::new()
        } else {
            format!(
                r#" stroke-dasharray="{}""#,
                edge.dash
                    .iter()
                    .map(|v| number(*v))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        writeln!(
            svg,
            r#"    <line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-opacity="{}" stroke-width="{}"{} />"#,
            number(source.x),
            number(source.y),
            number(target.x),
            number(target.y),
            xml_escape(&edge.stroke),
            number(clamp01(edge.opacity)),
            number(edge.width.max(0.0)),
            dash
        )
        .unwrap();
        if edge.arrow {
            write_arrow(&mut svg, source, target, edge);
        }
    }
    svg.push_str("  </g>\n");

    for node in &scene.nodes {
        match node.shape {
            GraphNodeShape::Circle => {
                writeln!(
                    svg,
                    r#"  <circle cx="{}" cy="{}" r="{}" fill="{}" fill-opacity="{}"/>"#,
                    number(node.x),
                    number(node.y),
                    number(node.radius),
                    xml_escape(&node.fill),
                    number(clamp01(node.opacity))
                )
                .unwrap();
            }
            GraphNodeShape::Diamond => {
                let points = format!(
                    "{},{} {},{} {},{} {},{}",
                    number(node.x),
                    number(node.y - node.radius),
                    number(node.x + node.radius),
                    number(node.y),
                    number(node.x),
                    number(node.y + node.radius),
                    number(node.x - node.radius),
                    number(node.y)
                );
                writeln!(
                    svg,
                    r#"  <polygon points="{}" fill="{}" fill-opacity="{}"/>"#,
                    points,
                    xml_escape(&node.fill),
                    number(clamp01(node.opacity))
                )
                .unwrap();
            }
        }
        if node.show_label {
            let anchor = match node.label_anchor {
                GraphTextAnchor::Start => "start",
                GraphTextAnchor::Middle => "middle",
                GraphTextAnchor::End => "end",
            };
            writeln!(
                svg,
                r#"  <text x="{}" y="{}" fill="{}" fill-opacity="{}" stroke="{}" stroke-width="3" paint-order="stroke fill" stroke-linejoin="round" text-anchor="{}" dominant-baseline="middle" font-family="-apple-system, BlinkMacSystemFont, Segoe UI, sans-serif" font-size="{}" font-weight="{}">{}</text>"#,
                number(node.label_x),
                number(node.label_y),
                xml_escape(&node.label_fill),
                number(clamp01(node.opacity)),
                xml_escape(&node.label_stroke),
                anchor,
                number(node.label_size),
                node.label_weight,
                xml_escape(&node.label)
            )
            .unwrap();
        }
    }
    svg.push_str("</svg>\n");
    Ok(svg)
}

/// Render graph topology as Graphviz DOT, leaving layout to Graphviz.
pub fn render_graph_dot(graph: &GraphData) -> String {
    let mut out = String::from("digraph mkb {\n");
    let mut nodes: Vec<_> = graph.nodes.iter().collect();
    nodes.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    for node in nodes {
        writeln!(
            out,
            "  \"{}\" [label=\"{}\"];",
            dot_escape(node.id.as_str()),
            dot_escape(&node.title)
        )
        .unwrap();
    }
    let mut edges: Vec<_> = graph.edges.iter().collect();
    edges.sort_by(|a, b| {
        (
            a.source.as_str(),
            a.target.as_str(),
            link_kind_order(a.kind),
        )
            .cmp(&(
                b.source.as_str(),
                b.target.as_str(),
                link_kind_order(b.kind),
            ))
    });
    for edge in edges {
        let style = match edge.kind {
            LinkKind::Transcludes => "solid",
            LinkKind::References => "dashed",
        };
        writeln!(
            out,
            "  \"{}\" -> \"{}\" [style=\"{style}\"];",
            dot_escape(edge.source.as_str()),
            dot_escape(edge.target.as_str())
        )
        .unwrap();
    }
    out.push_str("}\n");
    out
}

/// Serialize a positioned graph scene as pretty JSON.
pub fn render_graph_json(scene: &GraphScene) -> Result<String, String> {
    serde_json::to_string_pretty(scene).map_err(|error| error.to_string())
}

fn write_arrow(
    svg: &mut String,
    source: &GraphSceneNode,
    target: &GraphSceneNode,
    edge: &GraphSceneEdge,
) {
    let dx = target.x - source.x;
    let dy = target.y - source.y;
    let length = (dx * dx + dy * dy).sqrt();
    if length <= 1e-6 {
        return;
    }
    let ux = dx / length;
    let uy = dy / length;
    let px = -uy;
    let py = ux;
    let center_x = source.x + dx * 0.5;
    let center_y = source.y + dy * 0.5;
    let arrow_length = edge.arrow_size;
    let half_width = arrow_length / 1.6 / 2.0;
    let tip_x = center_x + ux * arrow_length * 0.5;
    let tip_y = center_y + uy * arrow_length * 0.5;
    let tail_x = center_x - ux * arrow_length * 0.5;
    let tail_y = center_y - uy * arrow_length * 0.5;
    let points = format!(
        "{},{} {},{} {},{}",
        number(tip_x),
        number(tip_y),
        number(tail_x + px * half_width),
        number(tail_y + py * half_width),
        number(tail_x - px * half_width),
        number(tail_y - py * half_width)
    );
    writeln!(
        svg,
        r#"  <polygon points="{}" fill="{}" fill-opacity="{}"/>"#,
        points,
        xml_escape(&edge.stroke),
        number(clamp01(edge.opacity))
    )
    .unwrap();
}

fn validate_scene(scene: &GraphScene) -> Result<(), String> {
    let finite = |value: f64| value.is_finite();
    if !finite(scene.width)
        || !finite(scene.height)
        || scene.width <= 0.0
        || scene.height <= 0.0
        || !finite(scene.view_box.x)
        || !finite(scene.view_box.y)
        || !finite(scene.view_box.width)
        || !finite(scene.view_box.height)
        || scene.view_box.width <= 0.0
        || scene.view_box.height <= 0.0
    {
        return Err("graph scene has invalid output or view-box dimensions".to_string());
    }
    for node in &scene.nodes {
        if ![
            node.x,
            node.y,
            node.radius,
            node.opacity,
            node.label_x,
            node.label_y,
            node.label_size,
        ]
        .into_iter()
        .all(finite)
            || node.radius < 0.0
            || node.label_size < 0.0
        {
            return Err(format!("graph scene node {} has invalid geometry", node.id));
        }
    }
    for edge in &scene.edges {
        if !finite(edge.opacity)
            || !finite(edge.width)
            || !finite(edge.arrow_size)
            || edge.width < 0.0
            || edge.arrow_size < 0.0
            || !edge.dash.iter().copied().all(finite)
        {
            return Err(format!(
                "graph scene edge {} -> {} has invalid style",
                edge.source, edge.target
            ));
        }
    }
    Ok(())
}

fn truncate_label(label: &str) -> String {
    let first = label.split_whitespace().next().unwrap_or(label);
    if first.len() < label.len() {
        format!("{first}…")
    } else {
        label.to_string()
    }
}

fn link_kind_order(kind: LinkKind) -> u8 {
    match kind {
        LinkKind::Transcludes => 0,
        LinkKind::References => 1,
    }
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn number(value: f64) -> String {
    let mut rendered = format!("{value:.4}");
    while rendered.contains('.') && rendered.ends_with('0') {
        rendered.pop();
    }
    if rendered.ends_with('.') {
        rendered.pop();
    }
    if rendered == "-0" {
        rendered = "0".to_string();
    }
    rendered
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn dot_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockId, GraphEdge, GraphNode};

    fn graph() -> GraphData {
        GraphData {
            nodes: vec![
                GraphNode {
                    id: BlockId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
                    title: "Alpha & <root>".to_string(),
                    in_degree: 0,
                    out_degree: 1,
                    root: true,
                    tags: vec!["ops".to_string()],
                },
                GraphNode {
                    id: BlockId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAW").unwrap(),
                    title: "Beta node".to_string(),
                    in_degree: 1,
                    out_degree: 0,
                    root: false,
                    tags: Vec::new(),
                },
            ],
            edges: vec![GraphEdge {
                source: BlockId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
                target: BlockId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAW").unwrap(),
                kind: LinkKind::Transcludes,
            }],
        }
    }

    #[test]
    fn deterministic_layout_and_svg_escape_content() {
        let options = GraphLayoutOptions::default();
        let a = layout_graph_scene(&graph(), options);
        let b = layout_graph_scene(&graph(), options);
        assert_eq!(a, b);
        let svg = render_graph_svg(&a).unwrap();
        assert!(svg.contains("Alpha &amp; &lt;root&gt;"));
        assert!(svg.contains("<circle"));
        assert!(svg.contains("<polygon"));
        assert!(svg.contains("stroke-dasharray=\"1 3\""));
        assert!(svg.contains("paint-order=\"stroke fill\""));
    }

    #[test]
    fn invalid_scene_geometry_is_rejected() {
        let mut scene = layout_graph_scene(&graph(), GraphLayoutOptions::default());
        scene.width = f64::NAN;
        assert!(render_graph_svg(&scene).is_err());
    }

    #[test]
    fn dot_and_json_outputs_are_stable() {
        let data = graph();
        let dot = render_graph_dot(&data);
        assert!(dot.contains("\"01ARZ3NDEKTSV4RRFFQ69G5FAV\""));
        assert!(dot.contains("style=\"solid\""));
        let scene = layout_graph_scene(&data, GraphLayoutOptions::default());
        let json = render_graph_json(&scene).unwrap();
        assert!(json.contains("\"view_box\""));
        assert!(json.contains("\"diamond\""));
    }

    #[test]
    fn dot_escape_keeps_titles_on_one_quoted_line() {
        assert_eq!(dot_escape("a\"b\nc\\d"), "a\\\"b\\nc\\\\d");
    }
}
