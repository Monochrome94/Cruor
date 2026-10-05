# Cruor

**Physics blood for Exanima.** Cruor (Latin: *blood shed from a wound*) replaces Exanima's blood with simulated liquid: wounds spray real drops that fly as coherent jets, stretch into strings and break into droplets, then splash, pool, run down walls and stain floors, walls and objects. Bloody weapons fling blood off the blade when swung.

Cruor is a plugin (a DLL) for the [Exanima Modding Toolkit (EMTK)](https://github.com/ExanimaModding/Toolkit), written in Rust. This repository contains its complete source and everything needed to build the release.

## Contents

| Path | What it is |
|---|---|
| `cruor/src/lib.rs` | The mod's complete source code |
| `cruor/Cargo.toml` | Rust project file (dependencies) |
| `cruor/config.toml` | Plugin description file EMTK reads |
| `package/Play Exanima with Cruor.vbs` | The launcher shipped in the release |
| `package/README - Cruor.txt` | The player README shipped in the release |
| `package/make-release.bat` | The author's script that builds and packs the release zip |

## Building

### 1. Tools (Windows 10/11, 64-bit)

- **Rust**, any recent stable version, from [rustup.rs](https://rustup.rs) (default toolchain `x86_64-pc-windows-msvc`)
- **[Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)** with the *Desktop development with C++* workload (EMTK compiles Microsoft Detours, which is C++)
- **[Git](https://git-scm.com)**

### 2. Get EMTK

```
git clone --recursive https://github.com/ExanimaModding/Toolkit.git
```

Cruor 1.0.0 was built against EMTK's main branch as downloaded in late September 2026, unmodified.

### 3. Place Cruor next to it

```
<any folder>\Toolkit\
<any folder>\cruor\     (this repository's cruor folder)
```

`cruor\Cargo.toml` refers to `..\Toolkit\bindings\rust\emf-rs`.

### 4. Build the mod loader

```
cd Toolkit
mkdir target\release\build\deps
cargo build --release
```

Output: `Toolkit\target\release\emtk.exe` and `emf.dll`.

> **About the `mkdir`:** with recent Rust versions, EMTK's `emf-sys` build script (`bindings\rust\emf-sys\build.rs`) stops with *"The system cannot find the path specified"*: newer Cargo arranges its build folders one level deeper than when the script was written, so the folder it expects is never created. Creating that empty folder first avoids it. This only adds a folder to the build output; EMTK's source is not changed.

### 5. Build Cruor

```
cd ..\cruor
mkdir target\release\build\deps
cargo build --release
```

Output: `cruor\target\release\blood_plugin.dll` (the same folder is needed for the same reason: Cruor builds EMTK's `emf-sys` bindings too). The crate kept its development name `blood-plugin`; the release renames the DLL to `cruor.dll`. It's the same file.

### 6. Release layout

```
mods\Cruor\cruor.dll              <- blood_plugin.dll, renamed
mods\Cruor\config.toml            <- cruor\config.toml
mods\Cruor\Cruor-settings.txt     <- optional; the same values are the built-in defaults
emtk\emtk.exe, emtk\emf.dll       <- from step 4, with EMTK's license files
Play Exanima with Cruor.vbs       <- package\
README - Cruor.txt                <- package\
```

`package\make-release.bat` does steps 4 to 6 automatically (change the folder paths at its top to yours).

## What the mod does (for reviewers)

- **Launcher:** `Play Exanima with Cruor.vbs` sets two environment variables (`EXANIMA_EXE`, `LD_LIBRARY_PATH`) and runs `emtk\emtk.exe` from the Exanima folder with its console window hidden.
- **Loader:** EMTK starts Exanima and loads the mods in its `mods` folder. It is open source and shipped unmodified.
- **The mod** hooks the game's blood-spray function and some of its OpenGL calls (through EMTK, which uses Microsoft Detours) to simulate and draw blood, all inside Exanima's own process.
- **No** network access, **no** starting other programs, **no** registry access, **no** changing memory protection or touching other processes.
- **Keyboard:** reads only its own hotkeys (F5 to F12, Numpad 2/4/5/6/8/-/+) to toggle the mod and change settings; nothing is recorded or sent.
- **Files:** writes only `Cruor-log.txt` (in the Exanima folder, or `Documents\Cruor`) and `mods\Cruor\Cruor-settings.txt`.
- **Crashes:** a vectored exception handler only notes in the log which module a crash happened in, then lets Windows handle it as usual.

## Built with

- [Exanima Modding Toolkit (EMTK)](https://github.com/ExanimaModding/Toolkit), MIT / Apache 2.0 (mirror: [Codeberg](https://codeberg.org/ExanimaModding/Toolkit))
- [Microsoft Detours](https://github.com/microsoft/Detours), MIT, used by EMTK
- Rust crates: [`log`](https://crates.io/crates/log), [`pretty_env_logger`](https://crates.io/crates/pretty_env_logger), and EMTK's `emf-rs` bindings
- [Exanima](https://store.steampowered.com/app/362490) by Bare Mettle Entertainment

Cruor is not affiliated with or endorsed by Bare Mettle Entertainment or the EMTK project.
