use serde::{Deserialize, Serialize};

const PLC_DIRECTORY_URL: &str = "https://plc.directory";

/// AT Protocol PDS client for making XRPC calls.
pub struct PdsClient {
    client: reqwest::Client,
    pds_url: String,
}

/// Response from PLC Directory for a DID document
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlcDidDocument {
    pub id: String,
    #[serde(default)]
    pub also_known_as: Vec<String>,
}

/// Resolve a DID to a handle via plc.directory.
/// Works for any did:plc DID regardless of which PDS hosts the account.
pub async fn resolve_handle_from_plc(did: &str) -> Option<String> {
    let client = reqwest::Client::new();
    let url = format!("{}/{}", PLC_DIRECTORY_URL, did);

    println!("[HTTP] GET {}", url);

    let response = match client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => return None,
    };

    if !response.status().is_success() {
        return None;
    }

    let doc: PlcDidDocument = match response.json().await {
        Ok(d) => d,
        Err(_) => return None,
    };

    // Extract handle from alsoKnownAs (format: "at://handle.example.com")
    doc.also_known_as
        .into_iter()
        .find(|s| s.starts_with("at://"))
        .map(|s| s.trim_start_matches("at://").to_string())
}

// ============================================================================
// Account Management Types
// ============================================================================

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAccountRequest {
    pub handle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_code: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CreateAccountResponse {
    pub did: String,
    pub handle: String,
    pub access_jwt: String,
    pub refresh_jwt: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RefreshSessionResponse {
    pub access_jwt: String,
    pub refresh_jwt: String,
    pub handle: String,
    pub did: String,
}

/// Request to create a session via com.atproto.server.createSession
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    /// Handle or email of the account
    pub identifier: String,
    /// Password
    pub password: String,
}

/// Response from com.atproto.server.createSession
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionResponse {
    pub did: String,
    pub handle: String,
    pub access_jwt: String,
    pub refresh_jwt: String,
}

#[derive(Deserialize, Debug)]
pub struct XrpcError {
    pub error: String,
    pub message: Option<String>,
}

// ============================================================================
// Record Management Types
// ============================================================================

/// Request to create a record via com.atproto.repo.createRecord
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecordRequest<T: Serialize> {
    /// The DID of the repo
    pub repo: String,
    /// The NSID of the record collection
    pub collection: String,
    /// The record to create
    pub record: T,
}

/// Response from com.atproto.repo.createRecord
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecordResponse {
    pub uri: String,
    pub cid: String,
}

/// Response from com.atproto.repo.listRecords
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ListRecordsResponse<T> {
    pub records: Vec<RecordEntry<T>>,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// A single record entry from listRecords
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RecordEntry<T> {
    pub uri: String,
    pub cid: String,
    pub value: T,
}

// ============================================================================
// app.alcman.book.entry Lexicon Types
// ============================================================================

/// Book metadata reference (bookRef in the lexicon)
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookRef {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isbn: Option<String>,
}

/// A user's book entry record (app.alcman.book.entry)
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BookEntryRecord {
    /// Record type identifier
    #[serde(rename = "$type")]
    pub record_type: String,
    /// Book metadata
    pub book: BookRef,
    /// User's notes about the book
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// User's reading status
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// When the user started reading the book
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// When the user finished reading the book
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// When this entry was created
    pub created_at: String,
}

impl BookEntryRecord {
    /// Create a new book entry record
    pub fn new(book: BookRef) -> Self {
        Self {
            record_type: "app.alcman.book.entry".to_string(),
            book,
            notes: None,
            status: None,
            started_at: None,
            finished_at: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Format created_at as a human-readable date and time string
    pub fn created_date(&self) -> String {
        chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map(|dt| dt.format("%B %d, %Y at %I:%M %p").to_string())
            .unwrap_or_else(|_| {
                // Fallback: just extract date part if parsing fails
                self.created_at
                    .split('T')
                    .next()
                    .unwrap_or(&self.created_at)
                    .to_string()
            })
    }

    /// Format a date string (for started_at, finished_at) as human-readable
    pub fn format_date(date_str: &str) -> String {
        chrono::DateTime::parse_from_rfc3339(date_str)
            .map(|dt| dt.format("%B %d, %Y").to_string())
            .unwrap_or_else(|_| {
                // Fallback: just extract date part if parsing fails
                date_str.split('T').next().unwrap_or(date_str).to_string()
            })
    }
}

// ============================================================================
// app.alcman.graph.follow Lexicon Types
// ============================================================================

/// A follow relationship record (app.alcman.graph.follow)
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FollowRecord {
    /// Record type identifier
    #[serde(rename = "$type")]
    pub record_type: String,
    /// DID of the user being followed
    pub subject: String,
    /// When the follow was created
    pub created_at: String,
}

impl FollowRecord {
    /// Create a new follow record
    pub fn new(subject: String) -> Self {
        Self {
            record_type: "app.alcman.graph.follow".to_string(),
            subject,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

// ============================================================================
// Feed Types
// ============================================================================

/// A feed item combining a book entry with author information
#[derive(Debug)]
pub struct FeedItem {
    /// The book entry record
    pub entry: BookEntryRecord,
    /// Username of the author
    pub author_username: String,
    /// DID of the author
    pub author_did: String,
    /// Local database book ID (if available)
    pub book_id: Option<String>,
}

impl FeedItem {
    /// Format started_at date if available
    pub fn formatted_started_at(&self) -> Option<String> {
        self.entry
            .started_at
            .as_ref()
            .map(|s| BookEntryRecord::format_date(s))
    }

    /// Format finished_at date if available
    pub fn formatted_finished_at(&self) -> Option<String> {
        self.entry
            .finished_at
            .as_ref()
            .map(|s| BookEntryRecord::format_date(s))
    }
}

/// Request to delete a record via com.atproto.repo.deleteRecord
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRecordRequest {
    /// The DID of the repo
    pub repo: String,
    /// The NSID of the record collection
    pub collection: String,
    /// The record key (rkey)
    pub rkey: String,
}

/// Response from com.atproto.sync.listRepos
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ListReposResponse {
    pub repos: Vec<RepoInfo>,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Repository info from listRepos
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub did: String,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub rev: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
}

/// Response from com.atproto.repo.describeRepo
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DescribeRepoResponse {
    pub handle: String,
    pub did: String,
    #[serde(default)]
    pub did_doc: Option<serde_json::Value>,
    #[serde(default)]
    pub collections: Option<Vec<String>>,
    #[serde(default)]
    pub handle_is_correct: Option<bool>,
}

pub type AtprotoResult<T> = Result<T, AtprotoError>;

#[derive(Debug)]
pub enum AtprotoError {
    /// HTTP request failed
    Request(reqwest::Error),
    /// XRPC error returned by the PDS
    Xrpc {
        error: String,
        message: Option<String>,
    },
    /// Invalid handle format
    InvalidHandle(String),
    /// PDS URL not configured
    NotConfigured,
}

impl std::fmt::Display for AtprotoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AtprotoError::Request(e) => write!(f, "HTTP request failed: {}", e),
            AtprotoError::Xrpc { error, message } => {
                write!(f, "XRPC error: {}", error)?;
                if let Some(msg) = message {
                    write!(f, " - {}", msg)?;
                }
                Ok(())
            }
            AtprotoError::InvalidHandle(h) => write!(f, "Invalid handle format: {}", h),
            AtprotoError::NotConfigured => write!(f, "PDS URL not configured"),
        }
    }
}

impl std::error::Error for AtprotoError {}

impl AtprotoError {
    /// Check if this error indicates an expired token
    pub fn is_expired_token(&self) -> bool {
        matches!(self, AtprotoError::Xrpc { error, .. } if error == "ExpiredToken")
    }
}

impl From<reqwest::Error> for AtprotoError {
    fn from(e: reqwest::Error) -> Self {
        AtprotoError::Request(e)
    }
}

impl PdsClient {
    pub fn new(pds_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            pds_url,
        }
    }

    pub fn from_env() -> Option<Self> {
        std::env::var("PDS_URL").ok().map(Self::new)
    }

    pub fn pds_url(&self) -> &str {
        &self.pds_url
    }

    pub async fn create_account(
        &self,
        handle: &str,
        email: Option<&str>,
        password: Option<&str>,
        invite_code: Option<&str>,
    ) -> AtprotoResult<CreateAccountResponse> {
        let url = format!("{}/xrpc/com.atproto.server.createAccount", self.pds_url);

        println!("[HTTP] POST {} (handle: {})", url, handle);

        let request = CreateAccountRequest {
            handle: handle.to_string(),
            email: email.map(String::from),
            password: password.map(String::from),
            invite_code: invite_code.map(String::from),
        };

        let response = self.client.post(&url).json(&request).send().await?;

        if response.status().is_success() {
            let account: CreateAccountResponse = response.json().await?;
            Ok(account)
        } else {
            let error: XrpcError = response.json().await.unwrap_or(XrpcError {
                error: "UnknownError".to_string(),
                message: Some("Failed to parse error response".to_string()),
            });
            Err(AtprotoError::Xrpc {
                error: error.error,
                message: error.message,
            })
        }
    }

    /// Create a session (login) on the PDS.
    ///
    /// Calls com.atproto.server.createSession
    pub async fn create_session(
        &self,
        identifier: &str,
        password: &str,
    ) -> AtprotoResult<CreateSessionResponse> {
        let url = format!("{}/xrpc/com.atproto.server.createSession", self.pds_url);

        println!("[HTTP] POST {} (identifier: {})", url, identifier);

        let request = CreateSessionRequest {
            identifier: identifier.to_string(),
            password: password.to_string(),
        };

        let response = self.client.post(&url).json(&request).send().await?;

        if response.status().is_success() {
            let session: CreateSessionResponse = response.json().await?;
            Ok(session)
        } else {
            let error: XrpcError = response.json().await.unwrap_or(XrpcError {
                error: "UnknownError".to_string(),
                message: Some("Failed to parse error response".to_string()),
            });
            Err(AtprotoError::Xrpc {
                error: error.error,
                message: error.message,
            })
        }
    }

    pub fn make_handle(&self, username: &str) -> Result<String, AtprotoError> {
        // Extract domain from PDS URL
        let domain = self
            .pds_url
            .strip_prefix("https://")
            .or_else(|| self.pds_url.strip_prefix("http://"))
            .unwrap_or(&self.pds_url);

        // Remove any trailing path or port
        let domain = domain.split('/').next().unwrap_or(domain);
        let domain = domain.split(':').next().unwrap_or(domain);

        // Validate username (basic validation)
        if username.is_empty() || username.contains('.') || username.contains('@') {
            return Err(AtprotoError::InvalidHandle(username.to_string()));
        }

        Ok(format!("{}.{}", username.to_lowercase(), domain))
    }

    /// Refresh an access token using a refresh token.
    ///
    /// Calls com.atproto.server.refreshSession
    pub async fn refresh_session(
        &self,
        refresh_jwt: &str,
    ) -> AtprotoResult<RefreshSessionResponse> {
        let url = format!("{}/xrpc/com.atproto.server.refreshSession", self.pds_url);

        println!("[HTTP] POST {}", url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", refresh_jwt))
            .send()
            .await?;

        if response.status().is_success() {
            let session: RefreshSessionResponse = response.json().await?;
            Ok(session)
        } else {
            let error: XrpcError = response.json().await.unwrap_or(XrpcError {
                error: "UnknownError".to_string(),
                message: Some("Failed to parse error response".to_string()),
            });
            Err(AtprotoError::Xrpc {
                error: error.error,
                message: error.message,
            })
        }
    }

    /// Create a record in a user's repository.
    ///
    /// Calls com.atproto.repo.createRecord
    pub async fn create_record<T: Serialize>(
        &self,
        access_jwt: &str,
        repo: &str,
        collection: &str,
        record: T,
    ) -> AtprotoResult<CreateRecordResponse> {
        let url = format!("{}/xrpc/com.atproto.repo.createRecord", self.pds_url);

        println!("[HTTP] POST {} (collection: {})", url, collection);

        let request = CreateRecordRequest {
            repo: repo.to_string(),
            collection: collection.to_string(),
            record,
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", access_jwt))
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            let result: CreateRecordResponse = response.json().await?;
            Ok(result)
        } else {
            let error: XrpcError = response.json().await.unwrap_or(XrpcError {
                error: "UnknownError".to_string(),
                message: Some("Failed to parse error response".to_string()),
            });
            Err(AtprotoError::Xrpc {
                error: error.error,
                message: error.message,
            })
        }
    }

    /// Create a record in a user's repository from raw JSON.
    ///
    /// Calls com.atproto.repo.createRecord
    pub async fn create_record_raw(
        &self,
        access_jwt: &str,
        repo: &str,
        collection: &str,
        record: serde_json::Value,
    ) -> AtprotoResult<CreateRecordResponse> {
        self.create_record(access_jwt, repo, collection, record)
            .await
    }

    /// Create a book entry record in a user's repository.
    pub async fn create_book_entry(
        &self,
        access_jwt: &str,
        did: &str,
        book: BookRef,
    ) -> AtprotoResult<CreateRecordResponse> {
        let record = BookEntryRecord::new(book);
        self.create_record(access_jwt, did, "app.alcman.book.entry", record)
            .await
    }

    /// List records from a user's repository.
    ///
    /// Calls com.atproto.repo.listRecords (public, no auth required)
    pub async fn list_records<T: serde::de::DeserializeOwned>(
        &self,
        repo: &str,
        collection: &str,
        limit: Option<u32>,
    ) -> AtprotoResult<ListRecordsResponse<T>> {
        let mut url = format!(
            "{}/xrpc/com.atproto.repo.listRecords?repo={}&collection={}",
            self.pds_url, repo, collection
        );

        if let Some(limit) = limit {
            url.push_str(&format!("&limit={}", limit));
        }

        println!("[HTTP] GET {}", url);

        let response = self.client.get(&url).send().await?;

        if response.status().is_success() {
            let result: ListRecordsResponse<T> = response.json().await?;
            Ok(result)
        } else {
            let error: XrpcError = response.json().await.unwrap_or(XrpcError {
                error: "UnknownError".to_string(),
                message: Some("Failed to parse error response".to_string()),
            });
            Err(AtprotoError::Xrpc {
                error: error.error,
                message: error.message,
            })
        }
    }

    /// List records from a user's repository as raw JSON.
    ///
    /// Calls com.atproto.repo.listRecords (public, no auth required)
    pub async fn list_records_raw(
        &self,
        repo: &str,
        collection: &str,
        limit: Option<u32>,
    ) -> AtprotoResult<serde_json::Value> {
        let mut url = format!(
            "{}/xrpc/com.atproto.repo.listRecords?repo={}&collection={}",
            self.pds_url, repo, collection
        );

        if let Some(limit) = limit {
            url.push_str(&format!("&limit={}", limit));
        }

        println!("[HTTP] GET {}", url);

        let response = self.client.get(&url).send().await?;

        if response.status().is_success() {
            let result: serde_json::Value = response.json().await?;
            Ok(result)
        } else {
            let error: XrpcError = response.json().await.unwrap_or(XrpcError {
                error: "UnknownError".to_string(),
                message: Some("Failed to parse error response".to_string()),
            });
            Err(AtprotoError::Xrpc {
                error: error.error,
                message: error.message,
            })
        }
    }

    /// List book entries from a user's repository.
    pub async fn list_book_entries(
        &self,
        did: &str,
    ) -> AtprotoResult<Vec<RecordEntry<BookEntryRecord>>> {
        let response: ListRecordsResponse<BookEntryRecord> = self
            .list_records(did, "app.alcman.book.entry", Some(100))
            .await?;
        Ok(response.records)
    }

    /// List all repositories on the PDS.
    ///
    /// Calls com.atproto.sync.listRepos (public, no auth required)
    pub async fn list_repos(&self, limit: Option<u32>) -> AtprotoResult<Vec<RepoInfo>> {
        let mut url = format!("{}/xrpc/com.atproto.sync.listRepos", self.pds_url);

        if let Some(limit) = limit {
            url.push_str(&format!("?limit={}", limit));
        }

        println!("[HTTP] GET {}", url);

        let response = self.client.get(&url).send().await?;

        if response.status().is_success() {
            let result: ListReposResponse = response.json().await?;
            Ok(result.repos)
        } else {
            let error: XrpcError = response.json().await.unwrap_or(XrpcError {
                error: "UnknownError".to_string(),
                message: Some("Failed to parse error response".to_string()),
            });
            Err(AtprotoError::Xrpc {
                error: error.error,
                message: error.message,
            })
        }
    }

    /// Describe a repository (get handle and other info).
    ///
    /// Calls com.atproto.repo.describeRepo (public, no auth required)
    pub async fn describe_repo(&self, repo: &str) -> AtprotoResult<DescribeRepoResponse> {
        let url = format!(
            "{}/xrpc/com.atproto.repo.describeRepo?repo={}",
            self.pds_url, repo
        );

        println!("[HTTP] GET {}", url);

        let response = self.client.get(&url).send().await?;

        if response.status().is_success() {
            let result: DescribeRepoResponse = response.json().await?;
            Ok(result)
        } else {
            let error: XrpcError = response.json().await.unwrap_or(XrpcError {
                error: "UnknownError".to_string(),
                message: Some("Failed to parse error response".to_string()),
            });
            Err(AtprotoError::Xrpc {
                error: error.error,
                message: error.message,
            })
        }
    }

    /// Delete a record from a user's repository.
    ///
    /// Calls com.atproto.repo.deleteRecord
    pub async fn delete_record(
        &self,
        access_jwt: &str,
        repo: &str,
        collection: &str,
        rkey: &str,
    ) -> AtprotoResult<()> {
        let url = format!("{}/xrpc/com.atproto.repo.deleteRecord", self.pds_url);

        println!(
            "[HTTP] POST {} (collection: {}, rkey: {})",
            url, collection, rkey
        );

        let request = DeleteRecordRequest {
            repo: repo.to_string(),
            collection: collection.to_string(),
            rkey: rkey.to_string(),
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", access_jwt))
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            let error: XrpcError = response.json().await.unwrap_or(XrpcError {
                error: "UnknownError".to_string(),
                message: Some("Failed to parse error response".to_string()),
            });
            Err(AtprotoError::Xrpc {
                error: error.error,
                message: error.message,
            })
        }
    }

    /// Create a follow record in a user's repository.
    pub async fn create_follow(
        &self,
        access_jwt: &str,
        follower_did: &str,
        subject_did: &str,
    ) -> AtprotoResult<CreateRecordResponse> {
        let record = FollowRecord::new(subject_did.to_string());
        self.create_record(access_jwt, follower_did, "app.alcman.graph.follow", record)
            .await
    }

    /// List follow records from a user's repository.
    pub async fn list_follows(&self, did: &str) -> AtprotoResult<Vec<RecordEntry<FollowRecord>>> {
        let response: ListRecordsResponse<FollowRecord> = self
            .list_records(did, "app.alcman.graph.follow", Some(100))
            .await?;
        Ok(response.records)
    }

    /// Check if a user is following another user and return the record key if so.
    pub async fn get_follow_rkey(
        &self,
        follower_did: &str,
        subject_did: &str,
    ) -> AtprotoResult<Option<String>> {
        let follows = self.list_follows(follower_did).await?;
        for follow in follows {
            if follow.value.subject == subject_did {
                // Extract rkey from URI: at://did/collection/rkey
                if let Some(rkey) = follow.uri.split('/').next_back() {
                    return Ok(Some(rkey.to_string()));
                }
            }
        }
        Ok(None)
    }

    /// Check if a user is following another user.
    pub async fn is_following(&self, follower_did: &str, subject_did: &str) -> AtprotoResult<bool> {
        let rkey = self.get_follow_rkey(follower_did, subject_did).await?;
        Ok(rkey.is_some())
    }

    /// Delete a follow record (unfollow).
    pub async fn delete_follow(
        &self,
        access_jwt: &str,
        follower_did: &str,
        subject_did: &str,
    ) -> AtprotoResult<()> {
        // Find the rkey for this follow
        let rkey = self.get_follow_rkey(follower_did, subject_did).await?;

        if let Some(rkey) = rkey {
            self.delete_record(access_jwt, follower_did, "app.alcman.graph.follow", &rkey)
                .await
        } else {
            // Not following, nothing to delete
            Ok(())
        }
    }
}
