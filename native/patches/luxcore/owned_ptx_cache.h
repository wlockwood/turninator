// Turninator's private LuxCore Windows CUDA cache. Apache-2.0.
// The caller supplies an application-owned root and SHA256 identities of the
// verified compiler modules and SDK build. These claims are not authentication.
#pragma once
#ifndef _WIN32
#error This qualified cache implementation requires Windows.
#endif
#include <windows.h>
#include <bcrypt.h>
#include <array>
#include <atomic>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <string>
#include <vector>
#include <stdexcept>

namespace turninator_luxcore {
class OwnedPtxCache {
public:
    static constexpr std::uint64_t MaxBytes = 64ull * 1024 * 1024;
    static std::string Sha256(const void *bytes, size_t size) {
        BCRYPT_ALG_HANDLE algorithm = nullptr;
        if (BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) < 0)
            throw std::runtime_error("CUDA cache SHA256 provider failed");
        std::array<unsigned char,32> digest{};
        const auto status = size > ULONG_MAX ? -1 : BCryptHash(algorithm, nullptr, 0,
            reinterpret_cast<PUCHAR>(const_cast<void *>(bytes)), static_cast<ULONG>(size),
            digest.data(), static_cast<ULONG>(digest.size()));
        BCryptCloseAlgorithmProvider(algorithm, 0);
        if (status < 0) throw std::runtime_error("CUDA cache SHA256 failed");
        const char *hex = "0123456789abcdef"; std::string result;
        for (auto b : digest) { result += hex[b >> 4]; result += hex[b & 15]; }
        return result;
    }
    static std::string Key(const std::vector<std::string> &options, const std::string &source,
            const std::string &program, const std::string &architecture) {
        // Length framing preserves argument boundaries and ordering.
        std::string input = "Turninator LuxCore PTX key v1\n";
        const auto add = [&](const std::string &s) { input += std::to_string(s.size()) + ":" + s; };
        add(source); add(program); add(architecture); add(std::to_string(options.size()));
        for (const auto &option : options) add(option);
        return Sha256(input.data(), input.size());
    }
    OwnedPtxCache(const std::string &utf8Root, const std::string &compiler, const std::string &sdk)
        : compilerIdentity(compiler), sdkIdentity(sdk) {
        if (!IsHash(compiler) || !IsHash(sdk))
            throw std::runtime_error("CUDA cache requires lowercase SHA256 compiler_identity and sdk_identity");
        if (utf8Root.empty() || utf8Root.find('\0') != std::string::npos)
            throw std::runtime_error("CUDA cache requires an absolute app-owned UTF8 directory");
        const int count = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, utf8Root.data(), static_cast<int>(utf8Root.size()), nullptr, 0);
        if (!count) throw std::runtime_error("CUDA cache directory is invalid UTF8");
        std::wstring wide(count,L'\0');
        MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, utf8Root.data(), static_cast<int>(utf8Root.size()), wide.data(), count);
        std::filesystem::path root(wide);
        // Limit the qualified surface to local drive paths, without alternate
        // data streams, device namespace escapes, traversal or reparse points.
        if (!root.is_absolute() || root.root_name().wstring().size()!=2 || wide.find(L':',2)!=std::wstring::npos)
            throw std::runtime_error("CUDA cache requires an absolute local drive directory");
        for (const auto &part : root.relative_path()) {
            const auto s=part.wstring();
            if (s==L".." || s==L"." || (!s.empty() && (s.back()==L'.' || s.back()==L' ')))
                throw std::runtime_error("CUDA cache rejects ambiguous directory components");
        }
        directory = root / "turninator-luxcore-ptx-v1" / compiler / sdk;
        CheckDirectory(true);
    }
    const std::filesystem::path &Directory() const { return directory; }
    std::filesystem::path Path(const std::string &key) const {
        if (!IsHash(key)) throw std::runtime_error("CUDA cache invalid key");
        return NativePath(directory / (key + ".ptxrec"));
    }
    bool Load(const std::string &key, std::string &ptx) const {
        ptx.clear(); CheckDirectory(false);
        Handle file(CreateFileW(Path(key).c_str(), GENERIC_READ,
            FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE, nullptr, OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT|FILE_FLAG_SEQUENTIAL_SCAN, nullptr));
        if (!file.Valid()) return false;
        BY_HANDLE_FILE_INFORMATION info{}; LARGE_INTEGER length{};
        if (!GetFileInformationByHandle(file.value,&info) ||
            (info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT|FILE_ATTRIBUTE_DIRECTORY)) ||
            !GetFileSizeEx(file.value,&length) || length.QuadPart < sizeof(Header) ||
            length.QuadPart > sizeof(Header)+MaxBytes) return false;
        Header h{}; DWORD got=0;
        if (!ReadFile(file.value,&h,sizeof h,&got,nullptr) || got!=sizeof h ||
            std::memcmp(h.magic,"TNPTX01",8) ||
            std::memcmp(h.compiler,compilerIdentity.data(),64) ||
            std::memcmp(h.sdk,sdkIdentity.data(),64) || std::memcmp(h.key,key.data(),64) ||
            !h.size || h.size>MaxBytes || length.QuadPart!=sizeof(Header)+h.size) return false;
        std::string data(static_cast<size_t>(h.size),'\0');
        if (!ReadFile(file.value,data.data(),static_cast<DWORD>(data.size()),&got,nullptr) || got!=data.size()) return false;
        const auto digest=Sha256(data.data(),data.size());
        if (std::memcmp(h.digest,digest.data(),64) || data.back()!='\0') return false;
        ptx.swap(data); return true;
    }
    void Store(const std::string &key, const char *ptx, size_t size) const {
        if (!size || size>MaxBytes) throw std::runtime_error("CUDA cache PTX size outside bound");
        if (!ptx || ptx[size-1]!='\0') throw std::runtime_error("CUDA cache PTX must be terminated");
        const auto destination=Path(key); CheckDirectory(false);
        Header h{}; std::memcpy(h.magic,"TNPTX01",8);
        std::memcpy(h.compiler,compilerIdentity.data(),64); std::memcpy(h.sdk,sdkIdentity.data(),64);
        std::memcpy(h.key,key.data(),64); h.size=size;
        const auto digest=Sha256(ptx,size); std::memcpy(h.digest,digest.data(),64);
        static std::atomic<unsigned long long> sequence{0};
        std::filesystem::path temporary; Handle file;
        for (int attempt=0;attempt<32;++attempt) {
            temporary=NativePath(directory/(key+".tmp-"+std::to_string(GetCurrentProcessId())+"-"+std::to_string(sequence++)));
            file.value=CreateFileW(temporary.c_str(),GENERIC_WRITE,0,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);
            if(file.Valid()) break;
            if(GetLastError()!=ERROR_FILE_EXISTS) throw std::runtime_error("CUDA cache temporary file creation failed");
        }
        if(!file.Valid()) throw std::runtime_error("CUDA cache temporary file collision limit");
        try {
            DWORD written=0;
            if(!WriteFile(file.value,&h,sizeof h,&written,nullptr) || written!=sizeof h ||
               !WriteFile(file.value,ptx,static_cast<DWORD>(size),&written,nullptr) || written!=size ||
               !FlushFileBuffers(file.value)) throw std::runtime_error("CUDA cache record write failed");
            file.Close(); CheckDirectory(false);
            // Windows can transiently deny replacement when another publisher
            // is closing its rename handle. Retry only this bounded IO race.
            for (unsigned attempt=0;;++attempt) {
                const auto attributes=GetFileAttributesW(destination.c_str());
                if(attributes!=INVALID_FILE_ATTRIBUTES && (attributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT)))
                    throw std::runtime_error("CUDA cache destination is not a regular owned record");
                if(MoveFileExW(temporary.c_str(),destination.c_str(),MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH)) break;
                const auto error=GetLastError();
                if(attempt==31 || (error!=ERROR_SHARING_VIOLATION && error!=ERROR_ACCESS_DENIED))
                    throw std::runtime_error("CUDA cache atomic publication failed: "+std::to_string(error));
                Sleep(2);
            }
        } catch(...) { file.Close(); DeleteFileW(temporary.c_str()); throw; }
    }
private:
    struct Header { char magic[8], compiler[64], sdk[64], key[64], digest[64]; std::uint64_t size; };
    static_assert(sizeof(Header)==272);
    struct Handle {
        HANDLE value=INVALID_HANDLE_VALUE;
        Handle()=default; explicit Handle(HANDLE h):value(h){}
        Handle(const Handle&)=delete; Handle& operator=(const Handle&)=delete;
        bool Valid() const {return value!=INVALID_HANDLE_VALUE;}
        void Close() {if(Valid()){CloseHandle(value);value=INVALID_HANDLE_VALUE;}}
        ~Handle(){Close();}
    };
    static bool IsHash(const std::string &s) {
        if(s.size()!=64)return false;
        for(char c:s)if(!((c>='0'&&c<='9')||(c>='a'&&c<='f')))return false;
        return true;
    }
    static std::filesystem::path NativePath(const std::filesystem::path &p) {
        auto copy=p; copy.make_preferred();
        return std::filesystem::path(L"\\\\?\\"+copy.wstring());
    }
    void CheckDirectory(bool create) const {
        auto current=directory.root_path();
        for(const auto &part:directory.relative_path()) {
            current/=part;
            const auto native=NativePath(current);
            auto attributes=GetFileAttributesW(native.c_str());
            if(attributes==INVALID_FILE_ATTRIBUTES && create) {
                if(!CreateDirectoryW(native.c_str(),nullptr) && GetLastError()!=ERROR_ALREADY_EXISTS)
                    throw std::runtime_error("CUDA cache owned directory creation failed");
                attributes=GetFileAttributesW(native.c_str());
            }
            if(attributes==INVALID_FILE_ATTRIBUTES || !(attributes&FILE_ATTRIBUTE_DIRECTORY) || (attributes&FILE_ATTRIBUTE_REPARSE_POINT))
                throw std::runtime_error("CUDA cache requires existing non-reparse owned directories");
        }
    }
    std::filesystem::path directory;
    std::string compilerIdentity,sdkIdentity;
};
}
