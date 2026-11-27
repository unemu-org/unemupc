// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exact clock domains for the IBM PC/AT system board (spec §6).
//!
//! IBM PC AT Technical Reference, System Board, Clock: the board oscillator
//! feeds the PIT through a divide-by-12.
//! IBM VGA/XGA Technical Reference, Miscellaneous Output Register; PC16550D
//! datasheet, Clock Input; MC146818 datasheet, Oscillator; Yamaha YMF262
//! datasheet, Clock and Sampling Frequency describe the independent domains.
//! See `backlog/docs/architecture.md` §Virtual clock for the time-source model.

use crate::VClock;

/// Virtual ticks in one PIT input clock. The time unit is kept here alone.
pub const TICKS_PER_PIT_CLOCK: u64 = 1;

/// A positive exact integer ratio, reduced to lowest terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ratio {
    /// Numerator in the documented unit.
    numerator: u64,
    /// Positive denominator in the documented unit.
    denominator: u64,
}

impl Ratio {
    /// Construct a positive ratio; zero frequencies and periods are invalid.
    pub const fn new(numerator: u64, denominator: u64) -> Self {
        assert!(numerator != 0 && denominator != 0);
        let (mut a, mut b) = (numerator, denominator);
        while b != 0 {
            let remainder = a % b;
            a = b;
            b = remainder;
        }
        Self {
            numerator: numerator / a,
            denominator: denominator / a,
        }
    }

    /// Numerator of the reduced exact ratio.
    #[inline]
    pub const fn numerator(self) -> u64 {
        self.numerator
    }

    /// Positive denominator of the reduced exact ratio.
    #[inline]
    pub const fn denominator(self) -> u64 {
        self.denominator
    }

    /// Divide a ratio by a positive integer.
    pub const fn divided_by(self, divisor: u64) -> Self {
        Self::new(self.numerator, self.denominator * divisor)
    }
}

/// The single board oscillator, exactly 315/22 MHz, expressed in Hz.
///
/// SMPTE 170M defines the NTSC colour subcarrier as 315/88 MHz; four times that
/// is the board's nominal 14.31818 MHz clock. IBM PC AT Technical Reference, System
/// Board, Clock, describes this system oscillator; the rational value retains
/// the subcarrier definition rather than rounding the quoted decimal.
pub const OSC: Ratio = Ratio::new(315_000_000, 22);
/// Exact timer input frequency in Hz, derived from the board oscillator.
pub const PIT_HZ: Ratio = OSC.divided_by(12);

/// A free-running clock anchored at virtual time zero.
///
/// Periods are exact ratios of virtual ticks per edge. All counts include edges
/// in `(a, b]`, excluding the epoch edge at zero. Several fast-clock edges can
/// share a tick; rounding is applied only at the observation boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockDomain {
    ticks_per_edge: Ratio,
}

impl ClockDomain {
    /// Define an exact period in virtual ticks per edge.
    pub(crate) const fn new(numerator: u64, denominator: u64) -> Self {
        Self {
            ticks_per_edge: Ratio::new(numerator, denominator),
        }
    }

    /// Define a domain from its exact frequency in Hz.
    pub const fn from_hz(hz: Ratio) -> Self {
        Self::new(
            PIT_HZ.numerator * TICKS_PER_PIT_CLOCK * hz.denominator,
            PIT_HZ.denominator * hz.numerator,
        )
    }

    /// Exact virtual ticks per edge, before any rounding.
    #[inline]
    pub const fn period(self) -> Ratio {
        self.ticks_per_edge
    }

    /// Divide the input clock, preserving its epoch and fractional phase.
    pub const fn divided_by(self, divisor: u64) -> Self {
        Self::new(
            self.ticks_per_edge.numerator * divisor,
            self.ticks_per_edge.denominator,
        )
    }

    /// Number of edges observed by a whole virtual tick, without narrowing.
    ///
    /// Device timing asks this on every access, so a product that fits 64
    /// bits divides there; the 128-bit division gives the same quotient and
    /// is kept for the rest of the range.
    #[inline]
    pub const fn edges_at(self, at: VClock) -> u128 {
        let numerator = self.ticks_per_edge.numerator;
        let denominator = self.ticks_per_edge.denominator;
        match at.0.checked_mul(denominator) {
            Some(product) => (product / numerator) as u128,
            None => at.0 as u128 * denominator as u128 / numerator as u128,
        }
    }

    /// Count edges in `(a, b]`; a reversed interval is invalid.
    #[inline]
    pub const fn edges_between(self, a: VClock, b: VClock) -> u128 {
        assert!(a.0 <= b.0);
        self.edges_at(b) - self.edges_at(a)
    }

    /// Rounded-up absolute edge, saturating at the end of the u128 range.
    #[inline]
    pub const fn edge_at_wide(self, n: u128) -> u128 {
        let numerator = self.ticks_per_edge.numerator as u128;
        let denominator = self.ticks_per_edge.denominator as u128;
        let fraction = ((n % denominator) * numerator).div_ceil(denominator);
        match (n / denominator).checked_mul(numerator) {
            Some(whole) => whole.saturating_add(fraction),
            None => u128::MAX,
        }
    }

    /// Absolute edge rounded up; returns None when virtual time is exhausted.
    ///
    /// As in [`ClockDomain::edges_at`], a product that fits 64 bits rounds
    /// there with the same result.
    #[inline]
    pub fn checked_edge_at(self, n: u128) -> Option<VClock> {
        if let Some(product) = u64::try_from(n)
            .ok()
            .and_then(|n| n.checked_mul(self.ticks_per_edge.numerator))
        {
            return Some(VClock(product.div_ceil(self.ticks_per_edge.denominator)));
        }
        let product = n.checked_mul(u128::from(self.ticks_per_edge.numerator))?;
        u64::try_from(product.div_ceil(u128::from(self.ticks_per_edge.denominator)))
            .ok()
            .map(VClock)
    }

    /// Absolute edge rounded up, saturating at the end of virtual time.
    #[inline]
    pub const fn edge_at(self, n: u64) -> VClock {
        VClock(self.ticks(n))
    }

    /// Duration of n edges from an edge-aligned start, rounded up once.
    ///
    /// Adding this duration to an arbitrary `now` re-rounds instead of carrying
    /// the clock's phase. It is exact for domains whose period is a whole number
    /// of ticks; use absolute edge arithmetic to retain fractional phase.
    /// Saturates at the virtual-time limit.
    #[inline]
    pub const fn ticks(self, n_edges: u64) -> u64 {
        let ticks = self.edge_at_wide(n_edges as u128);
        if ticks > u64::MAX as u128 {
            u64::MAX
        } else {
            ticks as u64
        }
    }

    /// First observable edge strictly after t, carrying the epoch's phase.
    #[inline]
    pub fn next_edge_after(self, t: VClock) -> Option<VClock> {
        self.checked_edge_at(self.edges_at(t) + 1)
    }
}

/// Intel 8254 input: board oscillator divided by twelve.
pub const PIT: ClockDomain = ClockDomain::from_hz(PIT_HZ);
/// CGA horizontal dot clock: the board oscillator.
pub const CGA_HDOT: ClockDomain = ClockDomain::from_hz(OSC);
/// VGA clock select 0, 25.175 MHz.
pub const VGA_25: ClockDomain = ClockDomain::from_hz(Ratio::new(25_175_000, 1));
/// VGA clock select 1, 28.322 MHz.
pub const VGA_28: ClockDomain = ClockDomain::from_hz(Ratio::new(28_322_000, 1));
/// PC16550D baud generator input frequency in Hz.
pub const UART_HZ: Ratio = Ratio::new(1_843_200, 1);
/// PC16550D baud generator input, 1.8432 MHz.
pub const UART: ClockDomain = ClockDomain::from_hz(UART_HZ);
/// MC146818 oscillator frequency in Hz.
pub const RTC_HZ: Ratio = Ratio::new(32_768, 1);
/// MC146818 oscillator, 32.768 kHz.
pub const RTC: ClockDomain = ClockDomain::from_hz(RTC_HZ);
/// YMF262 sample clock, board oscillator divided by 288.
pub const OPL3: ClockDomain = CGA_HDOT.divided_by(288);
/// Host-independent PCM output frequency in Hz.
pub const AUDIO_OUTPUT_HZ: Ratio = Ratio::new(48_000, 1);
/// Host-independent PCM output grid, 48 kHz.
pub const AUDIO_OUTPUT: ClockDomain = ClockDomain::from_hz(AUDIO_OUTPUT_HZ);
/// IBM 5170 6 MHz 286 board: 12 MHz crystal before CPU divide-by-two.
/// IBM 5170 Technical Reference, Microprocessor; Intel 82284 clock generator
/// datasheet, Clock Generator: CLK runs at the crystal frequency, and the
/// 80286 divides CLK by two internally. PCLK is CLK/2.
pub const AT_CPU_12: ClockDomain = ClockDomain::from_hz(Ratio::new(12_000_000, 1));
/// IBM 5170 8 MHz 286 board: 16 MHz crystal before CPU divide-by-two.
/// IBM 5170 Technical Reference, Microprocessor; Intel 82284 clock generator
/// datasheet, Clock Generator: CLK runs at the crystal frequency, and the
/// 80286 divides CLK by two internally. PCLK is CLK/2.
pub const AT_CPU_16: ClockDomain = ClockDomain::from_hz(Ratio::new(16_000_000, 1));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_domain_edges_round_up_and_carry_phase() {
        let domain = ClockDomain::new(5, 3);
        assert_eq!(
            (0..=6).map(|n| domain.edge_at(n).0).collect::<Vec<_>>(),
            [0, 2, 4, 5, 7, 9, 10]
        );
        assert_eq!(domain.next_edge_after(VClock(2)), Some(VClock(4)));
        assert_eq!(domain.next_edge_after(VClock(4)), Some(VClock(5)));
        assert_eq!(domain.edges_between(VClock(4), VClock(10)), 4);
        assert_eq!(domain.ticks(3), 5);
        assert_eq!(PIT.next_edge_after(VClock(u64::MAX)), None);
    }

    #[test]
    fn clock_domain_billion_edges_have_no_accumulated_rounding() {
        for domain in [
            PIT,
            CGA_HDOT,
            VGA_25,
            VGA_28,
            UART,
            RTC,
            OPL3,
            AUDIO_OUTPUT,
            AT_CPU_12,
            AT_CPU_16,
        ] {
            let p = domain.period();
            let n = 1_000_000_000;
            let at = domain.edge_at(n);
            assert_eq!(
                at.0 as u128,
                (n as u128 * p.numerator as u128).div_ceil(p.denominator as u128)
            );
            assert!(domain.edges_at(at) >= n as u128);
            assert!(domain.edges_at(VClock(at.0 - 1)) < n as u128);
            let next = domain.next_edge_after(at).unwrap();
            assert_eq!(
                next,
                domain.checked_edge_at(domain.edges_at(at) + 1).unwrap()
            );
            assert!(next > at);
        }
    }

    #[test]
    fn clock_domain_documented_crystals_and_conversion_through_osc_are_exact() {
        assert_eq!(OSC, Ratio::new(315_000_000, 22));
        assert_eq!(PIT_HZ, Ratio::new(105_000_000, 88));
        assert_eq!(PIT.period(), Ratio::new(TICKS_PER_PIT_CLOCK, 1));
        for (domain, hz) in [
            (CGA_HDOT, OSC),
            (VGA_25, Ratio::new(25_175_000, 1)),
            (VGA_28, Ratio::new(28_322_000, 1)),
            (UART, Ratio::new(1_843_200, 1)),
            (RTC, Ratio::new(32_768, 1)),
            (OPL3, OSC.divided_by(288)),
            (AUDIO_OUTPUT, Ratio::new(48_000, 1)),
            (AT_CPU_12, Ratio::new(12_000_000, 1)),
            (AT_CPU_16, Ratio::new(16_000_000, 1)),
        ] {
            let p = domain.period();
            let osc = CGA_HDOT.period();
            // Cross multiplication converts domain -> OSC -> PIT without rounding.
            assert_eq!(
                p.numerator as u128
                    * hz.numerator as u128
                    * OSC.denominator as u128
                    * osc.denominator as u128,
                p.denominator as u128
                    * hz.denominator as u128
                    * OSC.numerator as u128
                    * osc.numerator as u128
            );
        }
        assert_eq!(OPL3.period(), Ratio::new(PIT.ticks(24), 1));
        assert_eq!(AUDIO_OUTPUT.period(), Ratio::new(PIT.ticks(4375), 176));
    }
    #[test]
    fn clock_domain_all_pairs_convert_exactly_through_osc() {
        for k in [1, 17, 1_000_000] {
            let at = VClock(PIT.ticks(13_125_000 * k) + 7);
            for (domain, hz_numerator, hz_denominator) in [
                (PIT, 315_000_000u128, 22 * 12u128),
                (CGA_HDOT, 315_000_000, 22),
                (VGA_25, 25_175_000, 1),
                (VGA_28, 28_322_000, 1),
                (UART, 1_843_200, 1),
                (RTC, 32_768, 1),
                (OPL3, 315_000_000, 22 * 288),
                (AUDIO_OUTPUT, 48_000, 1),
                (AT_CPU_12, 12_000_000, 1),
                (AT_CPU_16, 16_000_000, 1),
            ] {
                let expected = u128::from(at.0) * hz_numerator * 22 * 12
                    / (hz_denominator * 315_000_000 * u128::from(TICKS_PER_PIT_CLOCK));
                assert_eq!(domain.edges_at(at), expected, "non-aligned time {at:?}");
            }
        }
        let common = VClock::from_pit(13_125_000);
        let osc = CGA_HDOT.period();
        for source in [
            PIT,
            CGA_HDOT,
            VGA_25,
            VGA_28,
            UART,
            RTC,
            OPL3,
            AUDIO_OUTPUT,
            AT_CPU_12,
            AT_CPU_16,
        ] {
            let n = source.edges_at(common);
            let p = source.period();
            let osc_edges = n * p.numerator as u128 * osc.denominator as u128
                / (p.denominator as u128 * osc.numerator as u128);
            assert_eq!(osc_edges, 157_500_000);
            assert_eq!(source.checked_edge_at(n), Some(common));
            for target in [
                PIT,
                CGA_HDOT,
                VGA_25,
                VGA_28,
                UART,
                RTC,
                OPL3,
                AUDIO_OUTPUT,
                AT_CPU_12,
                AT_CPU_16,
            ] {
                let q = target.period();
                let direct = n * p.numerator as u128 * q.denominator as u128
                    / (p.denominator as u128 * q.numerator as u128);
                let via_osc = osc_edges * osc.numerator as u128 * q.denominator as u128
                    / (osc.denominator as u128 * q.numerator as u128);
                assert_eq!(direct, target.edges_at(common));
                assert_eq!(direct, via_osc);
            }
        }
    }

    /// The 128-bit forms the 64-bit fast paths must reproduce exactly.
    fn wide_edges_at(domain: ClockDomain, at: VClock) -> u128 {
        let p = domain.period();
        at.0 as u128 * p.denominator as u128 / p.numerator as u128
    }

    fn wide_checked_edge_at(domain: ClockDomain, n: u128) -> Option<VClock> {
        let p = domain.period();
        let product = n.checked_mul(u128::from(p.numerator))?;
        u64::try_from(product.div_ceil(u128::from(p.denominator)))
            .ok()
            .map(VClock)
    }

    #[test]
    fn clock_domain_fast_paths_match_wide_arithmetic_at_edge_values() {
        let mut domains = vec![
            PIT,
            CGA_HDOT,
            VGA_25,
            VGA_28,
            VGA_25.divided_by(18),
            VGA_28.divided_by(8),
            UART,
            RTC,
            OPL3,
            AUDIO_OUTPUT,
            AT_CPU_12,
            AT_CPU_16,
        ];
        for (numerator, denominator) in [
            (1, 1),
            (1, u64::MAX),
            (u64::MAX, 1),
            (u64::MAX, u64::MAX - 1),
            (u64::MAX - 1, u64::MAX),
            (3, 1 << 32),
            ((1 << 32) + 1, 7),
        ] {
            domains.push(ClockDomain::new(numerator, denominator));
        }
        for domain in domains {
            let p = domain.period();
            let (numerator, denominator) = (p.numerator, p.denominator);
            // Times at and around the 64-bit product limit and around exact
            // edges, where truncation and rounding up change.
            let limit = u64::MAX / denominator;
            let mut times = vec![0, 1, 2, 3, u64::MAX, u64::MAX - 1, 1 << 40, 1 << 63];
            for base in [limit, numerator, numerator.saturating_mul(1_000_003)] {
                times.extend([base.saturating_sub(1), base, base.saturating_add(1)]);
            }
            for at in times {
                let at = VClock(at);
                assert_eq!(
                    domain.edges_at(at),
                    wide_edges_at(domain, at),
                    "{p:?} at {at:?}"
                );
                assert_eq!(
                    domain.next_edge_after(at),
                    wide_checked_edge_at(domain, wide_edges_at(domain, at) + 1),
                    "{p:?} after {at:?}"
                );
            }
            let edge_limit = u128::from(u64::MAX / numerator);
            let mut edges = vec![
                0,
                1,
                2,
                u128::from(u64::MAX),
                u128::from(u64::MAX) + 1,
                u128::MAX / u128::from(numerator),
                (u128::MAX / u128::from(numerator)).saturating_add(1),
                u128::MAX,
            ];
            for base in [edge_limit, u128::from(denominator)] {
                edges.extend([base.saturating_sub(1), base, base + 1]);
            }
            for n in edges {
                assert_eq!(
                    domain.checked_edge_at(n),
                    wide_checked_edge_at(domain, n),
                    "{p:?} edge {n}"
                );
            }
        }
    }

    #[test]
    fn clock_domain_checked_edges_exhaust_without_wrapping() {
        let n = u128::MAX / u128::from(RTC.period().numerator()) + 1;
        let result = std::panic::catch_unwind(|| RTC.checked_edge_at(n));
        assert_eq!(result.ok(), Some(None));
    }
    #[test]
    fn clock_domain_wide_edges_saturate_without_wrapping() {
        let result = std::panic::catch_unwind(|| RTC.edge_at_wide(u128::MAX));
        assert_eq!(result.ok(), Some(u128::MAX));
    }
}
