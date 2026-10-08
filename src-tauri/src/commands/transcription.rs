use crate::managers::model::{RecognitionLanguageError, RecognitionLanguageStatus};
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{get_settings, write_settings, ModelUnloadTimeout, RecognitionLanguage};
use serde::Serialize;
use specta::Type;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

#[derive(Serialize, Type)]
pub struct ModelLoadStatus {
    is_loaded: bool,
    current_model: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub fn set_model_unload_timeout(app: AppHandle, timeout: ModelUnloadTimeout) {
    let mut settings = get_settings(&app);
    settings.model_unload_timeout = timeout;
    write_settings(&app, settings);
}

#[tauri::command]
#[specta::specta]
pub fn get_model_load_status(
    transcription_manager: State<Arc<TranscriptionManager>>,
) -> Result<ModelLoadStatus, String> {
    Ok(ModelLoadStatus {
        is_loaded: transcription_manager.is_model_loaded(),
        current_model: transcription_manager.get_current_model(),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn get_recognition_language_status(
    recognition_language: Option<RecognitionLanguage>,
    app: AppHandle,
) -> RecognitionLanguageStatus {
    let manager = Arc::clone(app.state::<Arc<TranscriptionManager>>().inner());
    tauri::async_runtime::spawn_blocking(move || {
        manager.recognition_language_status(recognition_language.as_ref())
    })
    .await
    .unwrap_or_else(|error| RecognitionLanguageStatus::Incompatible {
        reason: RecognitionLanguageError::ModelUnavailable {
            message: format!("Recognition language validation task failed: {error}"),
        },
    })
}

#[tauri::command]
#[specta::specta]
pub fn unload_model_manually(
    transcription_manager: State<Arc<TranscriptionManager>>,
) -> Result<(), String> {
    transcription_manager.request_unload();
    Ok(())
}
