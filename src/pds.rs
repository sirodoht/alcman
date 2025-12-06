use crate::atproto::{AtprotoResult, BookRef, CreateRecordResponse, PdsClient};
use crate::auth::User;
use crate::database::Database;

/// Authenticated PDS client that automatically handles token refresh.
///
/// Use this for any authenticated PDS operations. It will automatically
/// refresh expired tokens and update them in the database.
pub struct AuthenticatedPds<'a> {
    pub client: &'a PdsClient,
    db: &'a Database,
    user_id: String,
    did: String,
    access_jwt: String,
    refresh_jwt: String,
}

impl<'a> AuthenticatedPds<'a> {
    /// Create a new authenticated PDS client for a user.
    /// Returns None if the user doesn't have the required AT Protocol credentials.
    pub fn new(client: &'a PdsClient, db: &'a Database, user: &User) -> Option<Self> {
        Some(Self {
            client,
            db,
            user_id: user.id.clone(),
            did: user.did.clone()?,
            access_jwt: user.access_jwt.clone()?,
            refresh_jwt: user.refresh_jwt.clone()?,
        })
    }

    /// Get the user's DID
    pub fn did(&self) -> &str {
        &self.did
    }

    /// Refresh the access token using the refresh token.
    /// Updates the database with new tokens.
    async fn refresh_token(&mut self) -> bool {
        match self.client.refresh_session(&self.refresh_jwt).await {
            Ok(response) => {
                // Update tokens in database
                if let Err(error) = self
                    .db
                    .update_user_tokens(&self.user_id, &response.access_jwt, &response.refresh_jwt)
                    .await
                {
                    eprintln!("Failed to update tokens in database: {error}");
                    return false;
                }
                println!("Refreshed AT Protocol tokens for user {}", self.user_id);
                self.access_jwt = response.access_jwt;
                self.refresh_jwt = response.refresh_jwt;
                true
            }
            Err(error) => {
                eprintln!("Failed to refresh tokens: {error}");
                false
            }
        }
    }

    /// Create a follow record
    pub async fn create_follow(
        &mut self,
        subject_did: &str,
    ) -> AtprotoResult<CreateRecordResponse> {
        let result = self
            .client
            .create_follow(&self.access_jwt, &self.did, subject_did)
            .await;

        if let Err(ref error) = result
            && error.is_expired_token()
            && self.refresh_token().await
        {
            return self
                .client
                .create_follow(&self.access_jwt, &self.did, subject_did)
                .await;
        }

        result
    }

    /// Delete a follow record (unfollow)
    pub async fn delete_follow(&mut self, subject_did: &str) -> AtprotoResult<()> {
        let result = self
            .client
            .delete_follow(&self.access_jwt, &self.did, subject_did)
            .await;

        if let Err(ref error) = result
            && error.is_expired_token()
            && self.refresh_token().await
        {
            return self
                .client
                .delete_follow(&self.access_jwt, &self.did, subject_did)
                .await;
        }

        result
    }

    /// Create a book entry
    pub async fn create_book_entry(
        &mut self,
        book: BookRef,
        status: Option<String>,
        notes: Option<String>,
    ) -> AtprotoResult<CreateRecordResponse> {
        let result = self
            .client
            .create_book_entry(
                &self.access_jwt,
                &self.did,
                book.clone(),
                status.clone(),
                notes.clone(),
            )
            .await;

        if let Err(ref error) = result
            && error.is_expired_token()
            && self.refresh_token().await
        {
            return self
                .client
                .create_book_entry(&self.access_jwt, &self.did, book, status, notes)
                .await;
        }

        result
    }
}
