//! Process exit codes.
//!
//! rimage's failure modes are distinct enough that a caller (a build script, a
//! CI job, a shell wrapper) benefits from telling them apart without parsing
//! stderr. The codes below are the contract that makes that possible; they are
//! stable, and adding a new one is a breaking change.
//!
//! The values follow the conventional Unix convention of `0` for success and a
//! small non-zero number for each category. `1` is deliberately not used for a
//! specific meaning: it is the generic "something failed" code countless tools
//! already emit, so reusing it would make a handled failure indistinguishable
//! from an unhandled one. (Crashes never produce `1` anyway: a Rust panic
//! exits as `101`, and a signal death is reported by the shell as `128+SIG` —
//! `134` for the abort that `panic = "abort"` raises.)

/// How the process ended.
///
/// Ordered from "everything worked" to "some of it did not", which is also the
/// order of increasing severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    /// Every input was processed.
    Success = 0,
    /// The command line, configuration, or output layout was unusable.
    ///
    /// Nothing was attempted: a bad argument, an empty file set, a `--resize`
    /// value the format cannot satisfy, two inputs mapping to one output.
    Usage = 2,
    /// Reading or decoding at least one input failed, and nothing was written.
    Input = 3,
    /// Encoding or writing at least one output failed, and nothing was written.
    ///
    /// The input was read successfully; the failure happened afterwards.
    Output = 4,
    /// Some files were written, others failed.
    ///
    /// Distinguishing this from [`ExitCode::Input`] / [`ExitCode::Output`]
    /// matters: a partial run has already modified the user's files, so a
    /// wrapper must not treat it as a clean no-op retry.
    Partial = 5,
}

impl ExitCode {
    /// The numeric value a shell will observe.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// The running verdict for a multi-file run.
///
/// Kept separate from [`ExitCode`] because two things about a run are not
/// expressible by folding over per-file exit codes alone:
///
/// 1. [`ExitCode::Partial`] needs to remember that *some* file succeeded at
///    some point. A fold that only keeps the last failure cannot tell a clean
///    total failure from a run that wrote half its outputs.
/// 2. Whether *any* file has been considered yet is genuinely a distinct
///    state, even though it currently reports the same code as a full success.
///    Conflating them would make "no files" look like "all files worked".
///
/// The lattice is not totally ordered. [`RunState::InputFailed`] and
/// [`RunState::OutputFailed`] are *incomparable*: either alone means nothing
/// was written, but together they mean some files were written and some were
/// not. Forcing a single rank order would have to give one of them precedence
/// and would report a partial run as a clean failure.
///
/// Use [`RunState::start`] then [`RunState::record`] per file; read the code
/// with [`RunState::exit_code`]. The type is `Copy`, so it can be accumulated
/// behind a mutex in the worker threads and folded once at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunState {
    /// No file has been recorded yet.
    #[default]
    Idle,
    /// Every file recorded so far was written, and at least one was.
    Succeeded,
    /// At least one input failed; no output has been written.
    InputFailed,
    /// At least one output failed; nothing was written successfully.
    OutputFailed,
    /// A success and a failure, or a failure on each side: a partial run.
    Mixed,
    /// A verdict decided outside the per-file loop, which passes through.
    ///
    /// Used for the configuration failures discovered before any file is
    /// touched (a bad argument, a colliding output path, a `--resize` the
    /// format cannot satisfy). It overrides the per-file state because those
    /// failures mean no file was processed at all.
    Fatal(ExitCode),
}

impl RunState {
    /// The state of a run that has not looked at any file yet.
    pub const fn start() -> Self {
        RunState::Idle
    }

    /// Record the outcome of one file.
    pub const fn record(self, file: ExitCode) -> Self {
        let next = match file {
            ExitCode::Success => RunState::Succeeded,
            ExitCode::Input => RunState::InputFailed,
            ExitCode::Output => RunState::OutputFailed,
            // A per-file outcome is never one of these. Propagating the raw
            // value keeps the run's own verdict visible rather than silently
            // downgrading it.
            other => RunState::Fatal(other),
        };

        self.merge(next)
    }

    /// Join this state with another.
    ///
    /// Written as a table because the lattice has more states than the exit
    /// code does, and the table is what defines which combination reports
    /// which code. The join is commutative and associative, so results may be
    /// folded in any order.
    pub const fn merge(self, next: RunState) -> Self {
        use RunState::*;

        match (self, next) {
            // Idle is the identity element.
            (Idle, other) | (other, Idle) => other,

            // A verdict decided outside the loop outranks per-file detail.
            (Fatal(code), _) | (_, Fatal(code)) => Fatal(code),

            // Two of the same class stay that class.
            (Succeeded, Succeeded) => Succeeded,
            (InputFailed, InputFailed) => InputFailed,
            (OutputFailed, OutputFailed) => OutputFailed,

            // A success next to a failure is the case the whole type exists
            // for: the run wrote something and failed at something.
            (Succeeded, InputFailed | OutputFailed) | (InputFailed | OutputFailed, Succeeded) => {
                Mixed
            }

            // The two failure sides are incomparable, so joining them is also
            // mixed: an input failure means that file produced no output while
            // an output failure means another file at least got that far.
            (InputFailed, OutputFailed) | (OutputFailed, InputFailed) => Mixed,

            (Mixed, _) | (_, Mixed) => Mixed,
        }
    }

    /// The exit code this state reports.
    pub const fn exit_code(self) -> ExitCode {
        match self {
            RunState::Idle | RunState::Succeeded => ExitCode::Success,
            RunState::InputFailed => ExitCode::Input,
            RunState::OutputFailed => ExitCode::Output,
            RunState::Mixed => ExitCode::Partial,
            RunState::Fatal(code) => code,
        }
    }
}

impl std::process::Termination for RunState {
    fn report(self) -> std::process::ExitCode {
        std::process::ExitCode::from(self.exit_code().as_u8())
    }
}

impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let description = match self {
            ExitCode::Success => "success",
            ExitCode::Usage => "usage error",
            ExitCode::Input => "input error",
            ExitCode::Output => "output error",
            ExitCode::Partial => "partial success",
        };

        write!(f, "{description} (exit {})", self.as_u8())
    }
}

#[cfg(test)]
mod tests {
    use super::{ExitCode, RunState};

    #[test]
    fn values_are_the_documented_contract() {
        assert_eq!(ExitCode::Success.as_u8(), 0);
        assert_eq!(ExitCode::Usage.as_u8(), 2);
        assert_eq!(ExitCode::Input.as_u8(), 3);
        assert_eq!(ExitCode::Output.as_u8(), 4);
        assert_eq!(ExitCode::Partial.as_u8(), 5);
    }

    #[test]
    fn nothing_recorded_is_success() {
        assert_eq!(RunState::start().exit_code(), ExitCode::Success);
    }

    #[test]
    fn all_files_succeeding_is_success() {
        let state = RunState::start()
            .record(ExitCode::Success)
            .record(ExitCode::Success);

        assert_eq!(state.exit_code(), ExitCode::Success);
    }

    #[test]
    fn every_file_failing_on_input_is_an_input_error() {
        let state = RunState::start()
            .record(ExitCode::Input)
            .record(ExitCode::Input);

        assert_eq!(state.exit_code(), ExitCode::Input);
    }

    #[test]
    fn every_file_failing_on_output_is_an_output_error() {
        let state = RunState::start()
            .record(ExitCode::Output)
            .record(ExitCode::Output);

        assert_eq!(state.exit_code(), ExitCode::Output);
    }

    #[test]
    fn a_success_anywhere_makes_a_failure_partial() {
        // This is the case the type exists for: the user's files were modified,
        // so a wrapper must not retry the run as if it had been a no-op.
        for failure in [ExitCode::Input, ExitCode::Output] {
            for order in [
                [ExitCode::Success, failure, ExitCode::Success],
                [failure, ExitCode::Success, ExitCode::Success],
                [ExitCode::Success, ExitCode::Success, failure],
            ] {
                let state = order
                    .iter()
                    .fold(RunState::start(), |state, code| state.record(*code));

                assert_eq!(
                    state.exit_code(),
                    ExitCode::Partial,
                    "success + {failure} in {order:?} must be partial"
                );
            }
        }
    }

    #[test]
    fn failures_on_both_sides_are_partial() {
        let state = RunState::start()
            .record(ExitCode::Input)
            .record(ExitCode::Output);

        assert_eq!(state.exit_code(), ExitCode::Partial);

        let reversed = RunState::start()
            .record(ExitCode::Output)
            .record(ExitCode::Input);

        assert_eq!(reversed.exit_code(), ExitCode::Partial);
        assert_eq!(state, reversed);
    }

    #[test]
    fn a_fatal_code_survives_later_per_file_results() {
        let state = RunState::start()
            .record(ExitCode::Usage)
            .record(ExitCode::Success)
            .record(ExitCode::Input);

        assert_eq!(state.exit_code(), ExitCode::Usage);
    }

    #[test]
    fn no_per_file_outcome_may_produce_usage_or_partial() {
        // `Partial` and `Usage` are whole-run verdicts. If a record call ever
        // produced them from a per-file code, the distinction would collapse.
        for code in [ExitCode::Success, ExitCode::Input, ExitCode::Output] {
            let state = RunState::start().record(code);
            assert!(
                !matches!(state, RunState::Mixed | RunState::Fatal(_)),
                "recording a single {code} must not decide the whole run"
            );
        }
    }

    #[test]
    fn merge_is_commutative() {
        let states = [
            RunState::Idle,
            RunState::Succeeded,
            RunState::InputFailed,
            RunState::OutputFailed,
            RunState::Mixed,
            RunState::Fatal(ExitCode::Usage),
        ];

        for left in states {
            for right in states {
                assert_eq!(
                    left.merge(right),
                    right.merge(left),
                    "{left:?} merged with {right:?} must not depend on order"
                );
            }
        }
    }

    #[test]
    fn merge_is_associative() {
        let states = [
            RunState::Idle,
            RunState::Succeeded,
            RunState::InputFailed,
            RunState::OutputFailed,
            RunState::Mixed,
            RunState::Fatal(ExitCode::Usage),
        ];

        for a in states {
            for b in states {
                for c in states {
                    assert_eq!(
                        a.merge(b).merge(c),
                        a.merge(b.merge(c)),
                        "{a:?}, {b:?}, {c:?} must merge associatively"
                    );
                }
            }
        }
    }

    #[test]
    fn folding_is_independent_of_order_for_every_permutation() {
        let files = [
            ExitCode::Success,
            ExitCode::Input,
            ExitCode::Output,
            ExitCode::Success,
        ];

        let expected = files
            .iter()
            .fold(RunState::start(), |state, code| state.record(*code));

        assert_eq!(expected.exit_code(), ExitCode::Partial);

        // Every rotation of the same multiset must agree, which is what lets a
        // parallel CLI fold partial results in completion order.
        for rotation in 0..files.len() {
            let state = files
                .iter()
                .cycle()
                .skip(rotation)
                .take(files.len())
                .fold(RunState::start(), |state, code| state.record(*code));

            assert_eq!(state, expected, "rotation {rotation} disagreed");
        }
    }

    #[test]
    fn display_names_the_class_and_the_number() {
        assert_eq!(ExitCode::Input.to_string(), "input error (exit 3)");
        assert_eq!(ExitCode::Partial.to_string(), "partial success (exit 5)");
    }
}
