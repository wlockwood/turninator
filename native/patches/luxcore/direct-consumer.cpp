#include <luxcore/luxcore.h>
#include <luxrays/core/geometry/matrix4x4.h>
#include <iostream>
#include <stdexcept>
int main() {
    try {
        // All utility implementations must resolve from the native DLL importlib.
        luxrays::Properties p; p.SetFromString("probe.value = 7\n");
        if(p.Get(luxrays::Property("probe.value")(0)).Get<int>() != 7) return 1;
        float values[16] = {1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1};
        luxrays::Matrix4x4 matrix(values);
        if(matrix.Determinant()!=1 || matrix.Transpose().Determinant()!=1) return 2;
        bool caught=false;
        try { p.Get("missing.required.property"); } catch(const std::runtime_error &) { caught=true; }
        if(!caught) return 3;
        luxcore::Init();
        auto scene=luxcore::Scene::Create();
        auto camera=std::make_unique<luxrays::Properties>();
        camera->SetFromString("scene.camera.lookat.orig = 0 0 2\nscene.camera.lookat.target = 0 0 0\nscene.camera.up = 0 1 0\nscene.camera.fieldofview = 50\nscene.materials.surface.type = matte\nscene.materials.surface.kd = 1 0 0\nscene.lights.sky.type = constantinfinite\nscene.lights.sky.color = 1 1 1\n");
        scene->Parse(std::move(camera));
        std::cout << "CONFIG_PROBE camera parsed\n" << std::flush;
        auto configuration=std::make_unique<luxrays::Properties>();
        configuration->Set(luxrays::Property("context.cuda.cache.directory")("D:/owned-\xc3\xa9-\xe5\xb1\xa4"));
        configuration->Set(luxrays::Property("context.cuda.cache.compiler_identity")(std::string(64,'a')));
        configuration->Set(luxrays::Property("context.cuda.cache.sdk_identity")(std::string(64,'b')));
        configuration->Set(luxrays::Property("context.unrelated.fixture")("unchanged filtering"));
        std::cout << "CONFIG_PROBE input properties=" << configuration->ToString() << std::flush;
        auto config=luxcore::RenderConfig::Create(std::move(configuration),scene);
        std::cout << "CONFIG_PROBE config created\n" << std::flush;
        const auto &normalized=config->ToProperties();
        for(const auto key:{"context.cuda.cache.directory","context.cuda.cache.compiler_identity","context.cuda.cache.sdk_identity"}){
            if(!normalized->IsDefined(key)) { std::cerr<<"CONFIG_FORWARD_FAIL: "<<key<<'\n'; return 5; }
            if(normalized->Get(key).Get<std::string>()!=config->GetProperties()->Get(key).Get<std::string>())return 6;
        }
        if(normalized->IsDefined("context.unrelated.fixture"))return 7;
        std::cout << "CONFIG_FORWARD_PASS exact private fields and UTF8 values; unrelated filtering preserved\n";
        std::cout << "DIRECT_CONSUMER_PASS properties/matrix/exception/destroy\n";
    } catch(const std::exception &e) { std::cerr << e.what(); return 4; }
}
