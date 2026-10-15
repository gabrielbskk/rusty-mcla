//! Baseline JPEG decoder (Huffman, 8 bit, 1 or 3 components, sampling factors up to 2),
//! enough for the motion-JPEG frames the converted videos are stored as.

use crate::{bail, Result};

pub struct Image {
    pub width: usize,
    pub height: usize,
    /// RGBA, 8 bits per channel
    pub rgba: Vec<u8>,
}

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38,
    31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[derive(Clone)]
struct Huff {
    /// codes of up to 9 bits: (length << 8) | symbol, 0 = longer code
    fast: [u16; 512],
    maxcode: [i32; 18],
    delta: [i32; 17],
    vals: Vec<u8>,
}

impl Huff {
    fn new(counts: &[u8], vals: &[u8]) -> Huff {
        let mut h = Huff { fast: [0; 512], maxcode: [-1; 18], delta: [0; 17], vals: vals.to_vec() };
        let mut code = 0i32;
        let mut k = 0usize;
        for len in 1..=16usize {
            let n = counts[len - 1] as usize;
            h.delta[len] = k as i32 - code;
            for i in 0..n {
                if len <= 9 && k + i < vals.len() {
                    let c = ((code + i as i32) as usize) << (9 - len);
                    for f in 0..(1usize << (9 - len)) {
                        h.fast[c + f] = ((len as u16) << 8) | vals[k + i] as u16;
                    }
                }
            }
            code += n as i32;
            k += n;
            h.maxcode[len] = if n > 0 { code - 1 } else { -1 };
            code <<= 1;
        }
        h.maxcode[17] = i32::MAX;
        h
    }
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
    /// a marker was reached: only zero bits from here
    marker: bool,
}

impl<'a> Bits<'a> {
    fn fill(&mut self) {
        while self.n <= 48 {
            let mut b = 0u8;
            if !self.marker && self.pos < self.d.len() {
                b = self.d[self.pos];
                if b == 0xff {
                    let nx = self.d.get(self.pos + 1).copied().unwrap_or(0xd9);
                    if nx == 0 {
                        self.pos += 2;
                    } else {
                        self.marker = true;
                        b = 0;
                    }
                } else {
                    self.pos += 1;
                }
            }
            self.acc |= (b as u64) << (56 - self.n);
            self.n += 8;
        }
    }
    #[inline]
    fn peek(&mut self, k: u32) -> u32 {
        if self.n < k {
            self.fill();
        }
        (self.acc >> (64 - k)) as u32
    }
    #[inline]
    fn skip(&mut self, k: u32) {
        self.acc <<= k;
        self.n -= k;
    }
    #[inline]
    fn get(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        let v = self.peek(k);
        self.skip(k);
        v
    }
    #[inline]
    fn decode(&mut self, h: &Huff) -> u8 {
        let p = self.peek(16);
        let f = h.fast[(p >> 7) as usize];
        if f != 0 {
            self.skip((f >> 8) as u32);
            return f as u8;
        }
        for len in 10..=16usize {
            let code = (p >> (16 - len)) as i32;
            if code <= h.maxcode[len] {
                self.skip(len as u32);
                return h.vals.get((code + h.delta[len]) as usize).copied().unwrap_or(0);
            }
        }
        self.skip(16);
        0
    }
    /// Continue after a restart marker.
    fn restart(&mut self) {
        self.acc = 0;
        self.n = 0;
        self.marker = false;
        if self.pos + 1 < self.d.len() && self.d[self.pos] == 0xff && (0xd0..=0xd7).contains(&self.d[self.pos + 1]) {
            self.pos += 2;
        }
    }
}

#[inline]
fn extend(v: u32, s: u32) -> i32 {
    if s == 0 {
        0
    } else if v < (1 << (s - 1)) {
        v as i32 - (1 << s) + 1
    } else {
        v as i32
    }
}

#[inline]
fn clamp8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

macro_rules! f2f {
    ($x:expr) => {
        (($x * 4096.0 + 0.5) as i32)
    };
}

#[inline(always)]
fn idct1d(s: [i32; 8]) -> ([i32; 4], [i32; 4]) {
    let (mut p2, mut p3) = (s[2], s[6]);
    let mut p1 = (p2 + p3) * f2f!(0.5411961f32);
    let t2 = p1 + p3 * f2f!(-1.847759065f32);
    let t3 = p1 + p2 * f2f!(0.765366865f32);
    p2 = s[0];
    p3 = s[4];
    let t0 = (p2 + p3) * 4096;
    let t1 = (p2 - p3) * 4096;
    let x = [t0 + t3, t1 + t2, t1 - t2, t0 - t3];
    let (mut t0, mut t1, mut t2, mut t3) = (s[7], s[5], s[3], s[1]);
    p3 = t0 + t2;
    let mut p4 = t1 + t3;
    p1 = t0 + t3;
    p2 = t1 + t2;
    let p5 = (p3 + p4) * f2f!(1.175875602f32);
    t0 *= f2f!(0.298631336f32);
    t1 *= f2f!(2.053119869f32);
    t2 *= f2f!(3.072711026f32);
    t3 *= f2f!(1.501321110f32);
    p1 = p5 + p1 * f2f!(-0.899976223f32);
    p2 = p5 + p2 * f2f!(-2.562915447f32);
    p3 *= f2f!(-1.961570560f32);
    p4 *= f2f!(-0.390180644f32);
    t3 += p1 + p4;
    t2 += p2 + p3;
    t1 += p2 + p4;
    t0 += p1 + p3;
    (x, [t0, t1, t2, t3])
}

/// Inverse DCT of one block into `out` (stride bytes per row).
fn idct(d: &[i32; 64], out: &mut [u8], stride: usize) {
    let mut v = [0i32; 64];
    for c in 0..8 {
        if d[c + 8] == 0 && d[c + 16] == 0 && d[c + 24] == 0 && d[c + 32] == 0 && d[c + 40] == 0 && d[c + 48] == 0 && d[c + 56] == 0 {
            let dc = d[c] * 4;
            for r in 0..8 {
                v[c + r * 8] = dc;
            }
        } else {
            let (mut x, t) = idct1d([d[c], d[c + 8], d[c + 16], d[c + 24], d[c + 32], d[c + 40], d[c + 48], d[c + 56]]);
            for k in x.iter_mut() {
                *k += 512;
            }
            v[c] = (x[0] + t[3]) >> 10;
            v[c + 56] = (x[0] - t[3]) >> 10;
            v[c + 8] = (x[1] + t[2]) >> 10;
            v[c + 48] = (x[1] - t[2]) >> 10;
            v[c + 16] = (x[2] + t[1]) >> 10;
            v[c + 40] = (x[2] - t[1]) >> 10;
            v[c + 24] = (x[3] + t[0]) >> 10;
            v[c + 32] = (x[3] - t[0]) >> 10;
        }
    }
    for r in 0..8 {
        let s = &v[r * 8..r * 8 + 8];
        let (mut x, t) = idct1d([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]);
        for k in x.iter_mut() {
            *k += 65536 + (128 << 17);
        }
        let o = &mut out[r * stride..r * stride + 8];
        o[0] = clamp8((x[0] + t[3]) >> 17);
        o[7] = clamp8((x[0] - t[3]) >> 17);
        o[1] = clamp8((x[1] + t[2]) >> 17);
        o[6] = clamp8((x[1] - t[2]) >> 17);
        o[2] = clamp8((x[2] + t[1]) >> 17);
        o[5] = clamp8((x[2] - t[1]) >> 17);
        o[3] = clamp8((x[3] + t[0]) >> 17);
        o[4] = clamp8((x[3] - t[0]) >> 17);
    }
}

#[derive(Default, Clone)]
struct Comp {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    td: usize,
    ta: usize,
    pred: i32,
    stride: usize,
    plane: Vec<u8>,
}

/// Decode one JPEG image. `scratch` planes are reused between calls via the returned image only.
pub fn decode(d: &[u8]) -> Result<Image> {
    if d.len() < 4 || d[0] != 0xff || d[1] != 0xd8 {
        bail!("not a JPEG");
    }
    let mut qt = [[16i32; 64]; 4];
    let mut dc: [Option<Huff>; 4] = [None, None, None, None];
    let mut ac: [Option<Huff>; 4] = [None, None, None, None];
    let mut comps: Vec<Comp> = Vec::new();
    let (mut w, mut h) = (0usize, 0usize);
    let mut restart = 0usize;
    let mut p = 2usize;
    loop {
        if p + 4 > d.len() {
            bail!("JPEG ends before the scan");
        }
        if d[p] != 0xff {
            p += 1;
            continue;
        }
        let m = d[p + 1];
        if m == 0xff {
            p += 1;
            continue;
        }
        let len = ((d[p + 2] as usize) << 8) | d[p + 3] as usize;
        let seg = d.get(p + 4..p + 2 + len).ok_or_else(|| crate::anyhow!("JPEG segment overruns"))?;
        match m {
            0xdb => {
                let mut i = 0;
                while i < seg.len() {
                    let (prec, id) = (seg[i] >> 4, (seg[i] & 15) as usize);
                    i += 1;
                    for k in 0..64 {
                        let v = if prec == 0 {
                            let v = *seg.get(i).unwrap_or(&16) as i32;
                            i += 1;
                            v
                        } else {
                            let v = ((*seg.get(i).unwrap_or(&0) as i32) << 8) | *seg.get(i + 1).unwrap_or(&16) as i32;
                            i += 2;
                            v
                        };
                        if id < 4 {
                            qt[id][ZIGZAG[k]] = v;
                        }
                    }
                }
            }
            0xc0 | 0xc1 => {
                if seg.len() < 6 || seg[0] != 8 {
                    bail!("only 8 bit JPEG");
                }
                h = ((seg[1] as usize) << 8) | seg[2] as usize;
                w = ((seg[3] as usize) << 8) | seg[4] as usize;
                let n = seg[5] as usize;
                if !(n == 1 || n == 3) || seg.len() < 6 + n * 3 {
                    bail!("unsupported JPEG component count");
                }
                comps.clear();
                for c in 0..n {
                    let s = &seg[6 + c * 3..9 + c * 3];
                    comps.push(Comp { id: s[0], h: (s[1] >> 4) as usize, v: (s[1] & 15) as usize, tq: (s[2] & 3) as usize, ..Default::default() });
                }
            }
            0xc2 => bail!("progressive JPEG is not supported"),
            0xc4 => {
                let mut i = 0;
                while i + 17 <= seg.len() {
                    let (class, id) = (seg[i] >> 4, (seg[i] & 3) as usize);
                    let counts = &seg[i + 1..i + 17];
                    let n: usize = counts.iter().map(|&c| c as usize).sum();
                    let vals = seg.get(i + 17..i + 17 + n).ok_or_else(|| crate::anyhow!("bad Huffman table"))?;
                    let t = Huff::new(counts, vals);
                    if class == 0 {
                        dc[id] = Some(t);
                    } else {
                        ac[id] = Some(t);
                    }
                    i += 17 + n;
                }
            }
            0xdd => restart = ((seg[0] as usize) << 8) | seg[1] as usize,
            0xda => {
                let n = seg[0] as usize;
                for k in 0..n {
                    let (id, t) = (seg[1 + k * 2], seg[2 + k * 2]);
                    if let Some(c) = comps.iter_mut().find(|c| c.id == id) {
                        c.td = (t >> 4) as usize & 3;
                        c.ta = (t & 3) as usize;
                    }
                }
                p += 2 + len;
                break;
            }
            0xd9 => bail!("JPEG without a scan"),
            _ => {}
        }
        p += 2 + len;
    }
    if w == 0 || h == 0 || comps.is_empty() {
        bail!("JPEG without a frame header");
    }
    let hmax = comps.iter().map(|c| c.h).max().unwrap_or(1).max(1);
    let vmax = comps.iter().map(|c| c.v).max().unwrap_or(1).max(1);
    if hmax > 2 || vmax > 2 || comps.iter().any(|c| c.h == 0 || c.v == 0) {
        bail!("unsupported JPEG sampling");
    }
    let (mw, mh) = ((w + 8 * hmax - 1) / (8 * hmax), (h + 8 * vmax - 1) / (8 * vmax));
    for c in comps.iter_mut() {
        c.stride = mw * c.h * 8;
        c.plane = vec![0u8; c.stride * mh * c.v * 8];
        if dc[c.td].is_none() || ac[c.ta].is_none() {
            bail!("JPEG scan uses a missing Huffman table");
        }
    }
    let mut bits = Bits { d, pos: p, acc: 0, n: 0, marker: false };
    let mut block = [0i32; 64];
    let mut count = 0usize;
    for my in 0..mh {
        for mx in 0..mw {
            if restart != 0 && count == restart {
                bits.restart();
                for c in comps.iter_mut() {
                    c.pred = 0;
                }
                count = 0;
            }
            count += 1;
            for c in comps.iter_mut() {
                let (hd, ha) = (dc[c.td].as_ref().unwrap(), ac[c.ta].as_ref().unwrap());
                let q = &qt[c.tq];
                for by in 0..c.v {
                    for bx in 0..c.h {
                        block.fill(0);
                        let s = bits.decode(hd) as u32;
                        let diff = if s == 0 { 0 } else { extend(bits.get(s.min(16)), s.min(16)) };
                        c.pred += diff;
                        block[0] = c.pred * q[0];
                        let mut k = 1;
                        while k < 64 {
                            let rs = bits.decode(ha);
                            let (r, s) = ((rs >> 4) as usize, (rs & 15) as u32);
                            if s == 0 {
                                if r == 15 {
                                    k += 16;
                                    continue;
                                }
                                break;
                            }
                            k += r;
                            if k > 63 {
                                break;
                            }
                            let z = ZIGZAG[k];
                            block[z] = extend(bits.get(s), s) * q[z];
                            k += 1;
                        }
                        let o = (my * c.v + by) * 8 * c.stride + (mx * c.h + bx) * 8;
                        idct(&block, &mut c.plane[o..], c.stride);
                    }
                }
            }
        }
    }
    // to RGBA
    let mut rgba = vec![255u8; w * h * 4];
    if comps.len() == 1 {
        let c = &comps[0];
        for y in 0..h {
            for x in 0..w {
                let v = c.plane[y * c.stride + x];
                rgba[(y * w + x) * 4..(y * w + x) * 4 + 3].copy_from_slice(&[v, v, v]);
            }
        }
    } else {
        let (cy, cb, cr) = (&comps[0], &comps[1], &comps[2]);
        let (sx, sy) = (if cb.h < cy.h { 1 } else { 0 }, if cb.v < cy.v { 1 } else { 0 });
        for y in 0..h {
            let yr = &cy.plane[y * cy.stride..];
            let br = &cb.plane[(y >> sy) * cb.stride..];
            let rr = &cr.plane[(y >> sy) * cr.stride..];
            let out = &mut rgba[y * w * 4..(y + 1) * w * 4];
            for x in 0..w {
                let l = (yr[x] as i32) << 16;
                let b = br[x >> sx] as i32 - 128;
                let r = rr[x >> sx] as i32 - 128;
                out[x * 4] = clamp8((l + 91881 * r + 32768) >> 16);
                out[x * 4 + 1] = clamp8((l - 22554 * b - 46802 * r + 32768) >> 16);
                out[x * 4 + 2] = clamp8((l + 116130 * b + 32768) >> 16);
            }
        }
    }
    Ok(Image { width: w, height: h, rgba })
}
