#include "facade.hpp"
// Compile once per bridge DLL. Fault macros are used only by fixture targets.
extern "C" {
int32_t tn_renderer_abi_version(tn_abi_info* out, tn_error* e) {
    const auto result=tn::facade::abi(out,e);
    if(result!=TN_OK) return result;
#ifdef TN_BAD_VERSION
    out->version=2;
#endif
#ifdef TN_BAD_SIZE
    out->scene_size=1;
#endif
#ifdef TN_BAD_BACKEND
    out->backend=TN_BACKEND_LUXCORE;
#endif
    return result;
}
int32_t tn_renderer_create(const tn_config* c,tn_context* p,tn_error* e) { return tn::facade::create(c,p,e); }
int32_t tn_renderer_describe(tn_context p,uint8_t* b,uint64_t n,uint64_t* r,tn_error* e) { return tn::facade::describe(p,b,n,r,e); }
int32_t tn_renderer_upload_scene(tn_context p,const tn_scene* s,tn_error* e) { return tn::facade::upload(p,s,e); }
int32_t tn_renderer_begin_frame(tn_context p,const tn_frame* f,tn_error* e) { return tn::facade::begin(p,f,e); }
int32_t tn_renderer_poll(tn_context p,tn_poll* o,tn_error* e) { return tn::facade::poll(p,o,e); }
int32_t tn_renderer_read_rgba8(tn_context p,uint8_t* b,uint64_t n,tn_image* o,tn_error* e) { return tn::facade::read(p,b,n,o,e); }
#ifndef TN_MISSING_EXPORT
int32_t tn_renderer_cancel(tn_context p,tn_error* e) { return tn::facade::cancel(p,e); }
#endif
void tn_renderer_destroy(tn_context p) { tn::facade::destroy(p); }
}
