//! Tauri global-shortcut implementation
//!
//! This module provides shortcut functionality using Tauri's built-in
//! global-shortcut plugin.

use log::{debug, error, warn};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::settings::{self, ShortcutBinding};

use super::handler::handle_shortcut_event;

#[derive(Default)]
struct RegisteredShortcuts {
    bindings: Mutex<HashMap<String, ShortcutBinding>>,
}

fn registrations(app: &AppHandle) -> tauri::State<'_, RegisteredShortcuts> {
    if app.try_state::<RegisteredShortcuts>().is_none() {
        app.manage(RegisteredShortcuts::default());
    }
    app.state::<RegisteredShortcuts>()
}

/// Only registrations owned by this backend, never another application's chord.
pub fn registered_shortcut(app: &AppHandle, id: &str) -> Result<Option<ShortcutBinding>, String> {
    let state = registrations(app);
    let registered = state
        .bindings
        .lock()
        .map_err(|_| "Shortcut registrations lock poisoned")?;
    Ok(registered.get(id).cloned())
}

/// Initialize shortcuts using Tauri's global-shortcut plugin
pub fn init_shortcuts(app: &AppHandle) {
    let default_bindings = settings::get_default_settings().bindings;
    let user_settings = settings::load_or_create_app_settings(app);

    // Register all default shortcuts, applying user customizations
    for (id, default_binding) in default_bindings {
        if id == "cancel" {
            continue; // Skip cancel shortcut, it will be registered dynamically
        }
        // Skip post-processing shortcut when the feature is disabled
        if id == "transcribe_with_post_process" && !user_settings.post_process_enabled {
            continue;
        }
        let binding = user_settings
            .bindings
            .get(&id)
            .cloned()
            .unwrap_or(default_binding);

        if let Err(e) = register_shortcut(app, binding) {
            error!("Failed to register shortcut {} during init: {}", id, e);
        }
    }
}

/// Normalize aliases and chord order using the same parser as OS registration.
pub fn normalize_shortcut(raw: &str) -> Result<String, String> {
    if raw.trim().is_empty() {
        return Err("Shortcut cannot be empty".into());
    }
    let mut parts: Vec<_> = raw
        .split('+')
        .map(|part| {
            let part = part.trim();
            if ["meta", "win", "windows"]
                .iter()
                .any(|alias| part.eq_ignore_ascii_case(alias))
            {
                "super"
            } else if part.eq_ignore_ascii_case("opt") {
                "alt"
            } else {
                part
            }
        })
        .collect();
    if parts
        .iter()
        .any(|part| part.eq_ignore_ascii_case("fn") || part.eq_ignore_ascii_case("function"))
    {
        return Err("The 'fn' key is not supported by Tauri global shortcuts".into());
    }
    parts.sort_by_key(|part| {
        ![
            "ctrl",
            "control",
            "shift",
            "alt",
            "option",
            "command",
            "cmd",
            "super",
            "commandorcontrol",
            "commandorctrl",
            "cmdorctrl",
            "cmdorcontrol",
        ]
        .iter()
        .any(|modifier| part.eq_ignore_ascii_case(modifier))
    });
    let shortcut = parts
        .join("+")
        .parse::<Shortcut>()
        .map_err(|error| format!("Failed to parse shortcut '{raw}': {error}"))?;
    let mut normalized = shortcut.into_string();
    normalized.make_ascii_lowercase();
    // Keep common key spellings accepted by both backends; Tauri's Display
    // uses KeyA/Digit1/ArrowUp, which HandyKeys cannot parse on a later switch.
    let key_start = normalized.rfind('+').map_or(0, |index| index + 1);
    let key = &normalized[key_start..];
    let prefix = if key.starts_with("key") && key.len() == 4 {
        3
    } else if (key.starts_with("digit") && key.len() == 6) || key.starts_with("arrow") {
        5
    } else {
        0
    };
    normalized.replace_range(key_start..key_start + prefix, "");
    Ok(normalized)
}

/// Validate a shortcut string for the Tauri global-shortcut implementation.
pub fn validate_shortcut(raw: &str) -> Result<(), String> {
    normalize_shortcut(raw).map(|_| ())
}

/// Register a shortcut using Tauri's global-shortcut plugin
pub fn register_shortcut(app: &AppHandle, mut binding: ShortcutBinding) -> Result<(), String> {
    if !super::binding_is_enabled(&settings::get_settings(app), &binding) {
        return Ok(());
    }
    binding.current_binding = normalize_shortcut(&binding.current_binding)?;
    let state = registrations(app);
    let mut registered = state
        .bindings
        .lock()
        .map_err(|_| "Shortcut registrations lock poisoned")?;
    if let Some(previous) = registered.get(&binding.id) {
        if normalize_shortcut(&previous.current_binding)? == binding.current_binding {
            return Ok(());
        }
        return Err(format!("Binding '{}' is already registered", binding.id));
    }
    let shortcut = binding
        .current_binding
        .parse::<Shortcut>()
        .map_err(|error| error.to_string())?;

    // Prevent duplicate registrations that would silently shadow one another
    if app.global_shortcut().is_registered(shortcut) {
        let error_msg = format!("Shortcut '{}' is already in use", binding.current_binding);
        warn!("register_tauri_shortcut duplicate error: {}", error_msg);
        return Err(error_msg);
    }

    // Clone binding.id for use in the closure
    let binding_id_for_closure = binding.id.clone();

    app.global_shortcut()
        .on_shortcut(shortcut, move |app_handle, scut, event| {
            if scut == &shortcut {
                let shortcut_string = scut.into_string();
                let is_pressed = event.state == ShortcutState::Pressed;
                // Mirrors the handy-keys event log line; the distinct prefix
                // makes it possible to tell which backend fired a shortcut
                // (e.g. when diagnosing the Secure Input fallback)
                debug!(
                    "tauri global-shortcut event: binding={}, shortcut={}, state={:?}",
                    binding_id_for_closure, shortcut_string, event.state
                );
                handle_shortcut_event(
                    app_handle,
                    &binding_id_for_closure,
                    &shortcut_string,
                    is_pressed,
                );
            }
        })
        .map_err(|e| {
            let error_msg = format!(
                "Couldn't register shortcut '{}': {}",
                binding.current_binding, e
            );
            error!("register_tauri_shortcut registration error: {}", error_msg);
            error_msg
        })?;
    registered.insert(binding.id.clone(), binding);

    Ok(())
}

/// Unregister a shortcut from Tauri's global-shortcut plugin
pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    let state = registrations(app);
    let mut registered = state
        .bindings
        .lock()
        .map_err(|_| "Shortcut registrations lock poisoned")?;
    let Some(previous) = registered.get(&binding.id) else {
        return Ok(());
    };
    if normalize_shortcut(&previous.current_binding)?
        != normalize_shortcut(&binding.current_binding)?
    {
        return Ok(());
    }
    let shortcut = normalize_shortcut(&previous.current_binding)?
        .parse::<Shortcut>()
        .map_err(|error| error.to_string())?;

    app.global_shortcut().unregister(shortcut).map_err(|e| {
        let error_msg = format!(
            "Failed to unregister shortcut '{}': {}",
            binding.current_binding, e
        );
        error!("unregister_tauri_shortcut error: {}", error_msg);
        error_msg
    })?;
    registered.remove(&binding.id);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_shortcut;

    #[test]
    fn rejects_side_specific_modifiers_the_parser_cannot_register() {
        for raw in ["option_left+space", "ctrl_right+space"] {
            assert!(validate_shortcut(raw).is_err(), "{raw} should be rejected");
        }
    }

    #[test]
    fn accepts_the_default_shortcuts() {
        for raw in [
            "option+space",
            "option+shift+space",
            "ctrl+space",
            "ctrl+shift+space",
            "alt+space",
            "escape",
        ] {
            assert_eq!(validate_shortcut(raw), Ok(()), "{raw} should be accepted");
        }
    }
}
