// Behavioral tests for the private Windows PTX record, independent of CUDA/GPU.
#include "owned_ptx_cache.h"
#include <iostream>
#include <thread>
#include <fstream>
using turninator_luxcore::OwnedPtxCache;
static void require(bool ok, const char *message) { if (!ok) throw std::runtime_error(message); }
template<class F> static void rejects(F f, const char *message) {
    bool threw = false; try { f(); } catch (const std::exception &) { threw = true; }
    require(threw, message);
}
int main(int argc, char **argv) {
    try {
        require(argc == 2 || argc == 3, "pass unique owned test root and optional foreign PTX fixture");
        const std::string root = argv[1], compiler(64, 'a'), sdk(64, 'b');
        rejects([&] { OwnedPtxCache c("", compiler, sdk); }, "missing root accepted");
        rejects([&] { OwnedPtxCache c("relative", compiler, sdk); }, "relative root accepted");
        rejects([&] { OwnedPtxCache c(root+"/../escape", compiler, sdk); }, "traversal root accepted");
        rejects([&] { OwnedPtxCache c(root+std::string(1, '\xff'), compiler, sdk); }, "invalid UTF8 root accepted");
        rejects([&] { OwnedPtxCache c(root, "../foreign", sdk); }, "invalid identity accepted");
        OwnedPtxCache a(root, compiler, sdk), otherCompiler(root, std::string(64,'c'), sdk);
        OwnedPtxCache otherSdk(root, compiler, std::string(64,'d'));
        const auto key = OwnedPtxCache::Key({"ab", "c"}, "source", "program", "89");
        require(key != OwnedPtxCache::Key({"a", "bc"}, "source", "program", "89"), "option framing collision");
        require(key != OwnedPtxCache::Key({"ab", "c"}, "source2", "program", "89"), "source collision");
        require(key != OwnedPtxCache::Key({"ab", "c"}, "source", "program", "90"), "architecture collision");
        std::string out;
        require(!a.Load(key, out), "cold cache hit");
        const char fixture[]="// CUDA 12.8\n.version 8.7\n";
        const std::string ptx(fixture, sizeof fixture);
        a.Store(key, ptx.data(), ptx.size());
        require(a.Load(key, out) && out == ptx, "warm record did not round trip");
        require(!otherCompiler.Load(key,out) && !otherSdk.Load(key,out), "identity crossed namespaces");
        // Copy a valid record across namespaces: the header must still reject it.
        std::filesystem::copy_file(a.Path(key), otherCompiler.Path(key));
        require(!otherCompiler.Load(key,out), "foreign compiler header accepted");
        { std::ofstream f(a.Path(key), std::ios::binary); f << "\x12\x34\x56\x78// CUDA 13.1 foreign PTX"; }
        require(!a.Load(key,out), "legacy 13.1 fixture accepted");
        if(argc == 3){
            std::filesystem::copy_file(argv[2],a.Path(key),std::filesystem::copy_options::overwrite_existing);
            require(!a.Load(key,out), "actual foreign PTX accepted");
            std::cout << "actual_foreign_ptx_rejected=" << argv[2] << '\n';
        }
        a.Store(key,ptx.data(),ptx.size());
        { std::fstream f(a.Path(key),std::ios::binary|std::ios::in|std::ios::out); f.seekp(-2,std::ios::end); f.put('X'); }
        require(!a.Load(key,out), "corrupt PTX accepted");
        a.Store(key,ptx.data(),ptx.size());
        std::filesystem::resize_file(a.Path(key), OwnedPtxCache::MaxBytes + 4096);
        require(!a.Load(key,out), "oversized record accepted");
        rejects([&] { a.Store(key,ptx.data(),OwnedPtxCache::MaxBytes+1); }, "oversized write accepted");
        // An owned directory at the destination forces publication to fail explicitly.
        const auto blocked = OwnedPtxCache::Key({},"blocked","program","89");
        std::filesystem::create_directory(a.Path(blocked));
        rejects([&] { a.Store(blocked,ptx.data(),ptx.size()); }, "write failure was hidden");
        std::exception_ptr threadFailure;
        const auto parallel = OwnedPtxCache::Key({},"parallel","program","89");
        std::jthread t([&] { try { OwnedPtxCache b(root,compiler,sdk); for(int i=0;i<30;++i)b.Store(parallel,ptx.data(),ptx.size()); } catch(...) { threadFailure=std::current_exception(); } });
        for(int i=0;i<30;++i) { a.Store(parallel,ptx.data(),ptx.size()); require(a.Load(parallel,out)&&out==ptx,"partial parallel record"); }
        t.join(); if(threadFailure)std::rethrow_exception(threadFailure);
        std::cout << "OWNED_CACHE_PASS: cold/warm/identity/header/foreign/corrupt/bounds/write-failure/parallel\n";
        return 0;
    } catch(const std::exception &e) { std::cerr << "FAIL: " << e.what() << '\n'; return 1; }
}
