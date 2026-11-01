pub mod auth;
pub mod health;
pub mod whatsapp;

use crate::error::AppError;

pub async fn not_found() -> AppError {
    AppError::NotFound
}
