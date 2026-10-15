#!/usr/bin/env python3
"""Convert the disc's Bink movies for the PC runtime.

    python tools/convert_video.py <game.iso> <out dir>

Writes <out>/intro.mcv + intro.mp3 and <out>/attract.mcv + attract.mp3.
The .mcv container is a list of JPEG frames:
    "MCV1", u32 width, u32 height, u32 fps * 1000, u32 frame count,
    (frame count + 1) x u64 offsets, JPEG data
The four Bink audio tracks (front pair, centre, LFE, rear pair) are folded to stereo.
"""
import glob, os, shutil, struct, subprocess, sys

MOVIES = {"intro": "/intro720.bik", "attract": "/attract720.bik"}


def find_ffmpeg():
    p = shutil.which("ffmpeg")
    if p:
        return p
    for g in glob.glob(os.path.expandvars(r"%LOCALAPPDATA%\Microsoft\WinGet\Packages\Gyan.FFmpeg*\*\bin\ffmpeg.exe")):
        return g
    sys.exit("ffmpeg not found")


def iso_files(iso):
    """Root directory of the XDVDFS game partition: name -> (offset, size)."""
    out = {}
    with open(iso, "rb") as f:
        for base in (0, 0xFD90000, 0x2080000, 0x18300000):
            f.seek(base + 0x10000)
            if f.read(20) != b"MICROSOFT*XBOX*MEDIA":
                continue
            root_sector, root_size = struct.unpack("<II", f.read(8))
            f.seek(base + root_sector * 2048)
            d = f.read(root_size)

            def walk(o):
                if o + 14 > len(d):
                    return
                l, r, sector, size, attr, n = struct.unpack_from("<HHIIBB", d, o)
                if l == 0xFFFF:
                    return
                name = d[o + 14:o + 14 + n].decode("latin1")
                out["/" + name.lower()] = (base + sector * 2048, size)
                if l:
                    walk(l * 4)
                if r:
                    walk(r * 4)

            walk(0)
            break
    return out


def split_jpegs(data):
    """Offsets of the JPEG images in a concatenated stream."""
    frames = []
    p, n = 0, len(data)
    while p + 4 <= n and data[p] == 0xFF and data[p + 1] == 0xD8:
        start = p
        p += 2
        # header segments up to the scan
        while True:
            if data[p] != 0xFF:
                raise ValueError("bad JPEG marker at %d" % p)
            m = data[p + 1]
            ln = (data[p + 2] << 8) | data[p + 3]
            p += 2 + ln
            if m == 0xDA:
                break
        # entropy data up to the end marker
        while True:
            p = data.index(b"\xff", p)
            m = data[p + 1]
            if m == 0 or 0xD0 <= m <= 0xD7:
                p += 2
                continue
            if m == 0xD9:
                p += 2
                break
            raise ValueError("unexpected marker %02x in scan" % m)
        frames.append((start, p))
    return frames


def convert(ff, src, out, name):
    print(name, "video ...", flush=True)
    r = subprocess.run([ff, "-v", "error", "-i", src, "-an", "-c:v", "mjpeg", "-q:v", "5", "-pix_fmt", "yuvj420p", "-f", "image2pipe", "-"], stdout=subprocess.PIPE, check=True)
    data = r.stdout
    frames = split_jpegs(data)
    # size and rate from the first frame header / the container
    w = h = 0
    p = 2
    while p < frames[0][1]:
        m = data[p + 1]
        ln = (data[p + 2] << 8) | data[p + 3]
        if m == 0xC0:
            h, w = struct.unpack_from(">HH", data, p + 5)
            break
        p += 2 + ln
    probe = subprocess.run([ff, "-i", src], stderr=subprocess.PIPE, stdout=subprocess.PIPE).stderr.decode("latin1")
    fps = 30.0
    for tok in probe.split(","):
        tok = tok.strip()
        if tok.endswith(" fps"):
            fps = float(tok.split()[0])
            break
    head = 20 + 8 * (len(frames) + 1)
    with open(os.path.join(out, name + ".mcv"), "wb") as f:
        f.write(b"MCV1" + struct.pack("<IIII", w, h, int(round(fps * 1000)), len(frames)))
        o = head
        for a, b in frames:
            f.write(struct.pack("<Q", o))
            o += b - a
        f.write(struct.pack("<Q", o))
        for a, b in frames:
            f.write(data[a:b])
    print("  %d frames %dx%d at %.2f fps, %.1f MB" % (len(frames), w, h, fps, (head + sum(b - a for a, b in frames)) / 1e6))
    print(name, "audio ...", flush=True)
    # tracks: 0 = front L/R, 1 = centre, 2 = LFE, 3 = rear L/R
    mix = ("[0:a:0][0:a:1][0:a:2][0:a:3]amerge=inputs=4,"
           "pan=stereo|c0=c0+0.707*c2+0.5*c3+0.6*c4|c1=c1+0.707*c2+0.5*c3+0.6*c5[a]")
    r = subprocess.run([ff, "-v", "error", "-y", "-i", src, "-filter_complex", mix, "-map", "[a]", "-ar", "44100", "-b:a", "192k", os.path.join(out, name + ".mp3")])
    if r.returncode != 0:
        # fewer tracks than expected: the first one alone
        subprocess.run([ff, "-v", "error", "-y", "-i", src, "-map", "0:a:0", "-ar", "44100", "-b:a", "192k", os.path.join(out, name + ".mp3")], check=True)


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    iso, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    ff = find_ffmpeg()
    files = iso_files(iso)
    for name, path in MOVIES.items():
        if path not in files:
            print("not on the disc:", path)
            continue
        off, size = files[path]
        tmp = os.path.join(out, name + ".bik")
        with open(iso, "rb") as f, open(tmp, "wb") as o:
            f.seek(off)
            left = size
            while left > 0:
                b = f.read(min(left, 1 << 24))
                if not b:
                    break
                o.write(b)
                left -= len(b)
        convert(ff, tmp, out, name)
        os.remove(tmp)


if __name__ == "__main__":
    main()
