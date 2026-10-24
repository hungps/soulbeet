use crate::models;
use dioxus::prelude::*;
use shared::library::DuplicateReport;

#[cfg(feature = "server")]
use super::server_error;
#[cfg(feature = "server")]
use crate::services::music_importer;
#[cfg(feature = "server")]
use crate::AuthSession;
#[cfg(feature = "server")]
use std::path::Path;

#[get("/api/folders", auth: AuthSession)]
pub async fn get_user_folders() -> Result<Vec<models::folder::Folder>, ServerFnError> {
    let claims = auth.0;

    models::folder::Folder::get_all_by_user(&claims.sub)
        .await
        .map_err(server_error)
}

#[post("/api/folders", auth: AuthSession)]
pub async fn create_user_folder(
    name: String,
    path: String,
) -> Result<models::folder::Folder, ServerFnError> {
    let claims = auth.0;

    if let Err(e) = tokio::fs::create_dir_all(&path).await {
        return Err(server_error(format!("Failed to create directory: {}", e)));
    }

    let folder = models::folder::Folder::create(&claims.sub, &name, &path)
        .await
        .map_err(server_error)?;

    #[cfg(feature = "server")]
    {
        let id = folder.id.clone();
        tokio::spawn(async move {
            let _ = crate::library::scan_folder_internal(&id, true).await;
        });
    }

    Ok(folder)
}

#[put("/api/folders/update", _: AuthSession)]
pub async fn update_folder(
    folder_id: String,
    name: String,
    path: String,
) -> Result<(), ServerFnError> {
    let old = models::folder::Folder::get_by_id(&folder_id)
        .await
        .map_err(server_error)?;
    models::folder::Folder::update(&folder_id, &name, &path)
        .await
        .map_err(server_error)?;

    #[cfg(feature = "server")]
    {
        if old.map(|folder| folder.path) != Some(path) {
            clear_indexed_folder(&folder_id).await;
            tokio::spawn(async move {
                let _ = crate::library::scan_folder_internal(&folder_id, true).await;
            });
        }
    }

    Ok(())
}

#[delete("/api/folders/delete", _: AuthSession)]
pub async fn delete_folder(folder_id: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    clear_indexed_folder(&folder_id).await;

    models::folder::Folder::delete(&folder_id)
        .await
        .map_err(server_error)
}

#[cfg(feature = "server")]
async fn clear_indexed_folder(folder_id: &str) {
    use crate::db::DB;

    let _ = sqlx::query("DELETE FROM library_tracks WHERE folder_id = ?")
        .bind(folder_id)
        .execute(&*DB)
        .await;
    let _ = sqlx::query("DELETE FROM library_releases WHERE folder_id = ?")
        .bind(folder_id)
        .execute(&*DB)
        .await;
    let _ = sqlx::query("DELETE FROM library_scan_state WHERE folder_id = ?")
        .bind(folder_id)
        .execute(&*DB)
        .await;
}

/// Find duplicate tracks across all of the user's library folders.
/// Returns tracks that exist in multiple libraries (same artist + title).
#[get("/api/folders/duplicates", auth: AuthSession)]
pub async fn get_folder_duplicates() -> Result<DuplicateReport, ServerFnError> {
    let claims = auth.0;

    let folders = models::folder::Folder::get_all_by_user(&claims.sub)
        .await
        .map_err(server_error)?;

    if folders.len() < 2 {
        return Ok(DuplicateReport {
            duplicates: vec![],
            total_duplicate_tracks: 0,
            libraries_scanned: folders.iter().map(|f| f.path.clone()).collect(),
        });
    }

    let paths: Vec<&Path> = folders.iter().map(|f| Path::new(&f.path)).collect();

    let importer = music_importer(None).await.map_err(server_error)?;
    importer.find_duplicates(&paths).await.map_err(server_error)
}
