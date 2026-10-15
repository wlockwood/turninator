// Real shared facade + a deliberately pending CPU backend. No SDK/GPU claims.
#include "backend.hpp"
#include "test_inputs.hpp"
#include <chrono>
#include <condition_variable>
#include <iostream>
#include <mutex>
#include <thread>
using namespace std::chrono_literals;
namespace {
struct Control {
    std::mutex mutex;
    std::condition_variable changed;
    bool entered=false, release=false, owner_calls_done=false, timed_out=false;
    bool throw_render=false, throw_upload=false, throw_describe=false, unknown_exception=false, throw_factory=false;
    bool destroyed=false, destroy_started=false, cancellation_seen=false;
    bool gate_idle_wait=false, gate_idle_after_render=false, idle_boundary=false, unsynchronized_stop=false;
    bool gate_upload=false, upload_entered=false, upload_release=false, upload_cancel_seen=false;
    std::vector<std::thread::id> threads;
    std::vector<uint64_t> revisions;
    uint32_t seed=0;
    uint32_t render_count=0;
    float position=0;
} control;
void check(bool b,const char* what) { if(!b) throw std::runtime_error(what); }
void record() { std::lock_guard lock(control.mutex); control.threads.push_back(std::this_thread::get_id()); }
struct PendingBackend final : tn::Backend {
    PendingBackend() { record(); }
    ~PendingBackend() noexcept override {
        record(); std::lock_guard lock(control.mutex); control.destroyed=true; control.changed.notify_all();
    }
    std::string describe() override {
        record(); if(control.throw_describe) throw std::runtime_error("describe fault");
        return R"({"schema_version":1,"backend":"fixture","sdk_version":"test","device_id":"cpu-fixture","device_name":"CPU fixture","api":"fixture","driver":"none"})";
    }
    void upload(const tn::Scene& s,tn::Work& work) override {
        record(); control.revisions.push_back(s.revision);
        control.position=s.positions_mm.empty()?0:s.positions_mm[0];
        if(control.throw_upload) throw std::runtime_error("upload fault");
        std::unique_lock lock(control.mutex);
        if(control.gate_upload) {
            control.upload_entered=true; control.changed.notify_all();
            while(!control.upload_release) {
                if(work.cancelled()) { control.upload_cancel_seen=true; control.changed.notify_all(); }
                control.changed.wait_for(lock,10ms); // pending SDK upload/drain
            }
        }
    }
    std::vector<uint8_t> render(const tn::Frame& f,tn::Work& work) override {
        record();
        std::unique_lock lock(control.mutex); control.entered=true;
        ++control.render_count;
        if(control.gate_idle_after_render) { control.gate_idle_wait=true; control.gate_idle_after_render=false; }
        control.seed=f.parameters.sample_seed; work.total=7; work.completed=3;
        control.changed.notify_all();
        while(!control.release) { // SDK/readback drain gate
            if(work.cancelled()) { control.cancellation_seen=true; control.changed.notify_all(); }
            control.changed.wait_for(lock,10ms);
        }
        if(control.unknown_exception) throw 17;
        if(control.throw_render) throw std::runtime_error("render fault");
        return std::vector<uint8_t>(16,42);
    }
};
tn_error error{};
uint8_t error_text[256]{};
tn_context create() {
    auto json=config_document().dump();
    tn_config cfg{sizeof(tn_config),1,reinterpret_cast<const uint8_t*>(json.data()),json.size(),1048576,1048576,8000,0};
    tn_context c=nullptr; check(tn_renderer_create(&cfg,&c,&error)==TN_OK,"create"); return c;
}
void upload(tn_context c,uint64_t revision,float x=7) {
    auto json=scene_document().dump();
    float p[3]={x,0,0}, n[3]={0,0,1};
    tn_scene s{sizeof(tn_scene),1,revision,p,n,p,nullptr,nullptr,1,0,reinterpret_cast<const uint8_t*>(json.data()),json.size()};
    check(tn_renderer_upload_scene(c,&s,&error)==TN_OK,"upload");
    p[0]=99; // a retained input pointer would observe the wrong position
}
uint32_t poll(tn_context c) {
    tn_poll p{sizeof(tn_poll),1}; check(tn_renderer_poll(c,&p,&error)==TN_OK,"poll"); return p.state;
}
void wait_state(tn_context c,uint32_t state) {
    const auto deadline=std::chrono::steady_clock::now()+5s;
    while(poll(c)!=state) { check(std::chrono::steady_clock::now()<deadline,"state deadline"); std::this_thread::yield(); }
}
void wait_entered() {
    std::unique_lock lock(control.mutex);
    check(control.changed.wait_for(lock,5s,[]{return control.entered;}),"worker render entry");
}
void release() { std::lock_guard lock(control.mutex); control.release=true; control.changed.notify_all(); }
void reset() {
    control.entered=control.release=control.owner_calls_done=control.timed_out=false;
    control.destroyed=control.destroy_started=control.cancellation_seen=false;
    control.gate_idle_wait=control.gate_idle_after_render=control.idle_boundary=control.unsynchronized_stop=false;
    control.gate_upload=control.upload_entered=control.upload_release=control.upload_cancel_seen=false;
    control.render_count=0;
    error_text[0]=0;
    control.throw_render=control.throw_upload=control.throw_describe=control.unknown_exception=control.throw_factory=false;
    control.threads.clear(); control.revisions.clear();
}
void verify_owner() {
    check(control.destroyed,"backend destruction"); check(control.threads.size()>=3,"backend lifecycle calls");
    for(auto id:control.threads) check(id==control.threads[0] && id!=std::this_thread::get_id(),"one dedicated SDK owner thread");
}
void pending_cancel_and_bounded_queue() {
    reset(); auto c=create(); upload(c,1); upload(c,2);
    std::string q=R"({"schema_version":1,"quality":{"kind":"filament","aa":"high","shadows":"high"}})";
    auto f=frame_input(2,q); check(tn_renderer_begin_frame(c,&f,&error)==TN_OK,"begin");
    f.sample_seed=0; q.assign(q.size(),'x'); wait_entered();
    // A blocking poll/cancel releases only through this watchdog, then fails.
    std::jthread watchdog([]{
        std::unique_lock lock(control.mutex);
        if(!control.changed.wait_for(lock,3s,[]{return control.owner_calls_done;})) {
            control.timed_out=true; control.release=true; control.changed.notify_all();
        }
    });
    check(poll(c)==TN_RENDERING,"pending rendering");
    std::jthread wrong_owner([&]{
        tn_poll p{sizeof(tn_poll),1};
        check(tn_renderer_poll(c,&p,nullptr)==TN_INVALID_ARGUMENT,"opaque context rejects wrong owner thread");
    }); wrong_owner.join();
    tn_poll progress{sizeof(tn_poll),1}; check(tn_renderer_poll(c,&progress,&error)==TN_OK && progress.completed_work==3 && progress.total_work==7,"atomic progress");
    for(int i=0;i<200;++i) check(tn_renderer_begin_frame(c,&f,&error)==TN_INVALID_STATE,"no queued frame growth");
    check(tn_renderer_cancel(c,&error)==TN_OK,"cancel");
    check(tn_renderer_cancel(c,&error)==TN_OK,"idempotent cancel");
    check(poll(c)==TN_CANCELLING,"cancel must await drain");
    { std::lock_guard lock(control.mutex); control.owner_calls_done=true; control.changed.notify_all(); }
    watchdog.join(); check(!control.timed_out,"poll/cancel blocked on SDK");
    release(); wait_state(c,TN_CANCELLED); check(poll(c)==TN_IDLE,"cancel restores Idle");
    check(control.revisions==std::vector<uint64_t>{2},"only latest Idle scene uploads");
    check(control.position==7 && control.seed==255,"owned typed scene/frame copies");
    q=R"({"schema_version":1,"quality":{"kind":"filament","aa":"high","shadows":"high"}})"; f=frame_input(2,q);
    check(tn_renderer_begin_frame(c,&f,&error)==TN_OK,"transform-only begin"); wait_state(c,TN_READY);
    uint8_t bytes[17]; std::fill(std::begin(bytes),std::end(bytes),uint8_t(91)); tn_image image{sizeof(tn_image),1};
    check(tn_renderer_read_rgba8(c,bytes,15,&image,&error)==TN_CAPACITY && image.required_bytes==16,"short capacity");
    for(auto b:bytes) check(b==91,"short read writes nothing");
    check(poll(c)==TN_READY,"short retry preserves Ready");
    check(tn_renderer_read_rgba8(c,bytes,17,&image,&error)==TN_OK && bytes[0]==42 && bytes[16]==91,"read retry bounds");
    check(control.revisions.size()==1,"transform reuses upload");
    upload(c,2,11); // replacing a numeric revision must still invalidate upload
    check(tn_renderer_begin_frame(c,&f,&error)==TN_OK,"replacement begin"); wait_state(c,TN_READY);
    check(control.revisions.size()==2 && control.position==11,"replacement upload generation");
    check(tn_renderer_cancel(c,&error)==TN_OK,"Ready cancel"); wait_state(c,TN_CANCELLED);
    tn_renderer_destroy(c); verify_owner();
}
void destroy_joins_pending() {
    reset(); auto c=create(); upload(c,1); const std::string q=R"({"schema_version":1,"quality":{"kind":"filament","aa":"high","shadows":"high"}})";
    auto f=frame_input(1,q); check(tn_renderer_begin_frame(c,&f,&error)==TN_OK,"begin destroy"); wait_entered();
    std::thread drainer([]{
        std::unique_lock lock(control.mutex);
        check(control.changed.wait_for(lock,5s,[]{return control.destroy_started&&control.cancellation_seen;}),"destroy requests worker cancellation");
        check(!control.destroyed,"destroy before readback drain");
        control.release=true; control.changed.notify_all();
    });
    { std::lock_guard lock(control.mutex); control.destroy_started=true; control.changed.notify_all(); }
    tn_renderer_destroy(c); drainer.join(); verify_owner();
}
void cancellation_during_upload_drains_and_reuploads() {
    reset(); auto c=create(); upload(c,7);
    {
        std::lock_guard lock(control.mutex); control.gate_upload=true; control.release=true;
    }
    const std::string q=R"({"schema_version":1,"quality":{"kind":"filament","aa":"high","shadows":"high"}})";
    auto f=frame_input(7,q); check(tn_renderer_begin_frame(c,&f,&error)==TN_OK,"begin gated upload");
    {
        std::unique_lock lock(control.mutex);
        check(control.changed.wait_for(lock,5s,[]{return control.upload_entered;}),"SDK upload entry");
    }
    check(tn_renderer_cancel(c,&error)==TN_OK,"cancel during upload");
    {
        std::unique_lock lock(control.mutex);
        check(control.changed.wait_for(lock,5s,[]{return control.upload_cancel_seen;}),"SDK upload observes cancellation");
        check(control.render_count==0,"cancelled upload must not start render");
    }
    check(poll(c)==TN_CANCELLING,"upload cancellation cannot acknowledge before drain");
    check(tn_renderer_begin_frame(c,&f,&error)==TN_INVALID_STATE,"no begin during upload drain");
    {
        std::lock_guard lock(control.mutex); control.upload_release=true; control.changed.notify_all();
    }
    wait_state(c,TN_CANCELLED); check(poll(c)==TN_IDLE,"upload drain restores Idle");
    check(tn_renderer_begin_frame(c,&f,&error)==TN_OK,"same-scene retry after interrupted upload");
    wait_state(c,TN_READY);
    check(control.revisions==std::vector<uint64_t>{7,7},"interrupted upload generation must be uploaded again");
    check(control.render_count==1,"only retried frame renders");
    uint8_t bytes[16]; tn_image image{sizeof(tn_image),1};
    check(tn_renderer_read_rgba8(c,bytes,sizeof(bytes),&image,&error)==TN_OK,"retried upload readback");
    tn_renderer_destroy(c); verify_owner();
}
void destroy_at_idle_wait_boundary() {
    reset(); auto c=create(); upload(c,1);
    {
        std::lock_guard lock(control.mutex);
        control.gate_idle_after_render=true; control.release=true;
    }
    const std::string q=R"({"schema_version":1,"quality":{"kind":"filament","aa":"high","shadows":"high"}})";
    auto f=frame_input(1,q); check(tn_renderer_begin_frame(c,&f,&error)==TN_OK,"begin idle-boundary frame");
    {
        std::unique_lock lock(control.mutex);
        check(control.changed.wait_for(lock,5s,[]{return control.idle_boundary;}),"controlled idle predicate boundary");
    }
    check(poll(c)==TN_READY,"frame drained before idle shutdown");
    uint8_t bytes[16]; tn_image image{sizeof(tn_image),1};
    check(tn_renderer_read_rgba8(c,bytes,sizeof(bytes),&image,&error)==TN_OK,"idle-boundary read");
    check(poll(c)==TN_IDLE,"idle before destroy");
    tn_renderer_destroy(c);
    check(!control.unsynchronized_stop,"stop predicate changed while worker still held wait mutex");
    verify_owner();
}
void exceptions_cleanup() {
    for(int stage=0;stage<5;++stage) {
        reset();
        if(stage>=3) {
            control.throw_describe=stage==3; control.throw_factory=stage==4;
            auto json=config_document().dump(); tn_config cfg{sizeof(tn_config),1,reinterpret_cast<const uint8_t*>(json.data()),json.size(),1048576,1048576,8000,0};
            tn_context c=nullptr; check(tn_renderer_create(&cfg,&c,&error)==TN_FAILURE && !c,"create exception translation");
            if(stage==3) verify_owner();
            else check(control.threads.size()==1 && control.threads[0]!=std::this_thread::get_id(),"factory exception worker ownership");
            continue;
        }
        auto c=create(); control.throw_upload=stage==0; control.throw_render=stage==1; control.unknown_exception=stage==2; control.release=true;
        upload(c,1); const std::string q=R"({"schema_version":1,"quality":{"kind":"filament","aa":"high","shadows":"high"}})";
        auto f=frame_input(1,q); check(tn_renderer_begin_frame(c,&f,&error)==TN_OK,"asynchronous begin");
        tn_poll p{sizeof(tn_poll),1}; const auto deadline=std::chrono::steady_clock::now()+5s;
        while(tn_renderer_poll(c,&p,&error)==TN_OK) { check(std::chrono::steady_clock::now()<deadline,"failure deadline"); std::this_thread::yield(); }
        check(tn_renderer_cancel(c,&error)==TN_FAILURE,"Failed cancel rejection");
        check(tn_renderer_begin_frame(c,&f,&error)==TN_FAILURE,"Failed begin rejection");
        uint64_t required=0; check(tn_renderer_describe(c,nullptr,0,&required,&error)==TN_CAPACITY && required>0,"Failed describe diagnostic");
        tn_renderer_destroy(c); verify_owner();
    }
}
void cache_config_before_factory() {
    for(const auto& value:std::vector<tn::Json>{nullptr,42,"","relative/cache",R"(C:cache)",R"(\cache)",R"(\\server)",R"(\\.\pipe\cache)",std::string("C:\\bad\0path",11),std::string("C:\\")+std::string(4094,'x')}) {
        reset(); control.throw_factory=true;
        auto document=config_document(); document["cache_directory"]=value;
        auto bytes=document.dump();
        tn_config cfg{sizeof(tn_config),1,reinterpret_cast<const uint8_t*>(bytes.data()),bytes.size(),1048576,1048576,8000,0};
        tn_context c=nullptr;
        check(tn_renderer_create(&cfg,&c,&error)==TN_INVALID_ARGUMENT && !c,"invalid cache directory rejected");
        check(control.threads.empty(),"invalid cache directory must not reach SDK factory");
    }
    reset();
    auto document=config_document(); document["cache_directory"]=R"(\\?\UNC\server\share\cache)";
    const auto bytes=document.dump();
    tn_config cfg{sizeof(tn_config),1,reinterpret_cast<const uint8_t*>(bytes.data()),bytes.size(),1048576,1048576,8000,0};
    check(tn::copy_config(&cfg).document.at("cache_directory")==R"(\\?\UNC\server\share\cache)","cache path copied without rewriting");
}
}
namespace tn {
uint32_t backend_id() noexcept { return TN_BACKEND_FIXTURE; }
uint32_t backend_flags() noexcept { return TN_FLAG_TEST_FIXTURE; }
std::unique_ptr<Backend> make_backend(const Config&) {
    if(control.throw_factory) { record(); throw std::runtime_error("factory fault"); }
    return std::make_unique<PendingBackend>();
}
}
namespace tn::test {
void before_idle_wait(const std::atomic<bool>& stopping,const std::atomic<bool>& cancellation) {
    {
        std::lock_guard lock(control.mutex);
        if(!control.gate_idle_wait) return;
        control.gate_idle_wait=false; control.idle_boundary=true; control.changed.notify_all();
    }
    // The facade still holds its wait mutex and has evaluated a false predicate.
    // Prompt atomic cancellation tells us the owner has entered destruction.
    const auto deadline=std::chrono::steady_clock::now()+5s;
    while(!cancellation.load()) {
        check(std::chrono::steady_clock::now()<deadline,"owner requests shutdown at wait boundary");
        std::this_thread::yield();
    }
    if(stopping.load()) {
        control.unsynchronized_stop=true;
        // Exit the worker through its normal exception cleanup instead of
        // sleeping forever after the deliberately exposed missed notification.
        throw std::runtime_error("stop predicate changed outside wait mutex");
    }
}
}
int main() {
    error={sizeof(tn_error),1,error_text,sizeof(error_text)};
    try { cache_config_before_factory(); pending_cancel_and_bounded_queue(); destroy_joins_pending(); cancellation_during_upload_drains_and_reuploads(); destroy_at_idle_wait_boundary(); exceptions_cleanup();
        std::cout<<"worker ownership, bounded queue, cancellation/render/upload drain, reupload, idle shutdown, copies, retry, exceptions and join passed\n"; return 0;
    } catch(const std::exception& e) { release(); std::cerr<<e.what()<<": "<<error_text<<"\n"; return 1; }
}
