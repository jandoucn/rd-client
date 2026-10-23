$ErrorActionPreference='Stop'
if(!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME){throw 'Only the declared lab host may run this task: set RUSTDESK_LAB_HOST to this COMPUTERNAME'}
& 'C:\RustDeskAgentLab\guest-fault-matrix.ps1'
