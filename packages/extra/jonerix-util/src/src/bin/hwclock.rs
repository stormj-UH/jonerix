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

use jxutil::syscall::*;
use std::ffi::CString;
use std::mem::MaybeUninit;
use std::process::ExitCode;

// ── linux/rtc.h UAPI ─────────────────────────────────────────────

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

// _IOR/_IOW encoding: dir(2) | size(14) | type(8) | nr(8)
// For RTC type is 'p' = 0x70.
const RTC_RD_TIME:  i64 = 0x80247009; // _IOR('p', 9,  rtc_time)
const RTC_SET_TIME: i64 = 0x4024700a; // _IOW('p', 10, rtc_time)

// ── clock_gettime / settimeofday ────────────────────────────────
extern "C" {
    fn time(t: *mut i64) -> i64;
    fn gmtime_r(t: *const i64, tm: *mut RtcTime) -> *mut RtcTime;
    fn timegm(tm: *const RtcTime) -> i64;
    fn settimeofday(tv: *const Timeval, tz: *const core::ffi::c_void) -> i32;
}

#[repr(C)]
struct Timeval { tv_sec: i64, tv_usec: i64 }

fn rtc_read(fd: i32) -> std::io::Result<RtcTime> {
    let mut t = MaybeUninit::<RtcTime>::zeroed();
    let r = unsafe { ioctl(fd, RTC_RD_TIME as _, t.as_mut_ptr()) };
    if r < 0 { return Err(std::io::Error::last_os_error()); }
    Ok(unsafe { t.assume_init() })
}

fn rtc_write(fd: i32, t: &RtcTime) -> std::io::Result<()> {
    let r = unsafe { ioctl(fd, RTC_SET_TIME as _, t as *const _) };
    if r < 0 { return Err(std::io::Error::last_os_error()); }
    Ok(())
}

fn format_rtc(t: &RtcTime) -> String {
    let months = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
    let wdays  = ["Sun","Mon","Tue","Wed","Thu","Fri","Sat"];
    let m_idx  = t.tm_mon.clamp(0, 11) as usize;
    let w_idx  = t.tm_wday.clamp(0, 6)  as usize;
    format!("{} {} {:2} {:02}:{:02}:{:02} {}",
        wdays[w_idx], months[m_idx], t.tm_mday,
        t.tm_hour, t.tm_min, t.tm_sec, 1900 + t.tm_year)
}

fn parse_date(s: &str) -> Option<RtcTime> {
    // Accept "YYYY-MM-DD HH:MM:SS" or "YYYY-MM-DDTHH:MM:SS".
    let s = s.replace('T', " ");
    let mut it = s.split(|c: char| c == '-' || c == ' ' || c == ':');
    let y: i32 = it.next()?.parse().ok()?;
    let m: i32 = it.next()?.parse().ok()?;
    let d: i32 = it.next()?.parse().ok()?;
    let hh: i32 = it.next()?.parse().ok()?;
    let mm: i32 = it.next()?.parse().ok()?;
    let ss: i32 = it.next()?.parse().ok()?;
    Some(RtcTime { tm_sec: ss, tm_min: mm, tm_hour: hh, tm_mday: d,
                   tm_mon: m - 1, tm_year: y - 1900,
                   tm_wday: 0, tm_yday: 0, tm_isdst: 0 })
}

fn open_rtc(dev: &str) -> std::io::Result<i32> {
    let c = CString::new(dev).unwrap();
    // O_RDWR = 2
    let fd = unsafe { open(c.as_ptr(), 2, 0) };
    if fd < 0 { Err(std::io::Error::last_os_error()) } else { Ok(fd) }
}

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
            "--utc" => {},                 // informational; we always use UTC
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

    let fd = match open_rtc(&device) {
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
            unsafe { time(&mut now); }
            let mut t = RtcTime::default();
            unsafe { gmtime_r(&now, &mut t); }
            match rtc_write(fd, &t) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => { eprintln!("hwclock: RTC_SET_TIME: {}", e); ExitCode::FAILURE }
            }
        }
        "hctosys" => match rtc_read(fd) {
            Ok(t) => {
                let sec = unsafe { timegm(&t) };
                let tv = Timeval { tv_sec: sec, tv_usec: 0 };
                if unsafe { settimeofday(&tv, std::ptr::null()) } < 0 {
                    eprintln!("hwclock: settimeofday: {}", std::io::Error::last_os_error());
                    ExitCode::FAILURE
                } else { ExitCode::SUCCESS }
            }
            Err(e) => { eprintln!("hwclock: RTC_RD_TIME: {}", e); ExitCode::FAILURE }
        },
        "set" => {
            let Some(d) = date else {
                eprintln!("hwclock: --set requires --date=STR"); return ExitCode::FAILURE;
            };
            let Some(t) = parse_date(&d) else {
                eprintln!("hwclock: cannot parse date '{}'", d); return ExitCode::FAILURE;
            };
            match rtc_write(fd, &t) {
                Ok(())  => ExitCode::SUCCESS,
                Err(e)  => { eprintln!("hwclock: RTC_SET_TIME: {}", e); ExitCode::FAILURE }
            }
        }
        _ => ExitCode::FAILURE,
    };

    unsafe { close(fd); }
    rc
}
