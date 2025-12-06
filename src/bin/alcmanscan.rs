use alcman::Database;
use alcman::gpt::{GptClient, GptConfig, GptError};
use epub::doc::EpubDoc;
use lopdf::Document;
use std::path::Path;
use std::{env, process};
use walkdir::WalkDir;

const BOOK_EXTENSIONS: &[&str] = &["epub", "mobi", "pdf", "docx", "txt"];

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        print_usage();
        process::exit(1);
    }

    // Check for --scan-dir option
    if args[0] == "--scan-dir" || args[0] == "-d" {
        if args.len() < 2 {
            eprintln!("Error: --scan-dir requires a directory path");
            print_usage();
            process::exit(1);
        }

        // Check for --save flag
        let save_to_db = args.iter().any(|a| a == "--save" || a == "-s");

        if let Err(e) = scan_directory(&args[1], save_to_db).await {
            eprintln!("Error scanning directory: {}", e);
            process::exit(1);
        }
        return;
    }

    // Default behavior: summarize book title
    let title = args.join(" ");

    let config = GptConfig::from_env();
    if config.api_key().is_none() {
        eprintln!(
            "OPENAI_API_KEY is not configured. Please export it before running the alcmanscan command."
        );
        process::exit(1);
    }

    let client = GptClient::new(config);
    if let Err(error) = run_scan(&client, &title).await {
        eprintln!("Failed to summarize \"{title}\": {error}");
        process::exit(1);
    }
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  alcmanscan \"Book Title\"              - Summarize a book by title");
    eprintln!("  alcmanscan --scan-dir <directory>    - Scan directory for book files");
    eprintln!(
        "  alcmanscan -d <directory>            - Scan directory for book files (short form)"
    );
    eprintln!("  alcmanscan --scan-dir <dir> --save   - Scan and save books to database");
    eprintln!("  alcmanscan -d <dir> -s               - Scan and save (short form)");
    eprintln!();
    eprintln!("Supported file types: epub, mobi, pdf, docx, txt");
}

async fn scan_directory(
    dir_path: &str,
    save_to_db: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(dir_path);

    if !path.exists() {
        return Err(format!("Directory '{}' does not exist", dir_path).into());
    }

    if !path.is_dir() {
        return Err(format!("'{}' is not a directory", dir_path).into());
    }

    // Initialize database if saving
    let db = if save_to_db {
        let database_url =
            env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:alcman.db".to_string());
        let db = Database::new(&database_url).await?;
        db.run_migrations().await?;
        Some(db)
    } else {
        None
    };

    // Canonicalize the base path for proper relative path calculation
    let base_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    println!("Scanning directory: {}", dir_path);
    if save_to_db {
        println!("Saving books to database...");
        println!("(storing paths relative to LIBRARY_PATH)");
    }
    println!();

    let mut count = 0;
    let mut saved_count = 0;

    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        let file_path = entry.path();

        if file_path.is_file()
            && let Some(ext) = file_path.extension()
            && let Some(ext_str) = ext.to_str()
        {
            let ext_lower = ext_str.to_lowercase();
            if BOOK_EXTENSIONS.contains(&ext_lower.as_str()) {
                println!("{}", file_path.display());

                // Calculate relative path from the base directory
                let relative_path = file_path
                    .canonicalize()
                    .ok()
                    .and_then(|p| p.strip_prefix(&base_path).ok().map(|r| r.to_path_buf()))
                    .unwrap_or_else(|| file_path.to_path_buf());
                let relative_path_str = relative_path.to_string_lossy().to_string();

                // Extract metadata based on file type
                let book_data = if ext_lower == "epub" {
                    extract_epub_metadata(file_path).map(|m| {
                        print_epub_metadata(&m);
                        BookData {
                            title: m.title,
                            author: m.author,
                            publication_year: parse_year(&m.date),
                            filepath: relative_path_str.clone(),
                        }
                    })
                } else if ext_lower == "pdf" {
                    extract_pdf_metadata(file_path).map(|m| {
                        print_pdf_metadata(&m);
                        BookData {
                            title: m.title,
                            author: m.author,
                            publication_year: parse_year(&m.creation_date),
                            filepath: relative_path_str.clone(),
                        }
                    })
                } else {
                    // For other formats, use filename as title
                    Some(BookData {
                        title: file_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(String::from),
                        author: None,
                        publication_year: None,
                        filepath: relative_path_str.clone(),
                    })
                };

                // Save to database if requested
                if let (Some(db), Some(data)) = (&db, book_data) {
                    if let Some(title) = &data.title {
                        match db
                            .upsert_book_by_filepath(
                                &data.filepath,
                                title,
                                data.author.as_deref(),
                                data.publication_year,
                            )
                            .await
                        {
                            Ok(_) => {
                                println!("  [SAVED]");
                                saved_count += 1;
                            }
                            Err(e) => {
                                eprintln!("  [ERROR saving: {}]", e);
                            }
                        }
                    } else {
                        println!("  [SKIPPED: no title]");
                    }
                }

                count += 1;
                println!();
            }
        }
    }

    println!("Found {} book file(s)", count);
    if save_to_db {
        println!("Saved {} book(s) to database", saved_count);
    }

    Ok(())
}

/// Data extracted from a book file for database storage
struct BookData {
    title: Option<String>,
    author: Option<String>,
    publication_year: Option<i32>,
    filepath: String,
}

/// Parse a year from various date formats
fn parse_year(date: &Option<String>) -> Option<i32> {
    let date = date.as_ref()?;

    // Only work with ASCII digits to avoid UTF-8 boundary issues
    let chars: Vec<char> = date.chars().collect();

    // Try to extract a 4-digit year from the beginning
    if chars.len() >= 4 {
        let first_four: String = chars[..4].iter().collect();
        if let Ok(year) = first_four.parse::<i32>()
            && (1000..=2100).contains(&year)
        {
            return Some(year);
        }
    }

    // Try to find any 4-digit year in the string
    for i in 0..chars.len().saturating_sub(3) {
        let four_chars: String = chars[i..i + 4].iter().collect();
        if let Ok(year) = four_chars.parse::<i32>()
            && (1800..=2100).contains(&year)
        {
            return Some(year);
        }
    }

    None
}

struct EpubMetadata {
    title: Option<String>,
    author: Option<String>,
    publisher: Option<String>,
    date: Option<String>,
    language: Option<String>,
    description: Option<String>,
}

fn extract_epub_metadata(path: &Path) -> Option<EpubMetadata> {
    let doc = EpubDoc::new(path).ok()?;

    Some(EpubMetadata {
        title: doc.mdata("title").map(|m| m.value.clone()),
        author: doc.mdata("creator").map(|m| m.value.clone()),
        publisher: doc.mdata("publisher").map(|m| m.value.clone()),
        date: doc.mdata("date").map(|m| m.value.clone()),
        language: doc.mdata("language").map(|m| m.value.clone()),
        description: doc.mdata("description").map(|m| m.value.clone()),
    })
}

fn print_epub_metadata(metadata: &EpubMetadata) {
    if let Some(title) = &metadata.title {
        println!("  Title: {}", title);
    }
    if let Some(author) = &metadata.author {
        println!("  Author: {}", author);
    }
    if let Some(publisher) = &metadata.publisher {
        println!("  Publisher: {}", publisher);
    }
    if let Some(date) = &metadata.date {
        println!("  Date: {}", date);
    }
    if let Some(language) = &metadata.language {
        println!("  Language: {}", language);
    }
    if let Some(description) = &metadata.description {
        // Truncate long descriptions
        let desc = if description.len() > 200 {
            format!("{}...", &description[..200])
        } else {
            description.clone()
        };
        println!("  Description: {}", desc);
    }
}

struct PdfMetadata {
    title: Option<String>,
    author: Option<String>,
    subject: Option<String>,
    creator: Option<String>,
    producer: Option<String>,
    creation_date: Option<String>,
}

fn extract_pdf_metadata(path: &Path) -> Option<PdfMetadata> {
    let doc = Document::load(path).ok()?;

    // Get the Info dictionary reference from trailer
    let info_ref = doc.trailer.get(b"Info").ok()?;
    let info_ref = info_ref.as_reference().ok()?;
    let info_dict = doc.get_dictionary(info_ref).ok()?;

    Some(PdfMetadata {
        title: get_pdf_string(&doc, info_dict, b"Title"),
        author: get_pdf_string(&doc, info_dict, b"Author"),
        subject: get_pdf_string(&doc, info_dict, b"Subject"),
        creator: get_pdf_string(&doc, info_dict, b"Creator"),
        producer: get_pdf_string(&doc, info_dict, b"Producer"),
        creation_date: get_pdf_string(&doc, info_dict, b"CreationDate"),
    })
}

fn get_pdf_string(doc: &Document, dict: &lopdf::Dictionary, key: &[u8]) -> Option<String> {
    let obj = dict.get(key).ok()?;

    // Handle both direct strings and references
    match obj {
        lopdf::Object::String(bytes, _) => {
            // Try UTF-16 BE first (starts with BOM 0xFE 0xFF)
            if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
                let utf16: Vec<u16> = bytes[2..]
                    .chunks(2)
                    .filter_map(|chunk| {
                        if chunk.len() == 2 {
                            Some(u16::from_be_bytes([chunk[0], chunk[1]]))
                        } else {
                            None
                        }
                    })
                    .collect();
                String::from_utf16(&utf16).ok()
            } else {
                // Try as Latin-1/UTF-8
                Some(String::from_utf8_lossy(bytes).to_string())
            }
        }
        lopdf::Object::Reference(r) => {
            if let Ok(lopdf::Object::String(bytes, _)) = doc.get_object(*r) {
                Some(String::from_utf8_lossy(bytes).to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Check if a string contains disallowed control characters (Unicode Cc category, except \t \n \r)
fn is_printable_text(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    // Check for replacement characters (indicates failed UTF-8 decoding)
    if s.contains('\u{FFFD}') {
        return false;
    }

    // Disallowed control characters (Unicode Cc category except \t, \n, \r)
    for c in s.chars() {
        match c {
            '\x00'..='\x08' | '\x0B' | '\x0C' | '\x0E'..='\x1F' | '\x7F' | '\u{80}'..='\u{9F}' => {
                return false;
            }
            _ => {}
        }
    }

    true
}

fn print_pdf_metadata(metadata: &PdfMetadata) {
    if let Some(title) = &metadata.title
        && is_printable_text(title)
    {
        println!("  Title: {}", title);
    }
    if let Some(author) = &metadata.author
        && is_printable_text(author)
    {
        println!("  Author: {}", author);
    }
    if let Some(subject) = &metadata.subject
        && is_printable_text(subject)
    {
        println!("  Subject: {}", subject);
    }
    if let Some(creator) = &metadata.creator
        && is_printable_text(creator)
    {
        println!("  Creator: {}", creator);
    }
    if let Some(producer) = &metadata.producer
        && is_printable_text(producer)
    {
        println!("  Producer: {}", producer);
    }
    if let Some(date) = &metadata.creation_date
        && is_printable_text(date)
    {
        println!("  Created: {}", date);
    }
}

async fn run_scan(client: &GptClient, title: &str) -> Result<(), GptError> {
    println!("Scanning \"{title}\"...");
    let summary = client.summarize_book(title).await?;
    println!("\nSummary: {summary}");
    Ok(())
}
