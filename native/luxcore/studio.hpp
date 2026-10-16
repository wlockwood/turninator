#pragma once
#include "backend.hpp"
#include <luxcore/luxcore.h>
#include <array>
#include <cmath>
#include <numbers>
namespace tn::lux {
// Canonical scene.py studio recipe, expressed in the prepared scene's mm units.
// The disk tessellation is fixed; emitted radiance preserves nominal total power
// per disk area. Camera rays cannot see the light geometry.
class Studio {
    using P=luxrays::Property;
    using V=std::array<double,3>;
    std::string preset;
    double extent=1;
    bool floor_present=false;
    static V cross(V a,V b){return {a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]};}
    static V unit(V a){const double n=std::sqrt(a[0]*a[0]+a[1]*a[1]+a[2]*a[2]);if(n<1e-12)throw InvalidInput("Degenerate studio orientation");for(auto& x:a)x/=n;return a;}
    static void object(luxcore::Scene& scene,const std::string& name,bool invisible){
        auto p=std::make_unique<luxrays::Properties>();const auto base="scene.objects."+name;
        p->Set(P(base+".shape")(name));p->Set(P(base+".material")(name));p->Set(P(base+".camerainvisible")(invisible));
        p->Set(P(base+".transformation")(std::vector<float>{1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1}));scene.Parse(std::move(p));
    }
public:
    void initialize(luxcore::Scene& scene,const Json& metadata){
        preset=metadata.at("appearance").at("studio_preset").get<std::string>();
        if(preset!="light"&&preset!="dark"&&preset!="diagram")throw InvalidInput("Unknown LuxCore studio preset; choose light, dark, or diagram");
        const auto& bounds=metadata.at("reference_bounds_mm");V span{};for(size_t k=0;k<3;++k)span[k]=bounds.at("max")[k].get<double>()-bounds.at("min")[k].get<double>();
        extent=std::max({std::hypot(span[0],span[1]),span[2],1.});floor_present=false;
        auto p=std::make_unique<luxrays::Properties>();
        const V world=preset=="dark"?V{.025*.12,.03*.12,.04*.12}:preset=="light"?V{.22*.35,.25*.35,.3*.35}:V{.35,.35,.35};
        p->Set(P("scene.lights.studio.type")("constantinfinite"));p->Set(P("scene.lights.studio.color")(world[0],world[1],world[2]));
        p->Set(P("scene.materials.tn_floor.type")("disney"));p->Set(P("scene.materials.tn_floor.roughness")(.7f));
        const V color=preset=="dark"?V{.025,.03,.04}:V{.18,.20,.24};p->Set(P("scene.materials.tn_floor.basecolor")(color[0],color[1],color[2]));
        constexpr std::array<double,3> power{550,180,650},diameter{2.2,2.5,1.6};
        constexpr unsigned segments=64;
        // The source nominal power scales as extent_m^2. The same factor
        // cancels against disk area, so radiance is independent of model size.
        const double polygon_area=.5*segments*std::sin(2*std::numbers::pi/segments);
        for(unsigned light=0;light<3;++light){
            const auto name="tn_studio"+std::to_string(light),base="scene.materials."+name;
            p->Set(P(base+".type")("matte"));p->Set(P(base+".kd")(0.f));p->Set(P(base+".emission")(static_cast<float>(power[light]/(std::numbers::pi*polygon_area*diameter[light]*diameter[light]*.25))));
            std::unique_ptr<float[]> vertices(luxcore::Scene::AllocVerticesBuffer(segments+1));std::unique_ptr<unsigned[]> triangles(luxcore::Scene::AllocTrianglesBuffer(segments));
            vertices[0]=vertices[1]=vertices[2]=0;
            for(unsigned i=0;i<segments;++i){const double angle=2*std::numbers::pi*i/segments;vertices[(i+1)*3]=static_cast<float>(std::cos(angle));vertices[(i+1)*3+1]=static_cast<float>(std::sin(angle));vertices[(i+1)*3+2]=0;triangles[i*3]=0;triangles[i*3+1]=i+1;triangles[i*3+2]=(i+1)%segments+1;}
            scene.DefineMeshExt(name,segments+1,segments,vertices.get(),triangles.get(),nullptr,nullptr,nullptr,nullptr);vertices.release();triangles.release();
        }
        constexpr unsigned cells=16,vertices_count=17*17,faces=16*16*2;
        std::unique_ptr<float[]> vertices(luxcore::Scene::AllocVerticesBuffer(vertices_count));std::unique_ptr<unsigned[]> triangles(luxcore::Scene::AllocTrianglesBuffer(faces));
        for(unsigned y=0;y<=cells;++y)for(unsigned x=0;x<=cells;++x){const size_t i=(y*17+x)*3;vertices[i]=static_cast<float>((double(x)/cells*200-100)*extent);vertices[i+1]=static_cast<float>((double(y)/cells*200-100)*extent);vertices[i+2]=-.01f;}
        for(unsigned y=0;y<cells;++y)for(unsigned x=0;x<cells;++x){const unsigned a=y*17+x,b=a+1,c=a+17,d=c+1;const size_t i=(y*cells+x)*6;const unsigned cell[]={a,b,d,a,d,c};std::copy(cell,cell+6,triangles.get()+i);}
        scene.DefineMeshExt("tn_floor",vertices_count,faces,vertices.get(),triangles.get(),nullptr,nullptr,nullptr,nullptr);vertices.release();triangles.release();
        scene.Parse(std::move(p));for(unsigned i=0;i<3;++i)object(scene,"tn_studio"+std::to_string(i),true);
    }
    void frame(luxcore::Scene& scene,const tn_frame& f){
        const bool floor=!f.transparent&&preset!="diagram";
        if(floor&&!floor_present)object(scene,"tn_floor",false);else if(!floor&&floor_present)scene.DeleteObject("tn_floor");floor_present=floor;
        const V back=unit({f.camera_position_mm[0]-f.camera_target_mm[0],f.camera_position_mm[1]-f.camera_target_mm[1],f.camera_position_mm[2]-f.camera_target_mm[2]});
        const V right=unit(cross({f.camera_up[0],f.camera_up[1],f.camera_up[2]},back)),up=cross(back,right);
        constexpr std::array<V,3> positions{V{-2.5,-3,4},V{3,-1,2},V{0,3,3}};constexpr std::array<double,3> diameter{2.2,2.5,1.6};
        for(unsigned i=0;i<3;++i){V location=positions[i];if(preset=="diagram")for(size_t k=0;k<3;++k)location[k]=right[k]*positions[i][0]+up[k]*positions[i][1]+back[k]*positions[i][2];
            const V normal=unit({-location[0],-location[1],-location[2]}),tangent=unit(cross(std::abs(normal[2])<.99?V{0,0,1}:V{0,1,0},normal)),bitangent=cross(normal,tangent);
            float matrix[16]{};for(size_t k=0;k<3;++k){matrix[k]=static_cast<float>(tangent[k]*diameter[i]*.5*extent);matrix[4+k]=static_cast<float>(bitangent[k]*diameter[i]*.5*extent);matrix[8+k]=static_cast<float>(normal[k]);matrix[12+k]=static_cast<float>(f.camera_target_mm[k]+location[k]*extent);}matrix[15]=1;scene.UpdateObjectTransformation("tn_studio"+std::to_string(i),matrix);
        }
    }
};
}
