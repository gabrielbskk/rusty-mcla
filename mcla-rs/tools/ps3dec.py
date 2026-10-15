"""Decrypt a PS3 save data file (PARAM.PFD protected) - the scheme of flatz's pfdtool.
usage: python ps3dec.py <save dir> <file name> <out>"""
import sys, struct, itertools
from Crypto.Cipher import AES

SYSCON = bytes.fromhex("D413B89663E1FE9F75143D3BB4565274")
SECURE_ID = bytes.fromhex("9B9545ED1C6B5AFDAC012D153FB22C0A")  # Midnight Club: Los Angeles (games.conf)
KNOWN = bytes.fromhex("0000000f0000000900000005") + b"Stat"     # how the plain save starts (the 360 one)

def ecb_e(k, b): return AES.new(k, AES.MODE_ECB).encrypt(b)
def ecb_d(k, b): return AES.new(k, AES.MODE_ECB).decrypt(b)
def cbc_d(k, iv, b): return AES.new(k, AES.MODE_CBC, iv).decrypt(b)
def xor(a, b): return bytes(x ^ y for x, y in zip(a, b))

def block(key, data, i):
    """One 16-byte block of the file (pfd_decrypt_data): out = D(in) ^ E(block number)."""
    ctr = ecb_e(key, struct.pack(">Q", i) + bytes(8))
    return xor(ecb_d(key, data[i * 16:i * 16 + 16]), ctr)

def main():
    d, name, out = sys.argv[1], sys.argv[2], sys.argv[3]
    pfd = open(d + "/PARAM.PFD", "rb").read()
    data = open(d + "/" + name, "rb").read()
    assert pfd[4:8] == b"PFDB"
    hdr_iv = pfd[0x10:0x20]
    sig = cbc_d(SYSCON, hdr_iv, pfd[0x20:0x60])
    hash_key = sig[40:60]
    cap, res, used = struct.unpack(">3Q", pfd[0x60:0x78])
    ent0 = 0x78 + cap * 8
    print("pfd version", struct.unpack(">Q", pfd[8:16])[0], "capacity", cap, "used", used)
    entry = None
    for k in range(used):
        e = pfd[ent0 + k * 272: ent0 + (k + 1) * 272]
        if len(e) < 272: break
        nm = e[8:73].split(b"\0")[0].decode("latin1")
        if nm: print("  entry", k, nm, struct.unpack(">Q", e[264:272])[0])
        if nm == name: entry = e
    assert entry, "file not in PARAM.PFD"
    file_key = entry[80:144]
    # pfd_generate_hash_key_for_secure_file_id: four fixed bytes with the id's bytes laid in around them
    ins = bytearray(16); j = 0
    for i in range(16):
        if i in (1, 2, 5, 8): ins[i] = {1: 11, 2: 15, 5: 14, 8: 10}[i]
        else: ins[i] = SECURE_ID[j]; j += 1
    # pfd_decrypt_with_portability: AES-CBC with the console's manager key, that hash key as IV
    cands = {"entry key": cbc_d(SYSCON, bytes(ins), file_key)[:16]}
    for n, k in cands.items():
        if block(k, data, 0) == KNOWN:
            print("key found:", n, k.hex())
            plain = b"".join(block(k, data, i) for i in range(len(data) // 16))
            size = struct.unpack(">Q", entry[264:272])[0]
            open(out, "wb").write(plain[:size])
            print("wrote", out, size)
            return
    print("no candidate matched; first block per candidate:")
    for n, k in list(cands.items())[:8]: print(" ", n, block(k, data, 0).hex())

if __name__ == "__main__":
    main()
