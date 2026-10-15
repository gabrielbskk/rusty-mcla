"""vehicle.py - the garage vehicle records of an original Midnight Club: Los Angeles save.

    decode(sav: bytes) -> {"slots": [ {...} x 30 ], ...}

A record is one `mcCarConfig` object ("CarCfg", 8176 bytes, the same bytes as a disc
`resources/vehicle/configs/*.xcc`), written into the "LocalOptions" entry as raw memory:
30 of them from LocalOptions + 1360 (sub_8238B6E8 bounds the slot index with 30).  The layout below is the game's own: the field names are
the ones its parser registers (sub_8238F4A8 -> sub_82398390 "CarCfgPerf", sub_82394388
"CarCfgExt"), the rest comes from the constructors / copy / network writers.  See vehicle.md.

Run as a script on a save to print the dict as JSON.  Everything not named is returned under
"_unknown" of its slot (offset in the record, length, hex), so no byte is lost.
"""
import json
import struct
import sys

LOCALOPTIONS = 0x4237          # data offset of the LocalOptions entry in the file
FIRST, SIZE, COUNT = 1360, 8176, 30

VEHICLES = ['vp_vw_golf_83', 'vp_nsn_240sx_98', 'vp_dsn_280z_75', 'vp_vw_scirocco_88', 'vp_chv_camaross_69', 'vp_frd_boss302_69', 'vp_pon_transam_79', 'vp_chv_impala_96',
            'vp_buk_grandnational_87', 'vp_dge_challenger_70', 'vp_frd_focus_04', 'vp_pon_solstice_06', 'vp_mit_eclipse_99', 'vp_chv_cobalt_05', 'vp_mzd_rx8_06', 'vp_duc_monster_s4r_07',
            'vp_frd_mustang_06', 'vp_chv_camaro_06', 'vp_chv_camaro_06_dub', 'vp_dge_charger_06', 'vp_dge_challenger_06', 'vp_dge_challenger_06_dub', 'vp_cry_300c_06', 'vp_vw_golf_07',
            'vp_nsn_350z_06', 'vp_mit_3000gt_99', 'vp_mzd_rx7_95', 'vp_nsn_skyline_99', 'vp_aud_rs4_07', 'vp_mit_evo_06', 'vp_duc_999r_06', 'vp_mbz_s600_07', 'vp_mbz_s600_07_dub',
            'vp_lam_miura_71', 'vp_aud_r8_08', 'vp_am_vantage_07', 'vp_mbz_sl65_07', 'vp_chv_corvettez06_07', 'vp_sln_s281_06', 'vp_lam_gallardo_06', 'vp_lam_murcielago_06',
            'vp_lam_murcielago_06_dub', 'vp_frd_cobra_04', 'vp_frd_gt_06', 'vp_sln_s7_06', 'vp_kaw_ninja_zx14_06', 'vp_chv_police_96', 'vp_citybus_07', 'vpd_chv_belair_55',
            'vpd_buk_riviera_63', 'vpd_chv_impala_64', 'vpd_chv_malibu_65', 'vpd_pon_gto_66', 'vpd_chv_chevelle_70', 'vpd_lr_hse_08', 'vpd_lr_sport_08', 'vpd_mit_eclipse_09',
            'vpd_aud_s5_08', 'vpd_mit_evox_08', 'vpd_cad_xlrv_08', 'vpd_mbz_clk63amg_08', 'vpd_mbz_cl65amg_08', 'vpd_am_db9_08', 'vpd_duc_1098_07']

# --- the game's own name tables (pointer table at 0x827E95F4 .. of the executable) ---
PERF_PARTS = ['exhaust', 'headers', 'intake', 'forced_induction', 'engine', 'electronics', 'nitrous',   # EngineState0..6
              'clutch', 'gear_kit',                                                                      # TransState0..1
              'shocks', 'sway_bars',                                                                     # SuspensionState0..1
              'brakes',                                                                                  # BrakeState0
              'tires']                                                                                   # TireState0
PERF_FIELDS = ['EngineState0', 'EngineState1', 'EngineState2', 'EngineState3', 'EngineState4', 'EngineState5', 'EngineState6',
               'TransState0', 'TransState1', 'SuspensionState0', 'SuspensionState1', 'BrakeState0', 'TireState0']
PAINT_FADE = ['None', 'ColorShift', 'FrontBack', 'LeftRight', 'TopBottom']
PAINT_TYPE = ['DirtyMatte', 'Matte', 'Gloss', 'Metallic', 'Pearl', 'Sparkle']
PAINT_AREA = ['Body', 'Doors', 'Hood', 'Front Bumper', 'Rear Bumper', 'Skirts']
RIM_MATERIAL = ['Painted', 'Chrome', 'Carbon Fiber', 'Brushed Metal']
VINYL_ZONE = ['TopVinyl', 'LeftVinyl', 'RightVinyl', 'FrontVinyl', 'BackVinyl']
ZONE_START = [0, 64, 128, 192, 208]      # 0x827E9770
ZONE_CAP = [64, 64, 64, 16, 16]          # 0x820510B0
VINYL_BLEND = ['normal', 'alpha cutout', 'negative alpha cutout']
GAUGE_TEXT = ['speedometer', 'odometer', 'trip', 'gear select', 'clock']
INTERIOR_GROUP = ['group 1', 'group 2', 'group 3', 'group 4', 'group 5', 'group 6 (hidden from user)', 'group 7 (hidden from user)',
                  'group 8 (hidden from user)', 'group 9 (hidden from user)', 'group 10 (hidden from user)']
LAMP = ['brake light', 'taillight', 'turn signal', 'headlight', 'high beam', 'side marker', 'reverse light', 'interior light', 'fog light',
        'red reflector', 'amber reflector', 'clear reflector', 'third brakelight', 'red coplight', 'blue coplight',
        'turn signal FL', 'turn signal FR', 'turn signal BL', 'turn signal BR']
KLIGHT = ['kLightBrakelight', 'kLightTaillight', 'kLightThirdBrakelight', 'kLightHeadlight', 'kLightHiBeam', 'kLightFogLight', 'kLightTurnSignal',
          'kLightPolice0', 'kLightPolice1', 'kLightPolice2', 'kLightInterior', 'kLightReverseLight', 'kLightNone',
          'kLightTurnSignalFL', 'kLightTurnSignalFR', 'kLightTurnSignalBL', 'kLightTurnSignalBR']
ABILITY = ['ZONE', 'AGRO', 'ROAR', 'EMP']          # garage switch of sub_826A6FF0; -1 = none
PARTS = ['conv_top', 'conv_topup', 'hood', 'bumper_r', 'trunk_swap', 'intercooler', 'bumper_f', 'fenders', 'skirts', 'spoiler', 'spoiler_b',
         'headlight', 'taillight', 'taillight_b', 'frontgrill', 'wheeliebar', 'blower', 'bike_tail', 'bike_tank', 'bike_cowl', 'door_l', 'door_r',
         'conv_topdown', 'exh_pipe', 'exh_side', 'armfl', 'armfr', 'armrl', 'armrr', 'shockfl', 'shockfr', 'shockrl', 'shockrr', 'guiderl',
         'guiderr', 'axle', 'subbumf', 'subbumr', 'widebody', 'spoiler_wide', 'hood_wide', 'door_l_wide', 'door_r_wide', 'trunk_wide', 'interior',
         'seats_f', 'seats_p', 'door_l_int', 'door_r_int', 'stereo', 'stereo_l', 'stereo_r', 'steer_whl', 'post_gauge', 'occluder_',
         # the last bytes of the 64-byte block are switches, not model parts
         'RiderIdx', 'CarbonFiberHood', 'TaillightMaterialIdx', 'ScissorDoors', 'WideBodyKit', 'SideExhaustInUse', 'UseIntercooler']
# tune/vehicle/wheel_models.list (the tables RimMdlIdx / TireTexIdx / BrakeMdlIdx index)
TIRES = ('tir_car_bfg_kdw_blk tir_car_bfg_rta_blk tir_car_bfg_rta_blkrl tir_car_bfg_rta_ol tir_car_bfg_rta_wht tir_car_bfg_rta_whtrl tir_car_bfg_rta_yel '
         'tir_car_pir_p0_blk tir_car_pir_p0_corsa tir_car_pir_p0_nero tir_car_pir_p0_rosso tir_car_bfg_rta_whtw '
         'tir_streetbike_tst tir_bike_pir_corsa3 tir_bike_pir_diablo_rosso tir_bike_pir_diablo_supercorsa').split()
BRAKES = ('brk_car_base brk_car_brembo_drilled brk_car_brembo_zinc brk_car_projmu_slotted brk_car_projmu_slotdrilled brk_stk_duc_999r_06 '
          'brk_stk_duc_monster_s4r_07 brk_stk_kaw_ninja_zx14_06 brk_stk_am_vantage_07 brk_car_brembo_base brk_car_brembo_base_blank '
          'brk_car_brembo_drilled_blank brk_car_brembo_slotted_blank brk_car_brembo_drilled_lrg_blank brk_stk_aud_r8_08').split()
RIMS = ('whl_am_basewheel whl_stk_buk_grandnational_87 whl_stk_chv_camaross_69 whl_stk_chv_cobalt_05 whl_stk_dge_challenger_70 whl_stk_dsn_280z_75 '
        'whl_stk_duc_999r_06 whl_stk_mit_3000gt_99 whl_stk_mit_eclipse_99 whl_stk_mit_evo_06 whl_stk_nsn_240sx_98 whl_stk_pon_solstice_06 '
        'whl_stk_pon_transam_79 whl_stk_vw_scirocco_88 whl_stk_lam_gallardo_06 whl_stk_chv_impala_96 whl_stk_cry_300c_06 whl_stk_dge_charger_06 '
        'whl_stk_frd_boss302_69 whl_stk_frd_focus_04 whl_stk_lam_murcielago_06 whl_stk_mzd_rx8_06 whl_stk_nsn_350z_06 whl_stk_frd_gt_06 '
        'whl_stk_frd_mustang_06 whl_stk_sln_s281_06 whl_stk_vw_golf_83 whl_stk_duc_monster_s4r_07 whl_stk_lam_miura_71 whl_stk_kaw_ninja_zx14_06 '
        'whl_stk_sln_s7_06 whl_stk_dge_challenger_06 whl_stk_nsn_skyline_99 whl_stk_chv_camaro_06 whl_stk_chv_police_96 whl_stk_vw_golf_07 '
        'whl_stk_mbz_sl65_07 whl_stk_frd_cobra_04 whl_stk_aud_rs4_07 whl_am_tenzo_mach10 whl_am_tis_tis10 whl_am_tis_m304 whl_am_konig_sideways '
        'whl_am_davin_rev71 whl_am_lexan_mystik whl_am_ggrames_57 whl_am_volk_gtv whl_am_works_equip3 whl_am_volk_gt7 whl_am_ggrames_rays99b '
        'whl_am_lexan_901 whl_am_works_emotion whl_am_5zigen_sportengine whl_am_volk_gtc whl_am_dpstr_ds01 whl_am_dpstr_ds03 whl_am_volk_le28n '
        'whl_am_konig_ffgraphite whl_am_iforg_vertex whl_am_tenzo_gf7 whl_am_volk_sfchallenger whl_am_rezax_universal whl_am_konig_unknown '
        'whl_am_iforg_essen whl_am_momo_rev whl_am_works_xd9 whl_am_tis_9 whl_am_davin_speedsp whl_am_tis_m302 whl_am_works_equip1 '
        'whl_am_works_vsxx whl_am_volk_24 whl_am_tis_m305 whl_am_lexan_fire whl_am_momo_rpm whl_am_works_vskf whl_am_dpstr_ds02 whl_am_tenzo_gt5 '
        'whl_am_lexan_ignite whl_am_tenzo_passion5 whl_am_lexan_dagger whl_am_iforg_daytona whl_am_konig_rewindg whl_am_davin_revls '
        'whl_am_asnti_af139 whl_am_iforg_classic whl_am_volk_te37 whl_am_iforg_monaco whl_am_iforg_synergy whl_am_iforg_equip whl_am_tis_m303 '
        'whl_am_asnti_af140 whl_am_works_s2r whl_am_asnti_af141 whl_am_momo_storm whl_am_iforg_concept whl_am_5zigen_5zr whl_am_davin_rev72 '
        'whl_am_asnti_af129 whl_am_lexan_vertex whl_am_amer_smoothie whl_am_amer_torqlite whl_am_amer_redline whl_am_amer_rebel '
        'whl_am_amer_cl200s whl_am_amer_saltflat whl_am_amer_torquethrustII whl_am_amer_aero whl_am_amer_razor whl_am_amer_sting '
        'whl_am_dpstr_ds04 whl_am_dpstr_ds05 whl_am_tis_m301 whl_am_tis_tis06 whl_am_tis_tis07 whl_am_tis_tis11 whl_am_tis_tis12 '
        'whl_stk_mzd_rx7_95 whl_stk_mbz_s600 whl_stk_chv_corvettez06_07 whl_am_dpstr_ds01a whl_am_dpstr_ds02a whl_am_dpstr_ds03a '
        'whl_am_dpstr_ds04a whl_am_dpstr_ds05a whl_stk_am_vantage_07 whl_am_bbs_air_i_ai whl_am_bbs_ch whl_am_bbs_lm whl_am_bbs_rs_gt '
        'whl_am_pm_perf_domino whl_am_pm_perf_gatlin whl_am_pm_perf_judge whl_am_pm_perf_marquee whl_am_pm_perf_tantrum '
        'whl_am_rc_comp_gladiator whl_am_rc_comp_havoc whl_am_rc_comp_raven whl_am_rc_comp_savage whl_am_rc_comp_vega whl_am_asnti_roma2 '
        'whl_am_lexan_marqi whl_am_placeholder whl_stk_aud_r8_08').split()
# the set-up files a record can come from: "<prefix>_<car>" (resources/vehicle/configs), "" = the car's own
CONFIG_PREFIXES = (['', 'starter', 'pinkslip', 'timetrial', 'Delivery', 'Payback', 'pyback', 'pyback_gn', 'pyback_plr', 'FirstRace', 'CIN_01', 'DETROIT',
                    'SC_delivery', 'SC_pinkslip', 'SC_timetrial', 'SC_tournament', 'good_GH_15_2a']
                   + ['GH_%02d_%d' % (n, k) for n in range(1, 36) for k in range(5)]
                   + ['SC_GH_%02d_%d' % (n, k) for n in range(26, 37) for k in range(4)]
                   + ['SH_%02d_%d' % (n, k) for n in range(1, 6) for k in range(4)])
PLATE_SIGNS = {0x5B: '[sign1]', 0x5C: '[sign2]', 0x5D: '[sign3]', 0x5E: '[sign4]'}

UNUSED_LAYER = bytes.fromhex('2fae2fae00000000ffffffff00000000ffff0000')      # constructor sub_82395138, 288 times
UNUSED_GROUP = bytes.fromhex('3c003c0000000000ffffffff00000000ffff0000')      # the 5 zone records at +0x8a0
NAN_W = bytes.fromhex('7f800001')


def joaat(s):
    """The game's string hash (sub_821C9790): lower case, one-at-a-time."""
    h = 0
    for c in s.lower().encode('latin1'):
        h = (h + c) & 0xffffffff
        h = (h + (h << 10)) & 0xffffffff
        h ^= h >> 6
    h = (h + (h << 3)) & 0xffffffff
    h ^= h >> 11
    return (h + (h << 15)) & 0xffffffff


def _enum(names, v):
    return names[v] if 0 <= v < len(names) else v


class _Rec:
    """One record with book-keeping of which bytes were read."""

    def __init__(self, b):
        self.b = b
        self.seen = bytearray(len(b))
        self.odd_pad = []

    def take(self, off, n):
        for i in range(off, off + n):
            self.seen[i] = 1
        return self.b[off:off + n]

    def u8(self, o):
        return self.take(o, 1)[0]

    def s8(self, o):
        return struct.unpack('>b', self.take(o, 1))[0]

    def u16(self, o):
        return struct.unpack('>H', self.take(o, 2))[0]

    def u32(self, o):
        return struct.unpack('>I', self.take(o, 4))[0]

    def i32(self, o):
        return struct.unpack('>i', self.take(o, 4))[0]

    def f32(self, o):
        return _f(struct.unpack('>f', self.take(o, 4))[0])

    def vec3(self, o):
        """A Vector3: three floats; the fourth word is alignment padding (NaN 0x7F800001 or 0)."""
        v = [_f(x) for x in struct.unpack('>3f', self.take(o, 12))]
        self.pad(o + 12, 4, (NAN_W, bytes(4)))
        return v

    def vec4(self, o):
        return [_f(x) for x in struct.unpack('>4f', self.take(o, 16))]

    def half(self, o, n=1):
        v = [_f(x) for x in struct.unpack('>%de' % n, self.take(o, 2 * n))]
        return v[0] if n == 1 else v

    def cstr(self, o, n):
        raw = self.take(o, n)
        z = raw.find(0)
        s = raw if z < 0 else raw[:z]
        rest = b'' if z < 0 else raw[z + 1:]
        if any(c not in (0, 0xCD) for c in rest):
            # (bytes after the terminator are left-overs of an earlier, longer name)
            self.odd_pad.append({'off': o + z + 1, 'len': len(rest), 'hex': rest.hex(), 'what': 'after string terminator'})
        return s.decode('latin1')

    def pad(self, o, n, ok=None):
        """Padding / filler: accepted silently when it holds one of the expected fillers."""
        raw = self.take(o, n)
        fill = ok or (bytes(n), b'\xcd' * n)
        if raw not in fill:
            self.odd_pad.append({'off': o, 'len': n, 'hex': raw.hex(), 'what': 'padding with unexpected bytes'})

    def unknown(self):
        out, i = [], 0
        while i < len(self.b):
            if self.seen[i]:
                i += 1
                continue
            j = i
            while j < len(self.b) and not self.seen[j]:
                j += 1
            out.append({'off': i, 'len': j - i, 'hex': self.b[i:j].hex()})
            i = j
        return out


def _f(x):
    if x != x:
        return 'nan'
    if x in (float('inf'), float('-inf')):
        return 'inf' if x > 0 else '-inf'
    return round(x, 6)


def _plate(raw):
    return ''.join(PLATE_SIGNS.get(c, chr(c) if 32 <= c < 127 else '\\x%02x' % c) for c in raw)


def _layer(r, o):
    """A vinyl layer record, 20 bytes (parser names group<z>_m_... ; writer sub_82393E20)."""
    raw = r.take(o, 20)
    sx, sy, px, py = struct.unpack('>4e', raw[0:8])
    skew, rot = struct.unpack('>2e', raw[12:16])
    shape = struct.unpack('>H', raw[16:18])[0]
    return raw, {
        'm_oScale': [_f(sx), _f(sy)],
        'm_oPos': [_f(px), _f(py)],
        'm_Color': {'argb_hex': raw[8:12].hex(), 'rgba': [raw[9], raw[10], raw[11], raw[8]]},
        'm_fSkew': _f(skew),
        'm_fRot': _f(rot),
        'm_iShapeIdx': -1 if shape == 0xffff else shape,
        'm_bNegateAlpha': _enum(VINYL_BLEND, raw[18]),
        'm_bIsVectorShape': raw[19],
    }


def decode_record(b, index=None):
    """One 8176-byte CarCfg record (a garage slot or a disc .xcc)."""
    assert len(b) == SIZE
    r = _Rec(b)
    out = {}
    if index is not None:
        out['slot'] = index
    type_name = r.cstr(8109, 32)
    profile = r.cstr(8141, 34)
    r.pad(8175, 1)
    vehicle_index = r.i32(8088)
    # (a garage slot is empty when it names no vehicle; a disc .xcc has no index until loaded)
    out['empty'] = (type_name in ('', 'blank')) or (index is not None and vehicle_index < 0)
    out['TypeName'] = type_name
    out['ProfileName'] = profile
    out['vehicle_index'] = vehicle_index
    out['vehicle_table_name'] = VEHICLES[vehicle_index] if 0 <= vehicle_index < len(VEHICLES) else None

    # ---- CarCfg header ----
    out['vtable'] = '0x%08x' % r.u32(0)                 # 0x8204F3D4 (360), 0x00AE26F8 (PS3)
    out['class_key'] = '0x%08x' % r.u32(4)              # 0x000FD813, set by the constructor
    out['base_unused_8'] = r.take(8, 12).hex()          # never written by the constructor
    # ---- CarCfgPerf (+20, 44 bytes) ----
    perf = {'vtable': '0x%08x' % r.u32(20), 'dirty_flags': r.u32(24)}
    st = list(r.take(28, 13))
    perf['stages'] = dict(zip(PERF_PARTS, st))
    perf['stages_raw'] = st
    perf['airbags_fitted'] = r.u8(41)                   # HydraulicsState0
    perf['hydraulics_fitted'] = r.u8(42)                # HydraulicsState1
    br = list(r.take(43, 13))
    perf['brand'] = dict(zip(PERF_PARTS, br))
    perf['brand_raw'] = br
    perf['unused_56'] = r.take(56, 3).hex()             # tail of the 31-byte block, no reader
    perf['upgrade_percent'] = r.s8(59)
    perf['upgrade_percent_override'] = r.s8(60)
    perf['unk_percent_61'] = list(r.take(61, 3))
    out['performance'] = perf

    # ---- CarCfgExt (+64, 8016 bytes) ----
    v = {'vtable': '0x%08x' % r.u32(64)}
    v['m_vInteriorCharacterNeonLightFalloff'] = r.f32(68)
    v['m_vInteriorCharacterGaugeConeAng'] = r.f32(72)
    v['m_vInteriorCharacterGaugeConeFalloff'] = r.f32(76)
    for name, o in (('ExtNeonColor', 0x50), ('IntNeonColor', 0x60), ('DashGaugeColor', 0x70), ('DashGaugeColorB', 0x80),
                    ('RimColorA', 0x90), ('RimColorB', 0xa0), ('RimColorC', 0xb0), ('RimColorD', 0xc0), ('RimColorE', 0xd0),
                    ('m_vBrushedMetalColorA', 0xe0), ('m_vBrushedMetalColorB', 0xf0), ('m_vBrushedMetalColorC', 0x100),
                    ('m_vBrakeCaliperColor', 0x110)):
        v[name] = r.vec3(o)
    for name, o in (('m_vHeadlightGlassColorNew', 0x120), ('m_vTaillightGlassColorNew', 0x130), ('m_vBumpedHeadlightColor', 0x140),
                    ('m_vBumpedTaillightColor', 0x150), ('m_vWindowTintColorNew', 0x160)):
        v[name] = r.vec4(o)
    v['ExtNeonIntensity'] = r.f32(0x170)
    v['IntNeonIntensity'] = r.f32(0x174)
    mats = struct.unpack('>5I', r.take(0x178, 20))
    v['RimMaterial'] = {k: _enum(RIM_MATERIAL, m) for k, m in zip('ABCDE', mats)}
    v['ExtNeonFlashRate'] = r.u8(0x18c)
    v['IntNeonFlashRate'] = r.u8(0x18d)
    r.pad(0x18e, 2)
    v['gauge_text'] = []
    for k in range(5):
        o = 0x190 + 0x20 * k
        v['gauge_text'].append({'element': GAUGE_TEXT[k], 'm_fColor': r.vec4(o), 'm_fSkew': r.f32(o + 16), 'm_iFontIdx': r.i32(o + 20)})
        r.pad(o + 24, 8)
    v['lights'] = []
    for k in range(19):
        o = 0x230 + 0x30 * k
        e = {'lamp': LAMP[k], 'LightBulbColor': r.vec3(o), 'LightCoverColor': r.vec3(o + 16), 'kind': _enum(KLIGHT, r.u32(o + 32))}
        r.pad(o + 36, 12)
        v['lights'].append(e)
    v['m_vBaseColor'] = [{'group': INTERIOR_GROUP[k], 'rgb': r.vec3(0x5c0 + 16 * k)} for k in range(10)]
    v['paint'] = []
    for k in range(6):
        o = 0x660 + 0x30 * k
        e = {'area': PAINT_AREA[k], 'ColorA': r.vec3(o), 'ColorB': r.vec3(o + 16),
             'PaintType': _enum(PAINT_TYPE, r.u32(o + 32)), 'PaintFade': _enum(PAINT_FADE, r.u32(o + 36)),
             'unpainted_flag': r.u8(o + 40)}
        r.pad(o + 41, 7)
        v['paint'].append(e)
    v['PreDmg'] = []
    for k in range(4):
        o = 0x780 + 0x20 * k
        v['PreDmg'].append({'m_vPos': r.vec3(o), 'm_fStrength': r.f32(o + 16)})
        r.pad(o + 20, 12)
    v['m_vInteriorCharacterNeonLightPos'] = r.vec3(0x800)
    v['m_vInteriorCharacterGaugeLightPos'] = r.vec3(0x810)
    v['m_vInteriorCharacterGaugeConeDir'] = r.vec3(0x820)
    v['Camber'] = [r.f32(0x830), r.f32(0x834)]
    for name, o in (('TireOffset', 0x838), ('RideHeight', 0x83a), ('TireWidth', 0x83c), ('RimSize', 0x83e), ('TireProfile', 0x840)):
        v[name] = [r.u8(o), r.u8(o + 1)]            # front, rear
    r.pad(0x842, 2)
    rim, tire, brake = r.u8(0x844), r.u8(0x845), r.u8(0x846)
    v['RimMdlIdx'] = rim
    v['rim_model'] = _enum(RIMS, rim) if rim < len(RIMS) else None
    v['TireTexIdx'] = tire
    v['tire_model'] = _enum(TIRES, tire) if tire < len(TIRES) else None
    v['BrakeMdlIdx'] = brake
    v['brake_model'] = _enum(BRAKES, brake) if brake < len(BRAKES) else None
    letters = r.take(0x847, 7)
    v['plate_letters'] = _plate(letters)
    v['plate_letters_raw'] = list(letters)
    r.pad(0x84e, 1)
    v['LicenseBG'] = r.u8(0x84f)
    v['dirty_flags'] = r.u32(0x850)
    v['m_iCarPaintOverride'] = r.i32(0x854)
    # (not preset ids, whatever the keys say: the vinyl generator's pending redraw - zone and layer -
    # which the garage sets and clears while it works: sub_8269D660, sub_823924E0 / sub_82392510,
    # renderer sub_8235AC78. -1, -1 at rest: in every save and set-up file)
    v['vinyl_preset_group'] = r.i32(0x858)
    v['vinyl_preset_index'] = r.i32(0x85c)
    parts = list(r.take(0x860, 62))
    v['parts'] = dict(zip(PARTS, parts))
    r.pad(0x89e, 2)
    v['vinyl_zone_group'] = []
    for z in range(5):
        raw, lay = _layer(r, 0x8a0 + 20 * z)
        lay['zone'] = VINYL_ZONE[z]
        lay['default'] = raw == UNUSED_GROUP
        v['vinyl_zone_group'].append(lay)
    zones = {}
    for z in range(5):
        layers = []
        for k in range(ZONE_CAP[z]):
            raw, lay = _layer(r, 0x904 + 20 * (ZONE_START[z] + k))
            if raw != UNUSED_LAYER:
                lay['layer'] = k
                lay['used'] = lay['m_iShapeIdx'] >= 0
                layers.append(lay)
        zones[VINYL_ZONE[z]] = layers
    v['vinyl'] = zones
    spare = []
    for k in range(224, 288):                           # allocated, outside every zone
        raw, lay = _layer(r, 0x904 + 20 * k)
        if raw != UNUSED_LAYER:
            lay['layer'] = k
            spare.append(lay)
    v['vinyl_spare_layers_224_287'] = spare
    # (the redraw's "whole zone" flag, left as the garage's last redraw set it: sub_82392538 1, sub_823924E0 0)
    v['vinyl_is_user_slot'] = r.u8(8068)
    r.pad(8069, 11)
    out['visual'] = v

    # ---- CarCfgInt (+8080, 8 bytes) and the record's tail ----
    out['carcfgint'] = {'vtable': '0x%08x' % r.u32(8080), 'dirty_flags': r.u32(8084)}
    out['dirty_flags_all'] = '0x%08x' % r.u32(8092)
    out['unk_8096'] = r.u32(8096)
    out['in_garage'] = r.u32(8100)
    h = r.u32(8104)
    out['source_config_hash'] = '0x%08x' % h
    src = None
    if h:
        for p in CONFIG_PREFIXES:
            n = (p + '_' + type_name) if p else type_name
            if joaat(n) == h:
                src = n
                break
    out['source_config'] = src
    ab = r.s8(8108)
    out['ability'] = _enum(ABILITY, ab) if ab >= 0 else ab

    # ---- derived, for the importer's convenience ----
    tint_a = v['m_vWindowTintColorNew'][3]
    d = {'neon_exterior_on': isinstance(v['ExtNeonIntensity'], float) and v['ExtNeonIntensity'] > 0.0,
         'neon_interior_on': isinstance(v['IntNeonIntensity'], float) and v['IntNeonIntensity'] > 0.0,
         'window_tint_level': None}
    if isinstance(tint_a, float):
        # sub_826AB520: alpha = 0.372 + (0.9 - 0.372) * k, k = 0.002, 0.1 .. 0.9 for the menu's rows 0..9
        d['window_tint_level'] = max(0, min(9, int(round((tint_a - 0.372) / (0.9 - 0.372) * 10))))
    d['origin'] = (src[:-len(type_name) - 1] if src and src.lower() != type_name.lower() else 'stock') if src else None
    out['derived'] = d

    out['_pad_unexpected'] = r.odd_pad
    out['_unknown'] = r.unknown()
    return out


def decode(sav):
    """A whole save file -> {"slots": [30 records]}."""
    ver, count = struct.unpack_from('>II', sav, 0)
    # walk the container rather than trusting the fixed offset
    off, lo = 8, None
    for _ in range(count):
        n = struct.unpack_from('>I', sav, off)[0]
        name = sav[off + 4:off + 4 + n].decode('latin1')
        size = struct.unpack_from('>I', sav, off + 4 + n + 1)[0]
        data = off + 4 + n + 5
        if name == 'LocalOptions':
            lo = data
        off = data + size
    if lo is None:
        raise ValueError('no LocalOptions entry')
    slots = [decode_record(sav[lo + FIRST + i * SIZE: lo + FIRST + (i + 1) * SIZE], i) for i in range(COUNT)]
    # the garage header in front of the records (decoded by the options agent, options.md):
    # LocalOptions + 1340 vehicles owned, + 1344 slot in use, + 1348 size of the vehicle table
    owned, current, _ = struct.unpack_from('>3i', sav, lo + 1340)
    for s in slots:
        # (the first `owned` slots are the garage; the others are never-used or stale copies)
        s['empty'] = s['empty'] or not 0 <= s['slot'] < owned
        s['current'] = s['slot'] == current and not s['empty']
    return {
        'save_version': ver,
        'garage_count': owned,
        'current_slot': current,
        'localoptions_offset': lo,
        'record_size': SIZE,
        'first_record_at_localoptions': FIRST,
        'slots': slots,
    }


if __name__ == '__main__':
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    data = open(sys.argv[1], 'rb').read()
    if len(data) == SIZE:                         # a disc .xcc is one record
        print(json.dumps(decode_record(data), indent=1))
    else:
        print(json.dumps(decode(data), indent=1))
