#pragma once
#include <windows.h>
#include <shlobj.h>
#include <shlwapi.h>
#include <string>
#include <vector>

#pragma comment(lib, "shell32.lib")
#pragma comment(lib, "shlwapi.lib")
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "uuid.lib")

namespace desktop_shortcut {
enum class Presence { missing, present, unknown, foreign };
inline bool allow_component(bool upgrade, Presence presence) {
    return !upgrade || presence == Presence::present;
}

// The caller supplies OS-known folders, never an MSI property or command line.
inline Presence inspect(const std::wstring& desktop, const std::wstring& target) {
    // Hold each ancestor without delete sharing during the read. Do not follow
    // a junction or symlink while inspecting an elevated installer input.
    struct Pins {
        std::vector<HANDLE> handles;
        ~Pins() { for (HANDLE h : handles) CloseHandle(h); }
    } pins;
    if (desktop.size() < 3 || desktop[1] != L':' || desktop[2] != L'\\') return Presence::unknown;
    for (size_t end = 3; ; ) {
        std::wstring ancestor = desktop.substr(0, end);
        HANDLE directory = CreateFileW(ancestor.c_str(), FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
        if (directory == INVALID_HANDLE_VALUE) return Presence::unknown;
        pins.handles.push_back(directory);
        BY_HANDLE_FILE_INFORMATION attributes{};
        if (!GetFileInformationByHandle(directory, &attributes)
            || (attributes.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT)
            || !(attributes.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)) return Presence::unknown;
        if (end == desktop.size()) break;
        end = desktop.find(L'\\', end + 1);
        if (end == std::wstring::npos) end = desktop.size();
    }
    const std::wstring path = desktop + L"\\RustDesk.lnk";
    HANDLE file = CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr,
        OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
    if (file == INVALID_HANDLE_VALUE) {
        return GetLastError() == ERROR_FILE_NOT_FOUND ? Presence::missing : Presence::unknown;
    }
    BY_HANDLE_FILE_INFORMATION info{};
    LARGE_INTEGER size{};
    if (!GetFileInformationByHandle(file, &info) || !GetFileSizeEx(file, &size)
        || (info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY))
        || info.nNumberOfLinks != 1 || size.QuadPart < 76 || size.QuadPart > 1048576) {
        CloseHandle(file);
        return Presence::unknown;
    }
    std::vector<BYTE> bytes(static_cast<size_t>(size.QuadPart));
    DWORD read = 0;
    BOOL ok = ReadFile(file, bytes.data(), static_cast<DWORD>(bytes.size()), &read, nullptr);
    CloseHandle(file);
    if (!ok || read != bytes.size()) return Presence::unknown;
    IStream* stream = SHCreateMemStream(bytes.data(), read);
    if (!stream) return Presence::unknown;
    IShellLinkW* link = nullptr;
    HRESULT hr = CoCreateInstance(CLSID_ShellLink, nullptr, CLSCTX_INPROC_SERVER,
        IID_PPV_ARGS(&link));
    IPersistStream* persist = nullptr;
    if (SUCCEEDED(hr)) hr = link->QueryInterface(IID_PPV_ARGS(&persist));
    if (SUCCEEDED(hr)) hr = persist->Load(stream);
    WCHAR actual[32768]{};
    if (SUCCEEDED(hr)) hr = link->GetPath(actual, 32768, nullptr, SLGP_RAWPATH);
    if (persist) persist->Release();
    if (link) link->Release();
    stream->Release();
    if (FAILED(hr) || !actual[0]) return Presence::unknown;
    return _wcsicmp(actual, target.c_str()) == 0 ? Presence::present : Presence::foreign;
}
}
