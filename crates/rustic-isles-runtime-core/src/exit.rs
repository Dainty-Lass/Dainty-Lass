//! Process exit and crash handling (spec-runtime-r3): the export table, the exception filter and budget, the line
//! texts, the stack trim, the engine-error sanitiser, the FName recovery decision and the crash-wait outcome. The DLL's
//! `ffi::exit` feeds these from the detours and the vectored handler.

use crate::patch::PatchResult;
use std::fmt::{self, Display, Write};
use std::ops::Range;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// The executable's file name, for addresses in its range that no module lookup answered.
pub const EXE_NAME: &str = "Dauntless-Win64-Shipping.exe";
/// The span after the image base counted as the executable (R3 3.2).
pub const IMAGE_SPAN: usize = 0x0800_0000;
/// Frames `RtlCaptureStackBackTrace` is asked for.
pub const CAPTURE_FRAMES: usize = 48;
/// Frames kept after the trim.
pub const MAX_FRAMES: usize = 32;
/// Traced exceptions logged per process, EXIT-5 and EXIT-12 together (R3 5.3).
pub const BUDGET: u32 = 96;
/// `EXCEPTION_MAXIMUM_PARAMETERS`.
pub const MAX_PARAMS: usize = 15;
/// The code both `abort`s end the process with.
pub const ABORT_CODE: u32 = 3;
/// `STATUS_ACCESS_VIOLATION`.
pub const ACCESS_VIOLATION: u32 = 0xC000_0005;
/// `STATUS_STACK_OVERFLOW`.
pub const STACK_OVERFLOW: u32 = 0xC000_00FD;
/// The flag that keeps the native 60 s crash wait (RT-14 for R3).
pub const DISABLE_CRASH_WAIT_SHORTEN: &str = "DISABLE_CRASH_WAIT_SHORTEN";

/// Codes never logged: debugger prints and control events, the MSVC thread name and C++ exceptions (Rust panics on
/// this target raise the latter too).
pub const FILTERED: [u32; 7] = [
    0x4001_0005,
    0x4001_0006,
    0x4001_0007,
    0x4001_0008,
    0x4001_000A,
    0x406D_1388,
    0xE06D_7363,
];

/// Whether `code` is never logged.
pub fn filtered(code: u32) -> bool {
    FILTERED.contains(&code)
}

/// The process-wide exception budget shared by EXIT-5 and EXIT-12. It never resets.
pub struct Budget(AtomicU32);

impl Budget {
    /// Nothing spent.
    pub const fn new() -> Self {
        Self(AtomicU32::new(0))
    }

    /// Spends one unit for a traced exception: true while the count is at most [`BUDGET`].
    pub fn take(&self) -> bool {
        // The load keeps a spent counter from growing (and ever wrapping)
        self.0.load(Ordering::Relaxed) < BUDGET && self.0.fetch_add(1, Ordering::SeqCst) < BUDGET
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new()
    }
}

/// The argument shape of an exit export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `(u32 code)`, no return.
    Code,
    /// `(i32 code)`, no return (the CRT's `exit` and `_exit`).
    CodeI32,
    /// `NtTerminateProcess(HANDLE, i32) -> i32`.
    NtTerminate,
    /// `TerminateProcess(HANDLE, u32) -> i32`.
    Terminate,
    /// `RaiseException(u32, u32, u32, *const usize)`.
    Raise,
    /// `abort()`.
    Abort,
}

/// One exit export (R3 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitExport {
    /// `EXIT-1` to `EXIT-11`.
    pub id: &'static str,
    /// The module `GetModuleHandleW` is asked for.
    pub module: &'static str,
    /// The export.
    pub export: &'static str,
    /// The tag in the lines.
    pub tag: &'static str,
    /// Its arguments.
    pub shape: Shape,
}

const fn x(
    id: &'static str,
    module: &'static str,
    export: &'static str,
    tag: &'static str,
    shape: Shape,
) -> ExitExport {
    ExitExport {
        id,
        module,
        export,
        tag,
        shape,
    }
}

/// EXIT-1 to EXIT-11, in install order.
pub const EXPORTS: [ExitExport; 11] = [
    x(
        "EXIT-1",
        "kernel32.dll",
        "ExitProcess",
        "kernel32!ExitProcess",
        Shape::Code,
    ),
    x(
        "EXIT-2",
        "ntdll.dll",
        "RtlExitUserProcess",
        "ntdll!RtlExitUserProcess",
        Shape::Code,
    ),
    x(
        "EXIT-3",
        "ntdll.dll",
        "NtTerminateProcess",
        "ntdll!NtTerminateProcess",
        Shape::NtTerminate,
    ),
    x(
        "EXIT-4",
        "kernelbase.dll",
        "TerminateProcess",
        "kernelbase!TerminateProcess",
        Shape::Terminate,
    ),
    x(
        "EXIT-5",
        "kernelbase.dll",
        "RaiseException",
        "kernelbase!RaiseException",
        Shape::Raise,
    ),
    x("EXIT-6", "ucrtbase.dll", "exit", "ucrtbase!exit", Shape::CodeI32),
    x("EXIT-7", "ucrtbase.dll", "_exit", "ucrtbase!_exit", Shape::CodeI32),
    x("EXIT-8", "ucrtbase.dll", "abort", "ucrtbase!abort", Shape::Abort),
    x("EXIT-9", "msvcrt.dll", "exit", "msvcrt!exit", Shape::CodeI32),
    x("EXIT-10", "msvcrt.dll", "_exit", "msvcrt!_exit", Shape::CodeI32),
    x("EXIT-11", "msvcrt.dll", "abort", "msvcrt!abort", Shape::Abort),
];

/// The table line for `id`.
pub fn export(id: &str) -> Option<&'static ExitExport> {
    EXPORTS.iter().find(|export| export.id == id)
}

/// The text after the last `\` or `/` of a module path.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// `addr(p)` (R3 3.2): `null`, `<module>+0x<HEX>`, the executable fallback, or raw hex. `module` is the holding
/// module's file name and the offset into it.
#[derive(Debug, Clone, Copy)]
pub struct Addr<'a> {
    /// The address.
    pub raw: usize,
    /// The module holding it, as `GetModuleHandleExW` found it.
    pub module: Option<(&'a str, usize)>,
    /// The executable's image base.
    pub image_base: usize,
}

/// The offset of `raw` into the executable's range, if it lies there.
pub fn exe_offset(raw: usize, image_base: usize) -> Option<usize> {
    let offset = raw.checked_sub(image_base)?;
    (image_base != 0 && offset < IMAGE_SPAN).then_some(offset)
}

impl Display for Addr<'_> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.raw == 0 {
            return out.write_str("null");
        }
        if let Some((name, offset)) = self.module {
            return write!(out, "{name}+0x{offset:X}");
        }
        match exe_offset(self.raw, self.image_base) {
            Some(offset) => write!(out, "{EXE_NAME}+0x{offset:X}"),
            None => write!(out, "0x{:X}", self.raw),
        }
    }
}

/// `retModule`: the holding module's file name, `?` when none holds it, `null` for no address.
pub fn module_text(raw: usize, module: Option<&str>) -> &str {
    match (raw, module) {
        (0, _) => "null",
        (_, Some(name)) => name,
        (_, None) => "?",
    }
}

/// Drops the leading frames inside this DLL's image, then keeps at most [`MAX_FRAMES`].
pub fn trim<'a>(frames: &'a [usize], own: &Range<usize>) -> &'a [usize] {
    let first = frames
        .iter()
        .position(|frame| !own.contains(frame))
        .unwrap_or(frames.len());
    let kept = &frames[first..];
    &kept[..kept.len().min(MAX_FRAMES)]
}

/// The thread fields every exit line ends with.
#[derive(Debug, Clone, Copy)]
pub struct Tail<'a> {
    /// The calling thread.
    pub tid: u32,
    /// The server game thread, 0 when unknown.
    pub game_tid: u32,
    /// The first frame outside this DLL.
    pub ret: Addr<'a>,
}

impl Display for Tail<'_> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let game = u8::from(self.game_tid != 0 && self.tid == self.game_tid);
        write!(
            out,
            " tid={} gameTid={} isGameThread={game} retAddr={} retModule={}",
            self.tid,
            self.game_tid,
            self.ret,
            module_text(self.ret.raw, self.ret.module.map(|(name, _)| name))
        )
    }
}

/// `null`, `self` (the current-process pseudo handle) or hex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle(pub isize);

impl Display for Handle {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            0 => out.write_str("null"),
            -1 => out.write_str("self"),
            handle => write!(out, "0x{:X}", handle as usize),
        }
    }
}

/// An exit export's arguments, as its detour received them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCall {
    /// EXIT-1, EXIT-2.
    Code(u32),
    /// EXIT-6, 7, 9, 10.
    CodeI32(i32),
    /// EXIT-3.
    NtTerminate {
        /// The process handle.
        handle: isize,
        /// The status.
        status: i32,
    },
    /// EXIT-4.
    Terminate {
        /// The process handle.
        handle: isize,
        /// The exit code.
        code: u32,
    },
    /// EXIT-8, EXIT-11.
    Abort,
}

impl ExitCall {
    /// The stack line's `code=`: unsigned decimal, the status bits for EXIT-3, 3 for `abort`.
    pub fn stack_code(self) -> u32 {
        match self {
            ExitCall::Code(code) | ExitCall::Terminate { code, .. } => code,
            ExitCall::CodeI32(code) | ExitCall::NtTerminate { status: code, .. } => code as u32,
            ExitCall::Abort => ABORT_CODE,
        }
    }
}

/// The entry line of an exit export (R3 4.3).
pub fn entry_line(tag: &str, call: ExitCall, tail: &Tail<'_>) -> String {
    match call {
        ExitCall::Code(code) => format!("[MitExit] {tag} code={code}{tail}"),
        ExitCall::CodeI32(code) => format!("[MitExit] {tag} code={code}{tail}"),
        ExitCall::NtTerminate { handle, status } => {
            format!(
                "[MitExit] {tag} handle={} status=0x{:X}{tail}",
                Handle(handle),
                status as u32
            )
        }
        ExitCall::Terminate { handle, code } => format!("[MitExit] {tag} handle={} code={code}{tail}", Handle(handle)),
        ExitCall::Abort => format!("[MitExit] {tag}{tail}"),
    }
}

/// The stack line every exit, EXIT-5 and the handler write: `[MitExit] <tag> code=<n> stack=<addr> ...`.
pub fn write_stack_line(out: &mut dyn Write, tag: &str, code: u32, frames: &[Addr<'_>]) -> fmt::Result {
    write!(out, "[MitExit] {tag} code={code} stack=")?;
    for (index, frame) in frames.iter().enumerate() {
        if index > 0 {
            out.write_char(' ')?;
        }
        write!(out, "{frame}")?;
    }
    Ok(())
}

/// [`write_stack_line`] as a `String`.
pub fn stack_line(tag: &str, code: u32, frames: &[Addr<'_>]) -> String {
    let mut line = String::new();
    let _ = write_stack_line(&mut line, tag, code, frames);
    line
}

/// The registers the handler logs.
#[derive(Debug, Clone, Copy)]
pub struct Registers<'a> {
    /// `Rip`, as an address.
    pub rip: Addr<'a>,
    /// `Rsp`.
    pub rsp: u64,
    /// `Rax`.
    pub rax: u64,
    /// `Rcx`.
    pub rcx: u64,
    /// `Rdx`.
    pub rdx: u64,
    /// `Rbx`.
    pub rbx: u64,
    /// `Rsi`.
    pub rsi: u64,
    /// `Rdi`.
    pub rdi: u64,
    /// `R8`.
    pub r8: u64,
    /// `R9`.
    pub r9: u64,
}

/// One exception as the line shows it (R3 5.4).
#[derive(Debug, Clone, Copy)]
pub struct Exception<'a> {
    /// `VEH` or `kernelbase!RaiseException`.
    pub tag: &'a str,
    /// The exception code.
    pub code: u32,
    /// The flags.
    pub flags: u32,
    /// The exception address.
    pub address: Addr<'a>,
    /// The record's parameters, already cut by [`params_of`].
    pub params: &'a [usize],
    /// The handler's registers, when the context was readable and the code is not a stack overflow.
    pub registers: Option<Registers<'a>>,
}

/// The parameters a record holds: the first `min(count, 15)`.
pub fn params_of(count: u32, information: &[usize]) -> &[usize] {
    let count = usize::try_from(count).unwrap_or(usize::MAX).min(MAX_PARAMS);
    &information[..count.min(information.len())]
}

/// Writes the exception line into `out`; the handler's stack-overflow case writes into a fixed stack buffer.
pub fn write_exception_line(out: &mut dyn Write, exception: &Exception<'_>) -> fmt::Result {
    write!(
        out,
        "[MitException] {} code=0x{:X} flags=0x{:X} address={}",
        exception.tag, exception.code, exception.flags, exception.address
    )?;
    if let Some(rva) = exe_offset(exception.address.raw, exception.address.image_base) {
        write!(out, " rva=+0x{rva:X}")?;
    }
    for (index, param) in exception.params.iter().take(MAX_PARAMS).enumerate() {
        let lead = if index == 0 { " params=" } else { "," };
        write!(out, "{lead}0x{param:X}")?;
    }
    if let Some(regs) = exception.registers {
        write!(
            out,
            " rip={} rsp=0x{:X} regs{{rax=0x{:X} rcx=0x{:X} rdx=0x{:X} rbx=0x{:X} rsi=0x{:X} rdi=0x{:X} r8=0x{:X} \
             r9=0x{:X}}}",
            regs.rip, regs.rsp, regs.rax, regs.rcx, regs.rdx, regs.rbx, regs.rsi, regs.rdi, regs.r8, regs.r9
        )?;
    }
    Ok(())
}

/// [`write_exception_line`] as a `String`.
pub fn exception_line(exception: &Exception<'_>) -> String {
    let mut line = String::new();
    let _ = write_exception_line(&mut line, exception);
    line
}

/// A text buffer of `N` bytes on the stack; writes past the end are cut at a character boundary.
pub struct FixedText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> FixedText<N> {
    /// Empty.
    pub const fn new() -> Self {
        Self { bytes: [0; N], len: 0 }
    }

    /// The text written so far.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or_default()
    }
}

impl<const N: usize> Default for FixedText<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Write for FixedText<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for character in text.chars() {
            let mut buffer = [0u8; 4];
            let encoded = character.encode_utf8(&mut buffer).as_bytes();
            let end = self.len + encoded.len();
            if end > N {
                return Err(fmt::Error);
            }
            self.bytes[self.len..end].copy_from_slice(encoded);
            self.len = end;
        }
        Ok(())
    }
}

/// The bytes of the FName header load at [`crate::offsets::exit::FNAME_HEADER_LOAD`]: `movzx eax, word ptr [rdi]`.
pub const FNAME_HEADER_BYTES: [u8; 3] = [0x0F, 0xB7, 0x07];
/// The deferred line when the recovery is not armed.
pub const FNAME_NOT_ARMED_LINE: &str =
    "[MitException] FName recovery not armed: +0x2660278 is not the expected header load";
/// Recovery lines written before the throttle.
pub const FNAME_FIRST_LINES: u64 = 64;
/// After those, one line per this many occurrences.
pub const FNAME_EVERY: u64 = 1024;

/// The arming check (R3 5.2): the header load's bytes and the function holding it, as read at install.
pub fn fname_armed(bytes: Option<[u8; 3]>, entry: Option<u32>) -> bool {
    bytes == Some(FNAME_HEADER_BYTES) && entry == Some(crate::offsets::exit::FNAME_TEXT_FN)
}

/// Whether this exception is the FName text-read fault: an access violation exactly at `site` while armed.
pub fn is_fname_fault(code: u32, rip: u64, site: usize, armed: bool) -> bool {
    armed && code == ACCESS_VIOLATION && usize::try_from(rip).is_ok_and(|rip| rip == site)
}

/// Whether recovery number `occurrence` (from 1) writes its line.
pub fn fname_line_due(occurrence: u64) -> bool {
    (1..=FNAME_FIRST_LINES).contains(&occurrence) || (occurrence > 0 && occurrence.is_multiple_of(FNAME_EVERY))
}

/// The recovery line; `access` is the record's second parameter when it has one.
pub fn fname_line(old_rdi: u64, access: Option<usize>, occurrence: u64) -> String {
    let mut line = format!("[MitException] recovered invalid FName text read at +0x2660278 rdi=0x{old_rdi:X} access=");
    match access {
        Some(access) => {
            let _ = write!(line, "0x{access:X}");
        }
        None => line.push_str("(unknown)"),
    }
    if occurrence > FNAME_FIRST_LINES {
        let _ = write!(line, " (occurrence {occurrence})");
    }
    line
}

/// Counts recoveries; never resets.
pub struct Occurrences(AtomicU64);

impl Occurrences {
    /// None yet.
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// The number of this occurrence, from 1.
    pub fn next(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed).saturating_add(1)
    }
}

impl Default for Occurrences {
    fn default() -> Self {
        Self::new()
    }
}

/// UTF-16 units read from the error history at most (R3 6.2).
pub const ERROR_HISTORY_UNITS: usize = 1023;

/// Reads the error history unit by unit through `read` (a checked read of unit `index`), stopping at a 0, after
/// [`ERROR_HISTORY_UNITS`], or at the first unreadable unit (then `true`: truncated).
pub fn read_error_history(mut read: impl FnMut(usize) -> Option<u16>) -> (Vec<u16>, bool) {
    let mut units = Vec::new();
    for index in 0..ERROR_HISTORY_UNITS {
        match read(index) {
            None => return (units, true),
            Some(0) => break,
            Some(unit) => units.push(unit),
        }
    }
    (units, false)
}

/// The engine's error text as one UTF-8 line: CR dropped, LF as `\n`, other controls as `?`, unpaired surrogates as
/// U+FFFD; an unreadable unit appends ` <unreadable/truncated>`.
pub fn error_text(units: &[u16], truncated: bool) -> String {
    let mut text = String::with_capacity(units.len());
    for character in char::decode_utf16(units.iter().copied()) {
        match character.unwrap_or(char::REPLACEMENT_CHARACTER) {
            '\r' => {}
            '\n' => text.push_str("\\n"),
            control if u32::from(control) < 0x20 => text.push('?'),
            other => text.push(other),
        }
    }
    if truncated {
        text.push_str(" <unreadable/truncated>");
    }
    text
}

/// EXIT-13's line.
pub fn engine_error_line(text: &str) -> String {
    format!("[MitEngineError] fatal error (native handler preserved) msg={text}")
}

/// What EXIT-14 did at install (R3 7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashWait {
    /// The flag kept the native wait.
    Flag,
    /// The site is not inside `ReportCrash` on this executable.
    NotInReportCrash,
    /// `patch_bytes` ran.
    Patched(PatchResult),
}

/// Before patching: the flag, then the `.pdata` check; `None` means patch.
pub fn crash_wait_skip(flag: bool, entry: Option<u32>) -> Option<CrashWait> {
    if flag {
        Some(CrashWait::Flag)
    } else if entry != Some(crate::offsets::exit::REPORT_CRASH) {
        Some(CrashWait::NotInReportCrash)
    } else {
        None
    }
}

fn patch_text(result: PatchResult) -> &'static str {
    match result {
        PatchResult::Applied => "patched",
        PatchResult::AlreadyApplied => "already patched",
        PatchResult::Mismatch => "NOT patched (bytes differ)",
        PatchResult::Unreadable => "NOT patched (unreadable)",
        PatchResult::ProtectFailed => "NOT patched (protection failed)",
    }
}

/// The deferred `[MitCrashWait]` line.
pub fn crash_wait_line(outcome: CrashWait) -> String {
    match outcome {
        CrashWait::Flag => {
            "[MitCrashWait] DISABLE_CRASH_WAIT_SHORTEN.flag present: native 60000 ms crash wait kept".to_owned()
        }
        CrashWait::NotInReportCrash => {
            "[MitCrashWait] +0x265CF15 is not inside ReportCrash (+0x265CEC0) on this exe: not patched".to_owned()
        }
        CrashWait::Patched(result) => {
            format!(
                "[MitCrashWait] ReportCrash wait at +0x265CF15 60000->5000 ms: {}",
                patch_text(result)
            )
        }
    }
}

/// EXIT-14's install result.
pub fn crash_wait_install(outcome: CrashWait) -> Result<(), &'static str> {
    match outcome {
        CrashWait::Flag => Err("flag DISABLE_CRASH_WAIT_SHORTEN"),
        CrashWait::NotInReportCrash => Err("not in ReportCrash"),
        CrashWait::Patched(PatchResult::Applied | PatchResult::AlreadyApplied) => Ok(()),
        CrashWait::Patched(result) => Err(patch_text(result)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offsets::{self, FoundBy};
    use crate::patch::decide;

    const BASE: usize = 0x7FF6_0000_0000;

    fn exe(raw: usize) -> Addr<'static> {
        Addr {
            raw,
            module: None,
            image_base: BASE,
        }
    }

    #[test]
    fn export_table() {
        let expected = [
            ("kernel32.dll", "ExitProcess", "kernel32!ExitProcess"),
            ("ntdll.dll", "RtlExitUserProcess", "ntdll!RtlExitUserProcess"),
            ("ntdll.dll", "NtTerminateProcess", "ntdll!NtTerminateProcess"),
            ("kernelbase.dll", "TerminateProcess", "kernelbase!TerminateProcess"),
            ("kernelbase.dll", "RaiseException", "kernelbase!RaiseException"),
            ("ucrtbase.dll", "exit", "ucrtbase!exit"),
            ("ucrtbase.dll", "_exit", "ucrtbase!_exit"),
            ("ucrtbase.dll", "abort", "ucrtbase!abort"),
            ("msvcrt.dll", "exit", "msvcrt!exit"),
            ("msvcrt.dll", "_exit", "msvcrt!_exit"),
            ("msvcrt.dll", "abort", "msvcrt!abort"),
        ];
        assert_eq!(EXPORTS.len(), 11);
        for (index, (row, (module, name, tag))) in EXPORTS.iter().zip(expected).enumerate() {
            assert_eq!(row.id, format!("EXIT-{}", index + 1));
            assert_eq!((row.module, row.export, row.tag), (module, name, tag));
            let line = offsets::target(row.id).unwrap_or_else(|| panic!("{} has no target line", row.id));
            assert_eq!(line.found_by, FoundBy::Export(module, name), "{}", row.id);
            assert_eq!(line.rva, 0);
        }
        assert_eq!(export("EXIT-5").map(|row| row.shape), Some(Shape::Raise));
        assert!(export("EXIT-12").is_none());
    }

    #[test]
    fn address_text() {
        assert_eq!(exe(0).to_string(), "null");
        let module = Addr {
            raw: 0x7FFA_1234_5678,
            module: Some(("KERNELBASE.dll", 0x2_5678)),
            image_base: BASE,
        };
        assert_eq!(module.to_string(), "KERNELBASE.dll+0x25678");
        assert_eq!(exe(BASE).to_string(), "Dauntless-Win64-Shipping.exe+0x0");
        assert_eq!(
            exe(BASE + IMAGE_SPAN - 1).to_string(),
            "Dauntless-Win64-Shipping.exe+0x7FFFFFF"
        );
        assert_eq!(exe(BASE + IMAGE_SPAN).to_string(), "0x7FF608000000");
        assert_eq!(exe(0xABC).to_string(), "0xABC");
        assert_eq!(file_name(r"C:\Windows\System32\ntdll.dll"), "ntdll.dll");
        assert_eq!(file_name("Z:/game/x.exe"), "x.exe");
        assert_eq!(file_name("bare.dll"), "bare.dll");
        assert_eq!(module_text(0, None), "null");
        assert_eq!(module_text(1, None), "?");
        assert_eq!(module_text(1, Some("a.dll")), "a.dll");
    }

    #[test]
    fn trims_own_frames() {
        let own = 0x1000..0x2000;
        let mut frames = vec![0x1100, 0x1F00];
        frames.extend((0..46).map(|n| 0x9000 + n));
        let kept = trim(&frames, &own);
        assert_eq!(kept.len(), 32);
        assert_eq!(kept[0], 0x9000);
        assert!(trim(&[0x1000, 0x1FFF], &own).is_empty());
        assert_eq!(
            trim(&[0x10, 0x1100, 0x20], &own),
            [0x10, 0x1100, 0x20],
            "only leading frames go"
        );
        assert!(trim(&[], &own).is_empty());
    }

    fn tail(game_tid: u32) -> Tail<'static> {
        Tail {
            tid: 42,
            game_tid,
            ret: Addr {
                raw: 0x7FFA_0000_1000,
                module: Some(("KERNEL32.DLL", 0x1000)),
                image_base: BASE,
            },
        }
    }

    #[test]
    fn entry_lines() {
        let t = tail(0);
        let rest = " tid=42 gameTid=0 isGameThread=0 retAddr=KERNEL32.DLL+0x1000 retModule=KERNEL32.DLL";
        let line = |id: &str, call| entry_line(export(id).map(|row| row.tag).unwrap_or_default(), call, &t);
        assert_eq!(
            line("EXIT-1", ExitCall::Code(0)),
            format!("[MitExit] kernel32!ExitProcess code=0{rest}")
        );
        assert_eq!(
            line("EXIT-2", ExitCall::Code(0xC000_0005)),
            format!("[MitExit] ntdll!RtlExitUserProcess code=3221225477{rest}")
        );
        assert_eq!(
            line(
                "EXIT-3",
                ExitCall::NtTerminate {
                    handle: 0,
                    status: -1_073_741_510
                }
            ),
            format!("[MitExit] ntdll!NtTerminateProcess handle=null status=0xC000013A{rest}")
        );
        assert_eq!(
            line("EXIT-4", ExitCall::Terminate { handle: -1, code: 3 }),
            format!("[MitExit] kernelbase!TerminateProcess handle=self code=3{rest}")
        );
        assert_eq!(
            line("EXIT-4", ExitCall::Terminate { handle: 0x1F4, code: 0 }),
            format!("[MitExit] kernelbase!TerminateProcess handle=0x1F4 code=0{rest}")
        );
        for id in ["EXIT-6", "EXIT-7", "EXIT-9", "EXIT-10"] {
            let tag = export(id).map(|row| row.tag).unwrap_or_default();
            assert_eq!(
                line(id, ExitCall::CodeI32(-1)),
                format!("[MitExit] {tag} code=-1{rest}")
            );
            assert_eq!(ExitCall::CodeI32(-1).stack_code(), u32::MAX);
        }
        assert_eq!(
            line("EXIT-8", ExitCall::Abort),
            format!("[MitExit] ucrtbase!abort{rest}")
        );
        assert_eq!(
            line("EXIT-11", ExitCall::Abort),
            format!("[MitExit] msvcrt!abort{rest}")
        );
        assert_eq!(ExitCall::Abort.stack_code(), 3);
        assert_eq!(
            ExitCall::NtTerminate {
                handle: 0,
                status: -1_073_741_510
            }
            .stack_code(),
            0xC000_013A
        );
        let game = tail(42).to_string();
        assert!(game.contains(" gameTid=42 isGameThread=1 "), "{game}");
        let other = Tail { tid: 7, ..tail(42) }.to_string();
        assert!(other.contains(" isGameThread=0 "));
        let none = Tail { ret: exe(0), ..tail(0) }.to_string();
        assert!(none.ends_with(" retAddr=null retModule=null"), "{none}");
        let raw = Tail {
            ret: exe(0x55),
            ..tail(0)
        }
        .to_string();
        assert!(raw.ends_with(" retAddr=0x55 retModule=?"), "{raw}");
    }

    #[test]
    fn stack_lines() {
        let frames = [exe(BASE + 0x10), exe(0x20)];
        assert_eq!(
            stack_line("msvcrt!abort", 3, &frames),
            "[MitExit] msvcrt!abort code=3 stack=Dauntless-Win64-Shipping.exe+0x10 0x20"
        );
        assert_eq!(
            stack_line("VEH", 3_221_225_477, &[]),
            "[MitExit] VEH code=3221225477 stack="
        );
    }

    #[test]
    fn filter_and_budget() {
        for code in FILTERED {
            assert!(filtered(code), "{code:#x}");
        }
        for code in [ACCESS_VIOLATION, 0x8000_0003, STACK_OVERFLOW] {
            assert!(!filtered(code), "{code:#x}");
        }
        let budget = Budget::new();
        for n in 1..=BUDGET {
            assert!(budget.take(), "traced exception {n} logs");
        }
        assert!(!budget.take(), "the 97th does not");
        assert!(!budget.take());
    }

    fn registers() -> Registers<'static> {
        Registers {
            rip: exe(BASE + 0x266_0278),
            rsp: 0x10,
            rax: 1,
            rcx: 2,
            rdx: 3,
            rbx: 4,
            rsi: 5,
            rdi: 6,
            r8: 7,
            r9: 0xA,
        }
    }

    #[test]
    fn exception_lines() {
        let info: [usize; 15] = std::array::from_fn(|n| n + 1);
        let plain = Exception {
            tag: "kernelbase!RaiseException",
            code: 0xE000_0001,
            flags: 0,
            address: exe(0x1234),
            params: params_of(0, &info),
            registers: None,
        };
        assert_eq!(
            exception_line(&plain),
            "[MitException] kernelbase!RaiseException code=0xE0000001 flags=0x0 address=0x1234"
        );
        let one = Exception {
            address: exe(BASE + 0x2659A20),
            params: params_of(1, &info),
            ..plain
        };
        assert_eq!(
            exception_line(&one),
            "[MitException] kernelbase!RaiseException code=0xE0000001 flags=0x0 \
             address=Dauntless-Win64-Shipping.exe+0x2659A20 rva=+0x2659A20 params=0x1"
        );
        let all = exception_line(&Exception {
            params: params_of(15, &info),
            ..plain
        });
        assert!(
            all.ends_with(" params=0x1,0x2,0x3,0x4,0x5,0x6,0x7,0x8,0x9,0xA,0xB,0xC,0xD,0xE,0xF"),
            "{all}"
        );
        assert_eq!(params_of(20, &info).len(), 15, "a record claiming 20 gives 15");
        let veh = Exception {
            tag: "VEH",
            code: ACCESS_VIOLATION,
            flags: 0,
            address: exe(BASE + 0x266_0278),
            params: params_of(2, &[0, 0xDEAD]),
            registers: Some(registers()),
        };
        assert_eq!(
            exception_line(&veh),
            "[MitException] VEH code=0xC0000005 flags=0x0 address=Dauntless-Win64-Shipping.exe+0x2660278 \
             rva=+0x2660278 params=0x0,0xDEAD rip=Dauntless-Win64-Shipping.exe+0x2660278 rsp=0x10 \
             regs{rax=0x1 rcx=0x2 rdx=0x3 rbx=0x4 rsi=0x5 rdi=0x6 r8=0x7 r9=0xA}"
        );
        // Stack overflow: no registers, into a fixed buffer
        let overflow = Exception {
            code: STACK_OVERFLOW,
            registers: None,
            ..veh
        };
        let mut fixed = FixedText::<1024>::new();
        assert!(write_exception_line(&mut fixed, &overflow).is_ok());
        assert_eq!(fixed.as_str(), exception_line(&overflow));
        assert!(!fixed.as_str().contains("regs{"));
        let mut small = FixedText::<8>::new();
        assert!(write_exception_line(&mut small, &overflow).is_err());
        assert_eq!(small.as_str(), "[MitExce");
    }

    #[test]
    fn fname_decision_and_throttle() {
        let site = BASE + offsets::exit::FNAME_HEADER_LOAD as usize;
        let rip = site as u64;
        assert!(is_fname_fault(ACCESS_VIOLATION, rip, site, true));
        assert!(!is_fname_fault(0xC000_001D, rip, site, true));
        assert!(!is_fname_fault(ACCESS_VIOLATION, rip + 1, site, true));
        assert!(!is_fname_fault(ACCESS_VIOLATION, rip - 1, site, true));
        assert!(!is_fname_fault(ACCESS_VIOLATION, rip, site, false));
        assert!(fname_armed(Some([0x0F, 0xB7, 0x07]), Some(0x0266_01A0)));
        assert!(!fname_armed(Some([0x0F, 0xB7, 0x06]), Some(0x0266_01A0)));
        assert!(!fname_armed(Some([0x0F, 0xB7, 0x07]), Some(0x0266_01A1)));
        assert!(!fname_armed(None, Some(0x0266_01A0)));
        assert!(!fname_armed(Some([0x0F, 0xB7, 0x07]), None));
        for n in 1..=64 {
            assert!(fname_line_due(n));
        }
        assert!(!fname_line_due(0));
        assert!(!fname_line_due(65));
        assert!(!fname_line_due(1023));
        assert!(fname_line_due(1024));
        assert!(fname_line_due(2048));
        assert_eq!(
            fname_line(0xDEAD_0000, Some(0xDEAD_0000), 1),
            "[MitException] recovered invalid FName text read at +0x2660278 rdi=0xDEAD0000 access=0xDEAD0000"
        );
        assert_eq!(
            fname_line(0x10, None, 1024),
            "[MitException] recovered invalid FName text read at +0x2660278 rdi=0x10 access=(unknown) \
             (occurrence 1024)"
        );
        let count = Occurrences::new();
        assert_eq!((count.next(), count.next()), (1, 2));
    }

    fn history(text: &[u16]) -> (Vec<u16>, bool) {
        read_error_history(|index| Some(text.get(index).copied().unwrap_or(0)))
    }

    #[test]
    fn error_texts() {
        let (units, cut) = history(&crate::wide("Fatal error!"));
        assert_eq!(error_text(&units, cut), "Fatal error!");
        let (units, cut) = history(&crate::wide("a\r\nb\tc"));
        assert_eq!(error_text(&units, cut), "a\\nb?c");
        assert_eq!(error_text(&[0x61, 0xD800, 0x62], false), "a\u{FFFD}b");
        assert_eq!(error_text(&[0xD83D, 0xDE00], false), "\u{1F600}");
        let long = vec![u16::from(b'x'); 2000];
        let (units, cut) = history(&long);
        assert_eq!((units.len(), cut), (ERROR_HISTORY_UNITS, false));
        let (units, cut) = read_error_history(|index| (index < 3).then_some(u16::from(b'y')));
        assert_eq!(error_text(&units, cut), "yyy <unreadable/truncated>");
        assert_eq!(
            engine_error_line("x"),
            "[MitEngineError] fatal error (native handler preserved) msg=x"
        );
    }

    #[test]
    fn crash_wait() {
        let entry = Some(offsets::exit::REPORT_CRASH);
        assert_eq!(crash_wait_skip(true, entry), Some(CrashWait::Flag));
        assert_eq!(
            crash_wait_skip(false, Some(0x0265_CEC1)),
            Some(CrashWait::NotInReportCrash)
        );
        assert_eq!(crash_wait_skip(false, None), Some(CrashWait::NotInReportCrash));
        assert_eq!(crash_wait_skip(false, entry), None);
        let cases = [
            (
                CrashWait::Flag,
                "[MitCrashWait] DISABLE_CRASH_WAIT_SHORTEN.flag present: native 60000 ms crash wait kept",
                Err("flag DISABLE_CRASH_WAIT_SHORTEN"),
            ),
            (
                CrashWait::NotInReportCrash,
                "[MitCrashWait] +0x265CF15 is not inside ReportCrash (+0x265CEC0) on this exe: not patched",
                Err("not in ReportCrash"),
            ),
            (
                CrashWait::Patched(PatchResult::Applied),
                "[MitCrashWait] ReportCrash wait at +0x265CF15 60000->5000 ms: patched",
                Ok(()),
            ),
            (
                CrashWait::Patched(PatchResult::AlreadyApplied),
                "[MitCrashWait] ReportCrash wait at +0x265CF15 60000->5000 ms: already patched",
                Ok(()),
            ),
            (
                CrashWait::Patched(PatchResult::Mismatch),
                "[MitCrashWait] ReportCrash wait at +0x265CF15 60000->5000 ms: NOT patched (bytes differ)",
                Err("NOT patched (bytes differ)"),
            ),
            (
                CrashWait::Patched(PatchResult::Unreadable),
                "[MitCrashWait] ReportCrash wait at +0x265CF15 60000->5000 ms: NOT patched (unreadable)",
                Err("NOT patched (unreadable)"),
            ),
            (
                CrashWait::Patched(PatchResult::ProtectFailed),
                "[MitCrashWait] ReportCrash wait at +0x265CF15 60000->5000 ms: NOT patched (protection failed)",
                Err("NOT patched (protection failed)"),
            ),
        ];
        for (outcome, line, install) in cases {
            assert_eq!(crash_wait_line(outcome), line);
            assert_eq!(crash_wait_install(outcome), install);
        }
        let (expected, replacement) = (
            offsets::exit::REPORT_CRASH_WAIT_BYTES,
            offsets::exit::REPORT_CRASH_WAIT_PATCHED,
        );
        assert_eq!(decide(&expected, &expected, &replacement), PatchResult::Applied);
        assert_eq!(
            decide(&replacement, &expected, &replacement),
            PatchResult::AlreadyApplied
        );
        assert_eq!(decide(&[0x90; 5], &expected, &replacement), PatchResult::Mismatch);
    }
}
