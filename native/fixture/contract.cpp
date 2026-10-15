// Raw caller-buffer checks complement the safe Rust host tests.
#include "turninator_renderer.h"
#include <algorithm>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>
static void check(bool condition) { if(!condition) throw std::runtime_error("raw ABI contract failure"); }
int main() {
    tn_context context=nullptr;
    try {
        uint8_t errors[128]{};
        tn_error error{sizeof(tn_error),1,errors,sizeof(errors)};
        const uint8_t json[]="{}";
        tn_config config{0,1,json,2,1048576,1048576,8000,0};
        check(tn_renderer_create(&config,&context,&error)==TN_INVALID_ARGUMENT && !context);
        config.size=sizeof(config);
        check(tn_renderer_create(&config,&context,&error)==TN_OK && context);
        uint8_t tiny[4]={73,73,73,73}; uint64_t required=0;
        check(tn_renderer_describe(context,tiny,sizeof(tiny),&required,&error)==TN_CAPACITY);
        check(required>sizeof(tiny) && required<1048576);
        check(std::all_of(std::begin(tiny),std::end(tiny),[](auto b){return b==73;}));
        std::vector<uint8_t> buffer(static_cast<size_t>(required)+8,91);
        const uint64_t capacity=required;
        check(tn_renderer_describe(context,buffer.data(),capacity,&required,&error)==TN_OK && required==capacity);
        check(std::all_of(buffer.begin()+static_cast<ptrdiff_t>(required),buffer.end(),[](auto b){return b==91;}));
        const std::string text(buffer.begin(),buffer.begin()+static_cast<ptrdiff_t>(required));
        check(text.find("\"device_id\":\"cpu-fixture\"")!=std::string::npos);
        tn_poll poll{sizeof(tn_poll),1,0,0,0,0};
        check(tn_renderer_poll(context,&poll,&error)==TN_OK && poll.state==TN_IDLE);
        tn_renderer_destroy(context); context=nullptr;
        std::cout<<"raw ABI caller buffer/size/describe retry checks passed\n";
        return 0;
    } catch(const std::exception &e) {
        if(context) tn_renderer_destroy(context);
        std::cerr<<e.what()<<"\n"; return 1;
    }
}
