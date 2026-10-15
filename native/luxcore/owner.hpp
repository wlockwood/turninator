#pragma once
#include "backend.hpp"
#include <condition_variable>
#include <functional>
#include <future>
#include <mutex>
#include <queue>
#include <thread>

namespace tn::lux {
// A retained idle C ABI context leases this owner without allocating a scene or
// GPU context. Job contexts retain independent Renderers on the same SDK thread.
// The last proxy releases and joins the owner before its module can be unloaded;
// no background thread is joined from a DLL static destructor.
class SdkOwner {
    std::mutex mutex;
    std::condition_variable changed;
    std::queue<std::function<void()>> tasks;
    bool stopping=false;
    std::thread worker;
public:
    SdkOwner():worker([this]{
        for(;;){std::function<void()> task;{
            std::unique_lock lock(mutex);changed.wait(lock,[this]{return stopping||!tasks.empty();});
            if(tasks.empty())return;task=std::move(tasks.front());tasks.pop();
        }task();}
    }){}
    ~SdkOwner(){
        {std::lock_guard lock(mutex);stopping=true;}changed.notify_one();worker.join();
    }
    template<class F> auto invoke(F&& operation){
        using R=std::invoke_result_t<F>;
        auto task=std::make_shared<std::packaged_task<R()>>(std::forward<F>(operation));auto result=task->get_future();
        {std::lock_guard lock(mutex);tasks.emplace([task]{(*task)();});}changed.notify_one();
        return result.get();
    }
    static std::shared_ptr<SdkOwner> acquire(){
        static std::mutex guard;static std::weak_ptr<SdkOwner> weak;
        std::lock_guard lock(guard);auto owner=weak.lock();if(!owner){owner=std::make_shared<SdkOwner>();weak=owner;}return owner;
    }
};
template<class Renderer> class OwnedRenderer final:public Backend {
    std::shared_ptr<SdkOwner> owner=SdkOwner::acquire();
    Renderer* renderer;
public:
    explicit OwnedRenderer(const Config& config):renderer(owner->invoke([&]{return new Renderer(config);})){}
    ~OwnedRenderer() noexcept override {try{owner->invoke([this]{delete renderer;});}catch(...){std::terminate();}}
    std::string describe() override{return owner->invoke([this]{return renderer->describe();});}
    void upload(const Scene& scene,Work& work) override {owner->invoke([&]{renderer->upload(scene,work);});}
    std::vector<uint8_t> render(const Frame& frame,Work& work) override{return owner->invoke([&]{return renderer->render(frame,work);});}
};
}
