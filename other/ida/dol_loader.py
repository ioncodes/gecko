import struct

import idaapi
import ida_bytes
import ida_entry
import ida_ida
import ida_name
import ida_segment


DOL_HEADER_SIZE = 0x100
NUM_TEXT = 7
NUM_DATA = 11

MEM1_BASE = 0x80000000
MEM1_END = 0x81800000  # 24 MiB
MEM2_BASE = 0x90000000
MEM2_END = 0x94000000  # 64 MiB (Wii)

SYSTEM_REGIONS = [
    (0x80000000, 0x80004000, ".os_lowmem",     "DATA"),
    (0xC0000000, 0xC0004000, ".os_lowmem_unc", "DATA"),
    (0xCC000000, 0xCC008004, ".mmio_gc",       "DATA"),
    (0xCD000000, 0xCD008000, ".mmio_wii",      "DATA"),
]

OS_GLOBALS = {
    0x80000000: ("OSGameCode", 4),
    0x80000004: ("OSMakerCode", 2),
    0x80000006: ("OSDiscNumber", 1),
    0x80000007: ("OSDiscVersion", 1),
    0x80000008: ("OSAudioStreaming", 1),
    0x80000009: ("OSStreamingBufferSize", 1),
    0x80000018: ("OSWiiMagic", 4),
    0x8000001C: ("OSGameCubeMagic", 4),
    0x80000020: ("OSNintendoBootCode", 4),
    0x80000024: ("OSVersion", 4),
    0x80000028: ("OSPhysicalMEM1Size", 4),
    0x8000002C: ("OSConsoleType", 4),
    0x80000030: ("OSArenaLo", 4),
    0x80000034: ("OSArenaHi", 4),
    0x80000038: ("OSFstLocation", 4),
    0x8000003C: ("OSFstMaxLength", 4),
    0x80000040: ("OSDebuggerHook", 4),
    0x80000044: ("OSDebuggerHookSize", 4),
    0x80000048: ("OSCurrentContextPhys", 4),
    0x8000004C: ("OSExceptionHandlerVector", 4),
    0x800000C0: ("OSCurrentContext", 4),
    0x800000C4: ("OSUserInterruptMask", 4),
    0x800000C8: ("OSExceptionType", 4),
    0x800000CC: ("OSVideoMode", 4),
    0x800000D0: ("OSARAMSize", 4),
    0x800000D4: ("OSCurrentFunction", 4),
    0x800000D8: ("OSDefaultThread", 4),
    0x800000DC: ("OSEarliestThread", 4),
    0x800000E0: ("OSLastThread", 4),
    0x800000E4: ("OSCurrentThread", 4),
    0x800000F0: ("OSPhysicalMEM2Size", 4),
    0x800000F4: ("OSConsoleSimulatedMEM2Size", 4),
    0x800000F8: ("OSBusClockSpeed", 4),
    0x800000FC: ("OSCpuClockSpeed", 4),
    0x80003100: ("BI2_PhysicalMEM1Size", 4),
    0x80003104: ("BI2_SimulatedMEM1Size", 4),
    0x8000310C: ("BI2_MEM1ArenaStart", 4),
    0x80003110: ("BI2_MEM1ArenaEnd", 4),
    0x80003118: ("BI2_PhysicalMEM2Size", 4),
    0x8000311C: ("BI2_SimulatedMEM2Size", 4),
    0x80003120: ("BI2_MEM2EndForPPC", 4),
    0x80003124: ("BI2_UsableMEM2Start", 4),
    0x80003128: ("BI2_UsableMEM2End", 4),
    0x80003130: ("BI2_IPCBufferStart", 4),
    0x80003134: ("BI2_IPCBufferEnd", 4),
    0x80003138: ("BI2_HollywoodVersion", 4),
    0x80003140: ("BI2_IOSVersion", 4),
    0x80003144: ("BI2_IOSBuildDate", 4),
    0x80003148: ("BI2_IOSReservedHeapStart", 4),
    0x8000314C: ("BI2_IOSReservedHeapEnd", 4),
    0x80003158: ("BI2_GDDRVendorCode", 4),
    0x8000315C: ("BI2_BootIndicator", 1),
    0x8000315D: ("BI2_LegacyDIModeFlag", 1),
    0x8000315E: ("BI2_DevkitBootProgramVersion", 2),
    0x80003180: ("BI2_GameID", 4),
    0x80003184: ("BI2_ApplicationType", 1),
    0x80003186: ("BI2_ApplicationType2", 1),
    0x80003188: ("BI2_MinimumIOSVersion", 4),
    0x80003198: ("BI2_DataPartitionOffset", 4),
    0x8000319C: ("BI2_DiscLayerType", 1),
}


def _file_size(li):
    here = li.tell()
    li.seek(0, 2)
    size = li.tell()
    li.seek(here)
    return size


def _addr_in_ram(addr):
    return MEM1_BASE <= addr < MEM1_END or MEM2_BASE <= addr < MEM2_END


def _parse_header(raw):
    if len(raw) < 0xE4:
        return None
    text_off = struct.unpack(">7I", raw[0x00:0x1C])
    data_off = struct.unpack(">11I", raw[0x1C:0x48])
    text_addr = struct.unpack(">7I", raw[0x48:0x64])
    data_addr = struct.unpack(">11I", raw[0x64:0x90])
    text_sz = struct.unpack(">7I", raw[0x90:0xAC])
    data_sz = struct.unpack(">11I", raw[0xAC:0xD8])
    bss_addr, bss_size, entry = struct.unpack(">3I", raw[0xD8:0xE4])

    sections = []
    for i in range(NUM_TEXT):
        if text_sz[i]:
            sections.append(("text", i, text_off[i], text_addr[i], text_sz[i]))
    for i in range(NUM_DATA):
        if data_sz[i]:
            sections.append(("data", i, data_off[i], data_addr[i], data_sz[i]))
    return sections, bss_addr, bss_size, entry


def _looks_like_dol(li):
    filesize = _file_size(li)
    if filesize < DOL_HEADER_SIZE:
        return False
    li.seek(0)
    raw = li.read(DOL_HEADER_SIZE)
    parsed = _parse_header(raw)
    if not parsed:
        return False
    sections, bss_addr, bss_size, entry = parsed

    if not sections:
        return False

    for _, _, off, addr, sz in sections:
        if off < DOL_HEADER_SIZE or off + sz > filesize:
            return False
        if not _addr_in_ram(addr):
            return False

    if not _addr_in_ram(entry):
        return False
    if bss_size and not _addr_in_ram(bss_addr):
        return False

    return True


def _add_segment(start, end, name, sclass):
    seg = idaapi.segment_t()
    seg.start_ea = start
    seg.end_ea = end
    seg.bitness = 1  # 32-bit
    seg.align = idaapi.saRelByte
    seg.comb = idaapi.scPub
    ida_segment.add_segm_ex(seg, name, sclass, idaapi.ADDSEG_NOSREG)


def accept_file(li, filename):
    if not _looks_like_dol(li):
        return 0
    return {"format": "Nintendo GameCube/Wii DOL executable"}


def load_file(li, neflags, format):
    li.seek(0)
    raw = li.read(DOL_HEADER_SIZE)
    parsed = _parse_header(raw)
    if not parsed:
        return 0
    sections, bss_addr, bss_size, entry = parsed

    idaapi.set_processor_type("PPC", idaapi.SETPROC_LOADER)
    ida_ida.inf_set_be(True)
    ida_ida.inf_set_app_bitness(32)

    for kind, idx, off, addr, sz in sections:
        name = ".%s%d" % (kind, idx)
        sclass = "CODE" if kind == "text" else "DATA"
        _add_segment(addr, addr + sz, name, sclass)
        li.seek(off)
        ida_bytes.put_bytes(addr, li.read(sz))

    if bss_size:
        _add_segment(bss_addr, bss_addr + bss_size, ".bss", "BSS")
        ida_bytes.put_bytes(bss_addr, b"\x00" * bss_size)

    for start, end, name, sclass in SYSTEM_REGIONS:
        seg = idaapi.segment_t()
        seg.start_ea = start
        seg.end_ea = end
        seg.bitness = 1
        seg.align = idaapi.saRelByte
        seg.comb = idaapi.scPub
        if ida_segment.add_segm_ex(seg, name, sclass, idaapi.ADDSEG_NOSREG):
            ida_bytes.put_bytes(start, b"\x00" * (end - start))

    for ea, (name, size) in OS_GLOBALS.items():
        if not ida_segment.getseg(ea):
            continue
        if size == 1:
            ida_bytes.create_byte(ea, 1)
        elif size == 2:
            ida_bytes.create_word(ea, 2)
        elif size == 4:
            ida_bytes.create_dword(ea, 4)
        ida_name.set_name(ea, name, ida_name.SN_FORCE)

    ida_entry.add_entry(entry, entry, "__start", True)

    return 1
