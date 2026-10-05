@echo off
rem Builds Cruor and the mod loader in release mode and packs a complete, ready-to-use zip:
rem Cruor + the mod loader (EMTK) + launcher + README + licenses + YOUR current settings.

set "TOOLS=C:\Users\penni\Documents\TOOLS"
set "PLUG=%TOOLS%\blood-plugin"
set "TK=%TOOLS%\Toolkit"
set "GAME=C:\Program Files (x86)\Steam\steamapps\common\Exanima"
set "DL=%USERPROFILE%\Downloads"
set "VER=1.0.0"
cd /d "%PLUG%"

rem ---- launcher and README: from this folder, or Downloads ----
for %%F in ("Play Exanima with Cruor.vbs" "README - Cruor.txt") do (
    if not exist "%PLUG%\%%~F" if exist "%DL%\%%~F" copy /y "%DL%\%%~F" "%PLUG%\%%~F" >nul
    if not exist "%PLUG%\%%~F" (
        echo Missing %%~F - save it into %PLUG% first.
        pause
        exit /b 1
    )
)

rem ---- 0. make sure src\lib.rs is the Cruor release code (not an old test build) ----
set /p FIRSTLINE=<"%PLUG%\src\lib.rs"
rem (the release code starts with "// Cruor 1.0.0 - physics blood"; test builds don't)
findstr /b /c:"// Cruor %VER% - physics blood" "%PLUG%\src\lib.rs" >nul
if errorlevel 1 goto wrongsource
echo Source: Cruor %VER% release code - OK

rem ---- 1. Cruor ----
if not exist "target\release\build\deps" mkdir "target\release\build\deps"
copy /b "%PLUG%\src\lib.rs"+,, "%PLUG%\src\lib.rs" >nul
echo [1/3] Building Cruor (release)...
cargo build --release > release-log.txt 2>&1
if errorlevel 1 (
    echo BUILD FAILED. Upload release-log.txt from the blood-plugin folder to Claude.
    start "" notepad release-log.txt
    pause
    exit /b 1
)

rem ---- 2. the mod loader (EMTK) ----
cd /d "%TK%"
if not exist "target\release\build\deps" mkdir "target\release\build\deps"
echo [2/3] Building the mod loader (release) - the first time takes a few minutes...
cargo build --release > "%PLUG%\toolkit-log.txt" 2>&1
set "EMTK_DIR=%TK%\target\release"
if not exist "%EMTK_DIR%\emtk.exe" set "EMTK_DIR=%TK%\target\debug"
if not exist "%EMTK_DIR%\emf.dll" set "EMTK_DIR=%TK%\target\debug"
if not exist "%EMTK_DIR%\emtk.exe" (
    echo Couldn't build or find the mod loader. Upload toolkit-log.txt from the blood-plugin folder to Claude.
    pause
    exit /b 1
)
echo      using mod loader from %EMTK_DIR%
cd /d "%PLUG%"

rem ---- 3. assemble and zip ----
echo [3/3] Packing...
set "OUT=%PLUG%\release-cruor"
if exist "%OUT%" rmdir /s /q "%OUT%"
mkdir "%OUT%\mods\Cruor"
mkdir "%OUT%\emtk"
copy /y "target\release\blood_plugin.dll" "%OUT%\mods\Cruor\cruor.dll" >nul
copy /y "config.toml" "%OUT%\mods\Cruor\config.toml" >nul
rem your settings ship with it (the newest you have)
if exist "%GAME%\mods\Cruor\Cruor-settings.txt" (
    copy /y "%GAME%\mods\Cruor\Cruor-settings.txt" "%OUT%\mods\Cruor\Cruor-settings.txt" >nul
    echo      included your settings from mods\Cruor
) else if exist "%GAME%\mods\blood-plugin\blood-settings.txt" (
    copy /y "%GAME%\mods\blood-plugin\blood-settings.txt" "%OUT%\mods\Cruor\Cruor-settings.txt" >nul
    echo      included your settings from mods\blood-plugin
) else (
    echo      no settings file found - the built-in defaults ^(your settings^) are used
)
copy /y "%EMTK_DIR%\emtk.exe" "%OUT%\emtk\emtk.exe" >nul
copy /y "%EMTK_DIR%\emf.dll" "%OUT%\emtk\emf.dll" >nul
for %%L in ("%TK%\LICENSE*") do copy /y "%%L" "%OUT%\emtk\" >nul
if exist "%TK%\crates\detours\ext\detours\LICENSE.md" copy /y "%TK%\crates\detours\ext\detours\LICENSE.md" "%OUT%\emtk\LICENSE-Detours.md" >nul
if exist "%TK%\crates\detours\ext\detours\LICENSE" copy /y "%TK%\crates\detours\ext\detours\LICENSE" "%OUT%\emtk\LICENSE-Detours" >nul
copy /y "Play Exanima with Cruor.vbs" "%OUT%\Play Exanima with Cruor.vbs" >nul
copy /y "README - Cruor.txt" "%OUT%\README - Cruor.txt" >nul

set "ZIP=%PLUG%\Cruor-%VER%.zip"
if exist "%ZIP%" del /q "%ZIP%"
powershell -NoProfile -Command "Compress-Archive -Path '%OUT%\*' -DestinationPath '%ZIP%' -Force"
echo.
echo Done: %ZIP%
echo Players unzip it into their Exanima folder and double-click "Play Exanima with Cruor.vbs".
explorer /select,"%ZIP%"
pause
exit /b 0

:wrongsource
echo.
echo ==============================================================
echo   STOPPED: src\lib.rs is NOT the Cruor %VER% release code.
echo   It starts with:
echo   %FIRSTLINE%
echo   Save the Cruor lib.rs into %PLUG%\src\ and run this again.
echo ==============================================================
pause
exit /b 1
