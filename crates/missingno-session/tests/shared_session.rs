//! With the `gb` feature, exercise the shared session component over a minimal
//! Game Boy ROM: command serialization, run/pause transitions, the breakpoint
//! stop reason, readout honesty across the run boundary, and a recording
//! round-trip — all through the client handle, no transport involved.

#![cfg(feature = "gb")]

use std::path::Path;
use std::time::{Duration, Instant};

use missingno_session::{SessionEvent, SessionHandle, SharedSession, StopReason, factory};

/// A 32 KiB all-NOP ROM: the `.gb` extension makes the registry claim it, and
/// the DMG core boots to PC 0x0100 and marches NOPs from there.
fn shared() -> SharedSession {
    let rom = vec![0x00u8; 0x8000];
    let console = factory::create_console(Path::new("test.gb"), &rom)
        .expect("factory should not error")
        .expect("gb factory should claim a .gb ROM");
    let debugger = console
        .into_debugger()
        .ok()
        .expect("gb has a debugger backend");
    SharedSession::spawn(debugger)
}

/// Spin until `cond` holds or the deadline passes; returns whether it held.
fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    cond()
}

#[test]
fn all_access_flows_through_the_handle() {
    let session = shared();
    let client = session.handle();
    // A readout across the channel reaches the owned core.
    assert_eq!(client.with_session(|s| s.pc()), 0x0100);
    // A command across the channel mutates it, visible to the next readout.
    client.with_session(|s| s.set_breakpoint(0x0150)).unwrap();
    assert!(client.with_session(|s| s.breakpoints().contains(&0x0150)));
}

#[test]
fn commands_from_many_clients_serialize() {
    let session = shared();
    // Several client handles, each on its own thread, set a distinct breakpoint.
    // The single request channel serializes them, so none is lost.
    let handles: Vec<SessionHandle> = (0..4).map(|_| session.handle()).collect();
    let threads: Vec<_> = handles
        .into_iter()
        .enumerate()
        .map(|(i, client)| {
            std::thread::spawn(move || {
                for step in 0..8u32 {
                    let address = 0x0100 + (i as u32) * 0x0100 + step;
                    client
                        .with_session(move |s| s.set_breakpoint(address))
                        .unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let breakpoints = session.handle().with_session(|s| s.breakpoints());
    assert_eq!(breakpoints.len(), 4 * 8, "every serialized set survived");
}

#[test]
fn run_and_pause_transition_state() {
    let session = shared();
    let client = session.handle();
    assert!(!client.is_running(), "starts paused");

    client.run();
    assert!(wait_until(|| client.is_running()), "run starts the loop");
    // The loop publishes a running status as it steps.
    assert!(wait_until(|| client.latest_status().is_some()));

    client.pause();
    assert!(!client.is_running(), "pause halts synchronously");
    // A readout serves directly from the owned core once paused.
    assert!(client.with_session(|s| s.pc()) >= 0x0100);
}

#[test]
fn breakpoint_stop_records_the_reason() {
    let session = shared();
    let client = session.handle();
    // The NOP march reaches 0x0105 within the first frame, so the run loop stops
    // almost at once.
    client.with_session(|s| s.set_breakpoint(0x0105)).unwrap();
    client.run();

    // Wait for the stop reason itself, not the running flag — the loop may not be
    // observed running before so prompt a breakpoint fires.
    assert!(
        wait_until(|| matches!(
            client.with_session(|s| s.last_stop().clone()),
            StopReason::Breakpoint
        )),
        "the breakpoint stop reason is published"
    );
    assert!(!client.is_running(), "the breakpoint halted the loop");
    assert_eq!(
        client.with_session(|s| s.pc()),
        0x0105,
        "stopped on the breakpoint address"
    );
}

#[test]
fn readouts_are_honest_across_the_run_boundary() {
    let session = shared();
    let client = session.handle();

    // Paused: a full memory read serves live from the owned core.
    assert_eq!(client.read_memory(0x0100, 4).unwrap(), vec![0, 0, 0, 0]);

    // Running with no interest window set: the published snapshot cannot honestly
    // answer an arbitrary span, so the read is refused rather than touching the
    // live core mid-run.
    client.run();
    assert!(wait_until(|| client.is_running()));
    assert!(
        client.read_memory(0xC000, 4).is_err(),
        "an uncovered span is refused while running"
    );

    // Paused again: the live read is available once more.
    client.pause();
    assert_eq!(client.read_memory(0x0100, 4).unwrap(), vec![0, 0, 0, 0]);
}

#[test]
fn interest_windows_publish_while_running() {
    let session = shared();
    let client = session.handle();
    client.set_memory_interest(vec![missingno_session::MemoryInterest {
        start: 0xC000,
        len: 16,
    }]);
    client.run();
    // Once a frame completes, the interest span is peeked into the window slot,
    // and the honest read is answered from it.
    assert!(wait_until(|| !client.latest_memory_windows().is_empty()));
    assert!(client.read_memory(0xC000, 4).is_ok());
    client.pause();
}

#[test]
fn recording_round_trips_through_a_file() {
    use missingno_core::recording::Recording;

    let session = shared();
    let client = session.handle();

    let path = std::env::temp_dir().join(format!("missingno-s1-{}.mprc", std::process::id()));
    client
        .start_recording(path.clone())
        .expect("gb has a save-state backend to seed the recording");

    // Free-run briefly so the loop notes some frames, then finalize.
    client.run();
    std::thread::sleep(Duration::from_millis(120));
    client.pause();
    client.stop_recording().expect("the recording writes out");

    // The file parses, and a fresh session replays it without divergence.
    let bytes = std::fs::read(&path).expect("recording file exists");
    let recording = Recording::from_bytes(&bytes).expect("a well-formed recording");
    assert!(recording.frames >= 1, "at least one frame was captured");

    let mut replay = session_engine();
    replay
        .replay_recording(&path)
        .expect("the recorded timeline replays deterministically");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn recording_and_replay_refuse_each_other() {
    let session = shared();
    let client = session.handle();
    let path = std::env::temp_dir().join(format!("missingno-excl-{}.mprc", std::process::id()));

    // A replay's inputs bypass the capture, so neither may wrap the other:
    // recording refuses mid-replay, and replay refuses mid-recording.
    client
        .start_recording(path.clone())
        .expect("gb has a save-state backend");
    client.run();
    std::thread::sleep(Duration::from_millis(60));
    client.pause();
    client.stop_recording().expect("the recording writes out");

    client
        .play_recording(path.clone())
        .expect("the recording replays");
    assert!(
        client.start_recording(path.clone()).is_err(),
        "recording must refuse while a replay is running"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn stopping_without_a_recording_is_silent() {
    let session = shared();
    assert!(
        session.handle().stop_recording().is_ok(),
        "finalizing nothing is a no-op, not an error"
    );
}

/// A console-only shared session over the same ROM — the plain-emulator host,
/// with no debugger inspection surface.
fn shared_console() -> SharedSession {
    let rom = vec![0x00u8; 0x8000];
    let console = factory::create_console(Path::new("test.gb"), &rom)
        .expect("factory should not error")
        .expect("gb factory should claim a .gb ROM");
    SharedSession::spawn_console(console)
}

/// Drain whatever session events have arrived without blocking.
fn drain(rx: &std::sync::mpsc::Receiver<SessionEvent>) -> Vec<SessionEvent> {
    rx.try_iter().collect()
}

#[test]
fn console_session_runs_and_publishes_frames() {
    let session = shared_console();
    let client = session.handle();
    assert!(!client.is_debugger(), "a console session hosts no debugger");

    client.run();
    assert!(
        wait_until(|| client.is_running()),
        "the console loop starts"
    );
    assert!(
        wait_until(|| client.latest_frame().is_some()),
        "the console loop publishes frames"
    );
    client.pause();
    assert!(!client.is_running());
}

#[test]
fn a_subscriber_sees_the_stop_event() {
    let session = shared();
    let client = session.handle();
    let events = client.subscribe();
    client.with_session(|s| s.set_breakpoint(0x0105)).unwrap();
    client.run();

    assert!(
        wait_until(|| drain(&events)
            .iter()
            .any(|e| matches!(e, SessionEvent::Stopped))),
        "the breakpoint stop reaches a subscriber"
    );
}

#[test]
fn saving_reports_a_notice_and_the_file_round_trips() {
    let session = shared();
    let client = session.handle();
    let events = client.subscribe();

    let path = std::env::temp_dir().join(format!("missingno-s2-{}.mpsv", std::process::id()));
    client.save_state(path.clone()).expect("a paused save answers");
    assert!(
        wait_until(|| drain(&events).iter().any(|e| matches!(
            e,
            SessionEvent::Notice(message) if message == "State saved"
        ))),
        "a save reports its outcome as a notice"
    );
    assert!(path.exists(), "the save file was written");

    // The saved state loads back without error.
    client.load_state(path.clone()).expect("the save loads back");
    assert!(wait_until(|| drain(&events).iter().any(|e| matches!(
        e,
        SessionEvent::Notice(message) if message == "State loaded"
    ))));
    let _ = std::fs::remove_file(&path);
}

/// A console session has no debugger, but it does have a state backend: saving
/// and loading are answered by the command queue, so both machine kinds serve
/// them.
#[test]
fn a_console_session_saves_and_loads_state() {
    let session = shared_console();
    let client = session.handle();

    let path = std::env::temp_dir().join(format!("missingno-s4-{}.mpsv", std::process::id()));
    client
        .save_state(path.clone())
        .expect("a console session saves state");
    assert!(path.exists());
    client
        .load_state(path.clone())
        .expect("a console session loads state");
    let _ = std::fs::remove_file(&path);
}

/// A save requested while the machine free-runs is answered by the run loop,
/// waiting out an off-boundary request rather than reporting a state it did not
/// capture.
#[test]
fn a_running_save_is_answered_by_the_run_loop() {
    let session = shared();
    let client = session.handle();
    client.run();

    let path = std::env::temp_dir().join(format!("missingno-s4-run-{}.mpsv", std::process::id()));
    let outcome = client.save_state(path.clone());
    client.pause();
    assert_eq!(outcome, Ok(()), "a running save answers its requester");
    assert!(path.exists(), "the save file was written");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn screenshot_captures_the_current_frame() {
    let session = shared();
    let client = session.handle();
    assert!(
        client.screenshot().is_some(),
        "the current display frame is captured on request"
    );
}

#[test]
fn into_machine_hands_back_the_hosted_debugger() {
    use missingno_session::ExtractedMachine;

    // A debugger session hands back a debugger; the frontend re-hosts it (or its
    // console) in a session of the other kind to toggle the debugger.
    let session = shared();
    match session.into_machine() {
        Some(ExtractedMachine::Debugger(debugger)) => {
            // The returned machine is live — its console still answers a peek.
            let _ = debugger.peek(0x0100);
        }
        _ => panic!("a debugger session hands back a debugger machine"),
    }
}

#[test]
fn into_machine_hands_back_a_plain_console() {
    use missingno_session::ExtractedMachine;

    let rom = vec![0x00u8; 0x8000];
    let console = factory::create_console(Path::new("test.gb"), &rom)
        .expect("factory should not error")
        .expect("gb factory should claim a .gb ROM");
    let session = SharedSession::spawn_console(console);
    assert!(
        matches!(session.into_machine(), Some(ExtractedMachine::Console(_))),
        "a console session hands back a plain console machine"
    );
}

/// A plain `Session` over the same ROM, for replaying a recording against.
fn session_engine() -> missingno_session::Session {
    let rom = vec![0x00u8; 0x8000];
    let console = factory::create_console(Path::new("test.gb"), &rom)
        .expect("factory should not error")
        .expect("gb factory should claim a .gb ROM");
    missingno_session::Session::new(console.into_debugger().ok().expect("gb has a debugger"))
}
