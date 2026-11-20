# Enshrouded Modding Tools

A collection of unofficial tools for modding the game Enshrouded.

## About this fork

This is a fork of [Brabb3l/kfc-parser](https://github.com/Brabb3l/kfc-parser) kept by Murcuryx.
Mods written for EML 0.2.0 load unchanged. What is different:

| | |
|---|---|
| No limit on how much a mod may touch | EML 0.2.0 stops with "cannot create a Lua reference" once the mods of one start have looked at about a million values. That limit is gone. |
| A failing mod does not stop the game | The game starts with the files of the last start that worked and a message names the mod and the error. |
| Game files are written safely | New files are written beside the old ones and swapped in at the end. A start that is interrupted leaves the old files as they were. |
| Load order | `dependencies` are checked and run first, and `run_after` in `mod.json` lets a mod run after another one. |
| Version conflicts | A mod that needs another version of a mod than the one installed is named, and you choose: start without it, or do not start. |
| Internal errors | An error inside the loader itself is reported and the game still starts. |
| Reproducible builds | The same source and tools give the same files, byte for byte, with no path of the build machine inside. |
| Fast start when nothing changed | A mod file is only read again when its size or time changed. A log file written into a mod folder does not cause a new patch run. |
| The same mods give the same game files | Byte for byte, on every start. Resources a mod creates keep their ids. |
| `emm check` | Runs everything a game start runs and writes nothing: shows whether the mods would load. |
| A report per mod | Time and memory use of every mod in the log. |
| Binary data in one step | An array of numbers can be filled from a buffer instead of one Lua table per element. |
| Lua lists | `a[#a + 1] = v` and `a[#a] = nil` work on the game's arrays. |
| Animations | `game.animation.import` makes a clip of the game from a `.manm` file of the mod, with events and root motion. |
| Shared resources | The log and `.cache/shared_resources.txt` name the resources that more than one mod changed, and in which order. |

### What the loader does on your computer

The loader has no network code. It does not connect to anything, does not download or
update anything, and does not send anything. It has no updater. It starts no other
program, changes nothing in the Windows registry and does not read the keyboard or
the mouse. It reads and writes files in the game folder, which is where the mods, the
cache, the log and the patched game data are, and nowhere else. Exports go to the
export folder you set.

A mod is Lua code with no access to the operating system: it has no `os` library, no
`io` library other than reading files of its own folder and writing into the export
folder, and it cannot load code at runtime. The `runtime` capability that would let a
mod load a DLL into the game is not active in this version, as in EML 0.2.0.

If you check the files with a tool that lists imported functions you will see Windows
socket functions (`ws2_32.dll`). Every program built with the Rust standard library for
this target that opens a file has them, including one that does nothing else. Nothing in
the loader or in the libraries it is built from calls them, and you can check that in the source.

The safest way to trust a build is to make it yourself:

```
rustup toolchain install stable
rustup target add x86_64-pc-windows-gnu
cargo build --release --locked --target x86_64-pc-windows-gnu -p dbghelp-proxy -p mod-manager-cli
```

The files are then in `target/x86_64-pc-windows-gnu/release`: `dbghelp.dll` goes next to
`enshrouded.exe`, `emm.exe` is the same loader as a program.
On Windows the `gnu` target needs MinGW-w64 (for example through MSYS2) on the PATH.

## Enshrouded Mod Loader (EML)

A tool used to load mods for the game Enshrouded.  

Documentation can be found [here](https://brabb3l.github.io/kfc-parser).  

## Discord Server

Join our [Discord server](https://discord.gg/HKKyeMsKfW) to talk about EML and Enshrouded modding in general.

## KFC Parser CLI (deprecated)

- Unpacking and repacking (1:1) all types of descriptor files.
- Extracting reflection data from the enshrouded executable. (Only windows x64)
- Disassembling and assembling of impact programs.

Refer to the [CLI documentation](crates/cli-legacy/README.md) for usage instructions.
