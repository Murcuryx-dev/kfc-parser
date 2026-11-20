# Setup

This document provides instructions on how to set up a modding environment for
the game Enshrouded using the Enshrouded Mod Loader (EML).

## Prerequisites

- Ensure you have a legal copy of the game Enshrouded installed on your system.
- Download the latest version of the Enshrouded Mod Manager CLI from [here](https://github.com/Brabb3l/kfc-parser/actions/workflows/build_release.yml).
- A text editor or IDE of your choice, preferably with [lua_ls](https://luals.github.io/) (e.g., Visual Studio Code).
- Basic knowledge of Lua programming.

## Creating a Mod

The mod manager cli can be used to create a new mod template.

```bash
emm.exe create -g <game-dir>
```

You will be prompted to enter a few details about your mod which will be placed
within the `mod.json` [manifest file](#mod-manifest).

After the command completes, a new directory named after your mod id will be
created in the mods directory of your game installation.
It will also generate definition files for the Lua language server to provide
autocompletion and type checking. They are located in `<game-dir>/.cache/lua`.

### Mod Structure

```
<mod-id>/
├── mod.json
├── README.md
├── .luarc.json
└── src/
    └── mod.lua
```

- `mod.json`: The manifest file containing metadata about your mod.
- `README.md`: A markdown file where you can provide a detailed description of your mod.
- `.luarc.json`: Configuration file for the Lua language server.
  If you don't use lua_ls or use another language server, you can safely delete this file.
- `src/mod.lua`: The main Lua script file where you will write your mod's code

### Mod Manifest

The `mod.json` file contains metadata about your mod and is essential for EML to
recognize and load it. Below is a detailed explanation of each field in the manifest:

- `id` (string, required): A **unique** identifier for your mod. The id may only
  contain lowercase letters, numbers, hyphens (`-`), and underscores (`_`).
  It must start with a letter.
- `name` (string, required): The display name of your mod.
  This is what users will see in the mod manager.
- `version` (string, required): The version of your mod, following [semantic versioning](https://semver.org/).
- `capabilities` (string list, required): A list of capabilities that your mod requires.
  Valid capabilities include:
  - `patch` (lua): Allows the mod to patch game data.
  - `export` (lua): Allows the mod to export arbitrary data into an export directory.
  - `runtime` (WIP): Allows the mod to run code at runtime.

- `description` (string, optional): A brief description of what your mod does.
  A detailed description can be provided in a separate `README.md` file.
- `authors` (string list, optional): A list of authors who contributed to the mod.
- `license` (string, optional): The license under which your mod is released (e.g., MIT, GPL-3.0).
  You may also include a `LICENSE` file in your mod directory.
- `icon` (string, optional): Path to an icon file (e.g., `icon.png`) that represents your mod.
  This icon will be displayed in the mod manager.
- `dependencies` (list, optional): Other mods that your mod needs. Each entry has an `id`,
  a `version` requirement (for example `"^1.2"` or `"*"`) and, if the mod also works
  without it, `"optional": true`.
  A dependency always runs before your mod. If a dependency that is not optional is
  missing, no mod is applied and the user is told which mod is missing.
  If a dependency is installed in another version than your mod asks for, the user is
  told which mod needs which version and chooses at every start of the game: start
  without your mod (and without the mods that need it), or do not start the game.
  With `emm run` this is an error unless `--skip-version-conflicts` is given.
- `run_after` (string list, optional): Ids of mods that your mod runs after when they
  are installed. Nothing happens when one of them is not installed. Use it when your
  mod changes the same data as another mod and has to see that mod's changes.

Mods that do not have to wait for each other run in the order of their ids. The order
is written to the log (`Mod order`). Mods that wait for each other in a circle are not
applied and the circle is named.

```json
{
  "id": "my_mod",
  "name": "My Mod",
  "version": "1.0.0",
  "capabilities": ["patch"],
  "dependencies": [{ "id": "some_library", "version": "^2.0" }],
  "run_after": ["another_mod"]
}
```

## Writing Your Mod

The main script file for your mod is located at `src/mod.lua`. This is the
entry point for your mod's code.  

Refer to the definition files in `<game-dir>/.cache/lua` for available
functions and types provided by EML and the game or check the examples
in the `examples` directory of the repository.
