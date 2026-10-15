// Strict description validation, independent of a GPU or SDK.
#include "backend.hpp"
#include "test_inputs.hpp"
#include <iostream>
#include <limits>
namespace tn {
uint32_t backend_id() noexcept { return TN_BACKEND_FIXTURE; }
uint32_t backend_flags() noexcept { return TN_FLAG_TEST_FIXTURE; }
std::unique_ptr<Backend> make_backend(const Config&) { return {}; }
}
int main() {
    try {
        const auto bytes=config_document().dump();
        const tn_config raw{sizeof(tn_config),1,reinterpret_cast<const uint8_t*>(bytes.data()),bytes.size(),1048576,1048576,8000,0};
        const auto config=tn::copy_config(&raw);
        auto info=tn::Json::parse(R"({"schema_version":1,"backend":"fixture","sdk_version":"test","device_id":"cpu-fixture","device_name":"CPU","api":"fixture","driver":"none"})");
        const auto validate=[&](const tn::Json& j){tn::validate_description(j.dump(),config);};
        validate(info); // old bridge identity is still valid
        info["diagnostics"]={{"schema_version",1},{"counters",tn::Json::object()},{"timings_ms",tn::Json::object()},{"settings",tn::Json::object()},{"artifacts",tn::Json::object()}};
        validate(info);
        for(int i=0;i<64;++i) {
            const auto key="a"+std::to_string(i)+std::string(127-std::to_string(i).size(),'x');
            info["diagnostics"]["counters"][key]=UINT64_MAX;
            info["diagnostics"]["timings_ms"][key]=std::numeric_limits<double>::max();
            info["diagnostics"]["settings"][key]=std::string(1024,'x');
            info["diagnostics"]["artifacts"][key]={{"path",R"(C:\module.dll)"},{"sha256",std::string(64,'A')}};
        }
        validate(info);
        const auto rejects=[&](tn::Json j){
            try {validate(j);} catch(const tn::InvalidInput&) {return;} catch(const tn::Json::exception&) {return;}
            throw std::runtime_error("malformed diagnostics accepted: "+j.dump().substr(0,256));
        };
        auto empty=info; for(auto key:{"counters","timings_ms","settings","artifacts"}) empty["diagnostics"][key]=tn::Json::object();
        for(auto map:{"counters","timings_ms","settings","artifacts"}) {
            auto duplicate=empty.dump();const auto needle=std::string("\"")+map+"\":{}";
            const auto value=info["diagnostics"][map].begin().value().dump();
            duplicate.replace(duplicate.find(needle),needle.size(),std::string("\"")+map+"\":{\"a\":"+value+",\"a\":"+value+"}");
            bool rejected=false;try {tn::validate_description(duplicate,config);} catch(const tn::InvalidInput&) {rejected=true;}
            if(!rejected) throw std::runtime_error("duplicate diagnostics map key accepted");
        }
        const auto bad_value=[&](const char* map,const tn::Json& value){auto j=empty;j["diagnostics"][map]["test"]=value;rejects(j);};
        for(const auto& v:std::vector<tn::Json>{-1,1.0,true,"1",nullptr,18446744073709551616.0}) bad_value("counters",v);
        for(const auto& v:std::vector<tn::Json>{-1,"1",true,nullptr}) bad_value("timings_ms",v);
        for(const auto& v:std::vector<tn::Json>{"",std::string(1025,'x'),"bad\ntext",tn::Json::parse(R"("bad\u0085")"),1,true,nullptr}) bad_value("settings",v);
        for(auto map:{"counters","timings_ms","settings","artifacts"}) {
            auto j=info; j["diagnostics"][map]["extra"]=j["diagnostics"][map].begin().value(); rejects(j);
            for(const auto& key:std::vector<std::string>{"","A","1a","a b","a/","a\n",std::string(129,'a'),"é"}) {
                j=empty;j["diagnostics"][map][key]=info["diagnostics"][map].begin().value();rejects(j);
            }
            j=empty;j["diagnostics"].erase(map);rejects(j);
            j=empty;j["diagnostics"][map]=tn::Json::array();rejects(j);
        }
        for(const auto& v:std::vector<tn::Json>{0,2,1.0,true,nullptr}) {auto j=empty;j["diagnostics"]["schema_version"]=v;rejects(j);}
        auto j=empty;j["diagnostics"]["unknown"]=1;rejects(j);
        j=empty;j["unknown"]=1;rejects(j);
        j=empty;j["diagnostics"]=nullptr;rejects(j);
        for(const auto& path:std::vector<std::string>{"","module.dll",R"(C:module.dll)",R"(\module.dll)",R"(\\?\C:/module.dll)",R"(\\.\pipe\x)",R"(C:\..\x)",R"(C:\bad|x)",std::string("C:\\bad\0x",9),"C:\\"+std::string(4094,'x')})
            bad_value("artifacts",{{"path",path},{"sha256",std::string(64,'a')}});
        for(const auto& hash:{std::string(63,'a'),std::string(65,'a'),std::string(64,'g')}) bad_value("artifacts",{{"path",R"(C:\module.dll)"},{"sha256",hash}});
        bad_value("artifacts",{{"path",R"(C:\module.dll)"},{"sha256",std::string(64,'a')},{"unknown",1}});
        bad_value("artifacts",{{"path",R"(C:\module.dll)"}});
        for(const auto& path:std::vector<std::string>{R"(C:/module.dll)",R"(\\server\share\module.dll)",R"(\\?\C:\module.dll)",R"(\\?\UNC\server\share\module.dll)","C:\\"+std::string(4093,'x'),tn::Json::parse(R"("C:\\legal-\u007f\u0085.dll")").get<std::string>()}) {
            j=empty;j["diagnostics"]["artifacts"]["module"]={{"path",path},{"sha256",std::string(64,'a')}};validate(j);
        }
        std::cout<<"diagnostics old/empty/max schema and malformed bounds/types/paths passed\n";
        return 0;
    } catch(const std::exception& e) {std::cerr<<e.what()<<"\n";return 1;}
}
