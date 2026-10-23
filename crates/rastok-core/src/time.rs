//! Relative execution time and civil timestamps are distinct quantities.
//!
//! [`LogicalTime`] counts signed nanoseconds from a run's logical zero.
//! [`Duration`] is a signed difference, not an instant. Logical arithmetic
//! saturates at the i64 bounds; civil arithmetic on [`Timestamp`] is checked.
//! A run's immutable civil origin relates the two only when time is observed
//! or a tumbling instance's civil start is identified.

use core::ops::{Add, Sub};

const NANOS_PER_SECOND: i64 = 1_000_000_000;

/// An instant on the relative execution timeline, in nanoseconds.
///
/// The first boot is at [`Self::ZERO`]. Recovery continues that same timeline.
/// Addition accepts a duration, never another instant:
///
/// ```compile_fail
/// use rastok_core::time::LogicalTime;
/// let invalid = LogicalTime::ZERO + LogicalTime::ZERO;
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct LogicalTime(i64);

impl LogicalTime {
    /// The first instant of a run.
    pub const ZERO: Self = Self(0);
    /// Lower bound of signed logical arithmetic.
    pub const MIN: Self = Self(i64::MIN);
    /// Upper bound of signed logical arithmetic.
    pub const MAX: Self = Self(i64::MAX);

    /// Constructs a logical instant from its nanosecond count.
    pub const fn from_nanoseconds(nanoseconds: i64) -> Self {
        Self(nanoseconds)
    }

    /// The signed count used at serialization and observation boundaries.
    pub const fn as_nanoseconds(self) -> i64 {
        self.0
    }
}

/// A signed duration in nanoseconds. The DSL's duration value has the same
/// range; this type distinguishes runtime durations from logical instants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Duration(i64);

impl Duration {
    /// No elapsed time.
    pub const ZERO: Self = Self(0);
    /// Smallest representable signed duration.
    pub const MIN: Self = Self(i64::MIN);
    /// Largest representable signed duration.
    pub const MAX: Self = Self(i64::MAX);

    /// Constructs a duration from its signed nanosecond count.
    pub const fn from_nanoseconds(nanoseconds: i64) -> Self {
        Self(nanoseconds)
    }

    /// The signed duration in nanoseconds.
    pub const fn as_nanoseconds(self) -> i64 {
        self.0
    }
}

impl Add<Duration> for LogicalTime {
    type Output = Self;

    fn add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(duration.0))
    }
}

impl Sub for LogicalTime {
    type Output = Duration;

    fn sub(self, earlier: Self) -> Duration {
        Duration(self.0.saturating_sub(earlier.0))
    }
}

/// A normalized civil instant: signed seconds from the Unix epoch and a
/// nonnegative fraction smaller than one second. For example, one nanosecond
/// before the epoch is `(-1, 999_999_999)`.
///
/// The carrier is independent of a port's declared precision or zone. Type
/// checking applies those constraints separately. This pair also carries
/// zone-less timestamp values; their interpretation belongs to their datatype.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    seconds: i64,
    nanoseconds: u32,
}

/// A civil timestamp whose fractional component is outside its domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimestampError {
    /// The fraction must be strictly smaller than one second.
    InvalidNanoseconds {
        /// Rejected fractional component.
        nanoseconds: u32,
    },
}

impl Timestamp {
    /// 1970-01-01T00:00:00Z, the Lab's default civil origin.
    pub const UNIX_EPOCH: Self = Self {
        seconds: 0,
        nanoseconds: 0,
    };
    /// Earliest representable civil instant.
    pub const MIN: Self = Self {
        seconds: i64::MIN,
        nanoseconds: 0,
    };
    /// Latest representable civil instant.
    pub const MAX: Self = Self {
        seconds: i64::MAX,
        nanoseconds: 999_999_999,
    };

    /// Constructs an already normalized civil pair.
    ///
    /// # Errors
    /// Returns [`TimestampError::InvalidNanoseconds`] for a fraction at or
    /// above one billion; the constructor does not silently normalize it.
    pub const fn try_new(seconds: i64, nanoseconds: u32) -> Result<Self, TimestampError> {
        if nanoseconds >= NANOS_PER_SECOND as u32 {
            return Err(TimestampError::InvalidNanoseconds { nanoseconds });
        }
        Ok(Self {
            seconds,
            nanoseconds,
        })
    }

    /// Converts signed epoch nanoseconds, normalizing negative fractions.
    pub const fn from_nanoseconds(nanoseconds: i64) -> Self {
        Self {
            seconds: nanoseconds.div_euclid(NANOS_PER_SECOND),
            nanoseconds: nanoseconds.rem_euclid(NANOS_PER_SECOND) as u32,
        }
    }

    /// Whole seconds from the epoch, rounded toward negative infinity.
    pub const fn seconds(self) -> i64 {
        self.seconds
    }

    /// Nonnegative nanosecond fraction, strictly below one billion.
    pub const fn nanoseconds(self) -> u32 {
        self.nanoseconds
    }

    /// Adds a signed duration, refusing overflow of the civil carrier.
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        let fraction = i64::from(self.nanoseconds) + duration.0.rem_euclid(NANOS_PER_SECOND);
        // Combine the fractional carry before checking the seconds. Otherwise
        // adding -1 ns to (i64::MIN, 1) would report a false overflow.
        let seconds = duration.0.div_euclid(NANOS_PER_SECOND) + fraction / NANOS_PER_SECOND;
        Some(Self {
            seconds: self.seconds.checked_add(seconds)?,
            nanoseconds: (fraction % NANOS_PER_SECOND) as u32,
        })
    }

    /// Subtracts a signed duration, refusing overflow of the civil carrier.
    pub fn checked_sub(self, duration: Duration) -> Option<Self> {
        let fraction = i64::from(self.nanoseconds) - duration.0.rem_euclid(NANOS_PER_SECOND);
        // Do not negate the duration: i64::MIN is a valid duration.
        let seconds =
            duration.0.div_euclid(NANOS_PER_SECOND) - fraction.div_euclid(NANOS_PER_SECOND);
        Some(Self {
            seconds: self.seconds.checked_sub(seconds)?,
            nanoseconds: fraction.rem_euclid(NANOS_PER_SECOND) as u32,
        })
    }

    /// Returns the exact difference if it fits the signed nanosecond duration.
    pub fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
        let mut seconds = self.seconds.checked_sub(earlier.seconds)?;
        let mut fraction = i64::from(self.nanoseconds) - i64::from(earlier.nanoseconds);
        // Give the fraction the difference's sign before multiplication. At
        // either i64 nanosecond boundary, the unadjusted product can overflow
        // even though the complete difference is representable.
        if seconds > 0 && fraction < 0 {
            seconds -= 1;
            fraction += NANOS_PER_SECOND;
        } else if seconds < 0 && fraction > 0 {
            seconds += 1;
            fraction -= NANOS_PER_SECOND;
        }
        seconds
            .checked_mul(NANOS_PER_SECOND)?
            .checked_add(fraction)
            .map(Duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_arithmetic_saturates_at_both_signed_bounds() {
        let one = Duration::from_nanoseconds(1);
        let minus_one = Duration::from_nanoseconds(-1);
        assert_eq!(LogicalTime::MAX + one, LogicalTime::MAX);
        assert_eq!(LogicalTime::MIN + minus_one, LogicalTime::MIN);
        assert_eq!(LogicalTime::MAX - LogicalTime::MIN, Duration::MAX);
        assert_eq!(LogicalTime::MIN - LogicalTime::MAX, Duration::MIN);
        assert_eq!(LogicalTime::ZERO + Duration::MIN, LogicalTime::MIN);
        assert_eq!(LogicalTime::ZERO + Duration::MAX, LogicalTime::MAX);
        assert_eq!(LogicalTime::from_nanoseconds(5) + minus_one, LogicalTime(4));
        assert_eq!(LogicalTime(4) - LogicalTime(5), minus_one);
    }

    #[test]
    fn timestamp_fraction_is_normalized_and_bounded() {
        assert_eq!(
            Timestamp::from_nanoseconds(-1),
            Timestamp::try_new(-1, 999_999_999).unwrap()
        );
        assert_eq!(
            Timestamp::from_nanoseconds(1_000_000_000),
            Timestamp::try_new(1, 0).unwrap()
        );
        assert_eq!(Timestamp::try_new(i64::MIN, 0), Ok(Timestamp::MIN));
        assert_eq!(
            Timestamp::try_new(i64::MAX, 999_999_999),
            Ok(Timestamp::MAX)
        );
        for nanoseconds in [1_000_000_000, u32::MAX] {
            assert_eq!(
                Timestamp::try_new(0, nanoseconds),
                Err(TimestampError::InvalidNanoseconds { nanoseconds })
            );
        }
    }

    #[test]
    fn civil_arithmetic_checks_the_pair_without_false_boundary_overflow() {
        let one = Duration::from_nanoseconds(1);
        let minus_one = Duration::from_nanoseconds(-1);
        assert_eq!(Timestamp::MAX.checked_add(one), None);
        assert_eq!(Timestamp::MIN.checked_add(minus_one), None);
        assert_eq!(Timestamp::MIN.checked_sub(one), None);
        assert_eq!(Timestamp::MAX.checked_sub(minus_one), None);
        assert_eq!(
            Timestamp::try_new(i64::MIN, 1)
                .unwrap()
                .checked_add(minus_one),
            Some(Timestamp::MIN)
        );
        assert_eq!(
            Timestamp::try_new(i64::MAX, 999_999_998)
                .unwrap()
                .checked_sub(minus_one),
            Some(Timestamp::MAX)
        );
        assert_eq!(
            Timestamp::UNIX_EPOCH.checked_add(Duration::MIN),
            Some(Timestamp::from_nanoseconds(i64::MIN))
        );
        assert_eq!(
            Timestamp::from_nanoseconds(i64::MIN).checked_sub(Duration::MIN),
            Some(Timestamp::UNIX_EPOCH)
        );
    }

    #[test]
    fn civil_difference_is_exact_at_nanosecond_bounds() {
        for nanoseconds in [i64::MIN, -1, 0, 1, i64::MAX] {
            let timestamp = Timestamp::from_nanoseconds(nanoseconds);
            assert_eq!(
                timestamp.checked_duration_since(Timestamp::UNIX_EPOCH),
                Some(Duration::from_nanoseconds(nanoseconds))
            );
        }
        assert_eq!(
            Timestamp::from_nanoseconds(i64::MAX)
                .checked_add(Duration::from_nanoseconds(1))
                .unwrap()
                .checked_duration_since(Timestamp::UNIX_EPOCH),
            None
        );
        assert_eq!(
            Timestamp::from_nanoseconds(i64::MIN)
                .checked_sub(Duration::from_nanoseconds(1))
                .unwrap()
                .checked_duration_since(Timestamp::UNIX_EPOCH),
            None
        );
        assert_eq!(Timestamp::MAX.checked_duration_since(Timestamp::MIN), None);
        assert_eq!(Timestamp::MIN.checked_duration_since(Timestamp::MAX), None);
    }

    #[test]
    fn civil_arithmetic_matches_independent_nanosecond_constructions() {
        for instant in [-20_000_000_001, -999_999_999, 0, 1, 20_000_000_001] {
            for nanoseconds in [-30_000_000_001, -1, 0, 999_999_999, 30_000_000_001] {
                let timestamp = Timestamp::from_nanoseconds(instant);
                let duration = Duration::from_nanoseconds(nanoseconds);
                assert_eq!(
                    timestamp.checked_add(duration),
                    Some(Timestamp::from_nanoseconds(instant + nanoseconds))
                );
                assert_eq!(
                    timestamp.checked_sub(duration),
                    Some(Timestamp::from_nanoseconds(instant - nanoseconds))
                );
                let later = Timestamp::from_nanoseconds(instant + nanoseconds);
                assert_eq!(later.checked_duration_since(timestamp), Some(duration));
            }
        }
    }
}
