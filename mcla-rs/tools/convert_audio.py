#!/usr/bin/env python3
"""Convert the Midnight Club: Los Angeles (Xbox 360) audio banks to PC formats.

The disc stores RAGE audio banks whose waves are XMA2. This tool unwraps them and has
ffmpeg decode them:

  wave banks   -> <out>/<bank path>/<wave name>.wav        (16-bit PCM, mono)
                  <out>/<bank path>/_index.tsv              (name, hash, samples, rate, flags)
  stream files -> <out>/<path>.mp3                          (stereo; surround folded down)

Wave names are recovered by hashing (Jenkins one-at-a-time, lower case) every name
found in the game's sound metadata (config/sounds.dat, game.dat, speech.dat).

usage: convert_audio.py <extracted root> <out dir> [--only substring] [--survey] [--jobs N]
  <extracted root> is searched for .../audio/X360/sfx directories.
"""
import os, re, struct, subprocess, sys, shutil, glob
from concurrent.futures import ThreadPoolExecutor


def find_ffmpeg():
    p = shutil.which("ffmpeg")
    if p:
        return p
    for g in glob.glob(os.path.expandvars(r"%LOCALAPPDATA%\Microsoft\WinGet\Packages\Gyan.FFmpeg*\*\bin\ffmpeg.exe")):
        return g
    sys.exit("ffmpeg not found")


def joaat(s):
    h = 0
    for c in s.lower().encode("latin1"):
        h = (h + c) & 0xFFFFFFFF
        h = (h + (h << 10)) & 0xFFFFFFFF
        h ^= h >> 6
    h = (h + (h << 3)) & 0xFFFFFFFF
    h ^= h >> 11
    h = (h + (h << 15)) & 0xFFFFFFFF
    return h


def build_names(config_dir):
    names = {}
    for f in ("sounds.dat", "game.dat", "speech.dat"):
        p = os.path.join(config_dir, f)
        if not os.path.exists(p):
            continue
        d = open(p, "rb").read()
        for m in re.finditer(rb"[A-Za-z0-9_\\/ .\-]{3,120}", d):
            s = m.group().decode("latin1")
            # strings are length-prefixed: try without the leading byte(s), and every "_" suffix
            for base in (s, s[1:], s[2:]):
                parts = base.split("_")
                for i in range(len(parts)):
                    t = "_".join(parts[i:])
                    if len(t) >= 2:
                        names.setdefault(joaat(t), t)
    for extra in ("LEFT", "RIGHT", "L", "R", "CENTRE", "CENTER", "LFE", "LS", "RS", "MONO", "FRONT_LEFT", "FRONT_RIGHT", "REAR_LEFT", "REAR_RIGHT"):
        names.setdefault(joaat(extra), extra)
    return names


def xma_riff(data, rate, samples):
    """Mono XMA2 stream wrapped in a RIFF/WAVE container with an XMA2WAVEFORMATEX."""
    blocks = max(1, len(data) // 2048)
    fmt = struct.pack("<HHIIHHH", 0x166, 1, rate, max(1, len(data) * rate // max(samples, 1)), 1, 16, 34)
    fmt += struct.pack("<HIIIIIIIBBH", 1, 0, samples, min(len(data), 2048 * 16), 0, samples, 0, 0, 0, 4, blocks)
    body = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


def is_stream(d):
    if len(d) < 0x30:
        return False
    nblocks, bsize = struct.unpack(">II", d[8:16])
    hdr, = struct.unpack(">I", d[0x2C:0x30])
    return bsize >= 0x800 and bsize % 0x800 == 0 and nblocks > 0 and hdr + nblocks * bsize == len(d)


def parse_bank(d):
    tab, = struct.unpack(">Q", d[0:8])
    n, _one, ds = struct.unpack(">III", d[16:28])
    info0 = tab + n * 16
    out = []
    for i in range(n):
        off, h, _sz = struct.unpack(">QII", d[tab + i * 16:tab + i * 16 + 16])
        o = info0 + off
        doff, _h2, size, ns = struct.unpack(">QIII", d[o:o + 20])
        rate, = struct.unpack(">H", d[o + 24:o + 26])
        flags, = struct.unpack(">I", d[o + 48:o + 52])
        out.append(dict(hash=h, data=d[ds + doff:ds + doff + size], rate=rate, samples=ns, flags=flags))
    return out


def parse_stream(d):
    btab, = struct.unpack(">Q", d[0:8])
    nblocks, bsize = struct.unpack(">II", d[8:16])
    ctab, = struct.unpack(">I", d[0x18:0x1C])
    nch, = struct.unpack(">I", d[0x24:0x28])
    hdr, = struct.unpack(">I", d[0x2C:0x30])
    chans = []
    info0 = ctab + nch * 16
    for c in range(nch):
        off, h, _sz = struct.unpack(">QII", d[ctab + c * 16:ctab + c * 16 + 16])
        o = info0 + off
        _z, _h2, size, ns = struct.unpack(">QIII", d[o:o + 20])
        rate, = struct.unpack(">H", d[o + 24:o + 26])
        chans.append(dict(hash=h, rate=rate, samples=ns, size=size, data=bytearray()))
    for b in range(nblocks):
        bo = hdr + b * bsize
        choff, = struct.unpack(">Q", d[bo:bo + 8])
        seekoff, = struct.unpack(">Q", d[bo + 8:bo + 16])
        counts = []
        for c in range(nch):
            _start, npk, _skip, _ns = struct.unpack(">IIII", d[bo + choff + c * 16:bo + choff + c * 16 + 16])
            counts.append(npk)
        total_seek = sum(counts)
        data0 = bo + ((seekoff + total_seek * 8 + 0x7FF) & ~0x7FF)
        p = data0
        for c in range(nch):
            chans[c]["data"] += d[p:p + counts[c] * 0x800]
            p += counts[c] * 0x800
    for c in chans:
        c["data"] = bytes(c["data"][:c["size"]]) if c["size"] else bytes(c["data"])
    return chans


def safe(name):
    return re.sub(r"[^A-Za-z0-9_.\-]", "_", name).strip(".").lower()


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    return r.returncode == 0, r.stderr.strip()[-300:]


def convert_bank(ff, src, out_dir, names):
    d = open(src, "rb").read()
    waves = parse_bank(d)
    os.makedirs(out_dir, exist_ok=True)
    lines = []
    ok = 0
    used = set()
    for w in waves:
        nm = safe(names.get(w["hash"], "%08x" % w["hash"]))
        if nm in used:
            nm = "%s_%08x" % (nm, w["hash"])
        used.add(nm)
        wav = os.path.join(out_dir, nm + ".wav")
        if not os.path.exists(wav) and w["data"]:
            tmp = os.path.join(out_dir, nm + ".xma")
            open(tmp, "wb").write(xma_riff(w["data"], w["rate"], w["samples"]))
            good, err = run([ff, "-y", "-v", "error", "-i", tmp, "-ac", "1", "-c:a", "pcm_s16le", wav])
            os.remove(tmp)
            if not good:
                print("  FAILED", src, nm, err)
                continue
        ok += 1
        lines.append("%s\t%08x\t%d\t%d\t%x" % (nm, w["hash"], w["samples"], w["rate"], w["flags"]))
    open(os.path.join(out_dir, "_index.tsv"), "w").write("\n".join(lines) + "\n")
    return ok, len(waves)


def convert_stream(ff, src, out_path, names):
    if os.path.exists(out_path):
        return 1, 1
    d = open(src, "rb").read()
    chans = parse_stream(d)
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    # Channels are stored in hash order: find out which speaker each one is from its name
    # ("<STREAM>.L", "..._LS", "..._RIGHT"); the names hash from the stream's own name.
    base = os.path.basename(src)
    roles = {"L": (1.0, 0.0), "R": (0.0, 1.0), "C": (0.707, 0.707), "LS": (0.6, 0.0), "RS": (0.0, 0.6), "LFE": (0.3, 0.3), "M": (0.707, 0.707)}
    def role(c):
        n = names.get(c["hash"], "").upper()
        if not n or n == "?":
            for suf, r in ((".L", "L"), (".R", "R"), (".C", "C"), (".LS", "LS"), (".RS", "RS"), ("_L", "L"), ("_R", "R"), ("_C", "C"), ("_LS", "LS"), ("_RS", "RS"), ("_LEFT", "L"), ("_RIGHT", "R"), (".LFE", "LFE")):
                if joaat(base + suf) == c["hash"]:
                    c["name"] = base + suf
                    return r
            return "M"
        c["name"] = n
        for suf, r in (("LS", "LS"), ("RS", "RS"), ("LFE", "LFE"), ("LEFT", "L"), ("RIGHT", "R"), ("CENTRE", "C"), ("CENTER", "C"), ("L", "L"), ("R", "R"), ("C", "C")):
            if n.endswith("_" + suf) or n.endswith("." + suf):
                return r
        return "M"
    rl = [role(c) for c in chans]
    if len(chans) == 2 and rl[0] == "R" and rl[1] == "L":
        chans.reverse()
        rl.reverse()
    if len(chans) == 2 and "M" in rl:
        rl = ["L", "R"]
    tmps = []
    for i, c in enumerate(chans):
        tmp = "%s.ch%d.xma" % (out_path, i)
        open(tmp, "wb").write(xma_riff(c["data"], c["rate"], c["samples"]))
        tmps.append(tmp)
    cmd = [ff, "-y", "-v", "error"]
    for t in tmps:
        # (say what it is: some payloads look like MPEG-TS to the prober)
        cmd += ["-f", "wav", "-i", t]
    n = len(tmps)
    enc = ["-c:a", "libmp3lame", "-q:a", "2"]
    if n == 1:
        cmd += enc + [out_path]
    else:
        # one mono stream per channel -> stereo, every speaker folded to its side
        left = "+".join("%.3f*c%d" % (roles[r][0], i) for i, r in enumerate(rl) if roles[r][0] > 0) or "0*c0"
        right = "+".join("%.3f*c%d" % (roles[r][1], i) for i, r in enumerate(rl) if roles[r][1] > 0) or "0*c0"
        cmd += ["-filter_complex", "".join("[%d:a]" % i for i in range(n)) + "amerge=inputs=%d,pan=stereo|c0=%s|c1=%s[a]" % (n, left, right), "-map", "[a]"] + enc + [out_path]
    good, err = run(cmd)
    for t in tmps:
        os.remove(t)
    if not good:
        print("  FAILED", src, err)
        return 0, 1
    open(out_path + ".txt", "w").write("\n".join("%d\t%08x\t%s\t%d\t%d" % (i, c["hash"], names.get(c["hash"], "?"), c["samples"], c["rate"]) for i, c in enumerate(chans)) + "\n")
    return 1, 1


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    root, out = args[0], args[1]
    only = None
    jobs = 12
    survey = "--survey" in sys.argv
    for i, a in enumerate(sys.argv):
        if a == "--only":
            only = sys.argv[i + 1].lower()
        if a == "--jobs":
            jobs = int(sys.argv[i + 1])
    if only:
        args = [a for a in args if a.lower() != only]
    ff = find_ffmpeg()
    cfg = None
    sfx_dirs = []
    for dp, dn, fn in os.walk(root):
        low = dp.replace("\\", "/").lower()
        if low.endswith("audio/x360/config"):
            cfg = dp
        if low.endswith("audio/x360/sfx"):
            sfx_dirs.append(dp)
    names = build_names(cfg) if cfg else {}
    print("%d names, %d sfx roots, ffmpeg %s" % (len(names), len(sfx_dirs), ff))
    work = []
    for sd in sfx_dirs:
        for dp, dn, fn in os.walk(sd):
            for f in fn:
                src = os.path.join(dp, f)
                rel = os.path.relpath(src, sd).replace("\\", "/")
                if only and only not in rel.lower():
                    continue
                work.append((src, rel))
    print("%d files" % len(work))

    def one(item):
        src, rel = item
        try:
            d = open(src, "rb").read(0x30)
            full = None
            if len(d) < 0x30:
                return rel, "skip", 0, 0
            size = os.path.getsize(src)
            nblocks, bsize = struct.unpack(">II", d[8:16])
            hdr, = struct.unpack(">I", d[0x2C:0x30])
            stream = bsize >= 0x800 and bsize % 0x800 == 0 and nblocks > 0 and hdr + (nblocks - 1) * bsize < size <= hdr + nblocks * bsize
            relsafe = "/".join(safe(p) for p in rel.split("/"))
            if survey:
                full = open(src, "rb").read()
                if stream:
                    ch = parse_stream(full)
                    return rel, "stream", len(ch), ch[0]["samples"] / max(ch[0]["rate"], 1)
                return rel, "bank", len(parse_bank(full)), 0
            if not stream and "--streams-only" in sys.argv:
                return rel, "bank", 1, 1
            if stream:
                ok, n = convert_stream(ff, src, os.path.join(out, relsafe + ".mp3"), names)
                return rel, "stream", ok, n
            ok, n = convert_bank(ff, src, os.path.join(out, relsafe), names)
            return rel, "bank", ok, n
        except Exception as e:  # keep going; report at the end
            return rel, "error %r" % (e,), 0, 0

    done = 0
    bad = []
    with ThreadPoolExecutor(jobs) as ex:
        for rel, kind, a, b in ex.map(one, work):
            done += 1
            if survey:
                print("%-8s %4d %8.1f  %s" % (kind, a, b, rel))
            elif kind.startswith("error") or a != b:
                bad.append((rel, kind, a, b))
            if not survey and done % 100 == 0:
                print("  %d / %d" % (done, len(work)), flush=True)
    print("done: %d files, %d with problems" % (done, len(bad)))
    for b in bad[:40]:
        print("  problem:", b)


if __name__ == "__main__":
    main()
