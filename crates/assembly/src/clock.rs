// SPDX-License-Identifier: AGPL-3.0-or-later
//! Virtual-clock configuration (§6).

use std::num::NonZeroU64;

use crate::error::ConfigError;

/// How the deterministic virtual clock is driven (§6).
///
/// Nominal CPU clocks lower to a throughput ratio using an approximate CPI;
/// see `backlog/docs/design/machine-files.md` §Nominal CPU clocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockConfig {
    /// Numerator A of the CPU rate A/B, in retired steps per PIT input clock.
    /// B is [`Self::retired_per_pit_clock_denominator`]. Both must be positive;
    /// validation rejects zero. Each completed REP iteration counts as a step.
    /// This is a modelling ratio, not a frequency or cycle cost.
    /// The unbound default is `1/1`; boards select their calibrated rate through
    /// [`crate::BoardModel::default_clock`]. Fractional PIT clocks
    /// carry across CPU calls; HLT advances whole PIT clocks independently.
    pub retired_per_pit_clock: u64,
    /// Denominator B of the retired-steps-per-PIT-clock rate A/B, default `1`.
    /// For example, A=2, B=1 emits one PIT clock per two retired steps; A=1,
    /// B=3 emits three clocks per step. The ratio need not be reduced.
    pub retired_per_pit_clock_denominator: u64,
    /// Optional alternate button position, also in completed steps/PIT clock.
    /// The base rate is selected at power-on unless the run requests turbo.
    pub turbo: Option<(u64, u64)>,
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            retired_per_pit_clock: machine::RETIRED_PER_PIT_CLOCK,
            retired_per_pit_clock_denominator: 1,
            turbo: None,
        }
    }
}

impl ClockConfig {
    /// Convert integer kHz to a reduced steps/PIT ratio, without rounding.
    /// Intel 80286 and 80287 Programmer's Reference Manual, Appendix B;
    /// Intel 80386 Programmer's Reference Manual, Chapter 17; i486 Processor
    /// Programmer's Reference Manual, Chapter 26 (ADD, MOV, Jcc timings).
    /// The assumed mix and conservative rounded CPI are documented in
    /// `backlog/docs/design/machine-files.md` §Nominal CPU clocks.
    pub fn from_khz(chip: crate::CpuChip, khz: std::num::NonZeroU32) -> Self {
        let cpi = match chip {
            crate::CpuChip::I80286 | crate::CpuChip::I80386 => 4,
            crate::CpuChip::I80486Dx | crate::CpuChip::I80486DxCpuid => 2,
        };
        let pit = device::clock::PIT_HZ;
        let numerator = u64::from(khz.get()) * 1000 * pit.denominator();
        let denominator = pit.numerator() * cpi;
        let (mut a, mut b) = (numerator, denominator);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Self {
            retired_per_pit_clock: numerator / a,
            retired_per_pit_clock_denominator: denominator / a,
            turbo: None,
        }
    }

    pub(crate) fn turbo_rate(&self) -> Result<Option<(NonZeroU64, NonZeroU64)>, ConfigError> {
        self.turbo
            .map(|(numerator, denominator)| {
                Self {
                    retired_per_pit_clock: numerator,
                    retired_per_pit_clock_denominator: denominator,
                    turbo: None,
                }
                .cpu_rate()
            })
            .transpose()
    }

    pub(crate) fn cpu_rate(&self) -> Result<(NonZeroU64, NonZeroU64), ConfigError> {
        match (
            NonZeroU64::new(self.retired_per_pit_clock),
            NonZeroU64::new(self.retired_per_pit_clock_denominator),
        ) {
            (Some(numerator), Some(denominator)) => Ok((numerator, denominator)),
            _ => Err(ConfigError::InvalidCpuRate {
                numerator: self.retired_per_pit_clock,
                denominator: self.retired_per_pit_clock_denominator,
            }),
        }
    }
}
