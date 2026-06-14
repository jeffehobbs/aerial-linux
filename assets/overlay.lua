-- aerial-linux overlay script.
--
-- Renders up to three OSD overlays over the aerial video:
--   * clock     — top-right, computed here from the local clock (1 Hz)
--   * weather   — top-left, read from the JSON state file
--   * now playing — bottom-left, read from the JSON state file
--
-- The state file is written by the Rust side (see src/overlay.rs); its path
-- comes from $AERIAL_OVERLAY_FILE. We only *render* here so that anything
-- needing network/D-Bus (weather, MPRIS) stays in Rust.

local utils = require 'mp.utils'
local state_file = os.getenv("AERIAL_OVERLAY_FILE")

local clock_ov = mp.create_osd_overlay("ass-events")
local weather_ov = mp.create_osd_overlay("ass-events")
local np_ov = mp.create_osd_overlay("ass-events")

local function read_state()
    if not state_file then return {} end
    local f = io.open(state_file, "r")
    if not f then return {} end
    local content = f:read("*all")
    f:close()
    return utils.parse_json(content or "") or {}
end

-- Escape ASS-significant characters in externally-sourced strings.
local function esc(s)
    s = s:gsub("\\", "\\\239\187\191") -- temp marker to avoid double-escaping
    s = s:gsub("{", "\\{"):gsub("}", "\\}")
    s = s:gsub("\\239\187\191", "\\\\")
    return s
end

local function set(ov, data)
    ov.data = data or ""
    ov:update()
end

-- Shared text treatment: white fill, no hard outline — contrast comes from a
-- soft, blurred drop shadow (a thin blurred halo keeps it legible over bright
-- skies too). `\an<n>` (corner) and `\fs<n>` (size) are prepended per element.
local function style(font)
    return "\\fn" .. font
        .. "\\1c&HFFFFFF&"                       -- white text
        .. "\\bord1\\3c&H000000&\\3a&H80&"       -- faint, half-transparent halo
        .. "\\shad1\\4c&H000000&\\4a&HA0&"       -- light, mostly-transparent shadow
        .. "\\blur5"                             -- soft edges
end

local function draw()
    local st = read_state()
    local font = (st.font and st.font ~= "") and st.font or "Inter"
    local s = style(font)

    if st.show_clock ~= false then
        set(clock_ov, string.format(
            "{\\an9%s\\b1\\fs64}%s\\N{\\fs28\\b0}%s",
            s, os.date("%H:%M"), os.date("%a %d %b")))
    else
        set(clock_ov, "")
    end

    if st.weather and st.weather ~= "" then
        set(weather_ov, "{\\an7" .. s .. "\\fs34}" .. esc(st.weather))
    else
        set(weather_ov, "")
    end

    if st.now_playing and st.now_playing ~= "" then
        set(np_ov, "{\\an1" .. s .. "\\fs30}" .. esc(st.now_playing))
    else
        set(np_ov, "")
    end
end

mp.add_periodic_timer(1, draw)
draw()
