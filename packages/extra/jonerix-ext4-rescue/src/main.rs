//! ext4-rescue — reset a corrupted ext4 inode's extent header in-place.
//!
//! When an ext4 inode's extent header gets garbage in it (wrong magic
//! number — we saw 0x204F where 0xF30A was expected), the kernel errors
//! on every access to that inode:
//!
//!     EXT4-fs error (device X): ext4_find_extent:944: inode #N:
//!     comm Y: pblk 0 bad header/extent: invalid magic - magic 204f, ...
//!
//! Even `rm` fails (err -117 EUCLEAN). Only repair tools can fix this.
//! jonerix can't ship e2fsprogs (GPL-2), and no permissive fsck.ext4
//! exists as of 2026-04-16 (see PR #107 discussion).
//!
//! This tool is a minimal, focused alternative: it locates the named
//! inode on disk and writes a fresh EMPTY extent header (magic=0xF30A,
//! entries=0, max_entries=4, depth=0) into inode.block[0..3], zeroing
//! the rest of the block[] array and i_size/i_blocks. After that, the
//! file exists but has no content — rm will succeed and free the
//! directory entry.
//!
//! Usage:
//!     ext4-rescue <device> <inode_num>
//!
//! The filesystem MUST be unmounted (or mounted read-only, root-only).
//! Writing to a mounted ext4 live is undefined behavior.
//!
//! Scope is deliberately narrow:
//!   - Only fixes bad extent headers on regular files (mode 0x8xxx).
//!   - Refuses to touch inodes < 11 (reserved: root=2, bad=1, journal=8).
//!   - Does NOT free data blocks (orphaned; next fsck or proper tool
//!     reclaims them). Better to leak space than to chain into more
//!     corruption while writing block bitmaps.
//!   - Does NOT remove directory entries.
//!
//! Hand-rolled to avoid crate deps (ext4_rs is tempting but its API is
//! 0.x and panics on errors; this tool is supposed to work when things
//! have already gone wrong).
//!
//! License: MIT. Copyright (c) 2026 jonerix contributors.

use std::env;
use std::fs::OpenOptions;
use std::os::unix::fs::FileExt;
use std::process::ExitCode;

// ---------------------------------------------------------------------------
// ext4 on-disk layout constants
// ---------------------------------------------------------------------------

const SUPERBLOCK_OFFSET: u64 = 1024;
const EXT4_SUPER_MAGIC: u16 = 0xEF53;
const EXT4_EXT_MAGIC: u16 = 0xF30A;

/// Flag in Ext4Inode.flags that says "extents tree, not block pointers".
/// When set, inode.block[0..15] holds an extent tree; otherwise it's the
/// legacy 12 direct + 3 indirect block-pointer layout.
const EXT4_EXTENTS_FL: u32 = 0x0008_0000;

// ---------------------------------------------------------------------------
// Tiny helpers for little-endian integer reads (ext4 is always LE on-disk)
// ---------------------------------------------------------------------------

fn le16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn put_le16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_le32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Superblock (fields we actually need)
// ---------------------------------------------------------------------------

struct Superblock {
    inodes_per_group: u32,
    block_size: u64,
    inode_size: u16,
    desc_size: u16,
    /// Has INCOMPAT_64BIT feature — group descriptor is 64 bytes, not 32.
    feature_64bit: bool,
}

impl Superblock {
    fn parse(buf: &[u8]) -> Result<Self, String> {
        if le16(buf, 0x38) != EXT4_SUPER_MAGIC {
            return Err(format!(
                "bad ext4 superblock magic: got 0x{:04x}, want 0x{:04x}",
                le16(buf, 0x38),
                EXT4_SUPER_MAGIC
            ));
        }
        let log_block_size = le32(buf, 0x18);
        let block_size = 1024u64 << log_block_size; // 1024, 2048, 4096, ...
        let inodes_per_group = le32(buf, 0x28);
        let inode_size = le16(buf, 0x58);
        let features_incompat = le32(buf, 0x60);
        let feature_64bit = features_incompat & 0x80 != 0;
        let desc_size = if feature_64bit {
            let d = le16(buf, 0xfe);
            if d == 0 {
                64
            } else {
                d
            }
        } else {
            32
        };
        Ok(Superblock {
            inodes_per_group,
            block_size,
            inode_size,
            desc_size,
            feature_64bit,
        })
    }
}

// ---------------------------------------------------------------------------
// Locate the inode table for a given block group
// ---------------------------------------------------------------------------

/// The Group Descriptor Table starts at block 1 (for 1K blocks) or block
/// just after the superblock padding. For 4K blocks, GDT starts at block
/// 1 (the block containing the superblock is block 0, and the SB lives
/// at offset 1024 inside it).
fn gdt_start_block(sb_block_size: u64) -> u64 {
    if sb_block_size == 1024 {
        2 // 0=boot, 1=superblock, 2=GDT
    } else {
        1 // 0=boot+superblock, 1=GDT (SB lives at 1024 into block 0)
    }
}

/// Read the inode table block number for a specific group.
/// Returns the disk block number where group `bgid`'s inode table begins.
fn read_inode_table_block(
    file: &std::fs::File,
    sb: &Superblock,
    bgid: u32,
) -> Result<u64, String> {
    let gdt_block = gdt_start_block(sb.block_size);
    let desc_offset =
        gdt_block * sb.block_size + (bgid as u64) * (sb.desc_size as u64);

    let mut desc = vec![0u8; sb.desc_size as usize];
    file.read_exact_at(&mut desc, desc_offset)
        .map_err(|e| format!("read group descriptor {}: {}", bgid, e))?;

    // bg_inode_table_lo at offset 0x08 (u32)
    let lo = le32(&desc, 0x08);
    // bg_inode_table_hi at offset 0x28 (u32) — only valid in 64bit mode
    let hi = if sb.feature_64bit && sb.desc_size >= 0x2c {
        le32(&desc, 0x28)
    } else {
        0
    };
    Ok(((hi as u64) << 32) | (lo as u64))
}

// ---------------------------------------------------------------------------
// The fix: reset the extent header in the inode
// ---------------------------------------------------------------------------

/// Offset of `block[]` array within the Ext4Inode struct.
/// Layout: mode(2) uid(2) size_lo(4) atime(4) ctime(4) mtime(4) dtime(4)
///         gid(2) links(2) blocks_lo(4) flags(4) osd1(4) = 40 = 0x28.
const BLOCK_OFFSET_IN_INODE: usize = 0x28;
const BLOCK_ARRAY_LEN: usize = 60; // 15 u32s
const I_SIZE_LO_OFFSET: usize = 0x04;
const I_SIZE_HI_OFFSET: usize = 0x6c;
const I_BLOCKS_LO_OFFSET: usize = 0x1c;
const I_FLAGS_OFFSET: usize = 0x20;
const I_MODE_OFFSET: usize = 0x00;

fn reset_inode_extent_header(
    inode_buf: &mut [u8],
    inode_num: u32,
    dry_run: bool,
) -> Result<(), String> {
    // Refuse to touch reserved inodes (1=badblocks, 2=root, 8=journal, ...)
    if inode_num < 11 {
        return Err(format!(
            "refusing to touch reserved inode {} (< 11)",
            inode_num
        ));
    }

    let mode = le16(inode_buf, I_MODE_OFFSET);
    let filetype = mode & 0xf000;
    let flags = le32(inode_buf, I_FLAGS_OFFSET);

    println!("  mode     = 0o{:06o} (type=0x{:04x})", mode, filetype);
    println!("  flags    = 0x{:08x}", flags);

    if filetype != 0x8000 {
        return Err(format!(
            "inode is not a regular file (type=0x{:04x}); refusing",
            filetype
        ));
    }
    if flags & EXT4_EXTENTS_FL == 0 {
        return Err(
            "inode does not use extents (EXT4_EXTENTS_FL clear); wrong tool".into(),
        );
    }

    // Peek at the current extent header.
    let eh_magic = le16(inode_buf, BLOCK_OFFSET_IN_INODE + 0);
    let eh_entries = le16(inode_buf, BLOCK_OFFSET_IN_INODE + 2);
    let eh_max = le16(inode_buf, BLOCK_OFFSET_IN_INODE + 4);
    let eh_depth = le16(inode_buf, BLOCK_OFFSET_IN_INODE + 6);
    println!(
        "  extent header: magic=0x{:04x} entries={} max={} depth={}",
        eh_magic, eh_entries, eh_max, eh_depth
    );

    if eh_magic == EXT4_EXT_MAGIC && eh_entries <= eh_max && eh_depth < 5 {
        return Err("extent header looks valid — nothing to fix".into());
    }

    if dry_run {
        println!("  [dry-run] would reset extent header to empty");
        return Ok(());
    }

    // Zero the entire block[] array (60 bytes)
    for i in 0..BLOCK_ARRAY_LEN {
        inode_buf[BLOCK_OFFSET_IN_INODE + i] = 0;
    }

    // Write an empty extent header:
    //   magic = 0xF30A
    //   entries_count = 0
    //   max_entries_count = 4  (inline fits 4 extents of 12 bytes after the 12-byte header)
    //   depth = 0
    //   generation = 0
    put_le16(inode_buf, BLOCK_OFFSET_IN_INODE + 0, EXT4_EXT_MAGIC);
    put_le16(inode_buf, BLOCK_OFFSET_IN_INODE + 2, 0);
    put_le16(inode_buf, BLOCK_OFFSET_IN_INODE + 4, 4);
    put_le16(inode_buf, BLOCK_OFFSET_IN_INODE + 6, 0);
    // generation at offset +8 (u32) already zeroed by the loop above

    // Zero file size (both halves) and block count.
    // The data blocks are now orphaned; next fsck or the tombstone
    // mechanism reclaims them. Better to leak than to scribble in
    // bitmaps from this rescue tool.
    put_le32(inode_buf, I_SIZE_LO_OFFSET, 0);
    put_le32(inode_buf, I_SIZE_HI_OFFSET, 0);
    put_le32(inode_buf, I_BLOCKS_LO_OFFSET, 0);

    // NB: we deliberately do NOT update the inode checksum. If metadata
    // checksums are enabled (feature_ro_compat_metadata_csum), the kernel
    // will log a warning on the next read but still accept the inode.
    // Writing a correct crc32c over inode+uuid+generation requires pulling
    // in crypto code; out of scope for this rescue tool.

    println!("  -> wrote empty extent header + zeroed size/blocks");
    Ok(())
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let (device_path, inode_num_arg, dry_run) = match args.len() {
        3 => (&args[1], &args[2], false),
        4 if args[1] == "-n" || args[1] == "--dry-run" => (&args[2], &args[3], true),
        _ => {
            eprintln!("usage: ext4-rescue [-n|--dry-run] <device> <inode_num>");
            eprintln!();
            eprintln!("Resets a corrupted ext4 inode's extent header to empty so the");
            eprintln!("file can be rm'd. The filesystem MUST be unmounted (or remounted");
            eprintln!("read-only). Only regular files. Refuses reserved inodes (<11).");
            return Err("usage".into());
        }
    };

    let inode_num: u32 = inode_num_arg
        .parse()
        .map_err(|e| format!("invalid inode number '{}': {}", inode_num_arg, e))?;

    let file = OpenOptions::new()
        .read(true)
        .write(!dry_run)
        .open(device_path)
        .map_err(|e| format!("open {}: {}", device_path, e))?;

    // --- 1. Read superblock ---
    let mut sb_buf = vec![0u8; 1024];
    file.read_exact_at(&mut sb_buf, SUPERBLOCK_OFFSET)
        .map_err(|e| format!("read superblock: {}", e))?;
    let sb = Superblock::parse(&sb_buf)?;
    println!("superblock:");
    println!("  block_size       = {}", sb.block_size);
    println!("  inodes_per_group = {}", sb.inodes_per_group);
    println!("  inode_size       = {}", sb.inode_size);
    println!("  desc_size        = {}", sb.desc_size);
    println!("  64-bit feature   = {}", sb.feature_64bit);

    // --- 2. Locate the target inode ---
    let bgid = (inode_num - 1) / sb.inodes_per_group;
    let index_in_bg = (inode_num - 1) % sb.inodes_per_group;
    println!(
        "inode #{}: group={}, index_in_group={}",
        inode_num, bgid, index_in_bg
    );

    let inode_table_block = read_inode_table_block(&file, &sb, bgid)?;
    let inode_byte_offset = inode_table_block * sb.block_size
        + (index_in_bg as u64) * (sb.inode_size as u64);
    println!(
        "  inode_table_block = {} (0x{:x})",
        inode_table_block, inode_table_block
    );
    println!(
        "  inode_disk_offset = {} (0x{:x})",
        inode_byte_offset, inode_byte_offset
    );

    // --- 3. Read the inode ---
    let mut inode_buf = vec![0u8; sb.inode_size as usize];
    file.read_exact_at(&mut inode_buf, inode_byte_offset)
        .map_err(|e| format!("read inode: {}", e))?;

    // --- 4. Fix the extent header ---
    reset_inode_extent_header(&mut inode_buf, inode_num, dry_run)?;

    // --- 5. Write back (unless dry-run) ---
    if !dry_run {
        file.write_all_at(&inode_buf, inode_byte_offset)
            .map_err(|e| format!("write inode: {}", e))?;
        file.sync_all()
            .map_err(|e| format!("sync: {}", e))?;
        println!("done. you can now `rm` the file; fsck the fs at next boot.");
    } else {
        println!("done (dry-run — no writes).");
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ext4-rescue: {}", e);
            ExitCode::FAILURE
        }
    }
}
