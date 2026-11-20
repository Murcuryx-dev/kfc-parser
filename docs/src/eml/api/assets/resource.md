# Resource

These are rather small data files that contain typed binary data.
They are referenced by a GUID, a qualified type name and a part index.
The part index of a resource is used to distinguish between multiple resources with the same guid and type that belong together.
The way how part indices are used is specific to the type of resource.
For example, the voxel chunks of a scene are made up of multiple parts where all have the same type and guid, but have different part indices that specify their position.  

They are provided as lua table-like userdata objects.
The structure of these objects is defined in the `types.lua` definition file.

## Accessing Resources

To access resources, you can use one of the `game.assets.get_resource*` functions.
Each of these functions will return either a single or a list of [`Resource`](#resource-object) objects.

Check `AssetManager` in the `base.lua` definition file for more information about the individual functions.

## Resource Object

Resource objects contain some metadata fields as well as the actual data of the resource stored in the `data` field.
When accessing the `data` field, it will return a typed userdata object depending on the type of the resource.
It acts like a regular lua table, but with some additional functionality.
When modifying the data of a resource (i.e., changing fields or adding/removing entries in arrays), these changes will be reflected in the resource itself.

It can also be assigned everywhere where a `Guid` or `ObjectReference` is expected, so you can pass it directly to functions and fields that expect these types without having to extract the guid manually.

**Important:** If you assign a reference value (i.e., a table or a userdata) to a field, it will create a deep copy of the value.
So subsequent modifications to the original value will **not** affect the resource's data.

## Creating Resources

You can create new resources using the `game.assets.create_resource` function.
There are two variants of this function. One for creating a new resource with a new guid, and one for creating additional parts for a given resource.

The first one takes two arguments:
- `value`: A value that is compatible with the specified type.
- `type`: The qualified type name (or a `Type` object) of the resource to create.

It returns a new [`Resource`](#resource-object) object that contains the specified data with a newly generated guid and a part index of `0`.

The second one takes two **additional** arguments:
- `guid`: The guid of the resource to create a new part for.
- `part_index`: The part index of the new resource part.

As described in the documentation comment in the definition file, this function should be called after creating a new resource with the first variant to create additional parts of the same resource.
If the resulting resource is not unique (i.e., there is already a resource with the same guid, type and part index), it will raise an error.

Check `AssetManager` in the `base.lua` definition file for more information about these functions.

## Arrays

An array of a resource is used like a Lua list. `#`, `ipairs`, `pairs` and the functions
`table.insert`, `table.remove`, `table.clear`, `table.unpack`, `table.concat`, `table.sort`
and `table.move` work on it. The end of an array can also be changed the way a Lua list is:

```lua
local events = resource.data.events

events[#events + 1] = events[1]  -- append a copy of the first element
events[#events] = nil            -- remove the last element
```

Arrays of a fixed size cannot grow or shrink.

An array of plain numbers (or of structs that only hold numbers) can be filled from a
buffer in one step, for example from a file of the mod. The buffer's unread bytes are
taken as the elements in their binary layout:

```lua
resource.data.samples = io.read("samples.bin")
```

## New resources

`game.assets.create_resource(value, type)` gives the new resource an id of its own. The id
is derived from the id of the mod and from how many resources the mod has created so far,
so the resource has the same id every time the game starts as long as the mod creates its
resources in the same order.
