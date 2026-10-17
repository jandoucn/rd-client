#include "../res/msi/CustomActions/DesktopShortcut.h"
#include <cstdio>

int wmain() {
    WCHAR machine[256]{}; DWORD length = 256;
    if (!GetComputerNameW(machine, &length) || wcscmp(machine, L"RDLAB-WIN11")) return 90;
    HRESULT com = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(com)) return 91;
    WCHAR temp[MAX_PATH]{}, unique[MAX_PATH]{};
    GetTempPathW(MAX_PATH, temp);
    GetTempFileNameW(temp, L"rdl", 0, unique);
    DeleteFileW(unique); CreateDirectoryW(unique, nullptr);
    std::wstring root(unique), path = root + L"\\RustDesk.lnk";
    std::wstring target = root + L"\\RustDesk.exe";
    int failed = 0, count = 0;
    auto check = [&](bool value, const char* name) { ++count; if (!value) { ++failed; printf("FAIL %s\n", name); } };
    using desktop_shortcut::Presence;
    check(desktop_shortcut::inspect(root, target) == Presence::missing, "absent");
    check(!desktop_shortcut::allow_component(true, desktop_shortcut::inspect(root, target)), "upgrade-missing-must-not-create");
    check(desktop_shortcut::allow_component(false, Presence::missing), "first-install-choice-preserved");
    IShellLinkW* link = nullptr; IPersistFile* persist = nullptr;
    CoCreateInstance(CLSID_ShellLink, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&link));
    if (!link) return 92;
    link->SetPath(target.c_str()); link->QueryInterface(IID_PPV_ARGS(&persist));
    if (!persist || FAILED(persist->Save(path.c_str(), TRUE))) return 93;
    persist->Release(); link->Release();
    check(desktop_shortcut::inspect(root, target) == Presence::present, "real-link");
    check(desktop_shortcut::allow_component(true, desktop_shortcut::inspect(root, target)), "upgrade-existing-may-create");
    check(!desktop_shortcut::allow_component(true, Presence::unknown), "unknown-no-create");
    check(!desktop_shortcut::allow_component(true, Presence::foreign), "foreign-no-create");
    check(desktop_shortcut::inspect(root, root + L"\\other.exe") == Presence::foreign, "foreign-target");
    HANDLE lock = CreateFileW(path.c_str(), GENERIC_READ, 0, nullptr, OPEN_EXISTING, 0, nullptr);
    check(desktop_shortcut::inspect(root, target) == Presence::unknown, "locked");
    CloseHandle(lock);
    check(GetFileAttributesW(path.c_str()) != INVALID_FILE_ATTRIBUTES, "old-file-retained");
    DeleteFileW(path.c_str());
    check(desktop_shortcut::inspect(root, target) == Presence::missing, "deleted-not-recreated");
    RemoveDirectoryW(root.c_str()); CoUninitialize();
    printf("%d cases, %d failures\n", count, failed);
    return failed ? 1 : 0;
}
