"""Decoder of the `Career` entry (data version 11, 200000 bytes at file offset 0x50ceb) of a
Midnight Club: Los Angeles save (mc4.sav / decrypted PS3 RAGE.SAV).

    decode(sav: bytes, raw: bool = False) -> dict

The layout is the one written by the career manager's save routine sub_826BF7B0 (read back by
sub_826C0E08); see career.md for the table and the functions behind every field.
Offsets in the result are relative to the entry's data unless they are called `file_off`.

Run as a script: `python career.py <save> [--raw]` prints the dict as indented JSON.
`raw` adds the 8176-byte vehicle records of the (normally empty) pink slip slots as hex.
"""
import json
import os
import re
import struct
import sys

ENTRY_OFF = 0x50CEB
ENTRY_SIZE = 200000
ADDON_OFF = 150000          # sub_826BF7B0: the South Central block starts at 0x249F0
RACE_TABLE = r'D:\MCLA\extracted\cache\tune\race\0x0f1f4d8d'   # only used to name race hashes

DIFFICULTIES = ['Easy', 'Medium', 'Hard', 'Impossible']
STATES = ['INACTIVE', 'CHECKING_PREREQUISITES', 'READY_TO_START', 'CHECKING_OBJECTIVES',
          'READY_TO_COMPLETE', 'COMPLETE']                      # name table at 0x8282A100
# condition type (mcCondition +44, set by the constructors 0x826D0CD8 ..) -> (class, saved u32s)
CONDITIONS = {
    0: ('Experience', ['start_amount']),
    1: ('String', []),
    2: ('Mission', []),
    3: ('Race', []),
    4: ('Tally', ['count']),
    5: ('TallyRace', ['count']),
    6: ('TallyMission', ['count']),
    7: ('Stat', ['start_value', 'value']),
}
DISTRICTS = ['Hollywood', 'Beaches', 'Hills', 'Downtown', 'SouthCentral']
VEHICLE_CLASSES = ['Tuner', 'Muscle', 'Exotic', 'SportBike', 'Luxury', 'Cop', 'SUV', 'LowRider',
                   'Misc1']                                     # table at 0x827E9ABC
ABILITIES = ['Zone', 'Agro', 'Roar', 'Pulse', 'Grip']           # table at 0x827E922C
PERF_LEVELS = [c + str(i) for c in 'DCBA' for i in range(5)]    # table at 0x827E9AE0
PART_TYPES = ['Roof', 'Hood', 'RearBumper', 'TruckSwap', 'FrontBumper', 'Fenders', 'Skirts',
              'Spoiler', 'SpoilerBody', 'Headlight', 'Taillight', 'TaillightBody', 'FrontGrill',
              'Wheeliebar', 'Blower', 'Intercooler', 'BikeTail', 'BikeTank', 'BikeCowl',
              'DoorLeft', 'DoorRight', 'Exhaust', 'ExhaustSide', 'WideBody', 'SpoilerWide',
              'HoodWide', 'DoorLeftWide', 'DoorRightWide', 'TrunkWide', 'Interior', 'FrontSeat',
              'IntDoorLeft', 'IntDoorRight', 'Stereo', 'StereoLeft', 'StereoRight',
              'SteeringWheel', 'PostGauge']                     # table at 0x827E93E8


def joaat(s, h=0):
    """The game's string hash sub_821C9790: lower case, backslash -> slash, one-at-a-time."""
    for c in s:
        o = ord(c)
        if 65 <= o <= 90:
            o += 32
        elif o == 92:
            o = 47
        h = (h + o) & 0xFFFFFFFF
        h = (h + (h << 10)) & 0xFFFFFFFF
        h ^= h >> 6
    h = (h + (h << 3)) & 0xFFFFFFFF
    h ^= h >> 11
    return (h + (h << 15)) & 0xFFFFFFFF


def map_order(names, buckets=11):
    """Iteration order of the game's hash map (sub_82389998: bucket = hash % 11, new nodes go to
    the head of their chain) - the experience systems of two such maps are saved in this order
    without their names."""
    b = {}
    for n in names:
        b.setdefault(joaat(n) % buckets, []).insert(0, n)
    return [n for k in sorted(b) for n in b[k]]


def race_ids(path=RACE_TABLE):
    try:
        with open(path, encoding='latin1') as f:
            return re.findall(r'Id\s+(\S+)', f.read())
    except OSError:
        return []


class Reader:
    def __init__(self, d, o=0):
        self.d = d
        self.o = o

    def u32(self):
        v = struct.unpack_from('>I', self.d, self.o)[0]
        self.o += 4
        return v

    def i32(self):
        v = struct.unpack_from('>i', self.d, self.o)[0]
        self.o += 4
        return v

    def u8(self):
        v = self.d[self.o]
        self.o += 1
        return v

    def raw(self, n):
        v = self.d[self.o:self.o + n]
        self.o += n
        return v


def cstr(b):
    return b.split(b'\0')[0].decode('latin1')


def decode(sav, raw=False):
    d = sav[ENTRY_OFF:ENTRY_OFF + ENTRY_SIZE]
    if len(d) != ENTRY_SIZE:
        raise ValueError('save too short for the Career entry')
    r = Reader(d)
    out = {}
    unknown = []
    layout = []
    mark = [0]

    def section(name):
        layout.append({'off': mark[0], 'len': r.o - mark[0], 'what': name})
        mark[0] = r.o

    def unk(off, data, note):
        unknown.append({'off': off, 'len': len(data), 'hex': data.hex(), 'note': note})

    races = race_ids()
    race_by_hash = {joaat(x): x for x in races}

    # ---- header -------------------------------------------------------------------------
    flags = r.u32()
    version = r.u8()
    out['entry'] = {'file_off': ENTRY_OFF, 'size': ENTRY_SIZE,
                    'container_version': struct.unpack_from('>I', sav, 0)[0],
                    'layout_version': version}
    out['flags'] = {'raw': flags,
                    'career_startup_done': bool(flags & 1),
                    'police_unlocked': bool(flags & 4)}
    if flags & ~5:
        unk(0, d[0:4], 'flag bits other than 1 and 4')
    if version != 11:
        raise ValueError('Career layout version %d, expected 11' % version)
    section('flags u32, layout version u8')

    # ---- event completion counters --------------------------------------------------------
    n = r.u32()
    events = []
    for _ in range(n):
        name = cstr(r.raw(32))
        c = [r.u32() for _ in range(4)]
        events.append({'name': name, 'completions': dict(zip(DIFFICULTIES, c))})
    out['events'] = events
    section('events: count + %d x 48' % n)

    # ---- experience manager ---------------------------------------------------------------
    out['money'] = r.u32()

    def system():
        amount = r.u32()
        k = r.u32()
        th = []
        for _ in range(k):
            tid = cstr(r.raw(64))
            th.append({'id': tid, 'reached': bool(r.u8())})
        return {'amount': amount, 'thresholds': th}

    def systems(names_for):
        k = r.u32()
        names = names_for(k)
        res = {}
        for i in range(k):
            s = system()
            res[names[i] if i < len(names) else 'system_%d' % i] = s
        return res

    exp = {}
    exp['main'] = systems(lambda k: ['Progression', 'Unlocking'] if k == 2 else [])
    exp['classes'] = systems(lambda k: map_order(VEHICLE_CLASSES) if k == len(VEHICLE_CLASSES) else [])
    exp['abilities'] = systems(lambda k: map_order(ABILITIES) if k == len(ABILITIES) else [])
    out['experience'] = exp
    out['reputation'] = exp['main'].get('Progression', {}).get('amount')
    section('experience manager: money, 3 groups of systems')

    # ---- missions -----------------------------------------------------------------------
    def requirement():
        hidden = bool(r.u8())
        satisfied = bool(r.u8())
        k = r.u32()
        conds = []
        for _ in range(k):
            t = r.u32()
            if t not in CONDITIONS:
                raise ValueError('unknown condition type %d at %#x' % (t, r.o - 4))
            cname, fields = CONDITIONS[t]
            c = {'type': cname, 'met': bool(r.u8())}
            for f in fields:
                c[f] = r.u32()
            conds.append(c)
        return {'hidden': hidden, 'satisfied': satisfied, 'conditions': conds}

    def missions(addon):
        k = r.u32()
        res = []
        for _ in range(k):
            off = r.o
            nraw = r.raw(128)
            name = cstr(nraw)
            pad = nraw[len(name) + 1:]
            if pad.strip(b'\xcd\x00'):
                unk(off + len(name) + 1, pad, 'mission name padding of ' + name)
            idx, log_slot, done_slot, state = r.i32(), r.i32(), r.i32(), r.u32()
            m = {'name': name, 'off': off, 'index': idx,
                 'active_slot': None if log_slot < 0 else log_slot,
                 'completed_slot': None if done_slot < 0 else done_slot,
                 'state': STATES[state] if state < len(STATES) else state, 'state_id': state,
                 'addon': addon}
            m['prerequisites'] = [requirement() for _ in range(r.u32())]
            m['objectives'] = [requirement() for _ in range(r.u32())]
            res.append(m)
        return res

    out['missions'] = missions(False)
    section('missions: count + %d records' % len(out['missions']))

    # ---- cruising hookman slots -------------------------------------------------------------
    nd = r.u32()
    slots = [[bool(r.u8()) for _ in range(r.u32())] for _ in range(nd)]
    locked = [bool(r.u8()) for _ in range(nd)]
    out['cruising_slots'] = [{'district': DISTRICTS[i] if i < len(DISTRICTS) else i,
                              'district_locked': locked[i], 'slots_hidden': slots[i]}
                             for i in range(nd)]
    section('slot system: districts, slot flags, district locks')

    # ---- special hookmen visibility ---------------------------------------------------------
    n = r.u32()
    out['alt_hookmen'] = [{'uid': r.u32(), 'visible': bool(r.u8())} for _ in range(n)]
    section('alt hookman visibility map: count + %d x 5' % n)

    # ---- unlock flags ---------------------------------------------------------------------
    out['tournaments_won'] = dict(zip(DISTRICTS[:4], [bool(r.u8()) for _ in range(4)]))
    n = r.u32()
    lv = [bool(r.u8()) for _ in range(n)]
    out['performance_levels_unlocked'] = {(PERF_LEVELS[i] if i < len(PERF_LEVELS) else str(i)): v
                                          for i, v in enumerate(lv)}
    rows, cols = r.u32(), r.u32()
    unlocked, fresh = {}, {}
    for a in range(rows):
        cl = VEHICLE_CLASSES[a] if a < len(VEHICLE_CLASSES) else str(a)
        cell = r.raw(2 * cols)
        names = [PART_TYPES[c] if c < len(PART_TYPES) else str(c) for c in range(cols)]
        unlocked[cl] = [names[c] for c in range(cols) if cell[2 * c]]
        fresh[cl] = [names[c] for c in range(cols) if cell[2 * c + 1]]
    out['body_parts_unlocked'] = unlocked
    out['body_parts_new'] = fresh
    out['abilities_unlocked'] = bool(r.u8())
    section('tournament wins, performance levels, body part grids, abilities flag')

    hashes = [r.u32() for _ in range(250)]
    out['races_unlocked'] = [race_by_hash.get(h, '0x%08x' % h) for h in hashes if h]
    used = sum(1 for h in hashes if h)
    if any(hashes[used:]):
        unk(r.o - 1000 + 4 * used, d[r.o - 1000 + 4 * used:r.o], 'race hashes after the first zero')
    section('unlocked race hashes: 250 x u32')

    tt = [bool(r.u8()) for _ in range(5)]
    times = [r.u32() for _ in range(20)]
    out['time_trials'] = [{'district': DISTRICTS[i], 'unlocked': tt[i],
                           'target_ms': times[4 * i:4 * i + 4]} for i in range(5)]
    section('time trial unlock flags (5) and target times (20 x u32)')

    hoff = r.o
    ring = [r.u32() for _ in range(12)]
    tail, head, count = r.u32(), r.u32(), r.u32()
    recent = []
    if count <= 12 and head < 12:
        recent = [ring[(head + 1 + i) % 12] for i in range(count)]      # oldest first
    else:
        unk(hoff, d[hoff:hoff + 60], 'race history with an impossible head / count')
    out['race_history'] = {'oldest_first': [{'race_index': x,
                                             'race': races[x] if x < len(races) else None}
                                            for x in recent],
                           'write_index': tail, 'head': head, 'count': count}
    section('race history ring: 12 x u32, write index, head, count')

    # ---- pink slip profiles -----------------------------------------------------------------
    uid_counter = r.u32()
    ps = []
    for i in range(5):
        off = r.o
        s = r.raw(8272)
        kind = struct.unpack_from('>i', s, 0)[0]
        e = {'slot': i, 'off': off, 'type': kind, 'empty': kind == -1,
             'name': cstr(s[4:68]), 'uid': struct.unpack_from('>i', s, 8256)[0],
             'vehicle_record_off': off + 80, 'vehicle_record_len': 8176}
        if kind != -1 or raw:
            e['vehicle_record_hex'] = s[80:8256].hex()
        if kind != -1:
            e['vehicle_name'] = cstr(s[80 + 8109:80 + 8109 + 64])
        for a, b, what in ((68, 80, 'pink slip slot %d bytes 68..80' % i),
                           (8260, 8272, 'pink slip slot %d bytes 8260..8272' % i)):
            if s[a:b].strip(b'\xcd'):
                unk(off + a, s[a:b], what)
        ps.append(e)
    out['pink_slips'] = {'uid_counter': uid_counter, 'slots': ps}
    section('pink slip uid counter + 5 x 8272 profile slots')

    # ---- scripts ----------------------------------------------------------------------------
    def scripts():
        k = r.u32()
        res = []
        for _ in range(k):
            stack = r.u32()
            ln = r.u32()
            name = r.raw(ln).decode('latin1')
            size = r.u32()
            data = r.raw(size)
            e = {'name': name, 'stack_size': stack, 'saved_bytes': size}
            if size % 4 == 0:
                e['saved_words'] = list(struct.unpack('>%dI' % (size // 4), data))
            else:
                e['saved_hex'] = data.hex()
            res.append(e)
        return res

    def residue(end, limit, where):
        """Bytes after the last record that are not the 0xCD fill: left over in the save buffer
        from an earlier, longer save (the routine does not clear it)."""
        seg = d[end:limit]
        nz = [i for i, b in enumerate(seg) if b != 0xCD]
        if not nz:
            return None
        blob = seg[:nz[-1] + 1]
        res = {'off': end, 'len': len(blob), 'hex': blob.hex(), 'where': where}
        # (seen: one more script record of an earlier save)
        try:
            q = Reader(blob)
            recs = []
            while q.o < len(blob):
                if len(blob) - q.o < 12:
                    res['tail_hex'] = blob[q.o:].hex()
                    break
                stack, ln = q.u32(), q.u32()
                if ln > 128:
                    raise ValueError
                name = q.raw(ln).decode('ascii')
                size = q.u32()
                data = q.raw(size)
                if len(data) != size:
                    raise ValueError
                recs.append({'name': name, 'stack_size': stack, 'saved_hex': data.hex()})
            res['looks_like_scripts'] = recs
        except (ValueError, struct.error, UnicodeDecodeError, IndexError):
            pass
        return res

    out['scripts'] = scripts()
    section('running scripts: count + %d records' % len(out['scripts']))
    main_end = r.o
    res = []
    x = residue(main_end, ADDON_OFF, 'after the main block')
    if x:
        res.append(x)
    layout.append({'off': main_end, 'len': ADDON_OFF - main_end,
                   'what': '0xCD fill' + (' (with stale bytes, see _residue)' if x else '')})

    # ---- South Central block ------------------------------------------------------------------
    r.o = ADDON_OFF
    mark[0] = ADDON_OFF
    present = r.u32()
    addon = {'present': present}
    if present == 1:
        am = missions(True)
        out['missions'] += am
        addon['missions'] = [m['name'] for m in am]
        addon['scripts'] = scripts()
    elif present != 0:
        unk(ADDON_OFF, d[ADDON_OFF:ADDON_OFF + 4], 'add-on block marker is neither 0 nor 1')
    out['addon'] = addon
    section('South Central block: marker, missions, scripts')
    x = residue(r.o, ENTRY_SIZE, 'after the South Central block')
    if x:
        res.append(x)
    layout.append({'off': r.o, 'len': ENTRY_SIZE - r.o,
                   'what': '0xCD fill' + (' (with stale bytes, see _residue)' if x else '')})
    out['entry']['used_main'] = main_end
    out['entry']['used_addon'] = r.o - ADDON_OFF

    # ---- views ------------------------------------------------------------------------------
    by_slot = sorted((m for m in out['missions'] if m['active_slot'] is not None),
                     key=lambda m: m['active_slot'])
    out['active_order'] = [m['name'] for m in by_slot]
    by_done = sorted((m for m in out['missions'] if m['completed_slot'] is not None),
                     key=lambda m: m['completed_slot'])
    out['completed_order'] = [m['name'] for m in by_done]

    out['_layout'] = layout
    out['_residue'] = res
    out['_unknown'] = unknown
    return out


if __name__ == '__main__':
    args = [a for a in sys.argv[1:] if not a.startswith('--')]
    if not args:
        sys.exit('usage: python career.py <save> [--raw]')
    with open(args[0], 'rb') as f:
        data = f.read()
    json.dump(decode(data, raw='--raw' in sys.argv), sys.stdout, indent=1)
    sys.stdout.write(os.linesep)
