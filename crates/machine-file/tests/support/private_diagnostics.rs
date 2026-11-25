// SPDX-License-Identifier: AGPL-3.0-or-later
//! Payload-free local-media error reporting; see user-media.md §Windows 95.
//! This host observer models no additional hardware.

pub fn error_summary(error: &machine::MachineError) -> String {
    use cpu::CpuError;
    match error {
        machine::MachineError::Cpu(error) => match error {
            CpuError::UnsupportedInstruction { cs, addr, .. } => {
                format!("unsupported instruction at {cs:04x}:{addr:08x}")
            }
            CpuError::Unmapped { addr } => format!("unmapped access at {addr:08x}"),
            CpuError::Fault { vector, cs, ip, .. } => {
                format!("CPU fault vector {vector} at {cs:04x}:{ip:08x}")
            }
            CpuError::UnsupportedSystem { cs, ip, .. } => {
                format!("unsupported system operation at {cs:04x}:{ip:08x}")
            }
            CpuError::DeliveryStalled { deliveries, cs, ip } => {
                format!("delivery stalled after {deliveries} at {cs:04x}:{ip:08x}")
            }
            CpuError::Memory(_) => "CPU memory error".into(),
            CpuError::UnexpectedExit { .. } => "unexpected CPU exit".into(),
            _ => "CPU error".into(),
        },
        machine::MachineError::Bus(_) => "bus error".into(),
        machine::MachineError::Exception { vector } => format!("guest exception vector {vector}"),
    }
}

#[test]
fn private_errors_omit_instruction_bytes_and_untrusted_strings() {
    let error = |bytes| {
        machine::MachineError::Cpu(cpu::CpuError::UnsupportedInstruction {
            cs: 0x1234,
            addr: 0x5678,
            bytes,
        })
    };
    let summary = error_summary(&error(b"SYNTHETIC-SECRET".to_vec()));
    assert_eq!(summary, "unsupported instruction at 1234:00005678");
    assert_eq!(summary, error_summary(&error(vec![0xde, 0xad, 0xbe, 0xef])));
    assert_eq!(
        error_summary(&cpu::CpuError::Memory("SYNTHETIC-SECRET".into()).into()),
        "CPU memory error"
    );
    assert_eq!(
        error_summary(
            &cpu::CpuError::UnexpectedExit {
                what: "SYNTHETIC-SECRET".into()
            }
            .into()
        ),
        "unexpected CPU exit"
    );
}
