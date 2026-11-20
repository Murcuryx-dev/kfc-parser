# Animation

`game.animation` brings animation clips into the game from files of your mod.

A clip of the game is a `keen::JointAnimation` resource. Making one by hand means reading
your own file format in Lua and filling the resource field by field. `game.animation.import`
does that in one call from a `.manm` file.

```lua
local clip = game.animation.import("my_attack.manm", "05d34456-0e6b-441b-b3be-2f8824e55e01", {
    events = true,
})

print(clip.id, clip.frames)
```

The second argument is the template: a clip of the game for the same skeleton, as a
resource or as its id. The new clip is a copy of the template with the animation of the
file in place of the template's. A file made for another skeleton (another number of
joints) is refused.

## Options

| option | meaning |
|---|---|
| `id` | The id of the new clip. Without it the loader gives one, the same at every start. |
| `same_length` | `true`: fail unless the file has as many frames as the template. |
| `events` | `true`: the template's events (footsteps, sounds), a `keen::AnimationSequenceContainer` with the template's id, are copied for the new clip. The id of another clip of the game: that clip's events are copied, which fails if it has none. |
| `root_motion` | A text file with the movement of the clip (see below). Makes a `keen::RootMotionAnimation` with the id of the clip. |
| `root_motion_template` | A movement track of the game, as a resource or its id. Everything the text file does not give is copied from it. Needed with `root_motion`. |

## What comes back

A table with `id`, `resource` (the new clip), `frames`, `joints`, `events` (whether the
template had events that were copied), and with root motion `root_motion` (the new track)
and `travel` (`x`, `y`, `z`: where the movement ends).

The clip is not used by anything yet. Put its id into an animation graph node or wherever
the game should play it.

`game.animation.read(file)` reads only the header of a file and makes nothing. It returns
`frameCount`, `jointCount`, `flags`, `positionFactor`, `positionTangent`, `scaleFactor`,
`headers`, `blocks` and `buffer`.

## The `.manm` file, version 1

All numbers are little-endian. The file holds exactly what a `keen::JointAnimation` holds,
in the game's own packed form.

| bytes | type | field |
|---|---|---|
| 4 | text | `MANM` |
| 4 | u32 | version, `1` |
| 2 | u16 | `frameCount` |
| 2 | u16 | `jointCount` |
| 2 | u16 | `flags` |
| 2 | u16 | number of segments |
| 4 | f32 | `positionFactor` |
| 4 | f32 | `positionTangent` |
| 4 | f32 | `scaleFactor` |
| 20 each | | one header per segment (below) |
| 4 | u32 | number of bytes of clip data, a multiple of 16 |
| | | the clip data |

A segment header:

| bytes | type | field |
|---|---|---|
| 4 | u32 | `dataOffset` |
| 2 | u16 | `startTime` |
| 2 | u16 | `endTime` |
| 2 | u16 | `usedOrientationJoints` |
| 2 | u16 | `nonConstOrientationJoints` |
| 2 | u16 | `usedPositionJoints` |
| 2 | u16 | `nonConstPositionJoints` |
| 2 | u16 | `usedScaleJoints` |
| 2 | u16 | `nonConstScaleJoints` |

The clip data is the game's `data` array as it is: blocks of 16 bytes. Nothing may follow it.

## The root motion file

Plain text, one position per line: `x y z` in metres, in the space the game's own tracks
use. One line per frame of the clip and one more (a clip of 150 frames has 151 lines). The
first line is the start, normally `0 0 0`.

The new track gets these positions, the orientation of the template's first frame for
every frame, the frame count of the clip, and as `startToEnd` the way back from the last
position to the start, as the game's own tracks have it.
