//! hwclock — query and set the hardware RTC. Clean-room
//! implementation that drives /dev/rtc0 directly via the kernel
//! RTC ioctl ABI (linux/rtc.h UAPI). No util-linux source consulted.
//!
//! Reference:
//!   - linux/rtc.h (kernel UAPI header — BSD-style licence exception
//!     covers it as headers-only interface boundary)
//!   - Documentation/admin-guide/rtc.rst
//!   - rtc(4) and hwclock(8) man pages (behavioural spec only)
//!
//! Supported subcommands:
//!   --show / -r       (default) print current RTC time
//!   --systohc / -w    copy system clock → RTC
//!   --hctosys / -s    copy RTC → system clock
//!   --set --date=STR  set RTC to STR (parsed as "YYYY-MM-DD HH:MM:SS")
//!   --utc             treat RTC as UTC (default, documented)
//!   --localtime       treat RTC as local time (we don't convert; noted)

#![deny(unsafe_op_in_unsafe_fn)]

use jxutil::syscall::*;
use std::ffi::CString;
use std::mem::MaybeUninit;
use std::process::ExitCode;

// ── linux/rtc.h UAPI ─────────────────────────────────────────────────────

/// Mirrors `struct rtc_time` from linux/rtc.h (kernel UAPI).
/// Layout: 9 consecutive i32 fields = 36 bytes on all Linux arches.
#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
struct RtcTime {
    tm_sec:   i32,
    tm_min:   i32,
    tm_hour:  i32,
    tm_mday:  i32,
    tm_mon:   i32,   // 0-11
    tm_year:  i32,   // years since 1900
    tm_wday:  i32,
    tm_yday:  i32,
    tm_isdst: i32,
}

/// Full C `struct tm` as defined by musl/glibc on 64-bit Linux.
///
/// This is LARGER than RtcTime because libc adds `tm_gmtoff` (long) and
/// `tm_zone` (*const char) at the end. gmtime_r writes ALL fields — passing
/// a pointer to the smaller RtcTime would overwrite the stack (P0 bug B04).
#[repr(C)]
#[derive(Clone)]
struct CTm {
    tm_sec:    i32,
    tm_min:    i32,
    tm_hour:   i32,
    tm_mday:   i32,
    tm_mon:    i32,
    tm_year:   i32,
    tm_wday:   i32,
    tm_yday:   i32,
    tm_isdst:  i32,
    tm_gmtoff: i64,
    tm_zone:   *const u8,
}

impl Default for CTm {
    fn default() -> Self {
        CTm {
            tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 0,
            tm_mon: 0, tm_year: 0, tm_wday: 0, tm_yday: 0,
            tm_isdst: 0, tm_gmtoff: 0, tm_zone: std::ptr::null(),
        }
    }
}

impl From<&CTm> for RtcTime {
    fn from(c: &CTm) -> Self {
        RtcTime {
            tm_sec: c.tm_sec, tm_min: c.tm_min, tm_hour: c.tm_hour,
            tm_mday: c.tm_mday, tm_mon: c.tm_mon, tm_year: c.tm_year,
            tm_wday: c.tm_wday, tm_yday: c.tm_yday, tm_isdst: c.tm_isdst,
        }
    }
}

// ── ioctl request constants ──────────────────────────────────────────────
//
// _IOR/_IOW encoding (linux/ioctl.h), arch-generic (x86_64, aarch64, riscv):
//   bits [31:30] = direction: 0=none, 1=write(to kernel), 2=read(from kernel)
//   bits [29:16] = size of user-space struct
//   bits [15:8]  = type byte ('p' = 0x70 for RTC)
//   bits  [7:0]  = sequence number
//
// RTC_RD_TIME: _IOR('p', 9, struct rtc_time)  = 0x80247009
// RTC_SET_TIME: _IOW('p', 10, struct rtc_time) = 0x4024700a
//
// Note: MIPS/SPARC/PowerPC use a DIFFERENT ioctl encoding. This crate
// targets x86_64 and aarch64 only (the jonerix supported arches).
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "riscv64"))]
const RTC_RD_TIME: u64 = 0x80247009;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "riscv64"))]
const RTC_SET_TIME: u64 = 0x4024700a;

// ── C function bindings ──────────────────────────────────────────────────
extern "C" {
    fn time(t: *mut i64) -> i64;
    fn gmtime_r(t: *const i64, tm: *mut CTm) -> *mut CTm;
    fn timegm(tm: *mut CTm) -> i64;
    fn settimeofday(tv: *const Timeval, tz: *const core::ffi::c_void) -> i32;
}

#[repr(C)]
struct Timeval { tv_sec: i64, tv_usec: i64 }

// ── RTC read / write helpers ─────────────────────────────────────────────

fn rtc_read(fd: i32) -> std::io::Result<RtcTime> {
    let mut t = MaybeUninit::<RtcTime>::zeroed();
    // SAFETY:
    //   - `fd` is a valid, open file descriptor for /dev/rtcN.
    //   - RTC_RD_TIME is the correct ioctl for the Linux RTC driver.
    //   - `t.as_mut_ptr()` is non-null, aligned, writable for 36 bytes.
    //   - On success the kernel fills all 9 fields → assume_init is safe.
    let r = unsafe { ioctl(fd, RTC_RD_TIME as _, t.as_mut_ptr()) };
    if r < 0 { return Err(std::io::Error::last_os_error()); }
    // SAFETY: ioctl returned 0, so the kernel fully initialized the struct.
    Ok(unsafe { t.assume_init() })
}

fn rtc_write(fd: i32, t: &RtcTime) -> std::io::Result<()> {
    // SAFETY:
    //   - `fd` is valid and open for writing.
    //   - RTC_SET_TIME is the correct ioctl for the Linux RTC driver.
    //   - `t` is a valid, aligned, readable pointer to a complete RtcTime.
    let r = unsafe { ioctl(fd, RTC_SET_TIME as _, t as *const _) };
    if r < 0 { return Err(std::io::Error::last_os_error()); }
    Ok(())
}

// ── Formatting ───────────────────────────────────────────────────────────

fn format_rtc(t: &RtcTime) -> String {
    // ISO-8601-like format matching modern util-linux hwclock (2.36+).
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}+0000",
        1900 + t.tm_year, t.tm_mon + 1, t.tm_mday,
        t.tm_hour, t.tm_min, t.tm_sec)
}

fn parse_date(s: &str) -> Option<RtcTime> {
    // Accept "YYYY-MM-DD HH:MM:SS", "YYYY-MM-DDTHH:MM:SS", or bare "YYYY-MM-DD".
    let mut it = s.split(|c: char| c == '-' || c == ' ' || c == ':' || c == 'T');
    let y: i32 = it.next()?.parse().ok()?;
    let m: i32 = it.next()?.parse().ok()?;
    let d: i32 = it.next()?.parse().ok()?;
    // B07 fix: default time to 00:00:00 for bare date strings.
    let hh: i32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mm: i32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let ss: i32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    Some(RtcTime { tm_sec: ss, tm_min: mm, tm_hour: hh, tm_mday: d,
                   tm_mon: m - 1, tm_year: y - 1900,
                   tm_wday: 0, tm_yday: 0, tm_isdst: 0 })
}

// ── open_rtc ─────────────────────────────────────────────────────────────

fn open_rtc(dev: &str, write: bool) -> std::io::Result<i32> {
    let c = CString::new(dev)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in path"))?;
    // B06 fix: use O_RDONLY for --show, O_RDWR only when writing.
    let flags = if write { 2 | 0x80000 } else { 0 | 0x80000 }; // O_RDWR|O_CLOEXEC or O_RDONLY|O_CLOEXEC
    // SAFETY:
    //   - `c.as_ptr()` is non-null, NUL-terminated, valid until after open returns.
    //   - flags are valid open(2) constants.
    //   - mode=0 is ignored (not creating a file).
    let fd = unsafe { open(c.as_ptr(), flags, 0) };
    if fd < 0 { Err(std::io::Error::last_os_error()) } else { Ok(fd) }
}

// ── main ─────────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut op = "show";
    let mut date: Option<String> = None;
    let mut device = "/dev/rtc0".to_string();
    let mut iter = args.iter().peekable();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--show" | "-r" => op = "show",
            "--systohc" | "-w" => op = "systohc",
            "--hctosys" | "-s" => op = "hctosys",
            "--set" => op = "set",
            "--utc" => {},
            "--localtime" => {},
            "--date" => { date = iter.next().cloned(); }
            s if s.starts_with("--date=") => { date = Some(s[7..].to_string()); }
            "-f" | "--rtc" => { if let Some(v) = iter.next() { device = v.clone(); } }
            "-h" | "--help" => {
                println!("Usage: hwclock [OPTION]...");
                println!("  -r, --show       show the RTC time (default)");
                println!("  -w, --systohc    copy system → RTC");
                println!("  -s, --hctosys    copy RTC → system");
                println!("      --set --date STR   set RTC to STR");
                println!("  -f, --rtc DEV    RTC device (default /dev/rtc0)");
                return ExitCode::SUCCESS;
            }
            _ => { eprintln!("hwclock: unknown option '{}'", a); return ExitCode::FAILURE; }
        }
    }

    let needs_write = op == "systohc" || op == "set";
    let fd = match open_rtc(&device, needs_write) {
        Ok(fd) => fd,
        Err(e) => { eprintln!("hwclock: cannot open {}: {}", device, e); return ExitCode::FAILURE; }
    };

    let rc = match op {
        "show" => match rtc_read(fd) {
            Ok(t)  => { println!("{}", format_rtc(&t)); ExitCode::SUCCESS },
            Err(e) => { eprintln!("hwclock: RTC_RD_TIME: {}", e); ExitCode::FAILURE }
        },
        "systohc" => {
            let mut now: i64 = 0;
            // SAFETY: &mut now is a valid, aligned *mut i64.
            unsafe { time(&mut now); }
            let mut tm = CTm::default();
            // SAFETY:
            //   - &now is a valid *const i64.
            //   - &mut tm is a valid *mut CTm with correct size (52 bytes on 64-bit).
            //   - gmtime_r is thread-safe and fills ALL fields of CTm on success.
            unsafe { gmtime_r(&now, &mut tm); }
            let t = RtcTime::from(&tm);
            match rtc_write(fd, &t) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => { eprintln!("hwclock: RTC_SET_TIME: {}", e); ExitCode::FAILURE }
            }
        }
        "hctosys" => match rtc_read(fd) {
            Ok(rtc) => {
                let mut tm = CTm {
                    tm_sec: rtc.tm_sec, tm_min: rtc.tm_min, tm_hour: rtc.tm_hour,
                    tm_mday: rtc.tm_mday, tm_mon: rtc.tm_mon, tm_year: rtc.tm_year,
                    tm_wday: rtc.tm_wday, tm_yday: rtc.tm_yday, tm_isdst: 0,
                    tm_gmtoff: 0, tm_zone: std::ptr::null(),
                };
                // SAFETY: &mut tm is a valid, fully-sized *mut CTm.
                let sec = unsafe { timegm(&mut tm) };
                let tv = Timeval { tv_sec: sec, tv_usec: 0 };
                // SAFETY: &tv is a valid *const Timeval; null tz is correct.
                if unsafe { settimeofday(&tv, std::ptr::null()) } < 0 {
                    eprintln!("hwclock: settimeofday: {}", std::io::Error::last_os_error());
                    ExitCode::FAILURE
                } else { ExitCode::SUCCESS }
            }
            Err(e) => { eprintln!("hwclock: RTC_RD_TIME: {}", e); ExitCode::FAILURE }
        },
        "set" => {
            // B05 fix: close fd before early return to avoid fd leak.
            let Some(d) = date else {
                eprintln!("hwclock: --set requires --date=STR");
                unsafe { close(fd); }
                return ExitCode::FAILURE;
            };
            let Some(t) = parse_date(&d) else {
                eprintln!("hwclock: cannot parse date '{}' (expected YYYY-MM-DD [HH:MM:SS])", d);
                unsafe { close(fd); }
                return ExitCode::FAILURE;
            };
            match rtc_write(fd, &t) {
                Ok(())  => ExitCode::SUCCESS,
                Err(e)  => { eprintln!("hwclock: RTC_SET_TIME: {}", e); ExitCode::FAILURE }
            }
        }
        _ => ExitCode::FAILURE,
    };

    // SAFETY: fd >= 0 from open_rtc, not previously closed on this path.
    unsafe { close(fd); }
    rc
}
