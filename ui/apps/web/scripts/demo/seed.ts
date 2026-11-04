/**
 * The demo's database (scripts/demo.ts), made through the app's own functions so every row is one the
 * dashboard could have written, audit trail included: staff of every tier and a newcomer waiting for
 * approval, settings with history, chat filter terms, three days of chat in every verdict, reports in
 * every state with notes, a reverted change, and a fleet of controllers in every state with a week of
 * samples. Run with the demo's environment; prints the agents' pairing codes as JSON.
 */
import { randomUUID } from "node:crypto";
import { eq, sql } from "drizzle-orm";
import { queryAudit } from "@/lib/audit/query";
import { revertEvent } from "@/lib/audit/revert";
import { type IncomingChat, ingestChat } from "@/lib/chat/store";
import { createTerm, updateTerm } from "@/lib/chat/terms";
import { db, schema } from "@/lib/db";
import type { Staff } from "@/lib/db/schema";
import { createController, ensureLocal } from "@/lib/fleet/controllers";
import { addNote, claimReport, closeReport, createReport } from "@/lib/reports/store";
import { updateSettings } from "@/lib/settings-change";
import { resolveSignIn } from "@/lib/staff";
import { demoPlayer } from "./game";

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const now = Date.now();

let seq = 0;
function member(
  name: string,
  discordId: string,
  role: Staff["role"] | null,
  approved = true,
): Staff {
  const at = new Date(now - (30 - seq) * DAY);
  const user = db
    .insert(schema.authUsers)
    .values({
      name,
      email: `${name.toLowerCase()}@staff.invalid`,
      emailVerified: true,
      createdAt: at,
      updatedAt: at,
    })
    .returning()
    .get();
  db.insert(schema.authAccounts)
    .values({
      userId: user.id,
      providerId: "discord",
      accountId: discordId,
      createdAt: at,
      updatedAt: at,
    })
    .run();
  seq++;
  const outcome = resolveSignIn(
    { userId: user.id, name, image: null, email: user.email, discordId },
    role && role !== "owner" ? { role, approved } : { role: null, approved },
  );
  if (!outcome.ok) throw new Error(`${name}: ${outcome.reason}`);
  return outcome.staff;
}

// Rook's id is DASHBOARD_OWNER_DISCORD_IDS in the demo's environment: the first owner.
const rook = member("Rook", "100000000000000001", "owner");
const wren = member("Wren", "100000000000000002", "admin");
const ash = member("Ash", "100000000000000003", "moderator");
const juniper = member("Juniper", "100000000000000004", "moderator");
member("Pike", "100000000000000005", "user");
member("Moss", "100000000000000006", null, false);

updateSettings(rook, {
  serverName: "Dainty Friends",
  accent: "teal",
  reportContextMinutes: 15,
  requireApproval: true,
  tosText:
    "## House rules\n\n- Be kind in city chat.\n- No cheating, exploits or real-money trading.\n- Report problems with `/report`.\n\nBreaking these can end in a timeout or a ban.",
});
updateSettings(wren, {
  moderationReasons: [
    "Harassment",
    "Hate speech",
    "Cheating",
    "Griefing",
    "Spam",
    "Inappropriate name",
    "RMT",
  ],
});

const term = (
  pattern: string,
  matchType: "word" | "substring" | "regex",
  action: "flag" | "mask" | "block",
  category: string,
  note: string | null = null,
) => ({
  pattern,
  matchType,
  action,
  category,
  note,
  enabled: true,
  channels: null,
});
createTerm(
  rook,
  term("noob", "word", "flag", "toxicity", "Usually banter; worth a look in context"),
);
createTerm(rook, term("darn", "word", "mask", "profanity"));
createTerm(
  ash,
  term(
    "(buy|sell)(ing)?\\s+(gold|aether|cells)",
    "regex",
    "block",
    "rmt",
    "Real-money trading adverts",
  ),
);
createTerm(ash, term("discord.gg/", "substring", "flag", "links"));
const typo = createTerm(juniper, term("cheeter", "word", "flag", "cheating"));
// A change and its revert, for the audit page's rollback
updateTerm(juniper, typo.id, {
  ...term("cheeter", "word", "block", "cheating"),
  note: "Too strict?",
});
const last = queryAudit({ pageSize: 1 }).events[0];
if (last) await revertEvent(wren, last.id);

// Fleet: this host, two agents the demo runs, one waiting for its agent, one gone quiet, one disabled
ensureLocal();
const frontier = createController(rook, { name: "Frontier", region: "eu-west" });
const aurora = createController(rook, { name: "Aurora", region: "us-east" });
createController(wren, { name: "Lantern", region: "ap-south" });
const quiet = createController(rook, { name: "Old Ironworks", region: "eu-north" });
db.update(schema.controllers)
  .set({
    tokenHash: randomUUID(),
    pairingCodeHash: null,
    pairingExpiresAt: null,
    pairedAt: now - 20 * DAY,
    lastSeenAt: now - 5 * HOUR,
    version: "0.1.0",
  })
  .where(eq(schema.controllers.id, quiet.controller.id))
  .run();
const retired = createController(rook, { name: "Testbed", region: null });
db.update(schema.controllers)
  .set({
    tokenHash: randomUUID(),
    pairingCodeHash: null,
    pairingExpiresAt: null,
    disabled: true,
    note: "Kept for its history",
  })
  .where(eq(schema.controllers.id, retired.controller.id))
  .run();

// A week of samples a minute apart (every fifth minute here), with an evening peak
const samples = [
  ["local", 18],
  [frontier.controller.id, 30],
  [aurora.controller.id, 12],
  [quiet.controller.id, 8],
] as const;
for (const [controllerId, peak] of samples) {
  const rows = [];
  for (
    let at = Math.floor((now - 7 * DAY) / 60_000) * 60_000;
    at < now - 5 * 60_000;
    at += 5 * 60_000
  ) {
    if (controllerId === quiet.controller.id && at > now - 5 * HOUR) break;
    const hour = new Date(at).getHours();
    const curve = Math.max(0, Math.sin(((hour - 8) / 24) * Math.PI * 2));
    const online = Math.round(peak * (0.25 + 0.75 * curve) * (0.85 + Math.random() * 0.3));
    rows.push({
      controllerId,
      at,
      online,
      huntsRunning: Math.min(8, Math.floor(online / 4)),
      huntsMax: 8,
    });
  }
  for (let i = 0; i < rows.length; i += 500)
    db.insert(schema.controllerSamples)
      .values(rows.slice(i, i + 500))
      .onConflictDoNothing()
      .run();
}

// Three days of chat on this host and Frontier, the filter rating each line
const people = (seed: number) => Array.from({ length: 10 }, (_, i) => demoPlayer(seed, i));
const PEOPLE = people(1);
const FRONTIER = people(2);
const AURORA = people(3);
const LINES = [
  "anyone up for a heroic shrike?",
  "lfg valomyr, need a healer",
  "gg everyone",
  "that was close",
  "where do I get more aether cells?",
  "the new lantern is so good",
  "who wants to run trials",
  "brb getting coffee",
  "my hammer build is finally done",
  "thanks for the carry!",
  "ramsgate is busy tonight",
  "darn, missed the stagger",
  "lol noob move on my part",
  "anyone seen the escalation boss?",
  "selling gold cheap, message me",
  "join us at discord.gg/dauntless-fun",
  "you're such a cheeter",
  "buy aether at my site",
  "what a darn good hunt",
  "this noob keeps dying",
];
const chat: IncomingChat[] = [];
for (let i = 0; i < 420; i++) {
  // Every fourth line is Frontier's, said by Frontier's players
  const who = i % 4 === 0 ? FRONTIER : PEOPLE;
  const [userId, name] = who[i % who.length]!;
  const direct = i % 13 === 0;
  const [rid, rname] = who[(i + 3) % who.length]!;
  chat.push({
    id: `seed-${i}`,
    at: now - Math.floor((420 - i) * ((3 * DAY) / 420)),
    channel: direct ? "direct" : (["city", "party", "guild"] as const)[i % 3]!,
    roomId: direct ? null : `room-${i % 3}`,
    sender: { userId, name },
    recipient: direct ? { userId: rid, name: rname } : null,
    body: LINES[(i * 7 + Math.floor(i / 10)) % LINES.length]!,
  });
}
ingestChat(chat.filter((_, i) => i % 4 !== 0));
ingestChat(
  chat.filter((_, i) => i % 4 === 0).map((m) => ({ ...m, id: `${m.id}-f` })),
  frontier.controller.id,
);

// Reports in every state, from the game and from staff
const report = (
  target: [string, string],
  reporter: [string, string] | null,
  category: Parameters<typeof createReport>[1]["category"],
  description: string,
  controllerId?: string,
  source: "game" | "dashboard" = "game",
) =>
  createReport(source === "game" ? { ingest: "backend" } : rook, {
    controllerId,
    source,
    externalId: source === "game" ? randomUUID() : null,
    reporter: reporter ? { userId: reporter[0], name: reporter[1] } : null,
    target: { userId: target[0], name: target[1] },
    category,
    description,
  }).report;
const p = PEOPLE;
report(
  p[3]!,
  p[0]!,
  "cheating",
  "Took no damage for the whole Shrike hunt, and the timer read 40 seconds.",
);
report(
  p[7]!,
  p[2]!,
  "harassment",
  "Kept following me around Ramsgate calling me names after I left their party.",
);
report(
  FRONTIER[6]!,
  null,
  "real_money_trading",
  "Advertising gold for real money in city chat.",
  frontier.controller.id,
);
const spam = report(p[9]!, p[4]!, "spam", "Posting the same Discord link every minute.");
claimReport(ash, spam.id);
addNote(ash, spam.id, "Seen it in chat, four times in five minutes. Talking to them.");
const griefing = report(
  p[1]!,
  p[8]!,
  "griefing",
  "Kept knocking the Behemoth off the edge so we failed the hunt.",
);
claimReport(juniper, griefing.id);
addNote(juniper, griefing.id, "Replay shows it was the map, not them.");
closeReport(juniper, griefing.id, "dismissed", "The map's edge, not the player.");
const hate = report(
  p[5]!,
  p[3]!,
  "hate_speech",
  "Slurs in party chat, screenshot attached in Discord.",
);
claimReport(wren, hate.id);
addNote(wren, hate.id, "Confirmed in the chat log.");
closeReport(wren, hate.id, "resolved", "Seven-day timeout and a warning.");
report(p[2]!, null, "inappropriate_name", "Name is a slur in Portuguese.", undefined, "dashboard");
report(AURORA[8]!, AURORA[1]!, "other", "Asked for my account details.", aurora.controller.id);

// Everything above happened in a fraction of a second: spread it over the last six days, in its order,
// so the audit log and reports read as a week's work. Chat and samples already carry their own times.
const seeded = Date.now();
const scale = (6 * DAY) / Math.max(1, seeded - now);
const tables = db.all<{ name: string }>(
  sql.raw("SELECT name FROM sqlite_master WHERE type = 'table'"),
);
for (const { name } of tables) {
  if (["chat_messages", "controller_samples"].includes(name) || name.startsWith("__")) continue;
  for (const { name: column } of db.all<{ name: string }>(
    sql.raw(`PRAGMA table_info("${name}")`),
  )) {
    if (!column.endsWith("_at")) continue;
    db.run(
      sql.raw(
        `UPDATE "${name}" SET "${column}" = ${now - 6 * DAY} + ("${column}" - ${now}) * ${scale}
         WHERE "${column}" BETWEEN ${now} AND ${seeded}`,
      ),
    );
  }
}

// The codes the runner hands its agents, on stdout's last line
console.log(JSON.stringify({ frontier: frontier.code, aurora: aurora.code }));
