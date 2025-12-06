use alcman::{Database, create_app};
use std::{env, sync::Arc};

#[tokio::main]
async fn main() {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();

    // Default values
    let mut port = 3000;
    let mut database_path = "./alcman.db".to_string();
    let mut serve = false;

    if args.len() == 1 {
        println!("alcman book abode");
        println!();
        println!("Usage:");
        println!("  {} --serve [OPTIONS]", args[0]);
        println!();
        println!("Options:");
        println!("  --serve                   Start the alcman server");
        println!("  --port <PORT>             Port to bind to (default: 3000)");
        println!("  --database <PATH>         Database file path (default: ./alcman.db)");
        println!();
        println!("Example:");
        println!(
            "  {} --serve --port 4000 --database /var/www/alcman/alcman.db",
            args[0]
        );
        println!();
        return;
    }

    // Parse arguments
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--serve" => serve = true,
            "--port" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u16>() {
                        Ok(p) => {
                            port = p;
                            i += 1; // Skip the next argument as it's the port value
                        }
                        Err(_) => {
                            eprintln!("Error: Invalid port number: {}", args[i + 1]);
                            std::process::exit(1);
                        }
                    }
                } else {
                    eprintln!("Error: --port requires a value");
                    std::process::exit(1);
                }
            }
            "--database" => {
                if i + 1 < args.len() {
                    database_path = args[i + 1].clone();
                    i += 1; // Skip the next argument as it's the database path
                } else {
                    eprintln!("Error: --database requires a value");
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!("Error: Unknown argument: {}", args[i]);
                println!("Use '{}' to see usage information.", args[0]);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    if !serve {
        println!("Error: --serve flag is required to start the server.");
        println!("Use '{}' to see usage information.", args[0]);
        std::process::exit(1);
    }

    // Initialize database
    let database_url = format!("sqlite:{}", database_path);
    let db = Database::new(&database_url)
        .await
        .expect("Failed to connect to database");

    // Run migrations
    if let Err(e) = db.run_migrations().await {
        eprintln!("Failed to run migrations: {}", e);
        std::process::exit(1);
    }

    println!(
        "Database ready for user registration API ({})",
        database_path
    );

    let app_state = Arc::new(db);

    // Build the router using the shared function
    let app = create_app(app_state);

    // Start the server
    let bind_address = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .expect("Failed to bind to address");

    println!("alcman running on http://{}", bind_address);

    axum::serve(listener, app)
        .await
        .expect("Failed to start server");
}
