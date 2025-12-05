use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};
use std::env;

use crate::AppState;
use crate::atproto::PdsClient;
use crate::database::Database;
use crate::templates::{ChangePasswordTemplate, LoginTemplate, ProfileTemplate, SignupTemplate};

// User-related structures
#[derive(sqlx::FromRow, Serialize, Clone)]
pub struct User {
    pub id: String,
    pub username: String,
    #[serde(skip)] // Never serialize password hash
    pub password_hash: String,
    pub created_at: String,
    /// atproto DID
    pub did: Option<String>,
    /// atproto access JWT
    #[serde(skip)]
    pub access_jwt: Option<String>,
    /// atproto refresh JWT
    #[serde(skip)]
    pub refresh_jwt: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct SignupForm {
    pub username: String,
    pub email: String,
    pub password: String,
    pub confirm_password: String,
    pub invite_code: String,
}

pub async fn login_page(State(db): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if current_user(&db, &headers).await.is_some() {
        return Redirect::to("/profile").into_response();
    }

    render_login(String::new(), None)
}

pub async fn login_submit(State(db): State<AppState>, Form(form): Form<LoginRequest>) -> Response {
    let username = form.username.trim().to_string();
    let password = form.password;

    if username.is_empty() {
        return render_login(String::new(), Some("Username cannot be empty".to_string()));
    }

    if password.is_empty() {
        return render_login(
            username.clone(),
            Some("Password cannot be empty".to_string()),
        );
    }

    match db.verify_user(&username, &password).await {
        Ok(Some(user)) => match db.create_session(&user.id).await {
            Ok(token) => {
                let mut response = Redirect::to("/").into_response();
                if let Some(cookie) = build_session_cookie(&token) {
                    response.headers_mut().insert(header::SET_COOKIE, cookie);
                }
                response
            }
            Err(error) => {
                eprintln!("Session creation error: {error}");
                render_login(
                    username,
                    Some("Could not create session. Please try again.".to_string()),
                )
            }
        },
        Ok(None) => render_login(username, Some("Invalid username or password".to_string())),
        Err(error) => {
            eprintln!("Authentication error: {error}");
            render_login(username, Some("Authentication failed".to_string()))
        }
    }
}

pub async fn signup_page(State(db): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if current_user(&db, &headers).await.is_some() {
        return Redirect::to("/profile").into_response();
    }

    if signups_disabled() {
        return signup_disabled_response();
    }

    render_signup(String::new(), String::new(), String::new(), None)
}

pub async fn signup_submit(State(db): State<AppState>, Form(form): Form<SignupForm>) -> Response {
    if signups_disabled() {
        return signup_disabled_response();
    }

    let username = form.username.trim().to_string();
    let email = form.email.trim().to_string();
    let password = form.password;
    let confirm_password = form.confirm_password;
    let invite_code = form.invite_code.trim().to_string();

    if username.is_empty() {
        return render_signup(
            String::new(),
            String::new(),
            String::new(),
            Some("Username cannot be empty".to_string()),
        );
    }

    if email.is_empty() {
        return render_signup(
            username.clone(),
            String::new(),
            invite_code.clone(),
            Some("Email cannot be empty".to_string()),
        );
    }

    if password.len() < 8 {
        return render_signup(
            username.clone(),
            email.clone(),
            invite_code.clone(),
            Some("Password must be at least 8 characters long".to_string()),
        );
    }

    if password != confirm_password {
        return render_signup(
            username.clone(),
            email.clone(),
            invite_code.clone(),
            Some("Passwords do not match".to_string()),
        );
    }

    // Try to create account on PDS if configured
    let pds_creds = match create_pds_account(&username, &email, &password, &invite_code).await {
        Ok(creds) => creds,
        Err(error) => {
            eprintln!("PDS account creation error: {error}");
            return render_signup(
                username,
                email,
                invite_code,
                Some(format!("Could not create AT Protocol account: {}", error)),
            );
        }
    };

    // Extract credentials for database storage
    let (did, access_jwt, refresh_jwt) = match &pds_creds {
        Some(creds) => (
            Some(creds.did.as_str()),
            Some(creds.access_jwt.as_str()),
            Some(creds.refresh_jwt.as_str()),
        ),
        None => (None, None, None),
    };

    match db
        .create_user_with_atproto(&username, &password, did, access_jwt, refresh_jwt)
        .await
    {
        Ok(user_id) => match db.create_session(&user_id).await {
            Ok(token) => {
                let mut response = Redirect::to("/").into_response();
                if let Some(cookie) = build_session_cookie(&token) {
                    response.headers_mut().insert(header::SET_COOKIE, cookie);
                }
                response
            }
            Err(error) => {
                eprintln!("Session creation error: {error}");
                render_signup(
                    username,
                    email,
                    invite_code,
                    Some("Could not create session. Please try again.".to_string()),
                )
            }
        },
        Err(error) => {
            if error.to_string().contains("already exists") {
                render_signup(
                    username,
                    email,
                    invite_code,
                    Some("Username already exists".to_string()),
                )
            } else {
                eprintln!("User registration error: {error}");
                render_signup(
                    username,
                    email,
                    invite_code,
                    Some("Could not create account. Please try again.".to_string()),
                )
            }
        }
    }
}

/// AT Protocol credentials returned from account creation
pub struct AtprotoCredentials {
    pub did: String,
    pub access_jwt: String,
    pub refresh_jwt: String,
}

/// Create an account on the configured PDS.
/// Returns credentials if successful, or None if PDS is not configured.
async fn create_pds_account(
    username: &str,
    email: &str,
    password: &str,
    invite_code: &str,
) -> Result<Option<AtprotoCredentials>, Box<dyn std::error::Error + Send + Sync>> {
    let Some(pds_client) = PdsClient::from_env() else {
        // PDS not configured, skip AT Protocol account creation
        return Ok(None);
    };

    // Generate handle from username
    let handle = pds_client.make_handle(username)?;

    // Prepare invite code (empty string means no invite code)
    let invite_code = if invite_code.is_empty() {
        None
    } else {
        Some(invite_code)
    };

    // Create account on PDS
    let response = pds_client
        .create_account(&handle, Some(email), Some(password), invite_code)
        .await?;

    println!(
        "Created AT Protocol account: {} (DID: {})",
        response.handle, response.did
    );

    Ok(Some(AtprotoCredentials {
        did: response.did,
        access_jwt: response.access_jwt,
        refresh_jwt: response.refresh_jwt,
    }))
}

pub async fn logout(State(db): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = extract_session_token(&headers)
        && let Err(error) = db.delete_session(&token).await
    {
        eprintln!("Failed to delete session: {error}");
    }

    let mut response = Redirect::to("/").into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, clear_session_cookie());
    response
}

/// Redirect /profile to /profile/{did} for the current user
pub async fn profile_redirect(State(db): State<AppState>, headers: HeaderMap) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    // Redirect to the user's profile by DID
    match user.did {
        Some(did) => Redirect::to(&format!("/profile/{}", did)).into_response(),
        None => {
            // User doesn't have a DID, show error or redirect home
            Redirect::to("/").into_response()
        }
    }
}

/// Show profile page for a specific DID
pub async fn profile_by_did_page(
    State(db): State<AppState>,
    headers: HeaderMap,
    Path(did): Path<String>,
) -> Response {
    let current = current_user(&db, &headers).await;

    // Look up the user by DID
    let profile_user = match db.get_user_by_did(&did).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Profile not found").into_response();
        }
        Err(error) => {
            eprintln!("Error fetching user by DID: {error}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let book_count = db.get_book_count().await.unwrap_or(0);

    // Current user's username for nav (empty if not logged in)
    let current_username = current
        .as_ref()
        .map(|u| u.username.clone())
        .unwrap_or_default();

    let template = ProfileTemplate {
        is_authenticated: current.is_some(),
        signups_disabled: signups_disabled(),
        username: current_username,
        profile_username: profile_user.username,
        profile_did: profile_user.did,
        book_count,
    };

    Html(template.render().unwrap()).into_response()
}

pub async fn change_password_page(State(db): State<AppState>, headers: HeaderMap) -> Response {
    let user = current_user(&db, &headers).await;

    if user.is_none() {
        return Redirect::to("/login").into_response();
    }

    let template = ChangePasswordTemplate {
        is_authenticated: true,
        signups_disabled: signups_disabled(),
        username: user.map(|u| u.username).unwrap_or_default(),
        error_message: None,
        success_message: None,
    };

    Html(template.render().unwrap()).into_response()
}

#[derive(Deserialize)]
pub struct ChangePasswordForm {
    pub new_password: String,
    pub confirm_password: String,
}

pub async fn change_password(
    State(db): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<ChangePasswordForm>,
) -> Response {
    let user = current_user(&db, &headers).await;

    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    if form.new_password != form.confirm_password {
        let template = ChangePasswordTemplate {
            is_authenticated: true,
            signups_disabled: signups_disabled(),
            username: user.username,
            error_message: Some("Passwords do not match".to_string()),
            success_message: None,
        };
        return Html(template.render().unwrap()).into_response();
    }

    // Update password
    match db.update_password(&user.id, &form.new_password).await {
        Ok(_) => {
            let template = ChangePasswordTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                error_message: None,
                success_message: Some("Password changed successfully".to_string()),
            };
            Html(template.render().unwrap()).into_response()
        }
        Err(error) => {
            eprintln!("Password update error: {error}");
            let template = ChangePasswordTemplate {
                is_authenticated: true,
                signups_disabled: signups_disabled(),
                username: user.username,
                error_message: Some("Could not update password. Please try again.".to_string()),
                success_message: None,
            };
            Html(template.render().unwrap()).into_response()
        }
    }
}

fn render_login(form_username: String, error_message: Option<String>) -> Response {
    let template = LoginTemplate {
        is_authenticated: false,
        signups_disabled: signups_disabled(),
        username: String::new(),
        form_username,
        error_message,
    };

    Html(template.render().unwrap()).into_response()
}

fn render_signup(
    form_username: String,
    form_email: String,
    form_invite_code: String,
    error_message: Option<String>,
) -> Response {
    let template = SignupTemplate {
        is_authenticated: false,
        signups_disabled: signups_disabled(),
        username: String::new(),
        form_username,
        form_email,
        form_invite_code,
        error_message,
    };

    Html(template.render().unwrap()).into_response()
}

pub async fn current_user(db: &Database, headers: &HeaderMap) -> Option<User> {
    let token = extract_session_token(headers)?;
    db.validate_session(&token).await.ok()?
}

fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;

    for cookie in cookie_header.split(';') {
        let trimmed = cookie.trim();
        if let Some(rest) = trimmed.strip_prefix("session_token=") {
            return Some(rest.to_string());
        }
    }

    None
}

fn build_session_cookie(token: &str) -> Option<HeaderValue> {
    HeaderValue::from_str(&format!(
        "session_token={token}; HttpOnly; Path=/; SameSite=Lax; Max-Age=604800"
    ))
    .ok()
}

fn clear_session_cookie() -> HeaderValue {
    HeaderValue::from_static("session_token=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax")
}

pub fn signups_disabled() -> bool {
    env::var("DISABLE_SIGNUPS")
        .map(|value| value.trim() == "1")
        .unwrap_or(false)
}

fn signup_disabled_response() -> Response {
    (StatusCode::FORBIDDEN, "signups are disabled.").into_response()
}
