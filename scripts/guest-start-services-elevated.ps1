$ErrorActionPreference='Stop'
Start-Service RustDeskManagementAgent -ErrorAction SilentlyContinue
Start-Service RustDesk -ErrorAction Stop
