#include "test_inputs.hpp"
static tn::Json canonical_scene_document(){auto j=scene_document();j["appearance"]["studio_preset"]="light";return j;}
#include <Windows.h>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <thread>
#include <array>
#include <functional>
#include <mutex>
#include "../../assets/render/color/turninator_agx.h"
using Clock=std::chrono::steady_clock;
static void require(bool b,const char* s) { if(!b) throw std::runtime_error(s); }
#include "resource_probe.hpp"
struct Api {
    HMODULE module{};std::wstring module_path;
#define TN_PROC(name) decltype(&tn_renderer_##name) name{};
    TN_PROC(abi_version) TN_PROC(create) TN_PROC(describe) TN_PROC(upload_scene)
    TN_PROC(begin_frame) TN_PROC(poll) TN_PROC(read_rgba8) TN_PROC(cancel) TN_PROC(destroy)
#undef TN_PROC
    explicit Api(const wchar_t* path):module_path(path) {
        module=LoadLibraryExW(path,nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32);
        require(module!=nullptr,"production DLL load failed");
#define TN_PROC(name) name=reinterpret_cast<decltype(name)>(GetProcAddress(module,"tn_renderer_" #name)); require(name!=nullptr,"missing production export");
        TN_PROC(abi_version) TN_PROC(create) TN_PROC(describe) TN_PROC(upload_scene)
        TN_PROC(begin_frame) TN_PROC(poll) TN_PROC(read_rgba8) TN_PROC(cancel) TN_PROC(destroy)
#undef TN_PROC
    }
    ~Api() { if(module) FreeLibrary(module); }
    void close(){require(FreeLibrary(module)!=0,"DLL unload failed");module=nullptr;require(GetModuleHandleW(module_path.c_str())==nullptr,"production DLL remained loaded");}
};
struct Context {
    Api& api; tn_context ctx{}; std::array<uint8_t,4096> message{};
    std::filesystem::path output;
    unsigned frame_number=0,upload_number=0;bool wall=false,look_up=false,shadow_fixture=false;std::function<void()> resource_hook;
    tn_error error{sizeof(tn_error),1,message.data(),message.size()};
    explicit Context(Api& a):api(a) {}
    ~Context() { if(ctx) api.destroy(ctx); }
    void check(int32_t result) { if(result!=TN_OK) throw std::runtime_error(reinterpret_cast<const char*>(message.data())); }
    void create(const std::string& cache,const std::string& device="") {
        auto j=config_document(); j["selection"]={{"backend","luxcore"},{"device",device.empty()?tn::Json(nullptr):tn::Json(device)},{"quality",{{"kind","luxcore"},{"samples",64},{"denoise",false}}}};
        j["identity"]["backend"]="luxcore"; j["cache_directory"]=cache;
        const uint64_t budget=uint64_t(16)*1024*1024*1024;
        j["limits"]["host_bytes"]=budget; j["limits"]["gpu_bytes"]=budget;
        auto s=j.dump(); tn_config c{sizeof(tn_config),1,reinterpret_cast<const uint8_t*>(s.data()),s.size(),budget,budget,8000,0};
        check(api.create(&c,&ctx,&error));
    }
    tn::Json describe() {
        uint64_t n=0; require(api.describe(ctx,nullptr,0,&n,&error)==TN_CAPACITY,"description query failed");
        std::vector<uint8_t> b(n); check(api.describe(ctx,b.data(),b.size(),&n,&error)); return tn::Json::parse(b);
    }
    void triangle(tn::Json metadata,bool empty=false,uint32_t material=0) {
        metadata["color"]["tone_map"]="turninator-agx-v1";
        metadata["reference_bounds_mm"]={{"min",{-7,-7,0}},{"max",{7,7,0}}};
        const auto s=metadata.dump(); const float p[]={-7,-7,0,7,-7,0,0,7,0},n[]={0,0,1,0,0,1,0,0,1}; const uint32_t t[]={0,1,2},m[]={material};
        tn_scene scene{sizeof(tn_scene),1,1,p,n,p,t,m,empty?0u:3u,empty?0u:1u,reinterpret_cast<const uint8_t*>(s.data()),s.size()};
        if(!output.empty())std::ofstream(output/("upload-"+std::to_string(++upload_number)+".json"))<<tn::Json{{"metadata",metadata},{"revision",1},{"positions",std::vector<float>(p,p+9)},{"normals",std::vector<float>(n,n+9)},{"triangles",std::vector<uint32_t>(t,t+3)},{"material",material},{"empty",empty}}.dump(2);
        check(api.upload_scene(ctx,&scene,&error));
    }
    void layer_wall(bool enabled) {
        wall=true;
        auto metadata=canonical_scene_document();metadata["color"]["tone_map"]="turninator-agx-v1";metadata["appearance"]["layer_appearance"]=enabled;metadata["appearance"]["first_layer_height_mm"]=0.6;metadata["appearance"]["layer_height_mm"]=0.3;metadata["appearance"]["groove_depth_mm"]=0.06;
        metadata["layer_filter"]["enabled"]=true;metadata["reference_bounds_mm"]={{"min",{-2,0,4}},{"max",{2,0,8}}};metadata["materials"][0]["base_color_linear"]={0.5,0.5,0.5};
        const auto s=metadata.dump();const float p[]={-2,0,4,2,0,4,2,0,8,-2,0,8},n[]={0,-1,0,0,-1,0,0,-1,0,0,-1,0};const uint32_t t[]={0,1,2,0,2,3},m[]={0,0};
        tn_scene scene{sizeof(tn_scene),1,1,p,n,p,t,m,4,2,reinterpret_cast<const uint8_t*>(s.data()),s.size()};check(api.upload_scene(ctx,&scene,&error));
    }
    void grid(unsigned side){
        auto metadata=canonical_scene_document();metadata["color"]["tone_map"]="turninator-agx-v1";
        std::vector<float> p,n;std::vector<uint32_t> indices,materials;
        for(unsigned y=0;y<=side;++y)for(unsigned x=0;x<=side;++x){p.insert(p.end(),{-7.f+14.f*x/side,-7.f+14.f*y/side,0});n.insert(n.end(),{0,0,1});}
        for(unsigned y=0;y<side;++y)for(unsigned x=0;x<side;++x){uint32_t a=y*(side+1)+x,b=a+1,c=a+side+1,d=c+1;indices.insert(indices.end(),{a,b,d,a,d,c});materials.insert(materials.end(),{0,0});}
        auto text=metadata.dump();tn_scene scene{sizeof(tn_scene),1,1,p.data(),n.data(),p.data(),indices.data(),materials.data(),static_cast<uint32_t>(p.size()/3),static_cast<uint32_t>(materials.size()),reinterpret_cast<const uint8_t*>(text.data()),text.size()};check(api.upload_scene(ctx,&scene,&error));
        std::ofstream(output/"grid-input.json")<<tn::Json{{"side",side},{"vertices",p.size()/3},{"triangles",materials.size()},{"metadata",metadata},{"formula","x=-7+14*column/side,y=-7+14*row/side,z=0; +Z normals; 2 CCW triangles/cell"}}.dump(2);
    }
    void slab(float thickness,float scattering=0,float ior=1,float roughness=.0001f) {
        auto metadata=canonical_scene_document();metadata["color"]["tone_map"]="turninator-agx-v1";
        metadata["appearance"]["studio_preset"]="diagram";
        metadata["reference_bounds_mm"]={{"min",{-7,-7,-thickness}},{"max",{7,7,0}}};
        auto& m=metadata["materials"][0];m["transmission"]=1;m["ior"]=ior;m["roughness"]=roughness;m["absorption_color_linear"]={0,0.5,1};m["absorption_density_per_mm"]=0.1;m["scattering_per_mm"]=scattering;
        const float vertices[]={-7,-7,0,7,-7,0,7,7,0,-7,7,0,-7,-7,-thickness,7,-7,-thickness,7,7,-thickness,-7,7,-thickness};
        const uint32_t faces[]={0,1,2,0,2,3,4,7,6,4,6,5,0,4,5,0,5,1,1,5,6,1,6,2,2,6,7,2,7,3,3,7,4,3,4,0};
        std::vector<float> p,n;std::vector<uint32_t> indices,materials;
        for(size_t t=0;t<12;++t){float edge1[3],edge2[3],normal[3];for(size_t k=0;k<3;++k){edge1[k]=vertices[faces[t*3+1]*3+k]-vertices[faces[t*3]*3+k];edge2[k]=vertices[faces[t*3+2]*3+k]-vertices[faces[t*3]*3+k];}
            for(size_t k=0;k<3;++k)normal[k]=edge1[(k+1)%3]*edge2[(k+2)%3]-edge1[(k+2)%3]*edge2[(k+1)%3];float length=std::sqrt(normal[0]*normal[0]+normal[1]*normal[1]+normal[2]*normal[2]);
            for(size_t v=0;v<3;++v){indices.push_back(static_cast<uint32_t>(indices.size()));for(size_t k=0;k<3;++k){p.push_back(vertices[faces[t*3+v]*3+k]);n.push_back(normal[k]/length);}}materials.push_back(0);
        }
        const auto s=metadata.dump();tn_scene scene{sizeof(tn_scene),1,1,p.data(),n.data(),p.data(),indices.data(),materials.data(),static_cast<uint32_t>(p.size()/3),static_cast<uint32_t>(materials.size()),reinterpret_cast<const uint8_t*>(s.data()),s.size()};
        std::ofstream(output/"slab-input.json")<<tn::Json{{"metadata",metadata},{"positions",p},{"normals",n},{"triangles",indices},{"triangle_materials",materials}}.dump(2);check(api.upload_scene(ctx,&scene,&error));
    }
    std::vector<uint8_t> render(unsigned width=65,unsigned height=49,bool denoise=false,double offset=0,unsigned samples=64,bool transparent=true) {
        const std::string q=tn::Json{{"schema_version",1},{"quality",{{"kind","luxcore"},{"samples",samples},{"denoise",denoise}}}}.dump();
        auto f=frame_input(1,q);f.width=width;f.height=height; f.camera_position_mm[1]=0;f.camera_position_mm[2]=30;f.camera_up[1]=1;f.camera_up[2]=0;f.vertical=40;
        f.model_to_world[3]=offset;f.transparent=transparent?1u:0u;
        if(look_up)f.camera_target_mm[2]=60;
        if(shadow_fixture){f.camera_position_mm[1]=-30;f.camera_position_mm[2]=20;f.camera_up[1]=0;f.camera_up[2]=1;f.model_to_world[11]=3;}
        if(wall){f.camera_position_mm[1]=-20;f.camera_position_mm[2]=6;f.camera_target_mm[2]=6;f.camera_up[1]=0;f.camera_up[2]=1;f.projection=1;f.vertical=5;f.model_to_world[3]=0;f.model_to_world[11]=offset;}
        check(api.begin_frame(ctx,&f,&error)); const auto start=Clock::now();tn_poll p{sizeof(tn_poll),1};uint64_t previous_work=0;tn::Json progress=tn::Json::array();
        do {check(api.poll(ctx,&p,&error));require(p.completed_work>=previous_work&&(!p.total_work||p.completed_work<=p.total_work),"progress is not monotonic/bounded");if(p.completed_work!=previous_work||progress.empty())progress.push_back({{"state",p.state},{"completed",p.completed_work},{"total",p.total_work}});previous_work=p.completed_work;if(resource_hook)resource_hook(); require(Clock::now()-start<std::chrono::minutes(10),"render timeout");std::this_thread::sleep_for(std::chrono::milliseconds(10));} while(p.state!=TN_READY);
        tn_image image{sizeof(tn_image),1};std::vector<uint8_t> pixels(size_t(width)*height*4,0x7b);
        require(api.read_rgba8(ctx,pixels.data(),pixels.size()-1,&image,&error)==TN_CAPACITY,"short capacity accepted");
        require(std::all_of(pixels.begin(),pixels.end(),[](uint8_t b){return b==0x7b;}),"short read wrote pixels");
        check(api.read_rgba8(ctx,pixels.data(),pixels.size(),&image,&error));
        require(image.required_bytes==pixels.size()&&image.width==width&&image.height==height,"wrong image dimensions/stride");
        for(size_t i=0;i<pixels.size();i+=4) if(!pixels[i+3]) require(!pixels[i]&&!pixels[i+1]&&!pixels[i+2],"zero alpha RGB not zero");
        const auto description=describe();const auto stats=description.at("diagnostics").at("counters");
        if(!output.empty()){
            const auto prefix="frame-"+std::to_string(++frame_number);std::ofstream raw(output/(prefix+".rgba"),std::ios::binary);raw.write(reinterpret_cast<const char*>(pixels.data()),static_cast<std::streamsize>(pixels.size()));
            std::ofstream(output/(prefix+".json"))<<tn::Json{{"width",width},{"height",height},{"offset",offset},{"transparent",transparent},{"seed",f.sample_seed},{"quality",tn::Json::parse(q)},{"description",description},{"progress",progress}}.dump(2);
        }
        require(stats.at("actual_min_samples").get<uint64_t>()>=samples,"actual minimum sample count below request");
        require(stats.at("actual_max_samples").get<uint64_t>()>=stats.at("actual_min_samples").get<uint64_t>(),"invalid sample range");
        require(stats.at("actual_total_samples").get<uint64_t>()>=uint64_t(width)*height*samples,"invalid total samples");
        return pixels;
    }
};
static int independent_cases(Api& api,const std::string& cache,const std::filesystem::path& root,bool volume) {
    tn::Json outcomes=tn::Json::array();unsigned failures=0;
    auto run=[&](const char* name,const std::function<void(Context&,tn::Json&)>& body){
        tn::Json result={{"case",name}};try{Context c(api);c.output=root/name;std::filesystem::create_directories(c.output);c.create(cache);body(c,result);result["passed"]=true;}
        catch(const std::exception& e){result["passed"]=false;result["error"]=e.what();++failures;}
        outcomes.push_back(result);std::ofstream(root/"cases.json")<<outcomes.dump(2);std::cout<<result.dump()<<std::endl;
    };
    constexpr size_t center=(49/2*65+65/2)*4;
    if(!volume){
        for(bool unlit:{true,false})run(unlit?"replacement_empty_unlit":"replacement_empty_lit",[&](Context& c,tn::Json& r){
            auto source=canonical_scene_document();source["materials"][0]["unlit"]=unlit;
            c.triangle(source);auto red=c.render();auto m=source;m["materials"][0]["base_color_linear"]={0,1,0};c.triangle(m);auto replaced=c.render();auto generation=c.describe()["diagnostics"]["counters"]["upload_generation"];
            c.triangle(m,true);auto empty=c.render();
            c.api.destroy(c.ctx);c.ctx=nullptr;c.create(cache);c.triangle(m);auto fresh=c.render();int error=0;for(size_t i=0;i<fresh.size();++i)error=std::max(error,std::abs(int(fresh[i])-int(replaced[i])));
            r["maximum_fresh_green_error"]=error;r["replacement_generation"]=generation;r["red_center"]=std::vector<uint8_t>(red.begin()+center,red.begin()+center+4);r["green_center"]=std::vector<uint8_t>(replaced.begin()+center,replaced.begin()+center+4);
            // Unlit output isolates cache invalidation and stays bit-exact.
            // Sampled disk lighting uses the existing A/B/A 2-code tolerance.
            require(generation==2&&red!=replaced&&error<=(unlit?0:2),"same-revision replacement differs from fresh green reference");require(std::all_of(empty.begin(),empty.end(),[](uint8_t x){return x==0;}),"empty stage retained geometry");
        });
        auto unlit=canonical_scene_document();unlit["materials"][0]["unlit"]=true;unlit["materials"][0]["base_color_linear"]={0.5,0,0};
        run("straight_alpha",[&](Context& c,tn::Json& r){c.triangle(unlit);auto b=c.render();size_t edges=0;int maximum=0;for(size_t i=0;i<b.size();i+=4)if(b[i+3]>10&&b[i+3]<245){++edges;for(size_t k=0;k<3;++k)maximum=std::max(maximum,std::abs(int(b[i+k])-int(b[center+k])));}r["partial_pixels"]=edges;r["maximum_edge_color_error"]=maximum;require(edges>10&&maximum<=3,"unlit edge is not straight alpha scene-linear color");});
        run("mapped259",[&](Context& c,tn::Json&){c.triangle(unlit);auto reference=c.render();auto many=unlit;while(many["materials"].size()<259)many["materials"].push_back(unlit["materials"][0]);c.triangle(many,false,258);auto b=c.render();require(std::abs(int(b[center])-int(reference[center]))<=2,"material258 differs from canonical unlit material");});
        run("oidn_alpha",[&](Context& c,tn::Json& r){c.triangle(unlit);auto a=c.render(),b=c.render(65,49,true);int alpha_error=0;for(size_t i=3;i<a.size();i+=4)alpha_error=std::max(alpha_error,std::abs(int(a[i])-int(b[i])));r["maximum_alpha_error"]=alpha_error;require(alpha_error==0,"OIDN altered alpha");require(std::abs(int(a[center])-int(b[center]))<10,"OIDN altered unlit interior");});
    }else{
        auto average=[](const std::vector<uint8_t>& pixels){std::array<double,3> rgb{};for(size_t y=20;y<29;++y)for(size_t x=28;x<37;++x)for(size_t k=0;k<3;++k)rgb[k]+=pixels[(y*65+x)*4+k]/81.;return rgb;};
        for(float thickness:{1.f,10.f}){const auto name=thickness==1?"absorption_1mm":"absorption_10mm";
            run(name,[&,thickness](Context& c,tn::Json& r){c.slab(thickness);auto b=c.render(65,49,false,0,256,false);auto actual=average(b);turninator::agx_v1::Rgba expected{};require(turninator::agx_v1::map_srgb({.35f*std::exp(-.1f*thickness),.35f*std::exp(-.05f*thickness),.35f,1},0,expected),"analytic color mapping failed");double error=0;for(size_t k=0;k<3;++k)error=std::max(error,std::abs(actual[k]-expected[k]*255));r["actual_mean_rgb"]=actual;r["expected_rgb"]=std::array<double,3>{expected[0]*255,expected[1]*255,expected[2]*255};r["maximum_byte_error"]=error;require(error<=4,"physical absorption differs from Beer-Lambert thickness");});
        }
        run("scattering_10mm",[&](Context& c,tn::Json& r){c.slab(10);auto clear=average(c.render(65,49,false,0,256,false));c.slab(10,.2f);auto scatter=average(c.render(65,49,false,0,256,false));double difference=0;for(size_t k=0;k<3;++k)difference+=std::abs(clear[k]-scatter[k]);r["clear_mean_rgb"]=clear;r["scatter_mean_rgb"]=scatter;r["sum_channel_difference"]=difference;require(difference>5,"nonzero physical scattering has no measurable effect");});
    }
    std::cout<<"INDEPENDENT_CASE_FAILURES="<<failures<<std::endl;return failures?1:0;
}
static int extended_cases(Api& api,const std::string& cache,const std::filesystem::path& root,const std::wstring& mode) {
    tn::Json outcomes=tn::Json::array();unsigned failures=0;
    auto run=[&](const char* name,const std::function<void(Context&,tn::Json&)>& body){
        tn::Json result={{"case",name}};try{Context c(api);c.output=root/name;std::filesystem::create_directories(c.output);c.create(cache);body(c,result);result["passed"]=true;}
        catch(const std::exception& e){result["passed"]=false;result["error"]=e.what();++failures;}
        outcomes.push_back(result);std::ofstream(root/"cases.json")<<outcomes.dump(2);std::cout<<result.dump()<<std::endl;
    };
    if(mode==L"--appearance"){
        run("exposure",[&](Context& c,tn::Json& r){auto m=canonical_scene_document();m["color"]["exposure_ev"]=1;bool rejected=false;try{c.triangle(m);}catch(const std::exception& e){r["rejection"]=e.what();rejected=true;}require(rejected,"nonzero exposure accepted despite frozen contract");});
        run("unknown_studio",[&](Context& c,tn::Json& r){auto m=canonical_scene_document();m["appearance"]["studio_preset"]="softbox";bool rejected=false;try{c.triangle(m);c.render();}catch(const std::exception& e){r["rejection"]=e.what();rejected=true;}require(rejected,"unknown studio accepted");});
        run("studio",[&](Context& c,tn::Json& r){c.look_up=true;tn::Json values=tn::Json::array();int error=0;for(const auto& name:{"light","dark","diagram"}){auto m=canonical_scene_document();m["appearance"]["studio_preset"]=name;c.triangle(m);auto pixels=c.render(65,49,false,0,64,false);std::array<float,3> world=std::string(name)=="light"?std::array<float,3>{.22f*.35f,.25f*.35f,.3f*.35f}:std::string(name)=="dark"?std::array<float,3>{.025f*.12f,.03f*.12f,.04f*.12f}:std::array<float,3>{.35f,.35f,.35f};turninator::agx_v1::Rgba expected{};turninator::agx_v1::map_srgb({world[0],world[1],world[2],1},0,expected);for(size_t k=0;k<3;++k)error=std::max(error,std::abs(int(pixels[k])-int(std::lround(expected[k]*255))));values.push_back({{"studio",name},{"corner",std::vector<uint8_t>(pixels.begin(),pixels.begin()+4)}});}r["backgrounds"]=values;r["maximum_background_error"]=error;require(error<=1,"canonical studio world/background ignored");});
        run("floor_shadow",[&](Context& c,tn::Json& r){c.shadow_fixture=true;auto m=canonical_scene_document();c.triangle(m,true);auto floor=c.render(128,96,false,0,256,false);c.triangle(m);auto object=c.render(128,96,false,0,256,false);auto mask=c.render(128,96,false,0,256,true);size_t shadow_pixels=0;int max_shadow=0;for(size_t i=0;i<floor.size();i+=4)if(mask[i+3]==0){int delta=0;for(size_t k=0;k<3;++k)delta+=int(floor[i+k])-int(object[i+k]);max_shadow=std::max(max_shadow,delta);if(delta>=15)++shadow_pixels;}r["shadow_pixels_outside_source"]=shadow_pixels;r["maximum_rgb_sum_shadow"]=max_shadow;require(shadow_pixels>10,"physical floor shadow missing outside source silhouette");});
    }else if(mode==L"--motion-heldout"){
        run("motion_heldout",[&](Context& c,tn::Json&){c.layer_wall(true);c.render(128,128,false,.375*5/128);c.render(128,128,true,.375*5/128);});
    }else if(mode==L"--layers"){
        run("layers_motion_oidn",[&](Context& c,tn::Json& r){
            c.layer_wall(false);auto flat=c.render(128,128);c.layer_wall(true);auto raw=c.render(128,128);auto clean=c.render(128,128,true);
            for(double fraction:{.25,.5,.75}){c.render(128,128,false,fraction*5/128);c.render(128,128,true,fraction*5/128);}
            auto returned=c.render(128,128);double difference=0;int repeat=0;for(size_t y=20;y<108;++y)for(size_t x=20;x<108;++x){const size_t i=(y*128+x)*4;difference+=std::abs(int(raw[i])-int(flat[i]))/double(88*88);for(size_t k=0;k<4;++k)repeat=std::max(repeat,std::abs(int(raw[i+k])-int(returned[i+k])));}
            r["layer_mean_absolute_difference"]=difference;r["return_maximum_error"]=repeat;require(difference>1&&repeat<=2,"CABI layer graph absent or motion history retained");
        });
    }else if(mode==L"--lifecycle"){
        auto begin=[&](Context& c){const std::string q=R"({"schema_version":1,"quality":{"kind":"luxcore","samples":256,"denoise":false}})";auto f=frame_input(1,q);f.width=f.height=2048;f.camera_position_mm[1]=0;f.camera_position_mm[2]=30;f.camera_up[1]=1;f.camera_up[2]=0;c.check(api.begin_frame(c.ctx,&f,&c.error));};
        run("progress_cancel_restart",[&](Context& c,tn::Json& r){
            c.triangle(canonical_scene_document());auto before=c.render();begin(c);tn_poll poll{sizeof(tn_poll),1};auto start=Clock::now();bool progress=false;uint64_t greatest=0;
            do{c.check(api.poll(c.ctx,&poll,&c.error));greatest=std::max(greatest,poll.completed_work);if(poll.completed_work>0&&poll.state!=TN_READY){progress=true;break;}std::this_thread::sleep_for(std::chrono::milliseconds(10));}while(poll.state!=TN_READY&&Clock::now()-start<std::chrono::seconds(5));
            r["progress_observed"]=progress;r["largest_completed_work"]=greatest;r["state_before_cancel"]=poll.state;start=Clock::now();c.check(api.cancel(c.ctx,&c.error));
            do{c.check(api.poll(c.ctx,&poll,&c.error));require(Clock::now()-start<std::chrono::seconds(30),"cancel drain timeout");std::this_thread::sleep_for(std::chrono::milliseconds(10));}while(poll.state!=TN_CANCELLED);
            r["cancel_drain_ms"]=std::chrono::duration<double,std::milli>(Clock::now()-start).count();tn_image image{sizeof(tn_image),1};uint8_t sentinel=0x7b;require(api.read_rgba8(c.ctx,&sentinel,1,&image,&c.error)==TN_INVALID_STATE&&sentinel==0x7b,"cancelled context exposed pixels");
            auto after=c.render();int error=0;for(size_t i=0;i<before.size();++i)error=std::max(error,std::abs(int(before[i])-int(after[i])));r["restart_maximum_error"]=error;
            require(error<=2,"cancel restart retained incomplete film");require(progress,"no progressive samples visible before completion");
        });
        run("pending_destroy",[&](Context& c,tn::Json& r){c.triangle(canonical_scene_document());begin(c);auto start=Clock::now();api.destroy(c.ctx);c.ctx=nullptr;r["destroy_drain_ms"]=std::chrono::duration<double,std::milli>(Clock::now()-start).count();require(r["destroy_drain_ms"].get<double>()<30000,"pending destroy failed to drain");});
        run("missing_device",[&](Context& c,tn::Json&){api.destroy(c.ctx);c.ctx=nullptr;bool rejected=false;try{c.create(cache,"cuda:99999");}catch(const std::exception&){rejected=true;}require(rejected&&!c.ctx,"missing device silently fell back");});
    }else if(mode==L"--resin"){
        run("ior146_resin",[&](Context& c,tn::Json& r){
            c.slab(1,0,1.46f,.025f);auto thin=c.render(65,49,false,0,256,false);c.slab(10,0,1.46f,.025f);auto thick=c.render(65,49,false,0,256,false);auto transparent=c.render(65,49,false,0,256,true);auto denoised=c.render(65,49,true,0,256,true);
            c.slab(10,.2f,1.46f,.025f);auto scattering=c.render(65,49,false,0,256,false);
            constexpr size_t center=(24*65+32)*4;r["thin_center"]=std::vector<uint8_t>(thin.begin()+center,thin.begin()+center+4);r["thick_center"]=std::vector<uint8_t>(thick.begin()+center,thick.begin()+center+4);r["transparent_center"]=std::vector<uint8_t>(transparent.begin()+center,transparent.begin()+center+4);r["scatter_center"]=std::vector<uint8_t>(scattering.begin()+center,scattering.begin()+center+4);
            require(thick[center]>20&&thin[center]>thick[center]+10,"IOR1.46 physical absorption absent");require(scattering!=thick,"IOR1.46 scattering absent");
            for(size_t i=3;i<transparent.size();i+=4)require(transparent[i]==denoised[i],"resin OIDN changed alpha");
        });
    }
    std::cout<<"EXTENDED_CASE_FAILURES="<<failures<<std::endl;return failures?1:0;
}
static int resources(Api& api,const std::string& cache,const std::filesystem::path& root){
    ResourceProbe probe;tn::Json report;report["before_context"]=probe.sample();uint64_t peak_host=0,peak_gpu=0;
    {
        Context c(api);c.output=root;c.create(cache);auto measure=[&]{auto m=probe.sample();peak_host=std::max(peak_host,m["private_bytes"].get<uint64_t>());peak_gpu=std::max(peak_gpu,m["gpu_local_bytes"].get<uint64_t>()+m["gpu_nonlocal_bytes"].get<uint64_t>());};c.resource_hook=measure;
        c.triangle(canonical_scene_document());c.render();report["warm_small"]=probe.sample();
        c.grid(512);c.render(4096,4096,true,0,64);report["large"]=probe.sample();report["large_description"]=c.describe();
        c.triangle(canonical_scene_document());c.render();report["returned_small"]=probe.sample();report["small_description"]=c.describe();
        report["measured_peak_private_bytes"]=peak_host;report["measured_peak_gpu_bytes"]=peak_gpu;
        std::ofstream(root/"resources.json")<<report.dump(2);
    }
    report["after_context_destroy"]=probe.sample();api.close();report["after_dll_unload"]=probe.sample();report["production_dll_unloaded"]=true;
    report["luxcore_module_unloaded"]=GetModuleHandleW(L"luxcore.dll")==nullptr;
    std::ofstream(root/"resources.json")<<report.dump(2);std::cout<<report.dump()<<std::endl;
    const uint64_t limit=uint64_t(16)<<30;
    require(peak_host<=limit&&peak_gpu<=limit,"actual process/GPU peak exceeds declared16GiB budget");
    require(report["large_description"]["diagnostics"]["counters"]["actual_min_samples"].get<uint64_t>()>=64,"4096 actual sample minimum below request");
    require(report["large"]["private_bytes"].get<uint64_t>()>report["returned_small"]["private_bytes"].get<uint64_t>()+(512ull<<20),"large-to-small host allocation retained");
    require(report["large_description"]["diagnostics"]["counters"]["observed_sdk_device_bytes"].get<uint64_t>()>report["small_description"]["diagnostics"]["counters"]["observed_sdk_device_bytes"].get<uint64_t>()+(16ull<<20),"large-to-small SDK geometry/BVH allocation retained");
    require(report["luxcore_module_unloaded"].get<bool>(),"SDK module still loaded after all contexts/DLL unload");
    std::cout<<"RESOURCE_4096_RELEASE_UNLOAD_PASS\n";return 0;
}
static tn::Json heap_accounting(){
    const auto n=GetProcessHeaps(0,nullptr);std::vector<HANDLE> heaps(n);require(GetProcessHeaps(n,heaps.data())==n,"heap list changed");
    uint64_t busy=0,free=0,blocks=0;
    for(const auto heap:heaps){if(!HeapLock(heap))continue;PROCESS_HEAP_ENTRY e{};while(HeapWalk(heap,&e)){if(e.wFlags&PROCESS_HEAP_ENTRY_BUSY){busy+=e.cbData;++blocks;}else if(!(e.wFlags&(PROCESS_HEAP_REGION|PROCESS_HEAP_UNCOMMITTED_RANGE)))free+=e.cbData;}HeapUnlock(heap);}
    return {{"busy_bytes",busy},{"free_bytes",free},{"busy_blocks",blocks},{"heaps",n}};
}
static int cuda_driver_cycles(Api& api,const std::string& cache,const std::filesystem::path& root){
    {Context c(api);c.create(cache);}
    auto cuda=GetModuleHandleW(L"nvcuda.dll");require(cuda,"CUDA driver unavailable");
    using GetDevice=int(__stdcall*)(int*,int);using Create=int(__stdcall*)(void**,unsigned,int);using Destroy=int(__stdcall*)(void*);
    auto get=reinterpret_cast<GetDevice>(GetProcAddress(cuda,"cuDeviceGet"));auto create=reinterpret_cast<Create>(GetProcAddress(cuda,"cuCtxCreate_v2"));auto destroy=reinterpret_cast<Destroy>(GetProcAddress(cuda,"cuCtxDestroy_v2"));require(get&&create&&destroy,"CUDA context symbols unavailable");
    ResourceProbe probe;tn::Json report=tn::Json::array();
    for(unsigned i=0;i<20;++i){int result=-1;std::thread worker([&]{int device=0;void* context=nullptr;result=get(&device,0);if(!result)result=create(&context,2,device);if(!result)result=destroy(context);});worker.join();require(!result,"CUDA context operation failed");auto sample=probe.sample();report.push_back(sample);std::ofstream(root/"cuda-driver.json")<<report.dump(2);std::cout<<"CUDA_DRIVER_CYCLE="<<i<<" "<<sample.dump()<<std::endl;}return 0;
}
static int same_context_cycles(Api& api,const std::string& cache,const std::filesystem::path& root){
    ResourceProbe probe;tn::Json report=tn::Json::array();{Context c(api);c.create(cache);for(unsigned i=0;i<20;++i){c.output=root/("cycle-"+std::to_string(i));std::filesystem::create_directories(c.output);c.triangle(canonical_scene_document());c.render(128,128,false);c.render(128,128,true);auto sample=probe.sample();report.push_back(sample);std::ofstream(root/"same-context.json")<<report.dump(2);std::cout<<"SAME_CONTEXT_CYCLE="<<i<<" "<<sample.dump()<<std::endl;}}std::cout<<"FINAL_DESTROY="<<probe.sample().dump()<<std::endl;return 0;
}
static int context_cycles(Api& api,const std::string& cache,const std::filesystem::path& root,bool raw_only=false,bool retain_compiler=false,bool lease_owner=false){
    struct Leases {std::vector<HMODULE> modules;~Leases(){for(auto i=modules.rbegin();i!=modules.rend();++i)FreeLibrary(*i);}} leases;
    if(retain_compiler)for(const auto name:{L"nvrtc-builtins64_128.dll",L"nvrtc64_120_0.dll"}){auto path=std::filesystem::path(api.module_path).parent_path()/name;auto module=LoadLibraryExW(path.c_str(),nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32);require(module,"compiler lease failed");leases.modules.push_back(module);}
    std::unique_ptr<Context> standby;if(lease_owner){standby=std::make_unique<Context>(api);standby->create(cache);}
    ResourceProbe probe;tn::Json report={{"baseline",probe.sample()},{"cycles",tn::Json::array()}};
    for(unsigned cycle=0;cycle<20;++cycle){
        uint64_t owner_id=0;
        {Context c(api);c.output=root/("cycle-"+std::to_string(cycle));std::filesystem::create_directories(c.output);c.create(cache);owner_id=c.describe()["diagnostics"]["counters"]["sdk_owner_thread_id"].get<uint64_t>();c.triangle(canonical_scene_document());c.render(128,128,false);if(!raw_only)c.render(128,128,true);}
        auto sample=probe.sample();sample["sdk_owner_thread_id"]=owner_id;
        require(sample["gpu_local_bytes"]==0&&sample["gpu_nonlocal_bytes"]==0,"GPU residency after job context destruction");
        if(lease_owner&&cycle)require(owner_id==report["cycles"][0]["sdk_owner_thread_id"],"leased SDK owner changed");
        report["cycles"].push_back(sample);std::ofstream(root/"contexts.json")<<report.dump(2);std::cout<<"CONTEXT_CYCLE="<<cycle<<" "<<sample.dump()<<std::endl;
    }
    const auto& first=report["cycles"][0];const auto& last=report["cycles"].back();
    require(last["gpu_local_bytes"]==0&&last["gpu_nonlocal_bytes"]==0,"GPU residency after context teardown");
    require(last["private_bytes"].get<uint64_t>()<=first["private_bytes"].get<uint64_t>()+(64ull<<20),"context CPU baseline grows");
    require(last["handles"].get<unsigned>()<=first["handles"].get<unsigned>()+2,"context handle baseline grows");
    require(last["threads"].get<unsigned>()<=first["threads"].get<unsigned>()+2,"context thread baseline grows");
    report["before_module_release_attribution"]=probe.attribution();standby.reset();api.close();report["after_module_release"]=probe.sample();report["after_module_release_attribution"]=probe.attribution();std::ofstream(root/"contexts.json")<<report.dump(2);
    require(!GetModuleHandleW(L"luxcore.dll")&&!GetModuleHandleW(L"nvrtc64_120_0.dll"),"SDK/compiler remained loaded after lease release");
    require(report["after_module_release"]["threads"].get<unsigned>()<=report["baseline"]["threads"].get<unsigned>()+2,"SDK workers outlived lease release");
    std::cout<<"RESIDENT_MODULE_CONTEXT_CYCLES_PASS\n";return 0;
}
static int lifetime_cycles(Api& api,const std::string& cache,const std::filesystem::path& root,unsigned count=3,bool hold_sdk=false,bool hold_opencl=false,bool initialize_only=false,bool load_only=false){
    HMODULE retained_sdk{};
    if(hold_opencl){Context warm(api);warm.create(cache);}
    if(hold_sdk||hold_opencl){
        require(GetModuleHandleExW(0,hold_sdk?L"luxcore.dll":L"OpenCL.dll",&retained_sdk)!=0,"diagnostic runtime retain failed");
        const auto path=tn::lux::module_path(retained_sdk);
        if(hold_sdk){
            require(std::filesystem::equivalent(path,std::filesystem::path(api.module_path).parent_path()/L"luxcore.dll")&&tn::lux::hash_file(path)==tn::lux::sdk_hash,"diagnostic retained SDK identity differs");
        }else{
            std::array<wchar_t,32768> system{};
            const auto length=GetSystemDirectoryW(system.data(),static_cast<UINT>(system.size()));
            require(length&&length<system.size()&&std::filesystem::equivalent(path,std::filesystem::path(system.data())/L"OpenCL.dll"),"diagnostic OpenCL dispatcher is outside System32");
        }
    }
    const auto dll=api.module_path;ResourceProbe probe;api.close();tn::Json report={{"baseline",probe.sample()},{"baseline_attribution",probe.attribution()},{"cycles",tn::Json::array()}};
    for(unsigned cycle=0;cycle<count;++cycle){
        tn::Json record;{Api loaded(dll.c_str());{Context c(loaded);c.output=root/("cycle-"+std::to_string(cycle));std::filesystem::create_directories(c.output);if(!load_only)c.create(cache);record["initialized_attribution"]=probe.attribution();if(!initialize_only){c.triangle(canonical_scene_document());c.render(128,128,false);record["raw_attribution"]=probe.attribution();c.render(128,128,true);}record["ready"]=probe.sample();record["ready_attribution"]=probe.attribution();}record["destroyed"]=probe.sample();loaded.close();}
        record["heap"]=heap_accounting();record["unloaded"]=probe.sample();record["attribution"]=probe.attribution();report["cycles"].push_back(record);std::ofstream(root/"lifetime.json")<<report.dump(2);std::cout<<"UNLOAD_CYCLE="<<cycle<<" "<<record["unloaded"].dump()<<" HEAP="<<record["heap"].dump()<<std::endl;
    }
    if(retained_sdk){require(FreeLibrary(retained_sdk)!=0,"diagnostic retained SDK release failed");report["after_retained_sdk_release"]=probe.sample();report["after_retained_sdk_release_attribution"]=probe.attribution();std::ofstream(root/"lifetime.json")<<report.dump(2);}
    const auto& first=report["cycles"][0]["unloaded"];const auto& last=report["cycles"].back()["unloaded"];
    require(last["gpu_local_bytes"]==0&&last["gpu_nonlocal_bytes"]==0,"GPU residency after context teardown");
    require(last["private_bytes"].get<uint64_t>()<=first["private_bytes"].get<uint64_t>()+(64ull<<20),"repeated unload CPU baseline grows");
    require(last["threads"].get<unsigned>()<=first["threads"].get<unsigned>()+2,"repeated unload thread pool grows");
    for(const auto& t:report["cycles"].back()["attribution"]["threads"])require(t["module"]!="tn_renderer_luxcore.dll"&&t["module"]!="luxcore.dll"&&t["module"]!="unmapped","SDK/unmapped worker outlived module");
    std::cout<<"REPEATED_UNLOAD_ATTRIBUTED_PASS\n";return 0;
}
int wmain(int argc,wchar_t** argv) {
    try {
        require(argc==4||argc==5,"arguments: production DLL, owned cache directory, output directory, optional --pose-only/--readback/--volume");
        Api api(argv[1]);tn_abi_info abi{sizeof(tn_abi_info),1};require(api.abi_version(&abi,nullptr)==TN_OK&&abi.backend==TN_BACKEND_LUXCORE&&!abi.flags,"not production LuxCore ABI");
        const auto utf8=std::filesystem::path(argv[2]).u8string();const std::string cache(utf8.begin(),utf8.end());
        if(argc==5&&std::wstring(argv[4])==L"--lifetime-retained-sdk")return lifetime_cycles(api,cache,std::filesystem::path(argv[3]),5,true);
        if(argc==5&&std::wstring(argv[4])==L"--lifetime-retained-opencl")return lifetime_cycles(api,cache,std::filesystem::path(argv[3]),5,false,true);
        if(argc==5&&std::wstring(argv[4])==L"--lifetime-initialize-only")return lifetime_cycles(api,cache,std::filesystem::path(argv[3]),5,false,false,true);
        if(argc==5&&std::wstring(argv[4])==L"--lifetime-load-only")return lifetime_cycles(api,cache,std::filesystem::path(argv[3]),5,false,false,true,true);
        if(argc==5&&std::wstring(argv[4])==L"--context-cycles")return context_cycles(api,cache,std::filesystem::path(argv[3]));
        if(argc==5&&std::wstring(argv[4])==L"--context-raw-cycles")return context_cycles(api,cache,std::filesystem::path(argv[3]),true);
        if(argc==5&&std::wstring(argv[4])==L"--context-runtime-leases")return context_cycles(api,cache,std::filesystem::path(argv[3]),false,true);
        if(argc==5&&std::wstring(argv[4])==L"--leased-owner-cycles")return context_cycles(api,cache,std::filesystem::path(argv[3]),false,false,true);
        if(argc==5&&std::wstring(argv[4])==L"--cuda-driver-cycles")return cuda_driver_cycles(api,cache,std::filesystem::path(argv[3]));
        if(argc==5&&std::wstring(argv[4])==L"--same-context-cycles")return same_context_cycles(api,cache,std::filesystem::path(argv[3]));
        if(argc==5&&std::wstring(argv[4])==L"--lifetime-attribution")return lifetime_cycles(api,cache,std::filesystem::path(argv[3]),1);
        if(argc==5&&(std::wstring(argv[4])==L"--lifetime-cycles"||std::wstring(argv[4])==L"--lifetime-extended"))return lifetime_cycles(api,cache,std::filesystem::path(argv[3]),std::wstring(argv[4])==L"--lifetime-extended"?20:3);
        if(argc==5&&std::wstring(argv[4])==L"--resources")return resources(api,cache,std::filesystem::path(argv[3]));
        if(argc==5&&(std::wstring(argv[4])==L"--readback"||std::wstring(argv[4])==L"--volume"))return independent_cases(api,cache,std::filesystem::path(argv[3]),std::wstring(argv[4])==L"--volume");
        if(argc==5&&(std::wstring(argv[4])==L"--appearance"||std::wstring(argv[4])==L"--motion-heldout"||std::wstring(argv[4])==L"--layers"||std::wstring(argv[4])==L"--lifecycle"||std::wstring(argv[4])==L"--resin"))return extended_cases(api,cache,std::filesystem::path(argv[3]),std::wstring(argv[4]));
        Context c(api);c.output=std::filesystem::path(argv[3]);c.create(cache);std::cout<<c.describe().dump()<<std::endl;
        c.triangle(canonical_scene_document());auto red=c.render();const size_t center=(49/2*65+65/2)*4;
        // AgX desaturates bright physical highlights; encoded RGB channel
        // ratios are not linear-light reflectance ratios.
        require(red[center]>80&&int(red[center])-red[center+1]>20&&red[center+3]>250,"opaque red physical render absent");
        require(red[3]==0,"transparent corner absent");
        const auto moved=c.render(65,49,false,8);const auto returned=c.render();
        size_t changed_pose=0;int repeat_error=0;for(size_t i=0;i<red.size();++i){changed_pose+=red[i]!=moved[i];repeat_error=std::max(repeat_error,std::abs(int(red[i])-int(returned[i])));}
        size_t first_difference=0;while(first_difference<red.size()&&red[first_difference]==returned[first_difference])++first_difference;
        std::cout<<"pose_changed_bytes="<<changed_pose<<" return_max_error="<<repeat_error<<" first_difference_byte="<<first_difference<<std::endl;
        require(changed_pose>100,"pose edit did not change pixels");require(repeat_error<=2,"A/B/A retains previous pose accumulation");
        if(argc==5){std::cout<<"PRODUCTION_POSE_ONLY_PASS\n";return 0;}
        std::ofstream out(std::filesystem::path(argv[3])/"opaque.rgba",std::ios::binary);out.write(reinterpret_cast<char*>(red.data()),static_cast<std::streamsize>(red.size()));
        auto green=canonical_scene_document();green["materials"][0]["base_color_linear"]={0,1,0};c.triangle(green);auto changed=c.render();
        require(int(changed[center+1])-changed[center]>20,"same numeric revision upload retained stale materials");
        c.triangle(canonical_scene_document(),true);auto empty=c.render();require(std::all_of(empty.begin(),empty.end(),[](uint8_t b){return b==0;}),"empty stage retained geometry");
        auto unlit=canonical_scene_document();unlit["materials"][0]["unlit"]=true;unlit["materials"][0]["base_color_linear"]={0.5,0,0};c.triangle(unlit);auto emission=c.render();size_t edges=0;
        for(size_t i=0;i<emission.size();i+=4)if(emission[i+3]>10&&emission[i+3]<245){++edges;for(size_t k=0;k<3;++k)require(std::abs(int(emission[i+k])-int(emission[center+k]))<=3,"unlit edge is not straight alpha scene-linear color");}
        require(edges>10,"alpha edge fixture has no partial coverage");
        auto many=canonical_scene_document();while(many["materials"].size()<259)many["materials"].push_back(unlit["materials"][0]);c.triangle(many,false,258);auto mapped=c.render();require(std::abs(int(mapped[center])-int(emission[center]))<=2,"material index 258/unlit mapping differs");
        auto denoised=c.render(65,49,true);require(std::abs(int(denoised[center])-int(emission[center]))<10,"OIDN changed unlit interior color");
        Context missing(api);bool rejected=false;try{missing.create(cache,"cuda:99999");}catch(const std::exception&){rejected=true;}require(rejected&&!missing.ctx,"missing requested device accepted");
        std::cout<<c.describe().dump()<<"\nPRODUCTION_CONFORMANCE_PASS\n";return 0;
    } catch(const std::exception& e) {std::cerr<<"CONFORMANCE_FAIL: "<<e.what()<<'\n';return 1;}
}
