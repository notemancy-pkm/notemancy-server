use meilisearch_sdk::client::Client;
use meilisearch_sdk::settings::Settings;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

// Create a static client instance
static CLIENT: Lazy<Client> = Lazy::new(|| {
    // The URL and API key should ideally come from configuration
    Client::new("http://localhost:7700", Some("aSampleMasterKey")).unwrap()
});

// Counter for document IDs
static COUNTER: AtomicUsize = AtomicUsize::new(1);

const INDEX_NAME: &str = "notes";

/// A document representing a note for indexing in MeiliSearch
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NoteDocument {
    /// The unique ID for the document in MeiliSearch
    pub id: usize,
    /// The relative path to the note file
    pub relpath: String,
    /// The title of the note
    pub title: String,
    /// The content of the note (without frontmatter)
    pub content: String,
}

/// A struct for search results
#[derive(Serialize, Deserialize, Debug)]
pub struct SearchResult {
    /// The number of hits found
    pub hits_count: usize,
    /// The actual note documents found
    pub hits: Vec<NoteDocument>,
}

// Asynchronous MeiliSearch functions that will be called by the synchronous wrappers

/// Configuration for MeiliSearch - async version
pub async fn configure_meilisearch_async() -> Result<(), Box<dyn Error>> {
    // Create the index if it doesn't exist
    if let Err(_) = CLIENT.get_index(INDEX_NAME).await {
        CLIENT.create_index(INDEX_NAME, Some("id")).await?;
    }

    // Configure the index settings
    let settings = Settings::new()
        .with_searchable_attributes(&["title", "content", "relpath"])
        .with_displayed_attributes(&["id", "relpath", "title", "content"])
        .with_filterable_attributes(&["relpath"])
        .with_ranking_rules(&[
            "words",
            "typo",
            "proximity",
            "attribute",
            "sort",
            "exactness",
        ]);

    let task = CLIENT
        .index(INDEX_NAME)
        .set_settings(&settings)
        .await?
        .wait_for_completion(&CLIENT, None, Some(Duration::from_secs(60)))
        .await?;

    if task.is_failure() {
        return Err(format!("Failed to configure index: {:?}", task.unwrap_failure()).into());
    }

    Ok(())
}

/// Add or update a note in the search index - async version
pub async fn index_note_async(note: &NoteDocument) -> Result<(), Box<dyn Error>> {
    let task = CLIENT
        .index(INDEX_NAME)
        .add_documents(&[note.clone()], Some("id"))
        .await?
        .wait_for_completion(&CLIENT, None, Some(Duration::from_secs(60)))
        .await?;

    if task.is_failure() {
        return Err(format!("Failed to index note: {:?}", task.unwrap_failure()).into());
    }

    Ok(())
}

/// Add or update multiple notes in the search index - async version
pub async fn index_notes_async(notes: &[NoteDocument]) -> Result<(), Box<dyn Error>> {
    if notes.is_empty() {
        return Ok(());
    }

    let task = CLIENT
        .index(INDEX_NAME)
        .add_documents(notes, Some("id"))
        .await?
        .wait_for_completion(&CLIENT, None, Some(Duration::from_secs(60)))
        .await?;

    if task.is_failure() {
        return Err(format!("Failed to index notes: {:?}", task.unwrap_failure()).into());
    }

    Ok(())
}

/// Search notes by exact relpath - async version
pub async fn search_by_relpath_async(relpath: &str) -> Result<SearchResult, Box<dyn Error>> {
    let results = CLIENT
        .index(INDEX_NAME)
        .search()
        .with_filter(&format!("relpath = '{}'", relpath.replace("'", "\\'")))
        .execute::<NoteDocument>()
        .await?;

    Ok(SearchResult {
        hits_count: results.hits.len(),
        hits: results.hits.into_iter().map(|hit| hit.result).collect(),
    })
}

/// Delete a note from the search index by its relpath - async version
pub async fn delete_note_from_index_async(relpath: &str) -> Result<(), Box<dyn Error>> {
    // First we need to find the document by its relpath
    let search_results = search_by_relpath_async(relpath).await?;

    if search_results.hits.is_empty() {
        // Note not found in index, nothing to delete
        return Ok(());
    }

    // Delete each document that matches the relpath
    for doc in search_results.hits {
        let task = CLIENT
            .index(INDEX_NAME)
            .delete_document(doc.id)
            .await?
            .wait_for_completion(&CLIENT, None, Some(Duration::from_secs(60)))
            .await?;

        if task.is_failure() {
            return Err(format!("Failed to delete note: {:?}", task.unwrap_failure()).into());
        }
    }

    Ok(())
}

/// Search notes by query string - async version
pub async fn search_notes_async(query: &str) -> Result<SearchResult, Box<dyn Error>> {
    let results = CLIENT
        .index(INDEX_NAME)
        .search()
        .with_query(query)
        .execute::<NoteDocument>()
        .await?;

    Ok(SearchResult {
        hits_count: results.hits.len(),
        hits: results.hits.into_iter().map(|hit| hit.result).collect(),
    })
}

/// Build the search index from all notes in the vault - async version
pub async fn build_search_index_async(vault_name: &str) -> Result<(), Box<dyn Error>> {
    // Configure MeiliSearch first
    configure_meilisearch_async().await?;

    // Get all notes from the vault
    let notes = notemancy_core::utils::list_notes(vault_name)?;

    // Create documents for each note
    let mut documents = Vec::new();
    for note_info in notes {
        let content = notemancy_core::crud::read_note(vault_name, &note_info.relpath, false)?;
        let document = NoteDocument {
            id: COUNTER.fetch_add(1, Ordering::SeqCst),
            relpath: note_info.relpath.clone(),
            title: note_info.title,
            content,
        };
        documents.push(document);
    }

    // Index all documents
    index_notes_async(&documents).await?;

    Ok(())
}

/// Get a new unique ID for a document
pub fn get_new_id() -> usize {
    COUNTER.fetch_add(1, Ordering::SeqCst)
}
