"""Xbox 360 (Xenos) shader microcode disassembler for the game's rgxa effect files.
(Reference for crates/shaders/src/xenos.rs, which finds and translates the programs at install time.)
usage: python xsh.py <fxl file> [shader name substring]   (lists the programs, disassembles the matches)
"""
import struct, sys, re

VOPS = ["add", "mul", "max", "min", "seq", "sgt", "sge", "sne", "frc", "trunc", "floor", "mad", "cndeq", "cndge", "cndgt", "dp4", "dp3", "dp2add", "cube", "max4",
        "setp_eq_push", "setp_ne_push", "setp_gt_push", "setp_ge_push", "kill_eq", "kill_gt", "kill_ge", "kill_ne", "dst", "maxa"]
SOPS = ["adds", "adds_prev", "muls", "muls_prev", "muls_prev2", "maxs", "mins", "seqs", "sgts", "sges", "snes", "frcs", "truncs", "floors", "exp", "logc", "log", "rcpc", "rcpf",
        "rcp", "rsqc", "rsqf", "rsq", "maxas", "maxasf", "subs", "subs_prev", "setp_eq", "setp_ne", "setp_gt", "setp_ge", "setp_inv", "setp_pop", "setp_clr", "setp_rstr",
        "kills_eq", "kills_gt", "kills_ge", "kills_ne", "kills_one", "sqrt", "?41", "mulsc0", "mulsc1", "addsc0", "addsc1", "subsc0", "subsc1", "sin", "cos", "retain_prev"]
VARGS = {"add": 2, "mul": 2, "max": 2, "min": 2, "seq": 2, "sgt": 2, "sge": 2, "sne": 2, "frc": 1, "trunc": 1, "floor": 1, "mad": 3, "cndeq": 3, "cndge": 3, "cndgt": 3, "dp4": 2,
         "dp3": 2, "dp2add": 3, "cube": 2, "max4": 1, "dst": 2, "maxa": 2}
CFOPS = ["nop", "exec", "exece", "cexec", "cexece", "cexec_pred", "cexec_pred_e", "loop", "endloop", "ccall", "ret", "cjmp", "alloc", "cexec_pred_clean", "cexec_pred_clean_e", "vsfetchdone"]
C = "xyzw"


def swz(s, n=4):
    return "".join(C[(i + ((s >> (2 * i)) & 3)) & 3] for i in range(n))


def mask(m):
    return "".join(C[i] for i in range(4) if m >> i & 1)


class Prog:
    def __init__(self, name, params, blob, is_ps):
        self.name, self.params, self.blob, self.is_ps = name, params, blob, is_ps
        self.reg = {}  # register -> name
        for (n, ty, r) in params:
            self.reg[(ty, r)] = n
        self.lit = {}
        self.parse()

    def parse(self):
        b = self.blob
        u = lambda o: struct.unpack_from(">I", b, o)[0]
        self.code_off0 = u(4)
        defs, sec = u(0x14), u(0x18)
        # literal constant definitions: {?, count?...} then {offset, reg|count}
        lit_size, code_size = u(sec), u(sec + 4)
        self.code_off = self.code_off0 + lit_size
        self.code_size = code_size
        self.litdef = []
        # plain reading: the literal block is lit_size bytes of float4s for the registers of the last definition
        self.litvals = [struct.unpack_from(">4f", b, self.code_off0 + k * 16) for k in range(lit_size // 16)]

    def cname(self, idx):
        # pixel shader constants live at 256.. in the combined space
        for (ty, r), n in self.reg.items():
            if ty == 2 and r == idx:
                return n
        if self.litvals:
            first = 256 - len(self.litvals)
            if idx >= first and idx < 256:
                v = self.litvals[idx - first]
                return "(" + ", ".join("%g" % x for x in v) + ")"
        return "c%d" % idx

    def src(self, reg, sel, sw, neg, n=4, absc=False):
        if sel:
            s = "r%d" % (reg & 0x3f)
            if reg & 0x80:
                s = "abs(%s)" % s
        else:
            s = self.cname(reg)
            if absc:
                s = "abs(%s)" % s
        sws = swz(sw, n)
        if not (n == 4 and sws == "xyzw"):
            s += "." + sws
        return ("-" if neg else "") + s

    def dis(self):
        b = self.blob
        out = []
        code = [struct.unpack_from(">3I", b, self.code_off + i * 12) for i in range(self.code_size // 12)]
        # control flow
        execs = []
        i = 0
        done = False
        while i < len(code) and not done:
            d0, d1, d2 = code[i]
            for (lo, hi) in ((d0, d1 & 0xffff), ((d1 >> 16) | ((d2 & 0xffff) << 16), d2 >> 16)):
                op = (hi >> 12) & 0xf
                addr, cnt, ser = lo & 0xfff, (lo >> 12) & 7, (lo >> 16) & 0xfff
                if CFOPS[op] in ("exec", "exece", "cexec", "cexece", "cexec_pred", "cexec_pred_e", "cexec_pred_clean", "cexec_pred_clean_e"):
                    execs.append((CFOPS[op], addr, cnt, ser, hi))
                    if CFOPS[op].endswith("e"):
                        done = True
                elif CFOPS[op] not in ("nop", "alloc"):
                    out.append("  cf %s lo=%08x hi=%04x" % (CFOPS[op], lo, hi))
                if execs and min(e[1] for e in execs) <= i + 1:
                    pass
            i += 1
            if execs and i >= min(e[1] for e in execs):
                break
        for (name, addr, cnt, ser, hi) in execs:
            if "pred" in name or name.startswith("cexec"):
                out.append("  %s (cond %04x):" % (name, hi))
            for k in range(cnt):
                if addr + k >= len(code):
                    break
                d0, d1, d2 = code[addr + k]
                is_fetch = (ser >> (2 * k)) & 1
                out.append("  %3d  " % (addr + k) + (self.fetch(d0, d1, d2) if is_fetch else self.alu(d0, d1, d2)))
        return out

    def fetch(self, d0, d1, d2):
        op = d0 & 0x1f
        src, dst = (d0 >> 5) & 0x3f, (d0 >> 12) & 0x3f
        if op == 1:
            ci = (d0 >> 20) & 0x1f
            ssw = (d0 >> 26) & 0x3f
            dsw = d1 & 0xfff
            ds = "".join("xyzw01?_"[(dsw >> (3 * i)) & 7] for i in range(4))
            name = self.reg.get((6, ci), "s%d" % ci)
            pred = ""
            if (d1 >> 31) & 1:
                pred = " (if %sp)" % ("" if d2 >> 31 else "!")
            # texel offsets (5 bits each, signed, in half texels) for multi-tap filters
            sx = lambda v: ((v & 0x1f) ^ 0x10) - 0x10
            ox, oy = sx(d2 >> 16) / 2.0, sx(d2 >> 21) / 2.0
            off = " +(%g,%g)" % (ox, oy) if (ox or oy) else ""
            # how the mip level is chosen: computed (the default), from the register set by
            # setTexLOD, or from the gradients set by setGradientH / V; and the level's bias
            lod = ""
            if not (d1 >> 28) & 1:
                lod += " nolod"
            if (d1 >> 29) & 1:
                lod += " reglod"
            if d2 & 1:
                lod += " reggrad"
            bias = (((d2 >> 2) & 0x7f) ^ 0x40) - 0x40
            if bias:
                lod += " bias %g" % (bias / 16.0)
            return "r%d.%s = tex(%s, r%d.%s%s%s)%s" % (dst, ds, name, src, self.sw3(ssw), off, lod, pred)
        if op == 0:
            return "vfetch r%d <- stream (d0=%08x d1=%08x d2=%08x)" % (dst, d0, d1, d2)
        ssw = (d0 >> 26) & 0x3f
        ds = "".join("xyzw01?_"[((d1 & 0xfff) >> (3 * i)) & 7] for i in range(4))
        names = {16: "getBCF", 17: "getCompTexLOD", 18: "getGradients", 19: "getWeights", 24: "setTexLOD", 25: "setGradientH", 26: "setGradientV"}
        if op in (24, 25, 26):
            return "%s(r%d.%s)" % (names[op], src, self.sw3(ssw))
        # getGradients: x = d/dx of the first component, y = d/dy of it, z, w = the same of the second
        return "r%d.%s = %s(r%d.%s)" % (dst, ds, names.get(op, "fetchop%d" % op), src, self.sw3(ssw))

    @staticmethod
    def sw3(s):
        # (a fetch's source swizzle is absolute: two bits a component)
        return "".join(C[(s >> (2 * i)) & 3] for i in range(3))

    def alu(self, d0, d1, d2):
        vdst, absc, sdst, export = d0 & 0x3f, (d0 >> 7) & 1, (d0 >> 8) & 0x3f, (d0 >> 15) & 1
        vmask, smask = (d0 >> 16) & 0xf, (d0 >> 20) & 0xf
        vclamp, sclamp, sop = (d0 >> 24) & 1, (d0 >> 25) & 1, (d0 >> 26) & 0x3f
        s3sw, s2sw, s1sw = d1 & 0xff, (d1 >> 8) & 0xff, (d1 >> 16) & 0xff
        n3, n2, n1 = (d1 >> 24) & 1, (d1 >> 25) & 1, (d1 >> 26) & 1
        predc, pred = (d1 >> 27) & 1, (d1 >> 28) & 1
        r3, r2, r1 = d2 & 0xff, (d2 >> 8) & 0xff, (d2 >> 16) & 0xff
        vop, sel3, sel2, sel1 = (d2 >> 24) & 0x1f, (d2 >> 29) & 1, (d2 >> 30) & 1, (d2 >> 31) & 1
        parts = []
        dn = lambda r: ("o%d" % r if self.is_ps else {62: "oPos", 63: "oPts"}.get(r, "o%d" % r)) if export else "r%d" % r
        if vmask:
            name = VOPS[vop] if vop < len(VOPS) else "v?%d" % vop
            a = [self.src(r1, sel1, s1sw, n1, 4, absc), self.src(r2, sel2, s2sw, n2, 4, absc), self.src(r3, sel3, s3sw, n3, 4, absc)][: VARGS.get(name, 2)]
            parts.append("%s.%s = %s%s(%s)" % (dn(vdst), mask(vmask), "sat " if vclamp else "", name, ", ".join(a)))
        # (a scalar operation without a destination still sets `ps`, the value *_prev operations read)
        if smask or sop != 50:
            name = SOPS[sop] if sop < len(SOPS) else "s?%d" % sop
            # scalar operands come out of src3: a = the w slot, b = the x slot
            ca = C[(3 + ((s3sw >> 6) & 3)) & 3]
            cb = C[(0 + (s3sw & 3)) & 3]
            base3 = self.src(r3, sel3, 0, n3, 4, absc)
            base3 = base3 if not base3.endswith(".xyzw") else base3
            if name in ("mulsc0", "mulsc1", "addsc0", "addsc1", "subsc0", "subsc1"):
                # constant (src3) with a register (src1 index in the low bits of the opcode pair)
                # (the register: the opcode's low bit, bits 2..5 of the third swizzle, its select bit)
                # (the operand's negate and absolute flags apply to BOTH, constant and register:
                # a negated subsc is register - constant)
                sg = "-" if n3 else ""
                rn = "r%d" % ((sop & 1) | (s3sw & 0x3C) | (sel3 << 1))
                if absc:
                    a = "%sabs(%s).%s, %sabs(%s).%s" % (sg, self.cname(r3), ca, sg, rn, cb)
                else:
                    a = "%s%s.%s, %s%s.%s" % (sg, self.cname(r3), ca, sg, rn, cb)
            elif name in ("adds", "muls", "maxs", "mins", "subs", "seqs", "sgts", "sges", "snes", "maxas", "maxasf"):
                a = "%s.%s, %s.%s" % (base3, ca, base3, cb) if name in ("adds", "muls", "maxs", "mins", "subs", "maxas", "maxasf") else "%s.%s" % (base3, ca)
            elif name in ("retain_prev", "setp_pop", "setp_clr", "setp_rstr"):
                a = ""
            else:
                a = "%s.%s" % (base3, ca)
            parts.append("%s.%s = %s%s(%s)" % (dn(sdst if not export else vdst), mask(smask) or "-", "sat " if sclamp else "", name, a))
        s = " ; ".join(parts)
        if pred:
            s += "   (if %sp)" % ("" if predc else "!")
        return s


def programs(path):
    d = open(path, "rb").read()
    progs = []
    # each program: u8 name length, name, params {u8 type? ...}, then the blob; find blobs and walk back
    for m in re.finditer(re.escape(bytes([0x10, 0x2A, 0x11])) + b"[\x00\x01]", d):
        o = m.start()
        is_ps = d[o + 3] == 0
        hdr = struct.unpack_from(">7I", d, o)
        total = hdr[1] + 0  # code offset
        sec = hdr[6]
        if sec > 0x4000 or hdr[1] > 0x4000 or o + sec + 8 > len(d):
            continue
        lit_size, code_size = struct.unpack_from(">II", d, o + sec)
        size = hdr[1] + lit_size + code_size
        # the parameter list just before the blob: [len][name]\0 [type u8][0][reg u8]... scan back for the name
        back = d[max(0, o - 1500):o]
        names = list(re.finditer(rb"([\x02-\x40])((?:VS|PS|vs|ps)[A-Za-z0-9_]{2,60})\x00", back))
        name = names[-1].group(2).decode() if names else "?"
        params = []
        if names:
            p = names[-1].end()
            seg = back[p:]
            for pm in re.finditer(rb"([\x00-\x10])\x00([\x00-\xff])\x00([\x02-\x40])([A-Za-z_][A-Za-z0-9_]{1,60})\x00", seg):
                params.append((pm.group(4).decode(), pm.group(1)[0], pm.group(2)[0]))
        progs.append(Prog(name, params, d[o:o + size + 64], is_ps))
    return progs


if __name__ == "__main__":
    ps = programs(sys.argv[1])
    want = sys.argv[2].lower() if len(sys.argv) > 2 else None
    for p in ps:
        print("%s %s  consts %s" % ("PS" if p.is_ps else "VS", p.name, " ".join("%s=%s%d" % (n, "s" if ty == 6 else "c", r) for n, ty, r in p.params)))
        if want and want in p.name.lower():
            print("  literals:", p.litvals)
            for l in p.dis():
                print(l)
