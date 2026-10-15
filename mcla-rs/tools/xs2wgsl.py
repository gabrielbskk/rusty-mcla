"""Xenos shader microcode -> WGSL, instruction by instruction, for the game's rgxa effect files.

REFERENCE IMPLEMENTATION. The install pipeline does this in Rust (crates/shaders, `mcla-tool shaders-wgsl <data cache folder>`), on the
player's machine, from the player's own disc; the generated WGSL is not kept in the repository. The Rust output is byte-identical to this
script's (apart from the first comment line); this file (with xsh.py and xsrun.py) stays as the readable description of the rules.

    xs2wgsl.py <fxl file> <program name> [function name]      prints the WGSL of one program

The output is a function over a register machine - nothing is simplified, so it computes what
the microcode computes (semantics after the ReXGlue SDK's `ucode.h`):

    fn <name>(rin: array<vec4<f32>, 16>) -> vec4<f32>                      pixel programs (colour 0)
    fn <name>(vin: array<vec4<f32>, 16>) -> XsVsOut { pos, o: array<vec4<f32>, 16> }   vertex programs

What the including shader has to supply:
    var<private> xs_c: array<vec4<f32>, 256>;      the float constants the program reads (its own
                                                    bank: pixel constants for a pixel program)
    fn xs_s<N>(uv: vec2<f32>) -> vec4<f32>          sampler N, level chosen by the hardware rule
    fn xs_s<N>_grad(uv, ddx: vec2<f32>, ddy: vec2<f32>)   ... with the registered gradients
    fn xs_s<N>_lod(uv, lod: f32)                    ... with the registered level
    fn xs_s<N>_off(uv, texels: vec2<f32>)           ... with an offset in texels
(only the variants a program uses are called; `xs_uses(prog)` lists them). Vertex fetches arrive
in `vin` in the order the program makes them.

Not handled (the translator stops with an error): loops, calls, conditional execution on bool
constants, memory export, relative addressing (a0 / aL).
"""
import struct, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import xsh

C = "xyzw"
VOPS = xsh.VOPS
SOPS = xsh.SOPS


def f(x):
    """a float literal WGSL accepts"""
    if x != x:
        return "0.0"
    s = repr(float(x))
    if "e" in s or "E" in s:
        return s
    return s if "." in s else s + ".0"


class Emit:
    def __init__(self, prog):
        self.p = prog
        self.lines = []
        self.uses = set()       # sampler function names called
        self.consts = set()     # constant registers read
        self.nfetch = 0

    def const(self, idx):
        first = 256 - len(self.p.litvals)
        if self.p.litvals and first <= idx < 256:
            v = self.p.litvals[idx - first]
            return "vec4<f32>(%s)" % ", ".join(f(x) for x in v)
        self.consts.add(idx)
        return "xs_c[%d]" % idx

    def src(self, reg, sel, sw, neg, absc):
        if sel:
            s = "r[%d]" % (reg & 0x3f)
            if reg & 0x80:
                s = "abs(%s)" % s
        else:
            s = self.const(reg)
            if absc:
                s = "abs(%s)" % s
        swz = "".join(C[(i + ((sw >> (2 * i)) & 3)) & 3] for i in range(4))
        if swz != "xyzw":
            s = "(%s).%s" % (s, swz)
        return "(-%s)" % s if neg else s

    def write(self, dest, mask, expr, clamp):
        """lines writing the components of `mask` of the vec4 expression into the destination"""
        out = []
        for i in range(4):
            if mask >> i & 1:
                e = "%s.%s" % (expr, C[i])
                if clamp:
                    e = "clamp(%s, 0.0, 1.0)" % e
                out.append("%s.%s = %s;" % (dest, C[i], e))
        return out

    def dest(self, reg, export):
        if not export:
            return "r[%d]" % reg
        if self.p.is_ps:
            if reg > 3:
                raise SystemExit("pixel program exports to register %d (depth?): not handled" % reg)
            return "o[%d]" % reg
        if reg == 62:
            return "opos"
        if reg == 63:
            return "opts"
        if reg > 15:
            raise SystemExit("vertex program exports to register %d: not handled" % reg)
        return "o[%d]" % reg

    def alu(self, d0, d1, d2, cond):
        vdst, absc, sdst, export = d0 & 0x3f, (d0 >> 7) & 1, (d0 >> 8) & 0x3f, (d0 >> 15) & 1
        sdst_rel = (d0 >> 14) & 1
        if (d0 >> 6) & 1 and not export:
            raise SystemExit("relative vector destination: not handled")
        vmask, smask = (d0 >> 16) & 0xf, (d0 >> 20) & 0xf
        vclamp, sclamp, sop = (d0 >> 24) & 1, (d0 >> 25) & 1, (d0 >> 26) & 0x3f
        s3sw, s2sw, s1sw = d1 & 0xff, (d1 >> 8) & 0xff, (d1 >> 16) & 0xff
        n3, n2, n1 = (d1 >> 24) & 1, (d1 >> 25) & 1, (d1 >> 26) & 1
        predc, pred = (d1 >> 27) & 1, (d1 >> 28) & 1
        if (d1 >> 29) & 1:
            raise SystemExit("register-relative constant addressing: not handled")
        r3, r2, r1 = d2 & 0xff, (d2 >> 8) & 0xff, (d2 >> 16) & 0xff
        vop, sel3, sel2, sel1 = (d2 >> 24) & 0x1f, (d2 >> 29) & 1, (d2 >> 30) & 1, (d2 >> 31) & 1
        vname = VOPS[vop] if vop < len(VOPS) else "?"
        sname = SOPS[sop] if sop < len(SOPS) else "?"
        if pred:
            c = "p" if predc else "!p"
            cond = c if cond is None else "(%s) && %s" % (cond, c)
        L = []
        vmask_w, smask_w = vmask, smask
        if export:
            vmask_w, smask_w = vmask & ~smask, smask & ~vmask
        need_v = vmask_w != 0 or vname.startswith(("kill", "setp"))
        need_s = sname != "retain_prev" or smask_w != 0
        # --- the vector operation
        side = []   # statements with side effects inside the condition
        if need_v:
            nargs = xsh.VARGS.get(vname, 2)
            a = self.src(r1, sel1, s1sw, n1, absc)
            b = self.src(r2, sel2, s2sw, n2, absc) if nargs > 1 else None
            c3 = self.src(r3, sel3, s3sw, n3, absc) if nargs > 2 else None
            one, zero = "vec4<f32>(1.0)", "vec4<f32>(0.0)"
            e = {
                "add": lambda: "%s + %s" % (a, b), "mul": lambda: "%s * %s" % (a, b),
                "max": lambda: "max(%s, %s)" % (a, b), "min": lambda: "min(%s, %s)" % (a, b),
                "seq": lambda: "select(%s, %s, %s == %s)" % (zero, one, a, b),
                "sgt": lambda: "select(%s, %s, %s > %s)" % (zero, one, a, b),
                "sge": lambda: "select(%s, %s, %s >= %s)" % (zero, one, a, b),
                "sne": lambda: "select(%s, %s, %s != %s)" % (zero, one, a, b),
                "frc": lambda: "(%s - floor(%s))" % (a, a), "trunc": lambda: "trunc(%s)" % a, "floor": lambda: "floor(%s)" % a,
                "mad": lambda: "%s * %s + %s" % (a, b, c3),
                "cndeq": lambda: "select(%s, %s, %s == %s)" % (c3, b, a, zero),
                "cndge": lambda: "select(%s, %s, %s >= %s)" % (c3, b, a, zero),
                "cndgt": lambda: "select(%s, %s, %s > %s)" % (c3, b, a, zero),
                "dp4": lambda: "vec4<f32>(dot(%s, %s))" % (a, b),
                "dp3": lambda: "vec4<f32>(dot((%s).xyz, (%s).xyz))" % (a, b),
                "dp2add": lambda: "vec4<f32>(dot((%s).xy, (%s).xy) + (%s).x)" % (a, b, c3),
                "max4": lambda: "vec4<f32>(max(max((%s).x, (%s).y), max((%s).z, (%s).w)))" % (a, a, a, a),
                "dst": lambda: "vec4<f32>(1.0, (%s).y * (%s).y, (%s).z, (%s).w)" % (a, b, a, b),
                "maxa": lambda: "max(%s, %s)" % (a, b),
            }
            if vname in e:
                L.append("let v = %s;" % e[vname]())
            elif vname.startswith("kill_"):
                op = {"kill_eq": "==", "kill_gt": ">", "kill_ge": ">=", "kill_ne": "!="}[vname]
                L.append("let vk = any(%s %s %s);" % (a, op, b))
                L.append("let v = vec4<f32>(select(0.0, 1.0, vk));")
                side.append("if (vk) { discard; }")
            else:
                raise SystemExit("vector operation %s: not handled" % vname)
        # --- the scalar operation
        if need_s:
            ia, ib = (3 + ((s3sw >> 6) & 3)) & 3, (s3sw & 3) & 3
            base = self.src(r3, sel3, 0, 0, absc)   # (negation applied below; the swizzle is taken by hand)
            sg = "-" if n3 else ""
            sa, sb = "(%s%s.%s)" % (sg, base, C[ia]), "(%s%s.%s)" % (sg, base, C[ib])
            if sname in ("mulsc0", "mulsc1", "addsc0", "addsc1", "subsc0", "subsc1"):
                # a constant (operand 3) with a register named by the opcode's low bit, bits 2..5 of
                # the third swizzle and its select bit
                reg = (sop & 1) | (s3sw & 0x3C) | (sel3 << 1)
                # (the operand's negate and absolute flags apply to BOTH, the constant and the
                # register - the reference translator sets them on either: a negated subsc is
                # register - constant)
                ka = "(%s%s.%s)" % (sg, self.const(r3) if not absc else "abs(%s)" % self.const(r3), C[ia])
                rb = "(%s%s.%s)" % (sg, ("r[%d]" % reg) if not absc else "abs(r[%d])" % reg, C[ib])
                L.append("let s = %s;" % {"mul": "%s * %s", "add": "%s + %s", "sub": "%s - %s"}[sname[:3]] % (ka, rb))
            else:
                T = {
                    "adds": "%s + %s" % (sa, sb), "adds_prev": "%s + ps" % sa, "muls": "%s * %s" % (sa, sb), "muls_prev": "%s * ps" % sa,
                    "maxs": "max(%s, %s)" % (sa, sb), "mins": "min(%s, %s)" % (sa, sb),
                    "seqs": "select(0.0, 1.0, %s == 0.0)" % sa, "sgts": "select(0.0, 1.0, %s > 0.0)" % sa,
                    "sges": "select(0.0, 1.0, %s >= 0.0)" % sa, "snes": "select(0.0, 1.0, %s != 0.0)" % sa,
                    "frcs": "(%s - floor(%s))" % (sa, sa), "truncs": "trunc(%s)" % sa, "floors": "floor(%s)" % sa,
                    "exp": "exp2(%s)" % sa, "logc": "max(log2(%s), -3.4e38)" % sa, "log": "log2(%s)" % sa,
                    "rcpc": "(1.0 / %s)" % sa, "rcpf": "(1.0 / %s)" % sa, "rcp": "(1.0 / %s)" % sa,
                    "rsqc": "inverseSqrt(%s)" % sa, "rsqf": "inverseSqrt(%s)" % sa, "rsq": "inverseSqrt(%s)" % sa,
                    "maxas": "max(%s, %s)" % (sa, sb), "maxasf": "max(%s, %s)" % (sa, sb),
                    "subs": "%s - %s" % (sa, sb), "subs_prev": "%s - ps" % sa, "sqrt": "sqrt(%s)" % sa,
                    "sin": "sin(%s)" % sa, "cos": "cos(%s)" % sa, "retain_prev": "ps",
                    "muls_prev2": "select(%s * ps, -3.4e38, ps == -3.4e38 || %s <= 0.0)" % (sa, sb),
                }
                P = {"setp_eq": "%s == 0.0", "setp_ne": "%s != 0.0", "setp_gt": "%s > 0.0", "setp_ge": "%s >= 0.0"}
                K = {"kills_eq": "%s == 0.0", "kills_gt": "%s > 0.0", "kills_ge": "%s >= 0.0", "kills_ne": "%s != 0.0", "kills_one": "%s == 1.0"}
                if sname in T:
                    L.append("let s = %s;" % T[sname])
                elif sname in P:
                    L.append("let sp = %s;" % (P[sname] % sa))
                    L.append("let s = select(1.0, 0.0, sp);")
                    side.append("p = sp;")
                elif sname in K:
                    L.append("let sk = %s;" % (K[sname] % sa))
                    L.append("let s = select(0.0, 1.0, sk);")
                    side.append("if (sk) { discard; }")
                else:
                    raise SystemExit("scalar operation %s: not handled" % sname)
        # --- the writes (after both operations have read their operands)
        W = []
        if need_v and vmask_w:
            W += self.write(self.dest(vdst, export), vmask_w, "v", vclamp)
        if need_s:
            if smask_w:
                W += self.write(self.dest(vdst if export else sdst, export), smask_w, "vec4<f32>(s)", sclamp)
            if sname != "retain_prev":
                W.append("ps = s;")
        if export:
            one = vmask & smask
            zero = (0xf & ~(vmask | smask)) if sdst_rel else 0
            for i in range(4):
                if one >> i & 1:
                    W.append("%s.%s = 1.0;" % (self.dest(vdst, True), C[i]))
                if zero >> i & 1:
                    W.append("%s.%s = 0.0;" % (self.dest(vdst, True), C[i]))
        W += side
        return L, W, cond

    def fetch(self, d0, d1, d2, cond):
        op = d0 & 0x1f
        src, dst = (d0 >> 5) & 0x3f, (d0 >> 12) & 0x3f
        ssw = (d0 >> 26) & 0x3f
        sw3 = "".join(C[(ssw >> (2 * i)) & 3] for i in range(3))
        dsw = d1 & 0xfff
        if (d1 >> 31) & 1:
            c = "p" if d2 >> 31 else "!p"
            cond = c if cond is None else "(%s) && %s" % (cond, c)
        L, W = [], []

        def store(expr):
            for i in range(4):
                k = (dsw >> (3 * i)) & 7
                if k < 4:
                    W.append("r[%d].%s = %s.%s;" % (dst, C[i], expr, C[k]))
                elif k == 4:
                    W.append("r[%d].%s = 0.0;" % (dst, C[i]))
                elif k == 5:
                    W.append("r[%d].%s = 1.0;" % (dst, C[i]))
        if op == 0:
            # (a vertex fetch: mini fetches that continue the previous one are not told apart here)
            L.append("let t = vin[%d];" % self.nfetch)
            self.nfetch += 1
            store("t")
        elif op == 1:
            n = (d0 >> 20) & 0x1f
            if (d0 >> 25) & 1:
                raise SystemExit("unnormalized texture coordinates: not handled")
            sx = lambda v: ((v & 0x1f) ^ 0x10) - 0x10
            ox, oy = sx(d2 >> 16) / 2.0, sx(d2 >> 21) / 2.0
            uv = "r[%d].%s" % (src, sw3[:2])
            if d2 & 1:
                fn = "xs_s%d_grad" % n
                call = "%s(%s, gh.xy, gv.xy)" % (fn, uv)
            elif (d1 >> 29) & 1:
                fn = "xs_s%d_lod" % n
                call = "%s(%s, lod)" % (fn, uv)
            elif ox or oy:
                fn = "xs_s%d_off" % n
                call = "%s(%s, vec2<f32>(%s, %s))" % (fn, uv, f(ox), f(oy))
            else:
                fn = "xs_s%d" % n
                call = "%s(%s)" % (fn, uv)
            self.uses.add(fn)
            L.append("let t = %s;" % call)
            store("t")
        elif op == 18:
            # getGradients: x = d/dx of the first component, y = d/dy of it, z, w = those of the second
            L.append("let gs = r[%d].%s;" % (src, sw3[:2]))
            L.append("let t = vec4<f32>(dpdx(gs.x), dpdy(gs.x), dpdx(gs.y), dpdy(gs.y));")
            store("t")
        elif op == 24:
            W.append("lod = r[%d].%s;" % (src, sw3[0]))
        elif op == 25:
            W.append("gh = r[%d].%s;" % (src, sw3))
        elif op == 26:
            W.append("gv = r[%d].%s;" % (src, sw3))
        else:
            raise SystemExit("fetch operation %d: not handled" % op)
        return L, W, cond

    def run(self, name):
        p = self.p
        b = p.blob
        code = [struct.unpack_from(">3I", b, p.code_off + i * 12) for i in range(p.code_size // 12)]
        execs, i, done = [], 0, False
        while i < len(code) and not done:
            d0, d1, d2 = code[i]
            for lo, hi in ((d0, d1 & 0xffff), ((d1 >> 16) | ((d2 & 0xffff) << 16), d2 >> 16)):
                cf = xsh.CFOPS[(hi >> 12) & 0xf]
                addr, cnt, ser = lo & 0xfff, (lo >> 12) & 7, (lo >> 16) & 0xfff
                if cf in ("exec", "exece", "cexec_pred", "cexec_pred_e", "cexec_pred_clean", "cexec_pred_clean_e"):
                    cond = None
                    if "pred" in cf:
                        cond = "p" if (hi >> 10) & 1 else "!p"
                    execs.append((addr, cnt, ser, cond))
                    if cf.endswith("e"):
                        done = True
                elif cf not in ("nop", "alloc"):
                    raise SystemExit("control flow %s: not handled" % cf)
            i += 1
            if execs and i >= min(e[0] for e in execs):
                break
        body = []
        for addr, cnt, ser, bcond in execs:
            if bcond is not None:
                # (the block's condition is taken at its entry: instructions in it may change p)
                body.append("    pb = %s;" % bcond)
            for k in range(cnt):
                d0, d1, d2 = code[addr + k]
                is_fetch = (ser >> (2 * k)) & 1
                L, W, cond = (self.fetch if is_fetch else self.alu)(d0, d1, d2, "pb" if bcond is not None else None)
                body.append("    { // %d" % (addr + k))
                body += ["        " + x for x in L]
                if cond is None:
                    body += ["        " + x for x in W]
                elif W:
                    body.append("        if (%s) {" % cond)
                    body += ["            " + x for x in W]
                    body.append("        }")
                body.append("    }")
        head = []
        if p.is_ps:
            head.append("fn %s(rin: array<vec4<f32>, 16>) -> vec4<f32> {" % name)
            head.append("    var r: array<vec4<f32>, 64>;")
            head.append("    for (var i = 0; i < 16; i = i + 1) { r[i] = rin[i]; }")
            head.append("    var o: array<vec4<f32>, 4>;")
        else:
            head.append("fn %s(vin: array<vec4<f32>, 16>) -> XsVsOut {" % name)
            head.append("    var r: array<vec4<f32>, 64>;")
            head.append("    var o: array<vec4<f32>, 16>;")
            head.append("    var opos = vec4<f32>(0.0);")
            head.append("    var opts = vec4<f32>(0.0);")
        head += ["    var ps = 0.0;", "    var p = false;", "    var pb = false;", "    var lod = 0.0;", "    var gh = vec3<f32>(0.0);", "    var gv = vec3<f32>(0.0);"]
        tail = ["    return o[0];", "}"] if p.is_ps else ["    return XsVsOut(opos, o);", "}"]
        return "\n".join(head + body + tail) + "\n"


def translate(path, prog_name, fn_name=None):
    """(wgsl, samplers used, constants read) of one program of an effect file"""
    for p in xsh.programs(path):
        if p.name == prog_name:
            e = Emit(p)
            src = e.run(fn_name or "xs_" + prog_name)
            return src, sorted(e.uses), sorted(e.consts)
    raise SystemExit("no program %s in %s" % (prog_name, path))


if __name__ == "__main__":
    src, uses, consts = translate(sys.argv[1], sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else None)
    print("// generated by tools/xs2wgsl.py from %s, program %s" % (os.path.basename(sys.argv[1]), sys.argv[2]))
    print("// samplers: %s" % " ".join(uses))
    print("// constants: %s" % " ".join("c%d" % c for c in consts))
    print(src)
