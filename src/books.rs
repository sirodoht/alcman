use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::atproto::{BookRef, PdsClient};
use crate::auth::{User, current_user, signups_disabled};
use crate::database::{BookUpdate, Database, NewBook};
use crate::gpt::BookMetadata;
use crate::gpt::{GptClient, GptConfig};
use crate::pds::AuthenticatedPds;
use crate::templates::{
    BookAddTemplate, BookDetailTemplate, BookEditChatTemplate, BookEditTemplate, BookListTemplate,
};

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
    match auth_pds.create_book_entry(book_ref).await {
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
            // Sync updated book to PDS
            if let Ok(Some(book)) = db.get_book_by_id(&book_id).await {
                sync_book_to_pds(
                    &db,
                    &user,
                    &book.title,
                    book.author.as_deref(),
                    book.publication_year,
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

    // Create GPT client and extract metadata
    let gpt = GptClient::new(GptConfig::from_env());

    if !gpt.has_api_key() {
        let template = BookAddTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            error_message: Some("AI features not available (API key not configured)".to_string()),
            extracted_metadata: None,
            model: form.model.clone(),
            query: query.to_string(),
        };
        return Html(template.render().unwrap()).into_response();
    }

    let metadata = match gpt.extract_book_metadata(query, &form.model).await {
        Ok(m) => m,
        Err(error) => {
            eprintln!("GPT error: {error}");
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

    // Create GPT client and process the instruction
    let gpt = GptClient::new(GptConfig::from_env());

    if !gpt.has_api_key() {
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
            error_message: Some("AI features not available (API key not configured)".to_string()),
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

    let edit_result = match gpt
        .edit_book_with_instruction(
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
            eprintln!("GPT error: {error}");
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

    // Convert BookEditResult to BookMetadata for the template
    let metadata = BookMetadata {
        title: edit_result.title,
        author: edit_result.author,
        publication_year: edit_result.publication_year,
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

    let new_book = NewBook {
        title,
        author,
        publication_year,
    };

    match db.create_book(new_book).await {
        Ok(book_id) => {
            // Sync book to PDS (don't fail if this errors)
            sync_book_to_pds(&db, &user, title, author, publication_year).await;
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
