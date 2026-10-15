"""options.py - decoder of the "LocalOptions" entry of a Midnight Club: Los Angeles save
(mc4.sav / decrypted RAGE.SAV), everything except the interior of the garage vehicle records.

    from options import decode
    d = decode(open('mc4.sav', 'rb').read())

`python options.py <save>` prints the dict as indented JSON.

The entry is a raw copy of the game's profile object (global 0x8288E920, 254032 bytes; the
byte right after it, +0x3E050, is the "needs saving" flag and is not saved).  Layout and the
functions that prove each field: see options.md.
"""
import json
import struct
import sys

SIZE = 254032
GARAGE_AT, SLOT, SLOTS = 1360, 8176, 30          # sub_8238B6E8: `cmpwi r30,30`
RECORDS_AT, RECORD, RECORDS = 0x3C370, 84, 77    # sub_8238CBC8 (77 constructors), sub_8238C9B0 (memset 6468)
DAMAGE_AT = 0x3DCB4                              # 30 x 6 floats (sub_8238ACD8)
COLLECT_AT, COLLECT_N = 0x3DF84, 60              # sub_8238B520
ABILITY_AT = 0x3DFC2                             # sub_8238AD58 / sub_8238AE58
PLAYLIST_AT, PLAYLIST_N = 0x3DFC8, 32            # sub_826410D8 / sub_826402C0

# tune/controller list (file 0xdcd4b916 "Maps 9"), index = the value of +0 / +2
CONTROL_CONFIGS = ['Throttle_Stick', 'Throttle_Trigger', 'Throttle_Stick_Manual', 'Throttle_Trigger_Manual',
                   'Custom_Pad', 'Steering_Wheel', 'Steering_Wheel_Manual', 'Custom_Steering_Wheel',
                   'Throttle_Stick_Hydraulics']
# table 0x8204FF90 (index, name) used by the .map loader sub_82391B40: mapping slot -> action
ACTIONS = {0: 'STEER_AXIS', 1: 'STEER_AXIS_UP_DN', 2: 'GAS', 3: 'BRAKE', 4: 'SHIFT_UP', 5: 'SHIFT_DOWN', 6: 'HANDBRAKE',
           7: 'WEIGHT_XFER', 8: 'NITRO', 9: 'HEADLIGHTS', 10: 'HORN', 11: 'IN_ZONE', 12: 'CAM_UP', 13: 'CAM_DOWN', 14: 'MAP',
           15: 'CAM_ZOOM_IN', 16: 'CAM_ZOOM_OUT', 17: 'PAUSE', 18: 'TALK', 19: 'MUSIC_UP', 20: 'MUSIC_DOWN', 21: 'MUSIC_PAUSE',
           22: 'CAM_PAN_LEFT', 23: 'CAM_PAN_RIGHT', 24: 'CAM_PAN_UP', 25: 'CAM_PAN_DOWN', 26: 'LOOKBACK', 27: 'CAMERA_LEFT',
           28: 'CAMERA_RIGHT', 29: 'CAMERA_UP', 30: 'POPUP_LEFT', 31: 'POPUP_RIGHT', 32: 'POPUP_UP', 33: 'POPUP_DOWN',
           34: 'POPUP_ANALOG_LEFT_RIGHT', 35: 'POPUP_ANALOG_UP_DOWN', 36: 'POPUP_SELECT', 37: 'POPUP_BACK',
           38: 'WHEEL_FRONT_LEFT', 39: 'WHEEL_FRONT_RIGHT', 40: 'WHEEL_BACK_LEFT', 41: 'WHEEL_BACK_RIGHT', 42: 'HUD_PLAYERLIST',
           43: 'CONVERTIBLE_UP_DN', 44: 'HYDRAULICS', 45: 'UI_START', 46: 'HUD_CYCLE', 64: 'PHOTO_CAM_MOVE_UP',
           65: 'PHOTO_CAM_MOVE_DN', 66: 'PHOTO_CAM_MOVE_LT', 67: 'PHOTO_CAM_MOVE_RT', 68: 'PHOTO_CAM_ZOOM_IN',
           69: 'PHOTO_CAM_ZOOM_OUT', 70: 'PHOTO_CAM_PAN_UP', 71: 'PHOTO_CAM_PAN_DN', 72: 'PHOTO_CAM_PAN_LT', 73: 'PHOTO_CAM_PAN_RT'}
DEVICES = {0: 'none', 8: 'button', 9: 'axis'}    # the .map files' *_DVC values
# (guess: RAGE pad button order - the option menu's glyph names L2 R2 L1 R1 RUP RRIGHT ... follow the
# language names at 0x828267A8; START = 11 and the d-pad = 12..15 agree with PAUSE / MAP / MUSIC_*)
BUTTONS = ['L2', 'R2', 'L1', 'R1', 'RUP', 'RRIGHT', 'RDOWN', 'RLEFT', 'SELECT', 'L3', 'R3', 'START', 'LUP', 'LRIGHT', 'LDOWN', 'LLEFT']
CAMERAS = ['Bumper', 'POV', 'Boom 1', 'Boom 2', 'Boom 3']               # table 0x827E929C (mcCameraOptionsForm)
ARROWS = ['option_arrow_compass', 'option_arrow_top', 'option_arrow_none']  # table 0x82826768+8 (mcGameOptionsForm)
GENRES = ['MUSIC_GENRE_ECLECTIC', 'MUSIC_GENRE_ELECTRONIC', 'MUSIC_GENRE_HARDROCK', 'MUSIC_GENRE_HIPHOP', 'MUSIC_GENRE_ROCK',
          'MUSIC_GENRE_TECHNO', 'MUSIC_GENRE_WEST_RAP', 'MUSIC_GENRE_ALL', 'MUSIC_GENRE_ON_THE_FLY']  # table 0x827DA540
LANGUAGES = {0: 'english', 1: 'spanish', 2: 'french', 3: 'german', 4: 'italian'}  # mcLanguageForm (5, 6: not named)
ABILITIES = ['zone', 'agro', 'roar', 'pulse']    # sub_8238AD58 jump table: aZON, aAGR, aROR, aPUL

# the game's vehicle table: the save's records are sorted by it (sub_8238C5A0), 69 entries
VEHICLES = ['vp_vw_golf_83', 'vp_nsn_240sx_98', 'vp_dsn_280z_75', 'vp_vw_scirocco_88', 'vp_chv_camaross_69', 'vp_frd_boss302_69',
            'vp_pon_transam_79', 'vp_chv_impala_96', 'vp_buk_grandnational_87', 'vp_dge_challenger_70', 'vp_frd_focus_04',
            'vp_pon_solstice_06', 'vp_mit_eclipse_99', 'vp_chv_cobalt_05', 'vp_mzd_rx8_06', 'vp_duc_monster_s4r_07',
            'vp_frd_mustang_06', 'vp_chv_camaro_06', 'vp_chv_camaro_06_dub', 'vp_dge_charger_06', 'vp_dge_challenger_06',
            'vp_dge_challenger_06_dub', 'vp_cry_300c_06', 'vp_vw_golf_07', 'vp_nsn_350z_06', 'vp_mit_3000gt_99', 'vp_mzd_rx7_95',
            'vp_nsn_skyline_99', 'vp_aud_rs4_07', 'vp_mit_evo_06', 'vp_duc_999r_06', 'vp_mbz_s600_07', 'vp_mbz_s600_07_dub',
            'vp_lam_miura_71', 'vp_aud_r8_08', 'vp_am_vantage_07', 'vp_mbz_sl65_07', 'vp_chv_corvettez06_07', 'vp_sln_s281_06',
            'vp_lam_gallardo_06', 'vp_lam_murcielago_06', 'vp_lam_murcielago_06_dub', 'vp_frd_cobra_04', 'vp_frd_gt_06',
            'vp_sln_s7_06', 'vp_kaw_ninja_zx14_06', 'vp_chv_police_96', 'vp_citybus_07', 'vpd_chv_belair_55', 'vpd_buk_riviera_63',
            'vpd_chv_impala_64', 'vpd_chv_malibu_65', 'vpd_pon_gto_66', 'vpd_chv_chevelle_70', 'vpd_lr_hse_08', 'vpd_lr_sport_08',
            'vpd_mit_eclipse_09', 'vpd_aud_s5_08', 'vpd_mit_evox_08', 'vpd_cad_xlrv_08', 'vpd_mbz_clk63amg_08',
            'vpd_mbz_cl65amg_08', 'vpd_am_db9_08', 'vpd_duc_1098_07',
            # 64..68: the Complete Edition's police vehicles (names found by hashing the disc's folders)
            'vp_se_chv_plc_96', 'vp_se_chv_chp_96', 'vp_se_dge_plc_06', 'vp_se_dge_ubp_06', 'vp_se_dge_chp_06']

# a vehicle's part-ownership record: category -> (offset in the record, bits)   sub_8238F0B0 / sub_8238F838
CATEGORIES = {14: (12, 6), 12: (13, 6), 11: (14, 6), 13: (15, 6), 17: (16, 6), 18: (17, 6), 19: (18, 6), 30: (19, 6), 16: (20, 6),
              1: (21, 6), 2: (22, 6), 3: (23, 6), 4: (24, 6), 21: (25, 6), 43: (26, 6), 44: (27, 6), 45: (28, 6), 46: (29, 6),
              10: (30, 80), 52: (40, 80), 7: (50, 16), 8: (52, 150), 9: (71, 10), 35: (73, 45), 6: (79, 6), 5: (80, 6), 53: (81, 6)}
# The categories' names: the sections of the vehicles' part files (tune/garage/<hash of <car>_parts.gtun>).
# From the code: the garage's menu section number gives the part slot of the vehicle record (sub_8269A400)
# and the category (sub_8269BA28) - section 1 hood, 2 bumper_r, 3 bumper_f, 5 skirts, 6 spoiler, 7 headlight,
# 8 taillight, 9 frontgrill, 11 blower, 12 intercooler, 13..15 bike_tail / tank / cowl, 16 door_l (the scissor
# doors), 18 widebody ("body"), 24 seats_f, 25 stereo, 28 steer_whl, 29 post_gauge, 30 rims, 31 tires,
# 32 brakes, 34 exh_pipe, 35..46 performance, 47 exh_side. 5 and 6 have no section: garage.FinishMiscPurchase
# (0 / 1) sets bit 0 of them for $1000 (sub_826AB520 at 0x826AEE08 / 0x826AEE80) - the paint shop and the
# vinyl editor, bought once per model (Confirm_Buy_Paint / Confirm_Buy_Vinyl of tune/ui/garage/Main.sc.xml).
# The random free part sub_8269DBF8 agrees (bikes 2 / 4 / 3, cars 11 12 13 17 | 46 43 45 | 19 18 | 14 10 | 21),
# and so does the 100 % save: every bought car's fitted parts have their bits.
# No part file has a frontgrill or a stereo section: 30 and 44 are never set.
CATEGORY_NAMES = {1: 'intercooler', 2: 'bike_tail', 3: 'bike_tank', 4: 'bike_cowl', 5: 'paint', 6: 'vinyl', 7: 'tires', 8: 'rims',
                  9: 'brakes', 10: 'exhaust', 11: 'bumper_front', 12: 'bumper_rear', 13: 'spoiler', 14: 'hood', 16: 'blower',
                  17: 'skirts', 18: 'headlight', 19: 'taillight', 21: 'body', 30: 'frontgrill', 35: 'performance', 43: 'seats_f',
                  44: 'stereo', 45: 'steer_whl', 46: 'post_gauge', 52: 'side_exhaust', 53: 'scissor_door'}
# performance: bit = part * 3 + stage, the parts in the record's order of thirteen (engine 0..6, clutch,
# gear kit, shocks, sway bars, brakes, tires), stage 0 stock, 1, 2 (sub_8269B7C0: base 0 / 21 / 27 / 33 / 36
# per group + 3 x place + the fitted stage; a stage 2 also sets stage 1). The shop reads the highest of
# stage 2, stage 1 (sub_8269B6D8) and makes the stages up to it free (sub_826A8640).
# the garage menu's section number -> category (sub_8269BA28)
MENU_SECTION_CATEGORY = {1: 14, 2: 12, 3: 11, 5: 17, 6: 13, 7: 18, 8: 19, 9: 30, 11: 16, 12: 1, 13: 2, 14: 3, 15: 4, 16: 53, 18: 21,
                         24: 43, 25: 44, 28: 45, 29: 46, 30: 8, 31: 7, 32: 9, 34: 10, 47: 52,
                         **{k: 35 for k in range(35, 47)}}


def joaat(s):
    """The game's name hash (lower-cased one-at-a-time)."""
    h = 0
    for c in s.lower().encode('latin1'):
        h = (h + c) & 0xffffffff
        h = (h + (h << 10)) & 0xffffffff
        h ^= h >> 6
    h = (h + (h << 3)) & 0xffffffff
    h ^= h >> 11
    return (h + (h << 15)) & 0xffffffff


VEHICLE_BY_HASH = {joaat(v): v for v in VEHICLES}


def find_entry(sav, name='LocalOptions'):
    """-> (data offset, size, data version) of a named entry of the save container."""
    ver, cnt = struct.unpack_from('>II', sav, 0)
    off = 8
    for _ in range(cnt):
        nl = struct.unpack_from('>I', sav, off)[0]
        off += 4
        nm = sav[off:off + nl].split(b'\0')[0].decode('latin1')
        off += nl
        dv = sav[off]
        sz = struct.unpack_from('>I', sav, off + 1)[0]
        off += 5
        if nm == name:
            return off, sz, dv
        off += sz
    raise ValueError('no %s entry' % name)


def _cstr(b):
    return b.split(b'\0')[0].decode('latin1')


def _r(x):
    return round(x, 6)


def _mapping(p, at):
    out = {}
    for i in range(74):
        dvc, key = struct.unpack_from('>2I', p, at + 8 * i)
        if (dvc, key) == (0, 0) and i not in ACTIONS:
            continue
        e = {'slot': i, 'device': DEVICES.get(dvc, dvc), 'key': key}
        if dvc == 8 and key < 16:
            e['button_guess'] = BUTTONS[key]
        out[ACTIONS.get(i, 'slot%d' % i)] = e
    return out


def _bits(b, n):
    return [i for i in range(n) if b[i >> 3] >> (i & 7) & 1]


def decode(sav):
    base, size, version = find_entry(sav)
    if size != SIZE:
        raise ValueError('LocalOptions is %d bytes, expected %d' % (size, SIZE))
    p = sav[base:base + size]
    u32 = lambda o: struct.unpack_from('>I', p, o)[0]
    i32 = lambda o: struct.unpack_from('>i', p, o)[0]
    u16 = lambda o: struct.unpack_from('>H', p, o)[0]
    i16 = lambda o: struct.unpack_from('>h', p, o)[0]
    f32 = lambda o: struct.unpack_from('>f', p, o)[0]
    unknown = []

    def unk(off, n, note):
        unknown.append({'off': off, 'len': n, 'hex': p[off:off + n].hex(), 'note': note})

    def pad(off, n, note):
        # padding proven by the code: reported only when it is not all zero
        if any(p[off:off + n]):
            unk(off, n, 'padding expected zero: ' + note)

    cfg_pad, cfg_wheel = i16(0), i16(2)
    cine = u32(1192)
    genre = u32(1328)
    lang = u32(1320)
    opts = {
        'control_config_pad': cfg_pad,
        'control_config_pad_name': CONTROL_CONFIGS[cfg_pad] if 0 <= cfg_pad < 9 else None,
        'control_config_wheel': cfg_wheel,
        'control_config_wheel_name': CONTROL_CONFIGS[cfg_wheel] if 0 <= cfg_wheel < 9 else None,
        'custom_mapping_pad': _mapping(p, 4),
        'custom_mapping_wheel': _mapping(p, 596),
        'vibration': bool(u32(1188)),
        'cinematics': cine,                       # 0 none, 1 both, 2 idle camera only, 3 camera shake only
        'idle_camera': cine in (1, 2),
        'camera_shake': cine in (1, 3),
        'gps_arrow': u32(1204),
        'gps_arrow_name': ARROWS[u32(1204)] if u32(1204) < 3 else None,
        'camera': u32(1208),
        'camera_name': CAMERAS[u32(1208)] if u32(1208) < 5 else None,
        'steering_sensitivity': _r(f32(1224)),    # -1.0 .. 1.0 in steps of 0.1 (slider -10..10)
        'convertible_top_down': bool(p[1228]),
        'subtitles': bool(u16(1238)),
        'volume_effects': _r(f32(1240)),          # 0.0 .. 1.0 in steps of 0.1 (slider 0..10)
        'volume_music': _r(f32(1244)),
        'volume_voice': _r(f32(1248)),
        'volume_player_vehicle': _r(f32(1252)),
        'volume_cutscene': _r(f32(1256)),
        'online_character': u32(1316),
        'language': lang,
        'language_name': LANGUAGES.get(lang),
        # (the garage's LOWERING PATTERN row, 0..4: garage.ChangeAirridePattern -> sub_8238C048;
        # read back for the menu at 0x826AF424 and copied into the air ride object, sub_8234FF70)
        'airride_pattern': p[1324],
        'music_shuffle': bool(p[1325]),
        'music_genre': genre,
        'music_genre_name': GENRES[genre] if genre < 9 else None,
        'time_of_day_hours': _r(f32(1332)),
        'custom_playlist': ['0x%08x' % u32(PLAYLIST_AT + 4 * i) for i in range(PLAYLIST_N) if u32(PLAYLIST_AT + 4 * i)],
    }
    # fields the code only initialises and compares (no reader found): kept raw
    unk(1196, 4, 'u32, default 0, cleared by -nocinematics together with 1192; no reader found')
    unk(1200, 4, 'u32, default 2; no reader found')
    unk(1212, 4, 'u32, default 0; no reader found')
    unk(1216, 4, 'u32, default 1; no reader found')
    unk(1220, 4, 'u32, default 0; no reader found')
    unk(1232, 4, 'f32, default 1.049 (0x8204E3D4); no reader found')
    unk(1236, 2, 'u16, default 0; no reader found')
    unk(1260, 4, 'u32, default 0; no reader found')
    unk(1264, 10, 'char[10], default empty; no writer found')
    unk(1274, 40, 'wchar[20], default empty; no writer found')
    unk(1336, 1, 'u8, default 1; no reader found')
    unk(0x3DFC0, 2, 'u16, only cleared by the reset sub_8238C9B0')
    unk(0x3DFC6, 1, 'u8, only cleared by the reset sub_8238C9B0')
    pad(1229, 3, 'after the convertible byte')
    pad(1314, 2, 'after the wide string')
    pad(1326, 2, 'after the music bytes')
    pad(1337, 3, 'after byte 1336')
    pad(1352, 8, 'alignment of the garage (16)')
    pad(0x3DFC7, 1, 'before the playlist')
    pad(254024, 8, 'alignment of the object (16)')

    count, current, nrec = i32(1340), i32(1344), i32(1348)
    slots = []
    for i in range(SLOTS):
        b = GARAGE_AT + i * SLOT
        name = _cstr(p[b + 8109:b + 8141])
        slots.append({'slot': i, 'in_use': i < count, 'vehicle': name, 'display_name': _cstr(p[b + 8141:b + 8176]),
                      'table_index': i32(b + 8088), 'valid': u32(b + 8100), 'name_hash': '0x%08x' % u32(b + 8104)})
    garage = {'count': count, 'current_slot': current,
              'current_vehicle': slots[current]['vehicle'].lower() if 0 <= current < count else None,
              'owned': [s['vehicle'].lower() for s in slots[:max(0, min(count, SLOTS))]],
              'slots': slots,
              # the brief gave slots 0..28 to the vehicle decoder; slot 29 is a garage record like
              # the others (stale in every sample) - kept whole so that nothing is lost
              'slot29_raw_hex': p[GARAGE_AT + 29 * SLOT:GARAGE_AT + 30 * SLOT].hex()}

    records = []
    for i in range(RECORDS):
        b = RECORDS_AT + i * RECORD
        h = u32(b + 4)
        spent_raw = u16(b + 82)
        owned = {}
        for cat, (off, bits) in sorted(CATEGORIES.items()):
            got = _bits(p[b + off:b + off + (bits + 7) // 8], bits)
            if got:
                owned[CATEGORY_NAMES.get(cat, 'cat%d' % cat)] = got
        rec = {'index': i, 'in_use': i < nrec, 'hash': '0x%08x' % h,
               'vehicle': VEHICLE_BY_HASH.get(h, ('0x%08x' % h) if h else None),
               'refund_value': i32(b + 8),          # written by the save hook sub_8238B3F8
               'spent_plus_1': spent_raw,           # 16-bit sum of what was spent on parts, kept + 1
               'owned_parts': owned}
        if u32(b):
            rec['vtable_word'] = '0x%08x' % u32(b)
        records.append(rec)

    damage = []
    for i in range(SLOTS):
        f = struct.unpack_from('>6f', p, DAMAGE_AT + 24 * i)
        damage.append({'slot': i, 'damage': _r(f[0]), 'zones': [_r(x) for x in f[1:5]], 'unused': _r(f[5])})

    coll = [bool(x) for x in p[COLLECT_AT:COLLECT_AT + COLLECT_N]]
    out = {
        'entry': {'offset': base, 'size': size, 'data_version': version},
        'options': opts,
        'garage': garage,
        'vehicle_record_count': nrec,
        'vehicle_records': records,
        'garage_damage': damage,
        'collectables': coll,
        'collectables_found': [i for i, c in enumerate(coll) if c],
        'collectables_count': sum(coll),
        'ability_levels': {n: p[ABILITY_AT + i] for i, n in enumerate(ABILITIES)},
        '_not_in_this_entry': ['money (statistic tPUR in "Stats")', 'reputation ("Career")', 'cheats', 'tutorial flags',
                               'speed units', 'brightness', 'HUD/minimap switches', 'autosave switch (save manager)',
                               'GPS switch (entry "enableGps")'],
        '_unknown': unknown,
    }
    return out


if __name__ == '__main__':
    for path in sys.argv[1:]:
        print(json.dumps(decode(open(path, 'rb').read()), indent=1))
