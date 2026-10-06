# Cruor

**Physics blood for Exanima.** Cruor (Latin: *blood shed from a wound*) replaces Exanima's blood with simulated liquid: wounds spray real drops that fly as coherent jets, stretch into strings and break into droplets, then splash, pool, run down walls and stain floors, walls and objects. Bloody weapons fling blood off the blade when swung.

Cruor is a plugin (a DLL) for the [Exanima Modding Toolkit (EMTK)](https://codeberg.org/ExanimaModding/Toolkit), written in Rust. This repository contains its complete source and everything needed to build the release.

## Contents

| Path | What it is |
|---|---|
| `cruor/src/lib.rs` | The mod's complete source code |
| `cruor/Cargo.toml` | Rust project file (dependencies) |
| `cruor/config.toml` | Plugin description file EMTK reads |
| `package/Play Exanima with Cruor.vbs` | The launcher shipped in the release |
| `package/Play Exanima with Cruor (troubleshooting).bat` | The same launcher with a visible message window |
| `cruor/rust-toolchain.toml` | Selects the nightly Rust toolchain |
| `package/README - Cruor.txt` | The player README shipped in the release |
| `package/make-release.bat` | The author's script that builds and packs the release zip |

## Just want to play?

**You don't need to build anything.** Download the release zip from the mod page, unzip it into your Exanima folder and start the game with `Play Exanima with Cruor.vbs`. Building from source is only for checking or changing the code.

## Building

> **Use the exact EMTK version below.** Cruor is built against EMTK's **`rust-plugins` tag** (commit `3b1f54f4cd274a1a45dfa0521090eaa91c6e8a67`, branch `plugins`) from **Codeberg**. EMTK's `main` branch and the `v0.1.0-beta.2` release are a different, newer loader (with a GUI) that uses a different plugin system: Cruor won't appear in their plugin list, and building against them fails or gives "OS error 126" for `emf.dll`.

### 1. Tools (Windows 10/11, 64-bit)

- **Rust** via [rustup.rs](https://rustup.rs). Cruor and this EMTK version build with the **nightly** toolchain (`cruor/rust-toolchain.toml` selects it automatically).
- **[Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)** with the *Desktop development with C++* workload (EMTK compiles Microsoft Detours, which is C++)
- **[Git](https://git-scm.com)**

### 2. Get EMTK at the right version

```
git clone https://codeberg.org/ExanimaModding/Toolkit.git
cd Toolkit
git checkout rust-plugins
git submodule update --init --recursive
cd ..
```

`git -C Toolkit rev-parse HEAD` should print `3b1f54f4cd274a1a45dfa0521090eaa91c6e8a67`. EMTK's code is used unmodified. (If you build EMTK with its own `cargo make`, that runs `cargo fmt`, which may reformat a file or two, for example reordering the module lines in `crates/emf/src/internal/utils/mod.rs`. That's formatting only; the compiled loader is the same.)

### 3. Place Cruor next to it

```
<any folder>\Toolkit\
<any folder>\cruor\     (this repository's cruor folder)
```

`cruor\Cargo.toml` refers to `..\Toolkit\bindings\rust\emf-rs`, so the two folders must sit side by side with exactly these names.

### 4. Build the mod loader

```
cd Toolkit
mkdir target\release\build\deps
set RUSTFLAGS=-C target-feature=+crt-static
cargo build --release
```

Output: `Toolkit\target\release\emtk.exe` and `emf.dll`.

- **The `mkdir`:** with recent Rust versions, EMTK's `emf-sys` build script stops with *"The system cannot find the path specified"*: newer Cargo nests its build folders one level deeper than the script expects, so the folder it looks for is never created. Creating it empty first avoids that. EMTK's source is not changed.
- **`RUSTFLAGS`:** builds the C runtime into the files, so players don't need the Visual C++ Redistributable. Optional for your own use.

### 5. Build Cruor

```
cd ..\cruor
mkdir target\release\build\deps
cargo build --release
```

(`RUSTFLAGS` from step 4 still applies in the same window.) Output: `cruor\target\release\blood_plugin.dll`. The crate kept its development name `blood-plugin`; the release renames the DLL to `cruor.dll`. It's the same file.

### 6. Release layout (all inside the Exanima folder, next to Exanima.exe)

```
mods\Cruor\cruor.dll              <- blood_plugin.dll, renamed
mods\Cruor\config.toml            <- cruor\config.toml
mods\Cruor\Cruor-settings.txt     <- optional; the same values are the built-in defaults
emtk\emtk.exe, emtk\emf.dll       <- from step 4, with EMTK's license files
Play Exanima with Cruor.vbs        <- package\  (starts the game, no window)
Play Exanima with Cruor (troubleshooting).bat  <- package\  (same, with a message window)
README - Cruor.txt                 <- package\
```

The launchers must be in the Exanima folder: they start `emtk\emtk.exe` from there, and EMTK loads the plugins from `mods\`. `package\make-release.bat` does steps 4 to 6 automatically (change the folder paths at its top to yours).

### Troubleshooting

- **Plugin list empty / nothing happens in game:** wrong EMTK version (see the note at the top of this section).
- **"OS error 126" for `emf.dll` or `cruor.dll`:** a DLL they need couldn't be loaded. Usually the wrong EMTK version, or the Visual C++ Redistributable (x64) missing when built without `crt-static`: [vc_redist.x64.exe](https://aka.ms/vs/17/release/vc_redist.x64.exe).
- **No `Cruor-log.txt`:** the mod never loaded (the log is written by the mod itself, in the Exanima folder, or `Documents\Cruor` if that isn't writable). Use the troubleshooting launcher to see the loader's messages.

## What the mod does (for reviewers)

- **Launchers:** both set two environment variables (`EXANIMA_EXE`, `LD_LIBRARY_PATH`) and run `emtk\emtk.exe` from the Exanima folder. `Play Exanima with Cruor.vbs` hides the loader's console window; `Play Exanima with Cruor (troubleshooting).bat` shows it.
- **Loader:** EMTK starts Exanima and loads the mods in its `mods` folder. It is open source and shipped unmodified.
- **The mod** hooks the game's blood-spray function and some of its OpenGL calls (through EMTK, which uses Microsoft Detours) to simulate and draw blood, all inside Exanima's own process.
- **No** network access, **no** starting other programs, **no** registry access, **no** changing memory protection or touching other processes.
- **Keyboard:** reads only its own hotkeys (F5 to F12, Numpad 2/4/5/6/8/-/+) to toggle the mod and change settings; nothing is recorded or sent.
- **Files:** writes only `Cruor-log.txt` (in the Exanima folder, or `Documents\Cruor`) and `mods\Cruor\Cruor-settings.txt`.
- **Crashes:** a vectored exception handler only notes in the log which module a crash happened in, then lets Windows handle it as usual.

## Built with

- [Exanima Modding Toolkit (EMTK)](https://codeberg.org/ExanimaModding/Toolkit), MIT / Apache 2.0, `rust-plugins` tag
- [Microsoft Detours](https://github.com/microsoft/Detours), MIT, used by EMTK
- Rust crates: [`log`](https://crates.io/crates/log), [`pretty_env_logger`](https://crates.io/crates/pretty_env_logger), and EMTK's `emf-rs` bindings
- [Exanima](https://store.steampowered.com/app/362490) by Bare Mettle Entertainment

Cruor is not affiliated with or endorsed by Bare Mettle Entertainment or the EMTK project.
