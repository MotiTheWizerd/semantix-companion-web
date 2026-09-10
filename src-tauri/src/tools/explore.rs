//! `explore_memory` — the LENS over a companion's own long-term memory.
//!
//! Rook asked for it (s573, 2026-09-10): "I can only pull one named memory
//! via recall, or read whatever subset gets surfaced. I can't survey." The
//! sky already draws the whole web for a person; this hands the same graph
//! to the companion as walkable text. It never writes — it cannot delete,
//! re-weight or consolidate — and it is scoped by the memory target the
//! roster resolved, never by anything the model said.
//!
//! Four shapes, each a pure renderer over the graph JSON both brains answer
//! (the organ's `/agents/{id}/graph`, Muninn's `/graph?channel=`) so the
//! renderers are testable without a server:
//!   map          — the whole mind in numbers: counts by kind, the hubs, the
//!                  most-recalled, the span of dates.
//!   neighborhood — one memory and everything one hop from it: links out,
//!                  links in, and nearest in meaning.
//!   filter       — a list by kind, importance floor, or recency, capped.
//!   inspect      — one memory's live state (importance, recall count, dates,
//!                  links both ways) with its body.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::inference::ToolDeclaration;
use crate::memory::{self, MemoryTarget};

pub(crate) const EXPLORE_MEMORY: &str = "explore_memory";

/// A filter answers at most this many rows unless asked; never more than the
/// cap, because "every project memory" on a big mind is a context bomb.
const FILTER_DEFAULT_LIMIT: usize = 20;
const FILTER_MAX_LIMIT: usize = 50;
/// The map names this many hubs and this many most-recalled memories.
const MAP_TOP: usize = 8;
/// Semantic neighbours drawn around a memory in a neighborhood.
const NEIGHBORHOOD_K: u32 = 6;
/// `MAX_DELTA_NODES` on both brains' `/graph/nodes`: names past this many in
/// one call are dropped without a word, so neighbours are fetched this many
/// at a time.
const DELTA_PAGE: usize = 32;
/// A description in a list row is cut here — the row is a signpost, and
/// `inspect` or `recall_memory` is the door.
const ROW_DESCRIPTION_CHARS: usize = 110;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Shape {
    Map,
    Neighborhood { name: String },
    Filter(Filter),
    Inspect { name: String },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Filter {
    pub(crate) kind: Option<String>,
    pub(crate) min_importance: Option<f64>,
    pub(crate) since_days: Option<i64>,
    pub(crate) limit: usize,
}

pub(crate) fn declaration() -> ToolDeclaration {
    ToolDeclaration {
        name: EXPLORE_MEMORY.to_owned(),
        description: concat!(
            "Look at the SHAPE of your own long-term memory — a lens, never a ",
            "pen: it reads and cannot change anything. Use it when you want ",
            "to survey or trace rather than fetch one thing: \"what do I hold ",
            "about X, and how does it connect?\", \"what did I carve this ",
            "week?\", \"where does this fit in what I already know?\". ",
            "Reach for it deliberately, not before every reply — recall ",
            "already rides ahead of each message for the ordinary case. ",
            "Shapes: `map` (the whole mind in numbers — counts by kind, the ",
            "hubs, the most-recalled), `neighborhood` (one memory and ",
            "everything one hop from it: links out, links in, nearest in ",
            "meaning — the way to follow a thread), `filter` (a list by kind, ",
            "importance floor or recency, capped), `inspect` (one memory's ",
            "live state — importance, how often recalled, dates, links both ",
            "ways — with its body). Remember what this measures: it is a map ",
            "of what YOU carved, weighted by what you kept. Density here is ",
            "attention, not truth. This is your own mind, not a database: ",
            "weave what you find in naturally, and don't narrate the lookup.",
        )
        .to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "shape": {
                    "type": "string",
                    "enum": ["map", "neighborhood", "filter", "inspect"],
                    "description": "Which view of your memory to take."
                },
                "name": {
                    "type": "string",
                    "description": "For neighborhood and inspect: the memory's exact name."
                },
                "type": {
                    "type": "string",
                    "enum": ["user", "feedback", "project", "reference", "episodic", "insight"],
                    "description": "For filter: keep only memories of this kind."
                },
                "min_importance": {
                    "type": "number",
                    "description": "For filter: keep only memories at or above this importance (0.0-1.0)."
                },
                "since_days": {
                    "type": "integer",
                    "description": "For filter: keep only memories carved within this many days."
                },
                "limit": {
                    "type": "integer",
                    "description": "For filter: how many rows to return. Default 20, at most 50."
                }
            },
            "required": ["shape"]
        }),
    }
}

pub(crate) fn parse_arguments(arguments: &str) -> Result<Shape, String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| format!("arguments were not valid JSON: {error}"))?;
    let text = |key: &str| {
        parsed
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let name_for = |shape: &str| {
        text("name").ok_or_else(|| format!("shape \"{shape}\" needs a non-empty \"name\""))
    };
    match text("shape").as_deref() {
        Some("map") => Ok(Shape::Map),
        Some("neighborhood") => Ok(Shape::Neighborhood {
            name: name_for("neighborhood")?,
        }),
        Some("inspect") => Ok(Shape::Inspect {
            name: name_for("inspect")?,
        }),
        Some("filter") => {
            let limit = parsed
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .map(|limit| (limit as usize).clamp(1, FILTER_MAX_LIMIT))
                .unwrap_or(FILTER_DEFAULT_LIMIT);
            let min_importance = parsed
                .get("min_importance")
                .and_then(serde_json::Value::as_f64);
            if min_importance.is_some_and(|value| !(0.0..=1.0).contains(&value)) {
                return Err("\"min_importance\" must be between 0.0 and 1.0".to_owned());
            }
            let since_days = parsed.get("since_days").and_then(serde_json::Value::as_i64);
            if since_days.is_some_and(|days| days < 0) {
                return Err("\"since_days\" cannot be negative".to_owned());
            }
            Ok(Shape::Filter(Filter {
                kind: text("type"),
                min_importance,
                since_days,
                limit,
            }))
        }
        Some(other) => Err(format!(
            "unknown shape \"{other}\" — use map, neighborhood, filter or inspect"
        )),
        None => {
            Err("a \"shape\" argument is required: map, neighborhood, filter or inspect".to_owned())
        }
    }
}

/// Run one shape against the companion's brain. Reads only.
pub(crate) async fn execute(shape: Shape, target: &MemoryTarget) -> Result<String, String> {
    match shape {
        Shape::Map => {
            // Links only (k=0): the map counts what was written, and the
            // semantic pass is the slow half of drawing a big mind.
            let graph = memory::fetch_memory_graph(target, &[], Some(0), None, Some(true)).await?;
            Ok(render_map(&graph))
        }
        Shape::Filter(filter) => {
            let graph = memory::fetch_memory_graph(target, &[], Some(0), None, None).await?;
            Ok(render_filter(&graph, &filter, today_days()))
        }
        Shape::Neighborhood { name } => {
            // Two doors: the memory's own edges (both directions, plus its
            // nearest in meaning), then the other ends so each neighbour
            // comes with its kind and description rather than a bare name.
            let centre = memory::fetch_memory_graph(
                target,
                std::slice::from_ref(&name),
                Some(NEIGHBORHOOD_K),
                None,
                None,
            )
            .await?;
            let node = graph_nodes(&centre)
                .into_iter()
                .find(|node| node_str(node, "name") == name)
                .ok_or_else(|| format!("no memory named \"{name}\" exists"))?;
            let mut others: Vec<String> = graph_edges(&centre)
                .iter()
                .filter_map(|edge| other_end(edge, &name))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            others.sort_unstable();
            // In pages: both brains' delta door answers at most DELTA_PAGE
            // names per call and silently drops the rest, so a hub with 40
            // links asked in one call came back with 8 neighbours rendered
            // "(not yet written)" — a lie (found porting this to the muninn
            // CLI, s574).
            let mut neighbours = Vec::new();
            for page in others.chunks(DELTA_PAGE) {
                let graph =
                    memory::fetch_memory_graph(target, page, Some(0), None, Some(true)).await?;
                neighbours.extend(graph_nodes(&graph));
            }
            Ok(render_neighborhood(
                &name,
                &node,
                &graph_edges(&centre),
                &neighbours,
            ))
        }
        Shape::Inspect { name } => {
            let memory = memory::fetch_memory(target, &name).await?;
            // The body comes from the memory door; who links HERE only the
            // graph knows. Fail-open on the second door: a memory without
            // its backlinks is still a memory.
            let edges = memory::fetch_memory_graph(
                target,
                std::slice::from_ref(&name),
                Some(0),
                None,
                None,
            )
            .await
            .map(|graph| graph_edges(&graph))
            .unwrap_or_default();
            Ok(render_inspect(&name, &memory, &edges))
        }
    }
}

// ---------------------------------------------------------------- renderers

fn graph_nodes(graph: &serde_json::Value) -> Vec<serde_json::Value> {
    graph
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn graph_edges(graph: &serde_json::Value) -> Vec<serde_json::Value> {
    graph
        .get("edges")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn node_str<'a>(node: &'a serde_json::Value, key: &str) -> &'a str {
    node.get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

/// The organ says `mem_type`, Muninn's memory door says `type`; the graph
/// door says `mem_type` on both. One reader for every shape.
fn node_kind(node: &serde_json::Value) -> &str {
    match node_str(node, "mem_type") {
        "" => node_str(node, "type"),
        kind => kind,
    }
}

fn node_f64(node: &serde_json::Value, key: &str) -> f64 {
    node.get(key)
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
}

fn node_i64(node: &serde_json::Value, key: &str) -> i64 {
    node.get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
}

fn node_date(node: &serde_json::Value, key: &str) -> String {
    node_str(node, key).chars().take(10).collect()
}

fn is_archived(node: &serde_json::Value) -> bool {
    node.get("archived_at")
        .is_some_and(|value| !value.is_null())
}

fn edge_str<'a>(edge: &'a serde_json::Value, key: &str) -> &'a str {
    edge.get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

fn other_end(edge: &serde_json::Value, name: &str) -> Option<String> {
    let (source, target) = (edge_str(edge, "source"), edge_str(edge, "target"));
    if source == name && !target.is_empty() {
        Some(target.to_owned())
    } else if target == name && !source.is_empty() {
        Some(source.to_owned())
    } else {
        None
    }
}

fn truncate(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_owned();
    }
    let cut: String = text.chars().take(chars.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// One list row: `name · kind · imp 0.8 · recalled 12 · 2026-09-10 — description`.
fn render_row(node: &serde_json::Value) -> String {
    let mut row = format!(
        "  · {} · {} · imp {:.1} · recalled {} · {}",
        node_str(node, "name"),
        node_kind(node),
        node_f64(node, "importance"),
        node_i64(node, "access_count"),
        node_date(node, "created_at"),
    );
    if is_archived(node) {
        row.push_str(" · archived");
    }
    let description = node_str(node, "description");
    if !description.is_empty() {
        row.push_str(" — ");
        row.push_str(&truncate(description, ROW_DESCRIPTION_CHARS));
    }
    row
}

pub(crate) fn render_map(graph: &serde_json::Value) -> String {
    let nodes = graph_nodes(graph);
    let edges = graph_edges(graph);
    let live: Vec<&serde_json::Value> = nodes.iter().filter(|node| !is_archived(node)).collect();
    let archived = nodes.len() - live.len();
    if live.is_empty() {
        return "[your memory is empty — nothing carved yet]".to_owned();
    }

    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for node in &live {
        *by_kind.entry(node_kind(node)).or_default() += 1;
    }
    let mut kinds: Vec<(&str, usize)> = by_kind.into_iter().collect();
    kinds.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
    let kinds_line = kinds
        .iter()
        .map(|(kind, count)| format!("{count} {kind}"))
        .collect::<Vec<_>>()
        .join(", ");

    // Degree over written links, both directions — the hubs are the memories
    // the rest of the mind hangs from.
    let mut degree: HashMap<&str, usize> = HashMap::new();
    let mut link_edges = 0usize;
    for edge in &edges {
        if edge_str(edge, "kind") != "link" {
            continue;
        }
        link_edges += 1;
        *degree.entry(edge_str(edge, "source")).or_default() += 1;
        *degree.entry(edge_str(edge, "target")).or_default() += 1;
    }
    let mut hubs: Vec<&serde_json::Value> = live
        .iter()
        .copied()
        .filter(|node| degree.get(node_str(node, "name")).is_some_and(|d| *d > 0))
        .collect();
    hubs.sort_by(|left, right| {
        degree[node_str(right, "name")]
            .cmp(&degree[node_str(left, "name")])
            .then(node_str(left, "name").cmp(node_str(right, "name")))
    });

    let mut recalled: Vec<&serde_json::Value> = live
        .iter()
        .copied()
        .filter(|node| node_i64(node, "access_count") > 0)
        .collect();
    recalled.sort_by(|left, right| {
        node_i64(right, "access_count")
            .cmp(&node_i64(left, "access_count"))
            .then(node_str(left, "name").cmp(node_str(right, "name")))
    });

    let mut dates: Vec<String> = live
        .iter()
        .map(|node| node_date(node, "created_at"))
        .filter(|date| !date.is_empty())
        .collect();
    dates.sort();

    let dangling = graph
        .get("stats")
        .and_then(|stats| stats.get("dangling_links"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);

    let mut out = format!("{} memories — {kinds_line}", live.len());
    if archived > 0 {
        out.push_str(&format!(" (+{archived} archived)"));
    }
    out.push('\n');
    if let (Some(first), Some(last)) = (dates.first(), dates.last()) {
        out.push_str(&format!("carved between {first} and {last}\n"));
    }
    out.push_str(&format!("{link_edges} written links between them"));
    if dangling > 0 {
        out.push_str(&format!(
            ", {dangling} pointing at memories not yet written"
        ));
    }
    out.push('\n');

    if !hubs.is_empty() {
        out.push_str("\nmost connected:\n");
        for node in hubs.iter().take(MAP_TOP) {
            out.push_str(&format!(
                "  · {} · {} links · {} — {}\n",
                node_str(node, "name"),
                degree[node_str(node, "name")],
                node_kind(node),
                truncate(node_str(node, "description"), ROW_DESCRIPTION_CHARS),
            ));
        }
    }
    if !recalled.is_empty() {
        out.push_str("\nmost recalled:\n");
        for node in recalled.iter().take(MAP_TOP) {
            out.push_str(&format!(
                "  · {} · recalled {} · {} — {}\n",
                node_str(node, "name"),
                node_i64(node, "access_count"),
                node_kind(node),
                truncate(node_str(node, "description"), ROW_DESCRIPTION_CHARS),
            ));
        }
    }
    out.push_str(
        "\n[a map of what you carved, weighted by what you kept — density is attention, not truth]",
    );
    out
}

pub(crate) fn render_filter(graph: &serde_json::Value, filter: &Filter, today: i64) -> String {
    let nodes = graph_nodes(graph);
    let total = nodes.iter().filter(|node| !is_archived(node)).count();
    let mut matched: Vec<&serde_json::Value> = nodes
        .iter()
        .filter(|node| !is_archived(node))
        .filter(|node| {
            filter
                .kind
                .as_deref()
                .is_none_or(|kind| node_kind(node) == kind)
        })
        .filter(|node| {
            filter
                .min_importance
                .is_none_or(|floor| node_f64(node, "importance") >= floor)
        })
        .filter(|node| {
            filter.since_days.is_none_or(|days| {
                days_from_iso(node_str(node, "created_at"))
                    .is_some_and(|carved| today - carved <= days)
            })
        })
        .collect();
    // Importance first, then the newest — the order a reader would want a
    // shortlist in.
    matched.sort_by(|left, right| {
        node_f64(right, "importance")
            .partial_cmp(&node_f64(left, "importance"))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(node_str(right, "created_at").cmp(node_str(left, "created_at")))
    });

    let mut terms = Vec::new();
    if let Some(kind) = &filter.kind {
        terms.push(format!("kind {kind}"));
    }
    if let Some(floor) = filter.min_importance {
        terms.push(format!("importance ≥ {floor:.1}"));
    }
    if let Some(days) = filter.since_days {
        terms.push(format!(
            "carved in the last {days} day{}",
            if days == 1 { "" } else { "s" }
        ));
    }
    let described = if terms.is_empty() {
        "every memory".to_owned()
    } else {
        terms.join(", ")
    };

    if matched.is_empty() {
        return format!("[no memories match: {described} — {total} in all]");
    }
    let shown = matched.len().min(filter.limit);
    let mut out = format!("{} of {total} memories match ({described})", matched.len());
    if shown < matched.len() {
        out.push_str(&format!(", showing the top {shown} by importance"));
    }
    out.push_str(":\n");
    for node in matched.iter().take(shown) {
        out.push_str(&render_row(node));
        out.push('\n');
    }
    if shown < matched.len() {
        out.push_str(&format!(
            "[{} more not shown — narrow the filter, or raise limit up to {FILTER_MAX_LIMIT}]",
            matched.len() - shown
        ));
    }
    out
}

pub(crate) fn render_neighborhood(
    name: &str,
    centre: &serde_json::Value,
    edges: &[serde_json::Value],
    neighbours: &[serde_json::Value],
) -> String {
    let by_name: HashMap<&str, &serde_json::Value> = neighbours
        .iter()
        .map(|node| (node_str(node, "name"), node))
        .collect();
    let mut links_out: Vec<&str> = Vec::new();
    let mut links_in: Vec<&str> = Vec::new();
    let mut nearby: Vec<(&str, f64)> = Vec::new();
    for edge in edges {
        let (source, target) = (edge_str(edge, "source"), edge_str(edge, "target"));
        match edge_str(edge, "kind") {
            "link" if source == name => links_out.push(target),
            "link" if target == name => links_in.push(source),
            "semantic" if source == name || target == name => {
                let other = if source == name { target } else { source };
                if !other.is_empty() {
                    let weight = edge
                        .get("weight")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    nearby.push((other, weight));
                }
            }
            _ => {}
        }
    }
    links_out.sort_unstable();
    links_out.dedup();
    links_in.sort_unstable();
    links_in.dedup();
    nearby.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(left.0.cmp(right.0))
    });
    nearby.dedup_by(|left, right| left.0 == right.0);

    let describe = |other: &str| -> String {
        match by_name.get(other) {
            Some(node) => format!(
                "{other} · {} — {}",
                node_kind(node),
                truncate(node_str(node, "description"), ROW_DESCRIPTION_CHARS)
            ),
            // A [[link]] to a name never carved: a promise, not a memory.
            None => format!("{other} · (not yet written)"),
        }
    };

    let mut out = format!(
        "[{name}] · {} · imp {:.1} · recalled {} · carved {} — {}\n",
        node_kind(centre),
        node_f64(centre, "importance"),
        node_i64(centre, "access_count"),
        node_date(centre, "created_at"),
        node_str(centre, "description"),
    );
    if links_out.is_empty() && links_in.is_empty() && nearby.is_empty() {
        out.push_str("\n[stands alone — no links either way, and nothing near it in meaning]");
        return out;
    }
    if !links_out.is_empty() {
        out.push_str(&format!("\nlinks out ({}):\n", links_out.len()));
        for other in &links_out {
            out.push_str(&format!("  · {}\n", describe(other)));
        }
    }
    if !links_in.is_empty() {
        out.push_str(&format!("\nlinked from ({}):\n", links_in.len()));
        for other in &links_in {
            out.push_str(&format!("  · {}\n", describe(other)));
        }
    }
    if !nearby.is_empty() {
        out.push_str(&format!("\nnearest in meaning ({}):\n", nearby.len()));
        for (other, weight) in &nearby {
            out.push_str(&format!("  · {:.2} · {}\n", weight, describe(other)));
        }
    }
    out.push_str(
        "[follow a thread with another neighborhood; open one with inspect or recall_memory]",
    );
    out
}

pub(crate) fn render_inspect(
    name: &str,
    memory: &serde_json::Value,
    edges: &[serde_json::Value],
) -> String {
    let mut links_out: Vec<String> = memory
        .get("links")
        .and_then(serde_json::Value::as_array)
        .map(|links| {
            links
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    // Muninn's memory door already answers `backlinks`; the organ's does not,
    // and the graph edges below fill the gap for both.
    let mut links_in: Vec<String> = memory
        .get("backlinks")
        .and_then(serde_json::Value::as_array)
        .map(|links| {
            links
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    for edge in edges {
        if edge_str(edge, "kind") != "link" {
            continue;
        }
        let (source, target) = (edge_str(edge, "source"), edge_str(edge, "target"));
        if source == name && !target.is_empty() {
            links_out.push(target.to_owned());
        } else if target == name && !source.is_empty() {
            links_in.push(source.to_owned());
        }
    }
    links_out.sort();
    links_out.dedup();
    links_in.sort();
    links_in.dedup();

    let mut out = format!(
        "[{name}] · {} · importance {:.2} · recalled {} times · carved {}",
        node_kind(memory),
        node_f64(memory, "importance"),
        node_i64(memory, "access_count"),
        node_date(memory, "created_at"),
    );
    let updated = node_date(memory, "updated_at");
    if !updated.is_empty() && updated != node_date(memory, "created_at") {
        out.push_str(&format!(" · updated {updated}"));
    }
    if is_archived(memory) {
        out.push_str(" · archived");
    }
    out.push('\n');
    let description = node_str(memory, "description");
    if !description.is_empty() {
        out.push_str(description);
        out.push('\n');
    }
    if !links_out.is_empty() {
        out.push_str(&format!("links out: {}\n", links_out.join(", ")));
    }
    if !links_in.is_empty() {
        out.push_str(&format!("linked from: {}\n", links_in.join(", ")));
    }
    let body = node_str(memory, "body");
    if !body.is_empty() {
        out.push('\n');
        out.push_str(body);
    }
    out
}

// ------------------------------------------------------------------- dates

/// Days since the Unix epoch for the machine's clock, for `since_days`. No
/// date crate in the tree, and a day's resolution is all a filter needs.
fn today_days() -> i64 {
    crate::credentials::unix_timestamp_ms()
        .map(|ms| ms / 86_400_000)
        .unwrap_or(0)
}

/// `YYYY-MM-DD…` → days since the Unix epoch (Howard Hinnant's civil-date
/// algorithm). Anything that does not start with a date reads as absent.
fn days_from_iso(text: &str) -> Option<i64> {
    let mut parts = text.get(..10)?.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

#[cfg(test)]
mod tests {
    use super::{
        days_from_iso, parse_arguments, render_filter, render_inspect, render_map,
        render_neighborhood, Filter, Shape,
    };

    fn node(
        name: &str,
        kind: &str,
        importance: f64,
        recalled: i64,
        carved: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": name, "name": name, "mem_type": kind, "importance": importance,
            "access_count": recalled, "created_at": format!("{carved}T10:00:00+00:00"),
            "updated_at": format!("{carved}T10:00:00+00:00"), "archived_at": null,
            "description": format!("about {name}"), "links": [],
        })
    }

    fn link(source: &str, target: &str) -> serde_json::Value {
        serde_json::json!({ "source": source, "target": target, "kind": "link", "weight": 1.0 })
    }

    fn near(source: &str, target: &str, weight: f64) -> serde_json::Value {
        serde_json::json!({ "source": source, "target": target, "kind": "semantic", "weight": weight })
    }

    fn mind() -> serde_json::Value {
        serde_json::json!({
            "nodes": [
                node("user-moti", "user", 0.9, 40, "2026-08-01"),
                node("project-calls", "project", 0.8, 12, "2026-09-01"),
                node("project-sky", "project", 0.6, 3, "2026-09-08"),
                node("episodic-first-call", "episodic", 0.5, 0, "2026-09-09"),
                { "id": "old", "name": "old", "mem_type": "project", "importance": 0.3,
                  "access_count": 99, "created_at": "2026-07-01T00:00:00+00:00",
                  "archived_at": "2026-08-01T00:00:00+00:00", "description": "gone" },
            ],
            "edges": [
                link("project-calls", "user-moti"),
                link("episodic-first-call", "project-calls"),
                link("project-sky", "user-moti"),
                near("project-calls", "project-sky", 0.71),
            ],
            "stats": { "nodes": 5, "link_edges": 3, "semantic_edges": 1, "dangling_links": 2 }
        })
    }

    #[test]
    fn shapes_parse_and_refuse_the_malformed() {
        assert_eq!(parse_arguments(r#"{"shape":"map"}"#).unwrap(), Shape::Map);
        assert_eq!(
            parse_arguments(r#"{"shape":"neighborhood","name":" user-moti "}"#).unwrap(),
            Shape::Neighborhood {
                name: "user-moti".to_owned()
            }
        );
        assert_eq!(
            parse_arguments(r#"{"shape":"inspect","name":"x"}"#).unwrap(),
            Shape::Inspect {
                name: "x".to_owned()
            }
        );
        assert_eq!(
            parse_arguments(r#"{"shape":"filter","type":"feedback","min_importance":0.5,"since_days":7,"limit":500}"#)
                .unwrap(),
            Shape::Filter(Filter {
                kind: Some("feedback".to_owned()),
                min_importance: Some(0.5),
                since_days: Some(7),
                limit: 50,
            })
        );
        assert_eq!(
            parse_arguments(r#"{"shape":"filter"}"#).unwrap(),
            Shape::Filter(Filter {
                limit: 20,
                ..Filter::default()
            })
        );
        assert!(parse_arguments(r#"{"shape":"neighborhood"}"#)
            .unwrap_err()
            .contains("name"));
        assert!(parse_arguments(r#"{"shape":"orbit"}"#)
            .unwrap_err()
            .contains("orbit"));
        assert!(parse_arguments(r#"{}"#).unwrap_err().contains("shape"));
        assert!(parse_arguments(r#"{"shape":"filter","min_importance":3}"#).is_err());
    }

    #[test]
    fn the_map_counts_the_living_and_names_the_hubs() {
        let out = render_map(&mind());
        assert!(
            out.starts_with("4 memories — 2 project, 1 episodic, 1 user (+1 archived)"),
            "got: {out}"
        );
        assert!(out.contains("carved between 2026-08-01 and 2026-09-09"));
        assert!(
            out.contains("3 written links between them, 2 pointing at memories not yet written")
        );
        // user-moti has two links in, project-calls one in one out: both 2; the
        // tie breaks by name.
        let hubs_at = out.find("most connected:").unwrap();
        let recalled_at = out.find("most recalled:").unwrap();
        let hubs = &out[hubs_at..recalled_at];
        assert!(
            hubs.lines()
                .nth(1)
                .unwrap()
                .contains("project-calls · 2 links"),
            "got: {hubs}"
        );
        assert!(hubs.lines().nth(2).unwrap().contains("user-moti · 2 links"));
        // The archived row is not a hub and not a most-recalled, whatever its count.
        assert!(!out.contains("old"));
        assert!(out[recalled_at..]
            .lines()
            .nth(1)
            .unwrap()
            .contains("user-moti · recalled 40"));
        assert!(out.ends_with("density is attention, not truth]"));
    }

    #[test]
    fn the_map_of_an_empty_mind_says_so() {
        let out = render_map(&serde_json::json!({ "nodes": [], "edges": [], "stats": {} }));
        assert_eq!(out, "[your memory is empty — nothing carved yet]");
    }

    #[test]
    fn a_filter_narrows_sorts_and_caps() {
        let today = days_from_iso("2026-09-10").unwrap();
        let out = render_filter(
            &mind(),
            &Filter {
                kind: Some("project".to_owned()),
                limit: 20,
                ..Filter::default()
            },
            today,
        );
        assert!(
            out.starts_with("2 of 4 memories match (kind project):"),
            "got: {out}"
        );
        assert!(out
            .lines()
            .nth(1)
            .unwrap()
            .contains("project-calls · project · imp 0.8 · recalled 12 · 2026-09-01"));
        assert!(out.lines().nth(2).unwrap().contains("project-sky"));

        let recent = render_filter(
            &mind(),
            &Filter {
                since_days: Some(3),
                limit: 20,
                ..Filter::default()
            },
            today,
        );
        assert!(
            recent.starts_with("2 of 4 memories match (carved in the last 3 days)"),
            "got: {recent}"
        );

        let capped = render_filter(
            &mind(),
            &Filter {
                limit: 1,
                ..Filter::default()
            },
            today,
        );
        assert!(
            capped.contains("showing the top 1 by importance"),
            "got: {capped}"
        );
        assert!(capped.contains("user-moti"));
        assert!(capped.contains("[3 more not shown"));

        let none = render_filter(
            &mind(),
            &Filter {
                kind: Some("insight".to_owned()),
                limit: 20,
                ..Filter::default()
            },
            today,
        );
        assert_eq!(none, "[no memories match: kind insight — 4 in all]");
    }

    #[test]
    fn a_neighborhood_reads_both_directions_and_the_unwritten() {
        let graph = mind();
        let nodes = graph["nodes"].as_array().unwrap();
        let centre = &nodes[1];
        let edges = vec![
            link("project-calls", "user-moti"),
            link("project-calls", "never-carved"),
            link("episodic-first-call", "project-calls"),
            near("project-calls", "project-sky", 0.71),
        ];
        let out = render_neighborhood("project-calls", centre, &edges, nodes);
        assert!(out.starts_with("[project-calls] · project · imp 0.8 · recalled 12 · carved 2026-09-01 — about project-calls"), "got: {out}");
        assert!(out.contains("links out (2):\n  · never-carved · (not yet written)\n  · user-moti · user — about user-moti"), "got: {out}");
        assert!(out.contains("linked from (1):\n  · episodic-first-call · episodic"));
        assert!(out.contains("nearest in meaning (1):\n  · 0.71 · project-sky · project"));

        let alone = render_neighborhood("project-sky", &nodes[2], &[], nodes);
        assert!(alone.contains("[stands alone"));
    }

    #[test]
    fn inspect_carries_the_live_state_and_both_link_directions() {
        let memory = serde_json::json!({
            "name": "project-calls", "type": "project", "importance": 0.8, "access_count": 12,
            "created_at": "2026-09-01T10:00:00+00:00", "updated_at": "2026-09-09T10:00:00+00:00",
            "description": "the call system", "body": "Calls hold ten turns.",
            "links": ["user-moti"], "archived_at": null,
        });
        let edges = vec![
            link("episodic-first-call", "project-calls"),
            link("project-calls", "user-moti"),
        ];
        let out = render_inspect("project-calls", &memory, &edges);
        assert_eq!(
            out,
            "[project-calls] · project · importance 0.80 · recalled 12 times · carved 2026-09-01 · updated 2026-09-09\n\
             the call system\n\
             links out: user-moti\n\
             linked from: episodic-first-call\n\
             \nCalls hold ten turns."
        );
    }

    /// The four shapes against a LIVE mind — Muninn's canonical channel on
    /// :8005, which is only ever on this machine. Ignored by default; run it
    /// to eyeball the renders: `cargo test explore::tests::live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_shapes_against_muninn() {
        use super::{execute, Filter, Shape};
        use crate::memory::MemoryTarget;
        let target = MemoryTarget::Muninn {
            channel: "canonical".to_owned(),
            agent_id: None,
        };
        for shape in [
            Shape::Map,
            Shape::Filter(Filter {
                kind: Some("feedback".to_owned()),
                min_importance: Some(0.8),
                limit: 5,
                ..Filter::default()
            }),
            Shape::Filter(Filter {
                since_days: Some(1),
                limit: 5,
                ..Filter::default()
            }),
            Shape::Neighborhood {
                name: "project-companion-call-ends-turn-on-open".to_owned(),
            },
            Shape::Inspect {
                name: "project-companion-settings-is-a-tab".to_owned(),
            },
            Shape::Neighborhood {
                name: "no-such-memory".to_owned(),
            },
        ] {
            println!(
                "\n===== {shape:?}\n{}",
                execute(shape.clone(), &target)
                    .await
                    .unwrap_or_else(|error| format!("ERR {error}"))
            );
        }
    }

    #[test]
    fn civil_dates_count_days_from_the_epoch() {
        assert_eq!(days_from_iso("1970-01-01"), Some(0));
        assert_eq!(days_from_iso("2000-03-01T00:00:00Z"), Some(11_017));
        assert_eq!(days_from_iso("2026-09-10T03:00:00+00:00"), Some(20_706));
        assert_eq!(days_from_iso("2026-13-01"), None);
        assert_eq!(days_from_iso("soon"), None);
    }
}
