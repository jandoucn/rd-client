#include "DesktopShortcut.h"
#include <msi.h>
#include <msiquery.h>
#pragma comment(lib, "msi.lib")

namespace {
bool property(MSIHANDLE install, const WCHAR* name, std::wstring& result) {
    WCHAR value[4096]{}; DWORD size = 4096;
    if (MsiGetPropertyW(install, name, value, &size) != ERROR_SUCCESS) return false;
    result.assign(value, size);
    return true;
}
bool folder(REFKNOWNFOLDERID id, std::wstring& result) {
    PWSTR value = nullptr;
    if (FAILED(SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, nullptr, &value))) return false;
    result = value; CoTaskMemFree(value);
    return true;
}
}

// Read-only inspection. Windows Installer retains ownership of shortcut writes
// and rollback; no paths or shell-link bytes cross a deferred CA boundary.
extern "C" UINT __stdcall InspectDesktopShortcut(MSIHANDLE install) {
    if (MsiSetPropertyW(install, L"RustDeskDesktopAllowed", L"0") != ERROR_SUCCESS)
        return ERROR_INSTALL_FAILURE;
    std::wstring installed, related, remove;
    if (!property(install, L"Installed", installed)
        || !property(install, L"WIX_UPGRADE_DETECTED", related)
        || !property(install, L"REMOVE", remove)) return ERROR_INSTALL_FAILURE;
    if (remove == L"ALL") return ERROR_SUCCESS;
    if (installed.empty() && related.empty())
        return MsiSetPropertyW(install, L"RustDeskDesktopAllowed", L"1");

    std::wstring desktop, programFiles, programFiles32;
    if (!folder(FOLDERID_PublicDesktop, desktop)
        || !folder(FOLDERID_ProgramFiles, programFiles)
        || !folder(FOLDERID_ProgramFilesX86, programFiles32)) return ERROR_INSTALL_FAILURE;
    HRESULT com = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(com) && com != RPC_E_CHANGED_MODE) return ERROR_INSTALL_FAILURE;
    auto state = desktop_shortcut::inspect(desktop, programFiles + L"\\RustDesk\\RustDesk.exe");
    if (state == desktop_shortcut::Presence::foreign)
        state = desktop_shortcut::inspect(desktop, programFiles32 + L"\\RustDesk\\RustDesk.exe");
    if (SUCCEEDED(com)) CoUninitialize();
    // Unknown or a different target must not reach the old package's uninstall,
    // which could delete that file even if the new component were disabled.
    if (state == desktop_shortcut::Presence::unknown || state == desktop_shortcut::Presence::foreign)
        return ERROR_INSTALL_FAILURE;
    if (MsiSetPropertyW(install, L"DesktopFolder", (desktop + L"\\").c_str()) != ERROR_SUCCESS)
        return ERROR_INSTALL_FAILURE;
    return MsiSetPropertyW(install, L"RustDeskDesktopAllowed",
        desktop_shortcut::allow_component(true, state) ? L"1" : L"0");
}
