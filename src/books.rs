use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::atproto::{BookRef, PdsClient, resolve_handle_from_plc};
use crate::auth::{User, current_user, signups_disabled};
use crate::claude::{ClaudeClient, ClaudeConfig};
use crate::database::{BookUpdate, Database};
use crate::gpt::BookMetadata;
use crate::gpt::{GptClient, GptConfig};
use crate::pds::AuthenticatedPds;
use crate::templates::{
    BookAddTemplate, BookDetailTemplate, BookEditChatTemplate, BookEditTemplate, BookListTemplate,
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

pub async fn book_list(State(db): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let user = current_user(&db, &headers).await;
    let books = db.get_all_books().await.unwrap_or_default();

    let template = BookListTemplate {
        is_authenticated: user.is_some(),
        signups_disabled: signups_disabled(),
        username: user.map(|u| u.username).unwrap_or_default(),
        books,
    };

    Html(template.render().unwrap())
}

/// Sync a book entry to the user's AT Protocol PDS.
/// Logs errors but does not propagate them.
async fn sync_book_to_pds(
    db: &Database,
    user: &User,
    title: &str,
    author: Option<&str>,
    publication_year: Option<i32>,
    status: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
) {
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
        title: title.to_string(),
        authors: author.map(|a| vec![a.to_string()]),
        publication_year,
        isbn: None,
    };

    // Create book entry on PDS
    match auth_pds
        .create_book_entry(book_ref, status, started_at, finished_at)
        .await
    {
        Ok(response) => {
            println!("Synced book to PDS: {} (uri: {})", title, response.uri);
        }
        Err(error) => {
            eprintln!("Failed to sync book to PDS: {error}");
            // Don't fail the local operation
        }
    }
}

pub async fn book_detail(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(book_id): Path<String>,
) -> Response {
    let user = current_user(&db, &headers).await;

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

    if let Some(pds_client) = PdsClient::from_env() {
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
                let title_matches = entry_book.title.to_lowercase() == book.title.to_lowercase();
                let author_matches = match (&entry_book.authors, &book.author) {
                    (Some(authors), Some(book_author)) => authors
                        .iter()
                        .any(|a| a.to_lowercase() == book_author.to_lowercase()),
                    (None, None) => true,
                    _ => false,
                };

                if title_matches && author_matches {
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
                    &book.title,
                    book.author.as_deref(),
                    book.publication_year,
                    None,
                    None,
                    None,
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

pub async fn book_add_page(State(db): State<AppState>, headers: HeaderMap) -> Response {
    let user = current_user(&db, &headers).await;

    if user.is_none() {
        return Redirect::to("/login").into_response();
    }

    let template = BookAddTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: user.map(|u| u.username).unwrap_or_default(),
        error_message: None,
        extracted_metadata: None,
        model: "gpt-5.1".to_string(),
        query: String::new(),
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
                title,
                author,
                publication_year,
                status,
                started_at,
                finished_at,
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
