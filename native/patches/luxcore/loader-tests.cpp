#include <Windows.h>
#include <cuew.h>
#include <filesystem>
#include <iostream>
#include <string>
int wmain(int argc, wchar_t **argv) {
    if (argc < 3) return 2;
    const std::wstring mode(argv[1]);
    const bool preload = mode != L"missing";
    const bool success = mode == L"preloaded" || mode == L"ambient";
    if (mode == L"ambient" || mode == L"wrong-builtins") {
        if (argc != 4) return 2;
        const std::filesystem::path ambient(argv[3]);
        if (!LoadLibraryExW((ambient / L"nvrtc64_130_0.dll").c_str(), nullptr,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32) ||
            !LoadLibraryExW((ambient / L"nvrtc-builtins64_131.dll").c_str(), nullptr,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)) return 3;
    }
    HMODULE expected = nullptr;
    if (preload) {
        const std::filesystem::path root(argv[2]);
        expected = LoadLibraryExW((root / L"nvrtc64_120_0.dll").c_str(), nullptr,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
        if (!expected) return 3;
        if (success && !LoadLibraryExW((root / L"nvrtc-builtins64_128.dll").c_str(), nullptr,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)) return 3;
    }
    const auto result = cuewInit(CUEW_INIT_NVRTC);
    if (!success) {
        if (result == CUEW_SUCCESS) { std::cerr << "LOADER_FAIL searched without verified preload\n"; return 4; }
    } else {
        int major = 0, minor = 0;
        if (result != CUEW_SUCCESS || !nvrtcVersion || nvrtcVersion(&major, &minor) != NVRTC_SUCCESS ||
            major != 12 || minor != 8 || reinterpret_cast<FARPROC>(nvrtcVersion) != GetProcAddress(expected, "nvrtcVersion")) {
            std::cerr << "LOADER_FAIL wrong compiler binding " << major << '.' << minor << '\n'; return 5;
        }
    }
    std::cout << "LOADER_PASS " << (success ? "verified 12.8 function binding" : "missing qualified compiler pair rejected") << '\n';
}
