use notemancy_core::config::get_vault_dir;
use notemancy_core::utils as core_utils; // to use functions like get_title
use rocket::serde::Serialize;
use std::cmp::Ordering;
use std::error::Error;
use std::fs;
use std::path::Path;

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
pub struct TreeNode {
    pub name: String,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relpath: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeNode>>,
}

/// Builds the file tree for the vault folder of the "main" vault,
/// skipping the root level. Returns a vector of TreeNode representing the top-level items.
pub fn build_file_tree() -> Result<Vec<TreeNode>, Box<dyn Error>> {
    let vault_dir = get_vault_dir("main")?;
    let root_path = Path::new(&vault_dir);
    let mut nodes = Vec::new();

    // Instead of including the root, iterate its children.
    for entry in fs::read_dir(root_path)? {
        let entry = entry?;
        if let Some(child_node) = build_tree_node(&entry.path(), root_path, "main")? {
            nodes.push(child_node);
        }
    }
    sort_nodes(&mut nodes);
    if nodes.is_empty() {
        Err("Vault directory is empty".into())
    } else {
        Ok(nodes)
    }
}

/// Recursively builds a tree node for the given path.
/// - `root` is the vault directory used to compute relative paths.
/// - `vault_name` is passed to core_utils::get_title for markdown files.
/// Files that are not markdown (neither .md nor .markdown) are skipped.
/// Directories that do not contain any markdown files are skipped as well.
fn build_tree_node(
    path: &Path,
    root: &Path,
    vault_name: &str,
) -> Result<Option<TreeNode>, Box<dyn Error>> {
    // Use the file name if available; otherwise (for the root) use the full path.
    let name = if let Some(file_name) = path.file_name() {
        file_name.to_string_lossy().to_string()
    } else {
        path.to_string_lossy().to_string()
    };

    let metadata = fs::metadata(path)?;
    if metadata.is_dir() {
        let mut children = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if let Some(child_node) = build_tree_node(&entry.path(), root, vault_name)? {
                children.push(child_node);
            }
        }
        // If no children remain (i.e. no markdown files in this folder/subfolders), skip it.
        if children.is_empty() {
            return Ok(None);
        }
        sort_nodes(&mut children);
        Ok(Some(TreeNode {
            name,
            is_dir: true,
            relpath: None,
            title: None,
            children: Some(children),
        }))
    } else {
        // Check file extension (in lowercase) to see if it's markdown.
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if extension == "md" || extension == "markdown" {
            // Compute the relative path from the vault root.
            let relpath = path.strip_prefix(root)?.to_string_lossy().to_string();
            // Get the note title using the notemancy-core function.
            let title =
                core_utils::get_title(vault_name, &relpath).unwrap_or_else(|_| "".to_string());
            Ok(Some(TreeNode {
                name,
                is_dir: false,
                relpath: Some(relpath),
                title: Some(title),
                children: None,
            }))
        } else {
            // Ignore files that are not markdown.
            Ok(None)
        }
    }
}

/// Recursively sorts nodes so that directories come first and items are ordered alphabetically (case‑insensitive).
fn sort_nodes(nodes: &mut Vec<TreeNode>) {
    nodes.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    for node in nodes.iter_mut() {
        if let Some(ref mut children) = node.children {
            sort_nodes(children);
        }
    }
}
