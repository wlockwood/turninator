// A deterministic CPU ABI exerciser. Never a renderer/SDK qualification result.
#include "backend.hpp"
#include <windows.h>
#include <bit>
#include <stdexcept>
#ifdef TN_DEPENDENT
extern "C" __declspec(dllimport) int tn_fixture_dependency_value();
#endif
namespace {
uint32_t matrix_fingerprint(const double* matrix) noexcept {
    uint32_t hash=2166136261u;
    for(size_t component=0;component<16;++component) {
        const auto bits=std::bit_cast<uint64_t>(matrix[component]);
        for(unsigned byte=0;byte<8;++byte) hash=(hash^static_cast<uint8_t>(bits>>(byte*8)))*16777619u;
    }
    return hash;
}
struct Fixture final : tn::Backend {
    std::string describe() override {
#ifdef TN_BAD_INFO
        return R"({"schema_version":2,"backend":"fixture","device_id":"cpu-fixture"})";
#elif defined(TN_BAD_INFO_UTF8)
        return "\xff";
#elif defined(TN_LARGE_INFO)
        return std::string(1048577,'x');
#else
        return R"({"schema_version":1,"backend":"fixture","sdk_version":"cpu-abi1","device_id":"cpu-fixture","device_name":"CPU ABI fixture","api":"fixture","driver":"none"})";
#endif
    }
    void upload(const tn::Scene&,tn::Work&) override {}
    std::vector<uint8_t> render(const tn::Frame& frame,tn::Work& work) override {
#ifdef TN_THROW
        throw std::runtime_error("fixture exception");
#endif
#ifdef TN_DELAYED
        HMODULE unexpected=LoadLibraryW(L"tn_fixture_dependency.dll");
        if(unexpected) { FreeLibrary(unexpected); throw std::runtime_error("ambient delayed dependency accepted"); }
        throw std::runtime_error("delayed dependency safely unavailable");
#endif
        const auto& f=frame.parameters;
        const double identity[16]={1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1};
        const auto fingerprint=matrix_fingerprint(f.model_to_world)^matrix_fingerprint(identity);
        const uint8_t pattern[16]={static_cast<uint8_t>(f.sample_seed),static_cast<uint8_t>(fingerprint),static_cast<uint8_t>(fingerprint>>8),255,
            static_cast<uint8_t>(fingerprint>>16),255,static_cast<uint8_t>(fingerprint>>24),static_cast<uint8_t>(f.transparent?128:255),
            0,0,255,255,0,0,0,static_cast<uint8_t>(f.transparent?0:255)};
        std::vector<uint8_t> bytes(size_t(f.width)*f.height*4);
        work.total=bytes.size();
        for(size_t i=0;i<bytes.size();++i) bytes[i]=pattern[i%16];
        work.completed=bytes.size(); return bytes;
    }
};
}
namespace tn {
uint32_t backend_id() noexcept { return TN_BACKEND_FIXTURE; }
uint32_t backend_flags() noexcept { return TN_FLAG_TEST_FIXTURE; }
std::unique_ptr<Backend> make_backend([[maybe_unused]] const Config& config) {
#ifdef TN_CACHE_REQUIRED
    if(!config.document.contains("cache_directory")) throw std::runtime_error("cache fixture requires explicit cache directory");
    const auto path=config.document.at("cache_directory").get<std::string>();
    if(!path.starts_with(R"(\\?\)")) throw std::runtime_error("cache fixture requires canonical Windows path");
    const auto suffix=Json::parse(R"("\\cache-directory-\u6b6f\u8eca")").get<std::string>();
    if(!path.ends_with(suffix)) throw std::runtime_error("cache fixture requires resolved Unicode directory");
#endif
#ifdef TN_DEPENDENT
    if(tn_fixture_dependency_value()!=17) throw std::runtime_error("bad dependency");
#endif
    return std::make_unique<Fixture>();
}
}
