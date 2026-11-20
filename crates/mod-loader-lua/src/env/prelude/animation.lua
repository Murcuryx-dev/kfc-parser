-- game.animation: bringing animation clips into the game from files of the mod.
-- This chunk runs once in every mod's environment; `game`, `io` and `error` are that mod's.

local JOINT_ANIMATION = "keen::JointAnimation"
local ROOT_MOTION = "keen::RootMotionAnimation"
local EVENTS = "keen::AnimationSequenceContainer"

local function number_of(value)
    if type(value) == "number" then return value end
    local ok, inner = pcall(function() return value.value end)
    if ok and type(inner) == "number" then return inner end
    return tonumber(tostring(value))
end

local function fail(message)
    error("game.animation: " .. message, 3)
end

-- A template may be given as a resource or as its id.
local function resource_of(template, type_name, what)
    if type(template) == "string" then
        local resource = game.assets.get_resource(template, type_name)
        if not resource then fail(what .. " " .. template .. " (" .. type_name .. ") was not found") end
        return resource
    end
    if template == nil then fail("no " .. what .. " given") end
    return template
end

--- Reads the header of a .manm clip file. The clip data itself is not copied: `buffer`
--- is left standing at its first byte.
local function read(file)
    local buffer = io.read(file)
    local function u8() return buffer:read_u8() end
    local function u16() return buffer:read_u16() end
    local function u32() return buffer:read_u32() end
    local function f32() return buffer:read_f32() end

    if buffer:remaining() < 28 or string.char(u8(), u8(), u8(), u8()) ~= "MANM" then
        fail(file .. " is not a .manm clip file")
    end
    local version = u32()
    if version ~= 1 then fail(file .. " is .manm version " .. version .. ", this loader reads version 1") end

    local clip = { file = file, frameCount = u16(), jointCount = u16(), flags = u16() }
    local segments = u16()
    clip.positionFactor = f32(); clip.positionTangent = f32(); clip.scaleFactor = f32()
    clip.headers = {}
    for i = 1, segments do
        clip.headers[i] = {
            dataOffset = u32(), startTime = u16(), endTime = u16(),
            usedOrientationJoints = u16(), nonConstOrientationJoints = u16(),
            usedPositionJoints = u16(), nonConstPositionJoints = u16(),
            usedScaleJoints = u16(), nonConstScaleJoints = u16(),
        }
    end
    local bytes = u32()
    if bytes % 16 ~= 0 or buffer:remaining() ~= bytes then
        fail(file .. " is cut off or damaged: it says " .. bytes .. " bytes of clip data and has " .. buffer:remaining())
    end
    clip.blocks = bytes // 16
    clip.buffer = buffer
    return clip
end

-- One position per line, "x y z" in metres, game space; one line per frame plus one.
local function read_root_motion(file, frames)
    local points = {}
    for x, y, z in io.read(file):to_string():gmatch("([%-%d%.eE+]+)%s+([%-%d%.eE+]+)%s+([%-%d%.eE+]+)") do
        points[#points + 1] = { tonumber(x), tonumber(y), tonumber(z) }
    end
    if #points ~= frames + 1 then
        fail(file .. " has " .. #points .. " positions, the clip needs " .. (frames + 1) .. " (one per frame and one more)")
    end
    return points
end

--- Makes a new animation clip from a .manm file of the mod.
---
--- `template` is a clip of the game for the same skeleton (a resource or its id). The new
--- clip is a copy of it with the file's animation in place of the template's.
---
--- options (all optional):
---   id                    the id of the new clip; without it the loader gives one
---   same_length           true: fail unless the file has as many frames as the template
---   events                true: the template's events (footsteps, sounds) are copied for the new clip
---   root_motion           a file with the movement of the clip, one "x y z" per line
---   root_motion_template  a movement track of the game to copy the rest from (needed with root_motion)
---
--- Returns a table: id, resource, frames, joints, events (true when copied), root_motion (its resource).
local function import(file, template, options)
    options = options or {}
    local source = resource_of(template, JOINT_ANIMATION, "the template clip")
    local clip = read(file)

    local joints = number_of(source.data.jointCount)
    if clip.jointCount ~= joints then
        fail(file .. " is for another skeleton: it has " .. clip.jointCount .. " joints, the template has " .. joints)
    end
    if options.same_length and clip.frameCount ~= number_of(source.data.frameCount) then
        fail(file .. " has " .. clip.frameCount .. " frames, the template has " .. tostring(source.data.frameCount))
    end

    local resource
    if options.id then
        if game.assets.get_resource(options.id, JOINT_ANIMATION) then fail("a clip with the id " .. options.id .. " exists already") end
        resource = game.assets.create_resource(source.data, JOINT_ANIMATION, options.id, 0)
    else
        resource = game.assets.create_resource(source.data, JOINT_ANIMATION)
    end
    local id = resource.guid

    local data = resource.data
    data.headers = clip.headers
    data.data = clip.buffer          -- the buffer's unread bytes are the clip data, taken as they are
    data.frameCount = clip.frameCount
    data.flags = clip.flags
    data.positionFactor = clip.positionFactor
    data.positionTangent = clip.positionTangent
    data.scaleFactor = clip.scaleFactor

    data = resource.data
    if #data.data ~= clip.blocks or #data.headers ~= #clip.headers or number_of(data.frameCount) ~= clip.frameCount then
        fail("the data of " .. file .. " did not arrive in the new clip")
    end

    local result = { id = id, resource = resource, frames = clip.frameCount, joints = joints, events = false }

    if options.events then
        local events = game.assets.get_resource(source.guid, EVENTS)
        if events then
            game.assets.create_resource(events.data, EVENTS, id, 0)
            result.events = true
        end
    end

    if options.root_motion then
        local points = read_root_motion(options.root_motion, clip.frameCount)
        local track_source = resource_of(options.root_motion_template, ROOT_MOTION, "the root motion template")
        local track = game.assets.create_resource(track_source.data, ROOT_MOTION, id, 0)

        -- as many positions and orientations as the clip needs, each a copy of the template's
        -- first one; then the positions of the file
        local positions, orientations = {}, {}
        for i = 1, #points do
            positions[i] = track.data.positions[1]
            orientations[i] = track.data.orientations[1]
        end
        track.data.positions = positions
        track.data.orientations = orientations

        local motion = track.data
        for i = 1, #points do
            local position = motion.positions[i]
            position.x = points[i][1]; position.y = points[i][2]; position.z = points[i][3]
        end
        local last = points[#points]
        motion.frameCount = clip.frameCount
        -- the game's own tracks hold the way back from the end to the start
        motion.startToEnd.x = -last[1]; motion.startToEnd.y = -last[2]; motion.startToEnd.z = -last[3]

        motion = track.data
        if #motion.positions ~= #points or #motion.orientations ~= #points or number_of(motion.frameCount) ~= clip.frameCount then
            fail("the movement of " .. options.root_motion .. " did not arrive in the new track")
        end
        result.root_motion = track
        result.travel = { x = last[1], y = last[2], z = last[3] }
    end

    return result
end

game.animation = { read = read, import = import }
