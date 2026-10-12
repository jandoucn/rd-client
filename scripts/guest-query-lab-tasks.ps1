$ErrorActionPreference='Stop'
Get-ScheduledTask | Where-Object { $_.TaskName -like '*RustDesk*' } |
    Select-Object TaskName,State,TaskPath |
    ConvertTo-Json -Depth 4 -Compress |
    Set-Content -LiteralPath 'C:\RustDeskAgentLab\task-query.json' -Encoding utf8
