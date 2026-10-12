$ErrorActionPreference='Stop'
if($env:COMPUTERNAME -notlike 'RDLAB-*'){throw 'Only named lab guests may run this task'}
& 'C:\RustDeskAgentLab\guest-fault-matrix.ps1'
