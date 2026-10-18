import struct, zlib, sys, os

def conv(src, dst):
    d = open(src, 'rb').read()
    off = struct.unpack_from('<I', d, 10)[0]
    w = struct.unpack_from('<i', d, 18)[0]
    h = struct.unpack_from('<i', d, 22)[0]
    bpp = struct.unpack_from('<H', d, 28)[0]
    row = (w * bpp // 8 + 3) // 4 * 4
    ah = abs(h)
    nb = bpp // 8
    rows = []
    for y in range(ah):
        sy = (ah - 1 - y) if h > 0 else y
        p = off + sy * row
        line = bytearray()
        for x in range(w):
            b = d[p + x * nb]
            g = d[p + x * nb + 1]
            r = d[p + x * nb + 2]
            line += bytes((r, g, b))
        rows.append(bytes(line))
    raw = b''.join(b'\x00' + r for r in rows)
    def ck(t, dd):
        c = t + dd
        return struct.pack('>I', len(dd)) + c + struct.pack('>I', zlib.crc32(c) & 0xffffffff)
    png = (b'\x89PNG\r\n\x1a\n'
           + ck(b'IHDR', struct.pack('>IIBBBBB', w, ah, 8, 2, 0, 0, 0))
           + ck(b'IDAT', zlib.compress(raw, 6))
           + ck(b'IEND', b''))
    open(dst, 'wb').write(png)
    return (w, ah)

if __name__ == '__main__':
    for f in sys.argv[1:]:
        out = os.path.splitext(f)[0] + '.png'
        print(out, conv(f, out))
