//! The in-ROM verdict protocol. A self-checking image latches its result into
//! a four-byte RESULT block — the verdict magic, then the failing sub-check's
//! code and the bytes it observed and expected — and the harness polls that
//! block as it drives the machine.

pub const PASS: u8 = 0xA5;
pub const FAIL: u8 = 0x5A;

/// A RESULT block as a ROM latched it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub passed: bool,
    pub code: u8,
    pub observed: u8,
    pub expected: u8,
}

impl Verdict {
    /// Read a latched verdict out of the block's four bytes; `None` while the
    /// magic says the ROM has not reported one.
    pub fn read(block: [u8; 4]) -> Option<Self> {
        let passed = match block[0] {
            PASS => true,
            FAIL => false,
            _ => return None,
        };
        Some(Verdict {
            passed,
            code: block[1],
            observed: block[2],
            expected: block[3],
        })
    }
}

/// What one turn of a harness's own quantum produced.
pub enum Poll {
    /// The machine advanced; these are the RESULT block's four bytes.
    Read([u8; 4]),
    /// The machine advanced somewhere the block is not worth reading.
    Pending,
    /// The processor can make no further progress; the block as it stands.
    Stopped([u8; 4]),
}

/// Why the poll ended.
pub enum Outcome {
    Reached(Verdict),
    /// The processor stopped before reporting; the RESULT byte as it stood.
    Stopped(u8),
    /// The budget ran out before the ROM reported; the RESULT byte as it stood.
    Exhausted(u8),
}

/// Drive a machine through at most `budget` of its own quanta, polling the
/// RESULT block after each, until the ROM latches a verdict. A verdict written
/// by the very instruction that stops the processor still counts.
pub fn poll_verdict(budget: u64, mut advance: impl FnMut() -> Poll) -> Outcome {
    let mut result = 0u8;
    for _ in 0..budget {
        let block = match advance() {
            Poll::Pending => continue,
            Poll::Read(block) => block,
            Poll::Stopped(block) => {
                return match Verdict::read(block) {
                    Some(verdict) => Outcome::Reached(verdict),
                    None => Outcome::Stopped(block[0]),
                };
            }
        };
        result = block[0];
        if let Some(verdict) = Verdict::read(block) {
            return Outcome::Reached(verdict);
        }
    }
    Outcome::Exhausted(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_block_reads_as_passed() {
        let verdict = Verdict::read([PASS, 1, 2, 3]).expect("a latched verdict");
        assert!(verdict.passed);
        assert_eq!(
            (verdict.code, verdict.observed, verdict.expected),
            (1, 2, 3)
        );
    }

    #[test]
    fn an_unwritten_block_carries_no_verdict() {
        assert_eq!(Verdict::read([0x00, 0, 0, 0]), None);
    }

    #[test]
    fn polling_stops_at_the_first_latched_verdict() {
        let mut turns = 0u64;
        let outcome = poll_verdict(100, || {
            turns += 1;
            match turns {
                1..=3 => Poll::Read([0, 0, 0, 0]),
                _ => Poll::Read([FAIL, 7, 8, 9]),
            }
        });
        assert!(matches!(outcome, Outcome::Reached(v) if !v.passed && v.code == 7));
        assert_eq!(turns, 4);
    }

    #[test]
    fn a_pending_turn_still_spends_budget() {
        let outcome = poll_verdict(4, || Poll::Pending);
        assert!(matches!(outcome, Outcome::Exhausted(0)));
    }

    #[test]
    fn a_verdict_latched_as_the_processor_stops_still_counts() {
        let outcome = poll_verdict(4, || Poll::Stopped([PASS, 0, 0, 0]));
        assert!(matches!(outcome, Outcome::Reached(v) if v.passed));
    }

    #[test]
    fn a_stop_without_a_verdict_reports_the_result_byte() {
        let outcome = poll_verdict(4, || Poll::Stopped([0x11, 0, 0, 0]));
        assert!(matches!(outcome, Outcome::Stopped(0x11)));
    }

    #[test]
    fn exhaustion_reports_the_last_block_read() {
        let outcome = poll_verdict(3, || Poll::Read([0x22, 0, 0, 0]));
        assert!(matches!(outcome, Outcome::Exhausted(0x22)));
    }
}
