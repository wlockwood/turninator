#include "facade.hpp"
#include "backend.hpp"
#include <algorithm>
#include <array>
#include <condition_variable>
#include <cstring>
#include <mutex>
#include <optional>
#include <thread>
namespace tn::facade {
namespace {
static_assert(std::atomic<uint32_t>::is_always_lock_free && std::atomic<uint64_t>::is_always_lock_free && std::atomic<bool>::is_always_lock_free);
template<class T> bool valid(const T* p) noexcept { return p&&p->size==sizeof(T)&&p->version==1; }
int32_t error(tn_error* e,int32_t code,const char* message) noexcept {
    if(valid(e)&&e->text&&e->text_capacity) {
        const auto n=std::min<uint64_t>(std::strlen(message),e->text_capacity-1);
        // SDK exception text is not necessarily UTF-8. Keep diagnostics bounded
        // and valid UTF-8 even when a third-party exception uses a legacy locale.
        for(size_t i=0;i<static_cast<size_t>(n);++i) { const auto b=static_cast<unsigned char>(message[i]); e->text[i]=(b>=32&&b<127)?b:static_cast<uint8_t>('?'); }
        e->text[n]=0;
    }
    return code;
}
struct Task { Frame frame; std::shared_ptr<const Scene> scene; uint64_t generation; };
struct Context {
    const std::thread::id owner=std::this_thread::get_id();
    const Config config;
    std::atomic<uint32_t> state{TN_IDLE};
    std::atomic<bool> cancellation{false}, stopping{false};
    std::atomic<uint64_t> completed{0}, total{0};
    std::shared_ptr<const Scene> scene;
    uint64_t generation=0;
    uint32_t width=0,height=0;
    std::vector<uint8_t> pixels;
    std::string description;
    std::array<char,4096> diagnostic{};
    std::mutex mutex;
    std::condition_variable changed;
    std::optional<Task> task;
    bool initialized=false;
    std::thread worker;
    explicit Context(Config c):config(std::move(c)) {}
    void set_diagnostic(const char* message) noexcept {
        const auto n=std::min(std::strlen(message),diagnostic.size()-1);
        std::memcpy(diagnostic.data(),message,n); diagnostic[n]=0;
    }
    void fail(const char* message) noexcept {
        set_diagnostic(message);
        state.store(TN_FAILED,std::memory_order_release);
    }
    void run() noexcept {
        std::unique_ptr<Backend> backend;
        // These owned inputs outlive exception unwinding and backend drain.
        std::optional<Task> next;
        Work work{cancellation,completed,total};
        try {
            backend=make_backend(config);
            if(!backend) throw std::runtime_error("native factory returned null backend");
            description=backend->describe(); validate_description(description,config);
            { std::lock_guard lock(mutex); initialized=true; } changed.notify_one();
            uint64_t uploaded_generation=0;
            for(;;) {
                {
                    std::unique_lock lock(mutex);
                    changed.wait(lock,[&]{return stopping.load()||task.has_value();});
                    if(stopping.load()) break;
                    next=std::move(task); task.reset();
                }
                // Drop retained CPU readback capacity before the SDK allocates
                // another image, rather than temporarily retaining two images.
                std::vector<uint8_t>().swap(pixels);
                if(!work.cancelled()) {
                    if(uploaded_generation!=next->generation) {
                        backend->upload(*next->scene,work);
                        if(!work.cancelled()) uploaded_generation=next->generation;
                    }
                    if(!work.cancelled()) {
                        auto image=backend->render(next->frame,work);
                        if(!work.cancelled()) {
                            const auto& p=next->frame.parameters;
                            if(image.size()!=uint64_t(p.width)*p.height*4) throw std::runtime_error("backend returned invalid RGBA8 byte count");
                            for(size_t i=0;i<image.size();i+=4) if(image[i+3]==0) image[i]=image[i+1]=image[i+2]=0;
                            pixels=std::move(image);
                        }
                    }
                }
                // Release owned queued inputs before publishing completion. From
                // Ready/Cancelled onwards the worker will not touch this frame.
                next.reset();
                uint32_t expected=TN_RENDERING;
                if(!state.compare_exchange_strong(expected,TN_READY,std::memory_order_release,std::memory_order_acquire)) {
                    if(expected==TN_CANCELLING) state.store(TN_CANCELLED,std::memory_order_release);
                }
            }
            backend.reset(); // SDK destructor on the exact factory owner thread
        } catch(const std::exception& e) {
            set_diagnostic(e.what());
            backend.reset(); // destructor must drain before Failed is observable
            next.reset(); state.store(TN_FAILED,std::memory_order_release);
        } catch(...) {
            set_diagnostic("unknown native worker exception");
            backend.reset(); next.reset(); state.store(TN_FAILED,std::memory_order_release);
        }
        // Also wake create after factory/description errors.
        { std::lock_guard lock(mutex); initialized=true; } changed.notify_one();
    }
    ~Context() noexcept {
        stopping=true; cancellation=true; changed.notify_one();
        if(worker.joinable()) worker.join();
    }
};
Context* context(tn_context p) noexcept {
    auto* c=static_cast<Context*>(p); return c&&c->owner==std::this_thread::get_id()?c:nullptr;
}
template<class F> int32_t guarded(tn_context p,tn_error* e,F&& f,bool diagnostics=false) noexcept {
    auto* c=context(p); if(!c) return error(e,TN_INVALID_ARGUMENT,"invalid context/owner thread");
    if(!diagnostics&&c->state.load(std::memory_order_acquire)==TN_FAILED) return error(e,TN_FAILURE,c->diagnostic.data());
    try { return f(*c); }
    catch(const InvalidInput& x) { return error(e,TN_INVALID_ARGUMENT,x.what()); }
    catch(const Json::exception&) { return error(e,TN_INVALID_ARGUMENT,"invalid schema1 JSON field"); }
    catch(const std::exception& x) {
        // No SDK operation runs here. All owner-side allocations happen in Idle
        // before a frame can be queued, so this failure has a single writer.
        c->fail(x.what()); return error(e,TN_FAILURE,c->diagnostic.data());
    }
    catch(...) { c->fail("unknown native facade exception"); return error(e,TN_FAILURE,c->diagnostic.data()); }
}
}
int32_t abi(tn_abi_info* out,tn_error* e) noexcept {
    if(!valid(out)) return error(e,TN_INVALID_ARGUMENT,"invalid ABI descriptor");
    *out={sizeof(tn_abi_info),1,backend_id(),backend_flags(),sizeof(tn_error),sizeof(tn_config),sizeof(tn_scene),sizeof(tn_frame),sizeof(tn_poll),sizeof(tn_image)};
    return TN_OK;
}
int32_t create(const tn_config* raw,tn_context* out,tn_error* e) noexcept {
    if(out) *out=nullptr;
    if(!out) return error(e,TN_INVALID_ARGUMENT,"null context output");
    try {
        auto c=std::make_unique<Context>(copy_config(raw));
        c->worker=std::thread([p=c.get()]{p->run();});
        { std::unique_lock lock(c->mutex); c->changed.wait(lock,[&]{return c->initialized;}); }
        if(c->state.load(std::memory_order_acquire)==TN_FAILED) return error(e,TN_FAILURE,c->diagnostic.data());
        *out=c.release(); return TN_OK;
    } catch(const InvalidInput& x) { return error(e,TN_INVALID_ARGUMENT,x.what()); }
    catch(const Json::exception&) { return error(e,TN_INVALID_ARGUMENT,"invalid schema1 config"); }
    catch(const std::exception& x) { return error(e,TN_FAILURE,x.what()); }
    catch(...) { return error(e,TN_FAILURE,"unknown native create exception"); }
}
int32_t describe(tn_context p,uint8_t* buffer,uint64_t capacity,uint64_t* required,tn_error* e) noexcept {
    return guarded(p,e,[&](Context& c)->int32_t {
        if(!required) return error(e,TN_INVALID_ARGUMENT,"null description length output");
        *required=c.description.size();
        if(capacity<*required) return error(e,TN_CAPACITY,"description capacity too small");
        if(!buffer) return error(e,TN_INVALID_ARGUMENT,"null description buffer");
        std::memcpy(buffer,c.description.data(),static_cast<size_t>(*required)); return TN_OK;
    },true);
}
int32_t upload(tn_context p,const tn_scene* raw,tn_error* e) noexcept {
    return guarded(p,e,[&](Context& c)->int32_t {
        if(c.state.load(std::memory_order_acquire)!=TN_IDLE) return error(e,TN_INVALID_STATE,"upload requires Idle");
        auto scene=copy_scene(raw,c.config);
        c.scene=std::move(scene); ++c.generation; return TN_OK;
    });
}
int32_t begin(tn_context p,const tn_frame* raw,tn_error* e) noexcept {
    return guarded(p,e,[&](Context& c)->int32_t {
        if(c.state.load(std::memory_order_acquire)!=TN_IDLE) return error(e,TN_INVALID_STATE,"begin requires Idle");
        if(!c.scene) return error(e,TN_INVALID_ARGUMENT,"begin requires uploaded geometry");
        Task next{copy_frame(raw,c.config,c.scene->revision),c.scene,c.generation};
        std::lock_guard lock(c.mutex);
        c.width=next.frame.parameters.width; c.height=next.frame.parameters.height;
        c.cancellation=false; c.completed=0; c.total=0; c.pixels.clear();
        c.task.emplace(std::move(next)); c.state.store(TN_RENDERING,std::memory_order_release);
        c.changed.notify_one(); return TN_OK;
    });
}
int32_t poll(tn_context p,tn_poll* out,tn_error* e) noexcept {
    return guarded(p,e,[&](Context& c)->int32_t {
        if(!valid(out)) return error(e,TN_INVALID_ARGUMENT,"invalid poll struct");
        const auto state=c.state.load(std::memory_order_acquire);
        if(state==TN_FAILED) return error(e,TN_FAILURE,c.diagnostic.data());
        const auto total=c.total.load(); const auto completed=c.completed.load();
        *out={sizeof(tn_poll),1,state,0,total?std::min(completed,total):completed,total};
        if(state==TN_CANCELLED) { c.pixels.clear(); c.state.store(TN_IDLE,std::memory_order_release); }
        return TN_OK;
    });
}
int32_t read(tn_context p,uint8_t* pixels,uint64_t capacity,tn_image* out,tn_error* e) noexcept {
    return guarded(p,e,[&](Context& c)->int32_t {
        if(c.state.load(std::memory_order_acquire)!=TN_READY) return error(e,TN_INVALID_STATE,"read requires Ready");
        if(!valid(out)) return error(e,TN_INVALID_ARGUMENT,"invalid image struct");
        const auto bytes=uint64_t(c.width)*c.height*4; *out={sizeof(tn_image),1,c.width,c.height,bytes};
        if(capacity<bytes) return error(e,TN_CAPACITY,"pixel capacity too small");
        if(!pixels) return error(e,TN_INVALID_ARGUMENT,"null pixel buffer");
        std::memcpy(pixels,c.pixels.data(),static_cast<size_t>(bytes)); c.pixels.clear();
        c.state.store(TN_IDLE,std::memory_order_release); return TN_OK;
    });
}
int32_t cancel(tn_context p,tn_error* e) noexcept {
    return guarded(p,e,[&](Context& c)->int32_t {
        auto state=c.state.load(std::memory_order_acquire);
        for(;;) {
            if(state==TN_FAILED) return error(e,TN_FAILURE,c.diagnostic.data());
            if(state!=TN_RENDERING&&state!=TN_READY) return TN_OK;
            c.cancellation=true;
            // Ready means SDK/readback is already drained. A pending Rendering
            // reaches Cancelled only in the worker, after render/destruction.
            const auto next=state==TN_READY?TN_CANCELLED:TN_CANCELLING;
            if(c.state.compare_exchange_weak(state,next,std::memory_order_acq_rel)) return TN_OK;
        }
    });
}
void destroy(tn_context p) noexcept {
    try { if(auto* c=context(p)) delete c; } catch(...) {} // never unwind across C
}
}
