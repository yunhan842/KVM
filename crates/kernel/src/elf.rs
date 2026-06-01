//! Minimal ELF64 program loader.
//!
//! Walks PT_LOAD segments and copies them into place. Single architecture
//! (x86-64), single endianness (little-endian), single class (ELF64). Not a
//! security parser — the input is the kernel-embedded user binary the build
//! pipeline produced. We bounds-check the INPUT side (the embedded slice) to
//! avoid UB reads off the end of `bytes`. We do NOT bounds-check the OUTPUT
//! side (`p_vaddr`); the caller (`user::setup`) ensures the user region is
//! mapped before calling.
//!
//! Design: docs/superpowers/specs/2026-05-31-minikvm-month3-c-userspace-design.md

// Items unused until Task 3 calls `elf::load` from `user::setup`.
#![allow(dead_code)]

const PT_LOAD: u32 = 1;

#[derive(Debug)]
pub enum ElfError {
    BadMagic,
    Not64Bit,
    NotLE,
    NotExec,
    WrongMachine,
    BadPhEntsize,
    /// The embedded slice is too short for what the headers reference (Ehdr,
    /// Phdr table, or a PT_LOAD segment's source range). Treated as a build-
    /// pipeline bug; the kernel-embedded binary should never trip this.
    Truncated,
}

/// Parse the ELF, copy each PT_LOAD to its `p_vaddr`, zero the BSS tail.
/// Returns the program entry point on success.
///
/// # Safety
/// Caller must ensure every PT_LOAD's `p_vaddr..p_vaddr + p_memsz` is mapped
/// writable within the intended user region. We don't bounds-check the
/// OUTPUT side.
pub unsafe fn load(bytes: &[u8]) -> Result<u64, ElfError> {
    // The full Ehdr is 64 bytes; bounds-check once so subsequent
    // rd16/rd32/rd64 reads through e_phnum can't panic on truncated input.
    if bytes.len() < 64 {
        return Err(ElfError::Truncated);
    }

    // 0x7F 'E' 'L' 'F' + EI_CLASS=2 (ELFCLASS64) + EI_DATA=1 (ELFDATA2LSB).
    if &bytes[0..4] != b"\x7fELF" {
        return Err(ElfError::BadMagic);
    }
    if bytes[4] != 2 {
        return Err(ElfError::Not64Bit);
    }
    if bytes[5] != 1 {
        return Err(ElfError::NotLE);
    }

    let e_type = rd16(bytes, 0x10);
    let e_machine = rd16(bytes, 0x12);
    if e_type != 2 {
        return Err(ElfError::NotExec);
    } // ET_EXEC
    if e_machine != 62 {
        return Err(ElfError::WrongMachine);
    } // EM_X86_64

    let e_entry = rd64(bytes, 0x18);
    let e_phoff = rd64(bytes, 0x20);
    let e_phentsize = rd16(bytes, 0x36);
    let e_phnum = rd16(bytes, 0x38);
    if e_phentsize != 56 {
        return Err(ElfError::BadPhEntsize);
    }
    // sizeof(Elf64_Phdr) = 4(p_type) + 4(p_flags) + 8(p_offset) + 8(p_vaddr)
    //                   + 8(p_paddr) + 8(p_filesz) + 8(p_memsz) + 8(p_align) = 56

    // Bounds-check the Phdr table itself.
    let phtab_end = (e_phoff as usize)
        .checked_add(
            (e_phnum as usize)
                .checked_mul(56)
                .ok_or(ElfError::Truncated)?,
        )
        .ok_or(ElfError::Truncated)?;
    if phtab_end > bytes.len() {
        return Err(ElfError::Truncated);
    }

    for i in 0..(e_phnum as usize) {
        let off = e_phoff as usize + i * 56;
        let p_type = rd32(bytes, off);
        if p_type != PT_LOAD {
            continue;
        }

        let p_offset = rd64(bytes, off + 8);
        let p_vaddr = rd64(bytes, off + 16);
        let p_filesz = rd64(bytes, off + 32);
        let p_memsz = rd64(bytes, off + 40);

        // Bounds-check the source slice for this segment. Without this,
        // a truncated ELF with p_offset + p_filesz > bytes.len() would make
        // `copy_nonoverlapping` read past the embedded slice — undefined
        // behavior. Trusted toolchain shouldn't emit this; cheap UB guard.
        let src_end = (p_offset as usize)
            .checked_add(p_filesz as usize)
            .ok_or(ElfError::Truncated)?;
        if src_end > bytes.len() {
            return Err(ElfError::Truncated);
        }

        // Malformed p_memsz < p_filesz means the copy below would write
        // past p_vaddr+p_memsz. Reject explicitly.
        if p_memsz < p_filesz {
            return Err(ElfError::Truncated);
        }

        // Copy p_filesz bytes from the embedded ELF to p_vaddr. Note:
        // copy_nonoverlapping with count=0 is a documented no-op, so the
        // degenerate-empty-PT_LOAD case (vaddr=0, filesz=0) common for
        // tiny programs with no live .data is safe.
        core::ptr::copy_nonoverlapping(
            bytes.as_ptr().add(p_offset as usize),
            p_vaddr as *mut u8,
            p_filesz as usize,
        );

        // Zero the BSS tail: addresses (p_vaddr + p_filesz)..(p_vaddr + p_memsz).
        if p_memsz > p_filesz {
            core::ptr::write_bytes(
                (p_vaddr + p_filesz) as *mut u8,
                0,
                (p_memsz - p_filesz) as usize,
            );
        }
    }

    // If e_phnum == 0 or no PT_LOAD was found, we still return Ok(e_entry):
    // the caller relies on the toolchain having produced at least one
    // segment. A misconfigured build that emits zero PT_LOADs would lead to
    // a #PF at iretq's dispatch — caught by slice 2's IDT.
    Ok(e_entry)
}

// --- little-endian field readers (no panic guard; preceded by length checks) ---

fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn rd64(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}
