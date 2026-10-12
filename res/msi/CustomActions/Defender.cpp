#include "pch.h"
#include <strutil.h>
#include <string>
#include <vector>

namespace {
std::wstring PsLiteral(const std::wstring& value) {
    std::wstring escaped = L"'";
    for (wchar_t ch : value) {
        escaped += ch;
        if (ch == L'\'') escaped += L'\'';
    }
    return escaped + L"'";
}

std::wstring EncodePowerShell(const std::wstring& script) {
    const char alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const BYTE* bytes = reinterpret_cast<const BYTE*>(script.data());
    const size_t length = script.size() * sizeof(wchar_t);
    std::wstring result;
    for (size_t i = 0; i < length; i += 3) {
        unsigned value = static_cast<unsigned>(bytes[i]) << 16;
        if (i + 1 < length) value |= static_cast<unsigned>(bytes[i + 1]) << 8;
        if (i + 2 < length) value |= bytes[i + 2];
        result += alphabet[(value >> 18) & 63]; result += alphabet[(value >> 12) & 63];
        result += i + 1 < length ? alphabet[(value >> 6) & 63] : '=';
        result += i + 2 < length ? alphabet[value & 63] : '=';
    }
    return result;
}
}

UINT __stdcall ConfigureDefender(MSIHANDLE install) {
    if (FAILED(WcaInitialize(install, "ConfigureDefender"))) return ERROR_SUCCESS;
    LPWSTR data = nullptr;
    if (FAILED(WcaGetProperty(L"CustomActionData", &data))) {
        WcaLog(LOGMSG_STANDARD, "Defender configuration data unavailable; installation continues.");
        return WcaFinalize(ERROR_SUCCESS);
    }
    std::wstring value(data); ReleaseStr(data);
    const size_t first = value.find(L'|'), second = first == std::wstring::npos ? first : value.find(L'|', first + 1);
    if (first == std::wstring::npos || second == std::wstring::npos) {
        WcaLog(LOGMSG_STANDARD, "Defender configuration data invalid; installation continues.");
        return WcaFinalize(ERROR_SUCCESS);
    }
    std::wstring choice = value.substr(0, first), main = value.substr(first + 1, second - first - 1), agent = value.substr(second + 1);
    if (choice != L"0" && choice != L"1" && choice != L"inherit") {
        WcaLog(LOGMSG_STANDARD, "Defender consent invalid; no changes attempted.");
        return WcaFinalize(ERROR_SUCCESS);
    }
    for (const auto& path : {main, agent}) {
        if (path.empty() || path.find_first_of(L"\r\n|") != std::wstring::npos) {
            WcaLog(LOGMSG_STANDARD, "Defender target path invalid; no changes attempted.");
            return WcaFinalize(ERROR_SUCCESS);
        }
    }
    // Shared script is compiled into the CA DLL by the project include path.
    const std::wstring script =
#include "DefenderScript.inc"
        ;
    std::wstring command = script + L"\ntry {$r=Invoke-RdDefenderConfiguration -Choice " + PsLiteral(choice)
        + L" -MainPath " + PsLiteral(main) + L" -AgentPath " + PsLiteral(agent)
        + L"; $r|ConvertTo-Json -Depth 5 -Compress|Write-Output; if($r.Status -notin @('verified','not_selected')){exit 2}}catch{Write-Warning $_.Exception.Message;exit 2};exit 0";
    WCHAR system[MAX_PATH];
    UINT systemLength = GetSystemDirectoryW(system, MAX_PATH);
    if (!systemLength || systemLength >= MAX_PATH) {
        WcaLog(LOGMSG_STANDARD, "Defender system executable path unavailable; installation continues.");
        return WcaFinalize(ERROR_SUCCESS);
    }
    std::wstring executable = std::wstring(system) + L"\\WindowsPowerShell\\v1.0\\powershell.exe";
    std::wstring line = L"\"" + executable + L"\" -NoProfile -NonInteractive -EncodedCommand " + EncodePowerShell(command);
    if (line.size() >= 32760) {
        WcaLog(LOGMSG_STANDARD, "Defender command exceeds Windows process limit; installation continues.");
        return WcaFinalize(ERROR_SUCCESS);
    }
    std::vector<wchar_t> mutableLine(line.begin(), line.end()); mutableLine.push_back(0);
    STARTUPINFOW startup{}; startup.cb = sizeof(startup);
    PROCESS_INFORMATION process{};
    if (!CreateProcessW(executable.c_str(), mutableLine.data(), nullptr, nullptr, FALSE, CREATE_NO_WINDOW, nullptr, system, &startup, &process)) {
        WcaLog(LOGMSG_STANDARD, "Defender helper unavailable (%lu); installation continues.", GetLastError());
        return WcaFinalize(ERROR_SUCCESS);
    }
    CloseHandle(process.hThread);
    DWORD waited = WaitForSingleObject(process.hProcess, 60000), code = 2;
    if (waited == WAIT_OBJECT_0) GetExitCodeProcess(process.hProcess, &code);
    else {
        TerminateProcess(process.hProcess, 2);
        WaitForSingleObject(process.hProcess, 5000);
        WcaLog(LOGMSG_STANDARD, "Defender helper timed out; configuration unverified.");
    }
    CloseHandle(process.hProcess);
    WcaLog(LOGMSG_STANDARD, "Defender configuration helper result: %lu. Details: HKLM\\SOFTWARE\\RustDesk\\Defender\\LastResult. 360 is not configured.", code);
    return WcaFinalize(ERROR_SUCCESS);
}
