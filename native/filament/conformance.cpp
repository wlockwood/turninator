#include "backend.hpp"
#include "../fixture/test_inputs.hpp"
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <thread>
#include <cmath>
#include <Windows.h>
#include <TlHelp32.h>
#include <Psapi.h>
#include <cstring>

// Exercise actual DLL load/unload ownership instead of retaining an implicit
// import-table reference for the lifetime of the conformance process.
struct Api {
    HMODULE module{};
    decltype(&tn_renderer_abi_version) abi_version{};
    decltype(&tn_renderer_create) create{};
    decltype(&tn_renderer_describe) describe{};
    decltype(&tn_renderer_upload_scene) upload{};
    decltype(&tn_renderer_begin_frame) begin{};
    decltype(&tn_renderer_poll) poll{};
    decltype(&tn_renderer_read_rgba8) read{};
    decltype(&tn_renderer_cancel) cancel{};
    decltype(&tn_renderer_destroy) destroy{};
    template<class T>void symbol(T& function,const char* name){auto address=GetProcAddress(module,name);if(!address)throw std::runtime_error(name);static_assert(sizeof(function)==sizeof(address));std::memcpy(&function,&address,sizeof(address));}
    void load(){
        if(module)throw std::runtime_error("DLL already loaded");wchar_t path[32768];const auto n=GetModuleFileNameW(nullptr,path,32768);if(!n||n==32768)throw std::runtime_error("executable path");
        const auto library=std::filesystem::canonical(std::filesystem::path(path).parent_path()/"tn_renderer_filament.dll");
        if(!SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32))throw std::runtime_error("DLL search policy");module=LoadLibraryExW(library.c_str(),nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32);if(!module)throw std::runtime_error("Filament DLL load");
        symbol(abi_version,"tn_renderer_abi_version");symbol(create,"tn_renderer_create");symbol(describe,"tn_renderer_describe");symbol(upload,"tn_renderer_upload_scene");symbol(begin,"tn_renderer_begin_frame");symbol(poll,"tn_renderer_poll");symbol(read,"tn_renderer_read_rgba8");symbol(cancel,"tn_renderer_cancel");symbol(destroy,"tn_renderer_destroy");
    }
    void unload(){if(module){const auto owned=module;module=nullptr;if(!FreeLibrary(owned))throw std::runtime_error("bridge FreeLibrary failed");if(GetModuleHandleW(L"tn_renderer_filament.dll"))throw std::runtime_error("bridge remained loaded after final owned reference released");}}
    ~Api(){try{unload();}catch(...){}}
} api;
uint64_t poll_max_ns=0,poll_calls=0,cancel_max_ns=0,cancel_calls=0;
int32_t measured_poll(tn_context c,tn_poll* p,tn_error* e){auto start=std::chrono::steady_clock::now();auto status=api.poll(c,p,e);auto ns=uint64_t(std::chrono::duration_cast<std::chrono::nanoseconds>(std::chrono::steady_clock::now()-start).count());poll_max_ns=std::max(poll_max_ns,ns);++poll_calls;return status;}
int32_t measured_cancel(tn_context c,tn_error* e){auto start=std::chrono::steady_clock::now();auto status=api.cancel(c,e);auto ns=uint64_t(std::chrono::duration_cast<std::chrono::nanoseconds>(std::chrono::steady_clock::now()-start).count());cancel_max_ns=std::max(cancel_max_ns,ns);++cancel_calls;return status;}
#define tn_renderer_abi_version api.abi_version
#define tn_renderer_create api.create
#define tn_renderer_describe api.describe
#define tn_renderer_upload_scene api.upload
#define tn_renderer_begin_frame api.begin
#define tn_renderer_poll measured_poll
#define tn_renderer_read_rgba8 api.read
#define tn_renderer_cancel measured_cancel
#define tn_renderer_destroy api.destroy

namespace {
char diagnostic[4096]{};
tn_error error{sizeof(error),1,reinterpret_cast<uint8_t*>(diagnostic),sizeof(diagnostic)};
void check(bool value,const char* label) { if(!value) throw std::runtime_error(std::string(label)+": "+diagnostic); }
void ok(int result,const char* label) { check(result==TN_OK,label); }
void save(const std::filesystem::path& path,const std::vector<uint8_t>& bytes){std::ofstream stream(path,std::ios::binary);stream.write(reinterpret_cast<const char*>(bytes.data()),bytes.size());check(bool(stream),"artifact write");}
void process_evidence(const std::filesystem::path& out){
    PROCESS_MEMORY_COUNTERS memory{};memory.cb=sizeof(memory);check(GetProcessMemoryInfo(GetCurrentProcess(),&memory,sizeof(memory))!=0,"process memory");
    tn::Json evidence{{"peak_working_set_bytes",memory.PeakWorkingSetSize},{"peak_pagefile_bytes",memory.PeakPagefileUsage}};std::ofstream(out/"process-memory.json")<<evidence.dump(2);
    auto handle=CreateToolhelp32Snapshot(TH32CS_SNAPMODULE,GetCurrentProcessId());check(handle!=INVALID_HANDLE_VALUE,"module snapshot");MODULEENTRY32W entry{};entry.dwSize=sizeof(entry);tn::Json modules=tn::Json::array();
    if(Module32FirstW(handle,&entry))do{const auto text=std::filesystem::path(entry.szExePath).u8string();modules.push_back(std::string(text.begin(),text.end()));}while(Module32NextW(handle,&entry));CloseHandle(handle);std::ofstream(out/"loaded-modules.json")<<modules.dump(2);
}
std::vector<uint8_t> render(tn_context c,tn_frame f) {
    ok(tn_renderer_begin_frame(c,&f,&error),"begin");
    tn_poll p{sizeof(p),1}; auto deadline=std::chrono::steady_clock::now()+std::chrono::seconds(120);
    do { ok(tn_renderer_poll(c,&p,&error),"poll"); check(p.state!=TN_FAILED,"SDK failure");
        check(std::chrono::steady_clock::now()<deadline,"timeout");
        if(p.state!=TN_READY) std::this_thread::sleep_for(std::chrono::milliseconds(1));
    } while(p.state!=TN_READY);
    tn_image image{sizeof(image),1}; std::vector<uint8_t> pixels(size_t(f.width)*f.height*4);
    uint8_t sentinel=123;
    check(tn_renderer_read_rgba8(c,&sentinel,1,&image,&error)==TN_CAPACITY && sentinel==123,"capacity preserves output");
    check(image.required_bytes==pixels.size(),"exact capacity");
    ok(tn_renderer_read_rgba8(c,pixels.data(),pixels.size(),&image,&error),"read");
    for(size_t i=0;i<pixels.size();i+=4) if(!pixels[i+3]) check(!(pixels[i]|pixels[i+1]|pixels[i+2]),"zero alpha RGB");
    return pixels;
}
struct Context { tn_context value{}; ~Context(){if(value)tn_renderer_destroy(value);} };
}
int main(int argc,char** argv) try {
    std::filesystem::path out=argc>1?argv[1]:"."; std::filesystem::create_directories(out);
    const bool quick=argc>2&&std::string(argv[2])=="quick";
    const std::string experiment=argc>3?argv[3]:"";
    api.load();
    tn_abi_info abi{sizeof(abi),1}; ok(tn_renderer_abi_version(&abi,&error),"ABI");
    check(abi.backend==TN_BACKEND_FILAMENT && abi.flags==0,"production identity");
    auto config=config_document();
    config["selection"]["quality"]["aa"]="low";config["selection"]["quality"]["shadows"]="low";
    constexpr uint64_t host=uint64_t(8)*1024*1024*1024,gpu=uint64_t(10)*1024*1024*1024;
    config["limits"]["host_bytes"]=host;config["limits"]["gpu_bytes"]=gpu;
    auto json=config.dump();tn_config ci{sizeof(ci),1,reinterpret_cast<const uint8_t*>(json.data()),json.size(),host,gpu,8000,0};
    Context c;ok(tn_renderer_create(&ci,&c.value,&error),"create");
    uint64_t required=0;check(tn_renderer_describe(c.value,nullptr,0,&required,&error)==TN_CAPACITY,"describe query");
    std::string info(size_t(required),' ');ok(tn_renderer_describe(c.value,reinterpret_cast<uint8_t*>(info.data()),info.size(),&required,&error),"describe");
    std::cout<<info<<'\n'; std::ofstream(out/"identity.json")<<info;
    for(const auto* name:{"cube","hollow"}) {
        std::ifstream input(std::filesystem::path(TN_FIXTURES)/(std::string(name)+"-10mm.mesh.bin"),std::ios::binary);
        check(bool(input),"fixture open");char magic[8];uint32_t version,nv,nt;input.read(magic,8);input.read(reinterpret_cast<char*>(&version),4);input.read(reinterpret_cast<char*>(&nv),4);input.read(reinterpret_cast<char*>(&nt),4);
        std::vector<float> positions(size_t(nv)*3);std::vector<uint32_t> triangles(size_t(nt)*3);
        input.read(reinterpret_cast<char*>(positions.data()),positions.size()*sizeof(float));input.read(reinterpret_cast<char*>(triangles.data()),triangles.size()*sizeof(uint32_t));
        check(bool(input),"fixture read");
        // Aligned face normals: deliberately split each triangle, like the host.
        std::vector<float> flat,normals;std::vector<uint32_t> indices,materials(nt,0);
        for(size_t t=0;t<nt;++t) {
            const auto a=triangles[t*3],b=triangles[t*3+1],d=triangles[t*3+2];
            float u[3],v[3],n[3];for(size_t k=0;k<3;++k){u[k]=positions[b*3+k]-positions[a*3+k];v[k]=positions[d*3+k]-positions[a*3+k];}
            n[0]=u[1]*v[2]-u[2]*v[1];n[1]=u[2]*v[0]-u[0]*v[2];n[2]=u[0]*v[1]-u[1]*v[0];float length=std::sqrt(n[0]*n[0]+n[1]*n[1]+n[2]*n[2]);
            for(size_t j=0;j<3;++j){indices.push_back(uint32_t(indices.size()));for(size_t k=0;k<3;++k){flat.push_back(positions[triangles[t*3+j]*3+k]);normals.push_back(n[k]/length);}}
        }
        auto scene=scene_document();scene["appearance"]["studio_preset"]="diagram";scene["reference_bounds_mm"]["max"]={10,10,10};scene["materials"][0]["base_color_linear"]={1,1,1};
        auto sj=scene.dump();tn_scene si{sizeof(si),1,1,flat.data(),normals.data(),flat.data(),indices.data(),materials.data(),uint32_t(flat.size()/3),nt,reinterpret_cast<const uint8_t*>(sj.data()),sj.size()};
        ok(tn_renderer_upload_scene(c.value,&si,&error),"upload");
        auto quality=tn::Json{{"schema_version",1},{"quality",config["selection"]["quality"]}}.dump();auto f=frame_input(1,quality);f.width=f.height=512;f.projection=1;f.vertical=16;f.near_mm=0.01;f.far_mm=100;
        f.camera_position_mm[0]=5;f.camera_position_mm[1]=-25;f.camera_position_mm[2]=5;f.camera_target_mm[0]=f.camera_target_mm[1]=f.camera_target_mm[2]=5;
        auto pixels=render(c.value,f);size_t intersection=0,uni=0,foreground=0;
        for(size_t y=0;y<512;++y)for(size_t x=0;x<512;++x){bool expected=x>=96&&x<416&&y>=96&&y<416;if(std::string(name)=="hollow"&&x>=192&&x<320&&y>=192&&y<320)expected=false;bool actual=pixels[(y*512+x)*4+3]!=0;intersection+=expected&&actual;uni+=expected||actual;foreground+=actual;}
        const double iou=double(intersection)/double(uni);std::cout<<name<<" foreground="<<foreground<<" iou="<<iou<<'\n';check(iou>=0.995,"frozen mask IoU");
        const auto expected_at=[&](int x,int y){return x>=96&&x<416&&y>=96&&y<416&&!(std::string(name)=="hollow"&&x>=192&&x<320&&y>=192&&y<320);};
        for(int y=1;y<511;++y)for(int x=1;x<511;++x){const bool actual=pixels[(size_t(y)*512+size_t(x))*4+3]!=0;const bool expected=expected_at(x,y);if(actual!=expected){bool boundary=false;for(int dy=-1;dy<=1;++dy)for(int dx=-1;dx<=1;++dx)boundary|=expected_at(x+dx,y+dy)!=expected;check(boundary,"frozen one-pixel boundary distance");}}
        if(std::string(name)=="hollow")check(pixels[(256*512+256)*4+3]==0,"hollow void retained");
        std::ofstream raw(out/(std::string(name)+"-512.rgba"),std::ios::binary);raw.write(reinterpret_cast<const char*>(pixels.data()),pixels.size());
        auto b=f;b.model_to_world[3]=1;auto bp=render(c.value,b);check(bp!=pixels,"pose changes pixels");auto again=render(c.value,f);check(again==pixels,"A B A exact history reset");
        if(std::string(name)=="cube") {
            if(!quick){f.width=f.height=4096;auto large=render(c.value,f);save(out/"cube-4096.rgba",large);f.width=f.height=512;}
            // Exercise actual physical PBR, mapped materials, floor, resin and
            // original-build-coordinate layer shading through the public ABI.
            uint64_t revision=2;
            const auto upload=[&](){sj=scene.dump();si.revision=revision++;si.json=reinterpret_cast<const uint8_t*>(sj.data());si.json_bytes=sj.size();ok(tn_renderer_upload_scene(c.value,&si,&error),"physical upload");f.geometry_revision=si.revision;};
            scene["appearance"]["studio_preset"]="light";scene["materials"][0]["base_color_linear"]={0.7,0.13,0.025};upload();
            auto physical=render(c.value,f);save(out/"physical-front-512.rgba",physical);check(physical!=pixels,"actual lit physical material");
            auto physical_b=f;physical_b.model_to_world[3]=1;render(c.value,physical_b);check(render(c.value,f)==physical,"physical A B A exact history reset");
            scene["appearance"]["layer_appearance"]=true;scene["appearance"]["first_layer_height_mm"]=0.5;scene["appearance"]["layer_height_mm"]=1.0;scene["appearance"]["groove_depth_mm"]=0.08;scene["layer_filter"]["enabled"]=true;upload();
            auto layers=render(c.value,f);save(out/"layers-front-512.rgba",layers);check(layers!=physical,"physical layer shader changes lit side");
            auto posed=f;posed.model_to_world[0]=0;posed.model_to_world[2]=1;posed.model_to_world[8]=-1;posed.model_to_world[10]=0;posed.model_to_world[11]=10;
            auto posed_layers=render(c.value,posed);save(out/"layers-posed-512.rgba",posed_layers);check(posed_layers!=layers,"layers retain build coordinates after ninety degree pose");check(render(c.value,f)==layers,"layer A B A history reset");
            for(size_t i=2;i<flat.size();i+=3)flat[i]-=3;scene["reference_bounds_mm"]["min"][2]=-3;scene["reference_bounds_mm"]["max"][2]=7;f.camera_position_mm[2]-=3;f.camera_target_mm[2]-=3;upload();
            auto shifted_layers=render(c.value,f);save(out/"layers-negative-origin-512.rgba",shifted_layers);int max_origin_error=0;for(size_t i=0;i<layers.size();++i)max_origin_error=std::max(max_origin_error,std::abs(int(layers[i])-int(shifted_layers[i])));std::cout<<"negative-origin maxcodeerror="<<max_origin_error<<'\n';check(max_origin_error<=1,"layer phase uses fixed full-source minZ");
            for(size_t i=2;i<flat.size();i+=3)flat[i]+=3;scene["reference_bounds_mm"]["min"][2]=0;scene["reference_bounds_mm"]["max"][2]=10;f.camera_position_mm[2]+=3;f.camera_target_mm[2]+=3;
            scene["appearance"]["first_layer_height_mm"]=0.02;scene["appearance"]["layer_height_mm"]=0.02;scene["appearance"]["groove_depth_mm"]=0.005;upload();auto faded=render(c.value,f);scene["appearance"]["layer_appearance"]=false;upload();check(render(c.value,f)==faded,"subpixel layer amplitude fades to zero without changing geometry");save(out/"layers-subpixel-faded-512.rgba",faded);
            scene["appearance"]["layer_appearance"]=true;scene["appearance"]["first_layer_height_mm"]=0.5;scene["appearance"]["layer_height_mm"]=1.0;scene["appearance"]["groove_depth_mm"]=0.08;upload();
            quality=tn::Json{{"schema_version",1},{"quality",{{"kind","filament"},{"aa","high"},{"shadows","high"}}}}.dump();f.quality_json=reinterpret_cast<const uint8_t*>(quality.data());f.quality_json_bytes=quality.size();auto high_layers=render(c.value,f);save(out/"layers-high-512.rgba",high_layers);
            auto high_pose=f;high_pose.model_to_world[0]=0;high_pose.model_to_world[2]=1;high_pose.model_to_world[8]=-1;high_pose.model_to_world[10]=0;high_pose.model_to_world[11]=10;auto high_posed=render(c.value,high_pose);save(out/"layers-high-posed-512.rgba",high_posed);check(high_posed!=high_layers,"High posed build-coordinate layers");check(render(c.value,f)==high_layers,"High layer A B A reset");
            scene["appearance"]["first_layer_height_mm"]=0.02;scene["appearance"]["layer_height_mm"]=0.02;scene["appearance"]["groove_depth_mm"]=0.005;upload();auto high_faded=render(c.value,f);scene["appearance"]["layer_appearance"]=false;upload();check(render(c.value,f)==high_faded,"High subpixel fade exact");save(out/"layers-high-faded-512.rgba",high_faded);
            scene["appearance"]["layer_appearance"]=true;scene["appearance"]["first_layer_height_mm"]=0.5;scene["appearance"]["layer_height_mm"]=1.0;scene["appearance"]["groove_depth_mm"]=0.08;upload();quality=tn::Json{{"schema_version",1},{"quality",config["selection"]["quality"]}}.dump();f.quality_json=reinterpret_cast<const uint8_t*>(quality.data());f.quality_json_bytes=quality.size();
            // Re-upload with the SAME numeric revision must replace buffers and
            // materials; the common facade maintains an internal generation.
            scene["materials"][0]["base_color_linear"]={0.02,0.12,0.8};sj=scene.dump();si.json=reinterpret_cast<const uint8_t*>(sj.data());si.json_bytes=sj.size();ok(tn_renderer_upload_scene(c.value,&si,&error),"same revision replacement");
            auto replacement=render(c.value,f);save(out/"same-revision-blue-512.rgba",replacement);check(replacement!=layers,"latest upload replaces same revision");
            scene["geometry_kind"]="beads";sj=scene.dump();si.json=reinterpret_cast<const uint8_t*>(sj.data());si.json_bytes=sj.size();check(tn_renderer_upload_scene(c.value,&si,&error)==TN_INVALID_ARGUMENT,"common guard rejects duplicate bead grooves");
            scene["appearance"]["layer_appearance"]=false;upload();auto beads=render(c.value,f);save(out/"beads-no-shader-grooves-512.rgba",beads);
            scene["geometry_kind"]="stepped";upload();check(render(c.value,f)==beads,"stepped geometry cannot receive duplicate shader grooves");
            scene["geometry_kind"]="original";upload();check(render(c.value,f)==beads,"bead geometry cannot receive duplicate grooves");
            scene["geometry_kind"]="original";scene["materials"][0]["base_color_linear"]={0.7,0.13,0.025};scene["materials"].push_back(scene["materials"][0]);scene["materials"][1]["name"]="blue";scene["materials"][1]["base_color_linear"]={0.02,0.12,0.8};
            for(size_t t=0;t<materials.size();++t)materials[t]=uint32_t(t%2);upload();auto mapped=render(c.value,f);save(out/"mapped-red-blue-512.rgba",mapped);check(mapped!=physical,"mapped triangle colors");
            f.camera_position_mm[0]=23;f.camera_position_mm[1]=-25;f.camera_position_mm[2]=22;f.vertical=23;f.transparent=0;auto floor_image=render(c.value,f);save(out/"studio-floor-light-512.rgba",floor_image);check(floor_image[3]==255,"opaque background alpha");
            f.transparent=1;auto object_mask=render(c.value,f);f.transparent=0;
            auto empty_si=si;empty_si.vertex_count=empty_si.triangle_count=0;empty_si.positions_mm=empty_si.normals=empty_si.build_positions_mm=nullptr;empty_si.triangles=empty_si.triangle_materials=nullptr;ok(tn_renderer_upload_scene(c.value,&empty_si,&error),"empty prepared geometry");
            auto empty_floor=render(c.value,f);save(out/"studio-empty-floor-512.rgba",empty_floor);size_t cast_pixels=0;for(size_t i=0;i<floor_image.size();i+=4)if(object_mask[i+3]==0&&int(empty_floor[i])+int(empty_floor[i+1])+int(empty_floor[i+2])-int(floor_image[i])-int(floor_image[i+1])-int(floor_image[i+2])>12)++cast_pixels;
            std::cout<<"floor cast/AO pixels="<<cast_pixels<<'\n';check(cast_pixels>100,"studio casts outside geometry silhouette");upload();
            for(const auto* profile:{"medium","high"}){
                const auto aa=std::string(profile)=="high"&&experiment=="shadow4k"?"medium":profile;
                const auto sh=std::string(profile)=="high"&&experiment=="aa8"?"medium":profile;
                quality=tn::Json{{"schema_version",1},{"quality",{{"kind","filament"},{"aa",aa},{"shadows",sh}}}}.dump();std::cout<<"quality test "<<quality<<'\n';f.quality_json=reinterpret_cast<const uint8_t*>(quality.data());f.quality_json_bytes=quality.size();auto qualified=render(c.value,f);save(out/(std::string("studio-")+profile+"-512.rgba"),qualified);check(qualified[3]==255,"quality profile keeps opaque alpha");
            }
            if(!quick){f.width=f.height=4096;auto qualified=render(c.value,f);save(out/"studio-high-4096.rgba",qualified);f.width=f.height=512;}
            quality=tn::Json{{"schema_version",1},{"quality",config["selection"]["quality"]}}.dump();f.quality_json=reinterpret_cast<const uint8_t*>(quality.data());f.quality_json_bytes=quality.size();
            scene["appearance"]["studio_preset"]="dark";upload();auto dark=render(c.value,f);save(out/"studio-floor-dark-512.rgba",dark);check(dark!=floor_image,"dark studio differs");
            for(auto& m:materials)m=0;scene["materials"].erase(1);scene["appearance"]["studio_preset"]="light";scene["materials"][0]["base_color_linear"]={0.08,0.4,0.7};scene["materials"][0]["transmission"]=0.85;scene["materials"][0]["roughness"]=0.08;scene["materials"][0]["absorption_density_per_mm"]=0.005;scene["materials"][0]["absorption_color_linear"]={0.3,0.7,0.9};f.transparent=1;upload();
            auto resin=render(c.value,f);save(out/"resin-transparent-512.rgba",resin);size_t translucent=0;for(size_t i=3;i<resin.size();i+=4)translucent+=resin[i]>5&&resin[i]<250;check(translucent>1000,"resin independent translucent alpha");check(resin[3]==0,"transparent surroundings and floor");
            auto resin_b=f;resin_b.model_to_world[3]=1;render(c.value,resin_b);check(render(c.value,f)==resin,"resin A B A exact history reset");
            quality=tn::Json{{"schema_version",1},{"quality",{{"kind","filament"},{"aa","high"},{"shadows","high"}}}}.dump();f.quality_json=reinterpret_cast<const uint8_t*>(quality.data());f.quality_json_bytes=quality.size();auto high_resin=render(c.value,f);save(out/"resin-high-transparent-512.rgba",high_resin);auto high_resin_b=f;high_resin_b.model_to_world[3]=1;render(c.value,high_resin_b);check(render(c.value,f)==high_resin,"High resin A B A exact reset");
            // Cancel after GPU work has started, then reuse after acknowledged
            // drain. Poll/cancel remain common-facade lock-free operations.
            ok(tn_renderer_begin_frame(c.value,&f,&error),"readback cancellation begin");tn_poll active{sizeof(active),1};
            do{ok(tn_renderer_poll(c.value,&active,&error),"readback work poll");std::this_thread::yield();}while(active.state==TN_RENDERING&&active.completed_work<2);
            ok(tn_renderer_cancel(c.value,&error),"readback cancel");tn_poll cancelled{sizeof(cancelled),1};do{ok(tn_renderer_poll(c.value,&cancelled,&error),"readback cancel poll");std::this_thread::yield();}while(cancelled.state!=TN_CANCELLED);
            check(render(c.value,f)==high_resin,"reuse after readback cancellation");
            for(int pose_index=0;pose_index<64;++pose_index){const auto angle=double(pose_index)*6.283185307179586/64;auto orbit=f;orbit.camera_position_mm[0]=5+30*std::cos(angle);orbit.camera_position_mm[1]=5+30*std::sin(angle);orbit.camera_position_mm[2]=22;render(c.value,orbit);}check(render(c.value,f)==high_resin,"High image stable after long orbit");
        }
        ok(tn_renderer_begin_frame(c.value,&f,&error),"cancel begin");ok(tn_renderer_cancel(c.value,&error),"cancel");
        tn_poll p{sizeof(p),1};do{ok(tn_renderer_poll(c.value,&p,&error),"cancel poll");check(p.state!=TN_FAILED,"cancel failure");std::this_thread::sleep_for(std::chrono::milliseconds(1));}while(p.state!=TN_CANCELLED);
    }
    // Smooth physical sphere: metallic diffuse is zero, and changing only
    // studio intensity while directional lights stay fixed isolates specular IBL.
    std::vector<float> sphere_positions,sphere_normals;std::vector<uint32_t> sphere_indices;
    constexpr uint32_t latitudes=24,longitudes=48;
    for(uint32_t lat=0;lat<=latitudes;++lat)for(uint32_t lon=0;lon<=longitudes;++lon){const double theta=double(lat)*3.141592653589793/latitudes,phi=double(lon)*6.283185307179586/longitudes;const std::array<float,3> normal{{float(std::sin(theta)*std::cos(phi)),float(std::sin(theta)*std::sin(phi)),float(std::cos(theta))}};for(auto n:normal){sphere_normals.push_back(n);sphere_positions.push_back(5+5*n);}}
    for(uint32_t lat=0;lat<latitudes;++lat)for(uint32_t lon=0;lon<longitudes;++lon){const auto a=lat*(longitudes+1)+lon,b=a+1,d=a+longitudes+2,e=d-1;sphere_indices.insert(sphere_indices.end(),{a,e,b,b,e,d});}
    std::vector<uint32_t> sphere_materials(sphere_indices.size()/3,0);auto showcase=scene_document();showcase["appearance"]["studio_preset"]="light";showcase["reference_bounds_mm"]["max"]={10,10,10};showcase["materials"][0]["base_color_linear"]={0.8,0.45,0.12};showcase["materials"][0]["metallic"]=1;showcase["materials"][0]["roughness"]=0.08;
    auto showcase_json=showcase.dump();tn_scene showcase_input{sizeof(showcase_input),1,300,sphere_positions.data(),sphere_normals.data(),sphere_positions.data(),sphere_indices.data(),sphere_materials.data(),uint32_t(sphere_positions.size()/3),uint32_t(sphere_indices.size()/3),reinterpret_cast<const uint8_t*>(showcase_json.data()),showcase_json.size()};
    const auto showcase_quality=tn::Json{{"schema_version",1},{"quality",{{"kind","filament"},{"aa","high"},{"shadows","high"}}}}.dump();auto showcase_frame=frame_input(300,showcase_quality);showcase_frame.width=showcase_frame.height=512;showcase_frame.projection=1;showcase_frame.vertical=16;showcase_frame.near_mm=0.01;showcase_frame.far_mm=100;showcase_frame.camera_position_mm[0]=23;showcase_frame.camera_position_mm[1]=-25;showcase_frame.camera_position_mm[2]=22;showcase_frame.camera_target_mm[0]=showcase_frame.camera_target_mm[1]=showcase_frame.camera_target_mm[2]=5;
    const auto showcase_render=[&](const char* name){showcase_json=showcase.dump();showcase_input.json=reinterpret_cast<const uint8_t*>(showcase_json.data());showcase_input.json_bytes=showcase_json.size();ok(tn_renderer_upload_scene(c.value,&showcase_input,&error),"PBR showcase upload");auto pixels=render(c.value,showcase_frame);save(out/(std::string(name)+"-512.rgba"),pixels);return pixels;};
    auto metal=showcase_render("sphere-metal-smooth");showcase["appearance"]["studio_preset"]="dark";auto dark_metal=showcase_render("sphere-metal-dark-ibl");uint64_t ibl_error=0,ibl_samples=0;for(size_t i=0;i<metal.size();i+=4)if(metal[i+3]==255&&dark_metal[i+3]==255)for(size_t channel=0;channel<3;++channel){ibl_error+=uint64_t(std::abs(int(metal[i+channel])-int(dark_metal[i+channel])));++ibl_samples;}std::cout<<"specular IBL mean-code-response="<<double(ibl_error)/double(ibl_samples)<<'\n';check(ibl_error>ibl_samples,"actual specular cubemap changes metallic material");
    showcase["appearance"]["studio_preset"]="light";showcase["materials"][0]["roughness"]=0.65;check(showcase_render("sphere-metal-rough")!=metal,"prefiltered roughness response");
    showcase["materials"][0]["base_color_linear"]={0.7,0.13,0.025};showcase["materials"][0]["roughness"]=0.35;showcase["materials"][0]["metallic"]=0.18;showcase["materials"][0]["anisotropy"]=0.65;showcase_render("sphere-silk");
    showcase["materials"][0]["metallic"]=0;showcase["materials"][0]["anisotropy"]=0;showcase["materials"][0]["coat"]=0.35;showcase["materials"][0]["coat_roughness"]=0.12;showcase_render("sphere-glossy-coat");
    // Prepared bead-like cylinder: smooth radial side normals and separately
    // split flat cap normals. The SDK must not average the cap/side boundary.
    std::vector<float> bead_positions,bead_normals;std::vector<uint32_t> bead_indices;
    const auto bead_vertex=[&](float x,float y,float z,float nx,float ny,float nz){const auto index=uint32_t(bead_positions.size()/3);bead_positions.insert(bead_positions.end(),{x,y,z});bead_normals.insert(bead_normals.end(),{nx,ny,nz});return index;};
    constexpr uint32_t ring=64;
    for(uint32_t end=0;end<2;++end)for(uint32_t i=0;i<ring;++i){const double angle=double(i)*6.283185307179586/ring;const float x=float(std::cos(angle)),y=float(std::sin(angle));bead_vertex(5+4*x,5+4*y,float(end)*10,x,y,0);}
    for(uint32_t i=0;i<ring;++i){const auto j=(i+1)%ring;bead_indices.insert(bead_indices.end(),{i,j,i+ring,j,j+ring,i+ring});}
    for(uint32_t end=0;end<2;++end){const float z=float(end)*10,nz=end?1.0f:-1.0f;const auto center=bead_vertex(5,5,z,0,0,nz),first=uint32_t(bead_positions.size()/3);for(uint32_t i=0;i<ring;++i){const double angle=double(i)*6.283185307179586/ring;bead_vertex(5+4*float(std::cos(angle)),5+4*float(std::sin(angle)),z,0,0,nz);}for(uint32_t i=0;i<ring;++i){const auto a=first+i,b=first+(i+1)%ring;if(end)bead_indices.insert(bead_indices.end(),{center,a,b});else bead_indices.insert(bead_indices.end(),{center,b,a});}}
    std::vector<uint32_t> bead_materials(bead_indices.size()/3,0);showcase["geometry_kind"]="beads";showcase_json=showcase.dump();showcase_input.positions_mm=showcase_input.build_positions_mm=bead_positions.data();showcase_input.normals=bead_normals.data();showcase_input.triangles=bead_indices.data();showcase_input.triangle_materials=bead_materials.data();showcase_input.vertex_count=uint32_t(bead_positions.size()/3);showcase_input.triangle_count=uint32_t(bead_indices.size()/3);auto bead_image=showcase_render("bead-smooth-side-flat-caps");
    const auto dot3=[](const std::array<double,3>& a,const std::array<double,3>& b){return a[0]*b[0]+a[1]*b[1]+a[2]*b[2];};
    const auto normalize3=[&](std::array<double,3> v){const auto norm=std::sqrt(dot3(v,v));for(auto& value:v)value/=norm;return v;};
    auto forward=normalize3({-18,30,-17}),right=normalize3({forward[1],-forward[0],0});const std::array<double,3> vertical{{right[1]*forward[2],-right[0]*forward[2],right[0]*forward[1]-right[1]*forward[0]}};
    std::array<uint8_t,3> cap_rgb{};int cap_error=0;bool first_cap=true;
    for(const auto xy:std::array<std::array<double,2>,5>{{{{0,0}},{{1,0}},{{-1,0}},{{0,1}},{{0,-1}}}}){const std::array<double,3> point{{xy[0],xy[1],5}};const auto x=size_t((dot3(point,right)/8+1)*256),y=size_t((1-dot3(point,vertical)/8)*256);check(x<512&&y<512&&bead_image[(y*512+x)*4+3]==255,"bead cap sample inside");for(size_t channel=0;channel<3;++channel){const auto value=bead_image[(y*512+x)*4+channel];if(first_cap)cap_rgb[channel]=value;cap_error=std::max(cap_error,std::abs(int(value)-int(cap_rgb[channel])));}first_cap=false;}
    std::cout<<"flat cap maxcodeerror="<<cap_error<<'\n';check(cap_error<=1,"flat caps retain split normals");
    //259 independently referenced material records instantiate259 SDK unlit
    // primitive ranges, including the highest valid shared ink/tool index258.
    auto ink_scene=scene_document();ink_scene["appearance"]["studio_preset"]="light";ink_scene["reference_bounds_mm"]["max"]={10,10,10};auto ink_material=ink_scene["materials"][0];ink_material["unlit"]=true;ink_material["base_color_linear"]={0.018,0.022,0.03};ink_scene["materials"]=tn::Json::array();
    std::vector<float> ink_positions,ink_normals;std::vector<uint32_t> ink_indices,ink_materials;
    for(uint32_t m=0;m<259;++m){auto material=ink_material;material["name"]="ink"+std::to_string(m);ink_scene["materials"].push_back(material);const float x=float(m%17)*0.5f+0.1f,z=float(m/17)*0.5f+0.1f;for(const auto point:std::array<std::array<float,3>,3>{{{{x,0,z}},{{x+0.35f,0,z}},{{x,0,z+0.35f}}}}){ink_indices.push_back(uint32_t(ink_indices.size()));ink_positions.insert(ink_positions.end(),point.begin(),point.end());ink_normals.insert(ink_normals.end(),{0,-1,0});}ink_materials.push_back(m);}
    auto ink_json=ink_scene.dump();tn_scene ink_input{sizeof(ink_input),1,200,ink_positions.data(),ink_normals.data(),ink_positions.data(),ink_indices.data(),ink_materials.data(),uint32_t(ink_positions.size()/3),259,reinterpret_cast<const uint8_t*>(ink_json.data()),ink_json.size()};ok(tn_renderer_upload_scene(c.value,&ink_input,&error),"259 unlit materials upload");
    const auto ink_quality=tn::Json{{"schema_version",1},{"quality",config["selection"]["quality"]}}.dump();auto ink_frame=frame_input(200,ink_quality);ink_frame.width=ink_frame.height=512;ink_frame.projection=1;ink_frame.vertical=16;ink_frame.near_mm=0.01;ink_frame.far_mm=100;ink_frame.camera_position_mm[0]=5;ink_frame.camera_position_mm[1]=-25;ink_frame.camera_position_mm[2]=5;ink_frame.camera_target_mm[0]=ink_frame.camera_target_mm[1]=ink_frame.camera_target_mm[2]=5;
    auto ink_light=render(c.value,ink_frame);save(out/"unlit-259-light-512.rgba",ink_light);size_t ink_pixels=0;std::array<uint8_t,3> ink_rgb{};for(size_t i=0;i<ink_light.size();i+=4)if(ink_light[i+3]){if(!ink_pixels)std::copy_n(ink_light.begin()+i,3,ink_rgb.begin());check(std::equal(ink_rgb.begin(),ink_rgb.end(),ink_light.begin()+i),"unlit RGB constant across259 normals/primitives");++ink_pixels;}check(ink_pixels>10000,"259 unlit visible triangles");
    ink_scene["appearance"]["studio_preset"]="dark";ink_json=ink_scene.dump();ink_input.json=reinterpret_cast<const uint8_t*>(ink_json.data());ink_input.json_bytes=ink_json.size();ok(tn_renderer_upload_scene(c.value,&ink_input,&error),"259 unlit dark upload");check(render(c.value,ink_frame)==ink_light,"unlit physical ink independent of studio lighting");std::cout<<"259 unlit materials pixels="<<ink_pixels<<" RGB="<<unsigned(ink_rgb[0])<<','<<unsigned(ink_rgb[1])<<','<<unsigned(ink_rgb[2])<<'\n';
    ink_scene["materials"][0]["unlit"]=1;ink_json=ink_scene.dump();ink_input.json=reinterpret_cast<const uint8_t*>(ink_json.data());ink_input.json_bytes=ink_json.size();check(tn_renderer_upload_scene(c.value,&ink_input,&error)==TN_INVALID_ARGUMENT,"unlit requires boolean");
    process_evidence(out);
    // Explicit unavailable selection fails before creating a Filament device.
    auto missing_config=config;missing_config["selection"]["device"]="vulkan:ffff:ffff:00000000000000000000000000000000";auto missing_json=missing_config.dump();auto missing_ci=ci;missing_ci.json=reinterpret_cast<const uint8_t*>(missing_json.data());missing_ci.json_bytes=missing_json.size();tn_context unavailable{};
    check(tn_renderer_create(&missing_ci,&unavailable,&error)==TN_FAILURE&&!unavailable,"missing explicit device fails");
    // Destroy an initialized context, unload all SDK code, reload and repeat
    // with a concrete selected UUID. Pending-frame destruction must synchronously
    // drain before the following FreeLibrary can release the code.
    tn_renderer_destroy(c.value);c.value=nullptr;
    auto budget_config=config;budget_config["limits"]["gpu_bytes"]=uint64_t(1024)*1024*1024;auto budget_json=budget_config.dump();auto budget_ci=ci;budget_ci.json=reinterpret_cast<const uint8_t*>(budget_json.data());budget_ci.json_bytes=budget_json.size();budget_ci.gpu_bytes=uint64_t(1024)*1024*1024;
    {Context limited;ok(tn_renderer_create(&budget_ci,&limited.value,&error),"budget context");auto metadata=scene_document().dump();tn_scene empty{sizeof(empty),1,1,nullptr,nullptr,nullptr,nullptr,nullptr,0,0,reinterpret_cast<const uint8_t*>(metadata.data()),metadata.size()};ok(tn_renderer_upload_scene(limited.value,&empty,&error),"budget empty upload");auto high_quality=tn::Json{{"schema_version",1},{"quality",{{"kind","filament"},{"aa","high"},{"shadows","high"}}}}.dump();auto oversized=frame_input(1,high_quality);oversized.width=oversized.height=4096;ok(tn_renderer_begin_frame(limited.value,&oversized,&error),"budget begin accepted asynchronously");tn_poll status{sizeof(status),1};int32_t polled;do{polled=tn_renderer_poll(limited.value,&status,&error);std::this_thread::yield();}while(polled==TN_OK&&status.state!=TN_FAILED);check(polled==TN_FAILURE&&std::string(diagnostic).find("GPU byte budget")!=std::string::npos,"4096 High rejected against oneGiB before target allocation");}
    budget_config=config;budget_config["limits"]["host_bytes"]=uint64_t(1024)*1024*1024;budget_json=budget_config.dump();budget_ci=ci;budget_ci.json=reinterpret_cast<const uint8_t*>(budget_json.data());budget_ci.json_bytes=budget_json.size();budget_ci.host_bytes=uint64_t(1024)*1024*1024;
    {Context limited;ok(tn_renderer_create(&budget_ci,&limited.value,&error),"host budget context");auto metadata=scene_document().dump();tn_scene empty{sizeof(empty),1,1,nullptr,nullptr,nullptr,nullptr,nullptr,0,0,reinterpret_cast<const uint8_t*>(metadata.data()),metadata.size()};ok(tn_renderer_upload_scene(limited.value,&empty,&error),"host budget upload");auto high_quality=tn::Json{{"schema_version",1},{"quality",{{"kind","filament"},{"aa","high"},{"shadows","high"}}}}.dump();auto oversized=frame_input(1,high_quality);oversized.width=oversized.height=4096;ok(tn_renderer_begin_frame(limited.value,&oversized,&error),"host budget begin asynchronous");tn_poll status{sizeof(status),1};int32_t polled;do{polled=tn_renderer_poll(limited.value,&status,&error);std::this_thread::yield();}while(polled==TN_OK&&status.state!=TN_FAILED);check(polled==TN_FAILURE&&std::string(diagnostic).find("host byte budget")!=std::string::npos,"4096 High rejected against oneGiB host before allocation");}
    api.unload();
    auto exact_config=config;exact_config["selection"]["device"]=tn::Json::parse(info).at("device_id");auto exact_json=exact_config.dump();auto exact_ci=ci;exact_ci.json=reinterpret_cast<const uint8_t*>(exact_json.data());exact_ci.json_bytes=exact_json.size();
    auto empty_scene=scene_document();empty_scene["appearance"]["studio_preset"]="diagram";auto empty_json=empty_scene.dump();const auto empty_quality=tn::Json{{"schema_version",1},{"quality",config["selection"]["quality"]}}.dump();
    tn::Json lifecycle=tn::Json::array();
    const int lifecycle_cycles=experiment.starts_with("cycles=")?std::stoi(experiment.substr(7)):8;
    check(lifecycle_cycles>=1&&lifecycle_cycles<=64,"bounded lifecycle iterations");
    for(int cycle=0;cycle<lifecycle_cycles;++cycle){
        api.load();Context pending;ok(tn_renderer_create(&exact_ci,&pending.value,&error),"reload explicit device create");tn_scene empty{sizeof(empty),1,1,nullptr,nullptr,nullptr,nullptr,nullptr,0,0,reinterpret_cast<const uint8_t*>(empty_json.data()),empty_json.size()};ok(tn_renderer_upload_scene(pending.value,&empty,&error),"empty stage upload");auto frame=frame_input(1,empty_quality);frame.width=frame.height=512;
        auto empty_pixels=render(pending.value,frame);for(auto value:empty_pixels)check(value==0,"empty stage transparent pixels");
        ok(tn_renderer_begin_frame(pending.value,&frame,&error),"pending destroy begin");tn_poll progress{sizeof(progress),1};do{ok(tn_renderer_poll(pending.value,&progress,&error),"pending destroy poll");std::this_thread::yield();}while(progress.state==TN_RENDERING&&progress.completed_work<2);
        tn_renderer_destroy(pending.value);pending.value=nullptr;api.unload();PROCESS_MEMORY_COUNTERS counters{};counters.cb=sizeof(counters);GetProcessMemoryInfo(GetCurrentProcess(),&counters,sizeof(counters));lifecycle.push_back({{"cycle",cycle},{"working_set_after_unload",counters.WorkingSetSize}});
    }
    std::ofstream(out/"destroy-unload-cycles.json")<<lifecycle.dump(2);
    std::ofstream(out/"nonblocking-calls.json")<<tn::Json{{"poll_calls",poll_calls},{"poll_max_ns",poll_max_ns},{"cancel_calls",cancel_calls},{"cancel_max_ns",cancel_max_ns}}.dump(2);
    PROCESS_MEMORY_COUNTERS final_memory{};final_memory.cb=sizeof(final_memory);check(GetProcessMemoryInfo(GetCurrentProcess(),&final_memory,sizeof(final_memory))!=0,"final process memory");std::ofstream(out/"final-process-memory.json")<<tn::Json{{"peak_working_set_bytes",final_memory.PeakWorkingSetSize},{"peak_pagefile_bytes",final_memory.PeakPagefileUsage},{"working_set_after_all_unloads",final_memory.WorkingSetSize}}.dump(2);
    std::cout<<"PASS real CABI masks, "<<(quick?"512 quick":"4096 High")<<", A-B-A, capacity, zero-alpha, cancellation, missing device, teardown\n";return 0;
} catch(const std::exception& e){std::cerr<<"FAIL "<<e.what()<<'\n';return 1;}
