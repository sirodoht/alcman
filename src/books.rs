use askama::Template;
use axum::{
    extract::{Form, Json, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::AppState;
use crate::atproto::{BookRef, PdsClient, resolve_handle_from_plc};
use crate::auth::{User, current_user, signups_disabled};
use crate::claude::{ClaudeClient, ClaudeConfig};
use crate::database::{BookUpdate, Database};
use crate::gpt::BookMetadata;
use crate::gpt::{GptClient, GptConfig};
use crate::pds::AuthenticatedPds;
use crate::templates::{
    BookAddTemplate, BookDetailTemplate, BookEditChatTemplate, BookEditTemplate,
    BookIncludeTemplate, BookListTemplate, BookNotesTemplate, LibraryBook,
};

/// Check if a model name is a Claude model
fn is_claude_model(model: &str) -> bool {
    model.starts_with("claude-")
}

/// Extract book metadata using either GPT or Claude based on model selection
async fn extract_metadata_with_ai(query: &str, model: &str) -> Result<BookMetadata, String> {
    if is_claude_model(model) {
        let claude = ClaudeClient::new(ClaudeConfig::from_env());
        if !claude.has_api_key() {
            return Err("Claude API key not configured".to_string());
        }
        claude
            .extract_book_metadata(query, model)
            .await
            .map(|m| BookMetadata {
                title: m.title,
                author: m.author,
                publication_year: m.publication_year,
            })
            .map_err(|e| e.to_string())
    } else {
        let gpt = GptClient::new(GptConfig::from_env());
        if !gpt.has_api_key() {
            return Err("OpenAI API key not configured".to_string());
        }
        gpt.extract_book_metadata(query, model)
            .await
            .map_err(|e| e.to_string())
    }
}

/// Edit book with instruction using either GPT or Claude based on model selection
async fn edit_book_with_ai(
    current_title: &str,
    current_author: Option<&str>,
    current_publication_year: Option<i32>,
    instruction: &str,
    model: &str,
) -> Result<BookMetadata, String> {
    if is_claude_model(model) {
        let claude = ClaudeClient::new(ClaudeConfig::from_env());
        if !claude.has_api_key() {
            return Err("Claude API key not configured".to_string());
        }
        claude
            .edit_book_with_instruction(
                current_title,
                current_author,
                current_publication_year,
                instruction,
                model,
            )
            .await
            .map(|r| BookMetadata {
                title: r.title,
                author: r.author,
                publication_year: r.publication_year,
            })
            .map_err(|e| e.to_string())
    } else {
        let gpt = GptClient::new(GptConfig::from_env());
        if !gpt.has_api_key() {
            return Err("OpenAI API key not configured".to_string());
        }
        gpt.edit_book_with_instruction(
            current_title,
            current_author,
            current_publication_year,
            instruction,
            model,
        )
        .await
        .map(|r| BookMetadata {
            title: r.title,
            author: r.author,
            publication_year: r.publication_year,
        })
        .map_err(|e| e.to_string())
    }
}

// Book-related structures
#[derive(sqlx::FromRow, Serialize, Clone)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub publication_year: Option<i32>,
    pub created_at: String,
}

impl Book {
    pub fn created_date(&self) -> &str {
        self.created_at
            .split('T')
            .next()
            .unwrap_or(&self.created_at)
    }
}

/// Activity record showing who interacted with a book
#[derive(Clone)]
pub struct BookActivity {
    pub username: String,
    pub did: String,
    pub action: String,
    pub created_at: String,
}

impl BookActivity {
    pub fn created_date(&self) -> &str {
        self.created_at
            .split('T')
            .next()
            .unwrap_or(&self.created_at)
    }
}

#[derive(Deserialize)]
pub struct EditBookForm {
    pub title: String,
    pub author: String,
    pub publication_year: String,
}

#[derive(Deserialize)]
pub struct EditChatForm {
    pub instruction: String,
    pub model: String,
}

#[derive(Deserialize)]
pub struct EditChatApplyForm {
    pub title: String,
    pub author: String,
    pub publication_year: String,
}

#[derive(Deserialize)]
pub struct BookAddExtractForm {
    pub query: String,
    pub model: String,
}

#[derive(Deserialize)]
pub struct BookAddRefineForm {
    pub title: String,
    pub author: String,
    pub publication_year: String,
    pub instruction: String,
    pub model: String,
    pub query: String,
}

#[derive(Deserialize)]
pub struct BookAddSaveForm {
    pub title: String,
    pub author: String,
    pub publication_year: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: String,
    pub model: String,
    pub query: String,
}

#[derive(Deserialize)]
pub struct BookListQuery {
    pub filter: Option<String>,
}

pub async fn book_list(
    State(db): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<BookListQuery>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let Some(user_did) = &user.did else {
        // User doesn't have a DID, show empty library
        let template = BookListTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            books: vec![],
            filter: query.filter,
        };
        return Html(template.render().unwrap()).into_response();
    };

    let Some(pds_client) = PdsClient::from_env() else {
        // PDS not configured, show empty library
        let template = BookListTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            books: vec![],
            filter: query.filter,
        };
        return Html(template.render().unwrap()).into_response();
    };

    // Fetch user's book entries from PDS
    let entries = match pds_client.list_book_entries(user_did).await {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Error fetching book entries: {error}");
            vec![]
        }
    };

    // Convert to LibraryBook items
    let mut books: Vec<LibraryBook> = Vec::new();
    for entry in entries {
        let book_ref = &entry.value.book;

        // Try to find matching book in local database for the ID
        let book_id = match db
            .find_book_by_title_author(
                &book_ref.title,
                book_ref
                    .authors
                    .as_ref()
                    .and_then(|a| a.first())
                    .map(|s| s.as_str()),
            )
            .await
        {
            Ok(Some(book)) => Some(book.id),
            _ => None,
        };

        let status = entry.value.status.clone();

        // Apply filter if specified
        if let Some(ref filter) = query.filter {
            match filter.as_str() {
                "reading" => {
                    if status.as_deref() != Some("reading") {
                        continue;
                    }
                }
                "finished" => {
                    if status.as_deref() != Some("finished") {
                        continue;
                    }
                }
                "wantToRead" => {
                    if status.as_deref() != Some("wantToRead") {
                        continue;
                    }
                }
                "dropped" => {
                    if status.as_deref() != Some("dropped") {
                        continue;
                    }
                }
                _ => {}
            }
        }

        books.push(LibraryBook {
            title: book_ref.title.clone(),
            author: book_ref.authors.as_ref().and_then(|a| a.first()).cloned(),
            publication_year: book_ref.publication_year,
            status,
            has_notes: entry.value.notes.is_some(),
            book_id,
        });
    }

    // Sort alphabetically by title
    books.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

    let template = BookListTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: user.username,
        books,
        filter: query.filter,
    };

    Html(template.render().unwrap()).into_response()
}

/// Data needed to sync a book entry to the PDS
struct PdsBookEntry<'a> {
    title: &'a str,
    author: Option<&'a str>,
    publication_year: Option<i32>,
    status: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
}

/// Sync a book entry to the user's AT Protocol PDS.
/// Logs errors but does not propagate them.
async fn sync_book_to_pds(db: &Database, user: &User, entry: PdsBookEntry<'_>) {
    let Some(pds_client) = PdsClient::from_env() else {
        // PDS not configured
        return;
    };

    let Some(mut auth_pds) = AuthenticatedPds::new(&pds_client, db, user) else {
        // User doesn't have AT Protocol credentials, skip sync
        return;
    };

    // Create book reference
    let book_ref = BookRef {
        title: entry.title.to_string(),
        authors: entry.author.map(|a| vec![a.to_string()]),
        publication_year: entry.publication_year,
        isbn: None,
    };

    // Create book entry on PDS
    match auth_pds
        .create_book_entry(book_ref, entry.status, entry.started_at, entry.finished_at)
        .await
    {
        Ok(response) => {
            println!(
                "Synced book to PDS: {} (uri: {})",
                entry.title, response.uri
            );
        }
        Err(error) => {
            eprintln!("Failed to sync book to PDS: {error}");
            // Don't fail the local operation
        }
    }
}

/// Check if two books match by title and author (case-insensitive)
fn books_match(
    entry_title: &str,
    entry_authors: Option<&Vec<String>>,
    book_title: &str,
    book_author: Option<&str>,
) -> bool {
    let title_matches = entry_title.to_lowercase() == book_title.to_lowercase();
    let author_matches = match (entry_authors, book_author) {
        (Some(authors), Some(ba)) => authors
            .iter()
            .any(|a| a.to_lowercase() == ba.to_lowercase()),
        (None, None) => true,
        _ => false,
    };
    title_matches && author_matches
}

pub async fn book_detail(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(book_id): Path<String>,
) -> Response {
    let user = current_user(&db, &headers).await;
    let current_did = user.as_ref().and_then(|u| u.did.clone());

    let book = match db.get_book_by_id(&book_id).await {
        Ok(Some(book)) => book,
        Ok(None) => return Redirect::to("/").into_response(),
        Err(error) => {
            eprintln!("Error fetching book: {error}");
            return Redirect::to("/").into_response();
        }
    };

    // Fetch activity from PDS
    let mut activities: Vec<BookActivity> = Vec::new();
    let mut in_current_user_library = false;
    let mut current_user_status: Option<String> = None;
    let mut current_user_notes: Option<String> = None;

    if let Some(pds_client) = PdsClient::from_env() {
        // Check if the current user has this book in their library
        if let Some(ref did) = current_did
            && let Ok(entries) = pds_client.list_book_entries(did).await
        {
            for entry in entries {
                let entry_book = &entry.value.book;
                if books_match(
                    &entry_book.title,
                    entry_book.authors.as_ref(),
                    &book.title,
                    book.author.as_deref(),
                ) {
                    in_current_user_library = true;
                    current_user_status = entry.value.status.clone();
                    current_user_notes = entry.value.notes.clone();
                    break;
                }
            }
        }

        // Get all repositories from the PDS
        let repos = match pds_client.list_repos(Some(100)).await {
            Ok(repos) => repos,
            Err(error) => {
                eprintln!("Error fetching repos from PDS: {error}");
                vec![]
            }
        };

        for repo in repos {
            let did = &repo.did;

            // Look up username from local database, or resolve handle from PLC directory
            let username = match db.get_user_by_did(did).await {
                Ok(Some(user)) => user.username,
                _ => {
                    // Resolve handle from PLC directory (works for any did:plc)
                    resolve_handle_from_plc(did)
                        .await
                        .unwrap_or_else(|| did.clone())
                }
            };

            // Fetch book entries for this repo
            let entries = match pds_client.list_book_entries(did).await {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            // Check if any entry matches this book
            for entry in entries {
                let entry_book = &entry.value.book;
                if books_match(
                    &entry_book.title,
                    entry_book.authors.as_ref(),
                    &book.title,
                    book.author.as_deref(),
                ) {
                    activities.push(BookActivity {
                        username: username.clone(),
                        did: did.clone(),
                        action: "added".to_string(),
                        created_at: entry.value.created_at.clone(),
                    });
                }
            }
        }

        // Sort by created_at descending (most recent first)
        activities.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    }

    let template = BookDetailTemplate {
        is_authenticated: user.is_some(),
        signups_disabled: signups_disabled(),
        username: user.map(|u| u.username).unwrap_or_default(),
        book,
        activities,
        in_current_user_library,
        current_user_status,
        current_user_notes,
    };
    Html(template.render().unwrap()).into_response()
}

pub async fn book_delete(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(book_id): Path<String>,
) -> Response {
    let user = current_user(&db, &headers).await;

    if user.is_none() {
        return Redirect::to("/login").into_response();
    }

    match db.delete_book(&book_id).await {
        Ok(_) => Redirect::to("/").into_response(),
        Err(error) => {
            eprintln!("Error deleting book: {error}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Could not delete book").into_response()
        }
    }
}

pub async fn book_edit_page(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(book_id): Path<String>,
) -> Response {
    let user = current_user(&db, &headers).await;

    if user.is_none() {
        return Redirect::to("/login").into_response();
    }

    match db.get_book_by_id(&book_id).await {
        Ok(Some(book)) => {
            let template = BookEditTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.map(|u| u.username).unwrap_or_default(),
                book,
                error_message: None,
            };
            Html(template.render().unwrap()).into_response()
        }
        Ok(None) => Redirect::to("/").into_response(),
        Err(error) => {
            eprintln!("Error fetching book: {error}");
            Redirect::to("/").into_response()
        }
    }
}

pub async fn book_edit_submit(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(book_id): Path<String>,
    Form(form): Form<EditBookForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let title = form.title.trim();
    if title.is_empty() {
        if let Ok(Some(book)) = db.get_book_by_id(&book_id).await {
            let template = BookEditTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                book,
                error_message: Some("Title is required".to_string()),
            };
            return Html(template.render().unwrap()).into_response();
        }
        return Redirect::to("/").into_response();
    }

    let author = if form.author.trim().is_empty() {
        None
    } else {
        Some(form.author.trim())
    };

    let publication_year = form.publication_year.trim().parse::<i32>().ok();

    let update = BookUpdate {
        title,
        author,
        publication_year,
    };

    match db.update_book(&book_id, update).await {
        Ok(_) => {
            // Sync updated book to PDS (status/dates are None for edits, we don't change them)
            if let Ok(Some(book)) = db.get_book_by_id(&book_id).await {
                sync_book_to_pds(
                    &db,
                    &user,
                    PdsBookEntry {
                        title: &book.title,
                        author: book.author.as_deref(),
                        publication_year: book.publication_year,
                        status: None,
                        started_at: None,
                        finished_at: None,
                    },
                )
                .await;
            }
            Redirect::to(&format!("/books/{}", book_id)).into_response()
        }
        Err(error) => {
            eprintln!("Book update error: {error}");
            if let Ok(Some(book)) = db.get_book_by_id(&book_id).await {
                let template = BookEditTemplate {
                    is_authenticated: true,
                    signups_disabled: signups_disabled(),
                    username: user.username.clone(),
                    book,
                    error_message: Some("Could not update book. Please try again.".to_string()),
                };
                return Html(template.render().unwrap()).into_response();
            }
            Redirect::to("/").into_response()
        }
    }
}

pub async fn book_edit_chat_page(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(book_id): Path<String>,
) -> Response {
    let user = current_user(&db, &headers).await;

    if user.is_none() {
        return Redirect::to("/login").into_response();
    }

    match db.get_book_by_id(&book_id).await {
        Ok(Some(book)) => {
            let template = BookEditChatTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.map(|u| u.username).unwrap_or_default(),
                book,
                error_message: None,
                edit_result: None,
            };
            Html(template.render().unwrap()).into_response()
        }
        Ok(None) => Redirect::to("/").into_response(),
        Err(error) => {
            eprintln!("Error fetching book: {error}");
            Redirect::to("/").into_response()
        }
    }
}

pub async fn book_edit_chat_submit(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(book_id): Path<String>,
    Form(form): Form<EditChatForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let book = match db.get_book_by_id(&book_id).await {
        Ok(Some(book)) => book,
        Ok(None) => return Redirect::to("/").into_response(),
        Err(error) => {
            eprintln!("Error fetching book: {error}");
            return Redirect::to("/").into_response();
        }
    };

    let instruction = form.instruction.trim();
    if instruction.is_empty() {
        let template = BookEditChatTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            book,
            error_message: Some("Please enter an instruction".to_string()),
            edit_result: None,
        };
        return Html(template.render().unwrap()).into_response();
    }

    // Edit book with AI (GPT or Claude based on model selection)
    let edit_result = match edit_book_with_ai(
        &book.title,
        book.author.as_deref(),
        book.publication_year,
        instruction,
        &form.model,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            eprintln!("AI error: {error}");
            let template = BookEditChatTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                book,
                error_message: Some(format!("AI error: {error}")),
                edit_result: None,
            };
            return Html(template.render().unwrap()).into_response();
        }
    };

    // Convert to BookEditResult for the template
    let edit_result = crate::gpt::BookEditResult {
        title: edit_result.title,
        author: edit_result.author,
        publication_year: edit_result.publication_year,
    };

    let template = BookEditChatTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: user.username,
        book,
        error_message: None,
        edit_result: Some(edit_result),
    };
    Html(template.render().unwrap()).into_response()
}

pub async fn book_edit_chat_apply(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(book_id): Path<String>,
    Form(form): Form<EditChatApplyForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    if user.is_none() {
        return Redirect::to("/login").into_response();
    }

    let title = form.title.trim();
    if title.is_empty() {
        return Redirect::to(&format!("/books/{}/edit-chat", book_id)).into_response();
    }

    let author = if form.author.trim().is_empty() {
        None
    } else {
        Some(form.author.trim())
    };

    let publication_year = form.publication_year.trim().parse::<i32>().ok();

    let update = BookUpdate {
        title,
        author,
        publication_year,
    };

    match db.update_book(&book_id, update).await {
        Ok(_) => Redirect::to(&format!("/books/{}", book_id)).into_response(),
        Err(error) => {
            eprintln!("Book update error: {error}");
            Redirect::to(&format!("/books/{}/edit-chat", book_id)).into_response()
        }
    }
}

// Combined add book page handlers

pub async fn book_add_page(
    State(db): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let user = current_user(&db, &headers).await;

    if user.is_none() {
        return Redirect::to("/login").into_response();
    }

    // Get prefilled query from URL parameter (e.g., /books/add?q=BookTitle+Author)
    let prefill_query = params.get("q").cloned().unwrap_or_default();

    let template = BookAddTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: user.map(|u| u.username).unwrap_or_default(),
        error_message: None,
        extracted_metadata: None,
        model: "gpt-5.1".to_string(),
        query: prefill_query,
    };

    Html(template.render().unwrap()).into_response()
}

pub async fn book_add_extract(
    State(db): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<BookAddExtractForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let query = form.query.trim();
    if query.is_empty() {
        let template = BookAddTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            error_message: Some("Please enter a book".to_string()),
            extracted_metadata: None,
            model: form.model.clone(),
            query: String::new(),
        };
        return Html(template.render().unwrap()).into_response();
    }

    // Extract metadata using AI (GPT or Claude based on model selection)
    let metadata = match extract_metadata_with_ai(query, &form.model).await {
        Ok(m) => m,
        Err(error) => {
            eprintln!("AI error: {error}");
            let template = BookAddTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                error_message: Some(format!("Could not identify book: {error}")),
                extracted_metadata: None,
                model: form.model.clone(),
                query: query.to_string(),
            };
            return Html(template.render().unwrap()).into_response();
        }
    };

    let template = BookAddTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: user.username,
        error_message: None,
        extracted_metadata: Some(metadata),
        model: form.model,
        query: query.to_string(),
    };
    Html(template.render().unwrap()).into_response()
}

pub async fn book_add_refine(
    State(db): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<BookAddRefineForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let instruction = form.instruction.trim();
    if instruction.is_empty() {
        // No instruction given, just return to the same state
        let metadata = BookMetadata {
            title: form.title.clone(),
            author: if form.author.trim().is_empty() {
                None
            } else {
                Some(form.author.clone())
            },
            publication_year: form.publication_year.trim().parse::<i32>().ok(),
        };
        let template = BookAddTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            error_message: Some("Please enter an instruction".to_string()),
            extracted_metadata: Some(metadata),
            model: form.model,
            query: form.query,
        };
        return Html(template.render().unwrap()).into_response();
    }

    let current_author = if form.author.trim().is_empty() {
        None
    } else {
        Some(form.author.trim())
    };
    let current_publication_year = form.publication_year.trim().parse::<i32>().ok();

    // Edit book with AI (GPT or Claude based on model selection)
    let metadata = match edit_book_with_ai(
        &form.title,
        current_author,
        current_publication_year,
        instruction,
        &form.model,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            eprintln!("AI error: {error}");
            let metadata = BookMetadata {
                title: form.title.clone(),
                author: current_author.map(|s| s.to_string()),
                publication_year: current_publication_year,
            };
            let template = BookAddTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                error_message: Some(format!("AI error: {error}")),
                extracted_metadata: Some(metadata),
                model: form.model,
                query: form.query,
            };
            return Html(template.render().unwrap()).into_response();
        }
    };

    let template = BookAddTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: user.username,
        error_message: None,
        extracted_metadata: Some(metadata),
        model: form.model,
        query: form.query,
    };
    Html(template.render().unwrap()).into_response()
}

pub async fn book_add_save(
    State(db): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<BookAddSaveForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let title = form.title.trim();
    if title.is_empty() {
        let template = BookAddTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            error_message: Some("Title is required".to_string()),
            extracted_metadata: None,
            model: form.model,
            query: form.query,
        };
        return Html(template.render().unwrap()).into_response();
    }

    let author = if form.author.trim().is_empty() {
        None
    } else {
        Some(form.author.trim())
    };

    let publication_year = form.publication_year.trim().parse::<i32>().ok();

    // Parse status, defaulting to wantToRead if empty
    let status = if form.status.trim().is_empty() {
        Some("wantToRead".to_string())
    } else {
        Some(form.status.trim().to_string())
    };

    // Parse dates (convert from YYYY-MM-DD to RFC3339 format)
    let started_at = if form.started_at.trim().is_empty() {
        None
    } else {
        Some(format!("{}T00:00:00Z", form.started_at.trim()))
    };

    let finished_at = if form.finished_at.trim().is_empty() {
        None
    } else {
        Some(format!("{}T00:00:00Z", form.finished_at.trim()))
    };

    // Use find_or_create to avoid duplicates
    match db
        .find_or_create_book(title, author, publication_year)
        .await
    {
        Ok(book_id) => {
            // Sync book to PDS (don't fail if this errors)
            sync_book_to_pds(
                &db,
                &user,
                PdsBookEntry {
                    title,
                    author,
                    publication_year,
                    status,
                    started_at,
                    finished_at,
                },
            )
            .await;
            Redirect::to(&format!("/books/{}", book_id)).into_response()
        }
        Err(error) => {
            eprintln!("Book creation error: {error}");
            let metadata = BookMetadata {
                title: form.title.clone(),
                author: author.map(|s| s.to_string()),
                publication_year,
            };
            let template = BookAddTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username.clone(),
                error_message: Some("Could not save book. Please try again.".to_string()),
                extracted_metadata: Some(metadata),
                model: form.model,
                query: form.query,
            };
            Html(template.render().unwrap()).into_response()
        }
    }
}

// Book include page - add a book from the global feed to your library

#[derive(Deserialize)]
pub struct BookIncludeQuery {
    pub title: Option<String>,
    pub author: Option<String>,
    pub year: Option<String>,
}

pub async fn book_include_page(
    State(db): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<BookIncludeQuery>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let template = BookIncludeTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: user.username,
        error_message: None,
        title: params.title.unwrap_or_default(),
        author: params.author.unwrap_or_default(),
        publication_year: params.year.unwrap_or_default(),
    };

    Html(template.render().unwrap()).into_response()
}

#[derive(Deserialize)]
pub struct BookIncludeForm {
    pub title: String,
    pub author: String,
    pub publication_year: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: String,
}

#[derive(Deserialize)]
pub struct ApiLibraryRequest {
    pub title: String,
    pub author: Option<String>,
    pub publication_year: Option<i32>,
    pub status: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Serialize)]
pub struct ApiLibraryResponse {
    success: bool,
    book_id: Option<String>,
    status: Option<String>,
    message: Option<String>,
}

pub async fn book_include_save(
    State(db): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<BookIncludeForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let title = form.title.trim();
    if title.is_empty() {
        let template = BookIncludeTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            error_message: Some("Title is required".to_string()),
            title: form.title,
            author: form.author,
            publication_year: form.publication_year,
        };
        return Html(template.render().unwrap()).into_response();
    }

    let author = if form.author.trim().is_empty() {
        None
    } else {
        Some(form.author.trim())
    };

    let publication_year = form.publication_year.trim().parse::<i32>().ok();

    // Parse status
    let status = if form.status.trim().is_empty() {
        Some("wantToRead".to_string())
    } else {
        Some(form.status.trim().to_string())
    };

    // Parse dates (convert from YYYY-MM-DD to RFC3339 format)
    let started_at = if form.started_at.trim().is_empty() {
        None
    } else {
        Some(format!("{}T00:00:00Z", form.started_at.trim()))
    };

    let finished_at = if form.finished_at.trim().is_empty() {
        None
    } else {
        Some(format!("{}T00:00:00Z", form.finished_at.trim()))
    };

    // Save book to local database (deduplicates by title/author/year)
    let book_id = match db
        .find_or_create_book(title, author, publication_year)
        .await
    {
        Ok(id) => id,
        Err(error) => {
            eprintln!("Book creation error: {error}");
            let template = BookIncludeTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                error_message: Some("Could not save book. Please try again.".to_string()),
                title: form.title,
                author: form.author,
                publication_year: form.publication_year,
            };
            return Html(template.render().unwrap()).into_response();
        }
    };

    // Sync to PDS with user's personal data
    sync_book_to_pds(
        &db,
        &user,
        PdsBookEntry {
            title,
            author,
            publication_year,
            status,
            started_at,
            finished_at,
        },
    )
    .await;

    Redirect::to(&format!("/books/{}", book_id)).into_response()
}

pub async fn api_library_add(
    State(db): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ApiLibraryRequest>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiLibraryResponse {
                success: false,
                book_id: None,
                status: None,
                message: Some("login required".to_string()),
            }),
        )
            .into_response();
    };

    let title = payload.title.trim();
    if title.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiLibraryResponse {
                success: false,
                book_id: None,
                status: None,
                message: Some("title is required".to_string()),
            }),
        )
            .into_response();
    }

    let author = payload
        .author
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let publication_year = payload.publication_year;

    let status_value = payload
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("wantToRead")
        .to_string();

    let started_at = payload
        .started_at
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|date| {
            if date.contains('T') {
                date.to_string()
            } else {
                format!("{date}T00:00:00Z")
            }
        });

    let finished_at = payload
        .finished_at
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|date| {
            if date.contains('T') {
                date.to_string()
            } else {
                format!("{date}T00:00:00Z")
            }
        });

    let book_id = match db
        .find_or_create_book(title, author, publication_year)
        .await
    {
        Ok(id) => id,
        Err(error) => {
            eprintln!("Book creation error (API): {error}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiLibraryResponse {
                    success: false,
                    book_id: None,
                    status: None,
                    message: Some("could not save book".to_string()),
                }),
            )
                .into_response();
        }
    };

    // Sync to PDS with user's personal data
    sync_book_to_pds(
        &db,
        &user,
        PdsBookEntry {
            title,
            author,
            publication_year,
            status: Some(status_value.clone()),
            started_at,
            finished_at,
        },
    )
    .await;

    (
        StatusCode::OK,
        Json(ApiLibraryResponse {
            success: true,
            book_id: Some(book_id),
            status: Some(status_value),
            message: None,
        }),
    )
        .into_response()
}

/// Show the book notes page
pub async fn book_notes_page(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(book_id): Path<String>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let book = match db.get_book_by_id(&book_id).await {
        Ok(Some(book)) => book,
        Ok(None) => return Redirect::to("/").into_response(),
        Err(error) => {
            eprintln!("Error fetching book: {error}");
            return Redirect::to("/").into_response();
        }
    };

    // Get PDS client
    let Some(pds_client) = PdsClient::from_env() else {
        let template = BookNotesTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            book,
            current_notes: None,
            error_message: Some("AT Protocol not configured".to_string()),
            success_message: None,
        };
        return Html(template.render().unwrap()).into_response();
    };

    let Some(user_did) = &user.did else {
        let template = BookNotesTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            book,
            current_notes: None,
            error_message: Some("You don't have an AT Protocol account".to_string()),
            success_message: None,
        };
        return Html(template.render().unwrap()).into_response();
    };

    // Find user's book entry for this book
    let entries = match pds_client.list_book_entries(user_did).await {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Error fetching book entries: {error}");
            let template = BookNotesTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                book,
                current_notes: None,
                error_message: Some("Could not fetch your library".to_string()),
                success_message: None,
            };
            return Html(template.render().unwrap()).into_response();
        }
    };

    // Find the matching entry
    let mut current_notes = None;
    for entry in entries {
        let entry_book = &entry.value.book;
        if books_match(
            &entry_book.title,
            entry_book.authors.as_ref(),
            &book.title,
            book.author.as_deref(),
        ) {
            current_notes = entry.value.notes.clone();
            break;
        }
    }

    let template = BookNotesTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: user.username,
        book,
        current_notes,
        error_message: None,
        success_message: None,
    };

    Html(template.render().unwrap()).into_response()
}

#[derive(Deserialize)]
pub struct BookNotesForm {
    pub notes: String,
}

/// Submit book notes
pub async fn book_notes_submit(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(book_id): Path<String>,
    Form(form): Form<BookNotesForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    // Validate notes length (max 20,000 characters)
    if form.notes.len() > 20_000 {
        let book = match db.get_book_by_id(&book_id).await {
            Ok(Some(book)) => book,
            Ok(None) => return Redirect::to("/").into_response(),
            Err(_) => return Redirect::to("/").into_response(),
        };
        let template = BookNotesTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            book,
            current_notes: Some(form.notes),
            error_message: Some("Notes must be 20,000 characters or less".to_string()),
            success_message: None,
        };
        return Html(template.render().unwrap()).into_response();
    }

    let book = match db.get_book_by_id(&book_id).await {
        Ok(Some(book)) => book,
        Ok(None) => return Redirect::to("/").into_response(),
        Err(error) => {
            eprintln!("Error fetching book: {error}");
            return Redirect::to("/").into_response();
        }
    };

    // Get PDS client
    let Some(pds_client) = PdsClient::from_env() else {
        let template = BookNotesTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            book,
            current_notes: Some(form.notes),
            error_message: Some("AT Protocol not configured".to_string()),
            success_message: None,
        };
        return Html(template.render().unwrap()).into_response();
    };

    let Some(mut auth_pds) = AuthenticatedPds::new(&pds_client, &db, &user) else {
        let template = BookNotesTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            book,
            current_notes: Some(form.notes),
            error_message: Some("AT Protocol credentials not available".to_string()),
            success_message: None,
        };
        return Html(template.render().unwrap()).into_response();
    };

    // Find user's book entry for this book
    let entries = match pds_client.list_book_entries(auth_pds.did()).await {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Error fetching book entries: {error}");
            let template = BookNotesTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                book,
                current_notes: Some(form.notes),
                error_message: Some("Could not fetch your library".to_string()),
                success_message: None,
            };
            return Html(template.render().unwrap()).into_response();
        }
    };

    // Find the matching entry and get its rkey
    let mut found_entry = None;
    for entry in entries {
        let entry_book = &entry.value.book;
        if books_match(
            &entry_book.title,
            entry_book.authors.as_ref(),
            &book.title,
            book.author.as_deref(),
        ) {
            // Extract rkey from URI: at://did/collection/rkey
            if let Some(rkey) = entry.uri.split('/').next_back() {
                found_entry = Some((rkey.to_string(), entry.value));
            }
            break;
        }
    }

    let Some((rkey, mut entry_record)) = found_entry else {
        let template = BookNotesTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            book,
            current_notes: Some(form.notes),
            error_message: Some("This book is not in your library".to_string()),
            success_message: None,
        };
        return Html(template.render().unwrap()).into_response();
    };

    // Update the notes field
    let notes = form.notes.trim().to_string();
    entry_record.notes = if notes.is_empty() {
        None
    } else {
        Some(notes.clone())
    };

    // Update the record on PDS
    match auth_pds.update_book_entry(&rkey, entry_record).await {
        Ok(_) => {
            println!("Updated notes for book {} (rkey: {})", book_id, rkey);
            let template = BookNotesTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                book,
                current_notes: if notes.is_empty() { None } else { Some(notes) },
                error_message: None,
                success_message: Some("Notes saved successfully".to_string()),
            };
            Html(template.render().unwrap()).into_response()
        }
        Err(error) => {
            eprintln!("Error updating book entry: {error}");
            let template = BookNotesTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                book,
                current_notes: if notes.is_empty() { None } else { Some(notes) },
                error_message: Some(format!("Could not save notes: {}", error)),
                success_message: None,
            };
            Html(template.render().unwrap()).into_response()
        }
    }
}
