' Play Exanima with Cruor - starts Exanima with the Cruor blood mod (through the Exanima
' Modding Toolkit), without a console window. Keep this file in the Exanima folder, next to
' Exanima.exe. The mod's messages go to Cruor-log.txt in the same folder.
Option Explicit
Dim sh, fso, here, env
Set sh = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")
here = fso.GetParentFolderName(WScript.ScriptFullName)
If Not fso.FileExists(here & "\Exanima.exe") Then
    MsgBox "Exanima.exe was not found next to this file." & vbCrLf & vbCrLf & _
        "Unzip the whole Cruor zip into your Exanima folder - the one that contains Exanima.exe." & vbCrLf & _
        "(In Steam: right-click Exanima, Manage, Browse local files.)", vbExclamation, "Cruor"
    WScript.Quit 1
End If
If Not fso.FileExists(here & "\emtk\emtk.exe") Then
    MsgBox "The emtk folder is missing. Unzip the WHOLE Cruor zip into your Exanima folder.", vbExclamation, "Cruor"
    WScript.Quit 1
End If
Set env = sh.Environment("PROCESS")
env("EXANIMA_EXE") = here & "\Exanima.exe"
env("LD_LIBRARY_PATH") = here & "\emtk"
sh.CurrentDirectory = here & "\emtk"
' 0 = hidden window, False = don't wait
sh.Run """" & here & "\emtk\emtk.exe""", 0, False
