#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent expanded-segment cmap4 model and synthetic sfnt cases.

The model materializes every covered segment code in order; it does not use the
Rust reader's query search. No third-party modules. Font scaffolding is the
accepted independent O0 encoder; cmap encoding and expected maps are separate.
"""
import hashlib
import random
import struct
from pathlib import Path


def u16(data, at):
    return struct.unpack_from('>H', data, at)[0]


def tables(font):
    count = u16(font, 4)
    return {font[p:p+4]: struct.unpack_from('>II', font, p+8)
            for p in range(12, 12+16*count, 16)}


def records(font):
    offset, length = tables(font)[b'cmap']
    cmap = font[offset:offset+length]
    assert u16(cmap, 0) == 0
    result = []
    for i in range(u16(cmap, 2)):
        platform, encoding, start = struct.unpack_from('>HHI', cmap, 4+8*i)
        fmt = u16(cmap, start)
        result.append(dict(index=i, platform=platform, encoding=encoding,
                           format=fmt, offset=offset+start, cmap_offset=start))
    return result


def dense(font, record_index):
    rec = records(font)[record_index]
    start = rec['offset']
    assert rec['format'] == 4
    sub = font[start:start+u16(font, start+2)]
    assert len(sub) == u16(sub, 2)
    count = u16(sub, 6)//2
    ends = struct.unpack_from('>'+str(count)+'H', sub, 14)
    starts = struct.unpack_from('>'+str(count)+'H', sub, 16+2*count)
    deltas = struct.unpack_from('>'+str(count)+'H', sub, 16+4*count)
    offsets = struct.unpack_from('>'+str(count)+'H', sub, 16+6*count)
    result = [0] * 65536
    for index, (low, high, delta, skip) in enumerate(zip(starts, ends, deltas, offsets)):
        if skip:
            first = 16 + 6*count + 2*index + skip
            glyphs = struct.unpack_from('>'+str(high-low+1)+'H', sub, first)
            values = [((glyph+delta) & 65535) if glyph else 0 for glyph in glyphs]
        else:
            values = [(code+delta) & 65535 for code in range(low, high+1)]
        result[low:high+1] = values
    return result


def encode(segments):
    """segments: (start,end,delta,None-or-raw-glyph-list); adds canonical sentinel."""
    segments = list(segments) + [(65535, 65535, 1, None)]
    n = len(segments)
    glyphs = []
    offsets = []
    for i, (low, high, delta, raw) in enumerate(segments):
        if raw is None:
            offsets.append(0)
        else:
            assert len(raw) == high-low+1
            offsets.append(2*(n-i+len(glyphs)))
            glyphs.extend(raw)
    words = [4, 16+8*n+2*len(glyphs), 0, 2*n,
             2*(1 << (n.bit_length()-1)), n.bit_length()-1,
             2*n-2*(1 << (n.bit_length()-1))]
    words += [s[1] for s in segments] + [0]
    words += [s[0] for s in segments]
    words += [s[2] & 65535 for s in segments]
    words += offsets + glyphs
    return struct.pack('>'+str(len(words))+'H', *words)


def cmap_table(subtables):
    """subtables: [(platform,encoding,bytes)], preserving explicit order."""
    result = bytearray(struct.pack('>HH', 0, len(subtables)))
    offset = 4+8*len(subtables)
    for platform, encoding, data in subtables:
        result.extend(struct.pack('>HHI', platform, encoding, offset))
        offset += len(data)
    for _, _, data in subtables:
        result.extend(data)
    return bytes(result)


def with_cmap(font, cmap):
    old = tables(font)
    payloads = {tag:font[at:at+length] for tag,(at,length) in old.items()}
    payloads[b'cmap'] = cmap
    count = len(payloads)
    power = 1 << (count.bit_length()-1)
    result = bytearray(struct.pack('>IHHHH', 65536, count, 16*power,
                                  power.bit_length()-1, 16*(count-power)))
    result.extend(bytes(16*count))
    for i,(tag,data) in enumerate(sorted(payloads.items())):
        while len(result)%4: result.append(0)
        at = len(result)
        padded = data + bytes((-len(data))%4)
        checksum = sum(struct.unpack('>'+str(len(padded)//4)+'I', padded)) & 0xffffffff
        struct.pack_into('>4sIII', result, 12+16*i, tag, checksum, at, len(data))
        result.extend(data)
    return bytes(result)


def generated(count=220, seed=0x17C4A9):
    from outline_model import font as outline_font
    rng = random.Random(seed)
    base, _ = outline_font([None]*257, seed=seed)
    for index in range(count):
        expected = [0]*65536
        segments = []
        cursor = rng.randrange(0, 8)
        for segment in range(rng.randrange(1, 24)):
            low = cursor + rng.randrange(1, 64)
            high = low + rng.randrange(1, 20)
            if index % 7 == 0 and segment == 0:
                low, high = 0, 3
            if segment % 2:
                delta = rng.randrange(65536)
                desired = [rng.randrange(257) if rng.randrange(4) else 0
                           for _ in range(high-low+1)]
                raw = [((gid-delta)&65535) if gid else 0 for gid in desired]
                # raw zero has special meaning even if modulo inversion wanted a nonzero.
                values = [((value+delta)&65535) if value else 0 for value in raw]
            else:
                initial = rng.randrange(1, 200)
                delta = (initial-low) & 65535
                raw = None
                values = list(range(initial, initial+high-low+1))
            segments.append((low,high,delta,raw))
            expected[low:high+1] = values
            cursor = high
        # Force a nonzero raw value that wraps to missing, plus both signed directions.
        low = cursor+64
        segments.append((low, low+3, 65533, [0, 3, 4, 259]))
        expected[low:low+4] = [0, 0, 1, 256]
        data = with_cmap(base, cmap_table([(3, 1, encode(segments))]))
        assert dense(data, 0) == expected
        yield data, dict(index=index, records=[dict(index=0,platform=3,encoding=1,
                                                  format=4,mappings=expected)],
                         sha256=hashlib.sha256(data).hexdigest())


if __name__ == '__main__':
    import json, sys
    data = Path(sys.argv[1]).read_bytes()
    out = records(data)
    for rec in out:
        if rec['format'] == 4:
            rec['mappings'] = dense(data, rec['index'])
    print(json.dumps(dict(records=out), separators=(',', ':')))
