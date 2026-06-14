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

local function draw()
    local st = read_state()

    if st.show_clock ~= false then
        set(clock_ov, string.format(
            "{\\an9\\fs64\\bord3\\shad1\\1c&HFFFFFF&}%s\\N{\\fs28}%s",
            os.date("%H:%M"), os.date("%a %d %b")))
    else
        set(clock_ov, "")
    end

    if st.weather and st.weather ~= "" then
        set(weather_ov, "{\\an7\\fs34\\bord2\\shad1}" .. esc(st.weather))
    else
        set(weather_ov, "")
    end

    if st.now_playing and st.now_playing ~= "" then
        set(np_ov, "{\\an1\\fs30\\bord2\\shad1}" .. esc(st.now_playing))
    else
        set(np_ov, "")
    end
end

mp.add_periodic_timer(1, draw)
draw()
