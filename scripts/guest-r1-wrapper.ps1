$ErrorActionPreference='Stop'
Start-Service RustDesk -ErrorAction SilentlyContinue
& 'C:\RustDeskAgentLab\guest-r1-boundary-test.ps1'
