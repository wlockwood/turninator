#pragma once
// Qualification-only process/WDDM accounting; no external helper process.
#include <Psapi.h>
#include <TlHelp32.h>
#include <dxgi1_4.h>
#include <wrl/client.h>
#include "runtime.hpp"
struct ResourceProbe {
    Microsoft::WRL::ComPtr<IDXGIAdapter3> adapter;
    ResourceProbe() {
        Microsoft::WRL::ComPtr<IDXGIFactory1> factory;
        require(SUCCEEDED(CreateDXGIFactory1(IID_PPV_ARGS(&factory))), "DXGI resource probe unavailable");
        for(UINT i=0;;++i) {
            Microsoft::WRL::ComPtr<IDXGIAdapter1> candidate;
            if(factory->EnumAdapters1(i,&candidate)==DXGI_ERROR_NOT_FOUND) break;
            DXGI_ADAPTER_DESC1 desc{};
            require(SUCCEEDED(candidate->GetDesc1(&desc)), "DXGI description failed");
            if(desc.VendorId==0x10de) { require(SUCCEEDED(candidate.As(&adapter)), "DXGI accounting unavailable"); break; }
        }
        require(adapter!=nullptr,"NVIDIA WDDM resource adapter absent");
    }
    tn::Json sample() {
        PROCESS_MEMORY_COUNTERS_EX process{};process.cb=sizeof(process);
        require(GetProcessMemoryInfo(GetCurrentProcess(),reinterpret_cast<PROCESS_MEMORY_COUNTERS*>(&process),sizeof(process))!=0,"process accounting failed");
        DXGI_QUERY_VIDEO_MEMORY_INFO local{},nonlocal{};
        require(SUCCEEDED(adapter->QueryVideoMemoryInfo(0,DXGI_MEMORY_SEGMENT_GROUP_LOCAL,&local)),"local GPU accounting failed");
        require(SUCCEEDED(adapter->QueryVideoMemoryInfo(0,DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,&nonlocal)),"nonlocal GPU accounting failed");
        DWORD handles=0;require(GetProcessHandleCount(GetCurrentProcess(),&handles)!=0,"handle accounting failed");
        DWORD threads=0;HANDLE snapshot=CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD,0);require(snapshot!=INVALID_HANDLE_VALUE,"thread accounting failed");
        THREADENTRY32 entry{};entry.dwSize=sizeof(entry);
        if(Thread32First(snapshot,&entry))do{if(entry.th32OwnerProcessID==GetCurrentProcessId())++threads;}while(Thread32Next(snapshot,&entry));
        CloseHandle(snapshot);
        return {{"private_bytes",uint64_t(process.PrivateUsage)},{"peak_private_bytes",uint64_t(process.PeakPagefileUsage)},
                {"gpu_local_bytes",local.CurrentUsage},{"gpu_nonlocal_bytes",nonlocal.CurrentUsage},{"gpu_budget_bytes",local.Budget},
                {"handles",handles},{"threads",threads}};
    }
    tn::Json attribution() {
        tn::Json modules=tn::Json::array(),threads=tn::Json::array();
        HANDLE snapshot=CreateToolhelp32Snapshot(TH32CS_SNAPMODULE|TH32CS_SNAPMODULE32,GetCurrentProcessId());
        require(snapshot!=INVALID_HANDLE_VALUE,"module attribution snapshot failed");
        MODULEENTRY32W module{};module.dwSize=sizeof(module);
        if(Module32FirstW(snapshot,&module)) do {
            const auto path=std::filesystem::canonical(module.szExePath);
            modules.push_back({{"name",tn::lux::utf8(std::filesystem::path(module.szModule))},{"path",tn::lux::utf8(path)},
                {"base",reinterpret_cast<uint64_t>(module.modBaseAddr)},{"bytes",module.modBaseSize},{"sha256",tn::lux::hash_file(path)}});
        } while(Module32NextW(snapshot,&module));
        CloseHandle(snapshot);
        using QueryThread=LONG(NTAPI*)(HANDLE,ULONG,PVOID,ULONG,PULONG);
        const auto query=reinterpret_cast<QueryThread>(GetProcAddress(GetModuleHandleW(L"ntdll.dll"),"NtQueryInformationThread"));
        require(query!=nullptr,"thread attribution API unavailable");
        snapshot=CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD,0);require(snapshot!=INVALID_HANDLE_VALUE,"thread attribution snapshot failed");
        THREADENTRY32 thread{};thread.dwSize=sizeof(thread);
        if(Thread32First(snapshot,&thread))do {
            if(thread.th32OwnerProcessID!=GetCurrentProcessId())continue;
            HANDLE handle=OpenThread(THREAD_QUERY_INFORMATION,FALSE,thread.th32ThreadID);
            require(handle!=nullptr,"own thread attribution handle failed");
            void* address=nullptr;const auto result=query(handle,9,&address,sizeof(address),nullptr);CloseHandle(handle);
            require(result>=0,"own thread start query failed");
            const auto start=reinterpret_cast<uint64_t>(address);std::string owner="unmapped";
            for(const auto& record:modules)if(start>=record.at("base").get<uint64_t>()&&start-record.at("base").get<uint64_t>()<record.at("bytes").get<uint64_t>()){owner=record.at("name").get<std::string>();break;}
            threads.push_back({{"id",thread.th32ThreadID},{"start",start},{"module",owner}});
        }while(Thread32Next(snapshot,&thread));
        CloseHandle(snapshot);
        return {{"modules",modules},{"threads",threads},{"reference_count_note","Windows exposes no reliable per-module loader reference count here; these are observed loaded mappings without acquiring extra references."}};
    }
};
