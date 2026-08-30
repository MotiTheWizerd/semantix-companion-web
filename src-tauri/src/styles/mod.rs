// CONVERSATIONAL STYLES — the voice a companion wears.
//
// A style is a reusable library entry: a name, an optional trait sheet (the
// "style card"), and real example exchanges that teach a voice by
// demonstration. It exists for the user who misses how an older model spoke,
// or who wants a persona — the exemplars ride the system prompt of whatever
// model the companion runs on, and models that follow instructions reproduce
// the voice.
//
// THE LINE THIS FEATURE HOLDS: styles transfer a VOICE, never an identity.
// The prompt block says "adopt this way of speaking"; it never tells a model
// it IS some other product, and it tells it to answer honestly about what it
// is when asked. A style is a coat, not a mask.

pub(crate) mod harvest;
pub(crate) mod repository;

use std::{path::Path, sync::Arc};

pub(crate) use repository::StyleRepository;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::{app_error::AppError, credentials::unix_timestamp_ms};

pub(crate) const STYLES_CHANGED_EVENT: &str = "styles://changed";

const MAX_NAME_LENGTH: usize = 80;
const MAX_DESCRIPTION_LENGTH: usize = 400;
const MAX_STYLE_CARD_LENGTH: usize = 8_000;
/// One exemplar side longer than this is a document, not a turn — and a
/// handful of monsters would eat the entire prompt budget of the style.
const MAX_EXEMPLAR_TEXT_LENGTH: usize = 8_000;
/// More pairs than this teaches nothing new; a voice saturates in dozens.
const MAX_EXEMPLARS: usize = 500;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Style {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    /// The distilled trait sheet — registers, habits, nevers. Optional: a
    /// style of nothing but exemplars still works, the card just names what
    /// the examples show.
    pub(crate) style_card: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    /// How many exemplars the style holds — the roster shows it without
    /// loading the pairs themselves.
    pub(crate) exemplar_count: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StyleExemplar {
    pub(crate) id: String,
    pub(crate) position: i64,
    pub(crate) user_text: String,
    pub(crate) companion_text: String,
    /// YYYY-MM of the source exchange, when known. A voice drifts across
    /// months; the month is unrecoverable once dropped.
    pub(crate) era: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StyleExemplarInput {
    user_text: String,
    companion_text: String,
    #[serde(default)]
    era: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateStyleInput {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    style_card: Option<String>,
    #[serde(default)]
    exemplars: Vec<StyleExemplarInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateStyleInput {
    style_id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    style_card: Option<String>,
    /// `None` keeps the stored exemplar set untouched — a rename or a card
    /// edit should not have to re-send thousands of pairs. `Some` replaces
    /// the set wholesale, empty included.
    #[serde(default)]
    exemplars: Option<Vec<StyleExemplarInput>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum StyleChangedEvent {
    Created {
        style: Style,
    },
    Updated {
        style: Style,
    },
    Deleted {
        #[serde(rename = "styleId")]
        style_id: String,
    },
}

pub(crate) struct StyleState {
    service: Arc<StyleService>,
}

impl StyleState {
    pub(crate) fn open(database_path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            service: Arc::new(StyleService {
                repository: StyleRepository::open(database_path)?,
            }),
        })
    }
}

struct StyleService {
    repository: StyleRepository,
}

impl StyleService {
    fn list(&self) -> Result<Vec<Style>, AppError> {
        self.repository.list()
    }

    fn exemplars(&self, style_id: &str) -> Result<Vec<StyleExemplar>, AppError> {
        self.require(style_id.trim())?;
        self.repository.exemplars(style_id.trim(), None)
    }

    fn create(&self, input: CreateStyleInput) -> Result<Style, AppError> {
        let CreateStyleInput {
            name,
            description,
            style_card,
            exemplars,
        } = input;
        let name = normalise_name(name)?;
        let description = normalise_description(description)?;
        let style_card = normalise_style_card(style_card)?;
        let exemplars = normalise_exemplars(exemplars)?;
        let timestamp = unix_timestamp_ms()?;
        let style = Style {
            id: Uuid::new_v4().to_string(),
            name,
            description,
            style_card,
            created_at: timestamp,
            updated_at: timestamp,
            exemplar_count: exemplars.len() as i64,
        };
        self.repository.insert(&style, &exemplars)?;
        Ok(style)
    }

    fn update(&self, input: UpdateStyleInput) -> Result<Style, AppError> {
        let UpdateStyleInput {
            style_id,
            name,
            description,
            style_card,
            exemplars,
        } = input;
        let current = self.require(style_id.trim())?;
        let name = normalise_name(name)?;
        let description = normalise_description(description)?;
        let style_card = normalise_style_card(style_card)?;
        let exemplars = exemplars.map(normalise_exemplars).transpose()?;
        let timestamp = unix_timestamp_ms()?;
        self.repository.update(
            &current.id,
            &name,
            description.as_deref(),
            style_card.as_deref(),
            exemplars.as_deref(),
            timestamp,
        )?;
        Ok(Style {
            name,
            description,
            style_card,
            updated_at: timestamp,
            exemplar_count: exemplars
                .as_ref()
                .map(|set| set.len() as i64)
                .unwrap_or(current.exemplar_count),
            ..current
        })
    }

    fn delete(&self, id: &str) -> Result<(), AppError> {
        let style = self.require(id.trim())?;
        self.repository.delete(&style.id)
    }

    fn require(&self, id: &str) -> Result<Style, AppError> {
        self.repository
            .get(id)?
            .ok_or_else(|| AppError::validation("That style no longer exists."))
    }
}

fn normalise_name(name: String) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::validation("Every style needs a name."));
    }
    if name.chars().count() > MAX_NAME_LENGTH {
        return Err(AppError::validation(format!(
            "A style name must be {MAX_NAME_LENGTH} characters or fewer."
        )));
    }
    Ok(name.to_owned())
}

fn normalise_description(description: Option<String>) -> Result<Option<String>, AppError> {
    let Some(description) = description else { return Ok(None) };
    let description = description.trim();
    if description.is_empty() {
        return Ok(None);
    }
    if description.chars().count() > MAX_DESCRIPTION_LENGTH {
        return Err(AppError::validation(format!(
            "A style description must be {MAX_DESCRIPTION_LENGTH} characters or fewer."
        )));
    }
    Ok(Some(description.to_owned()))
}

fn normalise_style_card(style_card: Option<String>) -> Result<Option<String>, AppError> {
    let Some(style_card) = style_card else { return Ok(None) };
    let style_card = style_card.trim();
    if style_card.is_empty() {
        return Ok(None);
    }
    if style_card.chars().count() > MAX_STYLE_CARD_LENGTH {
        return Err(AppError::validation(format!(
            "A style card must be {MAX_STYLE_CARD_LENGTH} characters or fewer."
        )));
    }
    Ok(Some(style_card.to_owned()))
}

fn normalise_exemplars(
    inputs: Vec<StyleExemplarInput>,
) -> Result<Vec<StyleExemplar>, AppError> {
    if inputs.len() > MAX_EXEMPLARS {
        return Err(AppError::validation(format!(
            "A style can hold at most {MAX_EXEMPLARS} example exchanges."
        )));
    }
    let mut exemplars = Vec::with_capacity(inputs.len());
    for (position, input) in inputs.into_iter().enumerate() {
        let user_text = input.user_text.trim();
        let companion_text = input.companion_text.trim();
        if user_text.is_empty() || companion_text.is_empty() {
            return Err(AppError::validation(
                "Every example exchange needs both sides: what was said and the reply.",
            ));
        }
        if user_text.chars().count() > MAX_EXEMPLAR_TEXT_LENGTH
            || companion_text.chars().count() > MAX_EXEMPLAR_TEXT_LENGTH
        {
            return Err(AppError::validation(format!(
                "Each side of an example exchange must be {MAX_EXEMPLAR_TEXT_LENGTH} characters or fewer."
            )));
        }
        let era = input
            .era
            .as_deref()
            .map(str::trim)
            .filter(|era| !era.is_empty())
            .map(str::to_owned);
        exemplars.push(StyleExemplar {
            id: Uuid::new_v4().to_string(),
            position: position as i64,
            user_text: user_text.to_owned(),
            companion_text: companion_text.to_owned(),
            era,
        });
    }
    Ok(exemplars)
}

#[tauri::command]
pub(crate) async fn list_styles(state: State<'_, StyleState>) -> Result<Vec<Style>, String> {
    let service = Arc::clone(&state.service);
    tauri::async_runtime::spawn_blocking(move || service.list())
        .await
        .map_err(|error| format!("Style task failed: {error}"))?
        .map_err(String::from)
}

#[tauri::command]
pub(crate) async fn get_style_exemplars(
    state: State<'_, StyleState>,
    style_id: String,
) -> Result<Vec<StyleExemplar>, String> {
    let service = Arc::clone(&state.service);
    tauri::async_runtime::spawn_blocking(move || service.exemplars(&style_id))
        .await
        .map_err(|error| format!("Style task failed: {error}"))?
        .map_err(String::from)
}

#[tauri::command]
pub(crate) async fn create_style(
    app: AppHandle,
    state: State<'_, StyleState>,
    input: CreateStyleInput,
) -> Result<Style, String> {
    let service = Arc::clone(&state.service);
    let style = tauri::async_runtime::spawn_blocking(move || service.create(input))
        .await
        .map_err(|error| format!("Style task failed: {error}"))?
        .map_err(String::from)?;
    let _ = app.emit(
        STYLES_CHANGED_EVENT,
        StyleChangedEvent::Created { style: style.clone() },
    );
    Ok(style)
}

#[tauri::command]
pub(crate) async fn update_style(
    app: AppHandle,
    state: State<'_, StyleState>,
    input: UpdateStyleInput,
) -> Result<Style, String> {
    let service = Arc::clone(&state.service);
    let style = tauri::async_runtime::spawn_blocking(move || service.update(input))
        .await
        .map_err(|error| format!("Style task failed: {error}"))?
        .map_err(String::from)?;
    let _ = app.emit(
        STYLES_CHANGED_EVENT,
        StyleChangedEvent::Updated { style: style.clone() },
    );
    Ok(style)
}

#[tauri::command]
pub(crate) async fn delete_style(
    app: AppHandle,
    state: State<'_, StyleState>,
    style_id: String,
) -> Result<(), String> {
    let service = Arc::clone(&state.service);
    let id = style_id.clone();
    tauri::async_runtime::spawn_blocking(move || service.delete(&id))
        .await
        .map_err(|error| format!("Style task failed: {error}"))?
        .map_err(String::from)?;
    let _ = app.emit(
        STYLES_CHANGED_EVENT,
        StyleChangedEvent::Deleted { style_id },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exemplar(user: &str, companion: &str) -> StyleExemplarInput {
        StyleExemplarInput {
            user_text: user.to_owned(),
            companion_text: companion.to_owned(),
            era: None,
        }
    }

    #[test]
    fn exemplars_are_positioned_in_input_order() {
        let exemplars =
            normalise_exemplars(vec![exemplar("a", "b"), exemplar("c", "d")]).expect("valid");
        assert_eq!(exemplars[0].position, 0);
        assert_eq!(exemplars[1].position, 1);
        assert_eq!(exemplars[1].user_text, "c");
    }

    #[test]
    fn a_one_sided_exemplar_is_refused() {
        assert!(normalise_exemplars(vec![exemplar("a", "  ")]).is_err());
        assert!(normalise_exemplars(vec![exemplar("", "b")]).is_err());
    }

    #[test]
    fn blank_optional_fields_store_as_null() {
        assert_eq!(normalise_description(Some("  ".into())).unwrap(), None);
        assert_eq!(normalise_style_card(Some(String::new())).unwrap(), None);
    }
}
