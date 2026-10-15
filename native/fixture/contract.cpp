// Raw caller-buffer checks complement the safe Rust host tests.
#include "turninator_renderer.h"
#include "test_inputs.hpp"
#include <algorithm>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>
#include <limits>
static void check(bool condition) { if(!condition) throw std::runtime_error("raw ABI contract failure"); }
int main() {
    tn_context context=nullptr;
    try {
        uint8_t errors[128]{};
        tn_error error{sizeof(tn_error),1,errors,sizeof(errors)};
        const uint8_t json[]="{}";
        tn_config config{0,1,json,2,1048576,1048576,8000,0};
        check(tn_renderer_create(&config,&context,&error)==TN_INVALID_ARGUMENT && !context);
        config.size=sizeof(config);
        check(tn_renderer_create(&config,&context,&error)==TN_INVALID_ARGUMENT && !context); // schema1 is mandatory
        std::cout<<"schema1 malformed config rejected\n";
        const auto valid_json=config_document().dump();
        config.json=reinterpret_cast<const uint8_t*>(valid_json.data()); config.json_bytes=valid_json.size();
        const auto invalid_config=[&](const std::string& bytes) {
            auto bad=config; bad.json=reinterpret_cast<const uint8_t*>(bytes.data()); bad.json_bytes=bytes.size();
            check(tn_renderer_create(&bad,&context,&error)==TN_INVALID_ARGUMENT && !context);
        };
        for(int i=0;i<11;++i) {
            auto bad=config_document();
            switch(i) {
                case 0: bad["schema_version"]=2; break;
                case 1: bad["schema_version"]=1.0; break;
                case 2: bad["identity"].erase("runtime_digest"); break;
                case 3: bad["limits"]["host_bytes"]=1; break;
                case 4: bad["selection"]["quality"]["aa"]="ultra"; break;
                case 5: bad["selection"]["quality"]["shadows"]=true; break;
                case 6: bad["limits"]["max_output_side"]=8001; break;
                case 7: bad["identity"]["backend"]="luxcore"; break;
                case 8: bad["unexpected"]=true; break;
                case 9: bad["identity"]["driver"]=nlohmann::json::parse(R"("driver\u007f")"); break;
                case 10: bad["identity"]["driver"]=nlohmann::json::parse(R"("driver\u0085")"); break;
            }
            invalid_config(bad.dump());
        }
        invalid_config("{\"schema_version\":1,\"schema_version\":1}");
        invalid_config(std::string("{\"bad\":\"")+char(-1)+"\"}");
        invalid_config(std::string(1048577,'x'));
        for(char control=0;control<32;++control) {
            auto bad=config_document(); bad["cache_directory"]=std::string(R"(C:\cache-)")+control;
            invalid_config(bad.dump());
        }
        for(const auto& path:std::vector<nlohmann::json>{nullptr,17,true,nlohmann::json::array(),nlohmann::json::object(),"","relative/cache",R"(C:cache)",R"(\cache)",R"(\\server)",R"(\\server\)",R"(\\.\pipe\cache)",R"(\\?\GLOBALROOT\cache)",R"(\\?\\\server\share)",R"(C:\app\..\cache)",R"(C:\bad|cache)",std::string("C:\\bad\0path",11),std::string("C:\\")+std::string(4094,'x')}) {
            auto bad=config_document(); bad["cache_directory"]=path;
            invalid_config(bad.dump());
        }
        for(const auto& path:std::vector<std::string>{R"(C:\)",R"(C:\app\cache)",R"(C:/app/cache)",R"(\\server\share\cache)",R"(//server/share/cache)",R"(\\?\C:\app\cache)",R"(\\?\UNC\server\share\cache)",nlohmann::json::parse(R"("C:\\cache\\\u6b6f\u8eca")").get<std::string>(),std::string("C:\\")+std::string(4093,'x')}) {
            auto document=config_document(); document["cache_directory"]=path;
            const auto bytes=document.dump();
            auto with_cache=config; with_cache.json=reinterpret_cast<const uint8_t*>(bytes.data()); with_cache.json_bytes=bytes.size();
            check(tn_renderer_create(&with_cache,&context,&error)==TN_OK && context);
            tn_renderer_destroy(context); context=nullptr;
        }
        for(const auto& suffix:{R"("cache-\u007f")",R"("cache-\u0080")",R"("cache-\u0085")",R"("cache-\u009f")"}) {
            auto document=config_document();
            document["cache_directory"]=R"(\\?\C:\)"+nlohmann::json::parse(suffix).get<std::string>();
            const auto bytes=document.dump();
            auto with_cache=config; with_cache.json=reinterpret_cast<const uint8_t*>(bytes.data()); with_cache.json_bytes=bytes.size();
            if(tn_renderer_create(&with_cache,&context,&error)!=TN_OK || !context) throw std::runtime_error("valid Windows DEL/C1 cache directory rejected");
            tn_renderer_destroy(context); context=nullptr;
        }
        std::cout<<"optional cache directory accepted; invalid type/path rejected\n";
        check(tn_renderer_create(&config,&context,&error)==TN_OK && context);
        uint8_t tiny[4]={73,73,73,73}; uint64_t required=0;
        check(tn_renderer_describe(context,tiny,sizeof(tiny),&required,&error)==TN_CAPACITY);
        check(required>sizeof(tiny) && required<1048576);
        check(std::all_of(std::begin(tiny),std::end(tiny),[](auto b){return b==73;}));
        std::vector<uint8_t> buffer(static_cast<size_t>(required)+8,91);
        const uint64_t capacity=required;
        check(tn_renderer_describe(context,buffer.data(),capacity,&required,&error)==TN_OK && required==capacity);
        check(std::all_of(buffer.begin()+static_cast<ptrdiff_t>(required),buffer.end(),[](auto b){return b==91;}));
        const std::string text(buffer.begin(),buffer.begin()+static_cast<ptrdiff_t>(required));
        check(text.find("\"device_id\":\"cpu-fixture\"")!=std::string::npos);
        tn_poll poll{sizeof(tn_poll),1,0,0,0,0};
        check(tn_renderer_poll(context,&poll,&error)==TN_OK && poll.state==TN_IDLE);
        auto scene_json=scene_document().dump();
        float positions[3]={0,0,0},normals[3]={0,0,1}; uint32_t triangles[3]={0,0,0},materials[1]={0};
        tn_scene scene{sizeof(tn_scene),1,7,positions,normals,positions,triangles,materials,1,1,reinterpret_cast<const uint8_t*>(scene_json.data()),scene_json.size()};
        for(int i=0;i<11;++i) {
            auto bad=scene_document();
            switch(i) {
                case 0: bad["materials"][0]["roughness"]=1.1; break;
                case 1: bad["appearance"]["layer_height_mm"]=0; break;
                case 2: bad["materials"][0]["ior"]=0; break;
                case 3: bad["materials"][0]["scattering_anisotropy"]=2; break;
                case 4: bad["reference_bounds_mm"]["min"][0]=2; break;
                case 5: bad["layer_filter"]["fade_start_pixels"]=3; break;
                case 6: bad["color"]["tone_map"]="turninator-agx-v1"; break;
                case 7: bad["color"]["output_alpha"]="premultiplied"; break;
                case 8: bad["appearance"]["layer_appearance"]=true; bad["geometry_kind"]="stepped"; break;
                case 9: bad["appearance"]["roughness"]="0.4"; break;
                case 10: bad["materials"][0]["scattering_per_mm"]=1e300; break;
            }
            const auto bytes=bad.dump(); auto input=scene; input.json=reinterpret_cast<const uint8_t*>(bytes.data()); input.json_bytes=bytes.size();
            check(tn_renderer_upload_scene(context,&input,&error)==TN_INVALID_ARGUMENT);
            check(tn_renderer_poll(context,&poll,&error)==TN_OK && poll.state==TN_IDLE);
        }
        auto bad_scene=scene; bad_scene.vertex_count=2000001;
        check(tn_renderer_upload_scene(context,&bad_scene,&error)==TN_INVALID_ARGUMENT);
        bad_scene=scene; bad_scene.triangle_count=4000001;
        check(tn_renderer_upload_scene(context,&bad_scene,&error)==TN_INVALID_ARGUMENT);
        positions[0]=std::numeric_limits<float>::infinity();
        check(tn_renderer_upload_scene(context,&scene,&error)==TN_INVALID_ARGUMENT); positions[0]=0;
        triangles[0]=1; check(tn_renderer_upload_scene(context,&scene,&error)==TN_INVALID_ARGUMENT); triangles[0]=0;
        materials[0]=1; check(tn_renderer_upload_scene(context,&scene,&error)==TN_INVALID_ARGUMENT); materials[0]=0;
        check(tn_renderer_upload_scene(context,&scene,&error)==TN_OK);
        const std::string q=R"({"schema_version":1,"quality":{"kind":"filament","aa":"high","shadows":"high"}})";
        for(int i=0;i<8;++i) {
            auto f=frame_input(7,q);
            switch(i) {
                case 0: f.model_to_world[15]=0; break;
                case 1: f.model_to_world[1]=std::numeric_limits<double>::quiet_NaN(); break;
                case 2: f.camera_up[2]=0; break;
                case 3: f.width=8001; break;
                case 4: f.near_mm=0; break;
                case 5: f.far_mm=f.near_mm; break;
                case 6: f.geometry_revision=8; break;
                case 7: f.transparent=2; break;
            }
            check(tn_renderer_begin_frame(context,&f,&error)==TN_INVALID_ARGUMENT);
            check(tn_renderer_poll(context,&poll,&error)==TN_OK && poll.state==TN_IDLE);
        }
        tn_renderer_destroy(context); context=nullptr;
        std::cout<<"raw ABI caller buffer/size/describe retry checks passed\n";
        return 0;
    } catch(const std::exception &e) {
        if(context) tn_renderer_destroy(context);
        std::cerr<<e.what()<<"\n"; return 1;
    }
}
