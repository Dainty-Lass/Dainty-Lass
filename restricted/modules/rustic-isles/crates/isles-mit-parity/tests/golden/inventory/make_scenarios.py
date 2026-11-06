"""Writes scenarios.json: the seed and the requests both the Node recorder (record.cjs) and the Rust golden test run.

    python3 make_scenarios.py
"""
import json


def gs(user=None):
    auth = {"IsGameserver": True}
    if user:
        auth["userId"] = user
    return auth


def pl(user):
    return {"userId": user}


S = []


def add(name, method, path, auth=None, body=None, raw=None, ctype="application/json"):
    s = {"name": name, "method": method, "path": path}
    if auth is not None:
        s["auth"] = auth
    if body is not None:
        s["body"] = body
    if raw is not None:
        s["raw"] = raw
    if ctype != "application/json":
        s["contentType"] = ctype
    S.append(s)


seed = {
    "users": [
        {
            "userId": "u1",
            "wallet": {"CURRENCY_NOTES": 5000, "CURRENCY_PJM_WEAPON": 50},
            "characters": [
                {
                    "characterId": "c1",
                    "instanced": [
                        {
                            "catalogId": "WP_AX_BEGINNER",
                            "instanceId": "I-AXE",
                            "itemData": "{\"CurrentLevel\":5,\"DataVersion\":1}",
                            "updateVersion": 3,
                        },
                        {"catalogId": "LT_TRIALS_00", "instanceId": "I-LANTERN", "itemData": None, "updateVersion": 0},
                    ],
                    "stacked": [
                        {"catalogId": "MAT_A", "quantity": 4},
                        {"catalogId": "CORE", "quantity": 1},
                        {"catalogId": "CURRENCY_NOTES", "quantity": 1},
                    ],
                }
            ],
            "tracks": {"ExperienceTrack_Weapon_Axe": 31275, "ExperienceTrack_Weapon_Sword": 100},
        },
        {
            "userId": "u2",
            "wallet": {"CURRENCY_NOTES": 10},
            "characters": [{"characterId": "c2", "instanced": [], "stacked": []}],
        },
    ]
}

# the migration stubs
add("migration stub, named", "POST", "/inventory/c1/cl1", gs())
add("migration stub, empty characterId (regex)", "POST", "/inventory//dauntlessrel-1.14.7:647472", gs())
add("migration stub, unauthenticated", "POST", "/inventory/c1/cl1")

# reads
add("read, gameserver", "GET", "/inventory/u1/c1", gs())
add("read, player", "GET", "/inventory/zzz/c1", pl("u1"))
add("read, not the character of that user", "GET", "/inventory/u1/c2", gs())
add("read, unknown character", "GET", "/inventory/u1/nope", gs())
add("read, sentinel", "GET", "/inventory/INVALID/c1", gs())
add("read, no characterId resolves to the first character", "GET", "/inventory/u1/", gs())
add("read, no characterId, sentinel", "GET", "/inventory/INVALID/", gs())
add("read, no characterId, player", "GET", "/inventory/u1/", pl("u1"))
add("read, unauthenticated", "GET", "/inventory/u1/c1")

# transactions
tx1 = {
    "characterId": "c1",
    "transactionId": "tx-1",
    "addStackedItems": [
        {"catalogId": "MAT_A", "quantity": 3},
        {"catalogId": "CURRENCY_NOTES", "quantity": 100},
        {"catalogId": "NEWSTACK", "quantity": 2, "source": "x"},
    ],
}
add("tx, grant stack and currency", "POST", "/inventory", gs("u1"), tx1)
add("tx, replay of the same body", "POST", "/inventory", gs("u1"), tx1)
add(
    "tx, replay with the keys in another order",
    "POST",
    "/inventory",
    gs("u1"),
    {
        "transactionId": "tx-1",
        "addStackedItems": [
            {"quantity": 3, "catalogId": "MAT_A"},
            {"quantity": 100, "catalogId": "CURRENCY_NOTES"},
            {"source": "x", "quantity": 2, "catalogId": "NEWSTACK"},
        ],
        "characterId": "c1",
    },
)
add(
    "tx, same id another body",
    "POST",
    "/inventory",
    gs("u1"),
    {"characterId": "c1", "transactionId": "tx-1", "addStackedItems": [{"catalogId": "MAT_A", "quantity": 4}]},
)
add(
    "tx, remove more than owned",
    "POST",
    "/inventory",
    gs("u1"),
    {
        "characterId": "c1",
        "transactionId": "tx-2",
        "removeStackedItems": [{"catalogId": "CORE", "quantity": 5}],
        "addStackedItems": [{"catalogId": "CORE", "quantity": 9}],
    },
)
add(
    "tx, remove a whole stack and report it",
    "POST",
    "/inventory",
    gs("u1"),
    {"characterId": "c1", "transactionId": "tx-3", "removeStackedItems": [{"catalogId": "CORE", "quantity": 1}]},
)
add(
    "tx, overspend currency",
    "POST",
    "/inventory",
    gs("u1"),
    {
        "characterId": "c1",
        "transactionId": "tx-4",
        "removeStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 999999}],
    },
)
add(
    "tx, spend currency",
    "POST",
    "/inventory",
    gs("u1"),
    {
        "characterId": "c1",
        "transactionId": "tx-5",
        "removeStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 500}, {"catalogId": "MAT_A", "quantity": 2}],
    },
)
add(
    "tx, instanced adds with placeholder ids",
    "POST",
    "/inventory",
    gs("u1"),
    {
        "characterId": "c1",
        "transactionId": "tx-6",
        "addInstancedItems": [
            {"catalogId": "TOKEN_CELL_EXCHANGE", "instanceId": "TOKEN_CELL_EXCHANGE"},
            {"catalogId": "WP_SW_BEGINNER", "instanceId": "I-SWORD", "itemData": None, "updateVersion": 0},
        ],
        "removeInstancedItems": [{"catalogId": "LT_TRIALS_00", "instanceId": "I-LANTERN"}],
    },
)
add(
    "tx, save with a newer version",
    "POST",
    "/inventory",
    gs("u1"),
    {
        "characterId": "c1",
        "transactionId": "tx-7",
        "saveInstancedItems": [
            {"catalogId": "WP_AX_BEGINNER", "instanceId": "I-AXE", "itemData": "{\"CurrentLevel\":6}", "updateVersion": 4}
        ],
    },
)
add(
    "tx, save with a stale version",
    "POST",
    "/inventory",
    gs("u1"),
    {
        "characterId": "c1",
        "transactionId": "tx-8",
        "saveInstancedItems": [
            {"catalogId": "WP_AX_BEGINNER", "instanceId": "I-AXE", "itemData": "old", "updateVersion": 1}
        ],
    },
)
add(
    "tx, save that would change the catalogId",
    "POST",
    "/inventory",
    gs("u1"),
    {
        "characterId": "c1",
        "transactionId": "tx-9",
        "saveInstancedItems": [
            {"catalogId": "TOKEN_CELL_EXCHANGE", "instanceId": "I-AXE", "itemData": "x", "updateVersion": 9}
        ],
    },
)
add(
    "tx, storage correction both ways",
    "POST",
    "/inventory",
    gs("u1"),
    {
        "characterId": "c1",
        "transactionId": "tx-10",
        "addInstancedItems": [{"catalogId": "WP_AC_TRIALS_00", "instanceId": "BAD"}],
        "addStackedItems": [{"catalogId": "LT_TRIALS_00", "quantity": 2}],
    },
)
add(
    "tx, impossible quantity for a non-stackable",
    "POST",
    "/inventory",
    gs("u1"),
    {
        "characterId": "c1",
        "transactionId": "tx-11",
        "addStackedItems": [{"catalogId": "LT_TRIALS_00", "quantity": 0}],
    },
)
add(
    "tx, starter dye ids are not stored",
    "POST",
    "/inventory",
    gs("u1"),
    {
        "characterId": "c1",
        "transactionId": "tx-12",
        "addStackedItems": [
            {"catalogId": "DYE_BANNER_SIGIL_DEFAULT", "quantity": 1},
            {"catalogId": "MAT_A", "quantity": 0},
        ],
    },
)
add(
    "tx, player bearer cannot grant",
    "POST",
    "/inventory",
    pl("u1"),
    {"characterId": "c1", "transactionId": "tx-13", "addStackedItems": [{"catalogId": "MAT_A", "quantity": 1}]},
)
add(
    "tx, player bearer can save",
    "POST",
    "/inventory",
    pl("u1"),
    {
        "characterId": "c1",
        "transactionId": "tx-14",
        "saveInstancedItems": [
            {"catalogId": "WP_AX_BEGINNER", "instanceId": "I-AXE", "itemData": "{\"CurrentLevel\":7}", "updateVersion": 5}
        ],
    },
)
add(
    "tx, not the character of that user",
    "POST",
    "/inventory",
    gs("u1"),
    {"characterId": "c2", "transactionId": "tx-15", "addStackedItems": [{"catalogId": "MAT_A", "quantity": 1}]},
)
add(
    "tx, accountId in the body names the user",
    "POST",
    "/inventory",
    gs(),
    {
        "accountId": "u2",
        "characterId": "c2",
        "transactionId": "tx-16",
        "addStackedItems": [{"catalogId": "MAT_A", "quantity": 5}],
    },
)
add(
    "tx, empty characterId resolves to the first character",
    "POST",
    "/inventory",
    gs("u1"),
    {"characterId": "", "transactionId": "tx-17", "addStackedItems": [{"catalogId": "MAT_A", "quantity": 1}]},
)
add(
    "tx, empty accountId and characterId from a bearer-less gameserver",
    "POST",
    "/inventory",
    gs(),
    {
        "accountId": "",
        "characterId": "",
        "transactionId": "tx-18",
        "addStackedItems": [{"catalogId": "MAT_A", "quantity": 1}],
    },
)
add(
    "tx, a gameserver's sentinel account becomes the configured one",
    "POST",
    "/inventory",
    gs(),
    {
        "accountId": "INVALID",
        "characterId": "c1",
        "transactionId": "tx-19",
        "addStackedItems": [{"catalogId": "MAT_A", "quantity": 1}],
        "removeInstancedItems": "odd",
    },
)
add(
    "tx, sentinel user is a no-op echo",
    "POST",
    "/inventory",
    pl("INVALID"),
    {
        "characterId": "c1",
        "transactionId": "tx-19",
        "addStackedItems": [{"catalogId": "MAT_A", "quantity": 1}],
        "saveInstancedItems": [{"x": 1}],
        "removeInstancedItems": "odd",
    },
)
add("tx, body is an array", "POST", "/inventory", gs("u1"), raw="[1]")
add("tx, body is not JSON", "POST", "/inventory", gs("u1"), raw="{nope")
add("tx, no body", "POST", "/inventory", gs("u1"), raw="")
add("tx, missing transactionId", "POST", "/inventory", gs("u1"), {"characterId": "c1"})
add(
    "tx, bad quantity",
    "POST",
    "/inventory",
    gs("u1"),
    {"characterId": "c1", "transactionId": "tx-20", "addStackedItems": [{"catalogId": "MAT_A", "quantity": 1.5}]},
)
add(
    "tx, instanced entry without an instanceId",
    "POST",
    "/inventory",
    gs("u1"),
    {"characterId": "c1", "transactionId": "tx-21", "saveInstancedItems": [{"catalogId": "WP_AX_BEGINNER"}]},
)
add("tx, unauthenticated", "POST", "/inventory", None, {"characterId": "c1", "transactionId": "tx-22"})
add(
    "tx, form-encoded body",
    "POST",
    "/inventory",
    gs("u1"),
    raw="characterId=c1&transactionId=tx-23",
    ctype="application/x-www-form-urlencoded",
)
add("read after the transactions", "GET", "/inventory/u1/c1", gs())
add("read, the second user", "GET", "/inventory/u2/c2", gs())

# one item
axe = {"characterId": "c1", "instanceId": "I-AXE", "catalogId": "WP_AX_BEGINNER"}
add("item, newer version", "POST", "/inventory/instanceditem", gs("u1"), dict(axe, itemData="{\"CurrentLevel\":9}", updateVersion=8))
add(
    "item, stale version answers the stored item",
    "POST",
    "/inventory/instanceditem",
    gs("u1"),
    dict(axe, itemData="{\"CurrentLevel\":1}", updateVersion=2),
)
add("item, no version", "POST", "/inventory/instanceditem", pl("u1"), dict(axe, itemData="{\"CurrentLevel\":2}"))
add(
    "item, unknown instance",
    "POST",
    "/inventory/instanceditem",
    gs("u1"),
    {"characterId": "c1", "instanceId": "nope", "catalogId": "WP_AX_BEGINNER", "itemData": "x", "updateVersion": 1},
)
add(
    "item, catalogId change refused",
    "POST",
    "/inventory/instanceditem",
    gs("u1"),
    {"characterId": "c1", "instanceId": "I-AXE", "catalogId": "TOKEN_CELL_EXCHANGE", "itemData": "x", "updateVersion": 99},
)
add(
    "item, not the character of that user",
    "POST",
    "/inventory/instanceditem",
    gs("u1"),
    dict(axe, characterId="c2", itemData="x", updateVersion=99),
)
add(
    "item, sentinel echo",
    "POST",
    "/inventory/instanceditem",
    gs(),
    {"accountId": "INVALID", "characterId": "c1", "instanceId": "i", "catalogId": "X", "itemData": None},
)
add(
    "item, sentinel user echo",
    "POST",
    "/inventory/instanceditem",
    pl("INVALID"),
    {"characterId": "c1", "instanceId": "i", "catalogId": "X", "updateVersion": 3},
)
add("item, body is an array", "POST", "/inventory/instanceditem", gs("u1"), raw="[]")
add(
    "item, invalid body",
    "POST",
    "/inventory/instanceditem",
    gs("u1"),
    {"characterId": "c1", "instanceId": "", "catalogId": "X"},
)
add("item, invalid version", "POST", "/inventory/instanceditem", gs("u1"), dict(axe, updateVersion=-1))
add("item, unauthenticated", "POST", "/inventory/instanceditem", None, {"characterId": "c1"})

# reforge
AX = "ExperienceTrack_Weapon_Axe"
add("reforge, reset by a player is forbidden", "DELETE", "/progression/u1/" + AX, pl("u1"))
add("reforge, reset of a track that is not a weapon's", "DELETE", "/progression/u1/ExperienceTrack_PlayerLevel", gs())
add("reforge, reset of the sentinel", "DELETE", "/progression/INVALID/" + AX, gs())
add("reforge, reset below the cap", "DELETE", "/progression/u1/ExperienceTrack_Weapon_Sword", gs())
add("reforge, reset of an unknown user", "DELETE", "/progression/nobody/" + AX, gs())
add("reforge, purchase before any reset", "POST", "/progression/purchase/u1/%s/3500" % AX, gs())
add("reforge, reset", "DELETE", "/progression/u1/" + AX, gs())
add("reforge, a duplicate reset replays", "DELETE", "/progression/u1/" + AX, gs())
add("reforge, purchase with the wrong amount", "POST", "/progression/purchase/u1/%s/3499" % AX, gs())
add("reforge, purchase by a player is forbidden", "POST", "/progression/purchase/u1/%s/3500" % AX, pl("u1"))
add("reforge, purchase with a bad amount", "POST", "/progression/purchase/u1/%s/0" % AX, gs())
add("reforge, purchase with a fractional amount", "POST", "/progression/purchase/u1/%s/1.5" % AX, gs())
add("reforge, purchase over the bound", "POST", "/progression/purchase/u1/%s/100001" % AX, gs())
add("reforge, purchase for the sentinel", "POST", "/progression/purchase/INVALID/%s/3500" % AX, gs())
add("reforge, purchase restores the banked XP", "POST", "/progression/purchase/u1/%s/3500" % AX, gs())
add("reforge, purchase again replays", "POST", "/progression/purchase/u1/%s/3500" % AX, gs())
add("reforge, purchase of a track that is not a weapon's", "POST", "/progression/purchase/u1/ExperienceTrack_PlayerLevel/10", gs())
add("reforge, unauthenticated", "DELETE", "/progression/u1/" + AX)

with open("scenarios.json", "w", encoding="utf-8") as out:
    json.dump({"seed": seed, "scenarios": S}, out, indent=1)
    out.write("\n")
print(len(S), "scenarios")
