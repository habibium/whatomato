use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::{Validate, ValidationError, ValidationErrors};

use crate::{db::message_templates::Template, routes::messages::check_body};

const NAME_MAX: usize = 100;

#[derive(Serialize, ToSchema)]
pub(super) struct MessageTemplate {
    pub id: Uuid,
    pub name: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Template> for MessageTemplate {
    fn from(template: Template) -> Self {
        Self {
            id: template.id,
            name: template.name,
            body: template.body,
            created_at: template.created_at,
            updated_at: template.updated_at,
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub(super) struct TemplateInput {
    /// Up to 100 characters
    pub name: String,
    /// Up to 4096 characters
    pub body: String,
}

impl Validate for TemplateInput {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        let name = self.name.trim().chars().count();
        if name == 0 || name > NAME_MAX {
            errors.add(
                "name",
                ValidationError::new("name")
                    .with_message("must be between 1 and 100 characters".into()),
            );
        }
        check_body(&self.body, &mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}
