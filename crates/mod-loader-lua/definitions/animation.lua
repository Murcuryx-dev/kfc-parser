--- @meta

--- Brings animation clips into the game from `.manm` files of the mod.
---
--- @class AnimationHelper
local AnimationHelper = {}

--- @class AnimationImportOptions
--- @field id Guid? -- The id of the new clip. Without it the loader gives one.
--- @field same_length boolean? -- Fail unless the file has as many frames as the template.
--- @field events (boolean|Guid|Resource)? -- `true`: copy the template's events (footsteps, sounds) for the new clip. The id of another clip, or its `keen::AnimationSequenceContainer`: copy that clip's events.
--- @field root_motion string? -- A file with the movement of the clip: one "x y z" per line, in metres, one line per frame and one more.
--- @field root_motion_template (Guid|Resource)? -- A `keen::RootMotionAnimation` of the game to copy the rest of the track from. Needed with `root_motion`.

--- @class ImportedAnimation
--- @field id Guid -- The id of the new clip (and of its events and movement track).
--- @field resource Resource -- The new `keen::JointAnimation`.
--- @field frames integer
--- @field joints integer
--- @field events boolean -- Whether the template had events that were copied.
--- @field root_motion Resource? -- The new `keen::RootMotionAnimation`.
--- @field travel { x: number, y: number, z: number }? -- Where the movement ends.

--- @class AnimationFile
--- @field file string
--- @field frameCount integer
--- @field jointCount integer
--- @field flags integer
--- @field positionFactor number
--- @field positionTangent number
--- @field scaleFactor number
--- @field headers table[]
--- @field blocks integer -- The number of 16-byte blocks of clip data.
--- @field buffer Buffer -- Stands at the first byte of the clip data.

--- Makes a new animation clip from a `.manm` file of the mod.
---
--- The new clip is a copy of `template`, a clip of the game for the same skeleton, with
--- the file's animation in place of the template's.
---
--- @param file string -- The path of the `.manm` file inside the mod.
--- @param template Guid|Resource -- A `keen::JointAnimation` of the game for the same skeleton.
--- @param options AnimationImportOptions?
--- @return ImportedAnimation
function AnimationHelper.import(file, template, options) end

--- Reads the header of a `.manm` file without making anything.
---
--- @param file string -- The path of the `.manm` file inside the mod.
--- @return AnimationFile
function AnimationHelper.read(file) end
