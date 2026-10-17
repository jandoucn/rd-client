@echo off
call "C:\RustDeskBuildTools\VC\Auxiliary\Build\vcvars64.bat"
if errorlevel 1 exit /b %errorlevel%
set "OUT=C:\Users\Administrator\AppData\Local\Temp\opencode"
cl /nologo /EHsc /std:c++17 /MT /W4 /Fe:"%OUT%\msi-desktop-shortcut-fixture.exe" /Fo:"%OUT%\msi-desktop-shortcut-fixture.obj" tests\msi_desktop_shortcut.cpp
if errorlevel 1 exit /b %errorlevel%
cl /nologo /EHsc /std:c++17 /MT /W4 /LD /Fe:"%OUT%\msi-desktop-shortcut-ca.dll" /Fo:"%OUT%\msi-desktop-shortcut-ca.obj" res\msi\CustomActions\DesktopShortcut.cpp /link /EXPORT:InspectDesktopShortcut /IMPLIB:"%OUT%\msi-desktop-shortcut-ca.lib"
exit /b %errorlevel%
