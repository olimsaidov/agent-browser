use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "checkbox",
    "radio",
    "combobox",
    "listbox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "treeitem",
    "Iframe",
];

const CONTENT_ROLES: &[&str] = &[
    "heading",
    "cell",
    "gridcell",
    "columnheader",
    "rowheader",
    "listitem",
    "article",
    "region",
    "main",
    "navigation",
];

const INVISIBLE_CHARS: &[char] = &[
    '\u{FEFF}',
    '\u{200B}',
    '\u{200C}',
    '\u{200D}',
    '\u{2060}',
    '\u{00A0}',
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AxNode {
    #[serde(deserialize_with = "string_or_int")]
    node_id: String,
    role: Option<AxValue>,
    name: Option<AxValue>,
    value: Option<AxValue>,
    properties: Option<Vec<AxProperty>>,
    #[serde(default, deserialize_with = "opt_vec_string_or_int")]
    child_ids: Option<Vec<String>>,
    backend_d_o_m_node_id: Option<i64>,
    ignored: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AxValue {
    value: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AxProperty {
    name: String,
    value: AxValue,
}

#[derive(Default)]
pub struct SnapshotOptions {
    pub interactive: bool,
    pub compact: bool,
    pub depth: Option<usize>,
    pub urls: bool,
}

#[derive(Clone, Serialize)]
pub struct SnapshotRef {
    #[serde(rename = "backendNodeId")]
    pub backend_node_id: i64,
    pub role: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct FormattedSnapshot {
    pub output: String,
    pub refs: HashMap<String, SnapshotRef>,
}

struct TreeNode {
    role: String,
    name: String,
    level: Option<i64>,
    checked: Option<String>,
    expanded: Option<bool>,
    selected: Option<bool>,
    disabled: Option<bool>,
    required: Option<bool>,
    value_text: Option<String>,
    backend_node_id: Option<i64>,
    children: Vec<usize>,
    parent_idx: Option<usize>,
    ref_id: Option<String>,
    depth: usize,
}

impl TreeNode {
    fn empty() -> Self {
        Self {
            role: String::new(),
            name: String::new(),
            level: None,
            checked: None,
            expanded: None,
            selected: None,
            disabled: None,
            required: None,
            value_text: None,
            backend_node_id: None,
            children: Vec::new(),
            parent_idx: None,
            ref_id: None,
            depth: 0,
        }
    }
}

pub fn format_snapshot(
    nodes: &[AxNode],
    options: &SnapshotOptions,
) -> Result<FormattedSnapshot, String> {
    let (mut tree_nodes, root_indices) = build_tree(nodes);
    let refs = assign_refs(&mut tree_nodes);

    let mut output = String::new();
    for root_idx in root_indices {
        render_tree(&tree_nodes, root_idx, 0, &mut output, options);
    }

    if options.compact {
        output = compact_tree(&output, options.interactive);
    }

    let output = output.trim().to_string();
    let output = if output.is_empty() {
        if options.interactive {
            "(no interactive elements)".to_string()
        } else {
            "(empty page)".to_string()
        }
    } else {
        output
    };

    Ok(FormattedSnapshot { output, refs })
}

fn build_tree(nodes: &[AxNode]) -> (Vec<TreeNode>, Vec<usize>) {
    let mut tree_nodes = Vec::with_capacity(nodes.len());
    let mut id_to_idx = HashMap::new();

    for (index, node) in nodes.iter().enumerate() {
        let role = extract_ax_string(&node.role);

        if (node.ignored.unwrap_or(false) && role != "RootWebArea") || role == "InlineTextBox" {
            tree_nodes.push(TreeNode::empty());
            id_to_idx.insert(node.node_id.clone(), index);
            continue;
        }

        let (level, checked, expanded, selected, disabled, required) =
            extract_properties(&node.properties);
        tree_nodes.push(TreeNode {
            role,
            name: extract_ax_string(&node.name),
            level,
            checked,
            expanded,
            selected,
            disabled,
            required,
            value_text: extract_ax_string_opt(&node.value),
            backend_node_id: node.backend_d_o_m_node_id,
            children: Vec::new(),
            parent_idx: None,
            ref_id: None,
            depth: 0,
        });
        id_to_idx.insert(node.node_id.clone(), index);
    }

    for (index, node) in nodes.iter().enumerate() {
        if let Some(child_ids) = &node.child_ids {
            for child_id in child_ids {
                if let Some(child_idx) = id_to_idx.get(child_id) {
                    tree_nodes[index].children.push(*child_idx);
                    tree_nodes[*child_idx].parent_idx = Some(index);
                }
            }
        }
    }

    for index in 0..tree_nodes.len() {
        if tree_nodes[index].role.is_empty() || tree_nodes[index].children.is_empty() {
            continue;
        }

        let children = tree_nodes[index].children.clone();
        let mut start = 0;
        while start < children.len() {
            if tree_nodes[children[start]].role != "StaticText" {
                start += 1;
                continue;
            }

            let mut end = start + 1;
            while end < children.len() && tree_nodes[children[end]].role == "StaticText" {
                end += 1;
            }

            if end > start + 1 {
                let aggregated_name = children[start..end]
                    .iter()
                    .map(|child_idx| tree_nodes[*child_idx].name.clone())
                    .collect::<String>();
                tree_nodes[children[start]].name = aggregated_name;
                for child_idx in &children[start + 1..end] {
                    tree_nodes[*child_idx] = TreeNode::empty();
                }
            }
            start = end;
        }

        if children.len() == 1
            && tree_nodes[children[0]].role == "StaticText"
            && tree_nodes[index].name == tree_nodes[children[0]].name
        {
            tree_nodes[children[0]] = TreeNode::empty();
        }
    }

    let mut is_child = vec![false; tree_nodes.len()];
    for node in &tree_nodes {
        for child_idx in &node.children {
            is_child[*child_idx] = true;
        }
    }

    let roots = is_child
        .iter()
        .enumerate()
        .filter_map(|(index, child)| (!child).then_some(index))
        .collect::<Vec<_>>();

    fn set_depth(nodes: &mut [TreeNode], idx: usize, depth: usize) {
        nodes[idx].depth = depth;
        let children = nodes[idx].children.clone();
        for child_idx in children {
            set_depth(nodes, child_idx, depth + 1);
        }
    }

    for root in &roots {
        set_depth(&mut tree_nodes, *root, 0);
    }

    (tree_nodes, roots)
}

fn assign_refs(nodes: &mut [TreeNode]) -> HashMap<String, SnapshotRef> {
    let mut refs = HashMap::new();
    let mut next_ref = 1;

    for node in nodes {
        let should_ref = INTERACTIVE_ROLES.contains(&node.role.as_str())
            || (CONTENT_ROLES.contains(&node.role.as_str()) && !node.name.is_empty());
        let Some(backend_node_id) = node.backend_node_id else {
            continue;
        };
        if !should_ref {
            continue;
        }

        let ref_id = format!("e{}", next_ref);
        next_ref += 1;
        node.ref_id = Some(ref_id.clone());
        refs.insert(
            ref_id,
            SnapshotRef {
                backend_node_id,
                role: node.role.clone(),
                name: node.name.clone(),
            },
        );
    }

    refs
}

fn render_tree(
    nodes: &[TreeNode],
    idx: usize,
    indent: usize,
    output: &mut String,
    options: &SnapshotOptions,
) {
    let node = &nodes[idx];

    if node.role.is_empty()
        || (node.role == "generic" && node.ref_id.is_none() && node.children.len() <= 1)
        || (node.role == "StaticText" && node.name.replace(INVISIBLE_CHARS, "").is_empty())
    {
        for child in &node.children {
            render_tree(nodes, *child, indent, output, options);
        }
        return;
    }

    if let Some(max_depth) = options.depth {
        if indent > max_depth {
            return;
        }
    }

    if node.role == "RootWebArea" || node.role == "WebArea" {
        for child in &node.children {
            render_tree(nodes, *child, indent, output, options);
        }
        return;
    }

    if options.interactive && node.ref_id.is_none() {
        for child in &node.children {
            render_tree(nodes, *child, indent, output, options);
        }
        return;
    }

    let mut line = format!("{}- {}", "  ".repeat(indent), node.role);
    let display_name = node.name.replace(INVISIBLE_CHARS, "");
    if !display_name.is_empty() {
        if let Ok(serialized) = serde_json::to_string(&display_name) {
            line.push_str(&format!(" {}", serialized));
        }
    }

    let mut attrs = Vec::new();
    if let Some(level) = node.level {
        attrs.push(format!("level={}", level));
    }
    if let Some(checked) = &node.checked {
        attrs.push(format!("checked={}", checked));
    }
    if let Some(expanded) = node.expanded {
        attrs.push(format!("expanded={}", expanded));
    }
    if node.selected == Some(true) {
        attrs.push("selected".to_string());
    }
    if node.disabled == Some(true) {
        attrs.push("disabled".to_string());
    }
    if node.required == Some(true) {
        attrs.push("required".to_string());
    }
    if let Some(ref_id) = &node.ref_id {
        attrs.push(format!("ref={}", ref_id));
    }
    if !attrs.is_empty() {
        line.push_str(&format!(" [{}]", attrs.join(", ")));
    }
    if let Some(value_text) = &node.value_text {
        if !value_text.is_empty() && value_text != &node.name {
            line.push_str(&format!(": {}", value_text));
        }
    }

    output.push_str(&line);
    output.push('\n');

    for child in &node.children {
        render_tree(nodes, *child, indent + 1, output, options);
    }
}

fn compact_tree(tree: &str, interactive: bool) -> String {
    let lines = tree.lines().collect::<Vec<_>>();
    let mut keep = vec![false; lines.len()];

    for (index, line) in lines.iter().enumerate() {
        if !(line.contains("ref=") || line.contains(": ")) {
            continue;
        }
        keep[index] = true;
        let indent = count_indent(line);
        for ancestor in (0..index).rev() {
            let ancestor_indent = count_indent(lines[ancestor]);
            if ancestor_indent < indent {
                keep[ancestor] = true;
                if ancestor_indent == 0 {
                    break;
                }
            }
        }
    }

    let output = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| keep[index].then_some(*line))
        .collect::<Vec<_>>()
        .join("\n");
    if output.trim().is_empty() && interactive {
        return "(no interactive elements)".to_string();
    }
    output
}

fn count_indent(line: &str) -> usize {
    let trimmed = line.trim_start();
    (line.len() - trimmed.len()) / 2
}

fn extract_ax_string(value: &Option<AxValue>) -> String {
    match value {
        Some(value) => match &value.value {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Number(number)) => number.to_string(),
            Some(Value::Bool(bool)) => bool.to_string(),
            _ => String::new(),
        },
        None => String::new(),
    }
}

fn extract_ax_string_opt(value: &Option<AxValue>) -> Option<String> {
    match value {
        Some(value) => match &value.value {
            Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
            Some(Value::Number(number)) => Some(number.to_string()),
            _ => None,
        },
        None => None,
    }
}

fn extract_properties(
    props: &Option<Vec<AxProperty>>,
) -> (
    Option<i64>,
    Option<String>,
    Option<bool>,
    Option<bool>,
    Option<bool>,
    Option<bool>,
) {
    let mut level = None;
    let mut checked = None;
    let mut expanded = None;
    let mut selected = None;
    let mut disabled = None;
    let mut required = None;

    if let Some(properties) = props {
        for prop in properties {
            match prop.name.as_str() {
                "level" => level = prop.value.value.as_ref().and_then(|value| value.as_i64()),
                "checked" => {
                    checked = prop.value.value.as_ref().map(|value| match value {
                        Value::String(text) => text.clone(),
                        Value::Bool(bool) => bool.to_string(),
                        _ => "false".to_string(),
                    });
                }
                "expanded" => expanded = prop.value.value.as_ref().and_then(|value| value.as_bool()),
                "selected" => selected = prop.value.value.as_ref().and_then(|value| value.as_bool()),
                "disabled" => disabled = prop.value.value.as_ref().and_then(|value| value.as_bool()),
                "required" => required = prop.value.value.as_ref().and_then(|value| value.as_bool()),
                _ => {}
            }
        }
    }

    (level, checked, expanded, selected, disabled, required)
}

fn string_or_int<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(text) => Ok(text),
        Value::Number(number) => Ok(number.to_string()),
        _ => Err(serde::de::Error::custom("expected string or number")),
    }
}

fn opt_vec_string_or_int<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<Vec<Value>>::deserialize(deserializer)?;
    Ok(opt.map(|values| {
        values
            .into_iter()
            .filter_map(|value| match value {
                Value::String(text) => Some(text),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
            .collect()
    }))
}
