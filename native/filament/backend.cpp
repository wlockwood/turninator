#include "backend.hpp"
#include "reservation.hpp"
#include "readback.hpp"
#include <Windows.h>
#include <filament/Engine.h>
#include <filament/Renderer.h>
#include <filament/Scene.h>
#include <filament/View.h>
#include <filament/Viewport.h>
#include <filament/Camera.h>
#include <filament/SwapChain.h>
#include <filament/VertexBuffer.h>
#include <filament/IndexBuffer.h>
#include <filament/Material.h>
#include <filament/MaterialInstance.h>
#include <filament/RenderableManager.h>
#include <filament/TransformManager.h>
#include <filament/LightManager.h>
#include <filament/IndirectLight.h>
#include <filament/Texture.h>
#include <ktxreader/Ktx1Reader.h>
#include <filament/ColorGrading.h>
#include <filament/ColorSpace.h>
#include <filament/ToneMapper.h>
#include <filament/Fence.h>
#include <backend/PixelBufferDescriptor.h>
#include <backend/platforms/VulkanPlatformWindows.h>
#include <utils/EntityManager.h>
#include <math/quat.h>
#include <algorithm>
#include <array>
#include <cmath>
#include <iomanip>
#include <sstream>
#include <thread>
#include "surface.inc"
#include "diagnostic.inc"
#include "resin.inc"
#include "studio.inc"

namespace tn {
namespace {
using namespace filament;
using namespace filament::math;
constexpr float mm_to_m=0.001f;
struct Vertex { float3 position; quatf tangents; float4 build; };
static_assert(sizeof(Vertex)==44);
float3 vector3(const Json& j) {return {j.at(0).get<float>(),j.at(1).get<float>(),j.at(2).get<float>()};}
double3 meters(const double* v) {return {v[0]*0.001,v[1]*0.001,v[2]*0.001};}
std::string device_id(VkPhysicalDevice device) {
    VkPhysicalDeviceIDProperties id{};id.sType=VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES;
    VkPhysicalDeviceProperties2 p{};p.sType=VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2;p.pNext=&id;
    bluevk::vkGetPhysicalDeviceProperties2(device,&p);
    if(std::all_of(std::begin(id.deviceUUID),std::end(id.deviceUUID),[](uint8_t b){return b==0;}))throw std::runtime_error("Vulkan physical-device UUID is unavailable");
    std::ostringstream s;s<<"vulkan:"<<std::hex<<std::setfill('0')<<std::setw(4)<<p.properties.vendorID<<":"<<std::setw(4)<<p.properties.deviceID<<":";
    for(auto value:id.deviceUUID)s<<std::setw(2)<<unsigned(value);
    return s.str();
}
class Platform final : public backend::VulkanPlatformWindows {
public:
    int8_t index=-1;
    Customization getCustomization() const noexcept override { auto c=Customization{};c.gpu.index=index;return c; }
};
// The SDK's loader uses bare vulkan-1.dll. Pin its resolution to System32 before
// SDK initialization and retain the module until every Engine has drained.
struct VulkanModule {
    HMODULE handle{};
    VulkanModule() {
        wchar_t system[MAX_PATH];const auto count=GetSystemDirectoryW(system,MAX_PATH);
        if(!count||count>=MAX_PATH)throw std::runtime_error("Cannot resolve System32 Vulkan loader");
        const std::wstring path=std::wstring(system)+L"\\vulkan-1.dll";
        handle=LoadLibraryExW(path.c_str(),nullptr,LOAD_LIBRARY_SEARCH_SYSTEM32);
        if(!handle)throw std::runtime_error("System32 Vulkan loader unavailable");
        wchar_t loaded[MAX_PATH];if(!GetModuleFileNameW(handle,loaded,MAX_PATH)||_wcsicmp(path.c_str(),loaded)!=0){FreeLibrary(handle);handle=nullptr;throw std::runtime_error("Loaded Vulkan loader is outside System32");}
    }
    ~VulkanModule(){if(handle)FreeLibrary(handle);}
};
class FilamentBackend final : public tn::Backend {
    Config config;
    VulkanModule loader;
    Platform platform;
    Engine* engine{};
    Renderer* renderer{};
    filament::Scene* scene{};
    Camera* camera{};
    utils::Entity camera_entity{},model{},ink{},floor{};
    std::vector<utils::Entity> lights;
    Material *surface{},*diagnostic{},*resin{};
    ColorGrading* grading{};
    IndirectLight* environment{};
    Texture* reflections{};
    std::array<float3,9> studio_sh{};
    std::vector<MaterialInstance*> instances;
    std::vector<MaterialInstance*> lit_instances;
    std::vector<VertexBuffer*> vertex_buffers;
    std::vector<IndexBuffer*> index_buffers;
    tn::Scene prepared;
    bool dirty=true;
    std::string description;
    VkPhysicalDeviceProperties properties{};
    VkSampleCountFlags target_samples{};
    struct TargetBudget {uint32_t width,height,shadow;uint8_t samples;uint64_t bytes;};
    std::vector<TargetBudget> recent_targets;

    void drain() {if(engine){engine->flushAndWait();engine->pumpMessageQueues();}}
    void remove_entity(utils::Entity& e) {
        if(e){scene->remove(e);engine->destroy(e);utils::EntityManager::get().destroy(e);e={};}
    }
    void clear_geometry() {
        if(!engine)return;drain();remove_entity(model);remove_entity(ink);remove_entity(floor);
        for(auto entity:lights){scene->remove(entity);engine->destroy(entity);utils::EntityManager::get().destroy(entity);}lights.clear();
        for(auto* value:vertex_buffers)engine->destroy(value);vertex_buffers.clear();
        for(auto* value:index_buffers)engine->destroy(value);index_buffers.clear();
        for(auto* value:instances)engine->destroy(value);instances.clear();lit_instances.clear();
        if(environment){scene->setIndirectLight(nullptr);engine->destroy(environment);environment=nullptr;}
    }
    void budget(uint64_t host,uint64_t gpu) const {
        if(host>config.host_bytes)throw std::runtime_error("Filament expansion exceeds host byte budget");
        if(gpu>config.gpu_bytes)throw std::runtime_error("Filament expansion exceeds GPU byte budget");
    }
    uint64_t retained_target_bytes() const {
        uint64_t bytes=0;for(const auto& target:recent_targets)bytes=filament_detail::add_bytes(bytes,target.bytes);return bytes;
    }
    void geometry_budget(const tn::Scene& input) const {
        const auto peak=filament_detail::reserve(input.positions_mm.size()/3,input.triangles.size()/3,input.metadata.dump().size(),sizeof(Vertex),retained_target_bytes(),0,true);
        budget(peak.host,peak.gpu);
    }
    MaterialInstance* material(const Json& physical,bool diagram,bool floor_material=false) {
        const bool unlit=physical.value("unlit",false);
        (void)diagram;
        const bool transmissive=!unlit&&!floor_material&&physical.value("transmission",0.0f)>0;
        auto* mi=(unlit?diagnostic:transmissive?resin:surface)->createInstance();if(!mi)throw std::runtime_error("Filament material allocation failed");instances.push_back(mi);
        const auto color=vector3(physical.at("base_color_linear"));
        // Coverage opacity approximates material transmission independently of
        // background coverage; refraction remains the SDK's screen-space model.
        const float transmission=transmissive?physical.at("transmission").get<float>():0;
        mi->setParameter("baseColor",float4{color,transmissive?std::max(0.08f,1.0f-transmission*0.8f):1.0f});
        if(!unlit){
            lit_instances.push_back(mi);
            for(const auto& pair:std::array<std::pair<const char*,const char*>,6>{{{"roughness","roughness"},{"metallic","metallic"},{"anisotropy","anisotropy"},{"coat","coat"},{"coatRoughness","coat_roughness"},{"ior","ior"}}})mi->setParameter(pair.first,physical.at(pair.second).get<float>());
            if(transmissive){
                auto tint=vector3(physical.at("absorption_color_linear"));
                const float density=physical.at("absorption_density_per_mm").get<float>()*1000.0f;
                const float scatter=physical.at("scattering_per_mm").get<float>()*1000.0f;
                float3 absorption;for(size_t k=0;k<3;++k)absorption[k]=-std::log(std::max(tint[k],0.0001f))*density+scatter;
                const auto lo=vector3(prepared.metadata.at("reference_bounds_mm").at("min")),hi=vector3(prepared.metadata.at("reference_bounds_mm").at("max"));
                const float thickness=std::max(0.000001f,std::min({hi.x-lo.x,hi.y-lo.y,hi.z-lo.z})*mm_to_m);
                mi->setParameter("transmission",transmission);mi->setParameter("absorption",absorption);mi->setParameter("thickness",thickness);
            }
            const auto& a=prepared.metadata.at("appearance");const auto& filter=prepared.metadata.at("layer_filter");
            const bool enabled=!floor_material && prepared.metadata.at("geometry_kind")=="original" && a.at("layer_appearance").get<bool>();
            mi->setParameter("layer",float4{a.at("first_layer_height_mm").get<float>(),a.at("layer_height_mm").get<float>(),a.at("groove_depth_mm").get<float>(),enabled?1.0f:0.0f});
            mi->setParameter("buildMinZ",prepared.metadata.at("reference_bounds_mm").at("min").at(2).get<float>());
            mi->setParameter("filtering",float4{1,filter.at("enabled").get<bool>()?1.0f:0.0f,filter.at("footprint_diagonal_factor").get<float>(),0});
            mi->setParameter("worldBuildGradient",float3{0,0,1});mi->setParameter("cameraPosition",float3{0,0,1});mi->setParameter("orthoViewDirection",float3{0,0,1});
        }
        return mi;
    }
    utils::Entity renderable(VertexBuffer* vb,IndexBuffer* ib,const Box& bounds,const std::vector<std::pair<MaterialInstance*,std::pair<uint32_t,uint32_t>>>& primitives,bool shadows) {
        auto entity=utils::EntityManager::get().create();RenderableManager::Builder builder(primitives.size());builder.boundingBox(bounds).culling(false).castShadows(shadows).receiveShadows(shadows);
        for(size_t p=0;p<primitives.size();++p)builder.material(p,primitives[p].first).geometry(p,RenderableManager::PrimitiveType::TRIANGLES,vb,ib,primitives[p].second.first,primitives[p].second.second);
        try{if(builder.build(*engine,entity)!=RenderableManager::Builder::Success)throw std::runtime_error("Filament renderable creation failed");scene->addEntity(entity);return entity;}catch(...){engine->destroy(entity);utils::EntityManager::get().destroy(entity);throw;}
    }
    utils::Entity geometry(const std::vector<Vertex>& vertices,const std::vector<uint32_t>& indices,const std::vector<std::pair<MaterialInstance*,std::pair<uint32_t,uint32_t>>>& primitives,const std::vector<std::pair<MaterialInstance*,std::pair<uint32_t,uint32_t>>>& ink_primitives={}) {
        struct UploadDrain{FilamentBackend* self;~UploadDrain() noexcept{try{self->drain();}catch(...){}}} upload_drain{this};
        auto* vb=VertexBuffer::Builder().vertexCount(uint32_t(vertices.size())).bufferCount(1)
            .attribute(VertexAttribute::POSITION,0,VertexBuffer::AttributeType::FLOAT3,offsetof(Vertex,position),sizeof(Vertex))
            .attribute(VertexAttribute::TANGENTS,0,VertexBuffer::AttributeType::FLOAT4,offsetof(Vertex,tangents),sizeof(Vertex))
            .attribute(VertexAttribute::CUSTOM0,0,VertexBuffer::AttributeType::FLOAT4,offsetof(Vertex,build),sizeof(Vertex)).build(*engine);
        if(!vb)throw std::runtime_error("Filament vertex allocation failed");vertex_buffers.push_back(vb);
        // SDK callbacks own copies even if an SDK exception interrupts this scope.
        // Normal completion drains both callbacks before returning.
        auto vertex_copy=std::make_unique<std::vector<Vertex>>(vertices);
        VertexBuffer::BufferDescriptor vertex_data(vertex_copy->data(),vertex_copy->size()*sizeof(Vertex),[](void*,size_t,void* p){delete static_cast<std::vector<Vertex>*>(p);},vertex_copy.get());vertex_copy.release();vb->setBufferAt(*engine,0,std::move(vertex_data));
        auto* ib=IndexBuffer::Builder().indexCount(uint32_t(indices.size())).bufferType(IndexBuffer::IndexType::UINT).build(*engine);
        if(!ib)throw std::runtime_error("Filament index allocation failed");index_buffers.push_back(ib);
        auto index_copy=std::make_unique<std::vector<uint32_t>>(indices);
        IndexBuffer::BufferDescriptor index_data(index_copy->data(),index_copy->size()*sizeof(uint32_t),[](void*,size_t,void* p){delete static_cast<std::vector<uint32_t>*>(p);},index_copy.get());index_copy.release();ib->setBuffer(*engine,std::move(index_data));
        float3 lo=vertices.front().position,hi=lo;for(const auto& v:vertices){lo=min(lo,v.position);hi=max(hi,v.position);}
        const Box bounds{(hi+lo)*0.5f,max((hi-lo)*0.5f,float3{1e-8f})};
        if(!ink_primitives.empty())ink=renderable(vb,ib,bounds,ink_primitives,true);
        return primitives.empty()?utils::Entity{}:renderable(vb,ib,bounds,primitives,true);
    }
    void realize(Work& work) {
        if(!dirty)return;clear_geometry();drain();if(work.cancelled())return;
        const uint64_t n=prepared.positions_mm.size()/3,t=prepared.triangles.size()/3;
        geometry_budget(prepared);
        const auto& a=prepared.metadata.at("appearance");const bool diagram=a.at("studio_preset")=="diagram";
        if(t){
            std::vector<Vertex> vertices;vertices.reserve(size_t(n));
            for(size_t i=0;i<n;++i){
                if((i%8192)==0&&work.cancelled())return;
                float3 normal{prepared.normals[i*3],prepared.normals[i*3+1],prepared.normals[i*3+2]};
                const double length=std::sqrt(double(normal.x)*normal.x+double(normal.y)*normal.y+double(normal.z)*normal.z);
                if(!(length>0))throw std::runtime_error("Filament requires nonzero aligned normals");
                normal=float3(double3(normal)/length);const float3 tangent=normalize(cross(std::abs(normal.z)<0.999f?float3{0,0,1}:float3{0,1,0},normal));
                const auto quaternion=mat3f::packTangentFrame(mat3f{tangent,cross(normal,tangent),normal});
                vertices.push_back({float3{prepared.positions_mm[i*3],prepared.positions_mm[i*3+1],prepared.positions_mm[i*3+2]}*mm_to_m,quaternion,{prepared.build_positions_mm[i*3],prepared.build_positions_mm[i*3+1],prepared.build_positions_mm[i*3+2],normal.z}});
            }
            std::vector<uint32_t> indices(size_t(t)*3);
            std::vector<std::pair<MaterialInstance*,std::pair<uint32_t,uint32_t>>> primitives,ink_primitives;
            const auto& materials=prepared.metadata.at("materials");
            std::vector<uint32_t> counts(materials.size()),starts(materials.size()),offsets(materials.size());
            for(auto m:prepared.triangle_materials)counts[m]+=3;
            for(size_t m=1;m<counts.size();++m)starts[m]=starts[m-1]+counts[m-1];offsets=starts;
            for(size_t k=0;k<t;++k){if((k%8192)==0&&work.cancelled())return;auto& offset=offsets[prepared.triangle_materials[k]];for(size_t j=0;j<3;++j)indices[offset++]=prepared.triangles[k*3+j];}
            for(size_t m=0;m<materials.size();++m)if(counts[m]){auto& list=materials[m].value("unlit",false)?ink_primitives:primitives;list.push_back({material(materials[m],diagram),{starts[m],counts[m]}});}
            if(!work.cancelled())model=geometry(vertices,indices,primitives,ink_primitives);
        }
        {
            const auto lo=vector3(prepared.metadata.at("reference_bounds_mm").at("min"))*mm_to_m,hi=vector3(prepared.metadata.at("reference_bounds_mm").at("max"))*mm_to_m;
            const float extent=std::max({hi.x-lo.x,hi.y-lo.y,hi.z-lo.z,0.001f});const auto center=(lo+hi)*0.5f;
            const bool dark=a.at("studio_preset")=="dark";
            environment=IndirectLight::Builder().irradiance(3,studio_sh.data()).reflections(reflections).intensity(dark?0.12f:0.35f).build(*engine);scene->setIndirectLight(environment);
            // Unit exposure intentionally uses unit-scale irradiance rather than
            // outdoor100000lux values intended for the default EV15 camera.
            const std::array<float3,3> directions{{{2.5f,3,-4},{-3,1,-2},{0,-3,-3}}};const std::array<float,3> powers{{5.5f,1.8f,4.0f}};
            for(size_t i=0;i<directions.size();++i){auto e=utils::EntityManager::get().create();lights.push_back(e);if(LightManager::Builder(LightManager::Type::DIRECTIONAL).direction(normalize(directions[i])).color({1,1,1}).intensity(powers[i]).castShadows(i==0).build(*engine,e)!=LightManager::Builder::Success)throw std::runtime_error("Filament studio light creation failed");scene->addEntity(e);}
            Json fm=prepared.metadata.at("materials").empty()?scene_document_dummy():prepared.metadata.at("materials").at(0);
            fm["base_color_linear"]=dark?Json::array({0.025,0.03,0.04}):Json::array({0.18,0.20,0.24});fm["roughness"]=0.7;fm["metallic"]=0;fm["anisotropy"]=0;fm["coat"]=0;
            const float r=extent*6,z=lo.z-extent*0.001f;const quatf q=mat3f::packTangentFrame(mat3f{});
            std::vector<Vertex> fv{{{center.x-r,center.y-r,z},q,{0,0,0,1}},{{center.x+r,center.y-r,z},q,{0,0,0,1}},{{center.x+r,center.y+r,z},q,{0,0,0,1}},{{center.x-r,center.y+r,z},q,{0,0,0,1}}};
            if(!diagram)floor=geometry(fv,{0,1,2,0,2,3},{{material(fm,false,true),{0,6}}});
        }
        drain();if(!work.cancelled())dirty=false;
    }
    static Json scene_document_dummy(){return Json{{"base_color_linear",{1,1,1}},{"roughness",0.7},{"metallic",0},{"anisotropy",0},{"coat",0},{"coat_roughness",0.1},{"ior",1.46}};}
public:
    explicit FilamentBackend(const Config& c):config(c){}
    void initialize() {
        budget(uint64_t(256)*1024*1024,uint64_t(64)*1024*1024);
        // Explicit selection is preflighted before Filament creates any device.
        const auto& selected=config.document.at("selection").at("device");
        if(!selected.is_null()){
            if(!bluevk::initialize())throw std::runtime_error("Vulkan initialization failed");
            VkApplicationInfo ai{};ai.sType=VK_STRUCTURE_TYPE_APPLICATION_INFO;ai.apiVersion=VK_API_VERSION_1_1;
            VkInstanceCreateInfo ci{};ci.sType=VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO;ci.pApplicationInfo=&ai;VkInstance instance{};
            if(bluevk::vkCreateInstance(&ci,nullptr,&instance)!=VK_SUCCESS)throw std::runtime_error("Vulkan selection preflight failed");
            bluevk::bindInstance(instance);struct InstanceGuard{VkInstance value;~InstanceGuard(){bluevk::vkDestroyInstance(value,nullptr);}} instance_guard{instance};uint32_t count=0;
            if(bluevk::vkEnumeratePhysicalDevices(instance,&count,nullptr)!=VK_SUCCESS||count>128)throw std::runtime_error("Vulkan device enumeration failed or exceeds128-device limit");std::vector<VkPhysicalDevice> devices(count);
            if(bluevk::vkEnumeratePhysicalDevices(instance,&count,devices.data())!=VK_SUCCESS)throw std::runtime_error("Vulkan physical-device list changed during selection");
            for(size_t i=0;i<devices.size()&&i<128;++i)if(device_id(devices[i])==selected.get<std::string>())platform.index=int8_t(i);
            if(platform.index<0)throw std::runtime_error("Explicit Filament Vulkan device is unavailable");
        }
        Engine::Config sdk_config;sdk_config.jobSystemThreadCount=4;sdk_config.resourceAllocatorCacheMaxAge=1;sdk_config.enableMultipleDirectionalLights=true;
        engine=Engine::create(Engine::Backend::VULKAN,&platform,nullptr,&sdk_config);if(!engine)throw std::runtime_error("Filament Vulkan Engine creation failed");
        const auto physical=platform.getPhysicalDevice();if(!physical)throw std::runtime_error("Filament did not report its physical device");
        bluevk::vkGetPhysicalDeviceProperties(physical,&properties);const auto actual=device_id(physical);
        target_samples=properties.limits.framebufferColorSampleCounts&properties.limits.framebufferDepthSampleCounts&properties.limits.sampledImageColorSampleCounts&properties.limits.sampledImageDepthSampleCounts;
        for(const auto format:{VK_FORMAT_R16G16B16A16_SFLOAT,VK_FORMAT_D32_SFLOAT}){
            VkImageFormatProperties supported{};const auto usage=VK_IMAGE_USAGE_SAMPLED_BIT|(format==VK_FORMAT_R16G16B16A16_SFLOAT?VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT:VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT);
            const auto status=bluevk::vkGetPhysicalDeviceImageFormatProperties(physical,format,VK_IMAGE_TYPE_2D,VK_IMAGE_TILING_OPTIMAL,usage,0,&supported);
            if(status!=VK_SUCCESS)throw std::runtime_error("Filament required HDR/depth attachment format is unavailable");target_samples&=supported.sampleCounts;
        }
        if(!selected.is_null()&&selected.get<std::string>()!=actual)throw std::runtime_error("Filament selected a different physical device");
        using Info=backend::Platform::DeviceInfoType;
        const auto name=platform.getDeviceInfo(Info::VULKAN_DEVICE_NAME,nullptr),driver_name=platform.getDeviceInfo(Info::VULKAN_DRIVER_NAME,nullptr),driver_info=platform.getDeviceInfo(Info::VULKAN_DRIVER_INFO,nullptr);
        if(name.empty()||driver_name.empty()||driver_info.empty())throw std::runtime_error("Filament actual device/driver identity is incomplete");
        description=Json{{"schema_version",1},{"backend","filament"},{"sdk_version","1.78.0+c1dc4e444784850d6dc891a1fb38747c30b85463"},{"device_id",actual},{"device_name",name.c_str()},{"api","Vulkan "+std::to_string(VK_VERSION_MAJOR(properties.apiVersion))+"."+std::to_string(VK_VERSION_MINOR(properties.apiVersion))+"."+std::to_string(VK_VERSION_PATCH(properties.apiVersion))},{"driver",std::string(driver_name.c_str())+" "+driver_info.c_str()}}.dump();
        scene=engine->createScene();renderer=engine->createRenderer();camera_entity=utils::EntityManager::get().create();camera=engine->createCamera(camera_entity);camera->setExposure(1.0f);
        surface=Material::Builder().package(tn_surface_package,sizeof(tn_surface_package)).build(*engine);diagnostic=Material::Builder().package(tn_diagnostic_package,sizeof(tn_diagnostic_package)).build(*engine);
        resin=Material::Builder().package(tn_resin_package,sizeof(tn_resin_package)).build(*engine);
        AgxToneMapper tone(AgxToneMapper::AgxLook::NONE);grading=ColorGrading::Builder().quality(ColorGrading::QualityLevel::HIGH).toneMapper(&tone).exposure(0).outputColorSpace(color::Rec709-color::sRGB-color::D65).build(*engine);
        auto bundle=std::make_unique<image::Ktx1Bundle>(tn_studio_package,uint32_t(sizeof(tn_studio_package)));
        if(!bundle->isCubemap()||bundle->getNumMipLevels()!=7||!bundle->getSphericalHarmonics(studio_sh.data()))throw std::runtime_error("Compiled Filament studio environment is incomplete");
        reflections=ktxreader::Ktx1Reader::createTexture(engine,bundle.release(),false);
        if(!scene||!renderer||!camera||!surface||!diagnostic||!resin||!grading||!reflections)throw std::runtime_error("Filament context resource creation failed");
        drain();
    }
    ~FilamentBackend() noexcept override {
        if(!engine)return;
        try {clear_geometry();if(surface)engine->destroy(surface);if(diagnostic)engine->destroy(diagnostic);if(resin)engine->destroy(resin);if(reflections)engine->destroy(reflections);if(grading)engine->destroy(grading);if(camera){engine->destroyCameraComponent(camera_entity);utils::EntityManager::get().destroy(camera_entity);}if(scene)engine->destroy(scene);if(renderer)engine->destroy(renderer);drain();Engine::destroy(&engine);}catch(...){try{Engine::destroy(&engine);}catch(...){}}
    }
    std::string describe() override{return description;}
    void upload(const tn::Scene& input,Work&) override {
        // Release the prior scene/buffers before charging and copying the new
        // snapshot, retaining only SDK target-cache reservations. This avoids
        // retaining large vector capacities after a large-to-small replacement.
        clear_geometry();drain();prepared=tn::Scene{};dirty=true;
        geometry_budget(input);prepared=input;
    }
    std::vector<uint8_t> render(const Frame& frame,Work& work) override {
        const auto& f=frame.parameters;const auto& quality=frame.quality.at("quality");const auto aa=quality.at("aa").get<std::string>(),shadow=quality.at("shadows").get<std::string>();
        const uint8_t samples=aa=="low"?1:4;const uint32_t shadow_size=shadow=="low"?1024:shadow=="medium"?2048:4096;
        if(!(target_samples&samples))throw std::runtime_error("Requested Filament MSAA"+std::to_string(samples)+" is unsupported by actual HDR16F/depth32F formats (sample mask="+std::to_string(target_samples)+")");
        const uint64_t pixels=uint64_t(f.width)*f.height;
        const uint64_t target_bytes=filament_detail::add_bytes(filament_detail::multiply_bytes(pixels,160+uint64_t(samples)*12),filament_detail::multiply_bytes(uint64_t(shadow_size)*shadow_size,8));
        uint64_t retained_bytes=0;
        for(const auto& target:recent_targets)if(target.width!=f.width||target.height!=f.height||target.shadow!=shadow_size||target.samples!=samples)retained_bytes=filament_detail::add_bytes(retained_bytes,target.bytes);
        // WDDM/SDK host commitment substantially exceeds explicit CPU vectors
        // at4096. Conservatively reserve the GPU allocation bound again as host
        // backing, in addition to source/readback/CPU staging and baseline.
        const auto reservation=filament_detail::reserve(prepared.positions_mm.size()/3,prepared.triangles.size()/3,prepared.metadata.dump().size(),sizeof(Vertex),filament_detail::add_bytes(target_bytes,retained_bytes),pixels,dirty);
        budget(reservation.host,reservation.gpu);
        work.total=4;work.completed=0;realize(work);work.completed=1;if(work.cancelled())return {};
        const bool diagram=prepared.metadata.at("appearance").at("studio_preset")=="diagram";
        const auto cp=meters(f.camera_position_mm),ct=meters(f.camera_target_mm);const double3 up{f.camera_up[0],f.camera_up[1],f.camera_up[2]};camera->lookAt(cp,ct,up);
        const double aspect=double(f.width)/f.height;
        if(f.projection==1){const double half=f.vertical*0.0005;camera->setProjection(Camera::Projection::ORTHO,-half*aspect,half*aspect,-half,half,f.near_mm*0.001,f.far_mm*0.001);}else camera->setProjection(f.vertical,aspect,f.near_mm*0.001,f.far_mm*0.001,Camera::Fov::VERTICAL);
        mat4f transform;for(size_t col=0;col<4;++col)for(size_t row=0;row<4;++row)transform[col][row]=float(f.model_to_world[row*4+col]*(col==3&&row<3?0.001:1));
        const double determinant=dot(double3(transform[0].xyz),cross(double3(transform[1].xyz),double3(transform[2].xyz)));
        if(!std::isfinite(determinant)||std::abs(determinant)<1e-12)throw std::runtime_error("Filament model transform is singular or outside the supported numeric range");
        if(model)engine->getTransformManager().setTransform(engine->getTransformManager().getInstance(model),transform);
        if(ink)engine->getTransformManager().setTransform(engine->getTransformManager().getInstance(ink),transform);
        const auto gradient=transpose(inverse(transform.upperLeft()))*float3{0,0,1};
        const float footprint=f.projection==1?float(f.vertical/double(f.height)):float(2*std::tan(f.vertical*3.141592653589793/360)/f.height*1000);
        for(auto* mi:lit_instances){mi->setParameter("worldBuildGradient",gradient);mi->setParameter("cameraPosition",float3(cp));mi->setParameter("orthoViewDirection",float3(normalize(cp-ct)));const auto& filter=prepared.metadata.at("layer_filter");mi->setParameter("filtering",float4{footprint,filter.at("enabled").get<bool>()?1.0f:0.0f,filter.at("footprint_diagonal_factor").get<float>(),f.projection==0?1.0f:0.0f});}
        if(floor){if(f.transparent)scene->remove(floor);else scene->addEntity(floor);}
        for(auto e:lights){LightManager::ShadowOptions options;options.mapSize=shadow_size;options.constantBias=0.000002f;options.normalBias=1;options.stable=true;options.shadowFarHint=float(f.far_mm*0.001);options.shadowNearHint=float(f.near_mm*0.001);engine->getLightManager().setShadowOptions(engine->getLightManager().getInstance(e),options);}
        struct Readback {std::vector<uint8_t> pixels;std::atomic<bool> complete=false;explicit Readback(size_t n):pixels(n){}};
        auto readback=std::make_shared<Readback>(size_t(pixels)*4);
        auto* view=engine->createView();auto* swap=engine->createSwapChain(f.width,f.height,SwapChain::CONFIG_TRANSPARENT);
        struct FrameResources{Engine* e;View* v;SwapChain* s;~FrameResources() noexcept{try{e->flushAndWait();e->pumpMessageQueues();e->destroy(v);e->destroy(s);}catch(...){}}} resources{engine,view,swap};
        view->setScene(scene);view->setCamera(camera);view->setViewport({0,0,f.width,f.height});view->setBlendMode(View::BlendMode::TRANSLUCENT);view->setPostProcessingEnabled(true);view->setColorGrading(grading);view->setDithering(View::Dithering::NONE);view->setAntiAliasing(aa=="high"?View::AntiAliasing::FXAA:View::AntiAliasing::NONE);
        MultiSampleAntiAliasingOptions msaa;msaa.enabled=samples>1;msaa.sampleCount=samples;msaa.customResolve=false;view->setMultiSampleAntiAliasingOptions(msaa);
        TemporalAntiAliasingOptions taa;taa.enabled=false;view->setTemporalAntiAliasingOptions(taa);
        AmbientOcclusionOptions ao;ao.enabled=!diagram;ao.radius=0.002f;ao.bias=0.00001f;ao.resolution=1;ao.quality=shadow=="low"?QualityLevel::LOW:shadow=="medium"?QualityLevel::MEDIUM:QualityLevel::HIGH;view->setAmbientOcclusionOptions(ao);view->setShadowType(ShadowType::PCF);
        Renderer::ClearOptions clear;clear.clear=true;clear.discard=false;const bool dark=prepared.metadata.at("appearance").at("studio_preset")=="dark";clear.clearColor=f.transparent?float4{0}:dark?float4{0.025f,0.03f,0.04f,1}:float4{0.22f,0.25f,0.3f,1};renderer->setClearOptions(clear);
        if(!renderer->beginFrame(swap,0))throw std::runtime_error("Filament refused deterministic offscreen frame");
        renderer->render(view);work.completed=2;
        auto* retained=new std::shared_ptr<Readback>(readback);
        renderer->readPixels(0,0,f.width,f.height,backend::PixelBufferDescriptor(readback->pixels.data(),readback->pixels.size(),backend::PixelDataFormat::RGBA,backend::PixelDataType::UBYTE,1,0,0,0,[](void*,size_t,void* p){std::unique_ptr<std::shared_ptr<Readback>> owner(static_cast<std::shared_ptr<Readback>*>(p));(*owner)->complete.store(true,std::memory_order_release);},retained));renderer->endFrame();
        const auto fence_status=Fence::waitAndDestroy(engine->createFence());
        drain();while(!readback->complete.load(std::memory_order_acquire)){engine->pumpMessageQueues();std::this_thread::yield();}
        if(fence_status!=Fence::FenceStatus::CONDITION_SATISFIED)throw std::runtime_error("Filament GPU completion fence failed; readback is invalid");work.completed=3;
        recent_targets.erase(std::remove_if(recent_targets.begin(),recent_targets.end(),[&](const auto& t){return t.width==f.width&&t.height==f.height&&t.shadow==shadow_size&&t.samples==samples;}),recent_targets.end());
        recent_targets.insert(recent_targets.begin(),TargetBudget{f.width,f.height,shadow_size,samples,target_bytes});if(recent_targets.size()>3)recent_targets.pop_back();
        // Filament final color is display-encoded premultiplied RGBA. Normalize
        // in that same documented producer domain; never encode sRGB twice.
        auto& result=readback->pixels;
        if(!f.transparent)for(size_t i=3;i<result.size();i+=4)if(result[i]!=255)throw std::runtime_error("Filament opaque readback has invalid alpha (requested MSAA="+std::to_string(samples)+", combined sampled HDR/depth support mask="+std::to_string(target_samples)+")");
        filament_detail::normalize_readback(result);
        work.completed=4;return std::move(result);
    }
};
}
uint32_t backend_id() noexcept{return TN_BACKEND_FILAMENT;}
uint32_t backend_flags() noexcept{return 0;}
std::unique_ptr<Backend> make_backend(const Config& config){auto p=std::make_unique<FilamentBackend>(config);p->initialize();return p;}
}
