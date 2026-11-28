// SPDX-License-Identifier: AGPL-3.0-or-later
//! File-backed PC assembly; core-contract.md §12.5.
use crate::FileMount;
use assembly::{
    AssembledMachine, AssemblyError, AssemblyOptions, MachineConfig, ObservationPolicy,
};
use std::{collections::BTreeMap, path::PathBuf, rc::Rc};
/// Assemble a config over host files.
pub fn assemble(config: &MachineConfig) -> Result<AssembledMachine, AssemblyError> {
    assemble_with_options(config, AssemblyOptions::default())
}
/// Assemble host files with run-only firmware permissions.
pub fn assemble_with_options(
    config: &MachineConfig,
    options: AssemblyOptions,
) -> Result<AssembledMachine, AssemblyError> {
    assemble_with_observation_and_options(config, &ObservationPolicy::default(), options)
}
/// Assemble host files with explicit capture routes.
pub fn assemble_with_observation(
    config: &MachineConfig,
    policy: &ObservationPolicy,
) -> Result<AssembledMachine, AssemblyError> {
    assemble_with_observation_and_options(config, policy, AssemblyOptions::default())
}
/// Assemble host files with capture routes and firmware permissions.
pub fn assemble_with_observation_and_options(
    config: &MachineConfig,
    policy: &ObservationPolicy,
    options: AssemblyOptions,
) -> Result<AssembledMachine, AssemblyError> {
    assembly::assemble_mounted(config, policy, options, Rc::new(FileMount::new()))
}
/// Reuse bases already verified in full by the host loader.
pub fn assemble_with_verified_bases(
    config: &MachineConfig,
    policy: &ObservationPolicy,
    options: AssemblyOptions,
    bases: &BTreeMap<PathBuf, journal_files::VerifiedBase>,
) -> Result<AssembledMachine, AssemblyError> {
    assemble_with_verified_bases_and_jit(config, policy, options, bases, None)
}
/// [`assemble_with_verified_bases`] with the embedder's JIT backend, which
/// `cpu.engine = "jit"` requires; see `assembly::assemble_mounted_with_jit`.
pub fn assemble_with_verified_bases_and_jit(
    config: &MachineConfig,
    policy: &ObservationPolicy,
    options: AssemblyOptions,
    bases: &BTreeMap<PathBuf, journal_files::VerifiedBase>,
    jit: Option<assembly::JitBackendFactory>,
) -> Result<AssembledMachine, AssemblyError> {
    assembly::assemble_mounted_with_jit(
        config,
        policy,
        options,
        Rc::new(FileMount::with_verified_bases(bases.clone())),
        jit,
    )
}
/// Validate hardware and every host input/output alias before opening a journal.
pub fn validate(config: &MachineConfig) -> Result<(), assembly::ConfigError> {
    config.validate_mounted(crate::mount_paths::validate_paths)
}
