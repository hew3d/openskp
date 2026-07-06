#!/usr/bin/env python3
"""extract_images — carve embedded CDib images (thumbnails + material textures)
out of a .skp. A CDib is `<u32 format><u32 length><raw image>` (format 1=JPEG,
4=PNG); we anchor on the image magic, read the 8-byte header before it, and
verify the declared length yields a complete image."""
import struct, re, sys, os

PNG, JPG = b"\x89PNG\r\n\x1a\n", b"\xff\xd8\xff"


def extract_images(d):
    out = []
    for m in re.finditer(rb"\x89PNG\r\n\x1a\n|\xff\xd8\xff", d):
        M = m.start()
        if M < 8:
            continue
        fmt, length = struct.unpack_from("<I", d, M-8)[0], struct.unpack_from("<I", d, M-4)[0]
        if fmt in (1, 4) and 8 <= length <= len(d) - M:
            img = d[M:M+length]
            kind = "png" if img[:4] == b"\x89PNG" else "jpg"
            tail = img.rstrip(b"\x00")
            complete = tail.endswith(b"\xaeB`\x82") if kind == "png" else tail.endswith(b"\xff\xd9")
            out.append({"off": M, "fmt": fmt, "kind": kind, "len": length,
                        "complete": complete, "bytes": img})
    return out


def main():
    if not sys.argv[1:]:
        print("usage: extract_images.py <file.skp> [outdir]"); return 1
    path = sys.argv[1]
    d = open(path, "rb").read()
    imgs = extract_images(d)
    print(f"{os.path.basename(path)}: {len(imgs)} embedded image(s)")
    outdir = sys.argv[2] if len(sys.argv) > 2 else None
    if outdir:
        os.makedirs(outdir, exist_ok=True)
    for i, im in enumerate(imgs):
        tag = "ok" if im["complete"] else "PARTIAL"
        print(f"  [{i}] {im['kind']} @0x{im['off']:06x}  {im['len']} bytes  ({tag})")
        if outdir:
            fn = os.path.join(outdir, f"{os.path.basename(path)[:-4]}_{i}.{im['kind']}")
            with open(fn, "wb") as f:
                f.write(im["bytes"])
            print(f"       -> {fn}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
