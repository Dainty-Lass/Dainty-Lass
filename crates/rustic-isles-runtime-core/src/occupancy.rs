//! `SP-3@R21` (spec-runtime-r21 3): the `[NetConnEdge]` line the Director's Training Grounds occupancy reads
//! (`rustic-isles-director`'s `occupancy`). Its text is an interface, so it carries no `[MitRuntime]` tag.

/// After every other SP-3 after handler (call 8).
pub const ORDER: i32 = 1900;
/// `last` before the first report.
pub const INITIAL: i32 = -2;
/// The count and max when there is no driver or the read failed.
pub const UNKNOWN: i32 = -1;

/// The game-thread state: SP-3 calls so far and the last count written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnEdge {
    frame: u64,
    last: i32,
}

impl Default for ConnEdge {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnEdge {
    /// No call yet, nothing reported.
    pub const fn new() -> Self {
        Self {
            frame: 0,
            last: INITIAL,
        }
    }

    /// Counts one SP-3 call (step 1); the first engine tick is 1.
    pub fn tick(&mut self) -> u64 {
        self.frame += 1;
        self.frame
    }

    /// Steps 4 and 5: the line when `count` differs from the last one written, which it then becomes.
    pub fn report(&mut self, count: i32, max: i32, driver: usize) -> Option<String> {
        if count == self.last {
            return None;
        }
        let text = line(self.frame, self.last, count, max, driver);
        self.last = count;
        Some(text)
    }
}

/// `[NetConnEdge] tick=<frame> connCount <last> -> <count> connMax=<max> NetDriver=<decimal|null>`.
pub fn line(frame: u64, last: i32, count: i32, max: i32, driver: usize) -> String {
    let driver = if driver == 0 {
        "null".to_owned()
    } else {
        driver.to_string()
    };
    format!("[NetConnEdge] tick={frame} connCount {last} -> {count} connMax={max} NetDriver={driver}")
}

/// `ClientConnections` `Num` and `Max` from the 8 bytes at driver `+0x98`.
pub fn counts(bytes: [u8; 8]) -> (i32, i32) {
    let [a, b, c, d, e, f, g, h] = bytes;
    (i32::from_le_bytes([a, b, c, d]), i32::from_le_bytes([e, f, g, h]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges() {
        let mut edge = ConnEdge::new();
        let driver = 2_730_612_336;
        let mut lines = Vec::new();
        for (count, max) in [(0, 0), (0, 0), (1, 4), (1, 4), (0, 4), (UNKNOWN, UNKNOWN)] {
            let frame = edge.tick();
            let driver = if count == UNKNOWN { 0 } else { driver };
            lines.extend(edge.report(count, max, driver).map(|line| (frame, line)));
        }
        assert_eq!(
            lines,
            [
                (
                    1,
                    "[NetConnEdge] tick=1 connCount -2 -> 0 connMax=0 NetDriver=2730612336".to_owned()
                ),
                (
                    3,
                    "[NetConnEdge] tick=3 connCount 0 -> 1 connMax=4 NetDriver=2730612336".to_owned()
                ),
                (
                    5,
                    "[NetConnEdge] tick=5 connCount 1 -> 0 connMax=4 NetDriver=2730612336".to_owned()
                ),
                (
                    6,
                    "[NetConnEdge] tick=6 connCount 0 -> -1 connMax=-1 NetDriver=null".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn text() {
        assert_eq!(
            line(9077, 1, 0, 4, 0xFFFF_FFFF_FFFF_FFF0),
            "[NetConnEdge] tick=9077 connCount 1 -> 0 connMax=4 NetDriver=18446744073709551600"
        );
        assert_eq!(
            line(40, INITIAL, UNKNOWN, UNKNOWN, 0),
            "[NetConnEdge] tick=40 connCount -2 -> -1 connMax=-1 NetDriver=null"
        );
        assert_eq!(counts([3, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]), (3, -1));
        assert_eq!(ORDER, 1900);
    }
}
