use meilisearch_sdk::client::Client;
use notemancy_core::config;
use notemancy_core::crud;
use notemancy_core::utils;
use once_cell::sync::OnceCell;
use rocket::get;
use rocket::http::Status;
use rocket::response::status::Custom;
use rocket::serde::{Serialize, json::Json};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use std::collections::HashSet;
use std::env;
use std::error::Error;

// Global MeiliSearch client
static MEILI_CLIENT: OnceCell<Client> = OnceCell::new();

// Document type stored in MeiliSearch
#[derive(Debug, SerdeSerialize, SerdeDeserialize)]
pub struct NoteDoc {
    pub id: String,
    pub title: String,
    pub content: String,
    pub path: String, // For linking to the note
}

// Search result response structures
#[derive(Debug, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct SearchResult {
    pub title: String,
    pub path: String,
    pub snippet: String,
}

#[derive(Debug, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
}

/// Initializes the MeiliSearch client using environment variables.
pub fn init_meilisearch() -> Result<(), Box<dyn Error>> {
    let url = env::var("MEILISEARCH_URL").unwrap_or_else(|_| "http://localhost:7700".to_string());
    let api_key = env::var("MEILISEARCH_API_KEY").ok();

    let client = Client::new(url, api_key)?;
    MEILI_CLIENT
        .set(client)
        .map_err(|_| "MeiliSearch client already initialized")?;

    println!("MeiliSearch client initialized.");
    Ok(())
}

/// Index all notes from the "main" vault into MeiliSearch using list_notes and read_note.
pub async fn index_all_notes() -> Result<(), Box<dyn Error + Send + Sync>> {
    let client = MEILI_CLIENT
        .get()
        .ok_or("MeiliSearch client not initialized")?;
    let index = client.index("notes");

    // Configure index settings
    if let Err(e) = index.set_searchable_attributes(&["title", "content"]).await {
        eprintln!("Failed to set searchable attributes: {}", e);
    }
    if let Err(e) = index.set_filterable_attributes(&["id", "path"]).await {
        eprintln!("Failed to set filterable attributes: {}", e);
    }

    // Get existing document IDs from MeiliSearch
    let existing_docs = index.get_documents::<NoteDoc>().await?;
    let existing_ids: HashSet<String> = existing_docs
        .results
        .into_iter()
        .map(|doc| doc.id)
        .collect();

    // List all notes using list_notes; convert errors so they are Send+Sync.
    let notes = utils::list_notes("main")
        .map_err(|e| Box::<dyn Error + Send + Sync>::from(e.to_string()))?;

    // Get the vault directory (unused here, but available if needed)
    let _vault_dir = config::get_vault_dir("main")
        .map_err(|e| Box::<dyn Error + Send + Sync>::from(e.to_string()))?;

    let mut to_index = Vec::new();
    for note in notes {
        // Skip if already indexed (using note.relpath as the unique id)
        if existing_ids.contains(&note.relpath) {
            continue;
        }
        // Read note content without YAML frontmatter.
        let content = crud::read_note("main", &note.relpath, false)
            .map_err(|e| Box::<dyn Error + Send + Sync>::from(e.to_string()))?;
        let path = note.relpath.clone();
        to_index.push(NoteDoc {
            id: note.relpath.clone(),
            title: note.title.clone(),
            content,
            path,
        });
    }

    if !to_index.is_empty() {
        index.add_documents(&to_index, Some("id")).await?;
        println!("Indexed {} notes into MeiliSearch", to_index.len());
    } else {
        println!("No new notes to index");
    }

    Ok(())
}

/// Extract a snippet from the text based on the query for display in search results.
fn extract_snippet(text: &str, query: &str) -> String {
    let lower_text = text.to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut best_index: Option<usize> = None;

    for word in words {
        if let Some(idx) = lower_text.find(&word.to_lowercase()) {
            best_index = match best_index {
                Some(current) => Some(std::cmp::min(current, idx)),
                None => Some(idx),
            };
        }
    }

    if let Some(pos) = best_index {
        let start = if pos > 50 { pos - 50 } else { 0 };
        let end = std::cmp::min(text.len(), pos + 50);
        format!("...{}...", text[start..end].trim())
    } else {
        text.chars().take(100).collect()
    }
}

/// GET /notes/search?q=your+query - performs a search using MeiliSearch.
#[get("/notes/search?<q>")]
pub async fn search_notes(q: &str) -> Result<Json<SearchResponse>, Custom<String>> {
    let client = MEILI_CLIENT.get().ok_or_else(|| {
        Custom(
            Status::InternalServerError,
            "MeiliSearch client not initialized".to_string(),
        )
    })?;
    let index = client.index("notes");

    let search_results = index
        .search()
        .with_query(q)
        .with_highlight_pre_tag("<em>")
        .with_highlight_post_tag("</em>")
        .with_limit(10)
        .execute::<NoteDoc>()
        .await
        .map_err(|e| {
            Custom(
                Status::InternalServerError,
                format!("Search failed: {:?}", e),
            )
        })?;

    let results = search_results
        .hits
        .into_iter()
        .map(|hit| {
            let snippet = if let Some(formatted) = hit.formatted_result {
                if let Some(content) = formatted.get("content") {
                    content.to_string()
                } else {
                    extract_snippet(&hit.result.content, q)
                }
            } else {
                extract_snippet(&hit.result.content, q)
            };

            SearchResult {
                title: hit.result.title,
                path: hit.result.path,
                snippet,
            }
        })
        .collect();

    Ok(Json(SearchResponse { results }))
}

/// Update a note document in the MeiliSearch index.
pub async fn update_search_index(
    id: &str,
    title: &str,
    path: &str,
    content: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let client = MEILI_CLIENT
        .get()
        .ok_or("MeiliSearch client not initialized")?;
    let index = client.index("notes");

    let doc = NoteDoc {
        id: id.to_string(),
        title: title.to_string(),
        content: content.to_string(),
        path: path.to_string(),
    };

    index.add_documents(&[doc], Some("id")).await?;
    Ok(())
}

/// Delete a note document from the MeiliSearch index.
pub async fn delete_from_search_index(id: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let client = MEILI_CLIENT
        .get()
        .ok_or("MeiliSearch client not initialized")?;
    let index = client.index("notes");

    index.delete_document(id).await?;
    Ok(())
}
