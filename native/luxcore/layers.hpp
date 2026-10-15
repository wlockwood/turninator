#pragma once
#include "backend.hpp"
#include <luxcore/luxcore.h>
#include <array>
#include <numbers>
#include <cmath>
#include <map>
namespace tn::lux {
// Physical phase/normal/filter intent comes from geometry.rs. Direct tangent
// projections avoid the pinned compiler's expansion of normalized subgraphs.
class LayerGraph {
    using P=luxrays::Property;
    using V=std::array<double,3>;
    struct Cost {uint64_t stack=0,ops=0;};
    std::unique_ptr<luxrays::Properties> p=std::make_unique<luxrays::Properties>();
    Json nodes=Json::object();
    std::map<std::pair<std::string,int>,Cost> costs;
    unsigned sequence=0;
    static uint64_t sum(uint64_t a,uint64_t b){if(a>UINT64_MAX-b)throw InvalidInput("Layer graph resource overflow");return a+b;}
    static Cost plus(Cost a,Cost b){return {sum(a.stack,b.stack),sum(a.ops,b.ops)};}
    std::string node(const char* type){auto n="tn_l"+std::to_string(sequence++);nodes[n]["type"]=type;p->Set(P("scene.textures."+n+".type")(type));return n;}
    template<class T> void set(const std::string& n,const char* key,T value){nodes[n][key]=value;p->Set(P("scene.textures."+n+"."+key)(value));}
    std::string constant(double value){auto n=node("constfloat1");set(n,"value",value);return n;}
    std::string vector(V xyz){auto n=node("constfloat3");p->Set(P("scene.textures."+n+".value")(xyz[0],xyz[1],xyz[2]));return n;}
    std::string unary(const char* type,const std::string& a){auto n=node(type);set(n,"texture",a);return n;}
    std::string op(const char* type,const std::string& a,const std::string& b){auto n=node(type);set(n,"texture1",a);set(n,"texture2",b);return n;}
    std::string add(const std::string&a,const std::string&b){return op("add",a,b);}
    std::string sub(const std::string&a,const std::string&b){return op("subtract",a,b);}
    std::string mul(const std::string&a,const std::string&b){return op("scale",a,b);}
    std::string div(const std::string&a,const std::string&b){return op("divide",a,b);}
    std::string dot(const std::string&a,const std::string&b){return op("dotproduct",a,b);}
    std::string pow(const std::string&a,double e){auto n=node("power");set(n,"base",a);set(n,"exponent",e);return n;}
    std::string clamp(const std::string&a,double lo,double hi){auto n=unary("clamp",a);set(n,"min",lo);set(n,"max",hi);return n;}
    std::string mix(const std::string&a,const std::string&b,const std::string&t){auto n=op("mix",a,b);set(n,"amount",t);return n;}
    std::string split(const std::string& v,unsigned k){auto n=unary("splitfloat3",v);set(n,"channel",k);return n;}
    std::string packed(const std::array<std::string,3>& v){auto n=node("makefloat3");for(unsigned k=0;k<3;++k)set(n,("texture"+std::to_string(k+1)).c_str(),v[k]);return n;}
    std::string matrix(const double* m,const std::string& v,bool transpose){std::array<std::string,3> out;for(size_t r=0;r<3;++r){V row;for(size_t c=0;c<3;++c)row[c]=m[transpose?c*4+r:r*4+c];out[r]=dot(vector(row),v);}return packed(out);}
    std::string cross(const std::string&a,const std::string&b){std::array<std::string,3>x,y,z;for(unsigned k=0;k<3;++k){x[k]=split(a,k);y[k]=split(b,k);}for(unsigned k=0;k<3;++k)z[k]=sub(mul(x[(k+1)%3],y[(k+2)%3]),mul(x[(k+2)%3],y[(k+1)%3]));return packed(z);}
    Cost cost(const std::string& name,int mode){
        const auto key=std::pair{name,mode};if(costs.contains(key))return costs.at(key);
        const auto& n=nodes.at(name);const auto type=n.at("type").get<std::string>();Cost result{0,1};
        auto child=[&](const char* field,int m){const auto& ref=n.at(field);return ref.is_string()?cost(ref.get<std::string>(),m):Cost{m?3u:1u,1};};
        auto append=[&](const char* field,int m){result=plus(result,child(field,m));};
        auto generic=[&]{auto f=cost(name,0);result=plus(plus(f,f),f);result=plus(result,{8,3});};
        if(type=="constfloat1"||type=="constfloat3")result={mode?3u:1u,1};
        else if(type=="shadingnormal"||type=="position"||type=="hitpointgrey"||type=="hitpointtriangleaov"){if(mode==2)generic();else result={mode?3u:1u,1};}
        else if(type=="normalmap"){if(mode==2)append("texture",1);else result.stack=mode?3:1;}
        else if(mode==2&&(type=="scale"||type=="mix")){append("texture1",2);append("texture2",2);append("texture1",0);append("texture2",0);if(type=="mix"){append("amount",2);append("amount",0);}}
        else if(mode==2&&(type=="add"||type=="subtract")){append("texture1",2);append("texture2",2);}
        else if(mode==2)generic();
        else if(type=="makefloat3"){append("texture1",0);append("texture2",0);append("texture3",0);}
        else if(type=="splitfloat3")append("texture",1);
        else if(type=="dotproduct"){append("texture1",1);append("texture2",1);}
        else if(type=="power"){append("base",0);append("exponent",0);}
        else if(type=="modulo"){append("texture",0);append("modulo",0);}
        else if(type=="band")append("amount",0);
        else if(type=="abs"||type=="clamp")append("texture",mode);
        else {const int m=type=="lessthan"?0:mode;append("texture1",m);append("texture2",m);if(type=="mix")append("amount",0);}
        costs.emplace(key,result);return result;
    }
public:
    uint64_t stack_floats=0,operation_count=0;
    std::unique_ptr<luxrays::Properties> build(const Json& metadata,const tn_frame& f){
        const auto& a=metadata.at("appearance");const double first=a.at("first_layer_height_mm"),pitch=a.at("layer_height_mm"),depth=a.at("groove_depth_mm");
        const auto one=constant(1),half=constant(.5);
        auto z=node("hitpointgrey");set(z,"dataindex",0u);set(z,"channel",0);
        auto first_region=op("lessthan",z,constant(first));
        auto phase=mix(div(sub(z,constant(first)),constant(pitch)),div(z,constant(first)),first_region);
        auto local_pitch=mix(constant(pitch),constant(first),first_region);
        auto wrapped=unary("modulo",phase);set(wrapped,"modulo",1.f);
        // <=16 GPU entries. Sine error <=(2*pi/15)^2/8=.021933; phase/pitch
        // remain continuous. This approximation is versioned in diagnostics.
        auto sine=node("band");set(sine,"amount",wrapped);set(sine,"interpolation",std::string("linear"));
        for(unsigned i=0;i<16;++i){const double t=double(i)/15;set(sine,("offset"+std::to_string(i)).c_str(),t);const auto value=.5+.5*std::sin(2*std::numbers::pi*t);p->Set(P("scene.textures."+sine+".value"+std::to_string(i))(value,value,value));}
        auto wave=sub(mul(sine,constant(2)),one);
        const auto n=node("shadingnormal");const auto* m=f.model_to_world;
        const double determinant=m[0]*(m[5]*m[10]-m[6]*m[9])-m[1]*(m[4]*m[10]-m[6]*m[8])+m[2]*(m[4]*m[9]-m[5]*m[8]);
        if(!std::isfinite(determinant)||std::abs(determinant)<1e-12)throw InvalidInput("Singular layer placement");
        const double inv[16]={(m[5]*m[10]-m[6]*m[9])/determinant,(m[2]*m[9]-m[1]*m[10])/determinant,(m[1]*m[6]-m[2]*m[5])/determinant,0,(m[6]*m[8]-m[4]*m[10])/determinant,(m[0]*m[10]-m[2]*m[8])/determinant,(m[2]*m[4]-m[0]*m[6])/determinant,0,(m[4]*m[9]-m[5]*m[8])/determinant,(m[1]*m[8]-m[0]*m[9])/determinant,(m[0]*m[5]-m[1]*m[4])/determinant,0,0,0,0,1};
        bool rigid=true;for(size_t r=0;r<3;++r)for(size_t c=0;c<3;++c){double d=0;for(size_t k=0;k<3;++k)d+=m[k*4+r]*m[k*4+c];rigid&=std::abs(d-(r==c?1.:0.))<1e-10;}
        auto normal_length=one;if(!rigid){auto nb=matrix(m,n,true);normal_length=pow(dot(nb,nb),.5);}
        auto nz=div(dot(vector({m[2],m[6],m[10]}),n),normal_length);
        std::string footprint,view;
        if(f.projection){footprint=constant(f.vertical/f.height);V direction;double length=0;for(size_t i=0;i<3;++i){direction[i]=f.camera_position_mm[i]-f.camera_target_mm[i];length+=direction[i]*direction[i];}for(auto& c:direction)c/=std::sqrt(length);view=vector(direction);}
        else {auto delta=sub(vector({f.camera_position_mm[0],f.camera_position_mm[1],f.camera_position_mm[2]}),node("position"));auto distance=pow(dot(delta,delta),.5);view=div(delta,distance);footprint=mul(distance,constant(2*std::tan(f.vertical*std::numbers::pi/360)/f.height));}
        auto projected=div(mul(constant(std::min(first,pitch)),unary("abs",dot(n,view))),mul(footprint,constant(std::sqrt(2.))));
        auto fraction=clamp(div(sub(projected,constant(2)),constant(2)),0,1);
        auto gain=metadata.at("layer_filter").at("enabled").get<bool>()?mul(mul(fraction,fraction),sub(constant(3),mul(constant(2),fraction))):one;
        auto derivative=mul(mul(div(constant(std::numbers::pi*depth),local_pitch),wave),mul(gain,sub(one,mul(nz,nz))));
        std::array<std::string,3> components;for(unsigned i=0;i<3;++i){components[i]=node("hitpointtriangleaov");set(components[i],"dataindex",i);}
        auto edge=matrix(m,packed(components),false);auto en=dot(edge,n);
        auto tangent_length=pow(clamp(sub(dot(edge,edge),mul(en,en)),1e-30,1e30),.5);
        auto axis=vector({inv[8],inv[9],inv[10]}),axis_n=dot(axis,n);
        auto axis_x=div(sub(dot(axis,edge),mul(axis_n,en)),tangent_length);
        auto axis_y=div(dot(axis,cross(n,edge)),tangent_length);
        auto local_x=mul(constant(-1),mul(derivative,axis_x)),local_y=mul(constant(-1),mul(derivative,axis_y));
        auto local_z=sub(div(add(one,mul(nz,derivative)),normal_length),mul(derivative,axis_n));
        // Common constant keeps channels in [0,1]. SDK normalmap normalizes the
        // final vector; the scale cannot alter its direction.
        double inverse_bound=0;for(size_t r=0;r<3;++r)for(size_t c=0;c<3;++c)inverse_bound+=inv[r*4+c]*inv[r*4+c];
        const auto scale=constant(.5/(std::sqrt(inverse_bound)*(1+2*std::numbers::pi*depth/std::min(first,pitch))));
        const auto encoded=packed({add(half,mul(scale,local_x)),add(half,mul(scale,local_y)),add(half,mul(scale,local_z))});
        nodes["tn_layers"]["type"]="normalmap";p->Set(P("scene.textures.tn_layers.type")("normalmap"));set("tn_layers","texture",encoded);
        // Pinned CompileTextureOps accounting, memoized before SDK allocation.
        for(auto it=nodes.begin();it!=nodes.end();++it)for(int mode=0;mode<3;++mode){const auto c=cost(it.key(),mode);stack_floats=std::max(stack_floats,c.stack);operation_count=sum(operation_count,c.ops);}
        return std::move(p);
    }
};
}
