// SPDX-License-Identifier: AGPL-3.0-or-later
//! Immutable sector sources and a sparse writable top; persistent-media.md
//! § Memory and hashing. This disk-format seam can also back hash-linked layers.

use crate::{Header, SECTOR_LEN};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io;
use std::rc::Rc;

/// One immutable parent in a sector-overlay chain. Host errors remain outside
/// controller state; the assembly adapter latches them at its host boundary.
/// See persistent-media.md § Memory and hashing.
pub trait SectorSource {
    fn byte_len(&self) -> u64;
    fn read_sector(&self, lba: u64) -> io::Result<[u8; SECTOR_LEN]>;
    fn check_unchanged(&self) -> io::Result<()>;
    /// Read an aligned range for bounded streaming exports. The final short CMOS
    /// block is permitted; layer implementations need only supply sector reads.
    fn read_range(&self, first_lba: u64, bytes: &mut [u8]) -> io::Result<()> {
        for (index, chunk) in bytes.chunks_mut(SECTOR_LEN).enumerate() {
            chunk.copy_from_slice(&self.read_sector(first_lba + index as u64)?[..chunk.len()]);
        }
        self.check_unchanged()
    }
}

/// A verified immutable sector source; persistent-media.md § Memory and hashing.
#[derive(Clone, Debug)]
pub struct VerifiedBase {
    source: Rc<dyn SectorSource>,
    identity: Header,
}

impl std::fmt::Debug for dyn SectorSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SectorSource")
            .field("byte_len", &self.byte_len())
            .finish()
    }
}

impl VerifiedBase {
    /// Wrap an immutable source whose complete identity the caller has already
    /// established, such as an all-zero blank disk or a verified layer chain.
    /// Reads go straight to the source; nothing is copied, hashed or materialized.
    pub fn from_verified_source(
        source: Rc<dyn SectorSource>,
        identity: Header,
    ) -> io::Result<Self> {
        if identity.base_length != source.byte_len() {
            return Err(io::Error::other("verified base length mismatch"));
        }
        identity.encode().map_err(io::Error::other)?;
        source.check_unchanged()?;
        Ok(Self { source, identity })
    }

    /// Build a generated zero base without allocating its capacity. The full
    /// hash is required because journals bind bytes, not source descriptors.
    pub fn zero(size: u64) -> Result<Self, crate::Error> {
        let mut identity = Header {
            base_length: size,
            base_digest: [0; 32],
            anchor_generation: 0,
        };
        identity.encode()?;
        let buffer = [0; 65536];
        let mut hash = Sha256::new();
        let mut remaining = size;
        while remaining != 0 {
            let count = remaining.min(buffer.len() as u64) as usize;
            hash.update(&buffer[..count]);
            remaining -= count as u64;
        }
        identity.base_digest = hash.finalize().into();
        Ok(Self {
            source: Rc::new(ZeroSource(size)),
            identity,
        })
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::Error> {
        let base = Self::memory(bytes);
        base.identity.encode()?;
        Ok(base)
    }

    pub(crate) fn memory(bytes: &[u8]) -> Self {
        Self {
            identity: Header {
                base_length: bytes.len() as u64,
                base_digest: crate::codec::digest(bytes),
                anchor_generation: 0,
            },
            source: Rc::new(MemorySource(bytes.to_vec())),
        }
    }

    pub fn identity(&self) -> &Header {
        &self.identity
    }
}

impl SectorSource for VerifiedBase {
    fn byte_len(&self) -> u64 {
        self.identity.base_length
    }
    fn read_sector(&self, lba: u64) -> io::Result<[u8; SECTOR_LEN]> {
        self.source.read_sector(lba)
    }
    fn check_unchanged(&self) -> io::Result<()> {
        self.source.check_unchanged()
    }
    fn read_range(&self, first_lba: u64, bytes: &mut [u8]) -> io::Result<()> {
        self.source.read_range(first_lba, bytes)
    }
}

struct ZeroSource(u64);
impl SectorSource for ZeroSource {
    fn byte_len(&self) -> u64 {
        self.0
    }
    fn read_sector(&self, _: u64) -> io::Result<[u8; SECTOR_LEN]> {
        Ok([0; SECTOR_LEN])
    }
    fn check_unchanged(&self) -> io::Result<()> {
        Ok(())
    }
}

struct MemorySource(Vec<u8>);
impl SectorSource for MemorySource {
    fn byte_len(&self) -> u64 {
        self.0.len() as u64
    }
    fn check_unchanged(&self) -> io::Result<()> {
        Ok(())
    }
    fn read_sector(&self, lba: u64) -> io::Result<[u8; SECTOR_LEN]> {
        let mut bytes = [0; SECTOR_LEN];
        if let Some(start) = usize::try_from(lba)
            .ok()
            .and_then(|n| n.checked_mul(SECTOR_LEN))
            .filter(|&n| n < self.0.len())
        {
            let end = (start + SECTOR_LEN).min(self.0.len());
            bytes[..end - start].copy_from_slice(&self.0[start..end]);
        }
        Ok(bytes)
    }
}

/// Deterministically ordered replacements over an immutable parent. A future
/// layer can implement `SectorSource` and become this top's parent unchanged.
/// See persistent-media.md § Memory and hashing.
pub struct SparseOverlay {
    parent: Rc<dyn SectorSource>,
    sectors: BTreeMap<u64, Box<[u8; SECTOR_LEN]>>,
}
impl std::fmt::Debug for SparseOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SparseOverlay")
            .field("byte_len", &self.byte_len())
            .field("sectors", &self.sectors)
            .finish()
    }
}
impl SparseOverlay {
    pub fn new(parent: Rc<dyn SectorSource>) -> Self {
        Self {
            parent,
            sectors: BTreeMap::new(),
        }
    }
    /// Test only written sectors; an unchanged disk needs no full comparison pass.
    pub fn has_changes(&self) -> io::Result<bool> {
        for (&lba, bytes) in &self.sectors {
            if self.parent.read_sector(lba)? != **bytes {
                return Ok(true);
            }
        }
        self.check_unchanged()?;
        Ok(false)
    }
    pub fn written_sectors(&self) -> usize {
        self.sectors.len()
    }
    pub fn replacements(&self) -> impl Iterator<Item = (&u64, &Box<[u8; SECTOR_LEN]>)> {
        self.sectors.iter()
    }
    pub fn replace(&mut self, lba: u64, bytes: Box<[u8; SECTOR_LEN]>) {
        self.sectors.insert(lba, bytes);
    }
    pub fn same_replacements(&self, other: &Self) -> bool {
        self.sectors == other.sectors
    }
    /// Explicit capture helper for small callers; mount and maintenance never
    /// materialize an image. Export streams sectors instead.
    pub fn materialize(&self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        for lba in 0..self.byte_len().div_ceil(SECTOR_LEN as u64) {
            bytes.extend_from_slice(&self.read_sector(lba)?);
        }
        bytes.truncate(self.byte_len() as usize);
        Ok(bytes)
    }
}
impl SectorSource for SparseOverlay {
    fn byte_len(&self) -> u64 {
        self.parent.byte_len()
    }
    fn check_unchanged(&self) -> io::Result<()> {
        self.parent.check_unchanged()
    }
    fn read_range(&self, first_lba: u64, bytes: &mut [u8]) -> io::Result<()> {
        self.parent.read_range(first_lba, bytes)?;
        let end = first_lba + bytes.len().div_ceil(SECTOR_LEN) as u64;
        for (&lba, sector) in self.sectors.range(first_lba..end) {
            let start = (lba - first_lba) as usize * SECTOR_LEN;
            let count = (bytes.len() - start).min(SECTOR_LEN);
            bytes[start..start + count].copy_from_slice(&sector[..count]);
        }
        self.check_unchanged()
    }
    fn read_sector(&self, lba: u64) -> io::Result<[u8; SECTOR_LEN]> {
        self.check_unchanged()?;
        self.sectors
            .get(&lba)
            .map_or_else(|| self.parent.read_sector(lba), |bytes| Ok(**bytes))
    }
}

#[cfg(test)]
mod zero_tests {
    use super::*;
    use unemu_core_api::{test_support::MemoryStorage, Storage};

    #[test]
    fn zero_identity_is_the_full_byte_hash_and_replacements_stay_sparse() {
        let size = 3 * SECTOR_LEN;
        let base = VerifiedBase::zero(size as u64).unwrap();
        assert_eq!(
            base.identity().base_digest,
            <[u8; 32]>::from(Sha256::digest(vec![0; size]))
        );
        assert_eq!(base.read_sector(0).unwrap(), [0; SECTOR_LEN]);
        assert_eq!(base.read_sector(2).unwrap(), [0; SECTOR_LEN]);
        let mut overlay = SparseOverlay::new(Rc::new(base.clone()));
        overlay.replace(2, Box::new([0xa5; SECTOR_LEN]));
        assert_eq!(overlay.written_sectors(), 1);
        assert_eq!(overlay.read_sector(2).unwrap(), [0xa5; SECTOR_LEN]);
        assert_eq!(base.read_sector(2).unwrap(), [0; SECTOR_LEN]);
        assert_eq!(
            SparseOverlay::new(Rc::new(base)).read_sector(2).unwrap(),
            [0; SECTOR_LEN]
        );
    }

    #[test]
    fn large_zero_base_has_no_capacity_sized_storage() {
        let base = VerifiedBase::zero(504 * 1024 * 1024).unwrap();
        assert_eq!(base.byte_len(), 528482304);
        assert_eq!(
            base.read_sector(base.byte_len() / 512 - 1).unwrap(),
            [0; SECTOR_LEN]
        );
        let mut overlay = SparseOverlay::new(Rc::new(base));
        assert_eq!(overlay.written_sectors(), 0);
        overlay.replace(1032191, Box::new([7; SECTOR_LEN]));
        assert_eq!(overlay.written_sectors(), 1);
        assert_eq!(
            std::mem::size_of::<ZeroSource>(),
            std::mem::size_of::<u64>()
        );
    }

    #[test]
    fn zero_journal_reopens_replacements_against_the_generated_identity() {
        let base = VerifiedBase::zero(2048).unwrap();
        let storage = MemoryStorage::new();
        let mut media = crate::JournalMedia::open_record(
            storage.open("zero.upfj").unwrap(),
            &base,
            2048,
            crate::SinkStatus::default(),
        )
        .unwrap();
        media.write_batch(&[(3, [0x5a; 512])]);
        media.finish().unwrap();
        let reopened = crate::JournalMedia::open_record(
            storage.open("zero.upfj").unwrap(),
            &base,
            2048,
            crate::SinkStatus::default(),
        )
        .unwrap();
        assert_eq!(reopened.read_sector(0), [0; 512]);
        assert_eq!(reopened.read_sector(3), [0x5a; 512]);
        assert_eq!(reopened.written_sectors(), 1);
        assert_eq!(
            crate::Header::decode(
                &storage.contents("zero.upfj").unwrap()[..crate::HEADER_LEN],
                &[0; 2048],
                2048
            )
            .unwrap()
            .base_digest,
            base.identity().base_digest
        );
    }
}
