# Deterministic device scheduler

Design and adoption record, 2026-11-09. The accepted review outcome is
[decision-15](../../decisions/decision-15%20-%20Deterministic%20device%20scheduler%20with%20instruction-exact%20interrupt%20boundaries.md).
Corrected timing and deadline dispatch are adopted as the only production loop.
A CPU-core defect found during adoption, and its resolution, is recorded below. The
original proposal is retained as design history where explicitly superseded.

## Adoption

Corrected instruction-boundary timing is the only machine loop. Production uses
`Deadline`; `InterruptTiming`, the legacy block loop and `ClockConfig.quantum`
are removed. CPU rate A/B counts completed steps, including REP iterations, with
carried phase. Assembly verifies zero-step execution before building devices and
rejects unsupported CPUs with a configuration error. Long64 machine runners and
CLI blob loading are intentionally retired; CPU adapter tests remain.

The non-default `reference-scheduler` feature exposes `PerClock` and its setter
in the machine crate. Assembly forwards that feature, and CLI enables it only
in dev-dependencies for integration tests. `MachineConfig` has no engine field:
a production config cannot select the oracle, even when features are unified by
an all-targets test build. Both engines share instruction-start access times,
completed-transaction-before-INTA ordering, IF/STI/SS readiness, exact active
caps, and halted event/bound jumps. Their ordered streams must match without
normalization, including `FinalState` (RAM hashes, registers, saved device state).

**DMA at device deadlines.** Every device-deadline visit ticks due devices, samples their interrupt outputs,
and settles pending DMA, including immediate follow-on requests, **before the
clock advances again**. This also applies when one CPU step spans several PIT
clocks. For example, a request at clock 2 within a step ending at clock 10 must
complete with device time 2, not 10. Native audio writes therefore reach their
source buffers at the sample boundary; late data cannot fill an already observed
interval. The rule applies to any DMA requester and is shared by the deadline
engine and the per-clock oracle.

DMA completion can update an IRQ level at that deadline. The CPU still accepts
interrupts only at completed instruction boundaries, after the stopping
instruction's bus effects and all pending DMA have settled. Moving a DMA byte
at a device deadline does not inject an interrupt inside an instruction.
The machine-crate mock regression
`dma_deadline_inside_a_multi_clock_cpu_step_completes_before_clock_advances`
pins both initial and immediate follow-on completions to clock 2 in a ten-clock
CPU step.

Access coupling is declared by `DeviceConfig.access_dependents`: an access to
one device can change the listed devices' next events. Assembly resolves roles
to bus indices and calls `Bus::add_access_coupling`. Before each port byte,
MMIO fragment, or mutable device borrow, the bus synchronizes the target first,
then its declared dependents in ascending bus-index (configuration) order, all
at the access-start clock and before the register side effect. The ordering is
independent of declaration order; duplicate edges are idempotent. Edges are
directed, with no recursive traversal: indirect affected timers must also be
listed. Cycles therefore do not recurse or duplicate synchronization.

This brings a dependent timer's last-synced time current before a changed input
can shorten its period. Invalidating its deadline alone could instead return
`At(stale_time + period) <= now` and stop the machine with "device deadline did
not advance". Synchronization clears the target and dependent cache slots; the
machine retains its existing global invalidation after the access and before
charging the stopping instruction. Unrelated devices receive no access sync.
`EveryPitClock` devices are already current and receive no extra sync, even
when declared as dependents. After a successful access, `Bus::settle_accessed`
visits only the touched targets and declared dependents, once each in bus
order, without expanding coupling edges. Unrelated counters remain lazy:
autonomous transitions still run at their deadlines. Full `Bus::settle` remains
mandatory before snapshots and at run entry, after rejected accesses and after
DMA completion. It still visits every device in bus order.

Assembly declares the system-control-to-PIT coupling for the shared gate and
speaker inputs. The cascaded PIC faces share state, but both are `Dormant`
with no elapsed timer or autonomous deadline, so global invalidation and pin
sampling cover their shared effects. Coupling declarations prepare time before
an access; they do not connect pins or implement register effects. See also
§Access synchronization and invalidation.

**Idle DMA probes.** `Device::dma_request_pending` is a passive, conservative
hint checked after due events have been dispatched. Its default is true, so
devices without a hint keep their existing probe and synchronization behavior.
An implementation may return false only when an access-synchronized probe
cannot return a request. Timed request creation must have a device deadline;
accesses, pauses, request consumption, reset/restore and immediate completion
must all be reflected by the hint. The SB16 exposes its request/pause state;
the floppy controller exposes its queued request and DOR DMA gate. True retains
the target/dependent synchronization, wiring order, controller access and
completion path. Each follow-on pass checks again at the same clock, so the
hint cannot defer a sample or completion until the next CPU boundary.

During adoption the CPU core (then pinned at
`70be0087b583f1e38bf3479d92e90e89ef998cf1`) decoded a Real16 block past the
16-bit IP wrap and did not observe stores into not-yet-executed bytes of the
current block, so execution could depend on the step budget, contrary to the
CPU seam contract. The BIOS-to-floppy run exposed it: after the boot sector
halts and the timer wakes it, execution runs off the end of segment 0, and the
deadline engine diverged from the per-clock oracle at clock 5,899,021. Three
tests pinned the defect until the core was fixed.

The core fixed both (a Real16 block ends at CS:FFFF and a fetch crossing FFFFh
raises the general-protection exception; a store into a later instruction of
the running block ends the block after the storing instruction) and the pin
moved to `54fad1766a458df7cedb6c0b4b5d50742628ef75`. Now
`real_mode_wrapped_fetch_is_independent_of_step_partitions` and
`real_mode_store_into_current_block_is_independent_of_step_partitions` assert
budget independence, and `deadline_matches_per_clock_on_bios_floppy` compares
the full BiosFloppy stream, `FinalState` included, without normalisation. The
production BIOS-to-floppy golden keeps its single COM1 `B`. Inspected at the
new pin: the run-off reaches the end of segment 0, an instruction crossing
FFFFh raises the general-protection exception (interrupt 13), which shares its
real-mode vector with IRQ5; the BIOS handler reads the master PIC's ISR (OCW3
0Bh, port 20h), finds nothing in service and returns, and the guest re-faults
in that loop (final CS:IP F000:FF2F, stable SP) until the step cap, instead of
stopping on an unsupported instruction. The golden asserts the marker, not
the stop kind. The old per-clock oracle's second `0x42` came from executing the
straddling bytes as if they were not faulting.

Legacy-versus-corrected tests and the scheduler A/B switch are deleted. Comparator
unit tests retain explicit instruction-start shifts and bounded cap endpoint
normalization for reviewing migration evidence. They never shift INTA, discard
accesses or normalize final state. The following old/new observations classify
all changed migration expectations: **(a) systematic** accounting/timestamp or
exact-cap changes; **(b) real ordering** changes in when effects become visible.
These are reviewed migration observations, not replacement byte/screen goldens.

| Workload/test | Old → new | Class | Reason |
|---|---|---|---|
| SerialReal16, RTC, synthetic floppy; ordinary port events in all workloads | Port access completion timestamp → instruction-start timestamp (−1 clock at 1/1); MMIO shift 0 | (a) systematic | Accesses happen before charging the stopping port instruction; bytes and access order preserved. Stops remain 12, 284 and 45 respectively; VGA stop remains 5. |
| Timer boot PIT divisor high-byte write | 18 → 17 | (a) systematic | OUT timestamp now denotes instruction start (Intel 8254, Mode 2). |
| Timer boot INTA vector 8 | 40 → 39 | (b) real ordering | Earlier divisor programming moves the timer edge and HLT wake one clock earlier; INTA occurs only at a ready boundary after bus effects (Intel 8259A, Interrupt Sequence; Intel 80286, HLT). INTA is never normalized. |
| Timer boot first UART OUT; stop | 46 → 44; 337 → 336 | (b) real ordering, plus (a) for OUT start | One clock from earlier INTA plus one from access-start stamping; five serial dots unchanged. |
| Keyboard `FinalState`, 8042 hash | 5226027366141998341 → 12846667287079830168 | (a) systematic | Verified in `i8042.rs`: `read_data` schedules `keyboard_ready_at = now + KEYBOARD_BYTE_CLOCKS` after consuming a keyboard byte; `save` serializes both `now` and that deadline. The one-clock earlier read changes the saved deadline. Access/INTA/EOI otherwise agree after the port shift; RAM/registers/serial/stop 233 unchanged. Saved state remains compared. |
| test386 POST port timestamp | 86 → 85 | (a) systematic | Port access-start stamping; POST bytes unchanged. |
| test386 CPU-error stop clock | 86 → 159 (virtual time 159 → 159) | (b) real ordering | Charge the 73 completed prefix steps before propagating the CPU error; the old early return lost those device clocks. Unsupported instruction and CPU/RAM/POST state unchanged; no INTA here. |
| BIOS POST checkpoint 05 (also BIOS-floppy/DOS prefix) | 131188 → 180336 (+49148) | (a) systematic | Three REP STOSW with CX=0x4000 at ROM offsets 612a/6141/6155 add 3×(16384−1)=49149 completed iterations; access-start subtracts one. |
| FDC write/readback cap and trailing access | 5001 → 5000; legacy IN COM1 at 5001 absent; RIP 31916 → 31915 | (a) systematic | Exact cap prevents the extra instruction/access; sector and serial `W` unchanged. The comparator does not discard that extra access. |
| `real16_spin_reaches_cap_with_default_clock` | retired range 32..=131105 → exactly 32 | (a) systematic | Exact active cap; virtual time still equals completed steps. |
| `real16_self_jump_idle_advances_virtual_time` | virtual time ≥1000 → exactly 1000 | (a) systematic | Exact cap, including the self-jump's completed steps. |
| BIOS cap, frozen old policy | 10059627 → 10000000 | (a) systematic | Block overshoot removed. |
| FreeDOS prompt poll endpoint, frozen old policy | 25191999 → 25000000 | (a) systematic | Block overshoot removed. This was only a coarse observation, not the first appearance of the prompt. |
| DIR endpoint, frozen old 80M command policy | 105000274 → 105000000 | (a) systematic | Exact endpoint; separate from the calibrated command budget below. |
| `prompt_and_stimulus_budgets_start_from_current_virtual_time` | Endpoint 32 → 35 | run policy (not hardware timing) | The prompt poll ends at 25; the full requested 10 command steps must execute before stopping. A cap at 32 now fails before key injection instead of truncating the command. |

Device fidelity follow-up (2026-11-10), with the same CPU pin and corrected
loop. These are hardware-model changes, not scheduler normalisations:

| Workload/test | Old → new | Reason and measured evidence |
|---|---|---|
| Mode-2 initial divisor 5, isolated PIT deadlines | OUT low at 6 → 5; high at 7 → 6 | Intel 8254 datasheet, Mode 2: count loads at clock 1, OUT becomes low at count=1 (clock 5), then rises and reloads at 6. The former model reloaded while driving OUT low. `deadlines_follow_only_channel_zero_loads_and_out_edges` and the mid-period rewrite regression check both phases. |
| Mode-2 divisor 100 restored at clock 50 | First low at 101 → 100 (sample index 50 → 49) | Same count=1 correction. `pit_restore_at_clock_50_matches_both_engines` compares every pin sample through clock 251 after restore. |
| Timer boot, first INTA / first UART OUT / halted stop | 39 / 44 / 336 → 38 / 43 / 335 | PIT initial load remains at 18. Its rising reload edge is now at 38; ready HLT accepts IRQ0 there, moving the handler and idle-stop endpoint by one clock. `fidelity_mode2_timer_trace_uses_count_one_low_phase` checks the tuple under both engines; running it against the original PIT produces `(39, 44, 336)`. Serial remains five dots. |
| VGA synthetic register-alias snapshot (initial fixture; superseded below) | Cell (0,0) `A/4E` → blank; `A/4E` is at (1,0); cursor (2,0), hidden → (0,0), visible | The guest writes CRTC indices 26h/28h/23h. IBM VGA Technical Reference, CRT Controller Address Register, decodes bits 4–0: these select 06h/08h/03h, not 0Dh/0Fh/0Ah as modulo 25 did. Display start and cursor remain zero. At that stage the guest bytes were unchanged; the review fixture below restores end-to-end start/cursor coverage. |
| Four-cell renderer aperture, start=2, cursor=0 (review correction) | Visible at (0,1) → hidden on a 2×2 grid | IBM VGA Technical Reference, Cursor Location: the full 16-bit cursor location compares with the address counter. Scanout's CPU-aperture wrap is an explicit modelling assumption; it must not alias cursor addresses. |
| VGA start/cursor guest (review correction) | Guest indices 26h/28h/23h → 2Dh/2Fh/2Ah; blank at (0,0), cursor (0,0) visible → `A/4E` at (0,0), cursor (2,0) hidden | The revised guest decodes to 0Dh/0Fh/0Ah using bits 4–0, exercising start address, cursor location and disable end to end. The separate device test still rejects modulo-25 aliasing. |
| Empty bay, SEEK head 1 to cylinder 90 / RECALIBRATE from PCN 90 | SIS `6D 00` → `25 5A`; `69 5A` → `71 00` | RDY is held active by the AT board model. NEC µPD765A, SEEK / RECALIBRATE: SEEK advances PCN without position feedback; RECALIBRATE clears PCN first and reports SE+EC after 77 failed TRACK 0 steps. Mock-bus regressions failed on HEAD with the old pairs. |
| Empty bay, READ ID / READ DATA / WRITE DATA | Immediate abnormal/not-ready result and IRQ → execution waits without a result, IRQ or DMA until reset | NEC µPD765A, READ ID / READ DATA and ST1 MA: an unsuccessful ID search needs index pulses to terminate. An empty slot supplies none. The BIOS can time out in software and reset the controller; all 13 fixture-required slow tests, including BIOS POST, BIOS floppy and DOS goldens, pass without changing their assertions. |
| Isolated RTC register C, periodic rate enabled with PIE clear | PF=0 → PF=40h | Motorola MC146818A datasheet, Register C and Table 5: PF latches independently of PIE. Calendar/update/alarm timing is unchanged; the new oscillator phase is independent of SET. |

Board evidence for the empty-bay and DOR corrections: IBM **Personal Computer
AT Technical Reference**, [Fixed Disk and Diskette Drive Adapter](https://www.minuszerodegrees.net/oa/OA%20-%20IBM%20PC%20AT%20Fixed%20Disk%20and%20Diskette%20Drive%20Adapter.pdf),
Digital Output Register (3F2h), p. 12; Digital Input Register (3F7h), p. 13;
Interface Lines, p. 40; logic diagrams, sheets 4–5 of 9. Pin 34 is Diskette
Change, read at 3F7h bit 7. ASSUMED: RDY remains active independently of the
selected slot; the available sheet-4 scan does not unambiguously resolve the
electrical tie. This is consistent with the existing four ready-change reset
polls. Command/PCN/status evidence is NEC **µPD765A/µPD7265**
[datasheet](https://hxc2001.com/download/datasheet/floppy/thirdparty/FDC/NEC/upd765A.pdf),
RESET, SEEK, RECALIBRATE, READ ID, READ DATA, SENSE INTERRUPT STATUS and ST0/ST1.
No step-pulse delay or rotational phase is added.

The passive VGA isolation fixture now exercises ignored upper index bits
(`EAh → 0Ah`) instead of modulo-25 aliasing (`23h → 0Ah`). The idle-SIS fixture
seeks mounted drive 0 instead of empty bay 1; its successful cylinder-25
completion remains the prerequisite for testing a later idle SIS.

PIT and RTC snapshot versions move from 2 to 3 to carry, respectively, the
current reload and incomplete LSB, and the fractional oscillator/divider phase.
VGA version 3 restricts the saved CRTC index to its five-bit hardware domain.
Restore rejects malformed phases/indices before mutating device state. UART
output and snapshot format follow the observation model (register-only v3).
The FDC review correction uses snapshot v3: the IRQ byte is the pre-gate
interrupt latch and execution may wait for index without a DMA context. Restore
rejects earlier versions and inconsistent wait states.

The BIOS-floppy divergence is excluded from classes (a)/(b): both engines use
the same corrected timing, and the cause was a CPU-core defect, fixed upstream
and re-pinned, not a scheduler difference.
Removing the Long64 `hello` demo/golden and replacing no-argument output with
usage is an intentional interface change, not a hardware expectation change.

**MEASURED policy calibration** with CPU `44403dd`, the pinned fixtures and
PIT-driven port 61h: the historical Bare POST rig uses rate 1/1; FreeDOS uses
the calibrated AT rate 8/13. Counts below are completed-step equivalents,
including halted time, not host duration. First visible milestones were narrowed
to a 10,000-step interval and replayed with one-step checks; completion requires
the full listing/free-space text and final prompt, not the end of a budget window.

| Policy | Observed milestone | Bound and margin |
|---|---|---|
| FDC write/readback | `W` OUT starts at 3,673; marker absent at 3,673 and present at 3,674 | 5,000: 1,326 steps after completion |
| test386 | Unsupported instruction after 159 completed steps (159 retired instructions) | 1,024: 865 steps |
| Bare BIOS POST | Final POST OUT starts at 8,024,894, completes at 8,024,895; `Boot failed` first appears at 12,750,391 | 20,000,000: 11,975,105 after POST; 7,249,609 after the visible failure milestone |
| FreeDOS prompt | First `Enter new date` text at 9,461,624; the 1,000,000-step poll observes it at 10,000,000 | 538,376 steps of polling overshoot |
| DIR and file-write/DIR | Injection at 10,000,000; DIR completes at 10,945,308 (945,308 steps), write/DIR at 11,167,482 (1,167,482 steps) | Command budget 3,000,000: 2,054,692 steps of DIR margin, 1,832,518 of write/DIR margin |
| Overall BIOS/DOS | Latest measured command completion is write/DIR at 11,167,482; the fully reserved command window ends at 13,000,000 | 20,000,000: 8,832,518 after actual completion, 7,000,000 after the reserved window |

**2026-11-12 FDC program update:** the guest now selects 250 kbps before DD
media access, adding six bytes and three instructions. The current `W` OUT
starts at 3,676 and completes at 3,677; the 5,000-step cap leaves 1,323 steps.
The row above preserves the earlier 3,673/3,674 calibration and 1,326-step
margin with CPU `44403dd`; the other calibration values remain historical.

The policy checks remaining room before injecting keys. With a 20,000,000-step
cap and 3,000,000-step command budget, a prompt observed **after 17,000,000**
produces `InsufficientCommandBudget` (`PolicyFailed` in the frontend), never a
shortened command. **MEASURED boundary check:** delaying the first poll to
17,000,000 succeeds and completes DIR; delaying it to 17,000,001 returns
`required=3,000,000, remaining=2,999,999` before injecting keys. A separate check
at the normal 10,000,000 prompt rejects a 12,999,999 total cap and accepts
13,000,000. These are policy limits, not command-completion milestones.

Screens, serial/POST bytes and media contents retain independent golden
assertions. The production one-million-step cadence captures DOS time
`12:00:12.90 am`; polling/injection changes can move that clock-derived text.

The informative release wall-time test requires
`UNEMUPC_SCHEDULER_WALL_TIMES=1`; otherwise it prints one skip line. It measures
the two corrected engines outside execution, using medians of three. Phase
profiling is separate from scheduler correctness.

The sections below preserve the proposal and migration history. The accepted
review outcome and adoption above supersede earlier proposed representations
and legacy compatibility paths.

## Answer and limits of the hypothesis

Replace the fixed ratio and global per-clock sweep with a rational instruction
clock, independent device clock domains, conservative next-event deadlines, and
CPU execution bounded by **retired instructions**. Service IRQs at architectural
instruction boundaries, acknowledging the PIC only when the CPU can accept the
interrupt immediately and the stopping instruction's bus effects have completed.
Keep a per-PIT-clock fallback for devices not yet migrated.

This makes timing correct **within an explicitly instruction-timed model**:
device periods do not depend on batching, and interrupt acceptance does not
depend on translated blocks. It does not model instruction cycle counts, ISA
bus cycles, DMA contention, or physical interrupt-entry latency. A faster CPU
profile means more retired instructions per unit of device time, not a claim
that every instruction takes one physical CPU cycle.

Two qualifications to the hypothesis matter:

- A deadline inside an instruction cannot stop that instruction halfway through
  in this model. Device transitions retain their scheduled timestamps; the CPU
  observes their effects at the first instruction boundary at or after the
  deadline. Execution must never pass that boundary before servicing them.
- Same effective CPU speed does **not** imply identical old goldens when IRQ
  latency and INTA ordering are corrected. An optimization-only mode must be
  identical. The corrected mode needs a separate, reviewed behavioral gate.
  Preserving block-dependent IRQ latency would defeat the purpose of the change.

Throughput is secondary. The retained
[machine-loop measurements](machine-time-profile.md#results) attribute
86–89% of real DOS workload wall time to the CPU and 11–13% to clock work;
the BIOS INT 19h spin is the outlier with roughly half in clock work. Those are
READ evidence from that experiment, not measurements repeated in this spike.
Its optimistic ceilings from removing the entire clock phase are not scheduler
speedup predictions. More frequent CPU returns can cost more than skipped ticks
save, especially while legacy devices still request every PIT boundary.

## Current contracts and gaps — READ evidence

Paths and line numbers in this section refer to the inspected source before
implementation of this proposal.

| Source | Observed behavior | Implication |
|---|---|---|
| `crates/machine/src/lib.rs:61`, `:398` | `RETIRED_PER_PIT_CLOCK = 1`; `advance_clock` carries a remainder but divides by the constant | CPU speed is fixed inside the machine |
| `crates/assembly/src/clock.rs:16`, `crates/assembly/src/assemble.rs:183` | The config declares `retired_per_pit_clock`, but assembly passes only `quantum` to the machine | Changing the ratio has no effect; zero is not validated |
| `crates/cpu/src/lib.rs:138`, `crates/cpu/src/x86jit_backend.rs:202` | `Cpu::run` forwards a **block** budget to x86jit | Neither deadline nor cap can be bounded by that number of instructions |
| `crates/machine/src/lib.rs:342` | CPU runs first; only then are elapsed PIT clocks replayed | Device edges can be latched after the CPU has passed their eligible acceptance boundaries |
| `crates/machine/src/lib.rs:347`, `:532` | Advance clocks, acknowledge/inject, then dispatch the stopping OUT/IN/MMIO | OUT-before-INTA is violated; queue emptiness substitutes for CPU acceptance readiness |
| `crates/machine/src/lib.rs:310`, `:255` | HLT advances one PIT clock at a time; virtual time includes halted clocks | Preserve the halted contribution and cap semantics when skipping idle time |
| `crates/machine/src/lib.rs:434`, `crates/bus/src/lib.rs:268` | Every PIT clock ticks every device and samples wired IRQ levels | A large `tick_all(now)` would lose short pulses |
| x86jit `x86jit-core/src/vm.rs:1889`, `:1963` | Budget tests count blocks; IRQ readiness is checked between interpreted blocks | A retirement counter alone does not provide an instruction-exact stopping point |
| x86jit `x86jit-core/src/vm.rs:1618` | `step_instruction` exists, but explicitly does not resume deferred accesses as `run` does | A machine-side loop around this function is not a complete scheduler seam |
| x86jit `x86jit-core/src/interp/mod.rs:295`, `crates/cli/tests/quantum.rs:48` | A self-jump `JMP $` is deliberately not counted as retired; the cap test avoids it | Exact retirement accounting must fix this, or that guest defeats a time cap |

The x86jit references above are to the dependency's **own** source tree at the
pinned revision; no dependency files were modified. Its `executed` counter is
not a replacement for the Real16 retired-instruction contract.

## Time model

### Units and configuration

Use a new scheduler type, provisionally `VInstant(u128)`, with reset at zero.
One PIT input period contains `S = 2^32` base units. A PIT edge at input count
`k` is exactly `k * S`; do not use a rounded nanosecond period. A 128-bit
timestamp avoids the short horizon a 64-bit count of such fine units would have.
All addition, multiplication, conversion, and deadline arithmetic is checked.
Exhausting a supported range returns a deterministic error, never wraps time.

Keep today's `VClock(u64)` meaning **PIT input counts** during migration. Do not
silently reinterpret it as the finer unit. Legacy trace comparison projects
`VInstant` to `floor(now / S)`; a later versioned trace format records full
precision. Both formats identify their units. The old wrapping `VClock::advance`
is not the scheduler's arithmetic policy.

`ClockConfig` should describe:

- `cpu_rate = A / B`, positive reduced rational **retired instructions per PIT
  period**. The compatibility default is `1 / 1`. An existing positive
  `retired_per_pit_clock = r` converts to `r / 1`; the machine must actually
  receive it. A rational field also permits slower fractional profiles.
- The modeled PIT frequency as a positive rational Hz value. The default RTC
  uses exact OSC = 315/22 MHz divided by twelve (13,125,000/11 Hz), from
  the IBM PC AT Technical Reference, System Board, System Timers and Oscillator.
  Calendar phase uses 11 units per virtual tick at the current scale; legacy
  1- and 3-unit snapshots restore at their saved ratio. Explicit RTC timing
  overrides retain their configured virtual-tick periods.
- Per-device clock-domain configuration and phase origins, constructed by the
  assembler and passed through generic device/scheduler seams.
- An explicit temporary `LegacyBoundaries` versus `InstructionDeadlines` mode
  during migration. The former alone retains a block quantum as guest timing.

In the corrected mode, a positive finite `max_instructions_per_call` is a host
cooperation bound. It may shorten a call but cannot delay events, an IRQ window,
or a policy target. Test values including 1, 64, and 65,536 for equivalent state.
Move this cooperation bound to run policy when the legacy mode is removed;
update the contrary statements in `design/machine-config.md` and
`assembly/src/run.rs` at that step. Do not keep calling a block budget an
instruction budget, or silently change the unit of the existing field.

CPU speed is a modeling ratio. Profiles for later CPU generations can select
different ratios without changing device frequencies, but that does not enable
386/Pentium ISA behavior or give a measured MHz rating. CPU semantic profiles,
new audio devices, and wall pacing are outside this implementation.

### Conversions, rounding, and phase carry

The CPU instruction period is `S * B / A` base units. A device domain at
`f_device` Hz has period `S * f_pit / f_device`, reduced to a rational `N / D`.
For example, RTC oscillator and audio sample domains can use 32,768, 44,100, or
48,000 Hz independently of the CPU ratio. No least-common-multiple of all future
frequencies is required.

Represent a domain's exact phase by an integral floor and remainder
`0 <= r < D`. Round an event's absolute timestamp **up** to the first base unit
at or after its exact phase:

```text
event(k) = epoch + ceil(k * N / D)

x       = r + n * N
delta   = floor(x / D) + (x % D != 0) - (r != 0)
r_next  = x % D
```

The second formula advances an already upward-rounded timestamp by `n` periods.
Carry `r_next`; do not repeatedly add `ceil(N / D)`. Compute division ceilings
using quotient and nonzero remainder rather than unchecked `x + D - 1`.
Splitting 65,536 instructions into arbitrary calls then produces the same time
as one call. Rounding error is less than one base unit relative to the exact
domain phase and does not grow with event count. Equal rounded deadlines use
the stable ordering below. Validate that each configured base period is at
least one base unit and all reduced intermediates fit the checked arithmetic;
reject unsupported ratios instead of creating zero-duration CPU instructions.

PIT counter division happens in the PIT domain, not by separately rounding the
counter's frequency. RTC divider stages similarly count oscillator edges.
Domains keep their phase when unrelated devices are accessed or the CPU stops.
A register write restarts a divider only when the device's hardware model says
so. Scheduling the next period from the late **observed** CPU boundary would
introduce drift and is forbidden.

Running time advances only for completed guest instructions. IRQ entry itself
has no retired charge. During HLT, advance the scheduler instant directly to
the next event or bound, without inventing instructions; retain the CPU's
fractional instruction phase for resumption. An integer halted interval shifts
the CPU's time origin and does not consume that phase remainder.

Preserve the policy API's instruction-equivalent counter separately:

```text
virtual_time = retired + floor(halted_base_units * A / (S * B))
```

Carry the fractional halted contribution across every idle jump and runner
call; never sum rounded per-jump contributions. At the default speed with
whole-PIT idle steps this is exactly `retired + halted_pit_clocks`, as today.
Other ratios give a documented generalization, including fractional halted
instruction equivalents. Keep `retired` available as an independent CPU count.
Do not substitute `floor(now * A / (S * B))`: independent rounding of active and
halted time can disagree with the actual number retired.

Freeze a machine's timing configuration after reset. Runtime speed changes
would require explicit epochs and a new policy for cap-unit conversion; they
are not implied by configurable construction-time profiles.

### Deadlines versus instruction boundaries

Given an event deadline `d > now`, find the smallest positive instruction count
`n_event` whose rounded CPU completion time is at least `d`, using the same
phase state as forward conversion. Checked integer inequality inversion, or a
bounded monotone search, suffices; a floating-point reciprocal does not.
Run at most that count, further limited by cooperation and policy caps.

If no instruction completes exactly at `d`, the last instruction straddles it.
After its completion, process **every** device transition due through that
boundary in chronological order before executing another instruction. In
particular, a low and a subsequent high within one slow instruction must both
reach the PIC inputs in order. Do not merely sample the final level. IRQ acceptance
lateness is less than one modeled instruction interval when IF and the
architectural interrupt gates are open. Masking and shadows may extend it.

RAM effects of the straddling CPU instruction precede deferred DMA memory
effects at that observation boundary. This is an explicit atomic-instruction
ordering convention, not sub-instruction DMA arbitration. Bus accesses use the
instruction-start timestamp described below; interrupt acceptance uses its
completed boundary. This convention also covers instructions that fault after
an access without charging time for a retirement that did not happen.

## Device and bus contracts

### An additive migration API

A bare `next_event() -> Option<...>` default cannot distinguish an unported
device from a device that is genuinely dormant. Use a small explicit enum:

```rust
enum NextEvent {
    EveryPitClock,
    At(VInstant),
    Dormant,
}

// Additions to Device; signatures are a design sketch.
fn next_event(&self, now: VInstant) -> NextEvent {
    NextEvent::EveryPitClock
}

fn advance_to(&mut self, now: VInstant) {
    self.tick(pit_projection(now));
}
```

`EveryPitClock` schedules the **next absolute PIT boundary**, not `now + S`.
For this variant call the existing `tick(VClock)` exactly once at each such
boundary, in existing device order. Its default is conservative, preserves
call-sensitive devices, and keeps unit-test/mock implementations working.
It deliberately prevents skipping those PIT boundaries until the device opts in.

An opted-in scheduled device overrides `advance_to` and promises:

1. `At(t)` is the earliest future autonomous transition that the machine must
   observe: IRQ level change, DMA request, output/sample, or another scheduled
   state transition. Earlier conservative deadlines are permitted; late ones
   are not. A pending PIT reload is an example of useful conservative scheduling.
2. `Dormant` means no autonomous event until a write, read side effect, external
   stimulus, reset, or coupled input changes the state. It does not mean that
   time freezes or an unlatched counter cannot be read later.
3. `advance_to(now)` advances elapsed state, is idempotent at the same instant,
   and supports lazy synchronization before reads/writes. It cannot skip a
   promised observable transition. The machine dispatches such transitions at
   their individual deadlines, including when several precede one CPU boundary.
4. After due transitions are settled, `At(t)` must satisfy `t > now`. Querying
   does not mutate state or consume an event. Immediate consequences of a bus
   operation are settled at its current timestamp, then a future deadline is
   returned. An unchanged due-now deadline is a contract error, not a busy loop.

Remove per-clock loops inside an opted-in device only after comparing its
arithmetic advancement to its original single-edge implementation. A PIT
`next_event` wrapper around today's `tick` still loops over every elapsed input
clock internally; that reduces visits to other devices but is not yet a fast
PIT implementation.

### Access synchronization and invalidation

The adopted whole-PIT-clock implementation and declared access coupling order
are described in §Adoption. The access-context sketch below is proposal history.

Separate **access timestamps** from timer advancement. Add an explicit
`DeviceAccessContext { now: VInstant, pit_clock: VClock }` to bus-dispatched
port/MMIO access methods and update trace-producing implementations to use it.
The bus stamps open-bus traces from the same context. Initially their existing
`TraceEvent::at` stores the PIT projection for A/B comparison; full precision
requires a versioned format. Never call an old device's `tick` merely to update
a trace timestamp: `I8042::tick` changes state even when elapsed time is zero.

Before a scheduled device access, drain global due events, then synchronize the
target with `advance_to(access_time)` so a PIT count or RTC register reads its
current value. A legacy target has already received all PIT ticks and needs no
extra advancement. Apply the access and its completion, sample changed IRQ
levels, invalidate affected deadlines, and settle immediate work. Reads can
invalidate deadlines too: consuming an 8042 byte and clearing RTC register C
are important examples. Split byte-wide ISA accesses retain bus byte order.

Also invalidate after DMA completion, controller changes, external input,
coupled signals, reset, and restore. The existing mutable-device escape hatch
must become a time-stamped mutation operation or use a guard that invalidates
on release; otherwise `push_scancode` can leave a cached `Dormant` entry forever.
The machine still knows devices only by stable wiring identity. Assembly owns
clock-domain and signal wiring; no PIT/RTC downcast enters the machine loop.

Start with one deadline slot per device and a stable linear minimum scan: there
are only 10–11 devices in the measured machines. Requery after mutations; do not
query every device on every PIT clock. A heap with generation-tagged stale-entry
rejection is justified only by a measured larger device set. Stable device IDs
and wiring order, never hash-map iteration, define equal-time order.

### Device migration inventory

| Device and current source | Required scheduling behavior | Fidelity boundary |
|---|---|---|
| PIT, `pit8253.rs:275`, `:486` | Schedule load edges and both OUT transitions; lazily synchronize all three counters for count reads/latches; include any routed channel-1/2 outputs | Preserve currently implemented modes and count wrapping before fixing other hardware deviations |
| RTC, `cmos_rtc.rs:165`, `:669` | Schedule UIP entry and second/update/alarm boundaries; synchronize before register access, reschedule on SET and timing writes; sample register-C clear immediately | Default second uses exact OSC = 315/22 MHz divided by twelve (13,125,000/11 PIT clocks), with carried calendar and 32,768 Hz periodic-divider remainders; retain the approximate 1193-PIT-clock UIP window and explicit virtual-tick timing overrides. SET freezes only the separate calendar phase, not the periodic divider |
| Keyboard, `i8042.rs:341`, `:492` | Preserve hold at the first PIT boundary after data read and refill at the second when enabled/queued; reschedule on data read, command-byte write, enable, and host injection | Two modeled PIT steps are an edge-visibility policy, not a measured keyboard serial baud rate |
| FDC, `upd765.rs:65`, `:882` | `Dormant` between command/access/DMA work; service request and completion at the instruction boundary; timestamp accesses directly | Seek/read/write timing remains immediate; no rotational-latency feature is added |
| DMA, `dma8237.rs:466` | `Dormant`; channel programming invalidates affected transfers; completion can create immediate IRQ work | Whole-transfer service is retained; arbitration, per-byte clocks, and CPU stalls are deferred |
| System control, `sys_control.rs` | `Dormant`; accesses synchronize the shared PIT, sample refresh/OUT2 and drive GATE2 | PIT snapshot owns the refresh flip-flop; assembly couples system-control accesses to PIT deadline invalidation |
| PIC/shared PIC, UART, POST, VGA | Opt into `Dormant` only after confirming tick merely stamps time; route access/cascade effects immediately | UART serialization and VGA retrace clocks remain absent; VGA's existing per-read status phase stays unchanged |

For PIT Mode 2, the one-clock low pulse and following rising edge must both be
scheduled. Mode 3 needs both half-cycle edges, including odd divisors and the
one-clock load delay. These requirements follow the Intel **8254 Programmable
Interval Timer** datasheet, “Mode 2: Rate Generator” and “Mode 3: Square Wave
Mode,” pp. 10–13 of the
[datasheet](https://www.scs.stanford.edu/10wi-cs140/pintos/specs/8254.pdf).
An implementation must not return only the next rising IRQ edge while leaving
the machine's remembered level high. Reset initializes remembered line levels
from actual reset pins without synthesizing a rising edge; subsequent
programming-induced changes are sampled immediately. Multiple edges while a
PIC request is already latched still obey its IRR coalescing rules.

Do not derive untested arithmetic for every PIT mode in one change. Compare
Modes 0, 2, and 3 against the reference stepper, including zero-as-65,536 and
odd/even divisors. Keep conservative per-clock scheduling for unsupported modes
and pathological small divisors until their present behavior is characterized.
Lazy synchronization must preserve latch and low/high-byte phase semantics.

The Motorola **MC146818 Real-Time Clock Plus RAM**, “Update Cycle,” “Registers
A–C,” and Table 6, defines oscillator/divider behavior and the UIP/update
relationship; see the [manufacturer datasheet](https://pccomponents.com/datasheets/MOTO-MC146818.PDF).
The present `1_193`-PIT-tick UIP window and frozen SET accumulation are emulator
approximations, not proof of that hardware timing. Scheduling must preserve
them during A/B. Periodic PF/PIE and divider phase now have separate device
regressions and an IRQ8/HLT integration test. The calendar counter and periodic
phase remain separate: SET freezes the former and divider reset clears the
latter, so their edges can lose alignment. This is an ASSUMED approximation,
not the shared-divider relationship shown in MC146818A, Update Cycle,
Figure 15 (Update-Ended and Periodic Interrupt Relationship). Accurate alignment
and divider-reset startup timing remain unmodelled.

Port `0x61` samples the shared PIT at the access clock. Bit 4 toggles on every
channel-1 OUT rising edge, including edges crossed by an arithmetic clock jump;
bit 5 samples channel-2 OUT. Bit 0 controls channel-2 GATE (Modes 0/2/3).
The PIT snapshot owns the refresh flip-flop and GATE2, while the system-control
snapshot owns its control nibble. Assembly invalidates the PIT's cached deadline
on system-control accesses through the bus access-coupling seam. Neither refresh
nor OUT2 requires an autonomous scheduler wakeup. IBM **Personal Computer AT
Technical Reference**, “System Timers,” p. 1-22 and System Board I/O, Port B,
describes the refresh and speaker wiring; see the
[1985 reference](https://www.minuszerodegrees.net/manuals/IBM_5170_Technical_Reference_6280070_SEP85.pdf).

**DERIVED — speed-check calibration:** IBM AT BIOS listing TEST1, checkpoint 11
(“VERIFY SPEED/REFRESH CLOCK RATES”) counts `IN 61h / TEST 10h / LOOPZ` and
`LOOPNZ` iterations across 256 complete bit-4 cycles. With channel 1 in Mode 2
at count 18, refresh requests occur every 18 PIT clocks; a complete bit-4 cycle
takes two requests (36 clocks). Each polling iteration retires three steps;
`DEC BL / JNZ` adds two steps per complete cycle. With throughput R steps per
PIT clock, **DERIVED** iterations per cycle are `n = (36R - 2) / 3`.
Starting at CX=0, the total iterations are `(-CX) mod 65536`; the BIOS accepts
`F8A7h ≤ CX ≤ F9FDh` (1539–1881 iterations, centred at 1710 over 256 cycles).
The centre gives `R = (3 × 1710 / 256 + 2) / 36 ≈ 0.6122`.
Initial phase and sampled final iterations add boundary effects, so calibration
uses the actual listing as well as this estimate.

**MEASURED by the supervisor:** with the 80286 core, PIT-driven refresh and
user-supplied IBM 5170 BIOS, the speed check passes from `4/7` through `2/3`.
`11/20` stops at checkpoint `04`; `27/40` stops at checkpoint `11`.
The `ibm-5170` default is `8/13 ≈ 0.6154`, near the derived centre for the
6 MHz AT. This is instruction throughput, not a table of 80286 cycle costs.
Bare synthetic rigs retain their explicit/default rates.

## Interrupt acceptance and instruction transactions

### PIC state must follow CPU acceptance

Separate three things: device IRQ line/edge, PIC pending request (INTR), and CPU
acceptance (INTA followed by IVT entry). A PIC request can remain pending while
IF is clear, provided its device input stays high. Do not move IRR to ISR, resolve a permanent vector, or populate a
CPU queue until the CPU reports an eligible architectural boundary.

The readiness predicate belongs in the CPU core. It includes IF, the STI
shadow, any modeled MOV/POP SS inhibition, no unfinished port/MMIO/retry
instruction, and no earlier acceptance in flight. `interrupts_enabled()` and
`!has_pending_irq()` together are insufficient. The hardware distinction
between IF and acknowledgement is described in Intel **80286 and 80287
Programmer's Reference Manual** (1987), §3.11.2, §5.2 and §9.2; see the
[manual](https://bitsavers.trailing-edge.com/components/intel/80286/210498-005_80286_and_80287_Programmers_Reference_Manual_1987.pdf).

At a ready boundary, poll the PIC, acknowledge its **current** winner, and enter
that vector before any next guest instruction. These operations form a
single-threaded transaction. An atomic core acceptance method must never
silently enqueue/defer a vector after readiness was established. IVT/stack
delivery faults may surface as deterministic CPU errors after INTA; do not undo
PIC history or run guest code past them. Physical multi-cycle INTA remains
unmodelled. An explicit controller acknowledge after a request disappears returns
the default IR7 vector without setting that chip's ISR7; the machine's atomic
poll/acknowledge transaction introduces no electrical race interval.

The machine forwards both transitions of each device's current IRQ output to
the PIC, at the existing deadline, access-settle and DMA-settle sampling points.
It does not synthesize a high/low pulse. Initial reset samples establish the
baseline without inventing an edge. Edge detection belongs to each PIC input;
a falling input withdraws its unacknowledged IRR bit even when masked. An
acknowledged ISR bit survives a fall and requires EOI. Holding an input high
through EOI does not request another interrupt in edge mode. ICW1 LTIM remains
unmodelled. See Intel 8259A datasheet, **Edge and Level Triggered Modes** and
**Interrupt Sequence** ([datasheet](https://www.cs.cmu.edu/~410/doc/8259A.pdf)).

PIC snapshot version 3 rejects IRR bits on low inputs and rejects version 2;
restore leaves the previous state unchanged on either error. ISR bits may remain
set after the corresponding input falls until software issues EOI.

Slave INT drives master IR2 as a level, including changes caused by IMR/EOI
port writes. If the slave request disappears before INTA, both slave IRR and
master IRR2 are withdrawn. An explicit acknowledge then returns master IR7,
leaving both ISRs unchanged. If a cascade has already been selected but its
slave has no winner, the slave supplies default IR7 with no new slave ISR bit;
master ISR2 is set and still needs EOI. The current atomic wiring cannot create
that latter race during normal machine execution.

PIT IRQ0 already exposes channel 0 OUT: Mode 2 is low for one clock per period,
Mode 3 holds each high/low half-period, and Mode 0 stays high after terminal
count until reload. Its deadlines preserve both OUT transitions. Reading count
or status does not consume IRQ0. The 8042 holds IRQ1/IRQ12 while OBF contains a
byte from the enabled source; FDC completion IRQ6 lasts until its first result
byte (seek/reset uses SENSE INTERRUPT STATUS); ATA IRQ14 lasts until normal
status read; RTC IRQ8 lasts until register C read. UART interrupts remain
unmodelled (IIR reports none), and DMA has no CPU IRQ output. No new clock visits
or per-instruction sampling are introduced.

When PIC INTR is pending but CPU readiness is false, ask the CPU to return at
the **first** eligible instruction boundary, or use one-instruction execution
until that seam exists. Checking only at the next device deadline is too late:
`STI; NOP; long_non_io_sequence` must accept after NOP even if no timer changes
again for thousands of instructions. POPF/IRET enabling IF require the same
observation, subject to the core's architectural rules. The STI shadow is
consumed by the following completed instruction, never by one whole block,
one CPU call, an I/O retry, a clock tick, or an interrupt entry.

For `STI; HLT`, HLT itself completes that following instruction. A pending
eligible IRQ then wakes at the boundary after HLT without executing the
post-HLT instruction first. IF-clear HLT returns `Halted` immediately, as today;
NMI/reset wakeups are outside current scope. Do not import later CPU inhibition
rules by guessing them in the machine: expose the gates the Real16 core models
and test their behavior there.

### OUT-before-INTA and completion ordering

At each CPU observation boundary, use this total order:

1. Account completed instructions **before** a stopping bus instruction and
   settle device deadlines through its start instant in
   `(deadline, stable_device_order)` order. After **each** transition
   propagate/sample its IRQ or coupled output, including falling edges.
2. At that start instant synchronize the bus target, perform all OUT/IN/MMIO
   transfers of the stopping instruction, and finish its completion handshakes.
   Sample resulting line changes immediately, including the low level after
   reading an 8042 byte. Never accept an IRQ inside this transaction.
3. Once that instruction completes, charge its one retired interval and settle
   due events through its completion instant. Without a bus stop, charge the
   returned retired delta and settle events through its completion directly.
   Drain DMA work in declared wiring order through `DmaMemory`/`note_write`,
   deliver completions, propagate outputs, and settle immediate follow-on work.
   Recompute affected deadlines. This preserves immediate FDC transfers before
   another guest instruction; it does not assign a physical DMA bus latency.
4. Finish due policy inputs in stable order after the completed CPU bus effect
   and before interrupt selection. A policy input becoming due inside an
   instruction waits for this boundary. Prompt-based input is still decided
   only at the policy's specified observation target.
5. Return for a reached hard cap or explicit policy observation target after
   settling that boundary, without accepting a new IRQ. Otherwise, if the CPU
   is eligible and PIC INTR is asserted, perform INTA and immediate CPU entry.
   If halted, this acceptance clears the machine's halted state and idle span.

Thus `OUT PIC_IMR, mask` can suppress an IRQ that would otherwise win at the
same completion boundary. `OUT PIC_COMMAND, EOI` clears the old ISR before
selecting the next request. Clock events at the access's **start** timestamp
precede the write; edges during its retired interval follow the write. A timer
load is eligible on the first PIT input edge strictly after the write. These
are deterministic tie conventions, not a claim of bus-cycle fidelity.
The open ordering issue `upc-kk2.8` is covered here; its fix must not be obscured
inside a tick-performance change.

The current CPU stop kinds need explicit transaction handling. OUT advances
IP and self-retires before its side effect reaches the bus; IN also advances
IP and awaits a value. MMIO can stop **before retirement**, possibly several
times for one instruction. Use one common convention: every externally serviced
access is timestamped at the **start of its architectural instruction**, after
the preceding completed instructions. Interrupt acceptance follows completion.

For a self-retired OUT/IN, account the returned prefix excluding that final
instruction, service the bus at the prefix timestamp, then charge its one
instruction interval. For pre-retirement MMIO, account the completed prefix,
service transfers at that same start instant, and resume **only that pending
instruction** until it retires or faults. Charge one interval only on successful
retirement. Completion cannot execute the next instruction. The core must
identify the prefix, access owner, and retirement phase; a guessed
`retired_delta == 0` is not enough. Intermediate CPU state is not exposed to the
embedding policy while this transaction is unfinished.

Repeated transfers retain order and the same start timestamp. A fault has no
retirement charge; completed bus side effects remain at the start timestamp.
This avoids advancing devices to a speculative end time that cannot be rolled
back if an instruction faults after consuming a device value. Reserving a
future **time** slot and ticking devices before that fault would violate the
retired-only running-time rule. Reserve budget room for completion, not device
time. Repeated zero-retire faults/requests need the progress guard below.

An input therefore samples the device at instruction start even if its
instruction spans an autonomous transition. This is an explicit approximation,
applied equally to IN, OUT, and MMIO; physical intra-instruction access timing
needs a cycle/bus model. It changes current end-of-run access timestamps in
corrected mode and requires the reviewed golden gate. The legacy comparison
mode retains current timestamps/order. This contract preserves exactly-once
completion without adding unsupported MMIO read-modify-write behavior.

## Run loop, HLT, and bounds

The following sketch uses helpers with the ordering above. Scheduler `now` and
CPU retirement state are consistent whenever a completed boundary is exposed
to the caller. A pending instruction transaction is internal to the loop.

```text
repeat:
    settle_current_boundary()  # due events, access/completion, DMA, policy input
    if reached_cap_or_policy_target(): return CapOrPolicyBoundary
    if cpu.irq_acceptance_ready() and pic.poll() is not None:
        vector = pic.acknowledge()
        cpu.accept_irq_now(vector)  # no guest instruction, no retired charge
        halted = false
        idle_elapsed = 0

    if halted:
        if not cpu.interrupts_enabled(): return Halted
        if idle_limit is None or idle_elapsed >= idle_limit: return Halted
        d = min(next_device_event, next_policy_input,
                idle_expiry, halted_cap_instant, next_policy_observation)
        if d is infinite: return Halted
        advance_halted_time_to(d)  # retain CPU phase and idle-cap remainder
        dispatch_each_event_through(d)
        continue                  # check cap, then IRQ, before idle expiry

    d = min(next_device_event, next_policy_input, next_policy_observation)
    n_event = first_completion_at_or_after(d)  # infinite if no event
    n_cap = cap - virtual_time                # exact active instruction room
    n_policy = policy_target - virtual_time   # if an instruction-unit target exists
    n = min(n_event, n_cap, n_policy, max_instructions_per_call)
    stop = cpu.run_retired(n, stop_when_irq_ready = pic.poll().is_some())
    if stop is bus access:
        service_instruction_transaction(stop) # prefix, start-time access, completion
    else:
        advance_active_time_by(stop.retired_delta)
        dispatch_each_event_through(cpu_boundary_time)
    if stop is HLT: halted = true
    handle_fault_or_progress_yield(stop)
```

`n_cap` and a pending policy observation also limit starting an instruction
transaction; zero room returns before starting one. An IRQ-window stop can
return earlier than `n`. A non-budget exit takes precedence over budget
exhaustion at the same boundary, so an OUT or HLT that uses the final slot is
still serviced exactly once. No instruction after the chosen boundary may
execute speculatively and become guest-visible.

For a policy time target in instruction equivalents, active execution uses
the same integer room as the hard cap. For an absolute device-time target,
use the same first-boundary conversion as a device deadline. Do not let a
cooperation yield trigger console polling or stimuli earlier than the declared
policy target; extra host yields must be invisible to the guest.

With halted fractional credit `h` in units with denominator `S * B`, the first
base instant reaching `m` remaining instruction equivalents is at
`ceil((m * S * B - h) / A)` base units from now. Compute this with checked
arithmetic. Include it in the minimum instead of jumping past a cap to a timer.
When halted, idle limits remain durations in PIT periods from halt/wake state;
retain elapsed fractional time across calls. A finer event can wake before one
whole PIT period passes. `None`/zero still allows an immediately acceptable IRQ
but advances no idle time. An eligible IRQ at idle expiry wins over the idle
stop; a hard cap wins over starting new IRQ entry at that same instant.

If all devices are dormant, a halted machine with a finite bound advances only
to the earliest bound, then returns; it does not invent periodic ticks. If a
legacy device remains, its next PIT boundary is still a real scheduled event.
A future deadline is not proof that HLT will wake: it may be a falling timer
edge, a masked request, or RTC housekeeping. Continue to the next event until
acceptance, idle limit, or cap. On return and reentry preserve the halted state,
line levels, domain remainders, idle span, and deadline state.

While draining a late batch, distinguish the CPU observation endpoint from the
event cursor. Advance each due device to its own scheduled timestamp and query
its next event relative to that cursor, then continue until the endpoint is
settled. Passing the already-late CPU time to every query would hide intermediate
transitions or incorrectly report a valid next edge as an expired deadline.

Bound host cooperation and detect lack of progress without using wall time:

- Finite instruction batches bound an I/O-free running guest. Correctly count
  `JMP $`; it is a completed instruction even though IP is unchanged.
- Impose a deterministic maximum on immediate transitions/completion attempts
  at one timestamp. A device returning the same expired deadline or a retry
  that makes no progress produces a diagnostic error, not a made-up clock tick.
- The CPU needs a deterministic work-yield for an unbounded chain of fault
  deliveries with zero retirement. The instruction budget alone cannot bound
  that host work. Such a yield does not age devices or consume STI shadow.
- A time cap bounds **virtual** time, not host execution cost. Do not infer a
  host runtime guarantee, particularly for one long REP instruction.

At a hard cap, settle transitions and stopping-instruction effects through the
final instant, then return. During active execution the instruction-equivalent
cap is exact; halted rounding is less than one base unit. A physical-time
target during active execution can land on the first boundary after it. This
tightens today's quantum-sized overshoot and can itself change policy/golden
endpoints. No events later than the final observation instant are drained.

## In-process device access

The loop above returns from the CPU at every port access and every access to
a trapped window, even when servicing it changes nothing but one device.
MEASURED before this seam with the host-profile counters: DOOM's demo loop
made 1.35 million MMIO returns per 50 million instructions and test386 3.55
million port returns per 80 million, 18 to 59 instructions per return. The
CPU core can instead call the embedder synchronously for chosen ranges and
continue the run (x86jit `DeviceIo`, its spec.md §5.4). The machine uses it
for the accesses in `machine::SERVED_RANGES`: the diagnostic checkpoint port
80h and the display window A0000h–BFFFFh. Other ports and windows (PIT,
PIC, 8042 among them) still return; they are later work.

The seam is backend-neutral (`cpu::device_access`): the machine declares
ranges with `Cpu::set_served_ranges` and runs `Cpu::run_steps_serving` with a
`DeviceAccessHandler`; it never sees x86jit types. A `DeviceAccess` carries
space, direction, address, width, value and `steps`, counted as
`CpuStepRun::steps` counts the matching stop (a port access includes its own
unit, an MMIO access only the units before it). The handler answers
`Handled(value)`, `Fallback` (the run returns the ordinary stop, nothing
changed) or `Fault` (the run ends with the error the handler recorded), and
may set the core's interrupt-request level (`IrqReady` at the first boundary
that accepts a maskable interrupt; the machine does not use it) or a stop position (`BudgetExhausted` short of
the budget, after any unfinished MMIO instruction completes). The x86jit
adapter registers ports as declared and only the trapped parts of MMIO
ranges; an access that does not fit one registered range, such as a word at
BFFFFh in a trapped region running past C0000h, returns as before.
Registration changes wait for a run start with no outstanding access; any
other refusal (overlap, exhausted handles) makes the run return an error.
The interrupt level is cleared when the call ends. A handler fault stays
latched in the core: later runs return the same error with no step until a
CPU reset, unlike a failed ordinary OUT, after which the CPU runs on. A
handler panic poisons the VM; its message is kept in the error.

The machine's handler (`machine::in_process`) does what the loop does for
that stop, in its order and at its virtual time: charge the steps before the
access, access the bus, invalidate deadlines, charge a port access's own
step, settle the accessed devices and their dependents and sample the wired
IRQ lines. It declines before changing anything when equality cannot be
proven:

- the access's steps would reach the next device deadline (the last step of
  a run bounded by it), whose ticks can need DMA through the CPU's memory;
- a cycle reaches a DMA requester or controller or one of their access
  dependents (a word OUT to 80h reaches the page register at 81h), or a
  requester already reports DMA pending;
- the bus would fault (undecoded MMIO, or an undecoded port under the
  faulting policy);
- it is a port write and a memory-map hook is attached, since the hook runs
  after every port write with the CPU in hand (EMS frames, speaker trace);
- an exception or interrupt delivery owns the transfer.

What only the loop can do with the CPU after an access ends the run at the
next boundary instead (after an MMIO access's instruction completes): an A20
line change after a port write (the loop syncs the gate), a raised reset
signal (the loop checks it first), and a request pending at the PIC (the
loop's own acceptance check runs there, as after the ordinary stop). The
core's interrupt-request level is not used for the last: its ready stop also
waits for a held single-step trap, a waiting event and an unreported
external numeric error, so it would acknowledge later than the loop. A
deadline the access moves earlier than the run's budget becomes the run's
stop position, and a failed deadline query also stops the run, so the loop
reports it after its reset and cap checks. The coprocessor acknowledgement
the loop polls after every return comes only from port F0h, which is never
served. After the run the loop charges only the steps the handler did not,
then services the stop as before; after a failed served access it also syncs
a changed A20 line and polls that acknowledgement, as after a failed stop.

An access costs one deadline query, made after it: the step at which the
clock reaches the next deadline is kept (from the loop's budget query for a
run's first access), so the check before an access is a comparison, and the
charge before it only moves the machine and bus clocks
(`Bus::advance_quiet`): nothing is due and every device's answer is cached.
`Machine::served_accesses` counts served port and MMIO accesses without the
host-profile feature.

Serving a register device that can move or raise deadlines (PIT, PIC, 8042)
needs work this stage does not do, because these hold only for the ranges
served now:

- one charge never ticks through several deadlines: the handler's charge
  after an access does not service DMA between them, as the loop does;
- DMA that becomes due during a served MMIO access is serviced after that
  instruction's step instead of before it;
- the delivery tap reads the controllers' vector bases when it drains a
  slice, which is right only while PIC ports always end a slice;
- MMIO registrations are computed from the trap layout once and never
  follow a later remap;
- the code trail samples slice boundaries, so it differs with serving on.

Observation runs after every CPU return as before. Fewer returns mean longer
slices, never longer runs: a run is still at most its budget, so the
delivery log's capacity bound (one delivery per step, `DELIVERY_LOG_MAX`
against the default 65,536-step limit) is unchanged. The host-profile events
`ServedPortIo` and `ServedMmio` count served accesses next to the return
counts.

`Machine::set_in_process_devices` (on by default) switches serving off, so
one workload can run both ways. The `support/serving.rs` lockstep comparison
in `pc-harness` runs test386, the Mode X, scrolling and mode 0Dh/13h VGA
goldens and the open-BIOS FreeDOS boot both ways and requires the same
outcome, counters, observed bytes, frames and device traces at every poll and
the same devices, RAM, processor state and delivery log at the end, and
requires that the served way served something.

## Determinism and future consumers

The execution state consists of CPU architectural state/retirement, scheduler
instant, rational phases, retired/idle cap credit, halted/idle state, device
states, remembered IRQ levels, stable wiring order, and timestamped input
cursor. Reset/replay must reconstruct all of it. Future snapshot work must
include phases and pending instruction transactions, or restrict snapshots to
settled boundaries; implementing machine snapshots remains deferred.

All event ordering is integer and explicit. No wall clock, RNG, asynchronous
input arrival, thread scheduling, collection hash order, or host batch size
decides guest-visible behavior. Record external inputs with virtual timestamps
and an ordering index. Repeated runs with the same pin, config, model version,
fixtures, and input schedule have the same output and timing state.

Audio rendering can schedule samples in its own rational domain and buffer them
for the frontend; host playback cannot clock the machine. Wall pacing belongs
in a frontend wrapper comparing a returned virtual instant with elapsed host
time, sleeping or dropping presentation work there. It never changes the
configured ratio, skips device events, or feeds elapsed wall time back into
device advancement. Interactive live input is recorded at explicit virtual
boundaries for replay; deterministic tests supply the record directly.

## Alternatives considered

| Alternative | Where it wins | Where it loses / disposition |
|---|---|---|
| Keep single-clock device ticks, batch the outer loop | Lowest migration risk; reference oracle for device transitions; can hoist empty-device visits | Does not repair block-dependent CPU latency or eager INTA. Retain as reference and default adapter, not final timing architecture |
| Jump to next event using the existing block budget | Small apparent implementation; HLT skipping can already be correct with conservative device deadlines | Running CPU can overshoot by variable instructions, alter registers/RAM before the IRQ should enter, and lose pulses if only final levels are sampled. Cannot rewind side-effecting guest execution; reject for corrected running time |
| Set block quantum to 1 | More frequent dispatcher checks without a core API change | One block still contains variable instructions, and shadow retirement can consume a block. Not an instruction guarantee; unsuitable as the correctness gate |
| Drive the existing single-step primitive from the machine | Useful independent oracle and possible diagnostic reference after completion/IRQ gaps are closed | Existing primitive does not supply the full run/retry/acceptance contract; repeated decode/entry may be costly. Request the complete seam in x86jit instead of reimplementing it here |
| Cycle-approximate instruction/bus timing | Can model long REP, variable instruction costs, DMA steals, and calibrated CPU generations more faithfully | Requires per-op costs and hardware oracles; changes decision-5 and golden meaning. Scheduler domains can support it later, but no uncalibrated cycle table belongs in this spike |
| One integer nanosecond clock or one LCM clock for every oscillator | Familiar units or exact periods for a fixed set of frequencies | Rounded per-period nanoseconds drift without carry; a universal LCM grows with future domains. Fixed fine units plus rational phases give bounded rounding without constraining domain choice |
| Tick devices from host/audio callbacks | Convenient pacing | Makes execution depend on wall scheduling. Reject; presentation consumes virtual-time output instead |

## Migration and A/B gates

Historical sequence, completed by the adoption above.

First freeze fixtures, CPU pin, config, block quantum, policies/input cadence,
and model version. Use two orthogonal comparison axes: **device dispatch**
(single-clock reference versus scheduled) and **CPU/IRQ semantics** (legacy
boundaries versus corrected instruction boundaries). Never change both axes
and then call a changed output an optimization regression.

1. Capture a complete baseline for every golden. Add the timing/IRQ diagnostics
   needed to locate a first difference: retired count, device/PIT time, virtual
   cap credit, stop reason, ordered accesses, line transitions, INTA vectors,
   PIC IRR/ISR, and relevant final device/CPU/RAM state. Keep wall timers outside
   execution. Existing output-only goldens are necessary but not sufficient.
2. Wire and validate the existing ratio into the machine, preserving `1 / 1`.
   Isolate this from deadline execution. Default-profile **all-golden equality**
   is mandatory; tests must show non-default ratios actually alter device time.
3. Add fine time/domain arithmetic, deadline slots, access context, and the
   default adapter under an experimental scheduler selection. Keep legacy CPU
   boundaries and acceptance order on the reference axis. Gate every stage on
   **all-golden equality**, including projected trace times and policy endpoints.
   In this mode replay clock events and DMA at the old service points; do not
   accidentally introduce the corrected ordering through a helper refactor.
4. Convert timestamp-only devices, then PIT, RTC, and 8042 one at a time. Compare
   each against its isolated reference stepper as well as every machine golden.
   Pending DMA remains serviced at legacy points in this comparison mode.
5. Land the requested Real16 seam in x86jit under that project's full gate,
   then re-pin its exact main revision here. Re-run **all** fast and slow
   goldens even before enabling corrected scheduling; include self-jump and
   retirement-accounting changes in the pin review.
6. Enable instruction deadlines and acceptance ordering together in an opt-in
   corrected mode. First compare the scheduled device engine with a
   single-clock **reference using the same corrected CPU/IRQ semantics**:
   all-golden equality is mandatory on that axis. Also compare corrected mode
   against the frozen old baseline for every golden. Report **identical or
   different**, first divergence, and its causal explanation individually.
   Do not regenerate goldens automatically. Review and accept intended timing,
   INTA, and tighter-cap changes with the new model identity.
7. Make corrected mode the default only after review of every golden and
   focused regressions below. Delete legacy production scheduling separately;
   retain a small single-clock test oracle. Adjust config/policy docs and
   record the decision amendments as part of that transition.

The gate inventory includes serial, timer boot, keyboard, RTC/cascade, VGA,
synthetic floppy boot, FDC write/readback, and test386, plus ignored BIOS POST,
BIOS-to-floppy boot, FreeDOS prompt, interactive DIR, and DOS file-write
sessions. Also compare every other golden found in the workspace, including
device golden tests. Fix the x86jit revision for each comparison. Exercise
cooperation sizes 1/64/65,536 in corrected mode with the **same** policy targets;
they must not change any observables. Old baseline quanta remain fixed.

Focused implementation tests, to be added with the corresponding changes:

| Test | What it must prove / expected baseline gap |
|---|---|
| Ratio validation and partitioned conversion | Zero/overflow rejected; ratios 1, 3, and 7/3 honored; arbitrary batch splits preserve phase; current assembly ignores the ratio |
| PIT isolated stepper versus deadlines | Load delay, Mode-2 low and rising edges, Mode-3 odd/even halves, latch/read sequencing, zero divisor, reprogramming; naive bulk final-level sampling loses edges |
| RTC boundaries and SET/read-C writes | UIP entry/exit and second/update/alarm sequence match reference; read-clear falls are observed even if immediately followed by other work |
| 8042 read/hold/refill and host injection | Exactly two modeled PIT boundaries after read, low sampled immediately, dormant queue rearmed on injection; an extra same-time sync must not release hold |
| Ready IRQ behind IF=0 | IRR remains pending and ISR unchanged until actual CPU acceptance; eager `service_interrupts` fails this assertion |
| `STI; NOP` and `STI; HLT` with pending INTR | Accept after the following instruction, independent of block shape and call size; no post-HLT instruction before handler |
| IMR/EOI OUT coincident with pending IRQ | Stopping OUT reaches PIC before INTA chooses winner; current run loop acknowledges first |
| MMIO/IN/OUT transaction at deadline/cap | All accesses use instruction start; value/completion and retirement precede INTA; multiple transfers charged once; post-access faults do not advance time for an unretired instruction |
| Falling and rising edges inside one slow instruction | Both reach PIC in timestamp order; final-level-only sampling is insufficient |
| HLT with cap before event, IF clear, idle expiry, repeated runner calls | No CPU retirement while idle; no jump past cap; IF-clear stops without tick/INTA; phase and idle credit preserved |
| Self-jump and zero-retire fault progress | `JMP $` consumes instruction budget/time; repeated fault/retry work yields or errors without inventing time |
| Configured slow/fast CPU and sample domains | Same physical-domain phase for a given virtual instant; CPU instruction counts differ by configured rational rate |

Device tests use `crates/testkit` mock-bus isolation; machine tests use mock CPU
and controller plus small Real16 ROMs for the instruction/IRQ seam. For each bug
fix, run its new regression against the preceding implementation and retain the
actual failure evidence. The predicted gaps above are **READ-derived test
requirements**, not claims that new tests were run in this spike.

## Spike validation — MEASURED versus unmeasured

Only this document was changed. No device/prototype tests were added because
no executable implementation was changed. No scheduler prototype was built:
the current CPU API cannot validate instruction-exact scheduling, and a PIT-only
wrapper would not test IRQ correctness. No scheduler A/B result, speedup, or
old/new golden equivalence is claimed.

The following baseline commands were run offline in the project Nix devShell
with both `cc` and `cargo` verified under `/nix/store`:

| Command | Measured result |
|---|---|
| `cargo build --offline --all-targets` | PASS, exit 0 |
| `cargo test --offline --workspace` | PASS, exit 0; 326 parent-suite tests passed, 0 failed, 5 ignored; two watchdog child tests also passed and are not counted twice |
| `cargo clippy --offline --all-targets --all-features -- -D warnings` | PASS, exit 0 |
| `cargo fmt --check` | PASS, exit 0 |

An ephemeral Python integer/Fraction check ran 193 assertions for the proposed
carry formula, batch partitioning, rational RTC/audio rounding, and first-boundary
selection. Rates checked were 1, 3, 7/3, 1/3, and 65,537/1,000 retired per PIT;
sample domains were 32,768/44,100/48,000 Hz. All passed. This validates arithmetic
examples only; it does not test Rust overflow handling or emulator behavior.

Slow ignored goldens were **not run**: this is documentation only, there is no
prototype to compare, and the worker command restrictions do not name an ignored
test to execute. All-golden A/B and performance remain proposed implementation
gates. Suitability of the API/performance tradeoff is a design inference;
hardware fidelity beyond the listed contracts remains to be tested.

## Proposed implementation beads

These historical titles describe proposed scopes, not an open-work inventory.
Order is dependency order; the existing INTA-ordering issue should be linked
to item 7 rather than duplicated.

1. **Capture scheduler timing and IRQ reference observations** — Freeze every
   golden's full behavior and add an isolated single-clock device oracle.
2. **Honor and validate the configured CPU-to-PIT ratio** — Wire positive
   ratios through assembly/machine with remainder tests; preserve default goldens.
3. **Introduce rational scheduler time and conservative device deadlines** —
   Add checked phases, access timestamps, deterministic slots and default
   per-clock adapter behind an experimental mode; preserve legacy behavior.
4. **Schedule PIT transitions without losing counter semantics** — Add PIT
   deadlines and arithmetic catch-up, tested against all modes currently modeled.
5. **Schedule RTC and keyboard events; park timestamp-only devices** — Migrate
   devices separately with isolated and all-golden A/B gates; retain model limits.
6. **Re-pin the instruction-exact Real16 CPU scheduler seam** — After the CPU
   project lands the request below, update the exact git pin and run every golden.
7. **Deliver interrupts at completed instruction boundaries** — Use exact
   budgets/readiness, transaction completion, OUT-before-INTA, bounded HLT jumps,
   and quantum-independent caps/policies; compare both golden axes explicitly.
8. **Adopt the reviewed timing model and remove legacy production scheduling** —
   Accept individually explained golden differences, update decision/config docs,
   retain reference tests, then measure CPU entry/exit and device work again.

Later hardware fidelity work (RTC divider/periodic IRQs, speaker audio,
audio serialization, timed FDC/DMA) requires separate scope and oracles. It is
not a hidden prerequisite for dispatching the **current** device models correctly.

## Request to the CPU core project

This is a scoped request draft, not an issue filed or a message sent. Implement
and land it in x86jit first, then re-pin here. Scope: **Real16 interpreter**;
keep other modes' generated code unchanged and `CpuState` / `jit_abi`
append-only under that project's workflow.

1. **Instruction-exact execution.** Provide a new opt-in run API taking a
   maximum number `N` of **additional architecturally completed guest
   instructions**, not blocks, IR operations, decode attempts, REP iterations,
   or interrupt entries. On normal budget exhaustion retire exactly `N`,
   leave IP at the next instruction, and execute no effect of instruction
   `N+1`. For an earlier stop return the actual retired delta, `0 <= delta <= N`.
   A block containing 100 instructions with `N=7` must stop after the seventh,
   regardless of lifting/cache/block grouping. `N=0` executes nothing and has
   no guest side effects. Preserve the old block API for other users.
2. **Reliable retirement.** Count completed branches to their own IP, including
   `JMP $`, and HLT. Count each successfully completed instruction once, including
   a software INT; faulting instructions, hardware/exception delivery, MMIO
   retries, and already serviced transfer parts are not extra retirements.
   Preserve the counter across calls and expose it without lossy conversion.
   Exhaustion and a guest-visible stop at the same retirement must surface the
   guest-visible stop first, so the embedder completes it before any next run.
3. **Pending-instruction completion.** Report stop phase/ownership explicitly:
   already retired but awaiting port result/side effect, versus unretired
   MMIO/retry, with the completed prefix separated from the accessing instruction.
   Provide completion-only resumption that cannot execute the next guest
   instruction and reports final retirement (or fault). Multiple transfers and
   retries remain owned by one instruction. This allows a common start-time
   access convention and charges its elapsed time exactly once on retirement,
   including the zero-charge case of a post-access fault. It does not ask to
   add unsupported MMIO RMW operations.
4. **Immediate interrupt readiness and acceptance.** Expose the core's complete
   maskable-IRQ readiness at a boundary, including IF, STI/other modeled shadows,
   and all pending port/MMIO/retry state. Provide immediate vector acceptance
   with no guest instruction before IVT entry and no deferred queue after a
   successful readiness transaction. An equivalent callback invoked **only
   when accepting** may combine PIC acknowledgement with entry. Return delivery
   errors explicitly. Merely exposing IF or queue emptiness is insufficient.
5. **First eligible IRQ boundary.** With the embedder reporting INTR pending,
   the exact-run API must stop before the next guest instruction at the first
   boundary where acceptance becomes legal, even inside a long lifted block.
   Leave the PIC untouched until the embedder handles that stop. Readiness must
   account for POPF/IRET, `STI; NOP`, and `STI; HLT`; consume inhibition per
   completed instruction and retain it through faulting/deferred retries.
6. **Progress semantics.** Return a deterministic work-yield for zero-retirement
   exception/retry loops without pretending instructions retired. Specify REP
   explicitly: in this first instruction-time model a completed REP retires
   once, not once per element. Its internal iterations do not advance device
   time; no sub-REP IRQ latency guarantee is claimed. If hardware-correct
   between-iteration interrupt timing becomes required, that needs a separately
   accepted time/accounting model rather than covertly changing `retired` units.

CPU-project acceptance tests must cover cut points inside straight-line blocks,
batch partition equivalence, self-jumps, budget-ending OUT/IN/HLT, deferred and
multipart MMIO completion, fault/nonretirement, STI/IF/IRET acceptance windows,
and a halted wakeup before post-HLT code. A pending IRQ with IF clear must never
invoke the acknowledgement callback. Test several lifted block arrangements
with the same instruction stream and budget. Run that project's full gate and
the generated-code non-regression checks for the other modes before landing.

Until this seam exists, per-clock device optimization can be measured under
legacy boundaries, but it must not be advertised as instruction-exact IRQ timing.

## Proposed decision

### Context

Retired-instruction time and halted clock advancement provide reproducible
execution, but the configured CPU ratio is unused, device advancement sweeps
every chip on every PIT clock, and interrupt observation still depends on CPU
block budgets. Eager PIC acknowledgement can precede the stopping OUT or occur
while IF/shadows prohibit CPU acceptance. Measurements show correctness is the
main motivation; real DOS workloads are predominantly CPU-bound.

### Decision

Adopt checked 128-bit scheduler instants with `2^32` units per PIT period,
rational per-domain phases, and configurable rational retired instructions per
PIT period, defaulting to today's effective rate. Preserve retired instructions
plus carried halted instruction equivalents as the policy time measure.

Add conservative device deadlines with a default per-PIT-clock adapter and
explicit dormant state. Synchronize scheduled devices lazily on access, separate
trace timestamps from ticks, and observe every IRQ/output transition in stable
timestamp order. Keep concrete clock/signal wiring in assembly.

Require a Real16 instruction-exact CPU budget and first-eligible IRQ boundary
contract before enabling corrected scheduling. Complete stopping-instruction
bus transactions and immediate DMA/output work before PIC acknowledgement;
acknowledge only when immediate CPU acceptance is legal. HLT jumps to the
earliest event or bound without retiring instructions. Cooperation batch size
must not change guest behavior.

Extend decision-5's unit representation and decision-14's halted advancement
implementation; replace decision-4's block-quantized latency and eager queued
injection with the acceptance transaction above. Keep old and corrected
semantics explicitly distinguished until all-golden comparisons are reviewed.

### Alternatives

Retain per-clock ticking as a reference and compatibility fallback. Reject
running-CPU event skipping based on block budgets because it cannot enforce
instruction deadlines. Defer cycle/bus approximation pending calibrated oracles
and a separate decision; exclude wall-clock-driven execution.

### Consequences

Device periods and IRQ acceptance become independent of lifter block formation;
clock domains support future RTC/audio consumers without host-time coupling.
PIT pulses, I/O completion, and PIC acknowledgement gain explicit ordering.
Idle jumps and fewer device visits may reduce work, with no promised speedup.

The change requires a CPU-project seam, phase/transaction state, isolated
reference tests, and versioned timing identity. Compatibility optimizations must
preserve every golden; corrected IRQ/cap semantics may require individually
explained and accepted golden changes. Current device approximations and
atomic-REP timing remain visible fidelity limits.

### Trigger to revisit

Revisit when a demonstrated workload needs between-REP interrupt timing,
instruction-specific cycle costs, DMA arbitration, NMI/reset wakeup, calibrated
CPU frequency behavior, or finer-than-supported domain resolution. Revisit the
linear deadline scan or return strategy only when new phase measurements show
them material. Host pacing or audio playback alone is not a reason to weaken
deterministic virtual time.

## Review outcome (2026-11-09)

An adversarial review accepted the direction with changes; the accepted form is
**decision-15**, which wins where this proposal differs:

- **Blocker:** the CPU core does not model the one-instruction interrupt inhibit
  after an SS load (`mov ss` / `pop ss`). Instruction-exact delivery must not be
  enabled until it exists; it is part of the stage-1 request.
- Time stays in whole PIT clocks (`VClock(u64)`); other domains keep rational
  phases and round events up to the next PIT clock. The 128-bit, 2^32-per-clock
  instants above are dropped.
- PIT refresh/speaker channels are synchronised lazily, never
  scheduled.
- A REP string iteration is one step of CPU time, so device time advances inside
  long string operations and an interrupt can be taken between iterations.
- The stage-1 request to the CPU core is narrowed to: an instruction-exact
  budget in steps, counting `jmp $`, the SS-load inhibit, an interrupt-readiness
  query and an optional stop when an interrupt becomes acceptable. Completion
  resumption, an acceptance callback and a zero-progress yield stay in the
  emulator.
- Bead 2 (CPU ratio) folds into the time-model bead; the legacy outer loop stays
  untouched until adoption; a golden comparator normalises the documented
  systematic shifts before corrected timing is reviewed.
