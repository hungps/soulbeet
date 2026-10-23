#[cfg(feature = "server")]
use super::folder::Folder;
#[cfg(feature = "server")]
use crate::db::DB;
#[cfg(feature = "server")]
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use serde::{Deserialize, Serialize};
#[cfg(feature = "server")]
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "server", derive(sqlx::FromRow))]
pub struct User {
    pub id: String,
    pub username: String,
    #[serde(skip)]
    pub password_hash: String,
    pub media_server: String,
    pub media_token: Option<String>,
    pub media_status: String,
    pub media_account_id: Option<String>,
}

#[cfg(feature = "server")]
impl User {
    pub async fn create(username: &str, password: &str) -> Result<User, String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| e.to_string())?
            .to_string();

        let id = Uuid::new_v4().to_string();

        let user = sqlx::query_as::<_, User>(
            "INSERT INTO users (id, username, password_hash, media_status) VALUES (?, ?, ?, ?) RETURNING *"
        )
        .bind(&id)
        .bind(username)
        .bind(password_hash)
        .bind(shared::system::MediaServerStatus::Unknown.as_str())
        .fetch_one(&*DB)
        .await
        .map_err(|e| e.to_string())?;

        Ok(user)
    }

    pub async fn verify(username: &str, password: &str) -> Result<User, String> {
        let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&*DB)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("User not found")?;

        let parsed_hash = PasswordHash::new(&user.password_hash).map_err(|e| e.to_string())?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .map_err(|_| "Invalid password")?;

        Ok(user)
    }

    pub async fn get_folders(&self) -> Result<Vec<Folder>, String> {
        sqlx::query_as::<_, Folder>("SELECT * FROM folders WHERE user_id = ?")
            .bind(&self.id)
            .fetch_all(&*DB)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn get_all() -> Result<Vec<User>, String> {
        sqlx::query_as::<_, User>("SELECT * FROM users")
            .fetch_all(&*DB)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn get_by_id(id: &str) -> Result<User, String> {
        let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&*DB)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("User not found")?;

        Ok(user)
    }

    pub async fn update_password(id: &str, password: &str) -> Result<(), String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| e.to_string())?
            .to_string();

        sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(password_hash)
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn update_username(id: &str, new_username: &str) -> Result<(), String> {
        sqlx::query("UPDATE users SET username = ? WHERE id = ?")
            .bind(new_username)
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn delete(id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_by_username(username: &str) -> Result<Option<User>, String> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&*DB)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn update_media(
        id: &str,
        server: &str,
        token: Option<&str>,
        status: &str,
        account_id: Option<&str>,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE users SET media_server = ?, media_token = ?, media_status = ?, media_account_id = ? WHERE id = ?",
        )
        .bind(server)
        .bind(token)
        .bind(status)
        .bind(account_id)
        .bind(id)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn set_active_media(
        id: &str,
        server: &str,
        token: Option<&str>,
        status: &str,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE users SET media_server = ?, media_token = ?, media_status = ? WHERE id = ?",
        )
        .bind(server)
        .bind(token)
        .bind(status)
        .bind(id)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn clear_media_account(id: &str) -> Result<(), String> {
        sqlx::query("UPDATE users SET media_account_id = NULL WHERE id = ?")
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_by_media_account(
        server: &str,
        account_id: &str,
    ) -> Result<Option<User>, String> {
        sqlx::query_as::<_, User>(
            "SELECT * FROM users WHERE media_server = ? AND media_account_id = ?",
        )
        .bind(server)
        .bind(account_id)
        .fetch_optional(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn get_connected_users() -> Result<Vec<User>, String> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE media_status IN (?, ?)")
            .bind(shared::system::MediaServerStatus::Connected.as_str())
            .bind(shared::system::MediaServerStatus::MissingReportRealPath.as_str())
            .fetch_all(&*DB)
            .await
            .map_err(|e| e.to_string())
    }
}
