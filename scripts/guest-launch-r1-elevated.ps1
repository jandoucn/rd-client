$ErrorActionPreference='Stop'
$shell=New-Object -ComObject Shell.Application
$shell.ShellExecute('C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe','-NoProfile -ExecutionPolicy Bypass -File C:\RustDeskAgentLab\guest-r1-wrapper.ps1','C:\RustDeskAgentLab','runas',0)
