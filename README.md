<div align="center">

# Gecko

A WIP GameCube/Wii emulator and debugger written in Rust.

<img src="images/wario.png" width="30%"> <img src="images/sunshine.png" width="30%"> <img src="images/windwaker.png" width="30%">
<img src="images/luigi.png" width="30%"> <img src="images/re4.png" width="30%"> <img src="images/debugger.png" width="30%">

</div>

## Status

Gecko is in early development. The IPL works flawlessly as far as I can tell. Many homebrew demos work, but game compatibility is still low. Some games may get to menu, some ingame but most will likely not do anything (or crash). Gecko is made with homebrew development and reverse engineering in mind. It currently supports:

- PowerPC interpreter
- DSP LLE interpreter
- IPL skip patches for NTSC and PAL
- `wgpu` based renderer backend
- `wesl` based shader compiler
- LUA scripting/hooks system for runtime introspection
- Probably the prettiest egui-based debugging UI for GameCube and Wii
- Symbol parsing from ELFs and IDA Pro databases
- [Support for web browser](https://gecko.layle.dev)
  - [incl. debugging capabilities](https://gecko.layle.dev/dbg)
- Terrible idle skipping :^)

## Projects

| Crate       | Description                                                                                                                     |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `tinyapp`   | Lightweight emulator application with an egui/wgpu GUI, optional Lua scripting and idle-skip optimization                       |
| `debugger`  | Interactive GUI debugger built on egui with rendering support, hooks and scripting capabilities                                 |
| `web`       | WebAssembly build of the emulator for browser deployment via wasm-bindgen, with optional debug UI                               |
| `multitool` | CLI utility for analyzing, disassembling and extracting GC/Wii binaries/images (DOL, IPL, ISO/RVZ) with support for PPC and DSP |

## Building

```sh
cargo build -p multitool --release                               # multi-tool
cargo build -p tinyapp --release                                 # main application
cargo build -p debugger --release                                # debugger
wasm-pack build crates/web --target web --out-dir pkg --release  # web version
```

Certain features require certain feature flags such as `scripting` and `scripting-mut-traps`, however, the debugger has them all enabled.  
For more information refer to the GitHub CI actions file. PGO optimized compilation is supported, refer to the `Justfile`.

## Usage

```sh
multitool ipl --action decode ipl.encoded.bin ipl.decoded.bin
tinyapp --dol homebrew.dol  # may also require a DSP depending on the DOL
tinyapp --dvd game.iso --ipl ipl.decoded.bin --dsp dsp_rom.bin
```

The CLI options are largely the same across the sub projects (such as the debugger). For more options, see `--help`.

## Sister Projects
Gecko is being developed alongside other amazing emulators that shaped how Gecko came to be. Without them, Gecko wouldn't exist!

- [lazuli](https://github.com/vxpm/lazuli) authored by vxpm
- [solstice](https://codeberg.org/hazelwiss/solstice) authored by hazelwiss