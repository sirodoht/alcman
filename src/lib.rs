use axum::{
    Router,
    routing::{get, post},
};
use std::sync::Arc;

pub mod atproto;
pub mod auth;
pub mod books;
pub mod claude;
pub mod database;
pub mod feed;
pub mod gpt;
pub mod pds;
pub mod templates;

pub use auth::User;
pub use books::Book;
pub use database::Database;

// Application state
pub type AppState = Arc<Database>;

// App creation function
pub fn create_app(db: AppState) -> Router {
    use auth::{
        change_password, change_password_page, follow_user, login_page, login_submit, logout,
        profile_by_did_page, profile_redirect, signup_page, signup_submit, unfollow_user,
    };
    use books::{
        book_add_extract, book_add_page, book_add_refine, book_add_save, book_delete, book_detail,
        book_edit_chat_apply, book_edit_chat_page, book_edit_chat_submit, book_edit_page,
        book_edit_submit, book_include_page, book_include_save, book_list,
    };
    use feed::{feed_page, global_feed_page};

    Router::new()
        .route("/", get(global_feed_page))
        .route("/books", get(book_list))
        .route("/login", get(login_page).post(login_submit))
        .route("/signup", get(signup_page).post(signup_submit))
        .route("/logout", post(logout))
        .route("/feed", get(feed_page))
        .route("/profile", get(profile_redirect))
        .route("/profile/{did}", get(profile_by_did_page))
        .route("/profile/{did}/follow", post(follow_user))
        .route("/profile/{did}/unfollow", post(unfollow_user))
        .route(
            "/profile/password",
            get(change_password_page).post(change_password),
        )
        .route("/books/add", get(book_add_page))
        .route("/books/add/extract", post(book_add_extract))
        .route("/books/add/refine", post(book_add_refine))
        .route("/books/add/save", post(book_add_save))
        .route(
            "/books/include",
            get(book_include_page).post(book_include_save),
        )
        .route("/books/{id}", get(book_detail))
        .route(
            "/books/{id}/edit",
            get(book_edit_page).post(book_edit_submit),
        )
        .route(
            "/books/{id}/edit-chat",
            get(book_edit_chat_page).post(book_edit_chat_submit),
        )
        .route("/books/{id}/edit-chat/apply", post(book_edit_chat_apply))
        .route("/books/{id}/delete", post(book_delete))
        .with_state(db)
}
