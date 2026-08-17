//! With the `vcs` feature, drive a Session over a minimal Atari VCS ROM through
//! the same factory the server uses, exercising the sub-instruction tick seam
//! and the two controller jacks a VCS is the only current core to have.

#![cfg(feature = "vcs")]

use std::path::Path;
use std::time::Duration;

use missingno_core::TvStandard;
use missingno_core::inspect::WatchTerm;
use missingno_core::launch::LaunchValues;
use missingno_core::ports::PeripheralId;
use missingno_core::recording::{EventKind, Recording};
use missingno_core::system::SystemConsole;
use missingno_core::video::DisplayTechnology;
use missingno_session::factory::LoadError;
use missingno_session::{Session, SharedSession, factory};
use missingno_vcs::debug::{JOYSTICK, LEFT_PORT, PADDLES, RIGHT_PORT};

fn value_term(key: &str, value: u32) -> WatchTerm {
    WatchTerm {
        key: key.to_string(),
        address: None,
        value: Some(value),
    }
}

/// A 4 KiB ROM whose reset vector points at its origin ($F000). The bytes
/// there decode to whatever; the beam advances per colour clock regardless of
/// what the CPU executes.
fn minimal_rom() -> Vec<u8> {
    let mut rom = vec![0xEA; 0x1000]; // NOPs
    rom[0xFFC] = 0x00;
    rom[0xFFD] = 0xF0;
    rom
}

/// A 4 KiB ROM that jumps to its own origin forever. The three-cycle loop does
/// not divide the scanline, so instruction boundaries walk the frame — a state
/// save, and so a recording, can start.
fn looping_rom() -> Vec<u8> {
    let mut rom: Vec<u8> = (0..0x1000).map(|i| [0x4C, 0x00, 0xF0][i % 3]).collect();
    rom[0xFFC] = 0x00;
    rom[0xFFD] = 0xF0;
    rom
}

fn console_from(rom: &[u8]) -> Box<dyn SystemConsole> {
    factory::create_console(Path::new("test.a26"), rom)
        .expect("vcs factory should claim an .a26 ROM")
}

fn console() -> Box<dyn SystemConsole> {
    console_from(&minimal_rom())
}

#[test]
fn a_board_no_cartridge_answers_to_is_refused() {
    let mut launch = LaunchValues::default();
    launch.set_choice("board", "F9");
    let Err(error) = factory::create_console_with(Path::new("test.a26"), &minimal_rom(), &launch)
    else {
        panic!("no board is catalogued as F9");
    };
    assert!(matches!(error, LoadError::InvalidValue { .. }));
}

#[test]
fn a_stated_standard_is_the_one_the_console_decodes_for() {
    let mut launch = LaunchValues::default();
    launch.set_choice("tv-standard", "pal");
    let console = factory::create_console_with(Path::new("test.a26"), &minimal_rom(), &launch)
        .expect("vcs factory should claim an .a26 ROM");
    assert!(matches!(
        console.video_out(),
        DisplayTechnology::Crt {
            standard: TvStandard::Pal,
            ..
        }
    ));
}

fn session() -> Session {
    Session::new(console().into_debugger())
}

/// The beam position from the running-status video summary ("beam N · line M").
fn beam(session: &Session) -> u32 {
    let summary = session.running_status().video_summary;
    let rest = summary
        .strip_prefix("beam ")
        .expect("summary starts with the beam position");
    let number = rest.split(' ').next().expect("a beam number");
    number.parse().expect("beam is numeric")
}

#[test]
fn watchables_list_the_pc_and_cart_bank_keys() {
    let session = session();
    let keys: Vec<&str> = session.watchables().iter().map(|w| w.key).collect();
    assert!(keys.contains(&"pc"));
    assert!(keys.contains(&"cart-bank"));
}

#[test]
fn compound_pc_bank_watch_round_trips() {
    let mut session = session();
    let compound = vec![value_term("pc", 0xF006), value_term("cart-bank", 1)];
    let added = session
        .add_watch(compound.clone())
        .expect("compound validates against the watchables");
    assert!(session.watches().contains(&added));
    session
        .remove_watch(compound)
        .expect("removes the compound");
    assert!(session.watches().is_empty());
}

#[test]
fn vcs_advertises_a_colour_clock_tick_that_steps_one_clock() {
    let mut session = session();
    assert_eq!(session.tick_name(), Some("colour clock"));

    // One colour clock advances the beam by exactly one, wrapping at line end.
    let before = beam(&session);
    session.step_tick();
    let after = beam(&session);
    assert!(
        after == before + 1 || after < before,
        "beam {before} -> {after} should advance by one colour clock"
    );
}

#[test]
fn plugging_swaps_what_a_port_carries() {
    let session = SharedSession::spawn_console(console());
    let client = session.handle();

    let surfaces = client.control_surfaces();
    assert_eq!(surfaces.ports.len(), 2, "the VCS has two controller jacks");
    assert_eq!(
        surfaces.ports[0].plugged,
        Some(JOYSTICK),
        "a VCS powers on with joysticks in both jacks"
    );
    assert!(
        surfaces
            .panel
            .iter()
            .any(|control| control.toggle().is_some()),
        "the console panel carries latching switches"
    );

    client
        .plug(LEFT_PORT, PADDLES)
        .expect("the left jack takes a paddle pair");
    assert_eq!(client.control_surfaces().ports[0].plugged, Some(PADDLES));
    assert_eq!(
        client.control_surfaces().ports[1].plugged,
        Some(JOYSTICK),
        "the other jack is untouched"
    );

    assert!(
        client.plug(RIGHT_PORT, PeripheralId(9)).is_err(),
        "a peripheral the jack does not accept is refused"
    );
}

#[test]
fn a_plug_during_capture_is_recorded_and_replayed() {
    let rom = looping_rom();
    let session = SharedSession::spawn_console(console_from(&rom));
    let client = session.handle();
    let path = std::env::temp_dir().join(format!("missingno-vcs-plug-{}.mprc", std::process::id()));

    client
        .start_recording(path.clone())
        .expect("the VCS has a save-state backend to seed the recording");
    // A recording seeds itself from a boundary state, which a frame boundary
    // need not be — so wait for the capture the run loop starts.
    client.run();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !client.is_recording() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(client.is_recording(), "the capture started");
    client
        .plug(LEFT_PORT, PADDLES)
        .expect("plugged while the capture runs");
    std::thread::sleep(Duration::from_millis(80));
    client.pause();
    client.stop_recording().expect("the recording writes out");

    let bytes = std::fs::read(&path).expect("recording file exists");
    let recording = Recording::from_bytes(&bytes).expect("a well-formed recording");
    assert!(
        recording.ports.contains(&(LEFT_PORT, JOYSTICK)),
        "the header states the configuration capture began with: {:?}",
        recording.ports
    );
    assert!(
        recording.events.iter().any(|event| matches!(
            event.kind,
            EventKind::Plug { port, peripheral } if port == LEFT_PORT && peripheral == PADDLES
        )),
        "the swap was captured: {:?}",
        recording.events
    );

    // Replaying reproduces both the header configuration and the swap.
    let mut replayed = console_from(&rom);
    missingno_core::recording::replay(&recording, replayed.as_mut())
        .expect("the recorded timeline replays deterministically");
    assert_eq!(replayed.plugged(LEFT_PORT), Some(PADDLES));

    let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "tools")]
#[test]
fn list_ports_spells_controls_the_way_set_control_takes_them() {
    use missingno_session::tools;
    use serde_json::json;

    let session = SharedSession::spawn_console(console());
    let handle = session.handle();
    let outcome = tools::call_session_tool(&handle, "Atari VCS", "list_ports", &json!({}))
        .expect("the machine surface serves list_ports")
        .expect("list_ports answers");
    let body = match outcome.first() {
        Some(tools::Content::Text(text)) => text.clone(),
        _ => panic!("list_ports answers with text"),
    };
    let listing: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");

    let left = &listing["ports"][0];
    assert_eq!(left["site"], json!("port0"));
    assert_eq!(left["plugged"], json!(JOYSTICK.0));
    let joystick = left["options"]
        .as_array()
        .expect("options")
        .iter()
        .find(|option| option["id"] == json!(JOYSTICK.0))
        .expect("the jack accepts a joystick");
    assert_eq!(joystick["provider"], json!("console"));
    let fire = joystick["controls"]
        .as_array()
        .expect("controls")
        .iter()
        .find(|control| control["role"] == json!("action0"))
        .expect("a joystick has a fire button");
    assert_eq!(fire["site"], json!("port0"));
    assert_eq!(fire["kind"], json!("button"));

    // A panel toggle states its two positions and its power-on level.
    let toggle = listing["panel_controls"]
        .as_array()
        .expect("panel controls")
        .iter()
        .find(|control| control["behaviour"] == json!("toggle"))
        .expect("the VCS panel latches its difficulty and TV-type switches");
    assert_eq!(toggle["site"], json!("panel"));
    assert!(toggle["positions"].as_array().is_some_and(|p| p.len() == 2));
    assert!(toggle["default_high"].is_boolean());

    // The plug tool takes the ids the listing reports, by number or by label.
    tools::call_session_tool(
        &handle,
        "Atari VCS",
        "plug",
        &json!({ "port": "port0", "peripheral": PADDLES.0 }),
    )
    .expect("the machine surface serves plug")
    .expect("paddles plug into the left jack");
    assert_eq!(handle.control_surfaces().ports[0].plugged, Some(PADDLES));
}
