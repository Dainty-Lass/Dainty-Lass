//! The exports: each forwards to the same-named function of the system `winmm.dll` or `version.dll`.
//!
//! Every parameter and result is register-width: on x86_64 none of these functions passes a float or a struct by
//! value, so `usize` carries each argument through bit for bit, as long as the count matches the SDK's.

use crate::forward::{self, Slot};

macro_rules! exports {
    ($($library:ident { $($name:ident($($arg:ident),*) $(else $fallback:expr)?;)* })*) => {
        $($(
            #[unsafe(no_mangle)]
            #[allow(clippy::too_many_arguments, reason = "the SDK's parameter count")]
            pub unsafe extern "system" fn $name($($arg: usize),*) -> usize {
                static SLOT: Slot = Slot::new();
                match SLOT.get(&forward::$library, concat!(stringify!($name), "\0")) {
                    // SAFETY: the address is the system function of this name, which takes these arguments.
                    Some(address) => unsafe {
                        let function = std::mem::transmute::<
                            usize,
                            unsafe extern "system" fn($(exports!(@usize $arg)),*) -> usize,
                        >(address);
                        function($($arg),*)
                    },
                    None => exports!(@fallback $($fallback)?),
                }
            }
        )*)*

        /// Every export's name and parameter count.
        #[cfg(test)]
        pub const TABLE: &[(&str, usize)] = &[$($((stringify!($name), <[&str]>::len(&[$(stringify!($arg)),*]))),*),*];
    };
    (@usize $arg:ident) => { usize };
    (@fallback) => { 0 };
    (@fallback $fallback:expr) => { $fallback };
}

exports! {
    VERSION {
        GetFileVersionInfoA(a, b, c, d);
        GetFileVersionInfoSizeA(a, b);
        GetFileVersionInfoW(a, b, c, d);
        GetFileVersionInfoSizeW(a, b);
        VerQueryValueA(a, b, c, d);
        VerQueryValueW(a, b, c, d);
        // A `version.dll` older than the `Ex` functions: the loader's own non-`Ex` export, `dwFlags` dropped.
        // SAFETY: the caller's own arguments, passed on.
        GetFileVersionInfoExA(a, b, c, d, e) else unsafe { GetFileVersionInfoA(b, c, d, e) };
        GetFileVersionInfoExW(a, b, c, d, e) else unsafe { GetFileVersionInfoW(b, c, d, e) };
        GetFileVersionInfoSizeExA(a, b, c) else unsafe { GetFileVersionInfoSizeA(b, c) };
        GetFileVersionInfoSizeExW(a, b, c) else unsafe { GetFileVersionInfoSizeW(b, c) };
        VerFindFileA(a, b, c, d, e, f, g, h) else 1;
        VerFindFileW(a, b, c, d, e, f, g, h) else 1;
        VerInstallFileA(a, b, c, d, e, f, g, h) else 1;
        VerInstallFileW(a, b, c, d, e, f, g, h) else 1;
        VerLanguageNameA(a, b, c);
        VerLanguageNameW(a, b, c);
        GetFileVersionInfoByHandle(a, b, c, d, e);
    }
    WINMM {
        // waveIn
        waveInAddBuffer(a, b, c);
        waveInClose(a);
        waveInGetDevCapsA(a, b, c);
        waveInGetDevCapsW(a, b, c);
        waveInGetErrorTextA(a, b, c);
        waveInGetErrorTextW(a, b, c);
        waveInGetID(a, b);
        waveInGetNumDevs();
        waveInGetPosition(a, b, c);
        waveInMessage(a, b, c, d);
        waveInOpen(a, b, c, d, e, f);
        waveInPrepareHeader(a, b, c);
        waveInReset(a);
        waveInStart(a);
        waveInStop(a);
        waveInUnprepareHeader(a, b, c);
        // waveOut
        waveOutBreakLoop(a);
        waveOutClose(a);
        waveOutGetDevCapsA(a, b, c);
        waveOutGetDevCapsW(a, b, c);
        waveOutGetErrorTextA(a, b, c);
        waveOutGetErrorTextW(a, b, c);
        waveOutGetID(a, b);
        waveOutGetNumDevs();
        waveOutGetPitch(a, b);
        waveOutGetPlaybackRate(a, b);
        waveOutGetPosition(a, b, c);
        waveOutGetVolume(a, b);
        waveOutMessage(a, b, c, d);
        waveOutOpen(a, b, c, d, e, f);
        waveOutPause(a);
        waveOutPrepareHeader(a, b, c);
        waveOutReset(a);
        waveOutRestart(a);
        waveOutSetPitch(a, b);
        waveOutSetPlaybackRate(a, b);
        waveOutSetVolume(a, b);
        waveOutUnprepareHeader(a, b, c);
        waveOutWrite(a, b, c);
        // midi
        midiConnect(a, b, c);
        midiDisconnect(a, b, c);
        // midiIn
        midiInAddBuffer(a, b, c);
        midiInClose(a);
        midiInGetDevCapsA(a, b, c);
        midiInGetDevCapsW(a, b, c);
        midiInGetErrorTextA(a, b, c);
        midiInGetErrorTextW(a, b, c);
        midiInGetID(a, b);
        midiInGetNumDevs();
        midiInMessage(a, b, c, d);
        midiInOpen(a, b, c, d, e);
        midiInPrepareHeader(a, b, c);
        midiInReset(a);
        midiInStart(a);
        midiInStop(a);
        midiInUnprepareHeader(a, b, c);
        // midiOut
        midiOutCacheDrumPatches(a, b, c, d);
        midiOutCachePatches(a, b, c, d);
        midiOutClose(a);
        midiOutGetDevCapsA(a, b, c);
        midiOutGetDevCapsW(a, b, c);
        midiOutGetErrorTextA(a, b, c);
        midiOutGetErrorTextW(a, b, c);
        midiOutGetID(a, b);
        midiOutGetNumDevs();
        midiOutGetVolume(a, b);
        midiOutLongMsg(a, b, c);
        midiOutMessage(a, b, c, d);
        midiOutOpen(a, b, c, d, e);
        midiOutPrepareHeader(a, b, c);
        midiOutReset(a);
        midiOutSetVolume(a, b);
        midiOutShortMsg(a, b);
        midiOutUnprepareHeader(a, b, c);
        // midiStream
        midiStreamClose(a);
        midiStreamOpen(a, b, c, d, e, f);
        midiStreamOut(a, b, c);
        midiStreamPause(a);
        midiStreamPosition(a, b, c);
        midiStreamProperty(a, b, c);
        midiStreamRestart(a);
        midiStreamStop(a);
        // mixer
        mixerClose(a);
        mixerGetControlDetailsA(a, b, c);
        mixerGetControlDetailsW(a, b, c);
        mixerGetDevCapsA(a, b, c);
        mixerGetDevCapsW(a, b, c);
        mixerGetID(a, b, c);
        mixerGetLineControlsA(a, b, c);
        mixerGetLineControlsW(a, b, c);
        mixerGetLineInfoA(a, b, c);
        mixerGetLineInfoW(a, b, c);
        mixerGetNumDevs();
        mixerMessage(a, b, c, d);
        mixerOpen(a, b, c, d, e);
        mixerSetControlDetails(a, b, c);
        // time
        timeBeginPeriod(a);
        timeEndPeriod(a);
        timeGetDevCaps(a, b);
        timeGetSystemTime(a, b);
        timeGetTime();
        timeKillEvent(a);
        timeSetEvent(a, b, c, d, e);
        // joy
        joyConfigChanged(a);
        joyGetDevCapsA(a, b, c);
        joyGetDevCapsW(a, b, c);
        joyGetNumDevs();
        joyGetPos(a, b);
        joyGetPosEx(a, b);
        joyGetThreshold(a, b);
        joyReleaseCapture(a);
        joySetCapture(a, b, c, d);
        joySetThreshold(a, b);
        // mci
        mciDriverNotify(a, b, c);
        mciDriverYield(a);
        mciExecute(a);
        mciFreeCommandResource(a);
        mciGetCreatorTask(a);
        mciGetDeviceIDA(a);
        mciGetDeviceIDFromElementIDA(a, b);
        mciGetDeviceIDFromElementIDW(a, b);
        mciGetDeviceIDW(a);
        mciGetDriverData(a);
        mciGetErrorStringA(a, b, c);
        mciGetErrorStringW(a, b, c);
        mciGetYieldProc(a, b);
        mciLoadCommandResource(a, b, c);
        mciSendCommandA(a, b, c, d);
        mciSendCommandW(a, b, c, d);
        mciSendStringA(a, b, c, d);
        mciSendStringW(a, b, c, d);
        mciSetDriverData(a, b);
        mciSetYieldProc(a, b, c);
        // mmio
        mmioAdvance(a, b, c);
        mmioAscend(a, b, c);
        mmioClose(a, b);
        mmioCreateChunk(a, b, c);
        mmioDescend(a, b, c, d);
        mmioFlush(a, b);
        mmioGetInfo(a, b, c);
        mmioInstallIOProcA(a, b, c);
        mmioInstallIOProcW(a, b, c);
        mmioOpenA(a, b, c);
        mmioOpenW(a, b, c);
        mmioRead(a, b, c);
        mmioRenameA(a, b, c, d);
        mmioRenameW(a, b, c, d);
        mmioSeek(a, b, c);
        mmioSendMessage(a, b, c, d);
        mmioSetBuffer(a, b, c, d);
        mmioSetInfo(a, b, c);
        mmioStringToFOURCCA(a, b);
        mmioStringToFOURCCW(a, b);
        mmioWrite(a, b, c);
        // sound
        PlaySound(a, b, c);
        PlaySoundA(a, b, c);
        PlaySoundW(a, b, c);
        sndPlaySoundA(a, b);
        sndPlaySoundW(a, b);
        // aux
        auxGetDevCapsA(a, b, c);
        auxGetDevCapsW(a, b, c);
        auxGetNumDevs();
        auxGetVolume(a, b);
        auxOutMessage(a, b, c, d);
        auxSetVolume(a, b);
        // driver
        CloseDriver(a, b, c);
        DefDriverProc(a, b, c, d, e);
        DriverCallback(a, b, c, d, e, f, g);
        DrvGetModuleHandle(a);
        GetDriverModuleHandle(a);
        OpenDriver(a, b, c);
        SendDriverMessage(a, b, c, d);
        // mm
        mmsystemGetVersion();
        mmDrvInstall(a, b, c, d);
        mmGetCurrentTask();
        mmTaskBlock(a);
        mmTaskCreate(a, b, c);
        mmTaskSignal(a);
        mmTaskYield();
        WOWAppExit(a);
    }
}

#[cfg(test)]
mod tests {
    use super::TABLE;

    const EXPECTED: &[(&str, usize)] = &include!("../tests/expected_exports.in");

    #[test]
    fn exports_match_the_spec() {
        let mut names: Vec<&str> = TABLE.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TABLE.len(), "duplicate export");

        let mut expected = EXPECTED.to_vec();
        expected.sort_unstable();
        let mut actual = TABLE.to_vec();
        actual.sort_unstable();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 197);
    }
}
