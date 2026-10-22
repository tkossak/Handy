#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use x11rb::connection::Connection;
use x11rb::protocol::xkb::{ConnectionExt as _, ID};
use x11rb::protocol::xproto::{
    ConnectionExt as _, CreateWindowAux, GrabMode, InputFocus, ModMask, WindowClass,
    KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

struct XServer(Child);

impl Drop for XServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn keycode(conn: &RustConnection, keysym: u32) -> u8 {
    let setup = conn.setup();
    let mapping = conn
        .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)
        .unwrap()
        .reply()
        .unwrap();
    let index = mapping
        .keysyms
        .chunks(usize::from(mapping.keysyms_per_keycode))
        .position(|symbols| symbols.contains(&keysym))
        .unwrap();
    setup.min_keycode + index as u8
}

fn chord(conn: &RustConnection, root: u32, modifier: u8, key: u8, hotkey: HotKey) {
    for (event_type, code) in [
        (KEY_PRESS_EVENT, modifier),
        (KEY_PRESS_EVENT, key),
        (KEY_RELEASE_EVENT, key),
        (KEY_RELEASE_EVENT, modifier),
    ] {
        conn.xtest_fake_input(event_type, code, 0, root, 0, 0, 0)
            .unwrap()
            .check()
            .unwrap();
    }
    for state in [HotKeyState::Pressed, HotKeyState::Released] {
        assert_eq!(
            GlobalHotKeyEvent::receiver()
                .recv_timeout(Duration::from_secs(2))
                .unwrap_or_else(|error| panic!(
                    "{} {state:?} was not delivered: {error}",
                    hotkey.into_string()
                )),
            GlobalHotKeyEvent {
                id: hotkey.id(),
                state
            }
        );
    }
    assert!(GlobalHotKeyEvent::receiver().try_recv().is_err());
}

#[test]
#[ignore = "Requires Xvfb and xmodmap; runs exclusively on its own X server"]
fn shortcuts_ignore_lock_state_and_release_all_grabs() {
    let original_display = std::env::var_os("DISPLAY");
    for scroll_slot in [ModMask::M3, ModMask::M5] {
        let mut server = XServer(
            Command::new("Xvfb")
                .args([
                    "-displayfd",
                    "1",
                    "-screen",
                    "0",
                    "640x480x24",
                    "-nolisten",
                    "tcp",
                    "-noreset",
                ])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut display_number = String::new();
        BufReader::new(server.0.stdout.take().unwrap())
            .read_line(&mut display_number)
            .unwrap();
        let display = format!(":{}", display_number.trim());
        let scroll_mapping = if scroll_slot == ModMask::M3 {
            "add mod3 = Scroll_Lock"
        } else {
            "add mod5 = Scroll_Lock"
        };
        assert!(Command::new("xmodmap")
            .env("DISPLAY", &display)
            .args(["-e", "clear mod3", "-e", "clear mod5", "-e", scroll_mapping])
            .status()
            .unwrap()
            .success());
        std::env::set_var("DISPLAY", &display);
        let (conn, screen) = RustConnection::connect(Some(&display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let window = conn.generate_id().unwrap();
        conn.create_window(
            0,
            window,
            root,
            0,
            0,
            320,
            240,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new(),
        )
        .unwrap()
        .check()
        .unwrap();
        conn.map_window(window).unwrap().check().unwrap();
        conn.set_input_focus(InputFocus::PARENT, window, 0u32)
            .unwrap()
            .check()
            .unwrap();
        conn.xkb_use_extension(1, 0).unwrap().reply().unwrap();
        let manager = GlobalHotKeyManager::new().unwrap();
        let hotkeys = [
            (
                HotKey::new(Some(Modifiers::CONTROL), Code::Backquote),
                ModMask::CONTROL,
                xkeysym::key::Control_L,
            ),
            (
                HotKey::new(Some(Modifiers::ALT), Code::Backquote),
                ModMask::M1,
                xkeysym::key::Alt_L,
            ),
        ];
        for (hotkey, _, _) in hotkeys {
            manager.register(hotkey).unwrap();
        }
        let key = keycode(&conn, xkeysym::key::grave);
        let all_locks = ModMask::LOCK | ModMask::M2 | scroll_slot;
        for bits in 0..8 {
            let locks = ModMask::from(
                (if bits & 1 != 0 {
                    u16::from(ModMask::LOCK)
                } else {
                    0
                }) | (if bits & 2 != 0 {
                    u16::from(ModMask::M2)
                } else {
                    0
                }) | (if bits & 4 != 0 {
                    u16::from(scroll_slot)
                } else {
                    0
                }),
            );
            conn.xkb_latch_lock_state(
                ID::USE_CORE_KBD.into(),
                all_locks,
                locks,
                false,
                0.into(),
                ModMask::default(),
                false,
                0,
            )
            .unwrap()
            .check()
            .unwrap();
            assert_eq!(
                conn.xkb_get_state(ID::USE_CORE_KBD.into())
                    .unwrap()
                    .reply()
                    .unwrap()
                    .locked_mods,
                locks
            );
            for (hotkey, _, modifier_sym) in hotkeys {
                chord(&conn, root, keycode(&conn, modifier_sym), key, hotkey);
            }
            println!("Scroll slot {scroll_slot:?}, lock mask {locks:?}: Ctrl+` and Alt+` pressed/released");
        }
        for (hotkey, mods, _) in hotkeys {
            manager.unregister(hotkey).unwrap();
            // A subsequent registration round trip flushes queued XUngrabKey requests.
            let barrier = HotKey::new(None, Code::F12);
            manager.register(barrier).unwrap();
            for bits in 0..=u16::from(all_locks) {
                if bits & !u16::from(all_locks) == 0 {
                    let mask = mods | ModMask::from(bits);
                    conn.grab_key(false, root, mask, key, GrabMode::ASYNC, GrabMode::ASYNC)
                        .unwrap()
                        .check()
                        .expect("unregister must release every lock combination");
                    conn.ungrab_key(key, root, mask).unwrap().check().unwrap();
                }
            }
            manager.unregister(barrier).unwrap();
            manager.register(hotkey).unwrap();
        }
        for (hotkey, _, modifier_sym) in hotkeys {
            chord(&conn, root, keycode(&conn, modifier_sym), key, hotkey);
            manager.unregister(hotkey).unwrap();
        }
        drop(manager);
    }
    if let Some(display) = original_display {
        std::env::set_var("DISPLAY", display);
    } else {
        std::env::remove_var("DISPLAY");
    }
}
