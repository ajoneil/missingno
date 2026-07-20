//! Input recordings: an initial full machine state plus a timestamped input
//! stream that drives it, replayable deterministically.
//!
//! A recording is the state framing of [`crate::state_file`] (the initial
//! machine state, captured through the save-state seam) followed by an
//! **input trace** — a record sequence in the container vocabulary, kind
//! `input`: each entry is a hardware-named control change stamped with the
//! frame boundary it lands on. Replay restores the initial state and re-applies
//! the input stream at its timestamps while stepping frames; the continuation
//! is deterministic by construction (identical initial state, identical inputs
//! at identical boundaries).
//!
//! ## Why frame index is the timestamp unit
//!
//! A console applies input at frame boundaries: the frontend drains queued
//! control changes, then emulates one frame. Nothing lands input at a finer
//! grain, so the honest timestamp is the frame index — frames elapsed since the
//! initial state — and that is exactly the quantity replay steps in. A finer
//! unit (instruction index) would be false precision the live input path never
//! produces.
//!
//! The recording also carries periodic frame-hash checkpoints, so replay can
//! detect divergence and report the frame it happened on rather than silently
//! continuing.

use std::hash::{Hash, Hasher};

use crate::system::{ControlId, ControlInput, StateError, SystemConsole};
use crate::video::Frame;

/// Magic bytes of a recording file. Distinct from the state file's `MPSV` and
/// the trace container's `MPRK` so the three framings are never confused.
pub const RECORDING_MAGIC: &[u8; 4] = b"MPRC";

/// Recording-container version. A reader rejects any other value outright — the
/// effort-wide breaking posture, regenerate rather than migrate.
pub const RECORDING_VERSION: u8 = 1;

/// One input event: a hardware-named control changed to a value at a frame
/// boundary (frames elapsed since the recording's initial state, applied before
/// that frame is stepped).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputRecord {
    pub frame: u64,
    pub control: ControlId,
    pub input: ControlInput,
}

/// A frame-hash checkpoint, for replay-divergence detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameCheck {
    pub frame: u64,
    pub hash: u64,
}

/// A recording: the initial machine state, the input stream that drives it, and
/// periodic frame-hash checkpoints.
#[derive(Clone, Debug, PartialEq)]
pub struct Recording {
    /// The initial full-state save file ([`crate::state_file`] `MPSV` bytes),
    /// carrying its own system id, ROM fingerprint, and version — so restore
    /// validates the target console for free.
    pub initial_state: Vec<u8>,
    /// The input trace, in ascending frame order.
    pub inputs: Vec<InputRecord>,
    /// Frame-hash checkpoints captured every [`check_interval`](Self::check_interval)
    /// frames.
    pub checks: Vec<FrameCheck>,
    /// Total frames recorded — how many frames replay steps.
    pub frames: u64,
    /// The checkpoint cadence in frames; `0` when no checks were captured.
    pub check_interval: u64,
}

/// Why a recording file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordingError {
    /// The leading bytes are not a recording file.
    BadMagic,
    /// The container version is not the one this build implements.
    UnsupportedVersion(u8),
    /// The data ended before a declared section was complete.
    Truncated,
    /// A field tag was not a known code, a count did not describe the whole
    /// input, or a frame stamp is out of range or out of order.
    BadEncoding,
    /// A length or count exceeded what the 32-bit container framing can carry.
    TooLarge,
}

/// A sanity ceiling on a recording's frame count, so a hostile file cannot ask
/// replay to step an absurd number of frames. Far above any real play session
/// (~19 days at 60 fps), it exists only to reject a garbage value.
pub const MAX_RECORDING_FRAMES: u64 = 100_000_000;

impl std::fmt::Display for RecordingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordingError::BadMagic => f.write_str("not a recording file"),
            RecordingError::UnsupportedVersion(v) => {
                write!(f, "unsupported recording version {v} (regenerate)")
            }
            RecordingError::Truncated => f.write_str("recording file is truncated"),
            RecordingError::BadEncoding => f.write_str("recording file has an unknown encoding"),
            RecordingError::TooLarge => f.write_str("recording is too large to encode"),
        }
    }
}

impl std::error::Error for RecordingError {}

/// The canonical frame hash the recorder and replayer agree on: the resolved
/// RGBA pixels and dimensions. A blank hash (`0`) stands for a frame the step
/// produced nothing for.
pub fn frame_hash(frame: &Frame) -> u64 {
    let rgba = frame.resolve_rgba();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rgba.width.hash(&mut hasher);
    rgba.height.hash(&mut hasher);
    rgba.pixels.hash(&mut hasher);
    hasher.finish()
}

impl Recording {
    /// Serialize into a recording file. Errors ([`RecordingError::TooLarge`]) if
    /// a length overruns the 32-bit framing rather than truncating it.
    pub fn to_bytes(&self) -> Result<Vec<u8>, RecordingError> {
        let len32 = |n: usize| u32::try_from(n).map_err(|_| RecordingError::TooLarge);

        let mut out = Vec::new();
        out.extend_from_slice(RECORDING_MAGIC);
        out.push(RECORDING_VERSION);

        out.extend_from_slice(&len32(self.initial_state.len())?.to_le_bytes());
        out.extend_from_slice(&self.initial_state);

        out.extend_from_slice(&self.frames.to_le_bytes());
        out.extend_from_slice(&self.check_interval.to_le_bytes());

        out.extend_from_slice(&len32(self.inputs.len())?.to_le_bytes());
        for event in &self.inputs {
            out.extend_from_slice(&event.frame.to_le_bytes());
            out.push(event.control.0);
            match event.input {
                ControlInput::Digital(pressed) => {
                    out.push(0);
                    out.push(pressed as u8);
                }
                ControlInput::Axis(value) => {
                    out.push(1);
                    out.extend_from_slice(&value.to_le_bytes());
                }
            }
        }

        out.extend_from_slice(&len32(self.checks.len())?.to_le_bytes());
        for check in &self.checks {
            out.extend_from_slice(&check.frame.to_le_bytes());
            out.extend_from_slice(&check.hash.to_le_bytes());
        }

        Ok(out)
    }

    /// Parse a recording file.
    pub fn from_bytes(bytes: &[u8]) -> Result<Recording, RecordingError> {
        let mut reader = Reader { bytes, pos: 0 };

        if reader.take(4)? != RECORDING_MAGIC {
            return Err(RecordingError::BadMagic);
        }
        let version = reader.u8()?;
        if version != RECORDING_VERSION {
            return Err(RecordingError::UnsupportedVersion(version));
        }

        let state_len = reader.u32()? as usize;
        let initial_state = reader.take(state_len)?.to_vec();

        let frames = reader.u64()?;
        // A frame count above the sanity ceiling is a garbage value — replay
        // would step an absurd number of frames.
        if frames > MAX_RECORDING_FRAMES {
            return Err(RecordingError::BadEncoding);
        }
        let check_interval = reader.u64()?;

        // Each event is at least 6 bytes on the wire, but clamp the reserve to
        // the bytes actually present so a hostile count cannot force a huge
        // allocation; the loop still errors cleanly on truncation.
        let input_count = reader.u32()? as usize;
        let mut inputs = Vec::with_capacity(input_count.min(reader.remaining()));
        let mut last_input_frame = 0u64;
        for _ in 0..input_count {
            let frame = reader.u64()?;
            // Inputs land before the frame they stamp is stepped, so a stamp at
            // or past the total is dead, and stamps must be ascending.
            if frame >= frames || frame < last_input_frame {
                return Err(RecordingError::BadEncoding);
            }
            last_input_frame = frame;
            let control = ControlId(reader.u8()?);
            let input = match reader.u8()? {
                0 => ControlInput::Digital(reader.u8()? != 0),
                1 => ControlInput::Axis(reader.f32()?),
                _ => return Err(RecordingError::BadEncoding),
            };
            inputs.push(InputRecord {
                frame,
                control,
                input,
            });
        }

        let check_count = reader.u32()? as usize;
        let mut checks = Vec::with_capacity(check_count.min(reader.remaining()));
        let mut last_check_frame = 0u64;
        for _ in 0..check_count {
            let frame = reader.u64()?;
            if frame >= frames || frame < last_check_frame {
                return Err(RecordingError::BadEncoding);
            }
            last_check_frame = frame;
            let hash = reader.u64()?;
            checks.push(FrameCheck { frame, hash });
        }

        // A well-formed recording is consumed exactly; trailing bytes mean the
        // framing did not describe the whole input.
        if reader.remaining() != 0 {
            return Err(RecordingError::BadEncoding);
        }

        Ok(Recording {
            initial_state,
            inputs,
            checks,
            frames,
            check_interval,
        })
    }
}

/// A cursor over recording bytes with bounds-checked little-endian reads.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], RecordingError> {
        let end = self.pos.checked_add(n).ok_or(RecordingError::Truncated)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(RecordingError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    /// Bytes not yet consumed — the upper bound on any count, so a hostile
    /// length can never drive a `with_capacity` past the data actually present.
    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn u8(&mut self) -> Result<u8, RecordingError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, RecordingError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, RecordingError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn f32(&mut self) -> Result<f32, RecordingError> {
        let b = self.take(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Captures a recording from a live console: snapshots the initial state, then
/// accumulates input events and frame-hash checkpoints as the caller steps the
/// console.
///
/// Starting re-seats the console from its own captured state (a boundary
/// save/restore), so the recorded timeline is the exact continuation replay
/// reproduces — the recording is self-consistent by construction.
pub struct Recorder {
    initial_state: Vec<u8>,
    inputs: Vec<InputRecord>,
    checks: Vec<FrameCheck>,
    frame: u64,
    check_interval: u64,
}

impl Recorder {
    /// Begin recording from the console's current boundary state. `check_interval`
    /// is the frame cadence for hash checkpoints (`0` disables them). Errors when
    /// the console has no save-state backend ([`StateError::Unsupported`]), when
    /// it is off an instruction boundary ([`StateError::NotAtBoundary`] — a frame
    /// boundary need not be one), or when its state cannot be restored, including
    /// the CGB double-speed boundary.
    pub fn start(
        console: &mut dyn SystemConsole,
        check_interval: u64,
    ) -> Result<Recorder, StateError> {
        let saved = console.save_state();
        let initial_state = match saved {
            Some(state) => state,
            None if console.state_schema().is_some() => return Err(StateError::NotAtBoundary),
            None => return Err(StateError::Unsupported),
        };
        console.load_state(&initial_state)?;
        Ok(Recorder {
            initial_state,
            inputs: Vec::new(),
            checks: Vec::new(),
            frame: 0,
            check_interval,
        })
    }

    /// Note an input applied at the current frame boundary — call alongside the
    /// matching `console.set_control`.
    pub fn note_input(&mut self, control: ControlId, input: ControlInput) {
        self.inputs.push(InputRecord {
            frame: self.frame,
            control,
            input,
        });
    }

    /// Note that a step produced a frame (or none, for a step that emitted no
    /// display). Checkpoints its hash on the interval, then advances the frame
    /// boundary — call once per stepped frame so the count tracks replay.
    pub fn note_frame(&mut self, frame: Option<&Frame>) {
        if self.check_interval != 0 && self.frame.is_multiple_of(self.check_interval) {
            self.checks.push(FrameCheck {
                frame: self.frame,
                hash: frame.map(frame_hash).unwrap_or(0),
            });
        }
        self.frame += 1;
    }

    /// Finalize the recording. An input noted after the last frame stepped (a
    /// press while paused, or between the final frame and the stop) is stamped
    /// on a frame replay never reaches, so the timeline ends without it.
    pub fn finish(mut self) -> Recording {
        self.inputs.retain(|input| input.frame < self.frame);
        Recording {
            initial_state: self.initial_state,
            inputs: self.inputs,
            checks: self.checks,
            frames: self.frame,
            check_interval: self.check_interval,
        }
    }
}

/// Why a replay stopped short.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// The initial state could not be restored into the target console — a state
    /// for a different ROM or system, an unsupported version, or an unsupported
    /// console. The most common honest cause is replaying against the wrong ROM.
    State(StateError),
    /// A frame-hash checkpoint disagreed: the console diverged from the recorded
    /// timeline at this frame.
    Diverged {
        frame: u64,
        expected: u64,
        actual: u64,
    },
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::State(error) => write!(f, "could not restore recording: {error}"),
            ReplayError::Diverged {
                frame,
                expected,
                actual,
            } => write!(
                f,
                "replay diverged at frame {frame} (expected hash {expected:#x}, got {actual:#x})"
            ),
        }
    }
}

impl std::error::Error for ReplayError {}

/// What a completed replay verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayOutcome {
    pub frames: u64,
    pub checks_verified: u64,
}

/// What one replayed frame concluded: keep going, stop at a divergence, or the
/// whole recording has been replayed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayStep {
    Continue,
    Diverged {
        frame: u64,
        expected: u64,
        actual: u64,
    },
    Finished,
}

/// A replay's position in a recording: which frame is next, and how far the
/// input and checkpoint streams have been consumed. It steps nothing itself, so
/// a caller that runs the frames blocking ([`replay`]) and one that runs them
/// from a paced loop share the same timeline handling.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReplayCursor {
    frame: u64,
    input_cursor: usize,
    check_cursor: usize,
    checks_verified: u64,
}

impl ReplayCursor {
    pub fn new() -> ReplayCursor {
        ReplayCursor::default()
    }

    /// The frame about to be stepped.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn checks_verified(&self) -> u64 {
        self.checks_verified
    }

    /// Apply every input the recording stamps on the frame about to be stepped.
    pub fn apply_inputs(&mut self, recording: &Recording, console: &mut dyn SystemConsole) {
        while let Some(event) = recording.inputs.get(self.input_cursor) {
            if event.frame != self.frame {
                break;
            }
            console.set_control(event.control, event.input);
            self.input_cursor += 1;
        }
    }

    /// Verify every checkpoint stamped on the frame just stepped, then advance
    /// the cursor. `produced` is that frame's display, or `None` for a step that
    /// emitted none.
    pub fn note_frame(&mut self, recording: &Recording, produced: Option<&Frame>) -> ReplayStep {
        let hash = produced.map(frame_hash).unwrap_or(0);
        while let Some(check) = recording.checks.get(self.check_cursor) {
            if check.frame != self.frame {
                break;
            }
            if check.hash != hash {
                return ReplayStep::Diverged {
                    frame: self.frame,
                    expected: check.hash,
                    actual: hash,
                };
            }
            self.checks_verified += 1;
            self.check_cursor += 1;
        }
        self.frame += 1;
        if self.frame < recording.frames {
            ReplayStep::Continue
        } else {
            ReplayStep::Finished
        }
    }
}

/// Restore a recording's initial state into `console`, then drive it frame by
/// frame — applying the input stream at its timestamps and verifying frame-hash
/// checkpoints. Deterministic by construction: an identical initial state and
/// identical inputs at identical boundaries reproduce the recorded run.
///
/// Returns [`ReplayError::State`] if the console rejects the initial state (the
/// wrong-ROM / version-mismatch cases), or [`ReplayError::Diverged`] with the
/// frame index the first checkpoint disagreed on.
pub fn replay(
    recording: &Recording,
    console: &mut dyn SystemConsole,
) -> Result<ReplayOutcome, ReplayError> {
    console
        .load_state(&recording.initial_state)
        .map_err(ReplayError::State)?;

    let mut cursor = ReplayCursor::new();
    for _ in 0..recording.frames {
        cursor.apply_inputs(recording, console);
        let produced = console.step_frame().display;
        match cursor.note_frame(recording, produced.as_ref()) {
            ReplayStep::Continue | ReplayStep::Finished => {}
            ReplayStep::Diverged {
                frame,
                expected,
                actual,
            } => {
                return Err(ReplayError::Diverged {
                    frame,
                    expected,
                    actual,
                });
            }
        }
    }

    Ok(ReplayOutcome {
        frames: recording.frames,
        checks_verified: cursor.checks_verified(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Recording {
        Recording {
            initial_state: vec![1, 2, 3, 4, 5],
            inputs: vec![
                InputRecord {
                    frame: 0,
                    control: ControlId(2),
                    input: ControlInput::Digital(true),
                },
                InputRecord {
                    frame: 3,
                    control: ControlId(2),
                    input: ControlInput::Digital(false),
                },
                InputRecord {
                    frame: 5,
                    control: ControlId(8),
                    input: ControlInput::Axis(0.75),
                },
            ],
            checks: vec![
                FrameCheck {
                    frame: 0,
                    hash: 0xABCD,
                },
                FrameCheck {
                    frame: 4,
                    hash: 0x1234,
                },
            ],
            frames: 8,
            check_interval: 4,
        }
    }

    #[test]
    fn round_trips_through_bytes() {
        let recording = sample();
        let bytes = recording.to_bytes().unwrap();
        assert_eq!(Recording::from_bytes(&bytes), Ok(recording));
    }

    #[test]
    fn rejects_bad_magic() {
        assert_eq!(
            Recording::from_bytes(b"XXXX\x01"),
            Err(RecordingError::BadMagic)
        );
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut bytes = sample().to_bytes().unwrap();
        bytes[4] = 0xEE;
        assert_eq!(
            Recording::from_bytes(&bytes),
            Err(RecordingError::UnsupportedVersion(0xEE))
        );
    }

    #[test]
    fn rejects_truncated() {
        let bytes = sample().to_bytes().unwrap();
        assert_eq!(
            Recording::from_bytes(&bytes[..bytes.len() - 3]),
            Err(RecordingError::Truncated)
        );
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut bytes = sample().to_bytes().unwrap();
        bytes.push(0);
        assert_eq!(
            Recording::from_bytes(&bytes),
            Err(RecordingError::BadEncoding)
        );
    }

    #[test]
    fn rejects_an_absurd_frame_count() {
        let mut recording = sample();
        recording.frames = MAX_RECORDING_FRAMES + 1;
        recording.inputs.clear();
        recording.checks.clear();
        let bytes = recording.to_bytes().unwrap();
        assert_eq!(
            Recording::from_bytes(&bytes),
            Err(RecordingError::BadEncoding)
        );
    }

    #[test]
    fn rejects_non_ascending_input_frames() {
        let mut recording = sample();
        recording.inputs = vec![
            InputRecord {
                frame: 5,
                control: ControlId(2),
                input: ControlInput::Digital(true),
            },
            InputRecord {
                frame: 2,
                control: ControlId(2),
                input: ControlInput::Digital(false),
            },
        ];
        recording.checks.clear();
        let bytes = recording.to_bytes().unwrap();
        assert_eq!(
            Recording::from_bytes(&bytes),
            Err(RecordingError::BadEncoding)
        );
    }

    #[test]
    fn rejects_an_event_at_or_past_the_frame_total() {
        let mut recording = sample();
        // frames is 8; an input stamped at 8 is dead — never applied.
        recording.inputs = vec![InputRecord {
            frame: 8,
            control: ControlId(2),
            input: ControlInput::Digital(true),
        }];
        recording.checks.clear();
        let bytes = recording.to_bytes().unwrap();
        assert_eq!(
            Recording::from_bytes(&bytes),
            Err(RecordingError::BadEncoding)
        );
    }

    #[test]
    fn a_recorder_never_writes_an_input_the_reader_rejects() {
        // A press between the last stepped frame and the stop is stamped on a
        // frame replay never reaches — exactly what the reader refuses above.
        // The writer must not be able to produce that file.
        let mut recorder = Recorder {
            initial_state: vec![1, 2, 3],
            inputs: Vec::new(),
            checks: Vec::new(),
            frame: 0,
            check_interval: 0,
        };
        recorder.note_input(ControlId(2), ControlInput::Digital(true));
        recorder.note_frame(None);
        // Recording stops here; this press lands past the timeline's end.
        recorder.note_input(ControlId(2), ControlInput::Digital(false));

        let recording = recorder.finish();
        assert_eq!(recording.frames, 1);
        assert!(
            recording.inputs.iter().all(|i| i.frame < recording.frames),
            "an input outlived the timeline: {:?}",
            recording.inputs
        );
        let bytes = recording.to_bytes().expect("serializes");
        assert!(
            Recording::from_bytes(&bytes).is_ok(),
            "the writer produced a file its own reader rejects"
        );
    }

    #[test]
    fn a_hostile_input_count_does_not_over_allocate() {
        // A truncated file claiming u32::MAX inputs must fail cleanly, not try
        // to reserve billions of entries.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(RECORDING_MAGIC);
        bytes.push(RECORDING_VERSION);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // empty initial state
        bytes.extend_from_slice(&8u64.to_le_bytes()); // frames
        bytes.extend_from_slice(&0u64.to_le_bytes()); // check_interval
        bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // input_count
        assert_eq!(
            Recording::from_bytes(&bytes),
            Err(RecordingError::Truncated)
        );
    }
}
