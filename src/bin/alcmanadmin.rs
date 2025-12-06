use alcman::Database;
use alcman::atproto::PdsClient;
use alcman::auth::{AtprotoCredentials, create_pds_account};
use clap::{Parser, Subcommand};
use std::env;

#[derive(Parser)]
#[command(name = "alcmandmin")]
#[command(about = "Admin CLI for alcman", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Database file path (default: ./alcman.db)
    #[arg(long, default_value = "./alcman.db")]
    database: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new user
    CreateUser {
        /// Username for the new user
        #[arg(long)]
        username: String,

        /// Email for the new user
        #[arg(long)]
        email: String,

        /// Password for the new user
        #[arg(long)]
        password: String,

        /// Invite code for PDS account creation (optional)
        #[arg(long, default_value = "")]
        invite_code: String,
    },

    /// Create a PDS session (login) and return tokens
    PdsCreateSession {
        /// Handle or email of the account
        #[arg(long)]
        identifier: String,

        /// Password
        #[arg(long)]
        password: String,
    },

    /// List records from a user's repository
    ListRecords {
        /// DID or handle of the repository
        #[arg(long)]
        repo: String,

        /// Collection NSID (e.g., app.alcman.book.entry, app.alcman.graph.follow)
        #[arg(long)]
        collection: String,

        /// Maximum number of records to return (default: 50)
        #[arg(long, default_value = "50")]
        limit: u32,
    },

    /// Create a record in a user's repository (requires authentication)
    CreateRecord {
        /// Access JWT (get from pds-create-session)
        #[arg(long)]
        access_jwt: String,

        /// DID of the repository (your DID)
        #[arg(long)]
        repo: String,

        /// Collection NSID (e.g., app.alcman.book.entry, app.alcman.graph.follow)
        #[arg(long)]
        collection: String,

        /// Record data as JSON string
        #[arg(long)]
        record: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Initialize database
    let database_url = format!("sqlite:{}", cli.database);
    let db = match Database::new(&database_url).await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Failed to connect to database: {}", e);
            std::process::exit(1);
        }
    };

    // Run migrations
    if let Err(e) = db.run_migrations().await {
        eprintln!("Failed to run migrations: {}", e);
        std::process::exit(1);
    }

    match cli.command {
        Commands::CreateUser {
            username,
            email,
            password,
            invite_code,
        } => {
            create_user(&db, &username, &email, &password, &invite_code).await;
        }
        Commands::PdsCreateSession {
            identifier,
            password,
        } => {
            pds_create_session(&identifier, &password).await;
        }
        Commands::ListRecords {
            repo,
            collection,
            limit,
        } => {
            list_records(&repo, &collection, limit).await;
        }
        Commands::CreateRecord {
            access_jwt,
            repo,
            collection,
            record,
        } => {
            create_record(&access_jwt, &repo, &collection, &record).await;
        }
    }
}

async fn create_user(
    db: &Database,
    username: &str,
    email: &str,
    password: &str,
    invite_code: &str,
) {
    println!("Creating user: {}", username);

    // Check PDS_URL is configured
    if env::var("PDS_URL").is_err() {
        eprintln!("Warning: PDS_URL not set. User will be created in database only.");
    }

    // Try to create account on PDS
    let pds_creds: Option<AtprotoCredentials> =
        match create_pds_account(username, email, password, invite_code).await {
            Ok(creds) => creds,
            Err(error) => {
                eprintln!("Failed to create PDS account: {}", error);
                std::process::exit(1);
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

    // Create user in database
    match db
        .create_user_with_atproto(username, password, did, access_jwt, refresh_jwt)
        .await
    {
        Ok(user_id) => {
            println!("Successfully created user!");
            println!("  User ID: {}", user_id);
            println!("  Username: {}", username);
            if let Some(creds) = &pds_creds {
                println!("  DID: {}", creds.did);
            } else {
                println!("  DID: (none - PDS not configured)");
            }
        }
        Err(error) => {
            eprintln!("Failed to create user in database: {}", error);
            std::process::exit(1);
        }
    }
}

async fn pds_create_session(identifier: &str, password: &str) {
    let Some(pds_client) = PdsClient::from_env() else {
        eprintln!("Error: PDS_URL environment variable not set");
        std::process::exit(1);
    };

    println!("Creating session for: {}", identifier);

    match pds_client.create_session(identifier, password).await {
        Ok(session) => {
            println!("Successfully created session!");
            println!("  DID: {}", session.did);
            println!("  Handle: {}", session.handle);
            println!("  Access JWT: {}", session.access_jwt);
            println!("  Refresh JWT: {}", session.refresh_jwt);
        }
        Err(error) => {
            eprintln!("Failed to create session: {}", error);
            std::process::exit(1);
        }
    }
}

async fn list_records(repo: &str, collection: &str, limit: u32) {
    let Some(pds_client) = PdsClient::from_env() else {
        eprintln!("Error: PDS_URL environment variable not set");
        std::process::exit(1);
    };

    match pds_client
        .list_records_raw(repo, collection, Some(limit))
        .await
    {
        Ok(records) => {
            // Pretty print the JSON
            println!(
                "{}",
                serde_json::to_string_pretty(&records).unwrap_or_else(|_| records.to_string())
            );
        }
        Err(error) => {
            eprintln!("Failed to list records: {}", error);
            std::process::exit(1);
        }
    }
}

async fn create_record(access_jwt: &str, repo: &str, collection: &str, record_json: &str) {
    let Some(pds_client) = PdsClient::from_env() else {
        eprintln!("Error: PDS_URL environment variable not set");
        std::process::exit(1);
    };

    // Parse the JSON record
    let record: serde_json::Value = match serde_json::from_str(record_json) {
        Ok(r) => r,
        Err(error) => {
            eprintln!("Failed to parse record JSON: {}", error);
            std::process::exit(1);
        }
    };

    match pds_client
        .create_record_raw(access_jwt, repo, collection, record)
        .await
    {
        Ok(response) => {
            println!("Successfully created record!");
            println!("  URI: {}", response.uri);
            println!("  CID: {}", response.cid);
        }
        Err(error) => {
            eprintln!("Failed to create record: {}", error);
            std::process::exit(1);
        }
    }
}
