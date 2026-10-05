@echo off
setlocal EnableExtensions

if /i not "%~1"=="_elevated" (
    powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Start-Process -FilePath '%~f0' -ArgumentList '_elevated' -Verb RunAs"
    exit /b 0
)

echo [1/6] Stopping RustDesk processes and services...
for %%S in (RustDesk "RustDesk Service" RustDeskManagementAgent) do (
    sc.exe stop "%%~S" >nul 2>&1
)
taskkill.exe /F /T /IM RustDesk.exe >nul 2>&1
taskkill.exe /F /T /IM rustdesk-management-agent.exe >nul 2>&1
timeout /t 2 /nobreak >nul

echo [2/6] Uninstalling every RustDesk MSI registration...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$roots=@('HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall','HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'); foreach($root in $roots){$keys=Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue; foreach($key in $keys){$item=Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction SilentlyContinue; if($item.DisplayName -like '*RustDesk*' -and $item.WindowsInstaller -eq 1){$code=$key.PSChildName; if($code -match '^\{[0-9A-Fa-f-]+\}$'){Write-Host ('Uninstalling '+$item.DisplayName+' '+$code); $p=Start-Process msiexec.exe -ArgumentList @('/x',$code,'/qn','/norestart') -Wait -PassThru; Write-Host ('msiexec exit '+$p.ExitCode)}}}}"

echo [3/6] Removing all RustDesk services...
for %%S in (RustDesk "RustDesk Service" RustDeskManagementAgent) do (
    sc.exe stop "%%~S" >nul 2>&1
    sc.exe delete "%%~S" >nul 2>&1
)
timeout /t 2 /nobreak >nul

echo [4/6] Removing RustDesk files and recovery metadata...
for %%D in (
    "%ProgramFiles%\RustDesk"
    "%ProgramFiles(x86)%\RustDesk"
    "%ProgramData%\RustDesk"
    "%LOCALAPPDATA%\RustDesk"
    "%LOCALAPPDATA%\rustdesk"
    "%APPDATA%\RustDesk"
) do (
    if exist "%%~D" rmdir /s /q "%%~D"
)
for %%F in (
    "%ProgramData%\Microsoft\Windows\Start Menu\Programs\Startup\RustDesk Tray.lnk"
    "%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\RustDesk Tray.lnk"
    "%PUBLIC%\Desktop\RustDesk.lnk"
    "%USERPROFILE%\Desktop\RustDesk.lnk"
) do (
    if exist "%%~F" del /f /q "%%~F"
)
for /d %%D in ("%TEMP%\.rustdesk-update-*") do (
    if exist "%%~D" rmdir /s /q "%%~D"
)

echo [5/6] Removing RustDesk registry metadata...
reg.exe delete "HKLM\SOFTWARE\RustDesk" /f >nul 2>&1
reg.exe delete "HKLM\SOFTWARE\WOW6432Node\RustDesk" /f >nul 2>&1
reg.exe delete "HKCU\SOFTWARE\RustDesk" /f >nul 2>&1
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$roots=@('HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall','HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'); foreach($root in $roots){$keys=Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue; foreach($key in $keys){$item=Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction SilentlyContinue; if($item.DisplayName -like '*RustDesk*'){Remove-Item -LiteralPath $key.PSPath -Recurse -Force -ErrorAction SilentlyContinue}}}"

echo [6/6] Verification...
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$names=@('RustDesk','RustDesk Service','RustDeskManagementAgent'); foreach($name in $names){$svc=Get-CimInstance Win32_Service -Filter ('Name=''{0}''' -f $name) -ErrorAction SilentlyContinue; if($svc){Write-Host ('SERVICE STILL EXISTS: '+$name+' ['+$svc.State+']')}else{Write-Host ('SERVICE CLEAN: '+$name)}}; $paths=@('C:\Program Files\RustDesk','C:\Program Files (x86)\RustDesk',(Join-Path $env:ProgramData 'RustDesk'),(Join-Path $env:LOCALAPPDATA 'RustDesk'),(Join-Path $env:LOCALAPPDATA 'rustdesk'),(Join-Path $env:APPDATA 'RustDesk')); foreach($path in $paths){Write-Host ($path+' exists: '+(Test-Path -LiteralPath $path))}; $left=0; foreach($root in @('HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall','HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall')){$keys=Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue; foreach($key in $keys){$item=Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction SilentlyContinue; if($item.DisplayName -like '*RustDesk*'){$left++}}}; Write-Host ('RustDesk uninstall entries left: '+$left)"

echo.
echo Cleanup finished. Reboot before installing the next test package if a service was locked.
pause
endlocal
