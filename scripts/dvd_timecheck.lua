local function timebase_log(emu)
    local r3 = emu:gpr(3)
    local r4 = emu:gpr(4)
    local r9 = emu:gpr(9)
    local r10 = emu:gpr(10)
    log(string.format("[timebase] result=%08X time=%08X ref1=%08X ref2=%08X taken=%s",
        r3, r4, r9, r10, r3 ~= 0 and "yes" or "no"))
end

traps = {
    cpu_pre = {
        [0x81300BD8] = timebase_log,
    },
    bus_write_pre = {
        virt = {
             [0x8145d548] = timebase_log,
        },
    },
}