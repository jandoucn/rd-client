$ErrorActionPreference='Stop'
Start-Service RustDesk -ErrorAction SilentlyContinue
Start-ScheduledTask -TaskName RustDeskAgentLabDriver
