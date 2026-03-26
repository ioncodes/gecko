STATE_MACHINE_VALUES = {
    [0] = "BS2_START",
    [1] = "BS2_WAIT_DVD",
    [2] = "BS2_WAKE_DVD",
    [3] = "BS2_LOAD_DISKID",
    [4] = "BS2_WAIT_DISKID",
    [5] = "BS2_CONFIRMED_COVER_CLOSED",
    [6] = "BS2_CONFIG_AUDIO_BUFFER",
    [7] = "BS2_WAIT_AUDIO_BUFFER",
    [8] = "BS2_LOAD_APPLOADER_HEADER",
    [9] = "BS2_WAIT_APPLOADER_HEADER",
    [10] = "BS2_LOAD_APPLOADER",
    [11] = "BS2_WAIT_APPLOADER",
    [12] = "BS2_DRIVE_APPLOADER",
    [13] = "BS2_WAIT_APPLOADER_LOAD",
    [14] = "BS2_LOAD_BANNER",
    [15] = "BS2_WAIT_BANNER",
    [16] = "BS2_RUN_APP",
    [17] = "BS2_COVER_CLOSED",
    [18] = "BS2_NO_DISK",
    [19] = "BS2_COVER_OPEN",
    [20] = "BS2_STOP_MOTOR",
    [21] = "BS2_WAIT_STOP_MOTOR",
    [22] = "BS2_WRONG_DISK",
    [23] = "BS2_FATAL_ERROR",
    [24] = "BS2_RETRY",
    [25] = "BS2_CANCELING",
    [26] = "BS2_MAX_STATE"
}

-- local function log_state_machine(emu)
--     local r3 = emu:gpr(0)
--     local state = STATE_MACHINE_VALUES[r3] or "Unknown"
--     log(string.format("[dvd] state machine=%d (%s)", r3, state))
-- end

local function log_state_machine_write(emu, virt_addr, phys_addr, size, value)
    local state = STATE_MACHINE_VALUES[value] or "Unknown"
    log(string.format("[dvd] state machine write: value=%d (%s) from pc=%08X", value, state, emu:pc()))
end

traps = {
    -- cpu_post = {
    --     [0x81300A9C] = log_state_machine,
    -- },
    bus_write_pre = {
        virt = {
             [0x8145d548] = log_state_machine_write,
        },
    },
}