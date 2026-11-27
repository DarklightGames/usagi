//! `usagi graph`: the project's classes, or its packages, as a graph.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, Write};

use serde_json::json;
use usagi_core::{ClassId, Parent, Project};

use crate::cli::GraphFormat;

/// What to draw.
pub struct Options {
    /// The classes to start from; none means every class.
    pub classes: Vec<String>,
    /// With `classes`: their ancestors, and their descendants. Neither given
    /// means both.
    pub ancestors: bool,
    pub descendants: bool,
    /// With `classes`: how many levels up and down.
    pub depth: Option<usize>,
    /// Packages, and which depends on which, in place of classes.
    pub packages: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Class,
    /// A class that is extended but isn't in the project.
    Missing,
    Package,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub package: Option<String>,
    /// Where the class's source is, as the caller shows paths.
    pub path: Option<String>,
    pub kind: NodeKind,
    /// The class extends classes that extend it back, so that its link to
    /// its parent is left out.
    pub cycle: bool,
}

/// An edge from the extending class to the class it extends, or from a
/// package to one it depends on, `count` times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// The classes `options` chooses.
fn select(project: &Project, options: &Options) -> Result<BTreeSet<ClassId>, String> {
    if options.classes.is_empty() {
        return Ok(project.classes().collect());
    }
    let (up, down) = match (options.ancestors, options.descendants) {
        (false, false) => (true, true),
        (up, down) => (up, down),
    };
    let depth = options.depth.unwrap_or(usize::MAX);
    let mut selected = BTreeSet::new();
    for name in &options.classes {
        let class = project
            .class(name)
            .ok_or_else(|| format!("no class named {:?}", name))?;
        selected.insert(class);
        if up {
            selected.extend(project.chain(class).skip(1).take(depth));
        }
        if down {
            let mut queue = VecDeque::from([(class, 0)]);
            while let Some((class, level)) = queue.pop_front() {
                if level < depth {
                    for &child in project.children(class) {
                        selected.insert(child);
                        queue.push_back((child, level + 1));
                    }
                }
            }
        }
    }
    Ok(selected)
}

/// Builds the graph of the classes `options` chooses. `show` is how a path
/// is shown.
pub fn build(
    project: &Project,
    options: &Options,
    show: &dyn Fn(&std::path::Path) -> String,
) -> Result<Graph, String> {
    let selected = select(project, options)?;
    if options.packages {
        return Ok(packages(project, &selected));
    }
    let mut graph = Graph::default();
    let mut index: BTreeMap<ClassId, usize> = BTreeMap::new();
    for &class in &selected {
        index.insert(class, graph.nodes.len());
        let origin = project.origin(class);
        graph.nodes.push(Node {
            name: project.name(class).to_string(),
            package: origin.package.clone(),
            path: Some(show(&origin.path)),
            kind: NodeKind::Class,
            cycle: *project.parent(class) == Parent::Cycle,
        });
    }
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    for &class in &selected {
        let from = index[&class];
        let to = match project.parent(class) {
            Parent::Class(parent) => match index.get(parent) {
                Some(&to) => to,
                None => continue,
            },
            Parent::Missing(name) => *missing.entry(name.to_lowercase()).or_insert_with(|| {
                graph.nodes.push(Node {
                    name: name.to_string(),
                    package: None,
                    path: None,
                    kind: NodeKind::Missing,
                    cycle: false,
                });
                graph.nodes.len() - 1
            }),
            Parent::Root | Parent::Cycle => continue,
        };
        graph.edges.push(Edge { from, to, count: 1 });
    }
    Ok(sorted(graph))
}

/// The packages of `selected`, with an edge for the classes of one that
/// extend a class of another.
fn packages(project: &Project, selected: &BTreeSet<ClassId>) -> Graph {
    let package = |class| project.origin(class).package.clone();
    let names: BTreeSet<String> = selected.iter().filter_map(|&c| package(c)).collect();
    let index: BTreeMap<&str, usize> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (name.as_str(), i))
        .collect();
    let mut counts: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for &class in selected {
        let Parent::Class(parent) = project.parent(class) else {
            continue;
        };
        if !selected.contains(parent) {
            continue;
        }
        if let (Some(from), Some(to)) = (package(class), package(*parent)) {
            if from != to {
                *counts
                    .entry((index[from.as_str()], index[to.as_str()]))
                    .or_default() += 1;
            }
        }
    }
    Graph {
        nodes: names
            .into_iter()
            .map(|name| Node {
                name,
                package: None,
                path: None,
                kind: NodeKind::Package,
                cycle: false,
            })
            .collect(),
        edges: counts
            .into_iter()
            .map(|((from, to), count)| Edge { from, to, count })
            .collect(),
    }
}

/// The graph with its nodes in the order of their names, so that the output
/// doesn't depend on how the files were found.
fn sorted(graph: Graph) -> Graph {
    let mut order: Vec<usize> = (0..graph.nodes.len()).collect();
    let key = |i: &usize| {
        let node = &graph.nodes[*i];
        (
            node.name.to_lowercase(),
            node.package.clone(),
            node.path.clone(),
        )
    };
    order.sort_by_key(key);
    let mut position = vec![0; order.len()];
    for (new, &old) in order.iter().enumerate() {
        position[old] = new;
    }
    let mut nodes: Vec<Option<Node>> = graph.nodes.into_iter().map(Some).collect();
    let mut edges: Vec<Edge> = graph
        .edges
        .into_iter()
        .map(|e| Edge {
            from: position[e.from],
            to: position[e.to],
            count: e.count,
        })
        .collect();
    edges.sort_by_key(|e| (e.to, e.from));
    Graph {
        nodes: order
            .into_iter()
            .map(|old| nodes[old].take().expect("each once"))
            .collect(),
        edges,
    }
}

pub fn write(out: &mut dyn Write, graph: &Graph, format: GraphFormat) -> io::Result<()> {
    match format {
        GraphFormat::Tree => tree(out, graph),
        GraphFormat::Dot => dot(out, graph),
        GraphFormat::Mermaid => mermaid(out, graph),
        GraphFormat::Json => json(out, graph),
    }
}

fn label(node: &Node) -> String {
    let named = match (&node.kind, &node.package) {
        (NodeKind::Missing, _) => format!("{} (not in the project)", node.name),
        (_, Some(package)) => format!("{} [{}]", node.name, package),
        (_, None) => node.name.clone(),
    };
    if node.cycle {
        format!("{} (extends itself, through its parents)", named)
    } else {
        named
    }
}

/// An indented tree of the classes: each under the class it extends. For
/// packages, each package and the ones it depends on.
fn tree(out: &mut dyn Write, graph: &Graph) -> io::Result<()> {
    if graph
        .nodes
        .first()
        .is_some_and(|n| n.kind == NodeKind::Package)
    {
        for (i, node) in graph.nodes.iter().enumerate() {
            let depends: Vec<String> = graph
                .edges
                .iter()
                .filter(|e| e.from == i)
                .map(|e| format!("{} ({})", graph.nodes[e.to].name, e.count))
                .collect();
            if depends.is_empty() {
                writeln!(out, "{}", node.name)?;
            } else {
                writeln!(out, "{} -> {}", node.name, depends.join(", "))?;
            }
        }
        return Ok(());
    }
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); graph.nodes.len()];
    let mut has_parent = vec![false; graph.nodes.len()];
    for edge in &graph.edges {
        children[edge.to].push(edge.from);
        has_parent[edge.from] = true;
    }
    for (i, _) in graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| !has_parent[*i])
    {
        writeln!(out, "{}", label(&graph.nodes[i]))?;
        tree_children(out, graph, &children, i, "")?;
    }
    Ok(())
}

fn tree_children(
    out: &mut dyn Write,
    graph: &Graph,
    children: &[Vec<usize>],
    node: usize,
    prefix: &str,
) -> io::Result<()> {
    let count = children[node].len();
    for (n, &child) in children[node].iter().enumerate() {
        let last = n + 1 == count;
        let (branch, next) = if last {
            ("└── ", "    ")
        } else {
            ("├── ", "│   ")
        };
        writeln!(out, "{}{}{}", prefix, branch, label(&graph.nodes[child]))?;
        tree_children(out, graph, children, child, &format!("{}{}", prefix, next))?;
    }
    Ok(())
}

/// The nodes by package, in the order of the packages, and those with none.
fn by_package(graph: &Graph) -> (BTreeMap<&str, Vec<usize>>, Vec<usize>) {
    let mut packages: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut loose = Vec::new();
    for (i, node) in graph.nodes.iter().enumerate() {
        match (&node.package, &node.kind) {
            (Some(package), NodeKind::Class) => packages.entry(package).or_default().push(i),
            _ => loose.push(i),
        }
    }
    (packages, loose)
}

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Graphviz. Edges point from a class to the one it extends, and the
/// classes are in a cluster for each package.
fn dot(out: &mut dyn Write, graph: &Graph) -> io::Result<()> {
    let packages_only = graph
        .nodes
        .first()
        .is_some_and(|n| n.kind == NodeKind::Package);
    writeln!(
        out,
        "digraph {} {{",
        if packages_only { "packages" } else { "classes" }
    )?;
    writeln!(out, "    rankdir=BT;")?;
    writeln!(out, "    node [shape=box, fontname=\"Helvetica\"];")?;
    if !packages_only {
        writeln!(out, "    edge [arrowhead=empty];")?;
    }
    let (packages, loose) = by_package(graph);
    for (n, (package, members)) in packages.iter().enumerate() {
        writeln!(out, "    subgraph cluster_{} {{", n)?;
        writeln!(out, "        label={};", quoted(package))?;
        for &i in members {
            let style = if graph.nodes[i].cycle {
                ", color=red"
            } else {
                ""
            };
            writeln!(
                out,
                "        n{} [label={}{}];",
                i,
                quoted(&graph.nodes[i].name),
                style
            )?;
        }
        writeln!(out, "    }}")?;
    }
    for i in loose {
        let node = &graph.nodes[i];
        let style = if node.kind == NodeKind::Missing {
            ", style=dashed"
        } else {
            ""
        };
        writeln!(out, "    n{} [label={}{}];", i, quoted(&node.name), style)?;
    }
    for edge in &graph.edges {
        if packages_only {
            writeln!(
                out,
                "    n{} -> n{} [label=\"{}\"];",
                edge.from, edge.to, edge.count
            )?;
        } else {
            writeln!(out, "    n{} -> n{};", edge.from, edge.to)?;
        }
    }
    writeln!(out, "}}")
}

fn mermaid_text(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "#quot;"))
}

/// A Mermaid flowchart, with a subgraph for each package.
fn mermaid(out: &mut dyn Write, graph: &Graph) -> io::Result<()> {
    let packages_only = graph
        .nodes
        .first()
        .is_some_and(|n| n.kind == NodeKind::Package);
    writeln!(out, "flowchart BT")?;
    let (packages, loose) = by_package(graph);
    for (n, (package, members)) in packages.iter().enumerate() {
        writeln!(out, "    subgraph p{}[{}]", n, mermaid_text(package))?;
        for &i in members {
            let class = if graph.nodes[i].cycle { ":::cycle" } else { "" };
            writeln!(
                out,
                "        n{}[{}]{}",
                i,
                mermaid_text(&graph.nodes[i].name),
                class
            )?;
        }
        writeln!(out, "    end")?;
    }
    for &i in &loose {
        let node = &graph.nodes[i];
        let class = if node.kind == NodeKind::Missing {
            ":::missing"
        } else {
            ""
        };
        writeln!(out, "    n{}[{}]{}", i, mermaid_text(&node.name), class)?;
    }
    for edge in &graph.edges {
        if packages_only {
            writeln!(out, "    n{} -->|{}| n{}", edge.from, edge.count, edge.to)?;
        } else {
            writeln!(out, "    n{} --> n{}", edge.from, edge.to)?;
        }
    }
    if loose
        .iter()
        .any(|&i| graph.nodes[i].kind == NodeKind::Missing)
    {
        writeln!(out, "    classDef missing stroke-dasharray: 5 5")?;
    }
    if graph.nodes.iter().any(|n| n.cycle) {
        writeln!(out, "    classDef cycle stroke:red")?;
    }
    Ok(())
}

fn json(out: &mut dyn Write, graph: &Graph) -> io::Result<()> {
    let kind = |node: &Node| match node.kind {
        NodeKind::Class => "class",
        NodeKind::Missing => "missing",
        NodeKind::Package => "package",
    };
    let value = json!({
        "nodes": graph.nodes.iter().enumerate().map(|(id, node)| json!({
            "id": id,
            "kind": kind(node),
            "name": node.name,
            "package": node.package,
            "path": node.path,
            "cycle": node.cycle,
        })).collect::<Vec<_>>(),
        "edges": graph.edges.iter().map(|edge| json!({
            "from": edge.from,
            "to": edge.to,
            "count": edge.count,
        })).collect::<Vec<_>>(),
    });
    writeln!(
        out,
        "{}",
        serde_json::to_string_pretty(&value).map_err(io::Error::other)?
    )
}
