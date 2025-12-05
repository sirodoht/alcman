use serde::{Deserialize, Serialize};

/// AT Protocol PDS client for making XRPC calls.
pub struct PdsClient {
    client: reqwest::Client,
    pds_url: String,
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
    /// Reading status
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// When the user started reading
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// When the user finished reading
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// When this entry was created
    pub created_at: String,
}

impl BookEntryRecord {
    /// Create a new book entry record
    pub fn new(book: BookRef, status: Option<String>, notes: Option<String>) -> Self {
        Self {
            record_type: "app.alcman.book.entry".to_string(),
            book,
            notes,
            status,
            started_at: None,
            finished_at: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
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

    /// Create a book entry record in a user's repository.
    pub async fn create_book_entry(
        &self,
        access_jwt: &str,
        did: &str,
        book: BookRef,
        status: Option<String>,
        notes: Option<String>,
    ) -> AtprotoResult<CreateRecordResponse> {
        let record = BookEntryRecord::new(book, status, notes);
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
}
