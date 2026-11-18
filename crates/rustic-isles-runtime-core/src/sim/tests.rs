use super::*;

#[test]
fn grammar() {
    let commands = parse("# ramp\nbots 8\n\nwander 10\nbots 16 BP_Other_C\n");
    assert_eq!(
        commands,
        Commands {
            bots: Some((16, Some("BP_Other_C".to_owned()))),
            wander: Some(10),
            ignored: 0,
        }
    );
    assert_eq!(parse("bots 4\nclear").bots, Some((0, None)), "the last wins");
    assert_eq!(parse("wander 0").wander, Some(0));
    let bad = parse("bots 65\nwander 601\nbots 2 Bad-Class\nbots\nbots x\nfly 3\nbots 1 A B");
    assert_eq!((bad.bots, bad.wander, bad.ignored), (None, None, 7));
    let long = format!("bots 1 {}", "A".repeat(MAX_CLASS + 1));
    assert_eq!(parse(&long).ignored, 1);
}

#[test]
fn lines() {
    assert_eq!(file_name(7777), "rustic_isles_sim_port7777.txt");
    assert_eq!(
        read_line("rustic_isles_sim_port7777.txt", 8, DEFAULT_CLASS, 5, 0),
        "[MitRuntime] [Sim] read rustic_isles_sim_port7777.txt: target 8 bots of BP_PlayerCharacter_C, wander 5 s"
    );
    assert!(read_line("f", 0, "C", 0, 2).ends_with("wander 0 s, 2 lines ignored"));
    assert_eq!(count_line(8, 8, 8), "[MitRuntime] [Sim] bots 8/8 (+8)");
    assert_eq!(count_line(56, 0, -8), "[MitRuntime] [Sim] bots 56/0 (-8)");
    assert_eq!(
        streak_line(3),
        "[MitRuntime] [Sim] 8 spawns failed in a row: target lowered to 3"
    );
    assert_eq!(
        on_line(7777),
        "[MitRuntime] [Sim] on (RUNTIME_SIM_PLAYERS.flag): reading rustic_isles_sim_port7777.txt once a second"
    );
}

#[test]
fn plan() {
    assert_eq!(slots_to_fill(&[], 10), [0, 1, 2, 3], "at most 4 a step");
    assert_eq!(slots_to_fill(&[0, 2], 4), [1, 3], "the lowest free slots");
    assert!(slots_to_fill(&[0, 1], 2).is_empty());
    assert!(slots_to_fill(&[0, 1, 2], 1).is_empty());
    let all: Vec<usize> = (0..20).collect();
    assert_eq!(
        slots_to_remove(&all, 0),
        [19, 18, 17, 16, 15, 14, 13, 12],
        "at most 8, highest first"
    );
    assert_eq!(slots_to_remove(&[3, 0, 7], 1), [7, 3]);
    assert!(slots_to_remove(&[0], 4).is_empty());
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

#[test]
fn positions() {
    let origin = [100.0, -50.0, 20.0];
    let first = ring(origin, 0);
    assert!((first[0] - 400.0).abs() < 0.01 && (first[1] + 50.0).abs() < 0.01);
    assert_eq!(first[2], 20.0);
    assert!(
        (distance(ring(origin, 16), origin) - 450.0).abs() < 0.01,
        "the second ring"
    );
    assert!((distance(ring(origin, 63), origin) - 750.0).abs() < 0.01);
    let mut wander = Wander::new(12345);
    let goals: Vec<[f32; 3]> = (0..1000).map(|_| wander.goal(origin)).collect();
    assert!(
        goals
            .iter()
            .all(|&goal| distance(goal, origin) <= WANDER_RADIUS + 0.01 && goal[2] == 20.0)
    );
    assert!(goals.windows(2).all(|pair| pair[0] != pair[1]));
}

fn found(fields: &[(&str, i32)]) -> Vec<(String, i32)> {
    fields.iter().map(|&(name, at)| (name.to_owned(), at)).collect()
}

#[test]
fn layout_check() {
    let good = found(&[("Controller", 0), ("Goal", 8), ("Extra", 0x20)]);
    assert_eq!(check_layout("L.F", layouts::MOVE, Some(&good)), Ok(()));
    let shifted = found(&[("Controller", 0), ("Goal", 0xC)]);
    assert_eq!(
        check_layout("L.F", layouts::MOVE, Some(&shifted)),
        Err(
            "[MitRuntime] [Sim] L.F refused: expected Controller@0x0, Goal@0x8; found Controller@0x0, Goal@0xC"
                .to_owned()
        )
    );
    assert!(check_layout("L.F", layouts::MOVE, Some(&found(&[("Controller", 0)]))).is_err());
    assert_eq!(
        check_layout("L.F", layouts::RETURN, None),
        Err("[MitRuntime] [Sim] L.F refused: expected ReturnValue@0x0; found unreadable".to_owned())
    );
    assert_eq!(check_layout("A.K2_DestroyActor", layouts::NONE, Some(&[])), Ok(()));
    assert!(
        check_layout("A.B", layouts::NONE, None).is_err(),
        "an unreadable chain fails closed"
    );
    let spawn_end = layouts::SPAWN.iter().map(|&(_, at)| at).max().unwrap_or(0) + 8;
    assert_eq!(spawn_end, layouts::SPAWN_SIZE);
}
