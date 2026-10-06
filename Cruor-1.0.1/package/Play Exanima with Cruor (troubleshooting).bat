@echo off
rem Play Exanima with Cruor - TROUBLESHOOTING launcher: same as the .vbs, but keeps a
rem window open that shows the mod loader's messages. Use it if the mod doesn't load.
rem This file must be in the Exanima game folder, next to Exanima.exe.

cd /d "%~dp0"
if not exist "%~dp0Exanima.exe" goto notfound
if not exist "%~dp0emtk\emtk.exe" goto noemtk

set "EXANIMA_EXE=%~dp0Exanima.exe"
set "LD_LIBRARY_PATH=%~dp0emtk"
cd /d "%~dp0emtk"
title Cruor - mod loader messages
echo Starting Exanima with Cruor (troubleshooting window)...
echo If the mod loads you'll see "Cruor 1.0.0 enabled" below.
echo.
emtk.exe
echo.
echo Exanima has closed. Copy the messages above if you're reporting a problem.
pause
exit /b 0

:notfound
echo Exanima.exe was not found next to this file.
echo Unzip the whole Cruor zip into your Exanima folder - the one that contains Exanima.exe.
pause
exit /b 1

:noemtk
echo The emtk folder is missing. Unzip the WHOLE Cruor zip into your Exanima folder.
pause
exit /b 1
