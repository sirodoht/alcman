use askama::Template;

use crate::books::Book;
use crate::gpt::{BookEditResult, BookMetadata};

#[derive(Template)]
#[template(path = "book_list.html")]
pub struct BookListTemplate {
    pub is_authenticated: bool,
    pub signups_disabled: bool,
    pub username: String,
    pub books: Vec<Book>,
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub is_authenticated: bool,
    pub signups_disabled: bool,
    pub username: String,
    pub form_username: String,
    pub error_message: Option<String>,
}

#[derive(Template)]
#[template(path = "signup.html")]
pub struct SignupTemplate {
    pub is_authenticated: bool,
    pub signups_disabled: bool,
    pub username: String,
    pub form_username: String,
    pub form_email: String,
    pub form_invite_code: String,
    pub error_message: Option<String>,
}

#[derive(Template)]
#[template(path = "book_detail.html")]
pub struct BookDetailTemplate {
    pub is_authenticated: bool,
    pub signups_disabled: bool,
    pub username: String,
    pub book: Book,
}

#[derive(Template)]
#[template(path = "book_edit.html")]
pub struct BookEditTemplate {
    pub is_authenticated: bool,
    pub signups_disabled: bool,
    pub username: String,
    pub book: Book,
    pub error_message: Option<String>,
}

#[derive(Template)]
#[template(path = "profile.html")]
pub struct ProfileTemplate {
    pub is_authenticated: bool,
    pub signups_disabled: bool,
    /// Current logged-in user's username (for nav)
    pub username: String,
    /// Profile owner's username
    pub profile_username: String,
    /// Profile owner's DID
    pub profile_did: Option<String>,
    /// Book entries from the PDS
    pub book_entries: Vec<crate::atproto::RecordEntry<crate::atproto::BookEntryRecord>>,
    /// Whether the current user can follow this profile (logged in, different user, profile has DID)
    pub can_follow: bool,
    /// Whether the current user is already following this profile
    pub is_following: bool,
}

#[derive(Template)]
#[template(path = "change_password.html")]
pub struct ChangePasswordTemplate {
    pub is_authenticated: bool,
    pub signups_disabled: bool,
    pub username: String,
    pub error_message: Option<String>,
    pub success_message: Option<String>,
}

#[derive(Template)]
#[template(path = "book_edit_chat.html")]
pub struct BookEditChatTemplate {
    pub is_authenticated: bool,
    pub signups_disabled: bool,
    pub username: String,
    pub book: Book,
    pub error_message: Option<String>,
    pub edit_result: Option<BookEditResult>,
}

#[derive(Template)]
#[template(path = "feed.html")]
pub struct FeedTemplate {
    pub is_authenticated: bool,
    pub signups_disabled: bool,
    pub username: String,
    /// Feed items (book entries with author info)
    pub feed_items: Vec<crate::atproto::FeedItem>,
}

#[derive(Template)]
#[template(path = "global_feed.html")]
pub struct GlobalFeedTemplate {
    pub is_authenticated: bool,
    pub signups_disabled: bool,
    pub username: String,
    /// Feed items (book entries with author info)
    pub feed_items: Vec<crate::atproto::FeedItem>,
}

#[derive(Template)]
#[template(path = "book_add.html")]
pub struct BookAddTemplate {
    pub is_authenticated: bool,
    pub signups_disabled: bool,
    pub username: String,
    pub error_message: Option<String>,
    /// Present after GPT extraction - contains title, author, publication_year
    pub extracted_metadata: Option<BookMetadata>,
    /// The model to use for GPT calls (preserved across requests)
    pub model: String,
    /// The original query that was searched (for display purposes)
    pub query: String,
}
