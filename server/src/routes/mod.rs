pub mod auth;
pub mod deliveries;
pub mod health;
pub mod messages;
pub mod templates;
pub mod whatsapp;

use crate::error::AppError;

pub async fn not_found() -> AppError {
    AppError::NotFound
}
