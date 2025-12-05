use serde::{Deserialize, Serialize};

/// AT Protocol PDS client for making XRPC calls.
pub struct PdsClient {
    client: reqwest::Client,
    pds_url: String,
}

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
pub struct XrpcError {
    pub error: String,
    pub message: Option<String>,
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
}
