use std::{num::NonZeroUsize, thread};

use clap::ArgMatches;

/// Parallelism the machine reports: the most concurrency that can ever pay off.
///
/// Used both as the `--threads` ceiling and as the value an over-large request
/// falls back to.
pub fn num_threads() -> usize {
    thread::available_parallelism()
        .unwrap_or(NonZeroUsize::new(4).unwrap())
        .get()
}

/// Concurrency asked for through `-t/--threads`, before any clamping.
///
/// Defaults to 1, matching the flag's own default. A `0` is raised to 1 here
/// anyway: this number is a divisor for the memory budget, and a zero divisor
/// would either panic or silently mean "unlimited".
///
/// Pair it with [`clamp`]; every caller wants the clamped value, and reading
/// the raw one is only useful for telling the user what was reduced.
pub fn requested(matches: &ArgMatches) -> usize {
    matches
        .get_one::<u16>("threads")
        .copied()
        .map(|threads| threads as usize)
        .unwrap_or(1)
        .max(1)
}

/// Reduce a requested concurrency to what this machine can actually run.
///
/// Asking for more workers than there are CPUs does not buy parallelism, but
/// it does shrink the memory budget: every concurrent image is assumed to hold
/// its own set of buffers, so the per-image ceiling is the available memory
/// divided by this number. Without the clamp, a request like `-t 64` on an
/// 8-core machine both wastes the budget and rejects images that a smaller
/// `-t` would have accepted — with an error message that blames the image.
///
/// Clamping rather than rejecting keeps a fixed script portable: the same
/// command line runs on a 4-core laptop and a 64-core workstation, taking the
/// parallelism each one can offer instead of failing on the smaller machine.
pub fn clamp(requested: usize) -> usize {
    requested.clamp(1, num_threads())
}

/// How many images the memory budget has to actually provide for.
///
/// The limiter never lets more than `clamp(requested)` images run at once, but
/// when there are fewer inputs than that the surplus is reserved for images
/// that will never exist: three files with `-t 16` still means three image
/// buffers alive, not sixteen. Since the per-image ceiling is the memory
/// divided by this number, dividing by sixteen there rejects an ordinary image
/// with a message that blames the image for a `-t` that was never used.
///
/// `files == 0` means there is nothing to count — the `--print-limits`
/// diagnostic runs without inputs — and the request is the only guide left.
pub fn in_flight(requested: usize, files: usize) -> usize {
    let clamped = clamp(requested);
    if files == 0 {
        clamped
    } else {
        clamped.min(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The machine's own parallelism is accepted unchanged, so a request the
    /// host can actually honour is never silently reduced.
    #[test]
    fn the_machines_own_parallelism_survives() {
        assert_eq!(clamp(num_threads()), num_threads());
    }

    /// Anything above it collapses onto it. This is the whole point of the
    /// clamp: an oversized `-t` must not divide the memory budget further.
    #[test]
    fn an_over_large_request_falls_back_to_the_machine() {
        assert_eq!(clamp(num_threads() + 1), num_threads());
        assert_eq!(clamp(usize::MAX), num_threads());
    }

    /// The lower bound holds even though the parser rejects `0` already: this
    /// number divides the memory budget, and a zero divisor is not a value the
    /// rest of the pipeline should have to defend against.
    #[test]
    fn the_request_is_never_below_one() {
        assert_eq!(clamp(0), 1);
        assert_eq!(clamp(1), 1);
    }

    /// Fewer inputs than workers means fewer images in flight, so the budget
    /// is divided by the input count. This is the case that used to reject a
    /// single large image under `-t 12` while `-t 1` accepted the same file.
    #[test]
    fn the_budget_follows_the_input_count_down() {
        assert_eq!(in_flight(16, 1), 1);
        assert_eq!(in_flight(16, 3), 3);
        assert_eq!(in_flight(4, 3), 3);
    }

    /// More inputs than workers brings the request back into play: the extras
    /// queue behind the limiter rather than being decoded at once.
    #[test]
    fn the_budget_never_exceeds_the_request() {
        assert_eq!(in_flight(2, 100), 2);
        assert_eq!(in_flight(num_threads(), usize::MAX), num_threads());
    }

    /// No inputs to count leaves the request in charge, so a diagnostic run
    /// without files still reports the ceiling the flag asks for.
    #[test]
    fn an_unknown_input_count_defers_to_the_request() {
        assert_eq!(in_flight(3, 0), 3);
        // Still clamped: an over-large request stays over-large either way.
        assert_eq!(in_flight(usize::MAX, 0), num_threads());
    }
}
