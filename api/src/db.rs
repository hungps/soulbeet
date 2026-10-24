#[cfg(feature = "server")]
use dioxus::fullstack::Lazy;
#[cfg(feature = "server")]
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

#[cfg(feature = "server")]
use crate::config::CONFIG;

#[cfg(feature = "server")]
pub static DB: Lazy<SqlitePool> = Lazy::new(|| async move {
    let database_url = CONFIG.database_url();

    if database_url.starts_with("sqlite:") {
        let path_str = database_url.trim_start_matches("sqlite:");
        let path = std::path::Path::new(path_str);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("Failed to create database directory");
            }
            std::fs::File::create(path).expect("Failed to create database file");
        }
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
        .expect("Failed to connect to database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Download monitors are process-local, so nothing is polling when the
    // process starts. Requests left active by a restart are surfaced as
    // stopped so the transfer panel can resume them.
    let _ = sqlx::query("UPDATE download_request_tracks SET state = 'stopped' WHERE state IN ('queued', 'in_progress', 'completed', 'importing')")
        .execute(&pool)
        .await;
    let _ = sqlx::query("UPDATE download_requests SET state = 'stopped' WHERE state = 'active'")
        .execute(&pool)
        .await;

    dioxus::Ok(pool)
});
