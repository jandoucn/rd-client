#include <windows.h>
#include <msiquery.h>
#include <string>
#include <vector>
#include <wincrypt.h>
#include <objbase.h>
#pragma comment(lib, "msi.lib")
#pragma comment(lib, "crypt32.lib")
#pragma comment(lib, "ole32.lib")

// This verifier is embedded in the package CA. Never execute a verifier supplied
// by an installed old client, a public MSI property or a writable sidecar.
static const wchar_t* kVerifier = LR"PS(
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$held=New-Object System.Collections.ArrayList
function Need([bool]$ok,[string]$reason){if(!$ok){throw $reason}}
function Protected([string]$path,[bool]$directory=$false){
 $i=Get-Item -LiteralPath $path -Force
 Need ($i.PSIsContainer -eq $directory) 'object_kind'
 for($p=$i;$null -ne $p;$p=if($p -is [IO.DirectoryInfo]){$p.Parent}else{$p.Directory}){Need (!$p.Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) 'reparse'}
 $acl=Get-Acl -LiteralPath $path
 Need ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -in @('S-1-5-18','S-1-5-32-544')) 'owner'
 if($directory){Need $acl.AreAccessRulesProtected 'inheritance'}
 $seen=@{}
 foreach($ace in $acl.Access){
  $sid=$ace.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
  Need ($ace.AccessControlType -eq 'Allow' -and $sid -in @('S-1-5-18','S-1-5-32-544')) 'acl'
  Need (([int]$ace.FileSystemRights -band 0x1f01ff) -eq 0x1f01ff -and !($ace.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly)) 'rights'
  $seen[$sid]=$true
 }
 Need ($seen.ContainsKey('S-1-5-18') -and $seen.ContainsKey('S-1-5-32-544')) 'principals'
}
function ReadBound([string]$path,[int]$limit){
 Protected $path
 $f=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
 [void]$held.Add($f);Need ($f.Length -gt 0 -and $f.Length -le $limit) 'length'
 $reader=New-Object IO.StreamReader($f,[Text.Encoding]::UTF8,$true,1024,$true)
 try{$text=$reader.ReadToEnd()}finally{$reader.Dispose()}
 $sha=[Security.Cryptography.SHA256]::Create();try{$f.Position=0;$hash=[BitConverter]::ToString($sha.ComputeHash($f)).Replace('-','').ToLowerInvariant()}finally{$sha.Dispose()}
 @{value=($text|ConvertFrom-Json);hash=$hash}
}
function Keys($v,[string[]]$expected){Need ((($v.PSObject.Properties.Name|Sort-Object) -join ',') -ceq (($expected|Sort-Object) -join ',')) 'fields'}
function Same($a,$b){Need ([string]$a -ceq [string]$b) 'binding'}
function Token([string]$v,[int]$n){Need ($v -cmatch ('^[a-f0-9]{'+$n+'}$')) 'token'}
function ProcessOwner($owner,[string]$path){
 Need ($owner.pid -gt 0 -and $owner.created_at -gt 0) 'process_identity'
 $p=Get-Process -Id $owner.pid;[void]$held.Add($p);$null=$p.Handle
 Need (!$p.HasExited -and $p.StartTime.ToUniversalTime().ToFileTimeUtc().ToString() -ceq $owner.created_at.ToString()) 'process_reused'
 Need ($p.Path -ieq $path) 'process_path'
}
)PS"
LR"PS(
try {
 $base='C:\ProgramData\RustDesk\ManagementAgent'
 if($removal){
  Token $challenge 32
  $session=(ReadBound "$base\state\msi-remove-session-$challenge.json" 8192).value
  $tx=[string]$session.transaction_id;$step=[string]$session.step;$requestNonce=[string]$session.nonce
 }
 Token $tx 32;Token $requestNonce 64
 Need ($step -in @('install','rollback')) 'step'
 Need ($product -match '^\{[0-9a-fA-F-]{36}\}$' -and $packageCode -match '^\{[0-9a-fA-F-]{36}\}$') 'product'
 $root="$base\transactions\$tx";Protected $root $true
 $permit=(ReadBound "$root\msi-$step-permit.json" 8192).value
 Keys $permit @('schema','source_kind','remove','transaction_id','step','nonce','plan_sha256','handoff_sha256','marker_sha256','transaction_sha256','package_relative_path','package_sha256','package_size','product_code','package_code','helper','helper_sha256')
 Need ($permit.schema -eq 2 -and $permit.source_kind -in @('transaction','local_repair')) 'schema';Same $permit.transaction_id $tx;Same $permit.step $step;Same $permit.nonce $requestNonce
 if($challenge -ne ''){
  Token $challenge 32
  if(!$removal){$session=(ReadBound "$root\msi-$step-session-$challenge.json" 8192).value}
  Keys $session @('schema','role','parent_product_code','parent_package_code','transaction_id','step','nonce','challenge','product_code','package_code','helper','ca')
  Need ($session.schema -eq 2) 'session_schema';Same $session.role $(if($removal){'remove'}else{'install'})
  Same $session.parent_product_code $permit.product_code;Same $session.parent_package_code $permit.package_code
  Same $session.transaction_id $tx;Same $session.step $step;Same $session.nonce $requestNonce
  Same $session.challenge $challenge;Same $session.product_code $product;Same $session.package_code $packageCode
  Same $session.helper.pid $permit.helper.pid;Same $session.helper.created_at $permit.helper.created_at
  Same $session.ca.pid $caPid;Same $session.ca.created_at $caCreated
 }
 Token $permit.package_sha256 64;Token $permit.helper_sha256 64
 $local=$permit.source_kind -eq 'local_repair'
 $planName=if($local){'local-repair-plan.json'}else{'prepared.json'}
 $otherName=if($local){'prepared.json'}else{'local-repair-plan.json'}
 Need (!(Test-Path -LiteralPath "$root\$otherName")) 'mixed_plan'
 $planRecord=ReadBound "$root\$planName" 65536;$plan=$planRecord.value;Same $planRecord.hash $permit.plan_sha256
 if($local){Need (!$removal -and $step -eq 'install' -and $plan.trigger -eq 'local_recovery' -and $plan.original_normal -eq 'missing') 'local_repair_contract'}
 Same $plan.transaction_id $tx;Need ($plan.schema -eq 1) 'plan_schema'
 Need ($plan.target_key -in @('windows-x86_64-msi-standard','windows-x86_64-msi-sos')) 'plan_target'
 Same $plan.helper.relative_path 'runtime/helper.exe'
 $handoffRecord=ReadBound "$root\handoff.json" 4096;$handoff=$handoffRecord.value;Same $handoffRecord.hash $permit.handoff_sha256
 Same $handoff.transaction_id $tx;Same $handoff.plan_sha256 $permit.plan_sha256;Need ($handoff.schema -eq 1) 'handoff_schema';Token $handoff.nonce 64
 $markerRecord=ReadBound "$base\state\maintenance.json" 4096;$marker=$markerRecord.value;Same $markerRecord.hash $permit.marker_sha256
 Need ($marker.schema -eq 1) 'marker_schema';Same $marker.transaction_id $tx;Same $marker.plan_sha256 $permit.plan_sha256;Same $marker.nonce $handoff.nonce;Same $marker.installation_id $plan.installation_id
 foreach($gate in @('disable-agent','disable-recovery')){Need (!(Test-Path -LiteralPath "$base\state\$gate")) 'local_gate'}
 $transactionRecord=ReadBound "$root\transaction.json" 65536;$transaction=$transactionRecord.value;Same $transactionRecord.hash $permit.transaction_sha256
 Same $transaction.id $tx;Same $transaction.package_sha256 $plan.package.sha256
 Need (($step -eq 'install' -and $transaction.phase -eq 'applying') -or ($step -eq 'rollback' -and $transaction.phase -eq 'rolling_back')) 'phase'
 $readyName=if($local){'local-ready.json'}else{'helper-ready.json'};$releaseName=if($local){'local-release.json'}else{'helper-release.json'}
 $ready=(ReadBound "$root\$readyName" 4096).value;$release=(ReadBound "$root\$releaseName" 4096).value
 foreach($v in @($ready,$release)){Need ($v.schema -eq 1) 'release_schema';Same $v.transaction_id $tx;Same $v.plan_sha256 $permit.plan_sha256;Same $v.nonce $handoff.nonce;Same $v.helper.pid $handoff.helper.pid;Same $v.helper.created_at $handoff.helper.created_at}
 Same $release.parent.pid $handoff.parent.pid;Same $release.parent.created_at $handoff.parent.created_at
 Need ($ready.ready_at -ge $handoff.created_at -and $release.released_at -ge $ready.ready_at -and ($release.released_at-$handoff.created_at) -lt 300 -and $release.released_at -le [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()) 'release_time'
 $helperPath="$root\runtime\helper.exe";Protected $helperPath;ProcessOwner $permit.helper $helperPath
 $helperFile=[IO.File]::Open($helperPath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read);[void]$held.Add($helperFile)
 $sha=[Security.Cryptography.SHA256]::Create();try{$helperHash=[BitConverter]::ToString($sha.ComputeHash($helperFile)).Replace('-','').ToLowerInvariant()}finally{$sha.Dispose()}
 Same $helperHash $permit.helper_sha256;Same $permit.helper_sha256 $plan.helper.sha256
 if($permit.helper.pid -ne $handoff.helper.pid -or $permit.helper.created_at -ne $handoff.helper.created_at){
  $old=Get-Process -Id $handoff.helper.pid -ErrorAction SilentlyContinue
  if($old){Need ($old.StartTime.ToUniversalTime().ToFileTimeUtc().ToString() -cne $handoff.helper.created_at.ToString()) 'old_helper_alive'}
 }
)PS"
LR"PS(
 $relative=[string]$permit.package_relative_path
 Need ($relative -cmatch '^(original/)?[A-Za-z0-9_.+-]+\.msi$') 'package_path'
 $expectedPath=Join-Path $root $relative.Replace('/','\')
 if(!$removal){Need ([IO.Path]::GetFullPath($original) -ieq $expectedPath) 'original_database'}
 $artifact=if($step -eq 'install'){$plan.package}else{(ReadBound "$root\native-assets.json" 65536).value.original_msi}
 Same $artifact.relative_path $relative;Same $artifact.sha256 $permit.package_sha256;Same $artifact.size $permit.package_size
 Protected $expectedPath
 $f=[IO.File]::Open($expectedPath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read);[void]$held.Add($f)
 Same $f.Length $permit.package_size
 $sha=[Security.Cryptography.SHA256]::Create();try{$hash=[BitConverter]::ToString($sha.ComputeHash($f)).Replace('-','').ToLowerInvariant()}finally{$sha.Dispose()};Same $hash $permit.package_sha256
 $installer=New-Object -ComObject WindowsInstaller.Installer;$db=$installer.OpenDatabase($expectedPath,0)
 $view=$db.OpenView("SELECT ``Value`` FROM ``Property`` WHERE ``Property``='ProductCode'");[void]$view.Execute();$record=$view.Fetch();Same $record.StringData(1) $permit.product_code;[void]$view.Close()
 $summary=$db.SummaryInformation(0);Same $summary.Property(9) $permit.package_code
 if($removal){
  Need (!$local -and $null -ne $permit.remove) 'removal_not_authorized'
  $remove=$permit.remove
  Same $remove.product_code $product;Same $remove.package_code $packageCode
  $assetsRecord=ReadBound "$root\native-assets.json" 65536;Same $assetsRecord.hash $remove.native_assets_sha256
  Same $assetsRecord.value.transaction_id $tx
  $old=if($step -eq 'install'){$assetsRecord.value.original_msi}else{$plan.package}
  Same $old.relative_path $remove.source.relative_path;Same $old.sha256 $remove.source.sha256;Same $old.size $remove.source.size
  $rel=[string]$old.relative_path;Need ($rel -cmatch '^(original/)?[A-Za-z0-9_.+-]+\.msi$') 'removal_source_path'
  $src=Join-Path $root $rel.Replace('/','\');Protected $src
  $sf=[IO.File]::Open($src,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read);[void]$held.Add($sf);Same $sf.Length $old.size
  $sha=[Security.Cryptography.SHA256]::Create();try{Same ([BitConverter]::ToString($sha.ComputeHash($sf)).Replace('-','').ToLowerInvariant()) $old.sha256}finally{$sha.Dispose()}
  $cache=[string]$remove.cached_path
  Need ([IO.Path]::GetDirectoryName($cache) -ieq 'C:\Windows\Installer' -and [IO.Path]::GetExtension($cache) -ieq '.msi') 'cached_path'
  Need ([IO.Path]::GetFullPath($original) -ieq $cache) 'removal_original_database'
  $cf=[IO.File]::Open($cache,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read);[void]$held.Add($cf)
  $sha=[Security.Cryptography.SHA256]::Create();try{Same ([BitConverter]::ToString($sha.ComputeHash($cf)).Replace('-','').ToLowerInvariant()) $remove.cached_sha256}finally{$sha.Dispose()}
  $oldDb=$installer.OpenDatabase($src,0);$v=$oldDb.OpenView("SELECT ``Value`` FROM ``Property`` WHERE ``Property``='ProductCode'");[void]$v.Execute();$oldRow=$v.Fetch();Same $oldRow.StringData(1) $product;[void]$v.Close()
  Same $oldDb.SummaryInformation(0).Property(9) $packageCode
 }else{Same $permit.product_code $product;Same $permit.package_code $packageCode}
 exit 0
}catch{[Console]::Error.WriteLine('MSI admission refused: '+$_.Exception.Message);exit 1}
finally{foreach($v in $held){$v.Dispose()}}
)PS";

namespace {
std::wstring Literal(const std::wstring& s) {
    std::wstring out = L"'";
    for (wchar_t c : s) { out += c; if (c == L'\'') out += c; }
    return out + L"'";
}
bool AdmissionPermit(const std::wstring& id, const std::wstring& step, const std::wstring& nonce,
                     const std::wstring& source, const std::wstring& product, const std::wstring& code,
                     const std::wstring& challenge = L"", DWORD caPid = 0, ULONGLONG caCreated = 0, bool removal = false) {
    const std::wstring script = L"$tx=" + Literal(id) + L";$step=" + Literal(step) + L";$requestNonce=" + Literal(nonce)
        + L";$original=" + Literal(source) + L";$product=" + Literal(product) + L";$packageCode=" + Literal(code)
        + L";$challenge=" + Literal(challenge) + L";$caPid=" + Literal(std::to_wstring(caPid)) + L";$caCreated=" + Literal(std::to_wstring(caCreated)) + L";$removal=" + (removal?L"$true;":L"$false;") + kVerifier;
    DWORD count = 0;
    if (!CryptBinaryToStringW(reinterpret_cast<const BYTE*>(script.data()), static_cast<DWORD>(script.size() * sizeof(wchar_t)), CRYPT_STRING_BASE64 | CRYPT_STRING_NOCRLF, nullptr, &count)) return false;
    std::vector<wchar_t> encoded(count);
    if (!CryptBinaryToStringW(reinterpret_cast<const BYTE*>(script.data()), static_cast<DWORD>(script.size() * sizeof(wchar_t)), CRYPT_STRING_BASE64 | CRYPT_STRING_NOCRLF, encoded.data(), &count)) return false;
    wchar_t system[MAX_PATH];
    if (!GetSystemDirectoryW(system, MAX_PATH)) return false;
    const std::wstring exe = std::wstring(system) + L"\\WindowsPowerShell\\v1.0\\powershell.exe";
    std::wstring command = L"\"" + exe + L"\" -NoLogo -NoProfile -NonInteractive -EncodedCommand " + encoded.data();
    if (command.size() >= 32767) return false;
    std::vector<wchar_t> args(command.begin(), command.end());args.push_back(0);
    STARTUPINFOW startup{};startup.cb=sizeof(startup);PROCESS_INFORMATION child{};
    BOOL inherit=FALSE;
#ifdef RUSTDESK_ADMISSION_TEST
    startup.dwFlags=STARTF_USESTDHANDLES;startup.hStdOutput=GetStdHandle(STD_OUTPUT_HANDLE);startup.hStdError=GetStdHandle(STD_ERROR_HANDLE);startup.hStdInput=GetStdHandle(STD_INPUT_HANDLE);inherit=TRUE;
#endif
    if (!CreateProcessW(exe.c_str(), args.data(), nullptr, nullptr, inherit, CREATE_NO_WINDOW, nullptr, system, &startup, &child)) return false;
    CloseHandle(child.hThread);
    DWORD result=1;
    if (WaitForSingleObject(child.hProcess, 60000) == WAIT_OBJECT_0) GetExitCodeProcess(child.hProcess, &result);
    else { TerminateProcess(child.hProcess, 1); WaitForSingleObject(child.hProcess, 5000); }
    CloseHandle(child.hProcess);
    return result == 0;
}
}

#ifdef RUSTDESK_ADMISSION_TEST
int wmain(int argc, wchar_t** argv) {
    if (argc != 7) return 2;
    return AdmissionPermit(argv[1], argv[2], argv[3], argv[4], argv[5], argv[6]) ? 0 : 1;
}
#endif

#ifndef RUSTDESK_ADMISSION_TEST
namespace {
std::wstring Property(MSIHANDLE install, const wchar_t* name) {
    DWORD size = 0;
    wchar_t empty = 0;
    UINT status = MsiGetPropertyW(install, name, &empty, &size);
    if (status != ERROR_SUCCESS && status != ERROR_MORE_DATA) throw status;
    if (size > 32760) throw ERROR_INVALID_DATA;
    std::vector<wchar_t> buffer(size + 1);DWORD capacity = size + 1;
    if (MsiGetPropertyW(install, name, buffer.data(), &capacity) != ERROR_SUCCESS) throw ERROR_INVALID_DATA;
    return std::wstring(buffer.data(), capacity);
}
void Refused(MSIHANDLE install) {
    MSIHANDLE record = MsiCreateRecord(0);
    if (record) { MsiRecordSetStringW(record, 0, L"RustDesk MSI admission refused before InstallInitialize");MsiProcessMessage(install, INSTALLMESSAGE_ERROR, record);MsiCloseHandle(record); }
}
bool SessionChallenge(MSIHANDLE install, const std::wstring& id, const std::wstring& step,
                      const std::wstring& nonce, const std::wstring& product, const std::wstring& code,
                      std::wstring& challenge, ULONGLONG& created, bool removal = false) {
    GUID value{};if(FAILED(CoCreateGuid(&value)))return false;
    wchar_t guid[40]{};if(!StringFromGUID2(value,guid,40))return false;
    for(wchar_t c:std::wstring(guid)){if(c!=L'{' && c!=L'}' && c!=L'-')challenge+=static_cast<wchar_t>(towlower(c));}
    FILETIME start{},exit{},kernel{},user{};
    if(!GetProcessTimes(GetCurrentProcess(),&start,&exit,&kernel,&user))return false;
    created=(static_cast<ULONGLONG>(start.dwHighDateTime)<<32)|start.dwLowDateTime;
    MSIHANDLE record=MsiCreateRecord(8);if(!record)return false;
    const std::wstring fields[]={removal?L"RDMSI_REMOVE_V3":L"RDMSI_ADMISSION_V2",id,step,nonce,product,code,challenge,std::to_wstring(GetCurrentProcessId()),std::to_wstring(created)};
    bool ok=true;for(UINT i=0;i<9;i++)if(MsiRecordSetStringW(record,i,fields[i].c_str())!=ERROR_SUCCESS)ok=false;
    // Only this installation's external UI callback receives this record. The
    // callback's protected response, not the public property, proves routing.
    if(ok)MsiProcessMessage(install,INSTALLMESSAGE_INFO,record);
    MsiCloseHandle(record);return ok;
}
}
extern "C" UINT __stdcall CheckAgentInstallAdmission(MSIHANDLE install) {
    try {
        // Probe and release within this action/thread. No mutex ownership crosses CA boundaries.
        HANDLE mutex = OpenMutexW(SYNCHRONIZE | MUTEX_MODIFY_STATE, FALSE, L"Global\\RustDeskManagementAgentInstall");
        DWORD error = GetLastError();
        DWORD observed = WAIT_OBJECT_0;
        if (!mutex && error != ERROR_FILE_NOT_FOUND) { Refused(install);return ERROR_INSTALL_FAILURE; }
        if (mutex) {
            observed = WaitForSingleObject(mutex, 0);
            bool released = true;
            if (observed == WAIT_OBJECT_0 || observed == WAIT_ABANDONED) released = ReleaseMutex(mutex) != FALSE;
            bool closed = CloseHandle(mutex) != FALSE;
            if (!released || !closed || (observed != WAIT_OBJECT_0 && observed != WAIT_TIMEOUT)) { Refused(install);return ERROR_INSTALL_FAILURE; }
        }
        std::wstring id=Property(install,L"RUSTDESK_ADMISSION_TX");
        const bool removal=!Property(install,L"UPGRADINGPRODUCTCODE").empty();
        if (observed == WAIT_TIMEOUT || removal || !id.empty() || !Property(install,L"RUSTDESK_AGENT_RECOVERY").empty()) {
            if (!Property(install,L"TRANSFORMS").empty() || !Property(install,L"PATCH").empty()
                || (removal && Property(install,L"REMOVE")!=L"ALL")) { Refused(install);return ERROR_INSTALL_FAILURE; }
            MSIHANDLE db=MsiGetActiveDatabase(install), summary=0;
            if (!db || MsiGetSummaryInformationW(db,nullptr,0,&summary)!=ERROR_SUCCESS) { if(db)MsiCloseHandle(db);Refused(install);return ERROR_INSTALL_FAILURE; }
            wchar_t code[128]{};DWORD length=127;UINT type=0;INT number=0;FILETIME time{};
            UINT read=MsiSummaryInfoGetPropertyW(summary,9,&type,&number,&time,code,&length);
            MsiCloseHandle(summary);MsiCloseHandle(db);
            const auto step=removal?L"":Property(install,L"RUSTDESK_ADMISSION_STEP"),nonce=removal?L"":Property(install,L"RUSTDESK_ADMISSION_NONCE"),product=Property(install,L"ProductCode");
            if(removal)id.clear();
            std::wstring challenge;ULONGLONG created=0;
            if(read!=ERROR_SUCCESS || !SessionChallenge(install,id,step,nonce,product,code,challenge,created,removal)
                || !AdmissionPermit(id,step,nonce,Property(install,L"OriginalDatabase"),product,code,challenge,GetCurrentProcessId(),created,removal)) { Refused(install);return ERROR_INSTALL_FAILURE; }
        } else {
            DWORD marker=GetFileAttributesW(L"C:\\ProgramData\\RustDesk\\ManagementAgent\\state\\maintenance.json");
            DWORD markerError=GetLastError();
            if(marker!=INVALID_FILE_ATTRIBUTES || (markerError!=ERROR_FILE_NOT_FOUND && markerError!=ERROR_PATH_NOT_FOUND)){Refused(install);return ERROR_INSTALL_FAILURE;}
        }
        return ERROR_SUCCESS;
    } catch (...) { Refused(install);return ERROR_INSTALL_FAILURE; }
}
#endif
