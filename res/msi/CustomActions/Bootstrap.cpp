#include "pch.h"
#include <strutil.h>
#include <shlobj.h>
#include <string>
#include <vector>

namespace {
std::wstring Literal(const std::wstring& value) {
    std::wstring result = L"'";
    for (wchar_t ch : value) {
        result += ch;
        if (ch == L'\'') result += ch;
    }
    return result + L"'";
}

std::wstring Encoded(const std::wstring& value) {
    const wchar_t alphabet[] = L"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const BYTE* bytes = reinterpret_cast<const BYTE*>(value.data());
    const size_t size = value.size() * sizeof(wchar_t);
    std::wstring result;
    for (size_t i = 0; i < size; i += 3) {
        unsigned n = static_cast<unsigned>(bytes[i]) << 16;
        if (i + 1 < size) n |= static_cast<unsigned>(bytes[i + 1]) << 8;
        if (i + 2 < size) n |= bytes[i + 2];
        result += alphabet[(n >> 18) & 63]; result += alphabet[(n >> 12) & 63];
        result += i + 1 < size ? alphabet[(n >> 6) & 63] : L'=';
        result += i + 2 < size ? alphabet[n & 63] : L'=';
    }
    return result;
}

SC_HANDLE OpenFixedMainService(DWORD access) {
    SC_HANDLE manager = OpenSCManagerW(nullptr, nullptr, SC_MANAGER_CONNECT);
    if (!manager) return nullptr;
    SC_HANDLE service = OpenServiceW(manager, L"RustDesk", access | SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS);
    CloseServiceHandle(manager);
    if (!service) return nullptr;
    DWORD required = 0;
    QueryServiceConfigW(service, nullptr, 0, &required);
    if (!required || required > 65536) { CloseServiceHandle(service); return nullptr; }
    std::vector<BYTE> buffer(required);
    auto config = reinterpret_cast<QUERY_SERVICE_CONFIGW*>(buffer.data());
    if (!QueryServiceConfigW(service, config, required, &required)
        || config->dwServiceType != SERVICE_WIN32_OWN_PROCESS
        || config->dwStartType == SERVICE_DISABLED
        || !config->lpServiceStartName || _wcsicmp(config->lpServiceStartName, L"LocalSystem") != 0
        || !config->lpBinaryPathName
        || _wcsicmp(config->lpBinaryPathName, L"\"C:\\Program Files\\RustDesk\\RustDesk.exe\" --service") != 0) {
        CloseServiceHandle(service);
        return nullptr;
    }
    return service;
}

bool QueryMain(SC_HANDLE service, SERVICE_STATUS_PROCESS& state) {
    DWORD size = 0;
    return QueryServiceStatusEx(service, SC_STATUS_PROCESS_INFO,
        reinterpret_cast<LPBYTE>(&state), sizeof(state), &size) != FALSE;
}
}

UINT __stdcall InspectBootstrapUpgrade(MSIHANDLE install) {
    if (FAILED(WcaInitialize(install, "InspectBootstrapUpgrade"))) return ERROR_INSTALL_FAILURE;
    // A private property is recomputed in the execute sequence, never trusted
    // from a public command-line override or an earlier UI search.
    if (MsiSetPropertyW(install, L"RdBootstrapUpgradeReady", L"") != ERROR_SUCCESS)
        return WcaFinalize(ERROR_INSTALL_FAILURE);
    if (MsiSetPropertyW(install, L"RdBootstrapMainWasRunning", L"") != ERROR_SUCCESS)
        return WcaFinalize(ERROR_INSTALL_FAILURE);
    LPWSTR raw = nullptr;
    if (FAILED(WcaGetProperty(L"RdBootstrapEdition", &raw))) return WcaFinalize(ERROR_INSTALL_FAILURE);
    const std::wstring edition(raw); ReleaseStr(raw);
    if (edition != L"standard" && edition != L"sos") return WcaFinalize(ERROR_INSTALL_FAILURE);
    PWSTR programData = nullptr;
    if (FAILED(SHGetKnownFolderPath(FOLDERID_ProgramData, 0, nullptr, &programData)))
        return WcaFinalize(ERROR_INSTALL_FAILURE);
    const std::wstring root = std::wstring(programData) + L"\\RustDesk";
    CoTaskMemFree(programData);
    WCHAR system[MAX_PATH];
    const UINT length = GetSystemDirectoryW(system, MAX_PATH);
    if (!length || length >= MAX_PATH) return WcaFinalize(ERROR_INSTALL_FAILURE);
    const std::wstring executable = std::wstring(system) + L"\\WindowsPowerShell\\v1.0\\powershell.exe";
    const std::wstring source =
#include "BootstrapPreflight.inc"
        ;
    const std::wstring script = source + L"\ntry {if(Test-RdBootstrapUpgrade -Root " + Literal(root)
        + L" -Edition " + Literal(edition)
        + L" -NormalPath 'C:\\Program Files\\RustDesk'){exit 0};exit 10}catch{exit 20}";
    const std::wstring line = L"\"" + executable + L"\" -NoProfile -NonInteractive -EncodedCommand " + Encoded(script);
    if (line.size() >= 32760) return WcaFinalize(ERROR_INSTALL_FAILURE);
    std::vector<wchar_t> arguments(line.begin(), line.end()); arguments.push_back(0);
    STARTUPINFOW startup{}; startup.cb = sizeof(startup);
    PROCESS_INFORMATION process{};
    if (!CreateProcessW(executable.c_str(), arguments.data(), nullptr, nullptr, FALSE,
            CREATE_NO_WINDOW, nullptr, system, &startup, &process)) {
        WcaLog(LOGMSG_STANDARD, "Bootstrap preflight process creation failed: %lu", GetLastError());
        return WcaFinalize(ERROR_INSTALL_FAILURE);
    }
    CloseHandle(process.hThread);
    DWORD code = 20;
    const DWORD waited = WaitForSingleObject(process.hProcess, 60000);
    if (waited == WAIT_OBJECT_0) GetExitCodeProcess(process.hProcess, &code);
    else {
        // This child only inspects metadata. No installation or service action
        // is in progress in it, and a timeout must not authorize replacement.
        TerminateProcess(process.hProcess, 20);
        WaitForSingleObject(process.hProcess, 5000);
    }
    CloseHandle(process.hProcess);
    WcaLog(LOGMSG_STANDARD, "Bootstrap preflight result: %lu (0=verified, 10=absent or locally blocked)", code);
    if (code == 10) return WcaFinalize(ERROR_SUCCESS);
    if (code != 0 || MsiSetPropertyW(install, L"RdBootstrapUpgradeReady", L"1") != ERROR_SUCCESS)
        return WcaFinalize(ERROR_INSTALL_FAILURE);
    SC_HANDLE service = OpenFixedMainService(0);
    SERVICE_STATUS_PROCESS state{};
    if (!service) return WcaFinalize(ERROR_INSTALL_FAILURE);
    const bool observed = QueryMain(service, state);
    CloseServiceHandle(service);
    if (!observed || (state.dwCurrentState != SERVICE_RUNNING && state.dwCurrentState != SERVICE_STOPPED))
        return WcaFinalize(ERROR_INSTALL_FAILURE);
    if (state.dwCurrentState == SERVICE_RUNNING
        && MsiSetPropertyW(install, L"RdBootstrapMainWasRunning", L"1") != ERROR_SUCCESS)
        return WcaFinalize(ERROR_INSTALL_FAILURE);
    return WcaFinalize(ERROR_SUCCESS);
}

UINT __stdcall StopMainForBootstrapRollback(MSIHANDLE install) {
    if (FAILED(WcaInitialize(install, "StopMainForBootstrapRollback"))) return ERROR_INSTALL_FAILURE;
    SC_HANDLE service = OpenFixedMainService(SERVICE_STOP);
    if (!service) return WcaFinalize(ERROR_INSTALL_FAILURE);
    SERVICE_STATUS_PROCESS state{};
    bool ok = QueryMain(service, state);
    HANDLE process = nullptr;
    if (ok && state.dwCurrentState != SERVICE_STOPPED) {
        process = OpenProcess(SYNCHRONIZE, FALSE, state.dwProcessId);
        if (!process) ok = false;
        if (ok && state.dwCurrentState != SERVICE_STOP_PENDING) {
            SERVICE_STATUS stopped{};
            ok = ControlService(service, SERVICE_CONTROL_STOP, &stopped) != FALSE;
        }
        const ULONGLONG deadline = GetTickCount64() + 60000;
        while (ok) {
            ok = QueryMain(service, state);
            if (!ok || (state.dwCurrentState == SERVICE_STOPPED && WaitForSingleObject(process, 0) == WAIT_OBJECT_0)) break;
            if (GetTickCount64() >= deadline) { ok = false; break; }
            Sleep(100);
        }
    }
    if (process) CloseHandle(process);
    CloseServiceHandle(service);
    WcaLog(LOGMSG_STANDARD, "Fixed main service quiesced before file rollback: %d", ok);
    return WcaFinalize(ok ? ERROR_SUCCESS : ERROR_INSTALL_FAILURE);
}
