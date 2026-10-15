// A deterministic ABI exerciser, never a renderer or SDK qualification result.
#include "turninator_renderer.h"
#include <windows.h>
#include <algorithm>
#include <cmath>
#include <cstring>
#include <exception>
#include <stdexcept>
#include <string>
#include <vector>
#ifdef TN_DEPENDENT
extern "C" __declspec(dllimport) int tn_fixture_dependency_value();
#endif
namespace {
struct Context {
    DWORD owner = GetCurrentThreadId();
    uint32_t state = TN_IDLE, polls = 0, width = 0, height = 0;
    uint64_t revision = 0;
    bool uploaded = false, pending = false;
    tn_frame frame{};
    std::vector<float> positions, normals, build;
    std::vector<uint32_t> triangles, materials;
    std::string scene, quality;
    ~Context() noexcept { pending = false; /* simulated drain before releasing data */ }
};
template<class T> bool valid(const T *p) { return p && p->size == sizeof(T) && p->version == TN_ABI_VERSION; }
int32_t error(tn_error *e, int32_t code, const char *message) noexcept {
    if (valid(e) && e->text_capacity && e->text) {
        const auto n = std::min<uint64_t>(std::strlen(message), e->text_capacity-1);
        std::memcpy(e->text, message, static_cast<size_t>(n)); e->text[n] = 0;
    }
    return code;
}
template<class F> int32_t guarded(tn_context p, tn_error *e, F f) noexcept {
    auto *c = static_cast<Context*>(p);
    if (!c || c->owner != GetCurrentThreadId()) return error(e,TN_INVALID_ARGUMENT,"invalid context/thread");
    if (c->state == TN_FAILED) return error(e,TN_FAILURE,"context Failed");
    try { return f(*c); }
    catch (const std::exception &x) { c->state=TN_FAILED; return error(e,TN_FAILURE,x.what()); }
    catch (...) { c->state=TN_FAILED; return error(e,TN_FAILURE,"unknown native exception"); }
}
bool json(const uint8_t *p,uint64_t size) { return size <= 1048576 && (size==0 || p); }
template<class T> void copy(std::vector<T>& out,const T* p,size_t n) {
    if(n) out.assign(p,p+n); else out.clear();
}
}
extern "C" {
int32_t tn_renderer_abi_version(tn_abi_info *out, tn_error *e) {
    if (!valid(out)) return error(e,TN_INVALID_ARGUMENT,"invalid descriptor");
    *out = {sizeof(tn_abi_info),TN_ABI_VERSION,TN_BACKEND_FIXTURE,TN_FLAG_TEST_FIXTURE,
        sizeof(tn_error),sizeof(tn_config),sizeof(tn_scene),sizeof(tn_frame),sizeof(tn_poll),sizeof(tn_image)};
#ifdef TN_BAD_VERSION
    out->version=2;
#endif
#ifdef TN_BAD_SIZE
    out->scene_size=1;
#endif
#ifdef TN_BAD_BACKEND
    out->backend=TN_BACKEND_LUXCORE;
#endif
    return TN_OK;
}
int32_t tn_renderer_create(const tn_config *cfg, tn_context *out, tn_error *e) {
    if(out) *out=nullptr;
    if(!out || !valid(cfg) || !json(cfg->json,cfg->json_bytes) || cfg->max_output_side==0 || cfg->reserved)
        return error(e,TN_INVALID_ARGUMENT,"invalid config");
    try {
#ifdef TN_DEPENDENT
        if(tn_fixture_dependency_value()!=17) throw std::runtime_error("bad dependency");
#endif
        *out = new Context(); return TN_OK;
    } catch(const std::exception &x) { return error(e,TN_FAILURE,x.what()); }
    catch(...) { return error(e,TN_FAILURE,"create exception"); }
}
int32_t tn_renderer_describe(tn_context p,uint8_t *buffer,uint64_t capacity,uint64_t *required,tn_error *e) {
    auto *c=static_cast<Context*>(p);
    if(!c || c->owner!=GetCurrentThreadId() || !required) return error(e,TN_INVALID_ARGUMENT,"invalid describe");
    const char *info=R"({"schema_version":1,"backend":"fixture","sdk_version":"cpu-abi1","device_id":"cpu-fixture","device_name":"CPU ABI fixture","api":"fixture","driver":"none"})";
#ifdef TN_BAD_INFO
    info=R"({"schema_version":2,"backend":"fixture","device_id":"cpu-fixture"})";
#endif
#ifdef TN_BAD_INFO_UTF8
    info="\xff";
#endif
    *required=std::strlen(info);
#ifdef TN_LARGE_INFO
    *required=1048577;
    return error(e,TN_CAPACITY,"info too large");
#endif
    if(capacity<*required) return error(e,TN_CAPACITY,"info capacity too small");
    if(!buffer) return error(e,TN_INVALID_ARGUMENT,"null info buffer");
    std::memcpy(buffer,info,static_cast<size_t>(*required)); return TN_OK;
}
int32_t tn_renderer_upload_scene(tn_context p, const tn_scene *s, tn_error *e) {
    return guarded(p,e,[&](Context &c)->int32_t {
        if(c.state!=TN_IDLE) return error(e,TN_INVALID_STATE,"upload requires Idle");
        if(!valid(s) || !json(s->json,s->json_bytes) || s->vertex_count>2000000 || s->triangle_count>4000000 ||
           (s->vertex_count && (!s->positions_mm || !s->normals || !s->build_positions_mm)) ||
           (s->triangle_count && (!s->triangles || !s->triangle_materials)))
            return error(e,TN_INVALID_ARGUMENT,"invalid scene spans");
        for(uint64_t i=0;i<uint64_t(s->triangle_count)*3;++i)
            if(s->triangles[i]>=s->vertex_count) return error(e,TN_INVALID_ARGUMENT,"invalid triangle index");
        const size_t n=size_t(s->vertex_count)*3;
        for(size_t i=0;i<n;++i) if(!std::isfinite(s->positions_mm[i]) || !std::isfinite(s->normals[i]) || !std::isfinite(s->build_positions_mm[i]))
            return error(e,TN_INVALID_ARGUMENT,"nonfinite geometry");
        copy(c.positions,s->positions_mm,n); copy(c.normals,s->normals,n); copy(c.build,s->build_positions_mm,n);
        copy(c.triangles,s->triangles,size_t(s->triangle_count)*3); copy(c.materials,s->triangle_materials,s->triangle_count);
        c.scene.assign(reinterpret_cast<const char*>(s->json),static_cast<size_t>(s->json_bytes));
        c.revision=s->revision; c.uploaded=true; return TN_OK;
    });
}
int32_t tn_renderer_begin_frame(tn_context p,const tn_frame *f,tn_error *e) {
    return guarded(p,e,[&](Context &c)->int32_t {
        if(c.state!=TN_IDLE) return error(e,TN_INVALID_STATE,"begin requires Idle");
        if(!valid(f) || !c.uploaded || f->geometry_revision!=c.revision || !f->width || !f->height || f->width>8000 || f->height>8000 ||
            !json(f->quality_json,f->quality_json_bytes) || f->reserved || f->projection>1 || f->transparent>1)
            return error(e,TN_INVALID_ARGUMENT,"invalid frame/revision");
#ifdef TN_THROW
        throw std::runtime_error("fixture exception");
#endif
#ifdef TN_DELAYED
        // Must not find this DLL beside the bridge, exe, cwd or on PATH.
        HMODULE unexpected=LoadLibraryW(L"tn_fixture_dependency.dll");
        if(unexpected) { FreeLibrary(unexpected); throw std::runtime_error("ambient delayed dependency accepted"); }
        throw std::runtime_error("delayed dependency safely unavailable");
#endif
        c.frame=*f; c.quality.assign(reinterpret_cast<const char*>(f->quality_json),static_cast<size_t>(f->quality_json_bytes));
        c.frame.quality_json=nullptr; // no caller pointers retained
        c.width=f->width; c.height=f->height; c.polls=0; c.pending=true; c.state=TN_RENDERING;
        return TN_OK;
    });
}
int32_t tn_renderer_poll(tn_context p,tn_poll *out,tn_error *e) {
    return guarded(p,e,[&](Context &c)->int32_t {
        if(!valid(out)) return error(e,TN_INVALID_ARGUMENT,"invalid poll");
        uint32_t state=c.state;
        if(c.state==TN_RENDERING && ++c.polls>=2) { c.state=state=TN_READY; c.pending=false; }
        if(c.state==TN_CANCELLING && ++c.polls>=2) { c.pending=false; state=TN_CANCELLED; c.state=TN_IDLE; }
        *out={sizeof(tn_poll),TN_ABI_VERSION,state,0,c.polls,2}; return TN_OK;
    });
}
int32_t tn_renderer_read_rgba8(tn_context p,uint8_t *pixels,uint64_t capacity,tn_image *out,tn_error *e) {
    return guarded(p,e,[&](Context &c)->int32_t {
        if(c.state!=TN_READY) return error(e,TN_INVALID_STATE,"read requires Ready");
        if(!valid(out)) return error(e,TN_INVALID_ARGUMENT,"invalid image");
        const uint64_t bytes=uint64_t(c.width)*c.height*4;
        *out={sizeof(tn_image),TN_ABI_VERSION,c.width,c.height,bytes};
        if(capacity<bytes) return error(e,TN_CAPACITY,"pixel capacity too small");
        if(!pixels) return error(e,TN_INVALID_ARGUMENT,"null pixels");
        const uint8_t pattern[16]={static_cast<uint8_t>(c.frame.sample_seed),0,0,255,
            0,255,0,static_cast<uint8_t>(c.frame.transparent?128:255),0,0,255,255,
            0,0,0,static_cast<uint8_t>(c.frame.transparent?0:255)};
        for(uint64_t i=0;i<bytes;++i) pixels[i]=pattern[i%16];
        c.state=TN_IDLE; return TN_OK;
    });
}
#ifndef TN_MISSING_EXPORT
int32_t tn_renderer_cancel(tn_context p,tn_error *e) {
    return guarded(p,e,[&](Context &c)->int32_t {
        if(c.state==TN_RENDERING || c.state==TN_READY) { c.state=TN_CANCELLING; c.polls=0; }
        return TN_OK;
    });
}
#endif
void tn_renderer_destroy(tn_context p) {
    auto *c=static_cast<Context*>(p);
    if(c && c->owner==GetCurrentThreadId()) { try { delete c; } catch(...) {} }
}
}
