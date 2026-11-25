// SPDX-License-Identifier: AGPL-3.0-or-later
//! The exceptions and interrupts the guest took, read between run slices
//! (§3.1).
//!
//! The CPU delivers exceptions and interrupts in the guest and keeps no
//! record of them unless asked ([`cpu::delivery`]). When a host diagnostic
//! asks ([`crate::Machine::enable_delivery_log`] for the diagnostic report,
//! [`crate::Machine::start_delivery_recording`] for a recording), the
//! machine enables the CPU's delivery log at its largest and, after every
//! run slice, moves its records into a [`DeliveryTap`]: a [`DeliveryRing`]
//! of the last [`RECENT_DELIVERIES`] entries for the report, and a
//! [`DeliveryRecording`], a ring of the last [`RECORDED_ENTRIES`] entries
//! with exact totals per vector, which the front end writes out when it
//! likes. An IRQ's controller input is named from the 8259's vector bases
//! under which it was acknowledged, read when the machine acknowledges it:
//! a port write before its delivery (one served while the CPU runs, or any
//! while a single-step trap or an NMI is delivered first) can change them.
//! An IRQ the CPU was handed without the machine's acknowledgement is named
//! from the bases when its slice ends.
//!
//! Both rings fold consecutive identical deliveries (the same kind, vector,
//! saved CS:IP, error code and VM transition) into one entry with a count and the retired
//! range, so a guest looping on one fault takes one entry and the entries
//! before the loop stay. The registers of the interrupted code, which the
//! CPU records with every delivery here, do not break a fold: a folded
//! entry shows those of its last delivery. The totals count every delivery.
//!
//! The log lives outside the CPU state the guest executes on and the tap
//! outside the CPU: retired counts, virtual time and every guest effect are
//! the same with either on or off. Off, the cost is one branch per slice.
//! On, each slice drains the CPU's log, an IRQ delivery reads the
//! interrupt controllers' vector bases once for its slice, and the CPU
//! takes a slower exit from its compiled code while a log exists; the
//! desktop keeps the report's log on for every machine, as the measurement
//! in `backlog/docs/design/observation.md` §Exception and interrupt
//! deliveries allows.
//! The formats are described in `backlog/docs/design/observation.md`
//! §Exception and interrupt deliveries.

use cpu::{Delivery, DeliveryCounts, DeliveryName};
use std::collections::{BTreeMap, VecDeque};
use std::fmt::{self, Write as _};

/// How many entries the diagnostic report shows.
pub const RECENT_DELIVERIES: usize = 64;

/// How many entries a recording keeps, the last ones.
pub const RECORDED_ENTRIES: usize = 10_000;

/// One entry of a [`DeliveryRing`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryEntry {
    /// `count` consecutive identical deliveries, the first at
    /// `first_retired`; `last` is the latest, with its bytes at CS:IP and
    /// its registers.
    Deliveries {
        first_retired: u64,
        count: u64,
        last: Delivery,
    },
    /// Deliveries the CPU's log overwrote before the machine read them.
    Dropped(u64),
}

impl DeliveryEntry {
    /// Whether `record` repeats this entry's delivery: the same kind,
    /// vector, saved CS:IP, error code and VM transition, whatever the registers.
    fn repeats(&self, record: &Delivery) -> bool {
        matches!(self, Self::Deliveries { last, .. }
            if (last.kind, last.vector, last.cs, last.ip, last.error_code, last.virtual_8086_before, last.virtual_8086_after)
                == (record.kind, record.vector, record.cs, record.ip, record.error_code, record.virtual_8086_before, record.virtual_8086_after))
    }

    /// The deliveries the entry stands for; 0 for dropped ones.
    pub fn count(&self) -> u64 {
        match self {
            Self::Deliveries { count, .. } => *count,
            Self::Dropped(_) => 0,
        }
    }
}

/// One line: `delivery <retired> <the delivery>` for one delivery,
/// `delivery <first>..<last> x<count> <the latest delivery>` for a folded
/// run, `dropped  <n> deliveries (…)` for a loss.
impl fmt::Display for DeliveryEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deliveries {
                first_retired,
                count,
                last,
            } => {
                if *count == 1 {
                    return write!(f, "delivery {last}");
                }
                let text = last.to_string();
                let rest = text.split_once(' ').map_or("", |(_, rest)| rest);
                write!(
                    f,
                    "delivery {first_retired}..{} x{count} {rest}",
                    last.retired
                )
            }
            Self::Dropped(count) => write!(
                f,
                "dropped  {count} deliveries (the CPU's log overflowed before the machine read it)"
            ),
        }
    }
}

/// The last entries of the deliveries pushed into it, oldest first, and
/// exact totals since it began.
#[derive(Clone, Debug)]
pub struct DeliveryRing {
    capacity: usize,
    entries: VecDeque<DeliveryEntry>,
    total: u64,
    dropped: u64,
    per_name: BTreeMap<DeliveryName, u64>,
}

impl DeliveryRing {
    /// A ring of at most `capacity` entries, at least one.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: VecDeque::new(),
            total: 0,
            dropped: 0,
            per_name: BTreeMap::new(),
        }
    }

    fn make_room(&mut self) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
    }

    /// Add one delivery: folded into the newest entry when it repeats it.
    pub fn push(&mut self, record: &Delivery) {
        self.total += 1;
        *self.per_name.entry(record.name()).or_default() += 1;
        if let Some(DeliveryEntry::Deliveries { count, last, .. }) = self
            .entries
            .back_mut()
            .filter(|entry| entry.repeats(record))
        {
            *count += 1;
            *last = *record;
            return;
        }
        self.make_room();
        self.entries.push_back(DeliveryEntry::Deliveries {
            first_retired: record.retired,
            count: 1,
            last: *record,
        });
    }

    /// Note `count` deliveries the CPU's log lost; they end any fold.
    pub fn drop_deliveries(&mut self, count: u64) {
        if count == 0 {
            return;
        }
        self.total += count;
        self.dropped += count;
        if let Some(DeliveryEntry::Dropped(dropped)) = self.entries.back_mut() {
            *dropped += count;
            return;
        }
        self.make_room();
        self.entries.push_back(DeliveryEntry::Dropped(count));
    }

    /// The entries kept, oldest first.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &DeliveryEntry> {
        self.entries.iter()
    }

    /// Every delivery since the ring began, dropped ones included.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// The deliveries the CPU's log lost since the ring began.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The deliveries the kept entries stand for.
    pub fn kept(&self) -> u64 {
        self.entries.iter().map(DeliveryEntry::count).sum()
    }

    /// The deliveries read since the ring began, per name, exceptions
    /// first, then NMI, IRQs and software interrupts, each by number.
    pub fn per_name(&self) -> &BTreeMap<DeliveryName, u64> {
        &self.per_name
    }

    fn absorb(&mut self, records: &[Delivery], dropped: u64) {
        self.drop_deliveries(dropped);
        for record in records {
            self.push(record);
        }
    }
}

/// The last entries and the counts, for the diagnostic report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentDeliveries {
    /// The last entries, oldest first, at most [`RECENT_DELIVERIES`].
    pub entries: Vec<DeliveryEntry>,
    /// The deliveries the entries stand for.
    pub kept: u64,
    /// Every delivery since the log was enabled.
    pub total: u64,
    /// Deliveries the CPU's log overwrote before the machine read them.
    pub dropped: u64,
}

/// A recording of the exceptions and interrupts the guest takes from the
/// moment it began: the last [`RECORDED_ENTRIES`] entries and the totals
/// per vector.
#[derive(Clone, Debug)]
pub struct DeliveryRecording {
    ring: DeliveryRing,
    /// The retired count when the recording began.
    began: u64,
}

impl DeliveryRecording {
    /// A recording that began at retired count `began`, keeping `entries`.
    pub fn new(began: u64, entries: usize) -> Self {
        Self {
            ring: DeliveryRing::new(entries),
            began,
        }
    }

    /// The deliveries recorded so far.
    pub fn ring(&self) -> &DeliveryRing {
        &self.ring
    }

    /// The recording as text, with the retired count `now`: a header, the
    /// kept entries oldest first, and the summary of the totals; see
    /// `backlog/docs/design/observation.md` §Exception and interrupt
    /// deliveries.
    pub fn text(&self, now: u64) -> String {
        let ring = &self.ring;
        let mut text = format!(
            "trace    exceptions and interrupts the guest took from retired {} to retired {now}\n\
             trace    the last {} entries, oldest first; an entry with a range and xN stands for N identical deliveries in a row\n",
            self.began,
            ring.entries.len()
        );
        for entry in ring.entries() {
            let _ = writeln!(text, "{entry}");
        }
        let _ = writeln!(
            text,
            "summary  {} deliveries, {} of them in the entries kept, {} dropped before they were read",
            ring.total,
            ring.kept(),
            ring.dropped
        );
        for (name, count) in ring.per_name() {
            let _ = writeln!(text, "summary  {name}: {count}");
        }
        text
    }
}

/// The controller input of an IRQ `vector`, from the master's and the
/// slave's vector bases (their low three bits ignored, as ICW2's are).
fn irq_input(bases: [Option<u8>; 2], vector: u8) -> Option<u8> {
    bases.into_iter().zip([0, 8]).find_map(|(base, first)| {
        let offset = vector.checked_sub(base? & 0xF8)?;
        (offset < 8).then_some(first + offset)
    })
}

/// What the machine collects from the CPU's delivery log; see the module
/// documentation.
#[derive(Default)]
pub struct DeliveryTap {
    recent: Option<DeliveryRing>,
    recording: Option<DeliveryRecording>,
    /// Records read from the CPU after the last slice.
    buffer: Vec<Delivery>,
}

impl DeliveryTap {
    /// Add the records in the buffer, read with `counts`.
    fn absorb(&mut self, counts: DeliveryCounts) {
        if let Some(recent) = &mut self.recent {
            recent.absorb(&self.buffer, counts.dropped);
        }
        if let Some(recording) = &mut self.recording {
            recording.ring.absorb(&self.buffer, counts.dropped);
        }
    }
}

impl<C: cpu::Cpu> crate::Machine<C> {
    /// The tap, with the CPU's log enabled at its largest: a slice with at
    /// most one delivery per step loses none, and any loss is counted as
    /// dropped. The log grows only with the records it holds.
    fn delivery_tap(&mut self) -> Result<&mut DeliveryTap, cpu::DeliveryLogError> {
        if self.deliveries.is_none() {
            self.cpu
                .enable_delivery_log(cpu::delivery::DELIVERY_LOG_MAX)?;
        }
        Ok(self.deliveries.get_or_insert_with(Box::default))
    }

    /// Start keeping the last [`RECENT_DELIVERIES`] entries of the
    /// exceptions and interrupts the guest takes, for a diagnostic report;
    /// see [`crate::delivery_tap`]. The CPU's log stays outside the state
    /// the guest executes on, so retired counts and every guest effect are
    /// unchanged. Calling it again keeps what was kept. A CPU without a
    /// delivery log refuses.
    pub fn enable_delivery_log(&mut self) -> Result<(), cpu::DeliveryLogError> {
        let tap = self.delivery_tap()?;
        tap.recent
            .get_or_insert_with(|| DeliveryRing::new(RECENT_DELIVERIES));
        Ok(())
    }

    /// Start recording every exception and interrupt the guest takes from
    /// now on ([`DeliveryRecording`]), keeping the last
    /// [`RECORDED_ENTRIES`] entries; read it with
    /// [`Self::delivery_recording`] and end it with
    /// [`Self::stop_delivery_recording`]. As [`Self::enable_delivery_log`],
    /// it leaves the run unchanged. Calling it again keeps the recording.
    pub fn start_delivery_recording(&mut self) -> Result<(), cpu::DeliveryLogError> {
        let began = self.cpu.retired();
        let tap = self.delivery_tap()?;
        tap.recording
            .get_or_insert_with(|| DeliveryRecording::new(began, RECORDED_ENTRIES));
        Ok(())
    }

    /// Whether a recording runs.
    pub fn delivery_recording_active(&self) -> bool {
        self.deliveries
            .as_deref()
            .is_some_and(|tap| tap.recording.is_some())
    }

    /// The recording's text so far; `None` while none runs.
    pub fn delivery_recording(&self) -> Option<String> {
        let recording = self.deliveries.as_deref()?.recording.as_ref()?;
        Some(recording.text(self.cpu.retired()))
    }

    /// End the recording; its text, `None` when none ran.
    pub fn stop_delivery_recording(&mut self) -> Option<String> {
        let text = self.delivery_recording();
        if let Some(tap) = self.deliveries.as_deref_mut() {
            tap.recording = None;
        }
        text
    }

    /// Name the controller input of an IRQ delivery from its vector, with
    /// the vector bases `bases_of` reads from the bus as it stands when the
    /// machine acknowledges the IRQ (see the module docs): the master's
    /// eight vectors are IRQ 0 to 7, the slave's IRQ 8 to 15. The bases are
    /// read once per acknowledgement while a delivery log exists.
    pub fn set_irq_bases_of(&mut self, bases_of: crate::IrqBasesOf) {
        self.irq_bases_of = Some(bases_of);
    }

    /// Move the CPU's delivery records into the tap; nothing without one.
    /// `bases` are the vector bases read when the IRQ the slice delivered
    /// was acknowledged; without them they are read now.
    pub(crate) fn read_deliveries(&mut self, bases: Option<[Option<u8>; 2]>) {
        let Some(tap) = self.deliveries.as_deref_mut() else {
            return;
        };
        tap.buffer.clear();
        let Some(counts) = self.cpu.drain_deliveries(&mut tap.buffer) else {
            return;
        };
        let irq = |record: &Delivery| record.kind == cpu::DeliveryKind::Irq;
        if let Some(bases_of) = self
            .irq_bases_of
            .as_ref()
            .filter(|_| tap.buffer.iter().any(irq))
        {
            let bases = bases.unwrap_or_else(|| bases_of(&self.bus));
            for record in tap.buffer.iter_mut().filter(|record| irq(record)) {
                record.irq = irq_input(bases, record.vector);
            }
        }
        tap.absorb(counts);
    }

    /// The last entries and the counts, when
    /// [`Self::enable_delivery_log`] enabled them.
    pub fn recent_deliveries(&self) -> Option<RecentDeliveries> {
        let tap = self.deliveries.as_deref()?;
        let recent = tap.recent.as_ref()?;
        Some(RecentDeliveries {
            entries: recent.entries().copied().collect(),
            kept: recent.kept(),
            total: recent.total(),
            dropped: recent.dropped(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bus, Machine, RunOutcome};
    use cpu::{diagnostic::Peek, x86jit_backend::RealSegments, Cpu, DeliveryKind, X86JitCpu};

    fn record(retired: u64, kind: DeliveryKind, vector: u8, ip: u32) -> Delivery {
        Delivery {
            retired,
            kind,
            vector,
            irq: None,
            cs: 0xF000,
            ip,
            linear: 0xF0000 + ip,
            error_code: None,
            cpl_before: 0,
            cpl_after: 0,
            virtual_8086_before: false,
            virtual_8086_after: false,
            bytes: [Peek::Value(0x90); cpu::delivery::DELIVERY_BYTES],
            regs: None,
        }
    }

    /// A machine on a bare 80286 with `program` at 1000h, INT 21h pointing
    /// at an IRET and INT 6 at a #UD handler that steps over the two-byte
    /// invalid opcode (`push bp; mov bp,sp; add word [bp+2],2; pop bp;
    /// iret`).
    fn bare_machine(program: &[u8]) -> Machine<X86JitCpu> {
        let mut cpu =
            X86JitCpu::new_i286_bare(0x100_0000, 0, 0x1000, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.write_mem(0x1000, program).unwrap();
        cpu.write_mem(0x21 * 4, &[0x00, 0x18, 0, 0]).unwrap();
        cpu.write_mem(0x1800, &[0xCF]).unwrap(); // IRET
        cpu.write_mem(6 * 4, &[0x10, 0x18, 0, 0]).unwrap();
        cpu.write_mem(
            0x1810,
            &[0x55, 0x89, 0xE5, 0x83, 0x46, 0x02, 0x02, 0x5D, 0xCF],
        )
        .unwrap();
        let bus = Bus::new(Vec::new(), bus::DecodeMap::default());
        Machine::new(cpu, bus, Box::new(crate::tests::NoIc), Vec::new(), None)
    }

    /// A recording started after the report's log counts from its own
    /// start: the first three INT 21h go to the report only, the recording
    /// keeps the next three and counts three in its summary, while the
    /// report keeps all six; the run is the same as a twin's without
    /// either. Stopping the recording returns its text and ends it.
    #[test]
    fn a_recording_started_after_the_report_log_counts_from_its_own_start() {
        let program = [
            0xCD, 0x21, 0xCD, 0x21, 0xCD, 0x21, // 1000 INT 21h x3
            0xCD, 0x21, 0xCD, 0x21, 0xCD, 0x21, // 1006 INT 21h x3
            0xFA, 0xF4, // 100C CLI; HLT
        ];
        let run = |diagnose: bool| {
            let mut machine = bare_machine(&program);
            if diagnose {
                machine.enable_delivery_log().unwrap();
            }
            // Three INT 21h and their IRETs.
            assert_eq!(
                machine.run_bounded(Some(6)).unwrap(),
                RunOutcome::RetiredCapReached
            );
            if diagnose {
                machine.start_delivery_recording().unwrap();
            }
            assert_eq!(machine.run_bounded(Some(1000)).unwrap(), RunOutcome::Halted);
            (machine.cpu().retired(), machine.virtual_time(), machine)
        };
        let (retired, time, mut machine) = run(true);
        let (plain_retired, plain_time, _) = run(false);
        assert_eq!((retired, time), (plain_retired, plain_time));
        let text = machine.stop_delivery_recording().unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 7, "{text}");
        assert_eq!(
            lines[0],
            format!(
                "trace    exceptions and interrupts the guest took from retired 6 to retired {retired}"
            )
        );
        assert!(lines[2].starts_with("delivery 7 software INT 21h vector 21 at 0000:1008 "));
        assert!(lines[4].starts_with("delivery 11 software INT 21h vector 21 at 0000:100c "));
        assert_eq!(
            lines[5..],
            [
                "summary  3 deliveries, 3 of them in the entries kept, 0 dropped before they were read",
                "summary  INT 21h: 3",
            ]
        );
        assert!(machine.delivery_recording().is_none());
        let recent = machine.recent_deliveries().unwrap();
        assert_eq!(
            (
                recent.entries.len(),
                recent.kept,
                recent.total,
                recent.dropped
            ),
            (6, 6, 6, 0)
        );
    }

    /// Fifty distinct software interrupts, then a loop of 100,000 identical
    /// #UD faults: the report's last entries and the recording both show
    /// the fifty, oldest first, and one folded entry with the loop's count,
    /// retired range and the registers of its last fault; the totals are
    /// exact.
    #[test]
    fn a_fault_loop_folds_into_one_entry_after_the_distinct_deliveries() {
        let mut program = vec![0xBC, 0x00, 0xF0]; // 1000 MOV SP,0F000h
        for _ in 0..50 {
            program.extend([0xCD, 0x21]); // 1003.. INT 21h x50
        }
        program.extend([
            0xBA, 0x02, 0x00, // 1067 MOV DX,2
            0xB9, 0x50, 0xC3, // 106A MOV CX,50000
            0x0F, 0xFF, // 106D #UD, stepped over
            0xE2, 0xFC, // 106F LOOP 106Dh
            0x4A, // 1071 DEC DX
            0x75, 0xF6, // 1072 JNZ 106Ah
            0xFA, 0xF4, // 1074 CLI; HLT
        ]);
        let mut machine = bare_machine(&program);
        machine.enable_delivery_log().unwrap();
        machine.start_delivery_recording().unwrap();
        assert_eq!(
            machine.run_bounded(Some(10_000_000)).unwrap(),
            RunOutcome::Halted
        );
        let recent = machine.recent_deliveries().unwrap();
        assert_eq!(
            (
                recent.entries.len(),
                recent.kept,
                recent.total,
                recent.dropped
            ),
            (51, 100_050, 100_050, 0)
        );
        for (n, entry) in recent.entries[..50].iter().enumerate() {
            let ip = 0x1005 + 2 * n;
            assert!(
                entry.to_string().starts_with(&format!(
                    "delivery {} software INT 21h vector 21 at 0000:{ip:04x} ",
                    2 + 2 * n
                )),
                "{entry}"
            );
        }
        // MOV SP, 50 INT/IRET pairs, MOV DX and MOV CX retire 103; each
        // #UD retires nothing, its handler five and the LOOP one, and
        // between the two inner loops DEC DX, JNZ and MOV CX retire three.
        let first = 103;
        let last = first + 99_999 * 6 + 3;
        let fold = format!("delivery {first}..{last} x100000 fault #UD vector 06 at 0000:106d ");
        assert!(
            recent.entries[50].to_string().starts_with(&fold),
            "{}",
            recent.entries[50]
        );
        // CX and DX change on every pass and do not break the fold, which
        // shows the registers of its last #UD: the second inner loop's last
        // pass, CX 1 and DX 1, with DEC DX's flags (2 to 1: none set).
        assert!(
            recent.entries[50].to_string().ends_with(
                " ax 0000 bx 0000 cx 0001 dx 0001 si 0000 di 0000 bp 0000 ds 0000 es 0000 \
                 ss:sp 0000:f000 flags 0002"
            ),
            "{}",
            recent.entries[50]
        );
        let text = machine.stop_delivery_recording().unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2 + 51 + 3, "{text}");
        assert_eq!(
            lines[2..53],
            recent
                .entries
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            lines[53..],
            [
                "summary  100050 deliveries, 100050 of them in the entries kept, 0 dropped before they were read",
                "summary  #UD: 100000",
                "summary  INT 21h: 50",
            ]
        );
    }

    /// A full ring drops its oldest entry; a loss ends a fold and stands
    /// as its own entry; the totals count every delivery.
    #[test]
    fn the_ring_keeps_the_last_entries_and_exact_totals() {
        let mut ring = DeliveryRing::new(3);
        for n in 0..5u64 {
            ring.push(&record(
                n * 10,
                DeliveryKind::Software,
                0x60 + n as u8,
                0x10,
            ));
        }
        ring.push(&record(60, DeliveryKind::Software, 0x64, 0x10));
        ring.drop_deliveries(7);
        ring.drop_deliveries(1);
        ring.push(&record(90, DeliveryKind::Software, 0x64, 0x10));
        let lines: Vec<String> = ring.entries().map(ToString::to_string).collect();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].starts_with("delivery 40..60 x2 software INT 64h vector 64 at f000:0010 "));
        assert_eq!(
            lines[1],
            "dropped  8 deliveries (the CPU's log overflowed before the machine read it)"
        );
        assert!(lines[2].starts_with("delivery 90 software INT 64h "));
        assert_eq!((ring.total(), ring.kept(), ring.dropped()), (15, 3, 8));
        assert_eq!(ring.per_name().values().sum::<u64>(), 7);
        assert_eq!(ring.per_name()[&DeliveryName::Int(0x64)], 3);
    }

    /// A different error code at the same CS:IP is another delivery: it
    /// starts a new entry; the same one folds.
    #[test]
    fn an_error_code_difference_breaks_a_fold() {
        let mut ring = DeliveryRing::new(8);
        for code in [0x00F8, 0x00F8, 0x0008] {
            let mut fault = record(10, DeliveryKind::Fault, 0x0D, 0x20);
            fault.error_code = Some(code);
            ring.push(&fault);
        }
        let counts: Vec<u64> = ring.entries().map(DeliveryEntry::count).collect();
        assert_eq!(counts, [2, 1]);
    }

    #[test]
    fn v86_mode_differences_break_delivery_folds() {
        let mut ring = DeliveryRing::new(8);
        for (before, after) in [(false, false), (true, false), (true, false), (false, true)] {
            let mut fault = record(10, DeliveryKind::Fault, 0x0D, 0x20);
            fault.virtual_8086_before = before;
            fault.virtual_8086_after = after;
            ring.push(&fault);
        }
        let counts: Vec<u64> = ring.entries().map(DeliveryEntry::count).collect();
        assert_eq!(counts, [1, 2, 1]);
    }

    /// IRQ inputs come from the vector bases, their low three bits ignored;
    /// a vector neither base covers has none.
    #[test]
    fn irq_inputs_follow_the_vector_bases() {
        let bases = [Some(0x08), Some(0x72)];
        assert_eq!(irq_input(bases, 0x08), Some(0));
        assert_eq!(irq_input(bases, 0x0F), Some(7));
        assert_eq!(irq_input(bases, 0x70), Some(8));
        assert_eq!(irq_input(bases, 0x77), Some(15));
        assert_eq!(irq_input(bases, 0x10), None);
        assert_eq!(irq_input([None, Some(0x70)], 0x08), None);
    }
}
