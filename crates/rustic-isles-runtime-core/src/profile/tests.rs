use super::*;

#[test]
fn schedule() {
    assert_eq!(next_window(1000, None), 91_000);
    assert_eq!(next_window(1000, Some(91_000)), 391_000);
}

#[test]
fn tallies() {
    let mut profile = Profile::default();
    profile.add(&[1, 2, 3]);
    profile.add(&[1, 2, 2, 3]);
    profile.add(&[4, 3]);
    profile.add(&[]);
    assert_eq!((profile.samples, profile.missed), (3, 1));
    assert_eq!(profile.own, HashMap::from([(1, 2), (4, 1)]));
    assert_eq!(
        profile.total,
        HashMap::from([(1, 2), (2, 2), (3, 3), (4, 1)]),
        "recursion counts once"
    );
    let mut functions = profile.functions();
    functions.sort_unstable();
    assert_eq!(functions, [1, 2, 3, 4]);
}

#[test]
fn texts() {
    let mut profile = Profile::default();
    for _ in 0..3 {
        profile.add(&[0x1010, 0x2000]);
    }
    profile.add(&[0x2000]);
    let context = Context {
        ms: 2000,
        connections: 0,
        role: HubRole::Hub,
        bots: 32,
    };
    let name = |address: usize| label(address, Some(("game.exe", 0x1000)));
    assert_eq!(
        lines(&profile, &context, &name),
        [
            "[MitRuntime] [Profile] game thread: 4 samples in 2.0 s (2/s, 0 missed), connections 0, role hub, bots 32",
            "[MitRuntime] [Profile] self 1/2 game.exe+0x10: 75.0% (3)",
            "[MitRuntime] [Profile] self 2/2 game.exe+0x1000: 25.0% (1)",
            "[MitRuntime] [Profile] total 1/2 game.exe+0x1000: 100.0% (4)",
            "[MitRuntime] [Profile] total 2/2 game.exe+0x10: 75.0% (3)",
            "[MitRuntime] [Profile] stack 1/2 75.0% (3): game.exe+0x10 < game.exe+0x1000",
            "[MitRuntime] [Profile] stack 2/2 25.0% (1): game.exe+0x1000",
        ]
    );
    assert_eq!(label(0x55, None), "0x55");
    let many: Vec<usize> = (0..40).collect();
    let mut wide = Profile::default();
    for function in &many {
        wide.add(&[*function]);
    }
    let out = lines(&wide, &context, &|address| format!("f{address:02}"));
    assert_eq!(out.iter().filter(|line| line.contains("] self ")).count(), TOP);
}
