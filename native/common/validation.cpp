#include "backend.hpp"
#include <algorithm>
#include <cmath>
#include <filesystem>
#include <limits>
#include <set>
#include <string_view>
namespace tn {
namespace {
constexpr uint64_t max_json=1048576;
void require(bool b,const char* message) { if(!b) throw InvalidInput(message); }
template<class T> void structure(const T* p) { require(p && p->size==sizeof(T) && p->version==1,"invalid ABI struct size/version"); }
Json parse(const uint8_t* data,uint64_t bytes) {
    require(data && bytes && bytes<=max_json,"invalid JSON span/byte bound");
    std::vector<std::set<std::string>> keys;
    try {
        auto result=Json::parse(data,data+static_cast<size_t>(bytes),[&](int depth,Json::parse_event_t event,Json& value){
            require(depth<=64,"JSON nesting limit");
            if(event==Json::parse_event_t::object_start) keys.emplace_back();
            if(event==Json::parse_event_t::key) require(keys.back().insert(value.get<std::string>()).second,"duplicate JSON key");
            if(event==Json::parse_event_t::object_end) keys.pop_back();
            return true;
        });
        require(result.is_object(),"JSON root must be object"); return result;
    } catch(const Json::exception&) { throw InvalidInput("invalid UTF-8/schema1 JSON"); }
}
void shape(const Json& j,std::initializer_list<const char*> required,std::initializer_list<const char*> optional={}) {
    require(j.is_object(),"expected schema1 object");
    for(auto key:required) require(j.contains(key),"missing schema1 field");
    for(auto it=j.begin();it!=j.end();++it) {
        const auto matches=[&](const char* key){return it.key()==key;};
        require(std::any_of(required.begin(),required.end(),matches)||std::any_of(optional.begin(),optional.end(),matches),"unknown schema1 field");
    }
}
uint64_t integer(const Json& j) { require(j.is_number_unsigned(),"expected unsigned integer"); return j.get<uint64_t>(); }
void schema(const Json& j) { require(integer(j.at("schema_version"))==1,"unsupported schema version"); }
double number(const Json& j,double min=-std::numeric_limits<double>::max(),double max=std::numeric_limits<double>::max()) {
    require(j.is_number(),"expected number"); const double value=j.get<double>();
    require(std::isfinite(value)&&value>=min&&value<=max,"numeric field out of range"); return value;
}
void boolean(const Json& j) { require(j.is_boolean(),"expected boolean"); }
std::string text(const Json& j,size_t max=256) {
    require(j.is_string(),"expected string"); const auto s=j.get<std::string>();
    require(!s.empty()&&s.size()<=max,"string length limit");
    for(size_t i=0;i<s.size();++i) {
        const auto b=static_cast<unsigned char>(s[i]);
        require(b>=32&&b!=127&&!(b==0xc2&&i+1<s.size()&&static_cast<unsigned char>(s[i+1])>=0x80&&static_cast<unsigned char>(s[i+1])<=0x9f),"control character in string");
    }
    return s;
}
void vector3(const Json& j,double min=-std::numeric_limits<double>::max(),double max=std::numeric_limits<double>::max()) {
    require(j.is_array()&&j.size()==3,"expected three components"); for(const auto& n:j) number(n,min,max);
}
void quality(const Json& j,const std::string& backend) {
    if(backend=="filament") {
        shape(j,{"kind","aa","shadows"}); require(text(j.at("kind"))==backend,"quality backend mismatch");
        for(auto key:{"aa","shadows"}) { const auto s=text(j.at(key)); require(s=="low"||s=="medium"||s=="high","unknown raster quality"); }
    } else {
        shape(j,{"kind","samples","denoise"}); require(text(j.at("kind"))==backend,"quality backend mismatch");
        const auto n=integer(j.at("samples")); require(n>=1&&n<=4096,"sample count out of range"); boolean(j.at("denoise"));
    }
}
void appearance(const Json& a) {
    shape(a,{"color","roughness","material_preset","studio_preset","resin_clarity","resin_cloudiness","layer_appearance","first_layer_height_mm","layer_height_mm","groove_depth_mm","display_rotation_degrees"});
    vector3(a.at("color"),0,1); vector3(a.at("display_rotation_degrees"));
    for(auto k:{"roughness","resin_clarity","resin_cloudiness"}) number(a.at(k),0,1);
    text(a.at("material_preset")); text(a.at("studio_preset")); boolean(a.at("layer_appearance"));
    require(number(a.at("first_layer_height_mm"))>0&&number(a.at("layer_height_mm"))>0,"layer height must be positive");
    number(a.at("groove_depth_mm"),0);
}
void material(const Json& m) {
    shape(m,{"name","base_color_linear","absorption_color_linear","roughness","metallic","anisotropy","coat","coat_roughness","transmission","ior","absorption_density_per_mm","scattering_per_mm","scattering_anisotropy"});
    text(m.at("name")); vector3(m.at("base_color_linear"),0,1); vector3(m.at("absorption_color_linear"),0,1);
    for(auto k:{"roughness","metallic","coat","coat_roughness","transmission"}) number(m.at(k),0,1);
    number(m.at("anisotropy"),-1,1); number(m.at("scattering_anisotropy"),-1,1);
    const auto ior=static_cast<float>(number(m.at("ior"),0));
    require(std::isfinite(ior)&&ior>0,"IOR must be positive finite f32");
    for(auto key:{"absorption_density_per_mm","scattering_per_mm"}) require(std::isfinite(static_cast<float>(number(m.at(key),0))),"coefficient must be finite f32");
}
template<class T> void copy(std::vector<T>& v,const T* p,size_t n) { if(n) v.assign(p,p+n); }
}
Config copy_config(const tn_config* raw) {
    structure(raw); require(raw->reserved==0,"config reserved field");
    Config c; c.document=parse(raw->json,raw->json_bytes); auto& j=c.document;
    shape(j,{"schema_version","selection","identity","limits"}); schema(j);
    const auto& selection=j.at("selection"); shape(selection,{"backend","device","quality"});
    c.backend=text(selection.at("backend")); require(c.backend=="filament"||c.backend=="luxcore","unknown backend");
    require(backend_id()==TN_BACKEND_FIXTURE || static_cast<uint32_t>(c.backend=="filament"?TN_BACKEND_FILAMENT:TN_BACKEND_LUXCORE)==backend_id(),"bridge backend mismatch");
    if(!selection.at("device").is_null()) text(selection.at("device"),4096);
    quality(selection.at("quality"),c.backend);
    const auto& id=j.at("identity");
    shape(id,{"backend","adapter_recipe_version","runtime_digest","color_digest","device","api","driver"},{"bridge_digest","shader_digest","material_digest","denoiser_digest"});
    require(text(id.at("backend"))==c.backend,"identity backend mismatch");
    const auto recipe=integer(id.at("adapter_recipe_version"));
    require(recipe>0&&recipe<=UINT32_MAX,"invalid adapter recipe version");
    for(auto k:{"runtime_digest","color_digest","device","api","driver"}) text(id.at(k),4096);
    for(auto k:{"bridge_digest","shader_digest","material_digest","denoiser_digest"}) if(id.contains(k)) text(id.at(k),4096);
    const auto& limits=j.at("limits");
    shape(limits,{"max_source_vertices","max_source_triangles","max_scene_bytes","max_output_side","host_bytes","gpu_bytes","scratch_bytes"});
    for(const auto& v:limits) integer(v);
    c.host_bytes=integer(limits.at("host_bytes")); c.gpu_bytes=integer(limits.at("gpu_bytes"));
    c.max_scene_bytes=integer(limits.at("max_scene_bytes")); const auto side=integer(limits.at("max_output_side"));
    require(side>=1&&side<=8000&&c.max_scene_bytes&&c.host_bytes&&c.gpu_bytes&&side==raw->max_output_side&&c.host_bytes==raw->host_bytes&&c.gpu_bytes==raw->gpu_bytes,"config budget fields mismatch/range");
    c.max_output_side=static_cast<uint32_t>(side); return c;
}
std::shared_ptr<const Scene> copy_scene(const tn_scene* raw,const Config& c) {
    structure(raw);
    require(raw->vertex_count<=2000000&&raw->triangle_count<=4000000,"derived geometry capacity");
    require((!raw->vertex_count||(raw->positions_mm&&raw->normals&&raw->build_positions_mm))&&(!raw->triangle_count||(raw->triangles&&raw->triangle_materials)),"null geometry span");
    const uint64_t geometry=uint64_t(raw->vertex_count)*36+uint64_t(raw->triangle_count)*16;
    require(raw->json_bytes<=std::min(c.max_scene_bytes,max_json)&&geometry<=c.gpu_bytes&&raw->json_bytes<=c.host_bytes&&geometry<=c.host_bytes-raw->json_bytes,"scene host/GPU/metadata budget");
    auto s=std::make_shared<Scene>(); s->revision=raw->revision; s->metadata=parse(raw->json,raw->json_bytes);
    const auto& j=s->metadata;
    shape(j,{"schema_version","appearance","materials","reference_bounds_mm","geometry_kind","layer_filter","color"}); schema(j);
    appearance(j.at("appearance")); const auto kind=text(j.at("geometry_kind"));
    require(kind=="original"||kind=="beads"||kind=="stepped","unknown geometry kind");
    require(!j.at("appearance").at("layer_appearance").get<bool>()||kind=="original","layer appearance requires original geometry");
    const auto& bounds=j.at("reference_bounds_mm"); shape(bounds,{"min","max"}); vector3(bounds.at("min")); vector3(bounds.at("max"));
    for(size_t i=0;i<3;++i) require(number(bounds.at("min")[i])<=number(bounds.at("max")[i]),"unordered bounds");
    const auto& filter=j.at("layer_filter"); shape(filter,{"enabled","fade_start_pixels","fade_end_pixels","footprint_diagonal_factor"}); boolean(filter.at("enabled"));
    require(number(filter.at("fade_start_pixels"))==2&&number(filter.at("fade_end_pixels"))==4&&number(filter.at("footprint_diagonal_factor"))==1.4142135623730951,"unsupported layer filter recipe");
    const auto& color=j.at("color"); shape(color,{"schema_version","exposure_ev","tone_map","output_transfer","output_alpha"}); schema(color);
    require(number(color.at("exposure_ev"))==0&&text(color.at("output_transfer"))=="srgb"&&text(color.at("output_alpha"))=="straight"&&text(color.at("tone_map"))==(c.backend=="filament"?"filament-agx-v1":"turninator-agx-v1"),"unsupported color recipe");
    const auto& materials=j.at("materials"); require(materials.is_array()&&materials.size()<=256,"material count"); for(const auto& m:materials) material(m);
    const size_t n=size_t(raw->vertex_count)*3;
    for(size_t i=0;i<n;++i) require(std::isfinite(raw->positions_mm[i])&&std::isfinite(raw->normals[i])&&std::isfinite(raw->build_positions_mm[i]),"nonfinite geometry");
    for(size_t i=0;i<size_t(raw->triangle_count)*3;++i) require(raw->triangles[i]<raw->vertex_count,"triangle index out of range");
    for(size_t i=0;i<raw->triangle_count;++i) require(raw->triangle_materials[i]<materials.size(),"material index out of range");
    copy(s->positions_mm,raw->positions_mm,n); copy(s->normals,raw->normals,n); copy(s->build_positions_mm,raw->build_positions_mm,n);
    copy(s->triangles,raw->triangles,size_t(raw->triangle_count)*3); copy(s->triangle_materials,raw->triangle_materials,raw->triangle_count);
    return s;
}
Frame copy_frame(const tn_frame* raw,const Config& c,uint64_t revision) {
    structure(raw); require(raw->geometry_revision==revision&&!raw->reserved&&raw->projection<=1&&raw->transparent<=1,"invalid frame/revision");
    require(raw->width&&raw->height&&raw->width<=c.max_output_side&&raw->height<=c.max_output_side,"frame dimensions");
    const auto bytes=uint64_t(raw->width)*raw->height*4;
    require(bytes<=c.host_bytes&&bytes<=c.gpu_bytes,"frame byte budget");
    for(size_t i=0;i<16;++i) require(std::isfinite(raw->model_to_world[i]),"nonfinite matrix");
    require(raw->model_to_world[12]==0&&raw->model_to_world[13]==0&&raw->model_to_world[14]==0&&raw->model_to_world[15]==1,"matrix must be affine");
    double direction[3];
    for(size_t i=0;i<3;++i) { require(std::isfinite(raw->camera_position_mm[i])&&std::isfinite(raw->camera_target_mm[i])&&std::isfinite(raw->camera_up[i]),"nonfinite camera"); direction[i]=raw->camera_position_mm[i]-raw->camera_target_mm[i]; }
    const auto distance=std::hypot(direction[0],direction[1],direction[2]);
    const double cross[3]={direction[1]*raw->camera_up[2]-direction[2]*raw->camera_up[1],direction[2]*raw->camera_up[0]-direction[0]*raw->camera_up[2],direction[0]*raw->camera_up[1]-direction[1]*raw->camera_up[0]};
    const auto cross_size=std::hypot(cross[0],cross[1],cross[2]);
    require(std::isfinite(distance)&&distance>=0.000001&&std::isfinite(cross_size)&&cross_size>=std::sqrt(1e-15),"degenerate camera");
    require(std::isfinite(raw->vertical)&&(raw->projection==0?(raw->vertical>=1&&raw->vertical<=150):raw->vertical>=0.000001)&&std::isfinite(raw->near_mm)&&std::isfinite(raw->far_mm)&&raw->near_mm>0&&raw->far_mm>raw->near_mm,"invalid projection/clip planes");
    Frame f; f.parameters=*raw; f.parameters.quality_json=nullptr;
    f.quality=parse(raw->quality_json,raw->quality_json_bytes); shape(f.quality,{"schema_version","quality"}); schema(f.quality); quality(f.quality.at("quality"),c.backend);
    return f;
}
void validate_description(const std::string& info,const Config& c) {
    const auto j=parse(reinterpret_cast<const uint8_t*>(info.data()),info.size());
    shape(j,{"schema_version","backend","sdk_version","device_id","device_name","api","driver"},{"compiler"}); schema(j);
    for(auto k:{"backend","sdk_version","device_id","device_name","api","driver"}) text(j.at(k),4096);
    require(text(j.at("backend"))==(backend_id()==TN_BACKEND_FIXTURE?"fixture":c.backend),"actual runtime backend mismatch");
    const auto& selected=c.document.at("selection").at("device");
    require(selected.is_null()||selected==j.at("device_id"),"actual runtime selected device mismatch");
    if(j.contains("compiler")&&!j.at("compiler").is_null()) {
        const auto& compiler=j.at("compiler"); shape(compiler,{"name","version","sha256","path"});
        text(compiler.at("name")); text(compiler.at("version")); const auto hash=text(compiler.at("sha256"));
        require(hash.size()==64&&std::all_of(hash.begin(),hash.end(),[](unsigned char b){return (b>='0'&&b<='9')||(b>='a'&&b<='f')||(b>='A'&&b<='F');}),"invalid compiler digest");
        require(std::filesystem::path(text(compiler.at("path"),32768)).is_absolute(),"compiler path must be absolute");
    }
}
}
