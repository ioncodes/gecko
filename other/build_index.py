import json
import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).parent

PLATFORMS = {
    "gc": {
        "roms_path": Path("/mnt/roms/Nintendo - GameCube"),
        "output_json": HERE / "game_index.json",
        "output_txt": HERE / "game_index.txt",
        "expected_magic": "gc",
    },
    "wii": {
        "roms_path": Path("/mnt/roms/Nintendo - Wii"),
        "output_json": HERE / "game_index_wii.json",
        "output_txt": HERE / "game_index_wii.txt",
        "expected_magic": "wii",
    },
}

GC_WHITELIST = ["(US"]
GC_BLACKLIST = [
    "Alpha", "Beta", "Proto", "Rev ", "Action Replay", "(v1.", "(Unl)", "Disc 2", "Disc 3", "Demo", "Preview", "Bonus", "Debug"
]

RVZ_MAGIC = b"RVZ\x01"
DISC_HEADER_OFFSET = 0x48 + 0x10
DISC_HEADER_SIZE = 0x80

WII_MAGIC = b"\x5D\x1C\x9E\xA3"
GC_MAGIC = b"\xC2\x33\x9F\x3D"


def is_allowed(name: str, platform: str) -> bool:
    if platform != "gc":
        return True
    lower = name.lower()
    if not any(w.lower() in lower for w in GC_WHITELIST):
        return False
    return not any(b.lower() in lower for b in GC_BLACKLIST)


def read_disc_header(fp) -> bytes:
    magic = fp.read(4)
    assert magic == RVZ_MAGIC, f"bad RVZ magic: {magic!r}"
    fp.seek(DISC_HEADER_OFFSET)
    return fp.read(DISC_HEADER_SIZE)


def parse_header(header: bytes) -> tuple[str, str, str | None]:
    game_code = header[:6].decode("ascii", errors="replace")
    name_bytes = header[0x20:DISC_HEADER_SIZE].split(b"\x00", 1)[0]
    internal_name = name_bytes.decode("utf-8", errors="replace").strip()
    if header[0x18:0x1C] == WII_MAGIC:
        kind = "wii"
    elif header[0x1C:0x20] == GC_MAGIC:
        kind = "gc"
    else:
        kind = None
    return game_code, internal_name, kind


def index_zip(zip_path: Path) -> tuple[str, str, str, str | None] | None:
    with zipfile.ZipFile(zip_path) as zf:
        rvz_entries = [n for n in zf.namelist() if n.lower().endswith(".rvz")]
        if not rvz_entries:
            return None
        with zf.open(rvz_entries[0]) as fp:
            header = read_disc_header(fp)
    game_code, internal_name, kind = parse_header(header)
    return game_code, internal_name, zip_path.stem, kind


def index_rvz(rvz_path: Path) -> tuple[str, str, str, str | None]:
    with rvz_path.open("rb") as fp:
        header = read_disc_header(fp)
    game_code, internal_name, kind = parse_header(header)
    return game_code, internal_name, rvz_path.stem, kind


def build_platform(platform: str) -> None:
    cfg = PLATFORMS[platform]
    roms_path: Path = cfg["roms_path"]
    output_json: Path = cfg["output_json"]
    output_txt: Path = cfg["output_txt"]
    expected_magic: str = cfg["expected_magic"]

    print(f"=== {platform} === scanning {roms_path}")
    collisions: dict[str, list[tuple[str, str]]] = {}
    rom_paths = sorted(p for p in roms_path.iterdir() if p.suffix.lower() in (".zip", ".rvz"))
    for rom_path in rom_paths:
        if not is_allowed(rom_path.name, platform):
            continue
        try:
            entry = index_zip(rom_path) if rom_path.suffix.lower() == ".zip" else index_rvz(rom_path)
        except Exception as e:
            print(f"[skip] {rom_path.name}: {e}")
            continue
        if entry is None:
            continue
        game_code, internal_name, stem, kind = entry
        if kind != expected_magic:
            print(f"[skip] {rom_path.name}: expected {expected_magic} disc, got {kind!r}")
            continue
        collisions.setdefault(game_code, []).append((internal_name, stem))
        print(f"{game_code}  {internal_name}  ({stem})")

    dupes = {code: entries for code, entries in collisions.items() if len(entries) > 1}
    print(f"\nindexed {len(collisions)} unique game codes across {sum(len(v) for v in collisions.values())} zips")
    if dupes:
        print(f"\n{len(dupes)} duplicate game code(s):")
        for code, entries in sorted(dupes.items()):
            print(f"  {code}:")
            for internal_name, stem in entries:
                print(f"    - {internal_name}  ({stem})")
    else:
        print("\nno duplicate game codes")

    index = {
        code: [{"internal_name": name, "filename": stem} for name, stem in entries]
        for code, entries in sorted(collisions.items())
    }
    output_json.write_text(json.dumps(index, indent=2, ensure_ascii=False), encoding="utf-8")
    print(f"\nwrote {output_json}")

    lines = [f"{code}={entries[0][1]}" for code, entries in sorted(collisions.items())]
    output_txt.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {output_txt}")


def main() -> None:
    args = sys.argv[1:]
    targets = args if args else list(PLATFORMS)
    unknown = [a for a in targets if a not in PLATFORMS]
    if unknown:
        print(f"unknown platform(s): {', '.join(unknown)}; valid: {', '.join(PLATFORMS)}")
        sys.exit(2)
    for platform in targets:
        build_platform(platform)


if __name__ == "__main__":
    main()
