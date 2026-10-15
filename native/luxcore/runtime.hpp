#pragma once
#include "backend.hpp"
#include <Windows.h>
#include <winver.h>
#include <bcrypt.h>
#include <filesystem>
#include <fstream>
#include <array>
#include <algorithm>
namespace tn::lux {
inline constexpr char sdk_hash[]="63e96b108e5007c6d378e2ac0b575d5db6b9e9e8b7b9a69a5b7cef3856f457b5";
inline constexpr char compiler_hash[]="4cbf85c6afdf0092be563772ec2cd142336d933d4c57b18fba584bb73514ea9c";
inline std::string utf8(const std::filesystem::path& p) {const auto s=p.u8string();return {s.begin(),s.end()};}
inline std::string hash_file(const std::filesystem::path& path) {
    struct Hash { BCRYPT_ALG_HANDLE algorithm{}; BCRYPT_HASH_HANDLE hash{};
        ~Hash() {if(hash) BCryptDestroyHash(hash);if(algorithm) BCryptCloseAlgorithmProvider(algorithm,0);} } h;
    if(BCryptOpenAlgorithmProvider(&h.algorithm,BCRYPT_SHA256_ALGORITHM,nullptr,0)<0 || BCryptCreateHash(h.algorithm,&h.hash,nullptr,0,nullptr,0,0)<0) throw std::runtime_error("SHA256 initialization failed");
    std::ifstream file(path,std::ios::binary);if(!file) throw std::runtime_error("Cannot read runtime artifact");
    std::array<char,65536> block{};
    while(file) {file.read(block.data(),block.size());const auto n=file.gcount();if(n&&BCryptHashData(h.hash,reinterpret_cast<PUCHAR>(block.data()),static_cast<ULONG>(n),0)<0) throw std::runtime_error("SHA256 update failed");}
    if(!file.eof()) throw std::runtime_error("Runtime artifact read failed");
    std::array<unsigned char,32> result{};if(BCryptFinishHash(h.hash,result.data(),static_cast<ULONG>(result.size()),0)<0) throw std::runtime_error("SHA256 finish failed");
    constexpr char hex[]="0123456789abcdef";std::string s;for(auto b:result){s+=hex[b>>4];s+=hex[b&15];}return s;
}
inline std::filesystem::path module_path(HMODULE module) {
    std::array<wchar_t,32768> path{};const auto n=GetModuleFileNameW(module,path.data(),static_cast<DWORD>(path.size()));
    if(!n||n>=path.size()) throw std::runtime_error("Runtime module path unavailable");
    return std::filesystem::canonical(std::filesystem::path(path.data(),path.data()+n));
}
inline std::string file_version(const std::filesystem::path& path){
    DWORD unused=0;const auto size=GetFileVersionInfoSizeW(path.c_str(),&unused);
    if(!size||size>(1u<<20))throw std::runtime_error("Driver file version unavailable");
    std::vector<uint8_t> data(size);if(!GetFileVersionInfoW(path.c_str(),0,size,data.data()))throw std::runtime_error("Driver file version read failed");
    VS_FIXEDFILEINFO* info=nullptr;UINT bytes=0;
    if(!VerQueryValueW(data.data(),L"\\",reinterpret_cast<void**>(&info),&bytes)||bytes<sizeof(*info)||info->dwSignature!=0xfeef04bd)throw std::runtime_error("Driver file version invalid");
    return std::to_string(HIWORD(info->dwFileVersionMS))+"."+std::to_string(LOWORD(info->dwFileVersionMS))+"."+std::to_string(HIWORD(info->dwFileVersionLS))+"."+std::to_string(LOWORD(info->dwFileVersionLS));
}
class Runtime {
    std::vector<HMODULE> retained;
public:
    Json artifacts=Json::object();
    std::filesystem::path root;
    Runtime() {
        HMODULE self{};if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,reinterpret_cast<LPCWSTR>(&backend_id),&self)) throw std::runtime_error("Bridge module path unavailable");
        root=module_path(self).parent_path();
        try {
            // Bind and attest all redistributed DLLs before SDK initialization.
            // Taking our own LoadLibrary reference also retains ambient matches.
            for(const auto& entry:runtime_artifacts) {
                const auto expected=std::filesystem::canonical(root/entry.name);
                const auto module=LoadLibraryExW(expected.c_str(),nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32);
                if(!module) throw std::runtime_error("Pinned runtime module could not load");
                retained.push_back(module); const auto actual=module_path(module);
                if(!std::filesystem::equivalent(expected,actual)||hash_file(actual)!=entry.sha256) throw std::runtime_error("Pinned runtime module path or SHA256 mismatch");
                // CUEW and OIDN subsequently bind by pinned module name. An
                // absolute load can coexist with an earlier same-name module;
                // attest the module their named lookup will actually return.
                if(GetModuleHandleW(entry.name)!=module)throw std::runtime_error("Pinned module name is bound to a conflicting loaded path");
                artifacts[entry.key]={{"path",utf8(actual)},{"sha256",entry.sha256}};
            }
            using Version=int(__cdecl*)(int*,int*);const auto version=reinterpret_cast<Version>(GetProcAddress(GetModuleHandleW(L"nvrtc64_120_0.dll"),"nvrtcVersion"));int major{},minor{};
            if(!version||version(&major,&minor)||major!=12||minor!=8) throw std::runtime_error("Pinned NVRTC version mismatch");
            // The pinned SDK's synchronous mesh-area OpenMP region otherwise
            // creates VCOMP workers that survive DLL unload. Configure this
            // dedicated caller thread before Init or any scene operation.
            HMODULE openmp{};
            if(!GetModuleHandleExW(0,L"vcomp140.dll",&openmp))throw std::runtime_error("SDK OpenMP runtime is unavailable");
            retained.push_back(openmp);
            std::array<wchar_t,32768> system{};const auto size=GetSystemDirectoryW(system.data(),static_cast<UINT>(system.size()));
            if(!size||size>=system.size())throw std::runtime_error("System runtime directory unavailable");
            const auto expected=std::filesystem::canonical(std::filesystem::path(system.data())/L"vcomp140.dll");
            const auto actual=module_path(openmp);const auto digest=hash_file(actual);
            const auto packaged=root/L"vcomp140.dll";
            const bool owned=std::filesystem::exists(packaged)&&actual==std::filesystem::canonical(packaged);
            const bool qualified=digest=="95d4ce4a6802d1e18b5e0e1722cc30ea72ca7e033f83828f05c0b7b993fe7cbf"||digest=="55aba23cdcd6484fbb06f4155b8ca75adfce7a881f10afd0c49457165e677164";
            if((!owned&&actual!=expected)||!qualified)throw std::runtime_error("Qualified VCOMP runtime path or hash differs; preparation requires the qualified runtime");
            using SetThreads=void(__cdecl*)(int);using GetThreads=int(__cdecl*)();
            const auto set_threads=reinterpret_cast<SetThreads>(GetProcAddress(openmp,"omp_set_num_threads"));
            const auto get_threads=reinterpret_cast<GetThreads>(GetProcAddress(openmp,"omp_get_max_threads"));
            if(!set_threads||!get_threads)throw std::runtime_error("OpenMP worker policy exports unavailable");
            for(const auto address:{reinterpret_cast<LPCWSTR>(set_threads),reinterpret_cast<LPCWSTR>(get_threads)}){
                HMODULE owner{};if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,address,&owner)||owner!=openmp)throw std::runtime_error("OpenMP policy export points outside verified runtime");
            }
            set_threads(1);if(get_threads()!=1)throw std::runtime_error("Could not establish dedicated worker OpenMP serial policy");
            artifacts["openmp"]={{"path",utf8(actual)},{"sha256",digest}};
        } catch(...) {release();throw;}
    }
    ~Runtime() {release();}
    Runtime(const Runtime&)=delete;
    Runtime& operator=(const Runtime&)=delete;
private:
    void release() noexcept {for(auto i=retained.rbegin();i!=retained.rend();++i) FreeLibrary(*i);retained.clear();}
    struct Entry {const wchar_t* name;const char* key;const char* sha256;};
    inline static constexpr Entry runtime_artifacts[]={
        {L"nvrtc-builtins64_128.dll","nvrtc_builtins","a7afa2a40cbd3f922a33d8bf71c13cb26ca707c7016a9bc5fcde36759ba668f5"},
        {L"nvrtc64_120_0.dll","nvrtc","02d2d7ef4690bf1a55dda1d3ac94c86ce8d16920071c87bcd3e70e94ef346e93"},
        {L"luxcore.dll","sdk",sdk_hash},
        {L"LuxOpenImageDenoise.dll","oidn","3251dcdb742493a7d86b2daf6dd6fb02c7aa2279b688a041d3b65733d0641270"},
        {L"LuxOpenImageDenoise_core.dll","oidn_core","de4668a98e437ea27dd599c028fd1b270335320ce3c2b6c7d184718bac2da9b7"},
        {L"LuxOpenImageDenoise_device_cpu.dll","oidn_cpu","f4630884ee83a18f4fa2818153361e5caa4a130e5ccfcedd0e9a648e72954fca"},
        {L"embree4.dll","embree","f477315fe0c2e6cf05dffb90ee45bfc24959d8f15714aa2cadd3c2931bd53c86"},
        {L"tbb12.dll","tbb","992b9b0d049d6ec37a5dafade96d838d5aea123b510fdeee140ae358e436665f"},
        {L"tbbmalloc.dll","tbbmalloc","a31901f7c544ba3168d8897966b3e4f029157611ec80fb990fd4193bce1a67a9"}
    };
};
}
