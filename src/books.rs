use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};
use std::env;

use crate::AppState;
use crate::atproto::{BookRef, PdsClient};
use crate::auth::{User, current_user, signups_disabled};
use crate::gpt::{GptClient, GptConfig};
use crate::templates::{
    BookDetailTemplate, BookEditChatTemplate, BookEditNotesTemplate, BookEditTemplate,
    BookFormTemplate, BookListTemplate, QuickAddTemplate,
};

// Book-related structures
#[derive(sqlx::FromRow, Serialize, Clone)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub publication_year: Option<i32>,
    pub filepath: Option<String>,
    pub notes: Option<String>,
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

#[derive(Deserialize)]
pub struct CreateBookForm {
    pub title: String,
    pub author: String,
    pub publication_year: String,
    pub notes: String,
}

#[derive(Deserialize)]
pub struct QuickAddForm {
    pub query: String,
    pub model: String,
}

#[derive(Deserialize)]
pub struct EditBookForm {
    pub title: String,
    pub author: String,
    pub publication_year: String,
}

#[derive(Deserialize)]
pub struct EditNotesForm {
    pub notes: String,
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
pub struct BookListQuery {
    pub notes: Option<String>,
}

pub async fn book_list(
    State(db): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<BookListQuery>,
) -> impl IntoResponse {
    let user = current_user(&db, &headers).await;
    let all_books = db.get_all_books().await.unwrap_or_default();

    let notes = query.notes.as_deref() == Some("true");
    let books = if notes {
        all_books
            .into_iter()
            .filter(|b| b.notes.is_some())
            .collect()
    } else {
        all_books
    };

    let template = BookListTemplate {
        is_authenticated: user.is_some(),
        signups_disabled: signups_disabled(),
        username: user.map(|u| u.username).unwrap_or_default(),
        books,
        notes,
    };

    Html(template.render().unwrap())
}

pub async fn book_form_page(State(db): State<AppState>, headers: HeaderMap) -> Response {
    let user = current_user(&db, &headers).await;

    if user.is_none() {
        return Redirect::to("/login").into_response();
    }

    let template = BookFormTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: user.map(|u| u.username).unwrap_or_default(),
        error_message: None,
    };

    Html(template.render().unwrap()).into_response()
}

pub async fn book_create(
    State(db): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CreateBookForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let title = form.title.trim();
    if title.is_empty() {
        let template = BookFormTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            error_message: Some("Title is required".to_string()),
        };
        return Html(template.render().unwrap()).into_response();
    }

    let author = if form.author.trim().is_empty() {
        None
    } else {
        Some(form.author.trim())
    };

    let publication_year = form.publication_year.trim().parse::<i32>().ok();

    let notes = if form.notes.trim().is_empty() {
        None
    } else {
        Some(form.notes.trim())
    };

    match db.create_book(title, author, publication_year, notes).await {
        Ok(_) => {
            // Sync book to PDS (don't fail if this errors)
            sync_book_to_pds(
                &user,
                title,
                author,
                publication_year,
                notes.map(String::from),
            )
            .await;
            Redirect::to("/").into_response()
        }
        Err(error) => {
            eprintln!("Book creation error: {error}");
            let template = BookFormTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username.clone(),
                error_message: Some("Could not create book. Please try again.".to_string()),
            };
            Html(template.render().unwrap()).into_response()
        }
    }
}

/// Sync a book entry to the user's AT Protocol PDS.
/// Logs errors but does not propagate them.
async fn sync_book_to_pds(
    user: &User,
    title: &str,
    author: Option<&str>,
    publication_year: Option<i32>,
    notes: Option<String>,
) {
    // Check if user has AT Protocol credentials
    let (Some(did), Some(access_jwt)) = (&user.did, &user.access_jwt) else {
        // User doesn't have AT Protocol credentials, skip sync
        return;
    };

    let Some(pds_client) = PdsClient::from_env() else {
        // PDS not configured
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
    match pds_client
        .create_book_entry(access_jwt, did, book_ref, notes)
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

    match db.get_book_by_id(&book_id).await {
        Ok(Some(book)) => {
            let template = BookDetailTemplate {
                is_authenticated: user.is_some(),
                signups_disabled: signups_disabled(),
                username: user.map(|u| u.username).unwrap_or_default(),
                book,
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

pub async fn book_download(State(db): State<AppState>, Path(book_id): Path<String>) -> Response {
    let book = match db.get_book_by_id(&book_id).await {
        Ok(Some(book)) => book,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Book not found").into_response();
        }
        Err(error) => {
            eprintln!("Error fetching book: {error}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let Some(filepath) = &book.filepath else {
        return (StatusCode::NOT_FOUND, "No file associated with this book").into_response();
    };

    // Get library path from environment, default to current directory
    let library_path = env::var("LIBRARY_PATH").unwrap_or_else(|_| ".".to_string());
    let full_path = std::path::Path::new(&library_path).join(filepath);

    if !full_path.exists() {
        eprintln!("File not found: {}", full_path.display());
        return (StatusCode::NOT_FOUND, "File not found on disk").into_response();
    }

    let file_contents = match std::fs::read(&full_path) {
        Ok(contents) => contents,
        Err(error) => {
            eprintln!("Error reading file: {error}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Could not read file").into_response();
        }
    };

    // Determine content type based on extension
    let content_type = match full_path.extension().and_then(|e| e.to_str()) {
        Some("pdf") => "application/pdf",
        Some("epub") => "application/epub+zip",
        Some("mobi") => "application/x-mobipocket-ebook",
        Some("txt") => "text/plain",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        _ => "application/octet-stream",
    };

    // Get filename for Content-Disposition header
    let filename = full_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");

    let headers = [
        (header::CONTENT_TYPE, content_type),
        (
            header::CONTENT_DISPOSITION,
            &format!("attachment; filename=\"{}\"", filename),
        ),
    ];

    (headers, file_contents).into_response()
}

pub async fn quick_add_page(State(db): State<AppState>, headers: HeaderMap) -> Response {
    let user = current_user(&db, &headers).await;

    if user.is_none() {
        return Redirect::to("/login").into_response();
    }

    let template = QuickAddTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: user.map(|u| u.username).unwrap_or_default(),
        error_message: None,
    };

    Html(template.render().unwrap()).into_response()
}

pub async fn quick_add_submit(
    State(db): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<QuickAddForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let query = form.query.trim();
    if query.is_empty() {
        let template = QuickAddTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            error_message: Some("Please enter a book".to_string()),
        };
        return Html(template.render().unwrap()).into_response();
    }

    // Create GPT client and extract metadata
    let gpt = GptClient::new(GptConfig::from_env());

    if !gpt.has_api_key() {
        let template = QuickAddTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            error_message: Some("AI features not available (API key not configured)".to_string()),
        };
        return Html(template.render().unwrap()).into_response();
    }

    let metadata = match gpt.extract_book_metadata(query, &form.model).await {
        Ok(m) => m,
        Err(error) => {
            eprintln!("GPT error: {error}");
            let template = QuickAddTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                error_message: Some(format!("Could not identify book: {error}")),
            };
            return Html(template.render().unwrap()).into_response();
        }
    };

    // Create the book with extracted metadata
    match db
        .create_book(
            &metadata.title,
            metadata.author.as_deref(),
            metadata.publication_year,
            None,
        )
        .await
    {
        Ok(book_id) => {
            // Sync book to PDS (don't fail if this errors)
            sync_book_to_pds(
                &user,
                &metadata.title,
                metadata.author.as_deref(),
                metadata.publication_year,
                None,
            )
            .await;
            Redirect::to(&format!("/books/{}", book_id)).into_response()
        }
        Err(error) => {
            eprintln!("Book creation error: {error}");
            let template = QuickAddTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username.clone(),
                error_message: Some("Could not save book. Please try again.".to_string()),
            };
            Html(template.render().unwrap()).into_response()
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

    match db
        .update_book(&book_id, title, author, publication_year)
        .await
    {
        Ok(_) => Redirect::to(&format!("/books/{}", book_id)).into_response(),
        Err(error) => {
            eprintln!("Book update error: {error}");
            if let Ok(Some(book)) = db.get_book_by_id(&book_id).await {
                let template = BookEditTemplate {
                    is_authenticated: true,
                    signups_disabled: signups_disabled(),
                    username: user.username,
                    book,
                    error_message: Some("Could not update book. Please try again.".to_string()),
                };
                return Html(template.render().unwrap()).into_response();
            }
            Redirect::to("/").into_response()
        }
    }
}

pub async fn book_edit_notes_page(
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
            let template = BookEditNotesTemplate {
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

pub async fn book_edit_notes_submit(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(book_id): Path<String>,
    Form(form): Form<EditNotesForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let notes = if form.notes.trim().is_empty() {
        None
    } else {
        Some(form.notes.trim())
    };

    match db.update_book_notes(&book_id, notes).await {
        Ok(_) => {
            // Fetch the book to get full details for PDS sync
            if let Ok(Some(book)) = db.get_book_by_id(&book_id).await {
                sync_book_to_pds(
                    &user,
                    &book.title,
                    book.author.as_deref(),
                    book.publication_year,
                    book.notes,
                )
                .await;
            }
            Redirect::to(&format!("/books/{}", book_id)).into_response()
        }
        Err(error) => {
            eprintln!("Notes update error: {error}");
            Redirect::to(&format!("/books/{}", book_id)).into_response()
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

    // Create GPT client and process the instruction
    let gpt = GptClient::new(GptConfig::from_env());

    if !gpt.has_api_key() {
        let template = BookEditChatTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            book,
            error_message: Some("AI features not available (API key not configured)".to_string()),
            edit_result: None,
        };
        return Html(template.render().unwrap()).into_response();
    }

    let edit_result = match gpt
        .edit_book_with_instruction(
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
            eprintln!("GPT error: {error}");
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

    match db
        .update_book(&book_id, title, author, publication_year)
        .await
    {
        Ok(_) => Redirect::to(&format!("/books/{}", book_id)).into_response(),
        Err(error) => {
            eprintln!("Book update error: {error}");
            Redirect::to(&format!("/books/{}/edit-chat", book_id)).into_response()
        }
    }
}
