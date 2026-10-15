//! Xenos (Xbox 360) shader microcode of the game's `rgxa` effect files -> WGSL, instruction by instruction.
//!
//! The program finder and the translator follow `tools/xsh.py` / `tools/xs2wgsl.py` (kept as the readable reference; the output
//! of this module is byte-identical to theirs). The output is a function over a register machine - nothing is simplified, so it
//! computes what the microcode computes (semantics after the ReXGlue SDK's `ucode.h`):
//!
//! ```text
//! fn <name>(rin: array<vec4<f32>, 16>) -> vec4<f32>                      pixel programs (colour 0)
//! fn <name>(vin: array<vec4<f32>, 16>) -> XsVsOut { pos, o: array<vec4<f32>, 16> }   vertex programs
//! ```
//!
//! The including shader supplies `xs_c`, the sampler functions `xs_s<N>`, `_grad`, `_lod`, `_off`, and the `XsVsOut` struct.
//! Not handled (an error): loops, calls, conditional execution on bool constants, memory export, relative addressing.

use regex::bytes::Regex;
use std::collections::BTreeSet;

const C: [char; 4] = ['x', 'y', 'z', 'w'];

const VOPS: [&str; 30] = [
    "add", "mul", "max", "min", "seq", "sgt", "sge", "sne", "frc", "trunc", "floor", "mad", "cndeq", "cndge", "cndgt", "dp4", "dp3", "dp2add", "cube", "max4", "setp_eq_push", "setp_ne_push",
    "setp_gt_push", "setp_ge_push", "kill_eq", "kill_gt", "kill_ge", "kill_ne", "dst", "maxa",
];
const SOPS: [&str; 51] = [
    "adds", "adds_prev", "muls", "muls_prev", "muls_prev2", "maxs", "mins", "seqs", "sgts", "sges", "snes", "frcs", "truncs", "floors", "exp", "logc", "log", "rcpc", "rcpf", "rcp", "rsqc", "rsqf",
    "rsq", "maxas", "maxasf", "subs", "subs_prev", "setp_eq", "setp_ne", "setp_gt", "setp_ge", "setp_inv", "setp_pop", "setp_clr", "setp_rstr", "kills_eq", "kills_gt", "kills_ge", "kills_ne",
    "kills_one", "sqrt", "?41", "mulsc0", "mulsc1", "addsc0", "addsc1", "subsc0", "subsc1", "sin", "cos", "retain_prev",
];
const CFOPS: [&str; 16] = ["nop", "exec", "exece", "cexec", "cexece", "cexec_pred", "cexec_pred_e", "loop", "endloop", "ccall", "ret", "cjmp", "alloc", "cexec_pred_clean", "cexec_pred_clean_e", "vsfetchdone"];

fn sop_name(i: u32) -> &'static str {
    SOPS.get(i as usize).copied().unwrap_or("?")
}

fn vop_name(i: u32) -> &'static str {
    VOPS.get(i as usize).copied().unwrap_or("?")
}

fn vargs(name: &str) -> u32 {
    match name {
        "frc" | "trunc" | "floor" | "max4" => 1,
        "mad" | "cndeq" | "cndge" | "cndgt" | "dp2add" => 3,
        _ => 2,
    }
}

/// A program the translator cannot handle, or data that is not a shader effect.
#[derive(Debug)]
pub struct XsError(pub String);

impl std::fmt::Display for XsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for XsError {}

type Res<T> = Result<T, XsError>;

fn err<T>(s: impl Into<String>) -> Res<T> {
    Err(XsError(s.into()))
}

/// One shader program of an effect file.
#[derive(Debug, Clone)]
pub struct Prog {
    pub name: String,
    pub is_ps: bool,
    blob: Vec<u8>,
    code_off: usize,
    code_size: usize,
    litvals: Vec<[f32; 4]>,
}

fn be32(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

impl Prog {
    fn parse(name: String, blob: Vec<u8>, is_ps: bool) -> Option<Prog> {
        let code_off0 = be32(&blob, 4)? as usize;
        let sec = be32(&blob, 0x18)? as usize;
        let lit_size = be32(&blob, sec)? as usize;
        let code_size = be32(&blob, sec + 4)? as usize;
        let mut litvals = Vec::new();
        for k in 0..lit_size / 16 {
            let mut v = [0f32; 4];
            for (j, x) in v.iter_mut().enumerate() {
                *x = f32::from_bits(be32(&blob, code_off0 + k * 16 + j * 4)?);
            }
            litvals.push(v);
        }
        Some(Prog { name, is_ps, blob, code_off: code_off0 + lit_size, code_size, litvals })
    }
}

/// Every program of an effect file (`xsh.programs`): the code blobs start with `10 2A 11 0|1`; the name sits in the parameter table before them.
pub fn programs(d: &[u8]) -> Vec<Prog> {
    let mut progs = Vec::new();
    let name_re = Regex::new(r"(?-u)([\x02-\x40])((?:VS|PS|vs|ps)[A-Za-z0-9_]{2,60})\x00").expect("regex");
    let mut o = 0usize;
    while o + 4 <= d.len() {
        if !(d[o] == 0x10 && d[o + 1] == 0x2A && d[o + 2] == 0x11 && (d[o + 3] == 0 || d[o + 3] == 1)) {
            o += 1;
            continue;
        }
        let here = o;
        o += 4; // (the pattern cannot overlap itself, so the next match starts after it)
        let is_ps = d[here + 3] == 0;
        let (Some(h1), Some(sec)) = (be32(d, here + 4), be32(d, here + 24)) else { continue };
        let (h1, sec) = (h1 as usize, sec as usize);
        if sec > 0x4000 || h1 > 0x4000 || here + sec + 8 > d.len() {
            continue;
        }
        let (Some(lit_size), Some(code_size)) = (be32(d, here + sec), be32(d, here + sec + 4)) else { continue };
        let size = h1 + lit_size as usize + code_size as usize;
        let back = &d[here.saturating_sub(1500)..here];
        let name = name_re.captures_iter(back).last().map(|c| String::from_utf8_lossy(&c[2]).into_owned()).unwrap_or_else(|| "?".into());
        let end = (here + size + 64).min(d.len());
        if let Some(p) = Prog::parse(name, d[here..end].to_vec(), is_ps) {
            progs.push(p);
        }
    }
    progs
}

/// A float as Python's `repr(float(x))` writes it (shortest round-trip digits; exponent form below 1e-4 and from 1e16, `.0` added to whole numbers).
fn py_repr(x: f64) -> String {
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf".into() } else { "inf".into() };
    }
    let e = format!("{:e}", x.abs());
    let (mant, exp) = e.split_once('e').expect("LowerExp");
    let exp: i32 = exp.parse().expect("exponent");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let decpt = exp + 1;
    let sign = if x < 0.0 { "-" } else { "" };
    if decpt > 16 || decpt <= -4 {
        let rest = &digits[1..];
        let m = if rest.is_empty() { digits[..1].to_string() } else { format!("{}.{}", &digits[..1], rest) };
        let ex = decpt - 1;
        format!("{sign}{m}e{}{:02}", if ex < 0 { '-' } else { '+' }, ex.abs())
    } else if decpt <= 0 {
        format!("{sign}0.{}{}", "0".repeat((-decpt) as usize), digits)
    } else if decpt as usize >= digits.len() {
        format!("{sign}{}{}.0", digits, "0".repeat(decpt as usize - digits.len()))
    } else {
        format!("{sign}{}.{}", &digits[..decpt as usize], &digits[decpt as usize..])
    }
}

/// A float literal WGSL accepts (`xs2wgsl.f`).
fn f(x: f64) -> String {
    if x.is_nan() {
        return "0.0".into();
    }
    let s = py_repr(x);
    if s.contains('e') || s.contains('E') || s.contains('.') {
        s
    } else {
        s + ".0"
    }
}

struct Emit<'a> {
    p: &'a Prog,
    uses: BTreeSet<String>,
    consts: BTreeSet<i64>,
    nfetch: usize,
}

type Lines = Vec<String>;
type Part = (Lines, Lines, Option<String>);

impl<'a> Emit<'a> {
    fn new(p: &'a Prog) -> Self {
        Emit { p, uses: BTreeSet::new(), consts: BTreeSet::new(), nfetch: 0 }
    }

    fn constant(&mut self, idx: u32) -> String {
        let idx = idx as i64;
        let first = 256 - self.p.litvals.len() as i64;
        if !self.p.litvals.is_empty() && first <= idx && idx < 256 {
            let v = self.p.litvals[(idx - first) as usize];
            return format!("vec4<f32>({})", v.iter().map(|x| f(*x as f64)).collect::<Vec<_>>().join(", "));
        }
        self.consts.insert(idx);
        format!("xs_c[{idx}]")
    }

    fn src(&mut self, reg: u32, sel: u32, sw: u32, neg: u32, absc: u32) -> String {
        let mut s;
        if sel != 0 {
            s = format!("r[{}]", reg & 0x3f);
            if reg & 0x80 != 0 {
                s = format!("abs({s})");
            }
        } else {
            s = self.constant(reg);
            if absc != 0 {
                s = format!("abs({s})");
            }
        }
        let swz: String = (0..4).map(|i| C[((i + ((sw >> (2 * i)) & 3)) & 3) as usize]).collect();
        if swz != "xyzw" {
            s = format!("({s}).{swz}");
        }
        if neg != 0 {
            format!("(-{s})")
        } else {
            s
        }
    }

    fn write(dest: &str, mask: u32, expr: &str, clamp: bool) -> Lines {
        let mut out = Vec::new();
        for i in 0..4 {
            if mask >> i & 1 != 0 {
                let mut e = format!("{expr}.{}", C[i]);
                if clamp {
                    e = format!("clamp({e}, 0.0, 1.0)");
                }
                out.push(format!("{dest}.{} = {e};", C[i]));
            }
        }
        out
    }

    fn dest(&self, reg: u32, export: bool) -> Res<String> {
        if !export {
            return Ok(format!("r[{reg}]"));
        }
        if self.p.is_ps {
            if reg > 3 {
                return err(format!("pixel program exports to register {reg} (depth?): not handled"));
            }
            return Ok(format!("o[{reg}]"));
        }
        if reg == 62 {
            return Ok("opos".into());
        }
        if reg == 63 {
            return Ok("opts".into());
        }
        if reg > 15 {
            return err(format!("vertex program exports to register {reg}: not handled"));
        }
        Ok(format!("o[{reg}]"))
    }

    fn alu(&mut self, d0: u32, d1: u32, d2: u32, cond: Option<String>) -> Res<Part> {
        let (vdst, absc, sdst, export) = (d0 & 0x3f, (d0 >> 7) & 1, (d0 >> 8) & 0x3f, (d0 >> 15) & 1 != 0);
        let sdst_rel = (d0 >> 14) & 1;
        if (d0 >> 6) & 1 != 0 && !export {
            return err("relative vector destination: not handled");
        }
        let (vmask, smask) = ((d0 >> 16) & 0xf, (d0 >> 20) & 0xf);
        let (vclamp, sclamp, sop) = ((d0 >> 24) & 1 != 0, (d0 >> 25) & 1 != 0, (d0 >> 26) & 0x3f);
        let (s3sw, s2sw, s1sw) = (d1 & 0xff, (d1 >> 8) & 0xff, (d1 >> 16) & 0xff);
        let (n3, n2, n1) = ((d1 >> 24) & 1, (d1 >> 25) & 1, (d1 >> 26) & 1);
        let (predc, pred) = ((d1 >> 27) & 1, (d1 >> 28) & 1);
        if (d1 >> 29) & 1 != 0 {
            return err("register-relative constant addressing: not handled");
        }
        let (r3, r2, r1) = (d2 & 0xff, (d2 >> 8) & 0xff, (d2 >> 16) & 0xff);
        let (vop, sel3, sel2, sel1) = ((d2 >> 24) & 0x1f, (d2 >> 29) & 1, (d2 >> 30) & 1, (d2 >> 31) & 1);
        let vname = vop_name(vop);
        let sname = sop_name(sop);
        let mut cond = cond;
        if pred != 0 {
            let c = if predc != 0 { "p" } else { "!p" };
            cond = Some(match cond {
                None => c.to_string(),
                Some(k) => format!("({k}) && {c}"),
            });
        }
        let mut l: Lines = Vec::new();
        let (mut vmask_w, mut smask_w) = (vmask, smask);
        if export {
            vmask_w = vmask & !smask & 0xf;
            smask_w = smask & !vmask & 0xf;
        }
        let need_v = vmask_w != 0 || vname.starts_with("kill") || vname.starts_with("setp");
        let need_s = sname != "retain_prev" || smask_w != 0;
        let mut side: Lines = Vec::new();
        if need_v {
            let nargs = vargs(vname);
            let a = self.src(r1, sel1, s1sw, n1, absc);
            let bb = if nargs > 1 { self.src(r2, sel2, s2sw, n2, absc) } else { String::new() };
            let cc = if nargs > 2 { self.src(r3, sel3, s3sw, n3, absc) } else { String::new() };
            let (one, zero) = ("vec4<f32>(1.0)", "vec4<f32>(0.0)");
            let e: Option<String> = match vname {
                "add" => Some(format!("{a} + {bb}")),
                "mul" => Some(format!("{a} * {bb}")),
                "max" => Some(format!("max({a}, {bb})")),
                "min" => Some(format!("min({a}, {bb})")),
                "seq" => Some(format!("select({zero}, {one}, {a} == {bb})")),
                "sgt" => Some(format!("select({zero}, {one}, {a} > {bb})")),
                "sge" => Some(format!("select({zero}, {one}, {a} >= {bb})")),
                "sne" => Some(format!("select({zero}, {one}, {a} != {bb})")),
                "frc" => Some(format!("({a} - floor({a}))")),
                "trunc" => Some(format!("trunc({a})")),
                "floor" => Some(format!("floor({a})")),
                "mad" => Some(format!("{a} * {bb} + {cc}")),
                "cndeq" => Some(format!("select({cc}, {bb}, {a} == {zero})")),
                "cndge" => Some(format!("select({cc}, {bb}, {a} >= {zero})")),
                "cndgt" => Some(format!("select({cc}, {bb}, {a} > {zero})")),
                "dp4" => Some(format!("vec4<f32>(dot({a}, {bb}))")),
                "dp3" => Some(format!("vec4<f32>(dot(({a}).xyz, ({bb}).xyz))")),
                "dp2add" => Some(format!("vec4<f32>(dot(({a}).xy, ({bb}).xy) + ({cc}).x)")),
                "max4" => Some(format!("vec4<f32>(max(max(({a}).x, ({a}).y), max(({a}).z, ({a}).w)))")),
                "dst" => Some(format!("vec4<f32>(1.0, ({a}).y * ({bb}).y, ({a}).z, ({bb}).w)")),
                "maxa" => Some(format!("max({a}, {bb})")),
                _ => None,
            };
            if let Some(e) = e {
                l.push(format!("let v = {e};"));
            } else if vname.starts_with("kill_") {
                let op = match vname {
                    "kill_eq" => "==",
                    "kill_gt" => ">",
                    "kill_ge" => ">=",
                    _ => "!=",
                };
                l.push(format!("let vk = any({a} {op} {bb});"));
                l.push("let v = vec4<f32>(select(0.0, 1.0, vk));".into());
                side.push("if (vk) { discard; }".into());
            } else {
                return err(format!("vector operation {vname}: not handled"));
            }
        }
        if need_s {
            let ia = ((3 + ((s3sw >> 6) & 3)) & 3) as usize;
            let ib = (s3sw & 3) as usize;
            let base = self.src(r3, sel3, 0, 0, absc);
            let sg = if n3 != 0 { "-" } else { "" };
            let sa = format!("({sg}{base}.{})", C[ia]);
            let sb = format!("({sg}{base}.{})", C[ib]);
            if matches!(sname, "mulsc0" | "mulsc1" | "addsc0" | "addsc1" | "subsc0" | "subsc1") {
                // a constant (operand 3) with a register named by the opcode's low bit, bits 2..5 of the third swizzle and its select bit
                // (the operand's negate and absolute flags apply to BOTH, the constant and the register)
                let reg = (sop & 1) | (s3sw & 0x3C) | (sel3 << 1);
                let k = self.constant(r3);
                let ka = format!("({sg}{}.{})", if absc == 0 { k } else { format!("abs({k})") }, C[ia]);
                let rb = format!("({sg}{}.{})", if absc == 0 { format!("r[{reg}]") } else { format!("abs(r[{reg}])") }, C[ib]);
                let op = match &sname[..3] {
                    "mul" => "*",
                    "add" => "+",
                    _ => "-",
                };
                l.push(format!("let s = {ka} {op} {rb};"));
            } else {
                let t: Option<String> = match sname {
                    "adds" => Some(format!("{sa} + {sb}")),
                    "adds_prev" => Some(format!("{sa} + ps")),
                    "muls" => Some(format!("{sa} * {sb}")),
                    "muls_prev" => Some(format!("{sa} * ps")),
                    "maxs" => Some(format!("max({sa}, {sb})")),
                    "mins" => Some(format!("min({sa}, {sb})")),
                    "seqs" => Some(format!("select(0.0, 1.0, {sa} == 0.0)")),
                    "sgts" => Some(format!("select(0.0, 1.0, {sa} > 0.0)")),
                    "sges" => Some(format!("select(0.0, 1.0, {sa} >= 0.0)")),
                    "snes" => Some(format!("select(0.0, 1.0, {sa} != 0.0)")),
                    "frcs" => Some(format!("({sa} - floor({sa}))")),
                    "truncs" => Some(format!("trunc({sa})")),
                    "floors" => Some(format!("floor({sa})")),
                    "exp" => Some(format!("exp2({sa})")),
                    "logc" => Some(format!("max(log2({sa}), -3.4e38)")),
                    "log" => Some(format!("log2({sa})")),
                    "rcpc" | "rcpf" | "rcp" => Some(format!("(1.0 / {sa})")),
                    "rsqc" | "rsqf" | "rsq" => Some(format!("inverseSqrt({sa})")),
                    "maxas" | "maxasf" => Some(format!("max({sa}, {sb})")),
                    "subs" => Some(format!("{sa} - {sb}")),
                    "subs_prev" => Some(format!("{sa} - ps")),
                    "sqrt" => Some(format!("sqrt({sa})")),
                    "sin" => Some(format!("sin({sa})")),
                    "cos" => Some(format!("cos({sa})")),
                    "retain_prev" => Some("ps".into()),
                    "muls_prev2" => Some(format!("select({sa} * ps, -3.4e38, ps == -3.4e38 || {sb} <= 0.0)")),
                    _ => None,
                };
                let setp = match sname {
                    "setp_eq" => Some("=="),
                    "setp_ne" => Some("!="),
                    "setp_gt" => Some(">"),
                    "setp_ge" => Some(">="),
                    _ => None,
                };
                let kill = match sname {
                    "kills_eq" => Some("== 0.0"),
                    "kills_gt" => Some("> 0.0"),
                    "kills_ge" => Some(">= 0.0"),
                    "kills_ne" => Some("!= 0.0"),
                    "kills_one" => Some("== 1.0"),
                    _ => None,
                };
                if let Some(t) = t {
                    l.push(format!("let s = {t};"));
                } else if let Some(op) = setp {
                    l.push(format!("let sp = {sa} {op} 0.0;"));
                    l.push("let s = select(1.0, 0.0, sp);".into());
                    side.push("p = sp;".into());
                } else if let Some(rhs) = kill {
                    l.push(format!("let sk = {sa} {rhs};"));
                    l.push("let s = select(0.0, 1.0, sk);".into());
                    side.push("if (sk) { discard; }".into());
                } else {
                    return err(format!("scalar operation {sname}: not handled"));
                }
            }
        }
        // the writes (after both operations have read their operands)
        let mut w: Lines = Vec::new();
        if need_v && vmask_w != 0 {
            w.extend(Self::write(&self.dest(vdst, export)?, vmask_w, "v", vclamp));
        }
        if need_s {
            if smask_w != 0 {
                w.extend(Self::write(&self.dest(if export { vdst } else { sdst }, export)?, smask_w, "vec4<f32>(s)", sclamp));
            }
            if sname != "retain_prev" {
                w.push("ps = s;".into());
            }
        }
        if export {
            let one = vmask & smask;
            let zero = if sdst_rel != 0 { 0xf & !(vmask | smask) } else { 0 };
            for i in 0..4 {
                if one >> i & 1 != 0 {
                    w.push(format!("{}.{} = 1.0;", self.dest(vdst, true)?, C[i]));
                }
                if zero >> i & 1 != 0 {
                    w.push(format!("{}.{} = 0.0;", self.dest(vdst, true)?, C[i]));
                }
            }
        }
        w.extend(side);
        Ok((l, w, cond))
    }

    fn fetch(&mut self, d0: u32, d1: u32, d2: u32, cond: Option<String>) -> Res<Part> {
        let op = d0 & 0x1f;
        let (src, dst) = ((d0 >> 5) & 0x3f, (d0 >> 12) & 0x3f);
        let ssw = (d0 >> 26) & 0x3f;
        let sw3: Vec<char> = (0..3).map(|i| C[((ssw >> (2 * i)) & 3) as usize]).collect();
        let dsw = d1 & 0xfff;
        let mut cond = cond;
        if (d1 >> 31) & 1 != 0 {
            let c = if d2 >> 31 != 0 { "p" } else { "!p" };
            cond = Some(match cond {
                None => c.to_string(),
                Some(k) => format!("({k}) && {c}"),
            });
        }
        let mut l: Lines = Vec::new();
        let mut w: Lines = Vec::new();
        let store = |w: &mut Lines, expr: &str| {
            for i in 0..4 {
                let k = (dsw >> (3 * i)) & 7;
                if k < 4 {
                    w.push(format!("r[{dst}].{} = {expr}.{};", C[i], C[k as usize]));
                } else if k == 4 {
                    w.push(format!("r[{dst}].{} = 0.0;", C[i]));
                } else if k == 5 {
                    w.push(format!("r[{dst}].{} = 1.0;", C[i]));
                }
            }
        };
        match op {
            0 => {
                // (a vertex fetch: mini fetches that continue the previous one are not told apart here)
                l.push(format!("let t = vin[{}];", self.nfetch));
                self.nfetch += 1;
                store(&mut w, "t");
            }
            1 => {
                let n = (d0 >> 20) & 0x1f;
                if (d0 >> 25) & 1 != 0 {
                    return err("unnormalized texture coordinates: not handled");
                }
                let sx = |v: u32| -> f64 { ((((v & 0x1f) ^ 0x10) as i32) - 0x10) as f64 };
                let (ox, oy) = (sx(d2 >> 16) / 2.0, sx(d2 >> 21) / 2.0);
                let uv = format!("r[{src}].{}{}", sw3[0], sw3[1]);
                let (fname, call);
                if d2 & 1 != 0 {
                    fname = format!("xs_s{n}_grad");
                    call = format!("{fname}({uv}, gh.xy, gv.xy)");
                } else if (d1 >> 29) & 1 != 0 {
                    fname = format!("xs_s{n}_lod");
                    call = format!("{fname}({uv}, lod)");
                } else if ox != 0.0 || oy != 0.0 {
                    fname = format!("xs_s{n}_off");
                    call = format!("{fname}({uv}, vec2<f32>({}, {}))", f(ox), f(oy));
                } else {
                    fname = format!("xs_s{n}");
                    call = format!("{fname}({uv})");
                }
                self.uses.insert(fname);
                l.push(format!("let t = {call};"));
                store(&mut w, "t");
            }
            18 => {
                // getGradients: x = d/dx of the first component, y = d/dy of it, z, w = those of the second
                l.push(format!("let gs = r[{src}].{}{};", sw3[0], sw3[1]));
                l.push("let t = vec4<f32>(dpdx(gs.x), dpdy(gs.x), dpdx(gs.y), dpdy(gs.y));".into());
                store(&mut w, "t");
            }
            24 => w.push(format!("lod = r[{src}].{};", sw3[0])),
            25 => w.push(format!("gh = r[{src}].{};", sw3.iter().collect::<String>())),
            26 => w.push(format!("gv = r[{src}].{};", sw3.iter().collect::<String>())),
            _ => return err(format!("fetch operation {op}: not handled")),
        }
        Ok((l, w, cond))
    }

    fn run(&mut self, name: &str) -> Res<String> {
        let p = self.p;
        let b = &p.blob;
        let ncode = p.code_size / 12;
        let mut code = Vec::with_capacity(ncode);
        for i in 0..ncode {
            let o = p.code_off + i * 12;
            match (be32(b, o), be32(b, o + 4), be32(b, o + 8)) {
                (Some(a), Some(c), Some(d)) => code.push((a, c, d)),
                _ => return err("program code runs past the end of the data"),
            }
        }
        let mut execs: Vec<(usize, u32, u32, Option<String>)> = Vec::new();
        let (mut i, mut done) = (0usize, false);
        while i < code.len() && !done {
            let (d0, d1, d2) = code[i];
            for (lo, hi) in [(d0, d1 & 0xffff), ((d1 >> 16) | ((d2 & 0xffff) << 16), d2 >> 16)] {
                let cf = CFOPS[((hi >> 12) & 0xf) as usize];
                let (addr, cnt, ser) = ((lo & 0xfff) as usize, (lo >> 12) & 7, (lo >> 16) & 0xfff);
                if matches!(cf, "exec" | "exece" | "cexec_pred" | "cexec_pred_e" | "cexec_pred_clean" | "cexec_pred_clean_e") {
                    let mut cond = None;
                    if cf.contains("pred") {
                        cond = Some(if (hi >> 10) & 1 != 0 { "p".to_string() } else { "!p".to_string() });
                    }
                    execs.push((addr, cnt, ser, cond));
                    if cf.ends_with('e') {
                        done = true;
                    }
                } else if cf != "nop" && cf != "alloc" {
                    return err(format!("control flow {cf}: not handled"));
                }
            }
            i += 1;
            if !execs.is_empty() && i >= execs.iter().map(|e| e.0).min().unwrap_or(0) {
                break;
            }
        }
        let mut body: Vec<String> = Vec::new();
        for (addr, cnt, ser, bcond) in execs {
            if let Some(c) = &bcond {
                // (the block's condition is taken at its entry: instructions in it may change p)
                body.push(format!("    pb = {c};"));
            }
            for k in 0..cnt as usize {
                let Some(&(d0, d1, d2)) = code.get(addr + k) else { return err("exec block runs past the end of the code") };
                let is_fetch = (ser >> (2 * k)) & 1 != 0;
                let bc = bcond.as_ref().map(|_| "pb".to_string());
                let (l, w, cond) = if is_fetch { self.fetch(d0, d1, d2, bc)? } else { self.alu(d0, d1, d2, bc)? };
                body.push(format!("    {{ // {}", addr + k));
                body.extend(l.iter().map(|x| format!("        {x}")));
                match cond {
                    None => body.extend(w.iter().map(|x| format!("        {x}"))),
                    Some(c) => {
                        if !w.is_empty() {
                            body.push(format!("        if ({c}) {{"));
                            body.extend(w.iter().map(|x| format!("            {x}")));
                            body.push("        }".into());
                        }
                    }
                }
                body.push("    }".into());
            }
        }
        let mut out: Vec<String> = Vec::new();
        if p.is_ps {
            out.push(format!("fn {name}(rin: array<vec4<f32>, 16>) -> vec4<f32> {{"));
            out.push("    var r: array<vec4<f32>, 64>;".into());
            out.push("    for (var i = 0; i < 16; i = i + 1) { r[i] = rin[i]; }".into());
            out.push("    var o: array<vec4<f32>, 4>;".into());
        } else {
            out.push(format!("fn {name}(vin: array<vec4<f32>, 16>) -> XsVsOut {{"));
            out.push("    var r: array<vec4<f32>, 64>;".into());
            out.push("    var o: array<vec4<f32>, 16>;".into());
            out.push("    var opos = vec4<f32>(0.0);".into());
            out.push("    var opts = vec4<f32>(0.0);".into());
        }
        for s in ["    var ps = 0.0;", "    var p = false;", "    var pb = false;", "    var lod = 0.0;", "    var gh = vec3<f32>(0.0);", "    var gv = vec3<f32>(0.0);"] {
            out.push(s.into());
        }
        out.extend(body);
        if p.is_ps {
            out.push("    return o[0];".into());
        } else {
            out.push("    return XsVsOut(opos, o);".into());
        }
        out.push("}".into());
        let mut s = out.join("\n");
        s.push('\n');
        Ok(s)
    }
}

/// The result of translating one program.
#[derive(Debug, Clone)]
pub struct Translated {
    pub wgsl: String,
    /// Sampler function names the program calls, sorted as text.
    pub samplers: Vec<String>,
    /// Constant registers it reads, ascending.
    pub consts: Vec<i64>,
}

/// Translate the first program called `prog_name` of an effect file's bytes into a WGSL function called `fn_name`.
pub fn translate(effect: &[u8], prog_name: &str, fn_name: &str) -> Result<Translated, XsError> {
    for p in programs(effect) {
        if p.name == prog_name {
            let mut e = Emit::new(&p);
            let wgsl = e.run(fn_name)?;
            return Ok(Translated { wgsl, samplers: e.uses.into_iter().collect(), consts: e.consts.into_iter().collect() });
        }
    }
    err(format!("no program {prog_name} in the effect file"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_float_text() {
        assert_eq!(py_repr(0.5), "0.5");
        assert_eq!(py_repr(12.0), "12.0");
        assert_eq!(py_repr(-1.0), "-1.0");
        assert_eq!(py_repr(0.0001), "0.0001");
        assert_eq!(py_repr(0.00001), "1e-05");
        assert_eq!(py_repr(1.5e-7), "1.5e-07");
        assert_eq!(py_repr(1e15), "1000000000000000.0");
        assert_eq!(py_repr(1e16), "1e+16");
        assert_eq!(py_repr(1.25e17), "1.25e+17");
        assert_eq!(py_repr(12.579999923706055), "12.579999923706055");
        assert_eq!(py_repr(3.4e38_f32 as f64), "3.3999999521443642e+38");
        assert_eq!(f(12.0), "12.0");
        assert_eq!(f(f64::NAN), "0.0");
        assert_eq!(f(1e-5), "1e-05");
        assert_eq!(f(0.0), "0.0");
        assert_eq!(f(-0.0), "-0.0");
    }
}
