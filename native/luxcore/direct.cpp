// Focused production backend TDD caller. Final lifecycle/output qualification
// uses conformance.cpp through the actual C ABI facade.
#include "test_inputs.hpp"
static tn::Json canonical_scene_document(){auto j=scene_document();j["appearance"]["studio_preset"]="light";return j;}
#include <filesystem>
#include <fstream>
#include <iostream>
#include <cmath>
#include "trace.hpp"
#include "layers.hpp"
#include "runtime.hpp"
namespace tn::lux::trace {
static std::filesystem::path output;static size_t generation=0,frame_number=0;
template<class T> void bytes(const std::string& name,const T* data,size_t count){std::ofstream f(output/name,std::ios::binary);f.write(reinterpret_cast<const char*>(data),static_cast<std::streamsize>(count*sizeof(T)));}
static void json(const std::string& name,const tn::Json& j){std::ofstream(output/name)<<j.dump(2);}
void scene_input(const tn::Scene& s,luxcore::Scene& sdk,const std::vector<std::string>& objects){
    ++generation;const auto prefix="upload-"+std::to_string(generation);
    json(prefix+".json",{{"revision",s.revision},{"metadata",s.metadata},{"positions",s.positions_mm},{"normals",s.normals},{"build_positions",s.build_positions_mm},{"triangles",s.triangles},{"triangle_materials",s.triangle_materials}});
    for(const auto& name:objects)sdk.SaveMesh(name,(output/(prefix+"-"+name+".ply")).string());
}
void frame_input(const Frame& f,luxcore::Scene& sdk,luxcore::RenderConfig& config){
    ++frame_number;const auto prefix="frame-"+std::to_string(frame_number);
    bytes(prefix+".parameters",&f.parameters,1);json(prefix+".quality.json",f.quality);
    json(prefix+".frame.json",{{"width",f.parameters.width},{"height",f.parameters.height},{"projection",f.parameters.projection},{"vertical",f.parameters.vertical}});
    std::ofstream(output/(prefix+".config"))<<config.ToProperties()->ToString();
    std::ofstream(output/(prefix+".scene"))<<sdk.ToProperties()->ToString();
    // Instance SaveMesh intentionally exports underlying source geometry;
    // actual instance placement is retained in the SDK scene properties.
    sdk.SaveMesh("mesh0",(output/(prefix+"-source.ply")).string());
}
void film(const std::vector<float>& rgba,const float* alpha,const std::vector<unsigned>& counts,luxcore::Film& film){const auto prefix="frame-"+std::to_string(frame_number);bytes(prefix+".hdr",rgba.data(),rgba.size());bytes(prefix+".alpha",alpha,counts.size()*2);bytes(prefix+".samples",counts.data(),counts.size());std::vector<float> normals(counts.size()*3);film.GetOutput<float>(luxcore::Film::OUTPUT_AVG_SHADING_NORMAL,normals.data());bytes(prefix+".normal",normals.data(),normals.size());}
}
static void require(bool b,const char* message){if(!b)throw std::runtime_error(message);}
int wmain(int argc,wchar_t** argv){try{
    if(argc==2&&std::wstring(argv[1])==L"--budget"){
        for(bool host:{false,true}){tn::Config c;c.host_bytes=uint64_t(16)<<30;c.gpu_bytes=uint64_t(16)<<30;if(host)c.host_bytes=1;else c.gpu_bytes=1;
            bool rejected=false;try{auto backend=tn::make_backend(c);}catch(const tn::InvalidInput&){rejected=true;}require(rejected,"insufficient runtime budget reached SDK initialization");}
        std::cout<<"PREINIT_RUNTIME_BUDGET_REJECTION_PASS\n";return 0;
    }
    if(argc==3&&std::wstring(argv[1])==L"--runtime-conflict"){
        auto ambient=LoadLibraryExW(argv[2],nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32);require(ambient!=nullptr,"ambient fixture load failed");
        bool rejected=false;try{tn::lux::Runtime runtime;}catch(const std::exception& e){std::cout<<e.what()<<'\n';rejected=true;}
        FreeLibrary(ambient);require(rejected,"same-name compiler from an unowned path was accepted");std::cout<<"COMPILER_CONFLICT_REJECTED\n";return 0;
    }
    if(argc==2&&std::wstring(argv[1])==L"--parse-graph"){
        auto metadata=canonical_scene_document();auto f=frame_input(1,"{}");auto scene=luxcore::Scene::Create();
        for(int variant=0;variant<3;++variant){
            metadata["layer_filter"]["enabled"]=true;f.projection=variant==1?1u:0u;if(variant==2)f.model_to_world[0]=2;
            tn::lux::LayerGraph graph;scene->Parse(graph.build(metadata,f));
            std::cout<<"variant="<<variant<<" stack_floats="<<graph.stack_floats<<" operations="<<graph.operation_count<<std::endl;
            require(graph.stack_floats<16384,"layer graph has excessive compiled evaluation stacks");
        }
        std::cout<<"ACTUAL_SDK_LAYER_GRAPH_PARSE_PASS\n";return 0;
    }
    require(argc==3||argc==4||argc==5,"arguments: existing owned cache, output directory, optional --extended or --replay input.json");
    tn::lux::trace::output=std::filesystem::path(argv[2]);
    auto j=config_document();j["selection"]={{"backend","luxcore"},{"device",nullptr},{"quality",{{"kind","luxcore"},{"samples",64},{"denoise",false}}}};j["identity"]["backend"]="luxcore";
    const auto cache=std::filesystem::path(argv[1]).u8string();j["cache_directory"]=std::string(cache.begin(),cache.end());
    const uint64_t budget=uint64_t(16)<<30;j["limits"]["host_bytes"]=budget;j["limits"]["gpu_bytes"]=budget;
    const auto config_json=j.dump();tn_config raw{sizeof(tn_config),1,reinterpret_cast<const uint8_t*>(config_json.data()),config_json.size(),budget,budget,8000,0};
    auto backend=tn::make_backend(tn::copy_config(&raw));std::cout<<backend->describe()<<std::endl;
    tn::Scene scene;scene.revision=1;scene.positions_mm={-2,0,4,2,0,4,2,0,8,-2,0,8};scene.build_positions_mm=scene.positions_mm;scene.normals={0,-1,0,0,-1,0,0,-1,0,0,-1,0};scene.triangles={0,1,2,0,2,3};scene.triangle_materials={0,0};
    scene.metadata=canonical_scene_document();scene.metadata["color"]["tone_map"]="turninator-agx-v1";scene.metadata["appearance"]["first_layer_height_mm"]=0.6;scene.metadata["appearance"]["layer_height_mm"]=0.3;scene.metadata["appearance"]["groove_depth_mm"]=0.06;scene.metadata["layer_filter"]["enabled"]=true;
    scene.metadata["reference_bounds_mm"]={{"min",{-2,0,4}},{"max",{2,0,8}}};scene.metadata["materials"][0]["base_color_linear"]={0.5,0.5,0.5};
    std::atomic<bool> cancelled=false;std::atomic<uint64_t> completed=0,total=0;tn::Work work{cancelled,completed,total};
    const std::string q=R"({"schema_version":1,"quality":{"kind":"luxcore","samples":64,"denoise":false}})";tn::Frame f;f.parameters=frame_input(1,q);f.parameters.width=f.parameters.height=128;f.parameters.projection=1;f.parameters.vertical=5;f.parameters.camera_position_mm[1]=-20;f.parameters.camera_position_mm[2]=6;f.parameters.camera_target_mm[2]=6;f.quality=tn::Json::parse(q);
    if(argc==5&&std::wstring(argv[3])==L"--replay"){
        std::ifstream source(argv[4]);tn::Json input;source>>input;
        scene.metadata=input.at("metadata");scene.positions_mm=input.at("positions").get<std::vector<float>>();scene.build_positions_mm=scene.positions_mm;scene.normals=input.at("normals").get<std::vector<float>>();scene.triangles=input.at("triangles").get<std::vector<uint32_t>>();
        if(input.contains("triangle_materials"))scene.triangle_materials=input.at("triangle_materials").get<std::vector<uint32_t>>();else scene.triangle_materials.assign(scene.triangles.size()/3,input.at("material").get<uint32_t>());
        f.parameters.width=65;f.parameters.height=49;f.parameters.projection=0;f.parameters.vertical=40;f.parameters.camera_position_mm[1]=0;f.parameters.camera_position_mm[2]=30;f.parameters.camera_target_mm[2]=0;f.parameters.camera_up[1]=1;f.parameters.camera_up[2]=0;
        f.quality["quality"]["samples"]=input.contains("triangle_materials")?256:64;
        backend->upload(scene,work);
        for(unsigned transparent:{1u,0u}){f.parameters.transparent=transparent;auto pixels=backend->render(f,work);std::ofstream out(std::filesystem::path(argv[2])/(transparent?"transparent.rgba":"opaque.rgba"),std::ios::binary);out.write(reinterpret_cast<const char*>(pixels.data()),static_cast<std::streamsize>(pixels.size()));std::cout<<backend->describe()<<std::endl;}
        std::cout<<"DIRECT_REPLAY_CAPTURED\n";return 0;
    }
    if(argc==4&&(std::wstring(argv[3])==L"--motion"||std::wstring(argv[3])==L"--motion-heldout")){
        scene.metadata["appearance"]["layer_appearance"]=true;backend->upload(scene,work);
        const auto phases=std::wstring(argv[3])==L"--motion-heldout"?std::vector<double>{.375}:std::vector<double>{0.,.25,.5,.75};
        for(double fraction:phases){f.parameters.model_to_world[11]=fraction*5/128;backend->render(f,work);}
        std::cout<<"DIRECT_MOTION_CAPTURED\n";return 0;
    }
    if(argc==4&&std::wstring(argv[3])==L"--extended"){
        auto save=[&](const char* name,const std::vector<uint8_t>& pixels){std::ofstream out(std::filesystem::path(argv[2])/name,std::ios::binary);out.write(reinterpret_cast<const char*>(pixels.data()),static_cast<std::streamsize>(pixels.size()));};
        auto compare=[&](const std::vector<uint8_t>& a,const std::vector<uint8_t>& b){int maximum=0;for(size_t i=0;i<a.size();++i)maximum=std::max(maximum,std::abs(int(a[i])-int(b[i])));return maximum;};
        scene.metadata["appearance"]["layer_appearance"]=true;backend->upload(scene,work);auto detail=backend->render(f,work);save("detail-raw.rgba",detail);
        f.quality["quality"]["denoise"]=true;auto denoised=backend->render(f,work);save("detail-denoised.rgba",denoised);f.quality["quality"]["denoise"]=false;
        for(size_t i=2;i<scene.positions_mm.size();i+=3){scene.positions_mm[i]-=10;scene.build_positions_mm[i]-=10;}
        scene.metadata["reference_bounds_mm"]["min"][2]=-6;scene.metadata["reference_bounds_mm"]["max"][2]=-2;backend->upload(scene,work);f.parameters.model_to_world[11]=10;
        auto negative=backend->render(f,work);save("negative-source.rgba",negative);const auto negative_error=compare(detail,negative);std::cout<<"negative_source_max_error="<<negative_error<<std::endl;require(negative_error<=2,"negative source minimum changes physical layers");
        // Rigid Y rotation about the wall center; source Z now runs along X.
        f.parameters.model_to_world[0]=0;f.parameters.model_to_world[2]=1;f.parameters.model_to_world[3]=4;f.parameters.model_to_world[8]=-1;f.parameters.model_to_world[10]=0;f.parameters.model_to_world[11]=6;
        auto posed=backend->render(f,work);save("posed.rgba",posed);
        f.parameters.model_to_world[0]=f.parameters.model_to_world[10]=1;f.parameters.model_to_world[2]=f.parameters.model_to_world[3]=f.parameters.model_to_world[8]=0;f.parameters.model_to_world[11]=10;f.parameters.vertical=30;
        auto faded=backend->render(f,work);save("faded.rgba",faded);
        f.parameters.vertical=5;auto returned=backend->render(f,work);save("returned.rgba",returned);const auto repeat_error=compare(negative,returned);
        std::cout<<"layer_pose_return_max_error="<<repeat_error<<std::endl;require(repeat_error<=2,"layer pose/fade edit retained accumulation");std::cout<<backend->describe()<<"\nDIRECT_EXTENDED_PASS\n";return 0;
    }
    backend->upload(scene,work);auto flat=backend->render(f,work);std::cout<<"flat "<<backend->describe()<<std::endl;
    backend->upload(scene,work);auto identical=backend->render(f,work);std::cout<<"identical "<<backend->describe()<<std::endl;
    int identity_error=0;for(size_t i=0;i<flat.size();++i)identity_error=std::max(identity_error,std::abs(int(flat[i])-int(identical[i])));
    std::cout<<"identical_upload_max_error="<<identity_error<<std::endl;
    require(identity_error<=2,"identical upload changes geometry/film");
    scene.metadata["appearance"]["layer_appearance"]=true;backend->upload(scene,work);auto layers=backend->render(f,work);std::cout<<"layers "<<backend->describe()<<std::endl;
    for(const auto& pair:{std::pair{"flat.rgba",&flat},{"layers.rgba",&layers}}){std::ofstream out(std::filesystem::path(argv[2])/pair.first,std::ios::binary);out.write(reinterpret_cast<const char*>(pair.second->data()),static_cast<std::streamsize>(pair.second->size()));}
    double difference=0;for(size_t y=20;y<108;++y)for(size_t x=20;x<108;++x)difference+=std::abs(int(flat[(y*128+x)*4])-int(layers[(y*128+x)*4]));difference/=88*88;
    std::cout<<"layer_closeup_mean_abs_difference="<<difference<<std::endl;require(difference>1.,"layer-enabled physical closeup equals flat surface");
    // Camera conversion must also preserve non-square perspective/ortho masks.
    // Repeated new layer uploads exercise teardown of the scalar carrier.
    for(unsigned projection:{1u,0u}){
        backend->upload(scene,work);f.parameters.width=193;f.parameters.height=97;f.parameters.projection=projection;f.parameters.vertical=projection?5:40;
        auto camera_pixels=backend->render(f,work);require(camera_pixels.size()==193*97*4,"camera dimensions changed");
    }
    std::cout<<"DIRECT_LAYER_PASS\n";return 0;
}catch(const std::exception& e){std::cerr<<"DIRECT_FAIL: "<<e.what()<<'\n';return 1;}}
