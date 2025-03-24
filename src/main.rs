#[macro_use]
extern crate rocket;

use chrono::{DateTime, Local};
use std::fs;
use std::path::Path;

mod utils;

use rocket::http::Status;
use rocket::response::status;
use rocket::serde::{Deserialize, Serialize, json::Json};
use rocket_cors::AllowedHeaders;
use rocket_cors::AllowedOrigins;
use rocket_cors::CorsOptions;

#[get("/")]
fn hello() -> &'static str {
    "hello world"
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct UploadNoteRequest {
    /// The relative path to the note (e.g. "notes/my-new-note.md")
    pub relpath: String,
    /// The complete contents of the note (which may include custom frontmatter or body)
    pub content: String,
}

#[post("/notes/upload", data = "<note>")]
fn upload_note(
    note: Json<UploadNoteRequest>,
) -> Result<rocket::response::status::Custom<&'static str>, rocket::response::status::Custom<String>>
{
    let vault_name = "main";
    let relpath = note.relpath.clone();
    let content = note.content.clone();

    // Derive the project (folder path) and title (from file name) from the given relpath.
    let path = std::path::Path::new(&relpath);
    // If there is a parent directory, use it; otherwise default to empty string.
    let project = path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    // The file stem (without extension) is used as the note title.
    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "".to_string());

    // Use the core function to create the note file (with default frontmatter).
    if let Err(e) = notemancy_core::crud::create_note(vault_name, &project, &title) {
        return Err(rocket::response::status::Custom(
            rocket::http::Status::InternalServerError,
            e.to_string(),
        ));
    }

    // Now overwrite the file with the provided content.
    match notemancy_core::config::get_vault_dir(vault_name) {
        Ok(vault_dir) => {
            let file_path = std::path::Path::new(&vault_dir).join(&relpath);
            match std::fs::write(&file_path, content) {
                Ok(_) => Ok(rocket::response::status::Custom(
                    rocket::http::Status::Ok,
                    "Note uploaded",
                )),
                Err(e) => Err(rocket::response::status::Custom(
                    rocket::http::Status::InternalServerError,
                    e.to_string(),
                )),
            }
        }
        Err(e) => Err(rocket::response::status::Custom(
            rocket::http::Status::InternalServerError,
            e.to_string(),
        )),
    }
}

#[get("/notes/tree")]
fn notes_tree() -> Result<Json<Vec<utils::TreeNode>>, rocket::response::status::Custom<String>> {
    match utils::build_file_tree() {
        Ok(nodes) => Ok(Json(nodes)),
        Err(e) => Err(rocket::response::status::Custom(
            rocket::http::Status::InternalServerError,
            e.to_string(),
        )),
    }
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct NoteQuery {
    /// The relative path to the note within the vault (e.g., "notes/my-note.md").
    relpath: String,
}

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
pub struct NoteContent {
    /// The title of the note, extracted from its YAML frontmatter.
    pub title: String,
    /// The parsed YAML frontmatter as a JSON object.
    pub frontmatter: serde_json::Value,
    /// The content of the note without the frontmatter.
    pub content: String,
}

#[get("/notes/content?<relpath>")]
fn note_content(relpath: String) -> Result<Json<NoteContent>, status::Custom<String>> {
    let vault_name = "main";

    // Determine the full file path using the vault directory and the relative path.
    let vault_dir = notemancy_core::config::get_vault_dir(vault_name)
        .map_err(|e| status::Custom(Status::InternalServerError, e.to_string()))?;
    let file_path = Path::new(&vault_dir).join(&relpath);

    // Retrieve file metadata to get the last modified time.
    let metadata = fs::metadata(&file_path)
        .map_err(|e| status::Custom(Status::InternalServerError, e.to_string()))?;
    let modified_time = metadata
        .modified()
        .map_err(|e| status::Custom(Status::InternalServerError, e.to_string()))?;
    let modified_datetime: DateTime<Local> = modified_time.into();
    let modified_str = modified_datetime.to_rfc3339();

    // Read the complete note including frontmatter.
    match notemancy_core::crud::read_note(vault_name, &relpath, true) {
        Ok(raw) => {
            // Parse YAML frontmatter if it exists.
            let (mut frontmatter, content) = if raw.starts_with("---") {
                if let Some(end_index) = raw.find("\n---\n") {
                    // Extract the YAML part (skip the initial '---\n' and exclude the closing delimiter).
                    let fm_str = &raw[4..end_index];
                    let body = raw[end_index + 5..].to_string();
                    // Parse the YAML frontmatter and convert it to JSON.
                    let parsed = serde_yaml::from_str::<serde_yaml::Value>(fm_str)
                        .map(|yaml| {
                            serde_json::to_value(yaml).unwrap_or_else(|_| serde_json::json!({}))
                        })
                        .unwrap_or_else(|_| serde_json::json!({}));
                    (parsed, body)
                } else {
                    (serde_json::json!({}), raw)
                }
            } else {
                (serde_json::json!({}), raw)
            };

            // Insert the last modified time into the frontmatter JSON.
            if let serde_json::Value::Object(ref mut map) = frontmatter {
                map.insert("last_modified".to_string(), serde_json::json!(modified_str));
            } else {
                frontmatter = serde_json::json!({ "last_modified": modified_str });
            }

            // Retrieve the note title as before.
            let title = notemancy_core::utils::get_title(vault_name, &relpath)
                .unwrap_or_else(|_| String::new());
            Ok(Json(NoteContent {
                title,
                frontmatter,
                content,
            }))
        }
        Err(e) => Err(status::Custom(Status::InternalServerError, e.to_string())),
    }
}

#[launch]
fn rocket() -> _ {
    let allowed_origins = AllowedOrigins::some_exact(&["http://localhost:5173"]);

    let cors = CorsOptions {
        allowed_origins,
        allowed_headers: AllowedHeaders::some(&["Authorization", "Accept", "Content-Type"]),
        allow_credentials: true,
        ..Default::default()
    }
    .to_cors()
    .expect("error creating CORS fairing");

    rocket::build()
        .attach(cors)
        .mount("/", routes![hello, notes_tree, note_content, upload_note])
}
