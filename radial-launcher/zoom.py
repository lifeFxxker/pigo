# -*- coding: utf-8 -*-
"""从 24bpp BMP 裁剪区域并放大导出 PNG，用于像素级核对渲染。
用法: python zoom.py <in.bmp> <out.png> <x> <y> <w> <h> <scale>
"""
import struct, sys, zlib

def load_bmp(path):
    d = open(path, 'rb').read()
    assert d[:2] == b'BM', 'not bmp'
    off = struct.unpack_from('<I', d, 10)[0]
    w = struct.unpack_from('<i', d, 18)[0]
    h = struct.unpack_from('<i', d, 22)[0]
    bpp = struct.unpack_from('<H', d, 28)[0]
    assert bpp == 24, 'expect 24bpp, got %d' % bpp
    row = (w * 3 + 3) // 4 * 4
    ah = abs(h)
    top_down = h < 0

    def px(x, y):
        sy = y if top_down else (ah - 1 - y)
        p = off + sy * row + x * 3
        b, g, r = d[p], d[p + 1], d[p + 2]
        return (r, g, b)
    return w, ah, px

def save_png(path, w, h, rows):
    raw = b''.join(b'\x00' + r for r in rows)
    def ck(t, dd):
        c = t + dd
        return struct.pack('>I', len(dd)) + c + struct.pack('>I', zlib.crc32(c) & 0xffffffff)
    png = b'\x89PNG\r\n\x1a\n'
    png += ck(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0))
    png += ck(b'IDAT', zlib.compress(raw, 9))
    png += ck(b'IEND', b'')
    open(path, 'wb').write(png)

def main():
    inp, out = sys.argv[1], sys.argv[2]
    x, y, w, h, s = map(int, sys.argv[3:8])
    W, H, px = load_bmp(inp)
    ow, oh = w * s, h * s
    rows = []
    for oy in range(oh):
        sy = y + oy // s
        line = bytearray()
        for ox in range(ow):
            sx = x + ox // s
            if 0 <= sx < W and 0 <= sy < H:
                r, g, b = px(sx, sy)
            else:
                r = g = b = 0
            line += bytes((r, g, b))
        rows.append(bytes(line))
    save_png(out, ow, oh, rows)
    print('%s -> %s  (%dx%d x%d)' % (inp, out, w, h, s))

if __name__ == '__main__':
    main()
