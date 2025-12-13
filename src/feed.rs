use askama::Template;
use axum::{
    extract::State,
    http::HeaderMap,
    response::{Html, IntoResponse, Redirect, Response},
};
use std::collections::HashMap;

use crate::AppState;
use crate::atproto::{FeedItem, PdsClient};
use crate::auth::{current_user, signups_disabled};
use crate::templates::{FollowingFeedTemplate, GlobalFeedTemplate};

/// Redirect root to /global
pub async fn home_redirect() -> Redirect {
    Redirect::to("/global")
}

/// Show feed page with book entries from followed users
pub async fn following_feed_page(State(db): State<AppState>, headers: HeaderMap) -> Response {
    let current = current_user(&db, &headers).await;

    // Require authentication
    let Some(current) = current else {
        return Redirect::to("/login").into_response();
    };

    let Some(current_did) = &current.did else {
        // User doesn't have a DID, show empty feed
        let template = FollowingFeedTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: current.username,
            feed_items: vec![],
        };
        return Html(template.render().unwrap()).into_response();
    };

    let Some(pds_client) = PdsClient::from_env() else {
        // PDS not configured, show empty feed
        let template = FollowingFeedTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: current.username,
            feed_items: vec![],
        };
        return Html(template.render().unwrap()).into_response();
    };

    // Get list of followed users
    let follows = match pds_client.list_follows(current_did).await {
        Ok(follows) => follows,
        Err(error) => {
            eprintln!("Error fetching follows: {error}");
            vec![]
        }
    };

    // Collect feed items from all followed users
    let mut feed_items: Vec<FeedItem> = Vec::new();

    for follow in follows {
        let subject_did = &follow.value.subject;

        // Look up user in local database, skip if not found
        let user = match db.get_user_by_did(subject_did).await {
            Ok(Some(user)) => user,
            Ok(None) => continue, // Skip users not in local DB
            Err(error) => {
                eprintln!("Error fetching user by DID: {error}");
                continue;
            }
        };

        // Fetch book entries for this user
        let entries = match pds_client.list_book_entries(subject_did).await {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("Error fetching book entries for {}: {error}", subject_did);
                continue;
            }
        };

        // Add entries as feed items
        for entry in entries {
            feed_items.push(FeedItem {
                entry: entry.value,
                author_username: user.username.clone(),
                author_did: subject_did.clone(),
                book_id: None,
                in_current_user_library: false,
                current_user_status: None,
            });
        }
    }

    // Sort by created_at descending (most recent first)
    feed_items.sort_by(|a, b| b.entry.created_at.cmp(&a.entry.created_at));

    let template = FollowingFeedTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: current.username,
        feed_items,
    };

    Html(template.render().unwrap()).into_response()
}

/// Normalize a book key for comparison (lowercase title + author)
fn normalize_book_key(title: &str, author: Option<&str>) -> String {
    let normalized_title = title.trim().to_lowercase();
    let normalized_author = author.map(|a| a.trim().to_lowercase()).unwrap_or_default();
    format!("{}|{}", normalized_title, normalized_author)
}

/// Show a public global feed with book entries from all users on the PDS.
/// Also saves any new books to the local database for deduplication.
pub async fn global_feed_page(State(db): State<AppState>, headers: HeaderMap) -> Response {
    let current = current_user(&db, &headers).await;
    let is_authenticated = current.is_some();
    let username = current
        .as_ref()
        .map(|u| u.username.clone())
        .unwrap_or_default();
    let current_did = current.as_ref().and_then(|u| u.did.clone());

    let Some(pds_client) = PdsClient::from_env() else {
        let template = GlobalFeedTemplate {
            is_authenticated,
            signups_disabled: signups_disabled(),
            username,
            current_did,
            feed_items: vec![],
        };
        return Html(template.render().unwrap()).into_response();
    };

    // Build lookup of current user's library (book_key -> status)
    let mut user_library: HashMap<String, Option<String>> = HashMap::new();
    if let Some(ref did) = current_did
        && let Ok(entries) = pds_client.list_book_entries(did).await
    {
        for entry in entries {
            let book = &entry.value.book;
            let author = book
                .authors
                .as_ref()
                .and_then(|a| a.first())
                .map(|s| s.as_str());
            let key = normalize_book_key(&book.title, author);
            user_library.insert(key, entry.value.status.clone());
        }
    }

    // Get all repositories from the PDS (not just local users)
    let repos = match pds_client.list_repos(Some(100)).await {
        Ok(repos) => repos,
        Err(error) => {
            eprintln!("Error fetching repos from PDS: {error}");
            vec![]
        }
    };

    let mut feed_items: Vec<FeedItem> = Vec::new();

    for repo in repos {
        let did = &repo.did;

        // Try to look up user in local database for username
        let author_username = match db.get_user_by_did(did).await {
            Ok(Some(user)) => user.username,
            _ => did.clone(), // Use DID as fallback if user not in local DB
        };

        // Fetch book entries for this repo
        let entries = match pds_client.list_book_entries(did).await {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("Error fetching book entries for {did}: {error}");
                continue;
            }
        };

        for entry in entries {
            // Save book to local database (deduplicates by title/author/year)
            let book = &entry.value.book;
            let book_id = match db
                .find_or_create_book(
                    &book.title,
                    book.authors
                        .as_ref()
                        .and_then(|a| a.first())
                        .map(|s| s.as_str()),
                    book.publication_year,
                )
                .await
            {
                Ok(id) => Some(id),
                Err(error) => {
                    eprintln!("Error saving book to database: {error}");
                    None
                }
            };

            // Check if this book is in the current user's library
            let author = book
                .authors
                .as_ref()
                .and_then(|a| a.first())
                .map(|s| s.as_str());
            let book_key = normalize_book_key(&book.title, author);
            let library_entry = user_library.get(&book_key);
            let in_current_user_library = library_entry.is_some();
            let current_user_status = library_entry.and_then(|s| s.clone());

            feed_items.push(FeedItem {
                entry: entry.value,
                author_username: author_username.clone(),
                author_did: did.clone(),
                book_id,
                in_current_user_library,
                current_user_status,
            });
        }
    }

    // Sort by created_at descending (most recent first)
    feed_items.sort_by(|a, b| b.entry.created_at.cmp(&a.entry.created_at));

    let template = GlobalFeedTemplate {
        is_authenticated,
        signups_disabled: signups_disabled(),
        username,
        current_did,
        feed_items,
    };

    Html(template.render().unwrap()).into_response()
}
