#!/usr/bin/env python3
"""Pack generated Rust outline tables into the engine's packed outline files.

usage: pack_outlines.py OUT_DIR TABLE.rs [TABLE.rs ...]

Each TABLE.rs is what engine/oracle/generate-builtin-outlines.py or
generate-substitute-outlines.py writes: `static <FACE>_COMMANDS: [BuiltinOutlineCommand; N]`
and `static <FACE>: [BuiltinOutline; M]` per face. One `<face>.bin` is written per face
(src/builtin_outlines/README.md gives the format). The packer re-reads what it wrote and
compares it with the parsed tables before it exits 0.
"""
import re, struct, sys, os

MAGIC = b"TPDFOUT1"
TAGS = {"MoveTo": 0, "LineTo": 1, "CubicTo": 2, "Close": 3}
COORDS = {"MoveTo": ("x", "y"), "LineTo": ("x", "y"), "CubicTo": ("x1", "y1", "x2", "y2", "x", "y"), "Close": ()}
MAX_NAME = 128

def parse(text):
    faces = {}
    for m in re.finditer(r"static (\w+?)(_COMMANDS)?: \[(BuiltinOutlineCommand|BuiltinOutline); (\d+)\] = \[(.*?)\n\];", text, re.S):
        face, is_cmd, _kind, count, body = m.group(1), m.group(2), m.group(3), int(m.group(4)), m.group(5)
        entry = faces.setdefault(face, {})
        if is_cmd:
            cmds = []
            for c in re.finditer(r"BuiltinOutlineCommand::(\w+)(?:\s*\{([^}]*)\})?", body):
                kind, fields = c.group(1), dict(re.findall(r"(\w+): (-?\d+)", c.group(2) or ""))
                cmds.append((kind, tuple(int(fields[k]) for k in COORDS[kind])))
            assert len(cmds) == count, (face, len(cmds), count)
            entry["commands"] = cmds
        else:
            glyphs = []
            for g in re.finditer(r'BuiltinOutline \{ face: BuiltinFace::\w+, name: b"([^"\\]*)", native_gid: (\d+), command_start: (\d+), command_len: (\d+) \}', body):
                glyphs.append((g.group(1).encode(), int(g.group(2)), int(g.group(3)), int(g.group(4))))
            assert len(glyphs) == count, (face, len(glyphs), count)
            entry["glyphs"] = glyphs
    return faces

def pack(commands, glyphs):
    out = bytearray(MAGIC) + struct.pack("<II", len(commands), len(glyphs))
    for kind, coords in commands:
        out += struct.pack("<B", TAGS[kind])
        for v in coords:
            assert -32768 <= v <= 32767, v
            out += struct.pack("<h", v)
    previous = None
    for name, gid, start, n in glyphs:
        assert 0 < len(name) <= MAX_NAME and (previous is None or previous < name), name
        assert start + n <= len(commands), name
        previous = name
        out += struct.pack("<B", len(name)) + name + struct.pack("<HII", gid, start, n)
    return bytes(out)

def unpack(blob):
    assert blob[:8] == MAGIC
    ncmd, nglyph = struct.unpack_from("<II", blob, 8); at = 16
    inverse = {v: k for k, v in TAGS.items()}; commands = []
    for _ in range(ncmd):
        kind = inverse[blob[at]]; at += 1
        coords = struct.unpack_from("<%dh" % len(COORDS[kind]), blob, at); at += 2 * len(COORDS[kind])
        commands.append((kind, tuple(coords)))
    glyphs = []
    for _ in range(nglyph):
        n = blob[at]; at += 1; name = blob[at:at + n]; at += n
        gid, start, cnt = struct.unpack_from("<HII", blob, at); at += 10
        glyphs.append((name, gid, start, cnt))
    assert at == len(blob)
    return commands, glyphs

def main():
    out_dir, tables = sys.argv[1], sys.argv[2:]
    os.makedirs(out_dir, exist_ok=True)
    for table in tables:
        for face, entry in parse(open(table, encoding="utf-8").read()).items():
            blob = pack(entry["commands"], entry["glyphs"])
            assert unpack(blob) == (entry["commands"], entry["glyphs"]), face
            path = os.path.join(out_dir, face.lower() + ".bin")
            with open(path, "wb") as f: f.write(blob)
            print(f"{path}: {len(entry['commands'])} commands, {len(entry['glyphs'])} glyphs, {len(blob)} bytes")

if __name__ == "__main__":
    main()
