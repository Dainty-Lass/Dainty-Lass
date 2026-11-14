//! Every `[MitRepGraph]` text (4.3): addresses as `0x<HEX>` (`null` for 0), RVAs as `+0x<HEX>`, counts in decimal.

use super::Source;

/// R12's tag.
pub const TAG: &str = "[MitRepGraph]";

/// `0x<HEX>`, upper case, no padding; `null` for 0.
pub fn addr(ptr: usize) -> String {
    if ptr == 0 {
        "null".to_owned()
    } else {
        format!("0x{ptr:X}")
    }
}

/// `+0x<HEX>`.
pub fn rva(rva: usize) -> String {
    format!("+0x{rva:X}")
}

/// The line a budget writes in place of its `B+1`-th.
pub fn suppressed(kind: &str) -> String {
    format!("{TAG} {kind}: further lines suppressed")
}

/// Budget kinds.
pub mod kinds {
    /// 6, queue.
    pub const QUEUED: &str = "quarantine queued";
    /// 6, stale.
    pub const STALE: &str = "quarantine stale";
    /// 7 step 3.
    pub const PRE_RESET: &str = "pre-channel reset";
    /// 7 step 4.
    pub const PRE_UNPROVEN: &str = "pre-channel unproven";
    /// 7 step 7, past the pair set.
    pub const PRE_BLOCKED: &str = "pre-channel blocked";
    /// 8 step 2.
    pub const BIND_QUARANTINED: &str = "bind quarantined";
    /// 8 step 3.
    pub const BIND_REFUSED: &str = "bind refused";
    /// 9, past the pair set.
    pub const SEND_BLOCKED: &str = "send blocked";
}

/// 6, queue.
pub fn queued(channel: usize, actor: usize, wrong: usize, correct: usize, source: Source, caller: usize) -> String {
    format!(
        "{TAG} quarantine: queued channel={} actor={} wrong={} correct={} source={} caller={}",
        addr(channel),
        addr(actor),
        addr(wrong),
        addr(correct),
        source.text(),
        rva(caller)
    )
}

/// 6, no free entry.
pub fn table_full(channel: usize, actor: usize, wrong: usize, correct: usize) -> String {
    format!(
        "{TAG} quarantine: table full channel={} actor={} wrong={} correct={}",
        addr(channel),
        addr(actor),
        addr(wrong),
        addr(correct)
    )
}

/// 6, stale.
pub fn dropped_stale(
    channel: usize,
    expected_actor: usize,
    current_actor: usize,
    expected_connection: usize,
    current_connection: usize,
) -> String {
    format!(
        "{TAG} quarantine: dropped stale channel={} expectedActor={} currentActor={} expectedConnection={} \
         currentConnection={}",
        addr(channel),
        addr(expected_actor),
        addr(current_actor),
        addr(expected_connection),
        addr(current_connection)
    )
}

/// 7 step 3's reasons.
pub mod reset_reason {
    /// The cached channel serves another connection.
    pub const CONNECTION: &str = "cached-channel-connection-mismatch";
    /// The cached channel carries another actor.
    pub const ACTOR: &str = "cached-channel-actor-mismatch";
    /// The cached channel or its connection is not live.
    pub const UNUSABLE: &str = "cached-channel-unusable";
}

/// 7 step 3.
pub struct Reset {
    /// The controller.
    pub actor: usize,
    /// Its own connection.
    pub actor_connection: usize,
    /// The manager's connection.
    pub target: usize,
    /// The cached channel.
    pub channel: usize,
    /// Its connection.
    pub channel_connection: usize,
    /// Its actor.
    pub channel_actor: usize,
    /// 1 written, 0 no longer matched, -1 the write's check failed.
    pub cleared: i32,
    /// One of `reset_reason`.
    pub reason: &'static str,
}

/// 7 step 3.
pub fn pre_reset(reset: &Reset) -> String {
    format!(
        "{TAG} pre-channel: reset cached channel actor={} actorConnection={} target={} channel={} \
         channelConnection={} channelActor={} cleared={} reason={}",
        addr(reset.actor),
        addr(reset.actor_connection),
        addr(reset.target),
        addr(reset.channel),
        addr(reset.channel_connection),
        addr(reset.channel_actor),
        reset.cleared,
        reset.reason
    )
}

/// 7 step 4.
pub fn pre_unproven(actor: usize, manager: usize, target: usize, frame: u32) -> String {
    format!(
        "{TAG} pre-channel: target connection unproven, allowed actor={} manager={} target={} frame={frame}",
        addr(actor),
        addr(manager),
        addr(target)
    )
}

/// 7 step 7 and 9: who was blocked from whom.
pub struct Blocked {
    /// The controller.
    pub actor: usize,
    /// Its own connection.
    pub actor_connection: usize,
    /// The connection it was kept off.
    pub target: usize,
    /// That connection's controller.
    pub target_controller: usize,
    /// That connection's owning actor.
    pub target_owner: usize,
    /// 5.3's reason.
    pub reason: &'static str,
}

/// 7 step 7.
pub fn pre_blocked(blocked: &Blocked, channel_before: usize, frame: u32) -> String {
    format!(
        "{TAG} pre-channel: blocked actor={} actorConnection={} target={} targetController={} targetOwner={} \
         channelBefore={} frame={frame} reason={}",
        addr(blocked.actor),
        addr(blocked.actor_connection),
        addr(blocked.target),
        addr(blocked.target_controller),
        addr(blocked.target_owner),
        addr(channel_before),
        blocked.reason
    )
}

/// 8 step 2.
pub fn bind_quarantined(channel: usize, actor: usize, caller: usize) -> String {
    format!(
        "{TAG} bind: refused quarantined channel={} actor={} caller={}",
        addr(channel),
        addr(actor),
        rva(caller)
    )
}

/// 8 step 3.
pub struct Refused {
    /// The controller.
    pub actor: usize,
    /// Its own connection.
    pub actor_connection: usize,
    /// The channel.
    pub channel: usize,
    /// The channel's connection.
    pub channel_connection: usize,
    /// That connection's controller.
    pub channel_controller: usize,
    /// That connection's owning actor.
    pub channel_owner: usize,
    /// The channel's actor before the bind.
    pub previous_actor: usize,
    /// The bind's flags, passed through.
    pub flags: u64,
    /// The caller's RVA.
    pub caller: usize,
    /// 5.3's reason.
    pub reason: &'static str,
    /// Whether the queue took it.
    pub queued: bool,
}

/// 8 step 3.
pub fn bind_refused(refused: &Refused) -> String {
    format!(
        "{TAG} bind: refused actor={} actorConnection={} channel={} channelConnection={} channelController={} \
         channelOwner={} previousActor={} flags=0x{:X} caller={} reason={} queued={}",
        addr(refused.actor),
        addr(refused.actor_connection),
        addr(refused.channel),
        addr(refused.channel_connection),
        addr(refused.channel_controller),
        addr(refused.channel_owner),
        addr(refused.previous_actor),
        refused.flags,
        rva(refused.caller),
        refused.reason,
        u8::from(refused.queued)
    )
}

/// 9 step 2.
pub fn send_blocked(blocked: &Blocked, flags: u8) -> String {
    format!(
        "{TAG} send: blocked actor={} actorConnection={} target={} targetController={} targetOwner={} \
         flags=0x{flags:X} reason={}",
        addr(blocked.actor),
        addr(blocked.actor_connection),
        addr(blocked.target),
        addr(blocked.target_controller),
        addr(blocked.target_owner),
        blocked.reason
    )
}

/// 10's reasons.
pub mod rpc_reason {
    /// The channel is in the quarantine.
    pub const QUARANTINED: &str = "quarantined-channel";
    /// The channel's actor is null or unreadable.
    pub const WITHOUT_ACTOR: &str = "channel-without-actor";
}

/// 10 step 3. `function` and `target` are names, `null` or `unreadable`.
pub fn rpc_dropped(
    function: &str,
    target: &str,
    channel: usize,
    connection: usize,
    reason: &str,
    count: u32,
) -> String {
    format!(
        "{TAG} rpc: dropped fn={function} target={target} channel={} connection={} reason={reason} count={count}",
        addr(channel),
        addr(connection)
    )
}

/// 10's one state line.
pub fn rpc_guard(on: bool) -> String {
    if on {
        format!("{TAG} rpc guard on")
    } else {
        format!("{TAG} rpc guard off (DISABLE_RPC_UNBOUND_CHANNEL_GUARD.flag)")
    }
}

/// 11.2 step 6.
pub fn drive_not_function(graph: usize, slot: usize) -> String {
    format!(
        "{TAG} drive: slot +0x2F8 of graph={} is {}, not a function of the game; not called",
        addr(graph),
        addr(slot)
    )
}

/// 11.2 step 9.
pub fn drive(graph: usize, class: &str, function_rva: usize, result: i32, connections: i32, deferred: i32) -> String {
    format!(
        "{TAG} drive: graph={} class={class} fn={} -> {result} connections={connections} deferred={deferred}",
        addr(graph),
        rva(function_rva)
    )
}
