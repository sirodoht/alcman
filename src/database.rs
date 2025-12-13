use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use sqlx::{Pool, Row, Sqlite, SqlitePool, migrate::MigrateDatabase};
use std::{fs, path::Path};

pub struct Database {
    pub pool: Pool<Sqlite>,
}

type DynError = Box<dyn std::error::Error + Send + Sync>;

/// Input payload for creating a book.
pub struct NewBook<'a> {
    pub title: &'a str,
    pub author: Option<&'a str>,
    pub publication_year: Option<i32>,
}

/// Input payload for updating a book.
pub struct BookUpdate<'a> {
    pub title: &'a str,
    pub author: Option<&'a str>,
    pub publication_year: Option<i32>,
}

impl Database {
    pub async fn new(database_url: &str) -> Result<Self, sqlx::Error> {
        // Create database if it doesn't exist
        if !Sqlite::database_exists(database_url).await.unwrap_or(false) {
            println!("Creating database {}", database_url);
            match Sqlite::create_database(database_url).await {
                Ok(_) => println!("Successfully created database"),
                Err(error) => panic!("Error creating database: {}", error),
            }
        } else {
            println!("Database already exists");
        }

        // Connect to database
        let pool = SqlitePool::connect(database_url).await?;

        // Disable WAL mode to avoid -shm and -wal files
        sqlx::query("PRAGMA journal_mode = DELETE")
            .execute(&pool)
            .await?;

        Ok(Database { pool })
    }

    pub async fn run_migrations(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("Running database migrations...");

        // Create migrations table if it doesn't exist
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS _migrations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                filename TEXT NOT NULL UNIQUE,
                executed_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Get all migration files
        let migrations_dir = Path::new("migrations");
        if !migrations_dir.exists() {
            println!("Migrations directory not found");
            return Ok(());
        }

        let mut entries: Vec<_> = fs::read_dir(migrations_dir)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext == "sql")
                    .unwrap_or(false)
            })
            .collect();

        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let filename = entry.file_name().to_string_lossy().to_string();

            // Check if migration has already been executed
            let executed = sqlx::query("SELECT filename FROM _migrations WHERE filename = ?")
                .bind(&filename)
                .fetch_optional(&self.pool)
                .await?
                .is_some();

            if executed {
                continue;
            }

            // Read and execute migration file
            let migration_sql = fs::read_to_string(entry.path())?;

            // Execute the migration in a transaction
            let mut tx = self.pool.begin().await?;

            // Split by semicolons and execute each statement
            for statement in migration_sql.split(';') {
                let statement = statement.trim();
                if !statement.is_empty() {
                    sqlx::query(statement).execute(&mut *tx).await?;
                }
            }

            // Record the migration as executed
            sqlx::query(
                "INSERT INTO _migrations (filename, executed_at) VALUES (?, datetime('now'))",
            )
            .bind(&filename)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;

            println!("Successfully executed migration: {}", filename);
        }

        println!("All migrations completed");
        Ok(())
    }

    // User-related database methods
    pub async fn get_all_users(&self) -> Result<Vec<crate::auth::User>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, username, password_hash, created_at, did, access_jwt, refresh_jwt FROM users ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        let users = rows
            .into_iter()
            .map(|row| crate::auth::User {
                id: row.get("id"),
                username: row.get("username"),
                password_hash: row.get("password_hash"),
                created_at: row.get("created_at"),
                did: row.get("did"),
                access_jwt: row.get("access_jwt"),
                refresh_jwt: row.get("refresh_jwt"),
            })
            .collect();

        Ok(users)
    }

    pub async fn create_user(&self, username: &str, password: &str) -> Result<String, DynError> {
        self.create_user_with_atproto(username, password, None, None, None)
            .await
    }

    /// Create a new user with optional AT Protocol credentials.
    pub async fn create_user_with_atproto(
        &self,
        username: &str,
        password: &str,
        did: Option<&str>,
        access_jwt: Option<&str>,
        refresh_jwt: Option<&str>,
    ) -> Result<String, DynError> {
        // Check if username already exists
        let existing_user = sqlx::query("SELECT id FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&self.pool)
            .await?;

        if existing_user.is_some() {
            return Err("Username already exists".into());
        }

        // Hash the password
        let password_hash = self.hash_password(password)?;

        // Generate new user ID
        let user_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        // Insert user into database
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at, updated_at, did, access_jwt, refresh_jwt) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&user_id)
        .bind(username)
        .bind(&password_hash)
        .bind(&now)
        .bind(&now)
        .bind(did)
        .bind(access_jwt)
        .bind(refresh_jwt)
        .execute(&self.pool)
        .await?;

        Ok(user_id)
    }

    pub async fn verify_user(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<crate::auth::User>, DynError> {
        let user_row = sqlx::query(
            "SELECT id, username, password_hash, created_at, did, access_jwt, refresh_jwt FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = user_row {
            let stored_hash: String = row.get("password_hash");

            if self.verify_password(password, &stored_hash)? {
                let user = crate::auth::User {
                    id: row.get("id"),
                    username: row.get("username"),
                    password_hash: stored_hash,
                    created_at: row.get("created_at"),
                    did: row.get("did"),
                    access_jwt: row.get("access_jwt"),
                    refresh_jwt: row.get("refresh_jwt"),
                };
                Ok(Some(user))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    fn hash_password(&self, password: &str) -> Result<String, DynError> {
        let argon2 = Argon2::default();
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| format!("Password hashing failed: {}", e))?
            .to_string();
        Ok(password_hash)
    }

    fn verify_password(&self, password: &str, hash: &str) -> Result<bool, DynError> {
        let argon2 = Argon2::default();
        let parsed_hash =
            PasswordHash::new(hash).map_err(|e| format!("Invalid password hash: {}", e))?;

        Ok(argon2
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }

    pub async fn update_password(&self, user_id: &str, new_password: &str) -> Result<(), DynError> {
        let password_hash = self.hash_password(new_password)?;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
            .bind(&password_hash)
            .bind(&now)
            .bind(user_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn update_username(&self, user_id: &str, new_username: &str) -> Result<(), DynError> {
        // Check if new username already exists for a different user
        let existing = sqlx::query("SELECT id FROM users WHERE username = ? AND id != ?")
            .bind(new_username)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;

        if existing.is_some() {
            return Err("Username already exists".into());
        }

        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("UPDATE users SET username = ?, updated_at = ? WHERE id = ?")
            .bind(new_username)
            .bind(&now)
            .bind(user_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get a user by their AT Protocol DID
    pub async fn get_user_by_did(&self, did: &str) -> Result<Option<crate::auth::User>, DynError> {
        let user_row = sqlx::query(
            "SELECT id, username, password_hash, created_at, did, access_jwt, refresh_jwt FROM users WHERE did = ?",
        )
        .bind(did)
        .fetch_optional(&self.pool)
        .await?;

        Ok(user_row.map(|row| crate::auth::User {
            id: row.get("id"),
            username: row.get("username"),
            password_hash: row.get("password_hash"),
            created_at: row.get("created_at"),
            did: row.get("did"),
            access_jwt: row.get("access_jwt"),
            refresh_jwt: row.get("refresh_jwt"),
        }))
    }

    // Session management methods
    pub async fn create_session(&self, user_id: &str) -> Result<String, DynError> {
        // Generate a simple session token (UUID)
        let token = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let session_id = uuid::Uuid::new_v4().to_string();

        // Insert session into database (no expiration)
        sqlx::query("INSERT INTO sessions (id, user_id, token, created_at) VALUES (?, ?, ?, ?)")
            .bind(&session_id)
            .bind(user_id)
            .bind(&token)
            .bind(&now)
            .execute(&self.pool)
            .await?;

        Ok(token)
    }

    pub async fn validate_session(
        &self,
        token: &str,
    ) -> Result<Option<crate::auth::User>, DynError> {
        let session_row = sqlx::query(
            "SELECT s.user_id, u.username, u.password_hash, u.created_at, u.did, u.access_jwt, u.refresh_jwt
             FROM sessions s
             JOIN users u ON s.user_id = u.id
             WHERE s.token = ?",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = session_row {
            let user = crate::auth::User {
                id: row.get("user_id"),
                username: row.get("username"),
                password_hash: row.get("password_hash"),
                created_at: row.get("created_at"),
                did: row.get("did"),
                access_jwt: row.get("access_jwt"),
                refresh_jwt: row.get("refresh_jwt"),
            };
            Ok(Some(user))
        } else {
            Ok(None)
        }
    }

    /// Update user's AT Protocol tokens
    pub async fn update_user_tokens(
        &self,
        user_id: &str,
        access_jwt: &str,
        refresh_jwt: &str,
    ) -> Result<(), DynError> {
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "UPDATE users SET access_jwt = ?, refresh_jwt = ?, updated_at = ? WHERE id = ?",
        )
        .bind(access_jwt)
        .bind(refresh_jwt)
        .bind(&now)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn delete_session(&self, token: &str) -> Result<(), DynError> {
        sqlx::query("DELETE FROM sessions WHERE token = ?")
            .bind(token)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    // Book-related database methods
    pub async fn create_book(&self, new_book: NewBook<'_>) -> Result<String, DynError> {
        let NewBook {
            title,
            author,
            publication_year,
        } = new_book;
        let book_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO books (id, title, author, publication_year, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&book_id)
        .bind(title)
        .bind(author)
        .bind(publication_year)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(book_id)
    }

    pub async fn get_all_books(&self) -> Result<Vec<crate::books::Book>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, title, author, publication_year, created_at FROM books ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        let books = rows
            .into_iter()
            .map(|row| crate::books::Book {
                id: row.get("id"),
                title: row.get("title"),
                author: row.get("author"),
                publication_year: row.get("publication_year"),
                created_at: row.get("created_at"),
            })
            .collect();

        Ok(books)
    }

    pub async fn get_book_by_id(
        &self,
        book_id: &str,
    ) -> Result<Option<crate::books::Book>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, title, author, publication_year, created_at FROM books WHERE id = ?",
        )
        .bind(book_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| crate::books::Book {
            id: row.get("id"),
            title: row.get("title"),
            author: row.get("author"),
            publication_year: row.get("publication_year"),
            created_at: row.get("created_at"),
        }))
    }

    pub async fn get_book_count(&self) -> Result<i64, sqlx::Error> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM books")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get("count"))
    }

    pub async fn delete_book(&self, book_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM books WHERE id = ?")
            .bind(book_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_book(
        &self,
        book_id: &str,
        update: BookUpdate<'_>,
    ) -> Result<(), sqlx::Error> {
        let BookUpdate {
            title,
            author,
            publication_year,
        } = update;
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE books SET title = ?, author = ?, publication_year = ?, updated_at = ? WHERE id = ?",
        )
        .bind(title)
        .bind(author)
        .bind(publication_year)
        .bind(&now)
        .bind(book_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Find a book by title and author, or create it if it doesn't exist.
    /// If found and the existing record has no publication year but we have one, update it.
    /// Returns the book ID.
    pub async fn find_or_create_book(
        &self,
        title: &str,
        author: Option<&str>,
        publication_year: Option<i32>,
    ) -> Result<String, DynError> {
        // Try to find existing book with matching title and author
        let existing =
            sqlx::query("SELECT id, publication_year FROM books WHERE title = ? AND author IS ?")
                .bind(title)
                .bind(author)
                .fetch_optional(&self.pool)
                .await?;

        if let Some(row) = existing {
            let book_id: String = row.get("id");
            let existing_year: Option<i32> = row.get("publication_year");

            // If we have a publication year and the existing one is NULL, update it
            if publication_year.is_some() && existing_year.is_none() {
                let now = chrono::Utc::now().to_rfc3339();
                sqlx::query("UPDATE books SET publication_year = ?, updated_at = ? WHERE id = ?")
                    .bind(publication_year)
                    .bind(&now)
                    .bind(&book_id)
                    .execute(&self.pool)
                    .await?;
            }

            return Ok(book_id);
        }

        // Create new book
        let book_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO books (id, title, author, publication_year, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&book_id)
        .bind(title)
        .bind(author)
        .bind(publication_year)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(book_id)
    }
}
