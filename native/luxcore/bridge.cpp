#include "runtime.hpp"
#include "layers.hpp"
#include "studio.hpp"
#include "owner.hpp"
#include "../../assets/render/color/turninator_agx.h"
#include <luxcore/luxcore.h>
#include <OpenImageDenoise/oidn.hpp>
#include <Psapi.h>
#include <chrono>
#include <thread>
#include <numeric>
#include <numbers>
#include <mutex>
#include <limits>
#ifdef TN_LUXCORE_DIAGNOSTIC_TRACE
#include "trace.hpp"
#endif

namespace tn::lux {
using luxrays::Properties;
using luxrays::Property;
using Clock=std::chrono::steady_clock;
static double elapsed(Clock::time_point start) {return std::chrono::duration<double,std::milli>(Clock::now()-start).count();}
static std::unique_ptr<Properties> properties(const std::string& text) {auto p=std::make_unique<Properties>();p->SetFromString(text);return p;}
static uint64_t sum(uint64_t a,uint64_t b) {if(a>UINT64_MAX-b) throw InvalidInput("LuxCore reservation overflow");return a+b;}
static uint64_t product(uint64_t a,uint64_t b) {if(b&&a>UINT64_MAX/b) throw InvalidInput("LuxCore reservation overflow");return a*b;}
static void rgb(Properties& p,const std::string& key,const Json& values) {p.Set(Property(key)(values[0].get<float>(),values[1].get<float>(),values[2].get<float>()));}
static void vector(Properties& p,const std::string& key,const double* values) {p.Set(Property(key)(values[0],values[1],values[2]));}
static std::unique_ptr<Properties> camera(const tn_frame& f) {
    auto p=std::make_unique<Properties>();vector(*p,"scene.camera.lookat.orig",f.camera_position_mm);vector(*p,"scene.camera.lookat.target",f.camera_target_mm);vector(*p,"scene.camera.up",f.camera_up);
    p->Set(Property("scene.camera.type")(f.projection?"orthographic":"perspective"));
    const double half=f.projection?f.vertical*.5:1.;const double aspect=double(f.width)/f.height;
    // Pinned GenerateRay maps jittered filmY to H-filmY-1. Compensate that
    // one-pixel raster origin in the screen window; preserve its full span.
    const double raster_origin=2*half/f.height;
    p->Set(Property("scene.camera.screenwindow")(-half*aspect,half*aspect,-half-raster_origin,half-raster_origin));
    p->Set(Property("scene.camera.fieldofview")(f.vertical));p->Set(Property("scene.camera.cliphither")(f.near_mm));p->Set(Property("scene.camera.clipyon")(f.far_mm));
    return p;
}
static void materials(luxcore::Scene& scene,const Json& records,bool layers=false) {
    auto p=std::make_unique<Properties>();
    for(size_t i=0;i<records.size();++i) {
        const auto& m=records[i];const auto name="m"+std::to_string(i),base="scene.materials."+name;
        const bool unlit=m.value("unlit",false);const float transmission=m.at("transmission").get<float>();
        const std::string opaque=transmission>0&&!unlit?base+"_opaque":base;
        p->Set(Property(opaque+".type")(unlit?"matte":"disney"));
        if(unlit) {p->Set(Property(opaque+".kd")(0.f));rgb(*p,opaque+".emission",m.at("base_color_linear"));p->Set(Property(opaque+".emission.directlightsampling.type")("DISABLED"));}
        else {
            rgb(*p,opaque+".basecolor",m.at("base_color_linear"));
            for(const auto& pair:{std::pair{"roughness","roughness"},{"metallic","metallic"},{"anisotropic","anisotropy"},{"clearcoat","coat"}}) p->Set(Property(opaque+"."+pair.first)(m.at(pair.second).get<float>()));
            p->Set(Property(opaque+".clearcoatgloss")(1.f-m.at("coat_roughness").get<float>()));p->Set(Property(opaque+".specular")(.5f));
        }
        if(transmission>0&&!unlit) {
            const auto glass=base+"_glass",volume="scene.volumes.v"+std::to_string(i);
            const bool delta=m.at("ior").get<float>()==1.f||m.at("roughness").get<float>()==0.f;
            // Equal-index interfaces have no optical roughness. The delta SDK
            // branch avoids roughglass's zero refractive half-vector Jacobian.
            p->Set(Property(glass+".type")(delta?"glass":"roughglass"));p->Set(Property(glass+".kr")(1.f));p->Set(Property(glass+".kt")(1.f));
            p->Set(Property(glass+".interiorior")(m.at("ior").get<float>()));p->Set(Property(glass+".exteriorior")(1.f));
            for(auto k:{"uroughness","vroughness"})p->Set(Property(glass+"."+k)(std::max(.0001f,m.at("roughness").get<float>())));
            const float density=m.at("absorption_density_per_mm").get<float>(),scatter=m.at("scattering_per_mm").get<float>();
            p->Set(Property(volume+".type")(scatter>0?"homogeneous":"clear"));p->Set(Property(volume+".ior")(m.at("ior").get<float>()));
            const auto& c=m.at("absorption_color_linear");p->Set(Property(volume+".absorption")((1-c[0].get<float>())*density,(1-c[1].get<float>())*density,(1-c[2].get<float>())*density));
            if(scatter>0) {p->Set(Property(volume+".scattering")(scatter));p->Set(Property(volume+".asymmetry")(m.at("scattering_anisotropy").get<float>()));p->Set(Property(volume+".multiscattering")(true));}
            p->Set(Property(glass+".volume.interior")("v"+std::to_string(i)));
            p->Set(Property(base+".type")("mix"));p->Set(Property(base+".material1")(name+"_opaque"));p->Set(Property(base+".material2")(name+"_glass"));p->Set(Property(base+".amount")(transmission));p->Set(Property(base+".volume.interior")("v"+std::to_string(i)));
        }
        if(layers&&!unlit)p->Set(Property(base+".bumptex")("tn_layers"));
    }
    scene.Parse(std::move(p));
}
class Renderer final:public Backend {
    // Declared first, destroyed last: all SDK/OIDN resources retain their DLLs.
    Runtime runtime;
    Config config;
    Json description,diagnostics;
    std::string device_mask,cache;
    luxcore::ScenePtr scene;
    luxcore::RenderConfigRPtr render_config;
    luxcore::RenderSessionRPtr session;
    std::vector<std::string> objects;
    Json metadata;
    Studio studio;
    uint64_t geometry_host{},geometry_gpu{},generation{};
    uint64_t frame_host{},frame_gpu{},task_count{};
    uint32_t last_width{},last_height{},last_aa{},last_seed{};
    bool started=false,last_transparent=false;
    void stop() {if(started) {const auto t=Clock::now();session->Stop();started=false;diagnostics["timings_ms"]["stop"]=elapsed(t);}}
    void release_session() {stop();session.reset();render_config.reset();}
    void reserve(uint64_t host,uint64_t gpu) {
        if(host>config.host_bytes||gpu>config.gpu_bytes) throw InvalidInput("LuxCore conservative SDK/film/denoiser/driver reservation exceeds budget");
        diagnostics["counters"]["host_reservation_bytes"]=host;diagnostics["counters"]["gpu_reservation_bytes"]=gpu;
    }
    void frame_scene(const tn_frame& f) {
        scene->Parse(camera(f));studio.frame(*scene,f);float column[16];for(size_t r=0;r<4;++r)for(size_t c=0;c<4;++c) column[c*4+r]=static_cast<float>(f.model_to_world[r*4+c]);
        for(const auto& object:objects)scene->UpdateObjectTransformation(object,column);
        if(metadata.at("appearance").at("layer_appearance").get<bool>()){
            LayerGraph graph;auto recipe=graph.build(metadata,f);
            // Include rounded GPU task stacks and both host/device compiled op
            // storage before handing the graph to the SDK. Padding covers SDK
            // implicit constants and the bounded physical material graphs.
            const auto stack=sum(graph.stack_floats,256),ops=sum(graph.operation_count,4096);
            reserve(sum(frame_host,sum(product(ops,32),64ull<<20)),sum(frame_gpu,sum(product(product(stack,4),task_count),sum(product(ops,16),64ull<<20))));
            diagnostics["counters"]["texture_stack_reservation_floats"]=stack;
            diagnostics["counters"]["texture_operation_reservation"]=ops;
            diagnostics["settings"]["layer_recipe"]="physical-tangent-band16-linear-v1; sine error <= 0.021933";
            scene->Parse(std::move(recipe));materials(*scene,metadata.at("materials"),true);scene->RemoveUnusedTextures();
        }
    }
    void denoise(std::vector<float>& rgba,uint32_t width,uint32_t height) {
        const auto start=Clock::now();auto device=::lux::oidn::newDevice(::lux::oidn::DeviceType::CPU);
        auto check=[&]{const char* message=nullptr;if(device.getError(message)!=::lux::oidn::Error::None)throw std::runtime_error(message?message:"OIDN device error");};check();
        device.set("setAffinity",false);device.commit();check();
        const size_t n=size_t(width)*height;std::vector<float> color(n*3),albedo(n*3),normal(n*3),result(n*3);
        for(size_t i=0;i<n;++i)for(size_t k=0;k<3;++k)color[i*3+k]=rgba[i*4+k];
        session->GetFilm().GetOutput<float>(luxcore::Film::OUTPUT_ALBEDO,albedo.data());session->GetFilm().GetOutput<float>(luxcore::Film::OUTPUT_AVG_SHADING_NORMAL,normal.data());
        // Monte Carlo guides are not intrinsically noise-free. Prefilter them
        // before using the high-quality clean-guide beauty model.
        for(const auto& guide:{std::pair{"albedo",albedo.data()},std::pair{"normal",normal.data()}}){
            auto auxiliary=device.newFilter("RT");auxiliary.set("maxMemoryMB",1024);
            auxiliary.setImage(guide.first,guide.second,::lux::oidn::Format::Float3,width,height);
            auxiliary.setImage("output",guide.second,::lux::oidn::Format::Float3,width,height);
            auxiliary.commit();check();auxiliary.execute();check();
        }
        auto filter=device.newFilter("RT");filter.set("hdr",true);filter.set("cleanAux",true);filter.set("maxMemoryMB",1024);
        filter.setImage("color",color.data(),::lux::oidn::Format::Float3,width,height);filter.setImage("albedo",albedo.data(),::lux::oidn::Format::Float3,width,height);filter.setImage("normal",normal.data(),::lux::oidn::Format::Float3,width,height);filter.setImage("output",result.data(),::lux::oidn::Format::Float3,width,height);
        filter.commit();check();diagnostics["settings"]["oidn_actual_quality"]=std::to_string(static_cast<int>(filter.get<::lux::oidn::Quality>("quality")));check();filter.execute();check();
        for(size_t i=0;i<n;++i)for(size_t k=0;k<3;++k)rgba[i*4+k]=result[i*3+k];
        diagnostics["timings_ms"]["denoise"]=elapsed(start);
    }
public:
    explicit Renderer(const Config& c):config(c) {
        if(!c.document.contains("cache_directory"))throw InvalidInput("LuxCore requires an existing app-owned cache_directory");
        const auto raw_cache=c.document.at("cache_directory").get<std::string>();
        auto directory=std::filesystem::path(std::u8string(raw_cache.begin(),raw_cache.end()));
        if(!directory.is_absolute()||!std::filesystem::is_directory(directory))throw InvalidInput("LuxCore cache_directory must exist");
        cache=utf8(std::filesystem::canonical(directory));
        static std::once_flag initialize;std::call_once(initialize,[]{luxcore::Init();});
        auto devices=luxcore::GetOpenCLDeviceDescs();const auto names=devices->GetAllUniqueSubNames("opencl.device");device_mask.assign(names.size(),'0');
        const auto& selected=c.document.at("selection").at("device");std::string id,name;bool found=false;
        for(size_t i=0;i<names.size();++i) if(devices->IsDefined(names[i]+".cuda.compute.major")) {
            const auto candidate="cuda:"+std::to_string(i);if(!selected.is_null()&&selected!=candidate)continue;
            id=candidate;name=devices->Get(Property(names[i]+".name")("")).Get<std::string>();device_mask[i]='1';found=true;break;
        }
        if(!found)throw InvalidInput("Requested CUDA/OptiX device unavailable; CPU fallback disabled");
        using DriverVersion=int(__stdcall*)(int*);const auto driver=GetModuleHandleW(L"nvcuda.dll");const auto get_version=reinterpret_cast<DriverVersion>(GetProcAddress(driver,"cuDriverGetVersion"));int version{};
        if(!get_version||get_version(&version))throw std::runtime_error("Actual CUDA driver identity unavailable");
        const auto implementation=GetModuleHandleW(L"nvcuda64.dll"),optix=GetModuleHandleW(L"nvoptix.dll");
        if(!implementation||!optix)throw std::runtime_error("Actual CUDA/OptiX driver modules unavailable");
        const auto implementation_path=module_path(implementation),optix_path=module_path(optix);
        runtime.artifacts["cuda_driver"]={{"path",utf8(implementation_path)},{"sha256",hash_file(implementation_path)}};
        runtime.artifacts["optix_driver"]={{"path",utf8(optix_path)},{"sha256",hash_file(optix_path)}};
        description={{"schema_version",1},{"backend","luxcore"},{"sdk_version",luxcore::GetPlatformDesc()->Get("version.number").Get<std::string>()},{"device_id",id},{"device_name",name},{"api","CUDA/OptiX"},{"driver","NVIDIA "+file_version(implementation_path)+"; CUDA "+std::to_string(version)}};
        description["compiler"]={{"name","NVRTC"},{"version","12.8"},{"sha256",runtime.artifacts.at("nvrtc").at("sha256")},{"path",runtime.artifacts.at("nvrtc").at("path")}};
        diagnostics={{"schema_version",1},{"counters",{{"omp_max_threads",1u},{"sdk_owner_thread_id",uint64_t(GetCurrentThreadId())}}},{"timings_ms",Json::object()},{"settings",{{"sdk_owner_policy","leased-shared-owner-v1"},{"sampling_policy","tile-aa-ceil-sqrt-plus-one-minimum-v1"},{"scene_units","millimeters"},{"cpu_task_policy","cpu-openmp-serial-v1"},{"denoiser","OIDN 2.5.0 CPU RT High HDR auxiliary-prefilter-v1 maxMemoryMB=1024"},{"color","turninator-agx-v1"},{"studio_recipe","canonical-world-disk64-floor16-v1"},{"material_recipe","physical-disney-volume-delta-equal-ior-v3-canonical-studio"},{"output_recipe","camera-black-premult-oidn-linear-unpremultiply-agx-v2"},{"layer_denoise_motion","approximate-v1-disable-denoise-for-stable-fine-layers"},{"session_recipe","dimensions-aa-seed-transparency-reuse-v2"},{"compiler_pair_sha256",compiler_hash}}},{"artifacts",runtime.artifacts}};
    }
    ~Renderer() noexcept override {try{release_session();}catch(...){ }session.reset();render_config.reset();scene.reset();}
    std::string describe() override {
        // Task5f adds the reviewed bounded diagnostics envelope to the facade.
        auto result=description;result["diagnostics"]=diagnostics;return result.dump();
    }
    void upload(const tn::Scene& input,Work& work) override {
        const auto start=Clock::now();release_session();scene.reset();objects.clear();metadata=input.metadata;++generation;
        const uint64_t vertices=input.positions_mm.size()/3,triangles=input.triangles.size()/3;
        // Includes original facade spans, temporary regrouping, worst-case split
        // mesh arrays, host and device BVH, retained scene, CUDA/JIT and driver.
        geometry_host=sum(2ull<<30,sum(product(vertices,64),product(triangles,384)));
        geometry_gpu=sum(1ull<<30,sum(product(vertices,48),product(triangles,256)));reserve(geometry_host,geometry_gpu);
        if(work.cancelled())return;
        scene=luxcore::Scene::Create();scene->SetDeleteMeshData(true);materials(*scene,metadata.at("materials"));
        studio.initialize(*scene,metadata);
        for(size_t m=0;m<metadata.at("materials").size();++m) {
            std::vector<uint32_t> indices;for(size_t t=0;t<input.triangle_materials.size();++t)if(input.triangle_materials[t]==m) indices.insert(indices.end(),input.triangles.begin()+t*3,input.triangles.begin()+t*3+3);
            if(indices.empty())continue;
            std::vector<uint32_t> remap(static_cast<size_t>(vertices),UINT32_MAX),source;source.reserve(indices.size());
            const bool layers=metadata.at("appearance").at("layer_appearance").get<bool>();
            if(layers){source=indices;std::iota(indices.begin(),indices.end(),0u);}
            else for(auto& index:indices){if(remap[index]==UINT32_MAX){remap[index]=static_cast<uint32_t>(source.size());source.push_back(index);}index=remap[index];}
            const auto count=static_cast<long>(source.size()),faces=static_cast<long>(indices.size()/3);
            std::unique_ptr<float[]> positions(luxcore::Scene::AllocVerticesBuffer(count)),normals(new float[size_t(count)*3]);
            std::unique_ptr<unsigned[]> mesh_indices(luxcore::Scene::AllocTrianglesBuffer(faces));
            for(size_t v=0;v<source.size();++v)for(size_t k=0;k<3;++k){positions[v*3+k]=input.positions_mm[source[v]*3+k];normals[v*3+k]=input.normals[source[v]*3+k];}
            std::copy(indices.begin(),indices.end(),mesh_indices.get());const auto name="mesh"+std::to_string(m);
            // Pinned 2.11.2 DefineMesh converts null attributes to engaged empty
            // spans, producing zero-length-but-present layers. The extended API
            // preserves nullptr as std::nullopt; never invoke that unsafe path.
            if(layers){
                // Every optional array is fully initialized here. Unlike the
                // position/index/normal arrays, the pinned SDK copies these
                // spans; the temporary vectors remain caller-owned.
                std::vector<float> uv(size_t(count)*2),colors(size_t(count)*3,1.f),alphas(size_t(count),1.f);
                const float min_z=metadata.at("reference_bounds_mm").at("min")[2].get<float>();
                // Float color R is a scalar carrier only. Pinned geometry
                // compilation copies floats and barycentrically interpolates;
                // hitpointgrey channel0 reads R without clamp/transfer. All
                // physical colors remain canonical per-triangle materials.
                // This avoids the pinned Stop() omission of vertexAOVBuff.
                for(size_t v=0;v<source.size();++v){uv[v*2]=v%3==1?1.f:0.f;uv[v*2+1]=v%3==2?1.f:0.f;colors[v*3]=input.build_positions_mm[source[v]*3+2]-min_z;}
                scene->DefineMesh(name,count,faces,positions.get(),mesh_indices.get(),normals.get(),uv.data(),colors.data(),alphas.data());positions.release();mesh_indices.release();normals.release();
                for(unsigned k=0;k<3;++k){std::vector<float> edge(static_cast<size_t>(faces));for(size_t t=0;t<edge.size();++t)edge[t]=input.positions_mm[source[t*3+1]*3+k]-input.positions_mm[source[t*3]*3+k];scene->SetMeshTriangleAOV(name,k,edge.data(),edge.size());}
            }else {scene->DefineMeshExt(name,count,faces,positions.get(),mesh_indices.get(),normals.get(),nullptr,nullptr,nullptr);positions.release();mesh_indices.release();normals.release();}
            auto object=std::make_unique<Properties>();object->Set(Property("scene.objects."+name+".shape")(name));object->Set(Property("scene.objects."+name+".material")("m"+std::to_string(m)));
            // Explicit identity selects an instance. Without this property the
            // pinned UpdateObjectTransformation applies a cumulative transform
            // directly to the source mesh instead of replacing instance pose.
            object->Set(Property("scene.objects."+name+".transformation")(std::vector<float>{1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1}));
            scene->Parse(std::move(object));objects.push_back(name);
        }
        diagnostics["counters"]["upload_generation"]=generation;diagnostics["counters"]["triangles"]=triangles;diagnostics["timings_ms"]["upload"]=elapsed(start);
#ifdef TN_LUXCORE_DIAGNOSTIC_TRACE
        trace::scene_input(input,*scene,objects);
#endif
    }
    std::vector<uint8_t> render(const Frame& frame,Work& work) override {
        const auto& f=frame.parameters;const uint32_t requested=frame.quality.at("quality").at("samples").get<uint32_t>();const bool use_denoise=frame.quality.at("quality").at("denoise").get<bool>();
        const uint32_t aa=static_cast<uint32_t>(std::ceil(std::sqrt(double(requested))))+1;
        task_count=product(32*32,product(aa,aa));task_count=product(sum(task_count,8191)/8192,8192);
        const uint64_t pixels=product(f.width,f.height);
        // Per-task backing includes path/ray/hit/material/volume/sample state;
        // texture evaluation stacks are additionally charged by frame_scene.
        frame_host=sum(geometry_host,sum(product(task_count,256),sum(product(pixels,use_denoise?512:256),use_denoise?uint64_t(1536)<<20:0)));
        frame_gpu=sum(geometry_gpu,sum(product(task_count,4096),product(pixels,256)));reserve(frame_host,frame_gpu);
        diagnostics["counters"]["gpu_task_count"]=task_count;
        diagnostics["counters"]["requested_minimum_samples"]=requested;diagnostics["counters"]["effective_aa_dimension"]=aa;diagnostics["counters"]["nominal_samples"]=uint64_t(aa)*aa;
        diagnostics["settings"]["denoise_enabled"]=use_denoise?"true":"false";
        work.total=pixels*aa*aa;
        try {
            const bool reuse=session&&started&&last_width==f.width&&last_height==f.height&&last_aa==aa&&last_seed==f.sample_seed&&last_transparent==(f.transparent!=0);
            auto start=Clock::now();
            if(reuse){session->BeginSceneEdit();frame_scene(f);session->EndSceneEdit();}
            else {
                release_session();frame_scene(f);
                auto p=properties("renderengine.type = TILEPATHOCL\nsampler.type = TILEPATHSAMPLER\ntile.multipass.enable = 0\ntile.multipass.convergencetest.threshold = 0\ntile.size = 32\ntilepathocl.devices.maxtiles = 1\nbatch.haltspp = 0\nbatch.halttime = 0\nbatch.haltthreshold = -1\nfilm.filter.type = NONE\nfilm.outputs.0.type = RGBA\nfilm.outputs.0.filename = unused.exr\nfilm.outputs.1.type = SAMPLECOUNT\nfilm.outputs.1.filename = unused-counts.exr\nfilm.outputs.2.type = ALBEDO\nfilm.outputs.2.filename = unused-albedo.exr\nfilm.outputs.3.type = AVG_SHADING_NORMAL\nfilm.outputs.3.filename = unused-normal.exr\nopencl.cpu.use = 0\nopencl.gpu.use = 1\nopencl.native.threads.count = 0\ncuda.optix.devices.select = 1\n");
                p->Set(Property("path.forceblackbackground.enable")(f.transparent!=0));
                p->Set(Property("opencl.devices.select")(device_mask));p->Set(Property("film.width")(f.width));p->Set(Property("film.height")(f.height));p->Set(Property("tilepath.sampling.aa.size")(aa));p->Set(Property("renderengine.seed")(f.sample_seed?f.sample_seed:1));
                p->Set(Property("context.cuda.cache.directory")(cache));p->Set(Property("context.cuda.cache.compiler_identity")(compiler_hash));p->Set(Property("context.cuda.cache.sdk_identity")(sdk_hash));
                render_config=luxcore::RenderConfig::Create(std::move(p),scene);session=luxcore::RenderSession::Create(render_config);session->Start();started=true;
                last_width=f.width;last_height=f.height;last_aa=aa;last_seed=f.sample_seed;last_transparent=f.transparent!=0;
            }
            diagnostics["counters"]["reused_scene_session"]=uint64_t(reuse);diagnostics["timings_ms"]["session_start_or_edit"]=elapsed(start);start=Clock::now();
#ifdef TN_LUXCORE_DIAGNOSTIC_TRACE
            trace::frame_input(frame,*scene,*render_config);
#endif
            while(!session->HasDone()) {
                if(work.cancelled()){stop();return {};}
                session->UpdateStats();
                const double count=session->GetStats()->Get(Property("stats.renderengine.total.samplecount")(0.)).Get<double>();
                if(std::isfinite(count)&&count>0){
                    const auto complete=static_cast<uint64_t>(std::min(count,static_cast<double>(work.total.load())));
                    work.completed=std::max(work.completed.load(),complete);
                }
                std::this_thread::sleep_for(std::chrono::milliseconds(10));
            }
            session->UpdateStats();
            uint64_t sdk_device_bytes=0;const auto& stats=session->GetStats();const auto& devices=stats->Get("stats.renderengine.devices");
            for(unsigned i=0;i<devices.GetSize();++i)sdk_device_bytes=sum(sdk_device_bytes,stats->Get("stats.renderengine.devices."+devices.Get<std::string>(i)+".memory.used").Get<unsigned long long>());
            diagnostics["counters"]["observed_sdk_device_bytes"]=sdk_device_bytes;
            diagnostics["timings_ms"]["render_wait"]=elapsed(start);if(work.cancelled()){stop();return {};}
            start=Clock::now();std::vector<unsigned> counts(static_cast<size_t>(pixels));session->GetFilm().GetOutput<unsigned>(luxcore::Film::OUTPUT_SAMPLECOUNT,counts.data());
            const auto [lo,hi]=std::minmax_element(counts.begin(),counts.end());const uint64_t total=std::accumulate(counts.begin(),counts.end(),uint64_t(0));
            diagnostics["counters"]["actual_min_samples"]=uint64_t(*lo);diagnostics["counters"]["actual_max_samples"]=uint64_t(*hi);diagnostics["counters"]["actual_total_samples"]=total;
            if(*lo<requested)throw std::runtime_error("Actual per-pixel sample minimum below requested quality");work.completed=total;
            std::vector<float> rgba(static_cast<size_t>(pixels)*4);session->GetFilm().GetOutput<float>(luxcore::Film::OUTPUT_RGBA,rgba.data());
            // The pinned film's RGBA color is scene-linear; alpha has an
            // independent numerator/weight channel and must not be tonemapped.
            const auto* alpha=session->GetFilm().GetChannel<float>(luxcore::Film::CHANNEL_ALPHA);
#ifdef TN_LUXCORE_DIAGNOSTIC_TRACE
            trace::film(rgba,alpha,counts,session->GetFilm());
#endif
            for(size_t i=0;i<pixels;++i)rgba[i*4+3]=alpha[i*2+1]>0?std::clamp(alpha[i*2]/alpha[i*2+1],0.f,1.f):0.f;
            if(use_denoise)denoise(rgba,f.width,f.height);
            // Film radiance averages covered and uncovered samples. Denoise in
            // that premultiplied linear domain; normalize only before AgX.
            if(f.transparent)for(size_t i=0;i<pixels;++i)for(size_t k=0;k<3;++k)
                rgba[i*4+k]=rgba[i*4+3]>0?rgba[i*4+k]/rgba[i*4+3]:0.f;
            std::vector<uint8_t> result(static_cast<size_t>(pixels)*4);
            for(size_t y=0;y<f.height;++y)for(size_t x=0;x<f.width;++x) {
                const size_t source=((f.height-y-1)*f.width+x)*4,dest=(y*f.width+x)*4;
                turninator::agx_v1::Rgba input{rgba[source],rgba[source+1],rgba[source+2],f.transparent?rgba[source+3]:1.f},mapped{};
                if(!turninator::agx_v1::map_srgb(input,0,mapped))throw std::runtime_error("Nonfinite LuxCore HDR/alpha film");
                for(size_t k=0;k<4;++k)result[dest+k]=static_cast<uint8_t>(std::lround(std::clamp(mapped[k],0.f,1.f)*255));
                if(!result[dest+3])result[dest]=result[dest+1]=result[dest+2]=0;
            }
            diagnostics["timings_ms"]["readback_and_color"]=elapsed(start);PROCESS_MEMORY_COUNTERS_EX memory{};memory.cb=sizeof(memory);if(GetProcessMemoryInfo(GetCurrentProcess(),reinterpret_cast<PROCESS_MEMORY_COUNTERS*>(&memory),sizeof(memory)))diagnostics["counters"]["observed_process_private_bytes"]=uint64_t(memory.PrivateUsage);
            if(work.cancelled()){stop();return {};}return result;
        } catch(...) {release_session();throw;}
    }
};
}
namespace tn {
uint32_t backend_id() noexcept {return TN_BACKEND_LUXCORE;}
uint32_t backend_flags() noexcept {return 0;}
std::unique_ptr<Backend> make_backend(const Config& config) {
    // Before retained module loads or SDK initialization. The loader's existing
    // import mappings are outside this backend's control; runtime/driver/SDK
    // backing allocations are included in this conservative baseline.
    if(config.host_bytes<(2ull<<30)||config.gpu_bytes<(1ull<<30))throw InvalidInput("LuxCore runtime reservation requires at least 2 GiB host and 1 GiB GPU budget");
    return std::make_unique<lux::OwnedRenderer<lux::Renderer>>(config);
}
}
