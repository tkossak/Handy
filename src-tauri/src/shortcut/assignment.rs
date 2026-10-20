//! Transactional changes to saved shortcut ownership and physical registrations.

use std::collections::HashMap;

use crate::settings::{self, KeyboardImplementation, ShortcutBinding};

/// System boundary: physical shortcut registrations and durable settings storage.
pub(super) trait BindingEnvironment {
    fn registered(&mut self, id: &str) -> Result<Option<ShortcutBinding>, String>;
    fn register(&mut self, binding: &ShortcutBinding) -> Result<(), String>;
    fn unregister(&mut self, binding: &ShortcutBinding) -> Result<(), String>;
    fn persist(&mut self, bindings: &HashMap<String, ShortcutBinding>) -> Result<(), String>;
}

pub(super) fn normalize_chord(
    raw: &str,
    implementation: KeyboardImplementation,
) -> Result<String, String> {
    if raw.trim().is_empty() {
        return Ok(String::new());
    }
    match implementation {
        KeyboardImplementation::Tauri => super::tauri_impl::normalize_shortcut(raw),
        KeyboardImplementation::HandyKeys => super::handy_keys::normalize_shortcut(raw),
    }
}

/// Transfer ownership, replace physical registrations, then durably commit.
/// Failure restores precisely the registrations present before the transaction,
/// not all saved bindings (which may be inactive or suspended for key capture).
pub(super) fn change_binding_transaction(
    settings: &mut settings::AppSettings,
    id: &str,
    raw: &str,
    environment: &mut impl BindingEnvironment,
) -> Result<ShortcutBinding, String> {
    let previous = settings::get_stored_binding(settings, id)?;
    let normalized = normalize_chord(raw, settings.keyboard_implementation)?;
    let mut owners: Vec<_> = settings
        .bindings
        .values()
        .filter(|binding| {
            binding.id != id
                && !normalized.is_empty()
                && normalize_chord(&binding.current_binding, settings.keyboard_implementation)
                    .is_ok_and(|chord| chord == normalized)
        })
        .cloned()
        .collect();
    owners.sort_by(|a, b| a.id.cmp(&b.id));
    if !owners.is_empty() && !matches!(id, "transcribe_language_1" | "transcribe_language_2") {
        return Err(format!(
            "Shortcut is already assigned to '{}'",
            owners[0].name
        ));
    }
    if owners.is_empty()
        && normalize_chord(&previous.current_binding, settings.keyboard_implementation)
            .is_ok_and(|chord| chord == normalized)
    {
        return Ok(previous);
    }

    let mut next = settings.bindings.clone();
    for owner in &owners {
        next.get_mut(&owner.id)
            .expect("owner is a saved binding")
            .current_binding
            .clear();
    }
    let mut updated = previous.clone();
    updated.current_binding = normalized;
    next.insert(id.to_string(), updated.clone());

    let mut registrations = Vec::new();
    for binding in std::iter::once(&previous).chain(owners.iter()) {
        if let Some(registered) = environment.registered(&binding.id)? {
            registrations.push(registered);
        }
    }
    let mut removed = Vec::new();
    for registered in &registrations {
        if let Err(error) = environment.unregister(registered) {
            return Err(restore_registrations(environment, &removed, error));
        }
        removed.push(registered.clone());
    }

    // Cancel remains lifecycle-managed: changing an idle assignment cannot grab it.
    let activate = if id == "cancel" {
        !updated.current_binding.is_empty() && registrations.iter().any(|binding| binding.id == id)
    } else {
        super::binding_is_enabled(settings, &updated)
    };
    if activate {
        if let Err(error) = environment.register(&updated) {
            return Err(restore_registrations(environment, &removed, error));
        }
    }
    if let Err(mut error) = environment.persist(&next) {
        if activate {
            if let Err(rollback) = environment.unregister(&updated) {
                error.push_str(&format!(
                    "; could not remove replacement shortcut: {rollback}"
                ));
            }
        }
        return Err(restore_registrations(environment, &removed, error));
    }
    settings.bindings = next;
    Ok(updated)
}

fn restore_registrations(
    environment: &mut impl BindingEnvironment,
    registrations: &[ShortcutBinding],
    mut error: String,
) -> String {
    for registered in registrations {
        if let Err(rollback) = environment.register(registered) {
            error.push_str(&format!(
                "; could not restore '{}': {rollback}",
                registered.id
            ));
        }
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{get_default_settings, RecognitionMode};
    use std::collections::HashMap;

    // Only OS registrations and durable storage are substituted. Chord parsing,
    // activation policy, ownership transfer and rollback use the production path.
    struct SystemBoundary {
        registrations: HashMap<String, ShortcutBinding>,
        implementation: KeyboardImplementation,
        memory: Option<serde_json::Value>,
        disk: Option<serde_json::Value>,
        external_chord: Option<String>,
        fail_registration: bool,
        fail_persistence: bool,
        fail_unregistration: Option<String>,
    }

    impl SystemBoundary {
        fn new(settings: &settings::AppSettings) -> Self {
            Self {
                registrations: settings
                    .bindings
                    .iter()
                    .filter(|(id, binding)| {
                        id.as_str() != "cancel"
                            && super::super::binding_is_enabled(settings, binding)
                    })
                    .map(|(id, binding)| (id.clone(), binding.clone()))
                    .collect(),
                implementation: settings.keyboard_implementation,
                memory: Some(serde_json::to_value(settings).unwrap()),
                disk: Some(serde_json::to_value(settings).unwrap()),
                external_chord: None,
                fail_registration: false,
                fail_persistence: false,
                fail_unregistration: None,
            }
        }

        fn owner(&self, chord: &str) -> Option<&str> {
            self.registrations
                .values()
                .find(|binding| {
                    normalize_chord(&binding.current_binding, self.implementation).unwrap()
                        == normalize_chord(chord, self.implementation).unwrap()
                })
                .map(|binding| binding.id.as_str())
        }

        fn saved(&self) -> settings::AppSettings {
            serde_json::from_value(self.disk.clone().unwrap()).unwrap()
        }
    }

    impl BindingEnvironment for SystemBoundary {
        fn registered(&mut self, id: &str) -> Result<Option<ShortcutBinding>, String> {
            Ok(self.registrations.get(id).cloned())
        }

        fn register(&mut self, binding: &ShortcutBinding) -> Result<(), String> {
            if self.fail_registration {
                self.fail_registration = false;
                return Err("OS registration failed".into());
            }
            if self.external_chord.as_ref().is_some_and(|chord| {
                normalize_chord(chord, self.implementation).unwrap() == binding.current_binding
            }) || self.owner(&binding.current_binding).is_some()
            {
                return Err("Chord is occupied".into());
            }
            self.registrations
                .insert(binding.id.clone(), binding.clone());
            Ok(())
        }

        fn unregister(&mut self, binding: &ShortcutBinding) -> Result<(), String> {
            if self.fail_unregistration.as_deref() == Some(binding.id.as_str()) {
                return Err("OS unregistration failed".into());
            }
            self.registrations.remove(&binding.id);
            Ok(())
        }

        fn persist(&mut self, bindings: &HashMap<String, ShortcutBinding>) -> Result<(), String> {
            settings::commit_bindings_to_store(self, bindings)
        }
    }

    impl settings::BindingSettingsStore for SystemBoundary {
        fn settings(&self) -> Option<serde_json::Value> {
            self.memory.clone()
        }

        fn set_settings(&mut self, settings: Option<serde_json::Value>) {
            self.memory = settings;
        }

        fn save(&mut self) -> Result<(), String> {
            if self.fail_persistence {
                self.fail_persistence = false;
                return Err("Disk is read-only".into());
            }
            self.disk = self.memory.clone();
            Ok(())
        }
    }

    fn configured(implementation: KeyboardImplementation) -> settings::AppSettings {
        let mut settings = get_default_settings();
        settings.keyboard_implementation = implementation;
        settings.recognition_language.mode = RecognitionMode::Pair;
        settings
            .bindings
            .get_mut("transcribe")
            .unwrap()
            .current_binding = "ctrl+space".into();
        settings
            .bindings
            .get_mut("transcribe_language_1")
            .unwrap()
            .current_binding = "ctrl+1".into();
        settings
            .bindings
            .get_mut("transcribe_language_2")
            .unwrap()
            .current_binding = "ctrl+2".into();
        settings
    }

    #[test]
    fn language_slot_takes_active_chord_and_releases_its_previous_chord() {
        for implementation in [
            KeyboardImplementation::Tauri,
            KeyboardImplementation::HandyKeys,
        ] {
            let mut settings = configured(implementation);
            let recognition = serde_json::to_value(&settings.recognition_language).unwrap();
            let mut system = SystemBoundary::new(&settings);

            change_binding_transaction(
                &mut settings,
                "transcribe_language_1",
                "Space+Control",
                &mut system,
            )
            .unwrap();

            assert_eq!(system.owner("ctrl+space"), Some("transcribe_language_1"));
            assert_eq!(system.owner("ctrl+1"), None);
            assert_eq!(settings.bindings["transcribe"].current_binding, "");
            assert_eq!(system.saved().bindings["transcribe"].current_binding, "");
            assert_eq!(
                serde_json::to_value(&settings.recognition_language).unwrap(),
                recognition
            );
            assert_eq!(
                settings.bindings["transcribe_language_2"].current_binding,
                "ctrl+2"
            );
        }
    }

    #[test]
    fn failed_registration_restores_both_owners_without_taking_external_chords() {
        for implementation in [
            KeyboardImplementation::Tauri,
            KeyboardImplementation::HandyKeys,
        ] {
            for external in [false, true] {
                let mut settings = configured(implementation);
                let before = serde_json::to_value(&settings).unwrap();
                let mut system = SystemBoundary::new(&settings);
                if external {
                    system.external_chord = Some("ctrl+9".into());
                } else {
                    system.fail_registration = true;
                }

                assert!(change_binding_transaction(
                    &mut settings,
                    "transcribe_language_1",
                    if external { "ctrl+9" } else { "ctrl+space" },
                    &mut system
                )
                .is_err());

                assert_eq!(serde_json::to_value(&settings).unwrap(), before);
                assert_eq!(serde_json::to_value(system.saved()).unwrap(), before);
                assert_eq!(system.owner("ctrl+space"), Some("transcribe"));
                assert_eq!(system.owner("ctrl+1"), Some("transcribe_language_1"));
                assert_eq!(system.owner("ctrl+9"), None);
            }
        }
    }

    #[test]
    fn takeover_clears_inactive_owners_without_activating_inactive_slots() {
        for implementation in [
            KeyboardImplementation::Tauri,
            KeyboardImplementation::HandyKeys,
        ] {
            for pair_mode in [false, true] {
                let mut settings = configured(implementation);
                settings.recognition_language.mode = if pair_mode {
                    RecognitionMode::Pair
                } else {
                    RecognitionMode::Single
                };
                settings.post_process_enabled = false;
                settings
                    .bindings
                    .get_mut("transcribe_with_post_process")
                    .unwrap()
                    .current_binding = "alt+shift+space".into();
                let mut system = SystemBoundary::new(&settings);

                change_binding_transaction(
                    &mut settings,
                    "transcribe_language_1",
                    "Space+Shift+Option",
                    &mut system,
                )
                .unwrap();

                assert_eq!(
                    settings.bindings["transcribe_with_post_process"].current_binding,
                    ""
                );
                assert!(!settings.post_process_enabled);
                assert_eq!(
                    system.owner("alt+shift+space"),
                    if pair_mode {
                        Some("transcribe_language_1")
                    } else {
                        None
                    }
                );
                assert_eq!(system.owner("ctrl+space"), Some("transcribe"));
                assert_eq!(system.owner("ctrl+1"), None);
            }
        }
    }

    #[test]
    fn ownership_follows_slots_and_explicit_empties_survive_reload_and_backend_switch() {
        for implementation in [
            KeyboardImplementation::Tauri,
            KeyboardImplementation::HandyKeys,
        ] {
            let mut settings = configured(implementation);
            settings.recognition_language.pair = vec!["pl".into(), "en".into()];
            let mut system = SystemBoundary::new(&settings);
            change_binding_transaction(
                &mut settings,
                "transcribe_language_1",
                "ctrl+space",
                &mut system,
            )
            .unwrap();
            change_binding_transaction(
                &mut settings,
                "transcribe_language_2",
                "ctrl+space",
                &mut system,
            )
            .unwrap();
            let mut reloaded = system.saved();
            reloaded.recognition_language.pair.swap(0, 1);
            reloaded.keyboard_implementation = match implementation {
                KeyboardImplementation::Tauri => KeyboardImplementation::HandyKeys,
                KeyboardImplementation::HandyKeys => KeyboardImplementation::Tauri,
            };
            let restarted = SystemBoundary::new(&reloaded);

            assert_eq!(reloaded.bindings["transcribe"].current_binding, "");
            assert_eq!(
                reloaded.bindings["transcribe_language_1"].current_binding,
                ""
            );
            assert_eq!(restarted.owner("ctrl+space"), Some("transcribe_language_2"));
            assert_eq!(restarted.owner("ctrl+1"), None);
            assert_eq!(restarted.owner("ctrl+2"), None);
        }
    }

    #[test]
    fn normalized_same_owner_is_a_noop_even_if_system_boundaries_would_fail() {
        for implementation in [
            KeyboardImplementation::Tauri,
            KeyboardImplementation::HandyKeys,
        ] {
            let mut settings = configured(implementation);
            settings
                .bindings
                .get_mut("transcribe_language_1")
                .unwrap()
                .current_binding = "ctrl+alt+shift+1".into();
            let before = serde_json::to_value(&settings).unwrap();
            let mut system = SystemBoundary::new(&settings);
            system.fail_registration = true;
            system.fail_persistence = true;
            system.fail_unregistration = Some("transcribe_language_1".into());

            change_binding_transaction(
                &mut settings,
                "transcribe_language_1",
                "1+Shift+Option+Control",
                &mut system,
            )
            .unwrap();

            assert_eq!(serde_json::to_value(&settings).unwrap(), before);
            assert_eq!(serde_json::to_value(system.saved()).unwrap(), before);
            assert_eq!(
                system.owner("ctrl+alt+shift+1"),
                Some("transcribe_language_1")
            );
        }
    }

    #[test]
    fn failed_persistence_rolls_back_both_assignments_and_physical_registrations() {
        for implementation in [
            KeyboardImplementation::Tauri,
            KeyboardImplementation::HandyKeys,
        ] {
            let mut settings = configured(implementation);
            let before = serde_json::to_value(&settings).unwrap();
            let mut system = SystemBoundary::new(&settings);
            system.fail_persistence = true;

            assert!(change_binding_transaction(
                &mut settings,
                "transcribe_language_1",
                "ctrl+space",
                &mut system
            )
            .is_err());

            assert_eq!(serde_json::to_value(&settings).unwrap(), before);
            assert_eq!(serde_json::to_value(system.saved()).unwrap(), before);
            assert_eq!(system.owner("ctrl+space"), Some("transcribe"));
            assert_eq!(system.owner("ctrl+1"), Some("transcribe_language_1"));
        }
    }

    #[test]
    fn invalid_chords_and_unregistration_failure_leave_existing_ownership_untouched() {
        for implementation in [
            KeyboardImplementation::Tauri,
            KeyboardImplementation::HandyKeys,
        ] {
            for invalid in [false, true] {
                let mut settings = configured(implementation);
                let before = serde_json::to_value(&settings).unwrap();
                let mut system = SystemBoundary::new(&settings);
                system.fail_unregistration = Some("transcribe".into());

                assert!(change_binding_transaction(
                    &mut settings,
                    "transcribe_language_1",
                    if invalid {
                        "ctrl+not-a-key"
                    } else {
                        "ctrl+space"
                    },
                    &mut system
                )
                .is_err());

                assert_eq!(serde_json::to_value(&settings).unwrap(), before);
                assert_eq!(serde_json::to_value(system.saved()).unwrap(), before);
                assert_eq!(system.owner("ctrl+space"), Some("transcribe"));
                assert_eq!(system.owner("ctrl+1"), Some("transcribe_language_1"));
            }
        }
    }

    #[test]
    fn normalized_letter_and_digit_assignments_remain_usable_after_backend_switch() {
        for chord in ["Shift+A+Control", "Shift+9+Control"] {
            let mut settings = configured(KeyboardImplementation::Tauri);
            let mut system = SystemBoundary::new(&settings);
            change_binding_transaction(&mut settings, "transcribe_language_1", chord, &mut system)
                .unwrap();
            let mut reloaded = system.saved();
            reloaded.keyboard_implementation = KeyboardImplementation::HandyKeys;
            let mut restarted = SystemBoundary::new(&reloaded);

            change_binding_transaction(
                &mut reloaded,
                "transcribe_language_2",
                chord,
                &mut restarted,
            )
            .unwrap();

            assert_eq!(restarted.owner(chord), Some("transcribe_language_2"));
            assert_eq!(
                reloaded.bindings["transcribe_language_1"].current_binding,
                ""
            );
        }
    }

    #[test]
    fn inactive_language_owner_and_enabled_post_processing_can_both_be_displaced() {
        for implementation in [
            KeyboardImplementation::Tauri,
            KeyboardImplementation::HandyKeys,
        ] {
            for owner in ["transcribe_language_2", "transcribe_with_post_process"] {
                let mut settings = configured(implementation);
                settings.recognition_language.mode = RecognitionMode::Auto;
                settings.post_process_enabled = true;
                settings.bindings.get_mut(owner).unwrap().current_binding = "ctrl+9".into();
                let mut system = SystemBoundary::new(&settings);

                change_binding_transaction(
                    &mut settings,
                    "transcribe_language_1",
                    "9+Control",
                    &mut system,
                )
                .unwrap();

                assert_eq!(settings.bindings[owner].current_binding, "");
                assert_eq!(system.saved().bindings[owner].current_binding, "");
                assert_eq!(system.owner("ctrl+9"), None);
                assert_eq!(system.owner("ctrl+space"), Some("transcribe"));
                assert!(settings.post_process_enabled);
            }
        }
    }

    #[test]
    fn suspended_capture_failure_does_not_reactivate_saved_shortcuts_before_resume() {
        let mut settings = configured(KeyboardImplementation::Tauri);
        let mut system = SystemBoundary::new(&settings);
        system.registrations.clear();
        system.fail_persistence = true;

        assert!(change_binding_transaction(
            &mut settings,
            "transcribe_language_1",
            "ctrl+space",
            &mut system
        )
        .is_err());

        assert!(system.registrations.is_empty());
        let resumed = SystemBoundary::new(&system.saved());
        assert_eq!(resumed.owner("ctrl+space"), Some("transcribe"));
        assert_eq!(resumed.owner("ctrl+1"), Some("transcribe_language_1"));
    }
}
