# RU10: one runtime tree for 1.12.0 and 1.14.7?

**Verdict: no. The shared part is 31.7% of the code without the rustic-freeza patches and 30.2% with them,
under the kill criterion's half. The two `ParadoxRuntime` trees stay apart** (DD8).

Analysis only, no build: there is no SDK here. Trees compared: `dauntless-1.12.0` (4d5dfaa) and `main` (ce1c42e),
the latter with and without Reforged's patches 0004–0039 (DD13, DD25).

## Numbers

Code lines: comments, blank lines and brace-only lines are not counted. The 1.12.0 tag ships with its comments
stripped, so raw line counts would overstate the difference. `MinHook/` is byte-identical in both trees and left
out. "Glue" means a line that differs only by an RVA or hex offset, a UE type name (`U*`, `A*`, `F*`, `E*`),
an `SDK::`/`Params::` name, a `_C` Blueprint name or a `/Game/` path.

| | 1.12.0 | 1.14.7 (Mystic + 0001) | 1.14.7 (+ 0004–0039) |
| --- | ---: | ---: | ---: |
| Code lines | 5,839 | 15,724 | 16,552 |
| Identical to the other tree | 5,082 | 5,082 | 5,081 |
| Differs by SDK glue only | 109 | 109 | 109 |
| Differs in logic, or in one tree only | 648 | 10,533 | 11,362 |
| **Shared / union** | | **31.7%** (5,191 / 16,372) | **30.2%** (5,190 / 17,201) |
| Shared / this tree | 88.9% | 33.0% | 31.4% |

Per top-level unit (function, struct, global; matched by name across files), 1.12.0 against Mystic's 1.14.7:

| Class | Units | 1.12.0 lines | 1.14.7 lines |
| --- | ---: | ---: | ---: |
| Identical | 358 | 1,550 | 1,550 |
| SDK glue only | 11 | 263 | 263 |
| Logic differs | 103 | 3,895 | 5,390 |
| 1.12.0 only | 17 | 131 | – |
| 1.14.7 only | 1,072 | – | 8,521 |

Per file: `dllmain.cpp` is 31% shared, `Networking.cpp` 74%, `Networking.h` 27%, and `constants.h` and
`framework.h` 100%. The files that are new in 1.14.7 (`p2p/`, `BackendUrl.h`, `P2PHooks*`, `RuntimeShared.h`,
`resource.h`) share nothing.

What the result shows: 1.14.7's runtime is not a sibling of 1.12.0's but a superset of it. 89% of 1.12.0
survives in 1.14.7, and the tree is 2.7 times larger. Glue-only differences are rare (109 lines). Where the two
trees differ, the code was rewritten (`ReplicateActorFreqHook`, `ProcessEventHook`, `InitServerHooks`). Many
1.14.7 changes are version-driven logic, not a different constant. For example `InitServerHooks` drops the
knockout and unhandled-exception hooks ("1.14.7 native target unresolved") and fences off 1.12 RVAs with
`if (false)`. A table of addresses can't express changes like these.

### Where 1.14.7's lines come from

`git blame -w` on ce1c42e, code lines only:

| Origin | Code lines | Of which shared with 1.12.0 |
| --- | ---: | ---: |
| Before the tag (1.12.0, unchanged) | 4,886 | 4,863 |
| dc50ec5, Mystic's port to 1.14.7 (also adds two months of development the tag lacked) | 6,601 | 320 |
| c7869b6, player host and realm hub | 4,031 | 5 |
| d0661bd, 2a6b567 (empty-hunt reap, realm hub fix) | 187 | 0 |
| Reforged 0001 (backend URL, stays) | 17 | 0 |
| **Reforged 0004–0039 (moves to rustic-freeza)** | **830** (5.0%) | 2 |

The freeza patches are 1,330 raw lines (18 commits, all in `dllmain.cpp`), and almost all of it is new
functions. Taking them out moves the fraction by only 1.5 points.

Sensitivity:
- Dropping everything else that 1.12.0 doesn't need as well (c7869b6, d0661bd, 2a6b567) gives 42.7%
  (5,190 / 12,160). That is still under half.
- Masking every decimal literal as glue too (`--loose`) adds 2 lines.

### What is tied to a version

These lines need a per-version value: a raw RVA (`BaseAddress + 0x…`, `*Rva`), a field offset (`+ 0x18`) or a
vtable slot.

| | 1.12.0 | 1.14.7 (Mystic) |
| --- | ---: | ---: |
| Lines carrying an RVA or offset | 648 (11%) | 1,173 (7%) |
| Lines in units that touch one | 3,933 (67%) | 8,982 (57%) |
| `BaseAddress + 0x` sites / distinct RVAs | 160 / 66 | 204 / 84 |
| Large hex constants (≥ 6 digits) | 98 | 159 (23 in common, mostly exception codes) |
| `MH_CreateHook` calls | 42 | 81 |

The 1.14.7-only units break down as 3,982 lines tied to an RVA or offset, 1,275 that use SDK types only, and
3,264 with no SDK at all (logging, perf, the player-host pipe, backend URL parsing).

## Method (rerun)

From a clone of rustic-isles, in a scratch directory outside it:

```bash
R=/path/to/rustic-isles; S=$PWD; mkdir v112 v147 v147m
git -C $R archive dauntless-1.12.0 ParadoxRuntime | tar -x -C v112
git -C $R archive ce1c42e ParadoxRuntime | tar -x -C v147
# Mystic's 1.14.7 plus 0001 only = ce1c42e without 0004-0039 and the follow-up that moves two of their calls
git -C $R archive 21b9ff5 ParadoxRuntime | tar -x -C v147m
git -C $R diff 4948437^ 4948437 | (cd v147m && git apply)
: > blame.tsv
for f in $(git -C $R ls-tree -r --name-only ce1c42e -- ParadoxRuntime | grep -v MinHook | grep -E '\.(cpp|h|hpp)$'); do
  git -C $R blame -w --porcelain ce1c42e -- $f |
    awk -v F="${f#ParadoxRuntime/}" '/^[0-9a-f]{40} [0-9]+ [0-9]+/{print F"\t"$3"\t"substr($1,1,7)}' >> blame.tsv
done
diff -r v112/ParadoxRuntime/MinHook v147/ParadoxRuntime/MinHook   # empty
python3 ru10.py v112/ParadoxRuntime v147m/ParadoxRuntime             # without freeza
python3 ru10.py v112/ParadoxRuntime v147/ParadoxRuntime --blame blame.tsv --units units.tsv
```

`ru10.py` (below) works like this:
1. It strips comments and splits each source file into top-level units by brace matching (namespaces and
   `extern "C"` are transparent).
2. It matches units across the two trees by name: functions by name, globals and preprocessor lines by file
   and name.
3. It runs `difflib` LCS on the normalised lines of each matched pair. Lines that match are "identical". In a
   replaced hunk, lines that match once the glue tokens are masked are "glue". The rest is logic.
4. "Shared" is identical plus glue. The union is shared plus each side's remaining lines.

The splitter is a heuristic, not a C++ parser. Spot checks of the largest units, and of the 11 glue-only and
17 1.12.0-only units, came out right. `--units` writes the per-unit table. `--blame` is valid only
against the ce1c42e tree, because its line numbers are that tree's.

## If the trees were folded anyway

There are two ways to fold, and neither pays:

- **Fold as is**, keeping each version's behaviour: the 103 diverged functions and 1,089 one-sided units go
  in `#if MP_GAME_1_12_0` blocks, which is 68% of the union. The result is two trees interleaved, with nothing
  gained.
- **1.14.7's tree for both** (re-port 1.12.0 onto it): one source behind a per-version glue layer, built with
  a switch, from the SDK of that version.
  - Layout: `rva/1_14_7.h` and `rva/1_12_0.h` (about 140 named RVAs and the 81 hook targets),
    `offsets/<ver>.h` (the field offsets and vtable slots from about 1,200 lines), `MP_GAME_VERSION` in the
    vcxproj, and `#if` around the version-driven logic above.
  - Cost:
    - (a) Refactoring 1.14.7 into the tables: one to two weeks of agent work. Each step needs a 1.14.7 SDK
      build and a hunt.
    - (b) The 1.12.0 values: the tag knows 66 RVAs, and about 30 comment lines in 1.14.7 pair a 1.12 RVA
      with its 1.14.7 counterpart. The rest have to be found on a 1.12.0 binary, and offsets come from a diff of the two
      Dumper-7 SDKs ("class layouts moved even where the engine version did not change",
      `docs/DAUNTLESS_1_14_7_PORT.md`). This is hands-on maintainer work with the game.
    - (c) 1.12.0 would then run 1.14.7's logic: the replication-graph drive, channel quarantine, realm hubs
      and player host. That needs a full canary, and it replaces Mystic's 1.12.0 runtime, which DD8 doesn't
      ask for.
    - (d) Freeza's 18 patches stop applying with `git apply --3way` until RZ2 rebases them.
  - In total: weeks of agent and maintainer time. Keeping two trees costs nothing, since 1.12.0's tree is
    frozen at the tag.

Revisit only if 1.12.0 has to gain 1.14.7's features, and do it as a port, not a fold.

<details><summary><code>ru10.py</code></summary>

```python
#!/usr/bin/env python3
"""RU10: compare two ParadoxRuntime trees per top-level unit (function, struct, global).

usage: ru10.py OLD_DIR NEW_DIR [--blame BLAME_TSV] [--units OUT_TSV]
Counts code lines: comments, blank lines and brace/semicolon-only lines are dropped.
"""
import os, re, sys, difflib, collections

SRC_EXT = ('.cpp', '.h', '.hpp')
SKIP_DIRS = {'MinHook'}  # vendored, byte-identical in both trees (checked separately)


def strip_comments(t):
    out, i, n = [], 0, len(t)
    while i < n:
        c = t[i]
        if t.startswith('//', i):
            j = t.find('\n', i)
            j = n if j < 0 else j
            out.append(' ' * (j - i)); i = j
        elif t.startswith('/*', i):
            j = t.find('*/', i + 2)
            j = n if j < 0 else j + 2
            out.append(''.join('\n' if ch == '\n' else ' ' for ch in t[i:j])); i = j
        elif c == 'R' and t.startswith('R"', i) and (i == 0 or not (t[i-1].isalnum() or t[i-1] == '_')):
            m = re.match(r'R"([^(\s]*)\(', t[i:])
            if m:
                end = t.find(')' + m.group(1) + '"', i)
                end = n if end < 0 else end + len(m.group(1)) + 2
                out.append(t[i:end]); i = end
            else:
                out.append(c); i += 1
        elif c in '"\'':
            j = i + 1
            while j < n and t[j] != c and t[j] != '\n':
                j += 2 if t[j] == '\\' else 1
            out.append(t[i:j + 1]); i = j + 1
        else:
            out.append(c); i += 1
    return ''.join(out)


def mask_literals(t):
    """Blank string/char literal contents so brace counting ignores them."""
    return re.sub(r'"(?:\\.|[^"\\\n])*"|\'(?:\\.|[^\'\\\n])*\'', lambda m: '"' + ' ' * (len(m.group(0)) - 2) + '"', t)


SKIP_CALL = {'__declspec', 'alignas', '__attribute__', 'decltype', 'if', 'while', 'for', 'switch', 'sizeof'}


def unit_name(header):
    h = ' '.join(header.split())
    h = re.sub(r'^template\s*<.*?>\s*', '', h)
    m = re.match(r'^(?:typedef\s+)?(struct|class|union|enum(?:\s+class)?)\s+(?:alignas\(\w+\)\s+)?(\w+)', h)
    if m:
        return m.group(1).split()[0] + ' ' + m.group(2)
    if re.match(r'^(?:typedef\s+)?(struct|union|enum)\s*$', h):
        return 'anon ' + h
    for m in re.finditer(r'([~\w:]+(?:operator\s*\S+?)?)\s*\(', h):
        nm = m.group(1).split('::')[-1] if m.group(1) not in SKIP_CALL else None
        if nm and nm not in SKIP_CALL and not re.fullmatch(r'\d+', nm):
            if '=' in h[:m.start()] and not h[:m.start()].rstrip().endswith('operator'):
                break
            return m.group(1)
    m = re.search(r'([\w:]+)\s*(?:\[[^\]]*\])*\s*=\s*$', h.split('{')[0]) or re.search(r'([\w:]+)\s*(?:\[[^\]]*\])*\s*=', h)
    if m:
        return 'init ' + m.group(1)
    return 'block ' + h[:40]


def decl_name(stmt):
    s = ' '.join(stmt.split())
    if s.startswith('#'):
        m = re.match(r'#\s*(\w+)\s*(\S*)', s)
        return '#' + m.group(1) + ' ' + m.group(2) if m else s
    m = re.search(r'([\w:]+)\s*(?:\[[^\]]*\])*\s*(?:=|;|\(|\{)', s)
    return 'decl ' + (m.group(1) if m else s[:40])


def split_units(path):
    raw = open(path, encoding='utf-8', errors='replace').read().replace('\r\n', '\n')
    code = strip_comments(raw)
    scan = mask_literals(code)
    lines = code.split('\n')
    units = []  # (name, kind, [lines])
    i, n = 0, len(scan)
    start = 0       # start offset of the current pending header/decl
    ns_stack = []   # transparent scopes: True for namespace/extern "C"
    paren = 0
    while i < n:
        c = scan[i]
        if c == '\n' and scan[start:i].lstrip().startswith('#') and not scan[start:i].rstrip().endswith('\\'):
            stmt = code[start:i]
            if stmt.strip():
                units.append((decl_name(stmt), 'pp', stmt, code.count(chr(10), 0, start) + 1))
            start = i + 1
        elif c == '(':
            paren += 1
        elif c == ')':
            paren -= 1
        elif c == ';' and paren == 0:
            stmt = code[start:i + 1]
            if stmt.strip():
                units.append((decl_name(stmt), 'decl', stmt, code.count(chr(10), 0, start) + 1))
            start = i + 1
        elif c == '{' and paren == 0:
            header = code[start:i]
            if re.search(r'(?:^|\s)(namespace(\s+[\w:]+)?|extern\s+"C")\s*$', header):
                ns_stack.append(True)
                start = i + 1
            else:
                depth, j = 0, i
                while j < n:
                    if scan[j] == '{':
                        depth += 1
                    elif scan[j] == '}':
                        depth -= 1
                        if depth == 0:
                            break
                    j += 1
                k = j + 1
                m = re.match(r'[ \t\w,*&\[\]]*;', scan[k:k + 200])
                if m:
                    k += m.end()
                units.append((unit_name(header), 'body', code[start:k], code.count(chr(10), 0, start) + 1))
                i = k
                start = k
                continue
        elif c == '}' and paren == 0 and ns_stack:
            ns_stack.pop()
            start = i + 1
        i += 1
    if code[start:].strip():
        units.append(('tail', 'decl', code[start:], code.count(chr(10), 0, start) + 1))
    return units


TRIVIAL = re.compile(r'^[{}();,\s]*$|^(else|do|default:|break;|return;|continue;|public:|private:|protected:)$')


def norm_lines(text, first=1):
    out, nums = [], []
    for off, ln in enumerate(text.split('\n')):
        s = ' '.join(ln.split())
        if s and not TRIVIAL.match(s):
            out.append(s)
            nums.append(first + off)
    return out, nums


HEX = re.compile(r'\b0x[0-9A-Fa-f]+\b[uUlL]*')
UE_TYPE = re.compile(r'\b(?:[UAFE][A-Z][A-Za-z0-9_]*[a-z][A-Za-z0-9_]*)\b')
PARAMS = re.compile(r'\b(?:Params|SDK)::\w+')
UE_STR = re.compile(r'"(?:/Game/|/Script/|Function |Default__)[^"]*"')
BP_IDENT = re.compile(r'\b\w+_C\b|\b\w+_[0-9A-F]{32}\b')
DEC = re.compile(r'\b\d+(?:\.\d+)?[fFuUlL]*\b')


def glue_mask(s, loose=False):
    s = UE_STR.sub('"#S"', s)
    s = PARAMS.sub('#P', s)
    s = HEX.sub('#H', s)
    s = BP_IDENT.sub('#B', s)
    s = UE_TYPE.sub('#T', s)
    if loose:
        s = DEC.sub('#N', s)
    return s


def collect(root):
    units = []
    for d, dirs, files in os.walk(root):
        dirs[:] = [x for x in dirs if x not in SKIP_DIRS]
        for f in sorted(files):
            if f.endswith(SRC_EXT) and not f.endswith('.example'):
                p = os.path.join(d, f)
                rel = os.path.relpath(p, root).replace('\\', '/')
                for name, kind, text, first in split_units(p):
                    ls, nums = norm_lines(text, first)
                    units.append({'file': rel, 'name': name, 'kind': kind, 'lines': ls, 'nums': nums})
    return units


def key_units(units):
    seen = collections.Counter()
    out = collections.OrderedDict()
    for u in units:
        if not u['lines']:
            continue
        base = u['name'] if u['kind'] == 'body' else u['file'] + ':' + u['name']
        seen[base] += 1
        k = base if seen[base] == 1 else f'{base}#{seen[base]}'
        out[k] = u
    return out


def compare(a_lines, b_lines, loose=False):
    """Return (identical, glue, a_only, b_only) line counts; glue counted once per matched pair."""
    sm = difflib.SequenceMatcher(None, a_lines, b_lines, autojunk=False)
    ident = sum(bl.size for bl in sm.get_matching_blocks())
    ident_b = set()
    for bl in sm.get_matching_blocks():
        ident_b.update(range(bl.b, bl.b + bl.size))
    glue = 0
    labels = ['rest'] * len(b_lines)
    for j in ident_b:
        labels[j] = 'ident'
    a_rest, b_rest = len(a_lines) - ident, len(b_lines) - ident
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == 'replace':
            ma = [glue_mask(x, loose) for x in a_lines[i1:i2]]
            mb = [glue_mask(x, loose) for x in b_lines[j1:j2]]
            gb = difflib.SequenceMatcher(None, ma, mb, autojunk=False).get_matching_blocks()
            g = sum(bl.size for bl in gb)
            for bl in gb:
                for j in range(bl.b, bl.b + bl.size):
                    labels[j1 + j] = 'glue'
            glue += g
            a_rest -= g
            b_rest -= g
    return ident, glue, a_rest, b_rest, labels


FREEZA = 'f8bcf60 feca68c d52417d 4d4980a ed3f444 2d41e84 cb019cd e5f2a02 9a95fa6 cccc237 bab5410 a63d454 d919559 4f31f00 7e808a9 19c63e8 c48d8d4 63c6f3c ce1c42e'.split()
GROUP = {'72e0ef1': 'pre-tag (1.12.0)', 'b143a2b': 'pre-tag (1.12.0)', 'dc50ec5': 'dc50ec5 port to 1.14.7',
         'c3a4755': 'c3a4755 license header', '55757b2': '55757b2 anti-cheat build',
         'c7869b6': 'c7869b6 player host/realm hub', 'd0661bd': 'd0661bd empty-hunt reap',
         '2a6b567': '2a6b567 realm hub fix', '4948437': 'Reforged 0001 backend URL'}
GROUP.update({s: 'Reforged 0004-0039 (freeza)' for s in FREEZA})


ADDR = re.compile(r'BaseAddress|Rva\b|RVA|\b0x0[0-9A-Fa-f]{6,}\b')
OFF = re.compile(r'\+\s*0x[0-9A-Fa-f]{1,4}\b|\boffsetof\b|\bIDX_\w+|vtable|VTable|Vtbl')
SDK_REF = re.compile(UE_TYPE.pattern + '|' + PARAMS.pattern)


def portability(lines):
    t = '\n'.join(lines)
    if ADDR.search(t) or OFF.search(t):
        return 'addr/offset'  # raw RVA, field offset or vtable slot: needs a per-version value
    return 'sdk' if SDK_REF.search(t) else 'plain'


def main():
    a_dir, b_dir = sys.argv[1], sys.argv[2]
    loose = '--loose' in sys.argv
    out_path = sys.argv[sys.argv.index('--units') + 1] if '--units' in sys.argv else None
    A, B = key_units(collect(a_dir)), key_units(collect(b_dir))
    blame = {}
    if '--blame' in sys.argv:
        for ln in open(sys.argv[sys.argv.index('--blame') + 1], encoding='utf-8'):
            f, n_, sha = ln.rstrip(chr(10)).split(chr(9))
            blame[f, int(n_)] = sha
    attr = collections.Counter()
    rows = []
    tot = collections.Counter()
    cls = collections.Counter()
    cls_lines = collections.Counter()
    for k in list(A) + [k for k in B if k not in A]:
        a, b = A.get(k), B.get(k)
        if a and b:
            i, g, ao, bo, labels = compare(a['lines'], b['lines'], loose)
            for j, lab in enumerate(labels):
                attr[lab, blame.get((b['file'], b['nums'][j]), '?')] += 1
            if ao == 0 and bo == 0:
                c = 'identical' if g == 0 else 'glue'
            else:
                c = 'logic'
            row = (k, a['file'], b['file'], c, i, g, ao, bo)
        elif a:
            c = 'only_old'
            row = (k, a['file'], '', c, 0, 0, len(a['lines']), 0)
        else:
            c = 'only_new'
            for n_ in b['nums']:
                attr['new_unit', blame.get((b['file'], n_), '?')] += 1
            row = (k, '', b['file'], c, 0, 0, 0, len(b['lines']))
        rows.append(row)
        cls[c] += 1
        cls_lines[c, 'old'] += len(a['lines']) if a else 0
        cls_lines[c, 'new'] += len(b['lines']) if b else 0
        tot['ident'] += row[4]; tot['glue'] += row[5]; tot['old_only'] += row[6]; tot['new_only'] += row[7]
    old_total = sum(len(u['lines']) for u in A.values())
    new_total = sum(len(u['lines']) for u in B.values())
    shared = tot['ident'] + tot['glue']
    union = shared + tot['old_only'] + tot['new_only']
    print(f'code lines: old={old_total} new={new_total}')
    print(f"lines: identical={tot['ident']} glue-only={tot['glue']} old-only/logic={tot['old_only']} new-only/logic={tot['new_only']}")
    print(f'shared={shared}  union={union}  shared/union={shared/union:.1%}  shared/old={shared/old_total:.1%}  shared/new={shared/new_total:.1%}')
    print('units:', dict(cls))
    for c in ('identical', 'glue', 'logic', 'only_old', 'only_new'):
        print(f"  {c:10s} units={cls[c]:5d} old_lines={cls_lines[c, 'old']:6d} new_lines={cls_lines[c, 'new']:6d}")
    if blame:
        groups = {}
        for (lab, sha), cnt in attr.items():
            groups.setdefault(GROUP.get(sha, sha), collections.Counter())[lab] += cnt
        print('new-side lines by origin (ident / glue / rest-in-matched / new-unit):')
        for g, c in sorted(groups.items(), key=lambda x: -sum(x[1].values())):
            print(f"  {g:28s} {c['ident']:6d} {c['glue']:5d} {c['rest']:6d} {c['new_unit']:6d}  total={sum(c.values())}")
    pc = collections.Counter()
    for k, b in B.items():
        if k not in A:
            pc[portability(b['lines'])] += len(b['lines'])
    print('new-only units by binding (lines):', dict(pc))
    for label, units in (('old', A), ('new', B)):
        tot_ = sum(len(u['lines']) for u in units.values())
        bound_units = sum(len(u['lines']) for u in units.values() if portability(u['lines']) == 'addr/offset')
        bound_lines = sum(1 for u in units.values() for l in u['lines'] if portability([l]) == 'addr/offset')
        print(f'{label} tree: lines in units touching RVAs/offsets={bound_units} ({bound_units/tot_:.0%}), '
              f'lines carrying one={bound_lines} ({bound_lines/tot_:.0%})')
    if out_path:
        with open(out_path, 'w', encoding='utf-8') as f:
            f.write('key\told_file\tnew_file\tclass\tidentical\tglue\told_rest\tnew_rest\n')
            for r in rows:
                f.write('\t'.join(map(str, r)) + '\n')


if __name__ == '__main__':
    main()
```

</details>
