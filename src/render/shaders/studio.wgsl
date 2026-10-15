// wgpu-studio-v1: build-space geometry, world-space PBR and deterministic AA.
struct Uniforms {
    right: vec4<f32>, up: vec4<f32>, forward: vec4<f32>, eye: vec4<f32>,
    projection: vec4<f32>, clipping: vec4<f32>,
    row0: vec4<f32>, row1: vec4<f32>, row2: vec4<f32>,
    light_right: vec4<f32>, light_up: vec4<f32>, light_back: vec4<f32>, center: vec4<f32>,
    screen: vec4<f32>, layers: vec4<f32>, build: vec4<f32>, sample: vec4<f32>, quality: vec4<f32>,
}
struct Material {
    base: vec4<f32>, surface: vec4<f32>, volume: vec4<f32>, absorption: vec4<f32>, flags: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage,read> materials: array<Material>;
@group(1) @binding(0) var shadow_depth: texture_depth_2d;
@group(1) @binding(1) var scene_depth: texture_depth_2d;

struct Vertex {
    @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>,
    @location(2) build: vec3<f32>, @location(3) material: u32,
}
struct Surface {
    @builtin(position) clip: vec4<f32>, @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>, @location(2) build: vec3<f32>,
    @location(3) @interpolate(flat) material: u32,
}
fn rotate(p:vec3<f32>)->vec3<f32> {return vec3<f32>(dot(u.row0.xyz,p),dot(u.row1.xyz,p),dot(u.row2.xyz,p));}
fn project(world:vec3<f32>)->vec4<f32> {
    let p=world-u.eye.xyz;
    let v=vec3<f32>(dot(p,u.right.xyz),dot(p,u.up.xyz),dot(p,u.forward.xyz));
    let n=u.clipping.x;let f=u.clipping.y;
    var clip=vec4<f32>(v.x/u.projection.x,v.y/u.projection.y,(v.z-n)/(f-n),1.);
    if u.projection.z<0.5 {clip=vec4<f32>(clip.xy,f/(f-n)*v.z-f*n/(f-n),v.z);}
    clip.x+=u.sample.x*2./u.screen.x*clip.w;
    clip.y+=u.sample.y*2./u.screen.y*clip.w;
    return clip;
}
fn light_project(world:vec3<f32>)->vec4<f32> {
    let p=world-u.center.xyz;
    return vec4<f32>(dot(p,u.light_right.xyz)/(u.center.w*2.),dot(p,u.light_up.xyz)/(u.center.w*2.),0.5-dot(p,u.light_back.xyz)/(u.center.w*6.),1.);
}
@vertex fn vs_main(v:Vertex)->Surface {
    var o:Surface;o.world=rotate(v.position)+vec3<f32>(u.row0.w,u.row1.w,u.row2.w);
    o.clip=project(o.world);o.normal=rotate(v.normal);o.build=v.build;o.material=v.material;return o;
}
@vertex fn vs_shadow(v:Vertex)->@builtin(position) vec4<f32> {
    return light_project(rotate(v.position)+vec3<f32>(u.row0.w,u.row1.w,u.row2.w));
}
@vertex fn vs_floor(@builtin(vertex_index) i:u32)->Surface {
    let corners=array<vec2<f32>,6>(vec2<f32>(-1.,-1.),vec2<f32>(1.,-1.),vec2<f32>(1.,1.),vec2<f32>(-1.,-1.),vec2<f32>(1.,1.),vec2<f32>(-1.,1.));
    var o:Surface;o.world=vec3<f32>(corners[i]*u.center.w*100.,-u.center.w*0.00001);
    o.clip=project(o.world);o.normal=vec3<f32>(0.,0.,1.);o.build=vec3<f32>(0.);o.material=0xffffffffu;return o;
}
@vertex fn vs_fullscreen(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    return vec4<f32>(vec2<f32>(f32((i<<1u)&2u),f32(i&2u))*2.-1.,0.,1.);
}
fn environment(d:vec3<f32>,rough:f32)->vec3<f32> {
    // Analytic radiance field: three finite studio softboxes plus a hemisphere.
    // Convolving Gaussian lobes with roughness preserves broad reflected energy.
    let dark=u.screen.w>0.5&&u.screen.w<1.5;
    var result=select(vec3<f32>(0.11,0.13,0.17),vec3<f32>(0.012,0.016,0.025),dark)*(0.65+0.35*max(d.z,0.));
    let dirs=array<vec3<f32>,3>(normalize(vec3<f32>(-2.5,-3.,4.)),normalize(vec3<f32>(3.,-1.,2.)),normalize(vec3<f32>(0.,3.,3.)));
    let colors=array<vec3<f32>,3>(vec3<f32>(5.,4.6,4.2),vec3<f32>(1.3,1.6,2.),vec3<f32>(4.5,4.7,5.));
    let widths=array<f32,3>(0.035,0.06,0.025);
    for(var i=0u;i<3u;i++) {
        let variance=widths[i]+rough*rough*0.7;
        result+=colors[i]*widths[i]/variance*exp((dot(d,dirs[i])-1.)/variance);
    }
    return result;
}
@fragment fn fs_background(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    if u.screen.z>0.5 {return vec4<f32>(0.);}
    if u.screen.w>1.5 {return vec4<f32>(1.);}
    let ndc=vec2<f32>(p.x/u.screen.x*2.-1.,1.-p.y/u.screen.y*2.);
    let d=normalize(u.forward.xyz+u.right.xyz*ndc.x*u.projection.x+u.up.xyz*ndc.y*u.projection.y);
    return vec4<f32>(environment(d,1.),1.);
}
fn shadow(world:vec3<f32>,n:vec3<f32>)->f32 {
    let facing=dot(n,u.light_back.xyz);
    if facing<=0.0001 {return 1.;}
    let clip=light_project(world+n*u.center.w*0.0003);
    let uv=vec2<f32>(clip.x*0.5+0.5,0.5-clip.y*0.5);
    let size=i32(u.quality.x);let center=vec2<i32>(uv*f32(size));
    let radius=i32(u.sample.w);var lit=0.;var count=0.;
    // Compare each tap against this receiver plane at that texel's center.
    // A constant receiver depth makes tilted flat faces self-shadow across PCF taps.
    let gradient=(2./3.)*vec2<f32>(dot(n,u.light_right.xyz),-dot(n,u.light_up.xyz))/(facing*f32(size));
    for(var y=-radius;y<=radius;y++) {for(var x=-radius;x<=radius;x++) {
        let at=center+vec2<i32>(x,y);
        if any(at<vec2<i32>(0))||any(at>=vec2<i32>(size)) {lit+=1.;}
        else {
            let receiver=clip.z+dot(vec2<f32>(at)+0.5-uv*f32(size),gradient);
            lit+=select(0.,1.,receiver-0.0002<=textureLoad(shadow_depth,at,0));
        }
        count+=1.;
    }}return lit/count;
}
fn reconstruct(pixel:vec2<f32>,depth:f32)->vec3<f32> {
    let n=u.clipping.x;let f=u.clipping.y;
    let z=select(f*n/(f-depth*(f-n)),n+depth*(f-n),u.projection.z>0.5);
    let ndc=vec2<f32>((pixel.x-u.sample.x)/u.screen.x*2.-1.,1.-(pixel.y+u.sample.y)/u.screen.y*2.);
    let scale=select(z,1.,u.projection.z>0.5);
    return u.eye.xyz+u.forward.xyz*z+u.right.xyz*ndc.x*u.projection.x*scale+u.up.xyz*ndc.y*u.projection.y*scale;
}
fn ambient_occlusion(p:Surface,n:vec3<f32>)->f32 {
    let radius=u.center.w*0.10;
    let depth=dot(p.world-u.eye.xyz,u.forward.xyz);
    let span=u.projection.y*select(depth,1.,u.projection.z>0.5);
    let pixels=clamp(radius*u.screen.y/(2.*span),2.,96.);
    var occ=0.;let count=u32(u.build.w);
    for(var i=0u;i<count;i++) {
        let t=(f32(i)+0.5)/f32(count);let angle=f32(i)*2.39996323;
        let delta=vec2<f32>(cos(angle),sin(angle))*sqrt(t)*pixels;
        let at=vec2<i32>(p.clip.xy+delta);
        if all(at>=vec2<i32>(0))&&all(at<vec2<i32>(u.screen.xy)) {
            let d=textureLoad(scene_depth,at,0);
            if d<1. {
                let v=reconstruct(vec2<f32>(at)+0.5,d)-p.world;let dist=length(v);
                let horizon=max(dot(n,v)/max(dist,0.00001)-0.04,0.);
                occ+=horizon*(1.-smoothstep(radius*0.25,radius,dist));
            }
        }
    }return clamp(1.-2.5*occ/f32(count),0.25,1.);
}
fn fresnel(f0:vec3<f32>,v:f32)->vec3<f32> {return f0+(1.-f0)*pow(1.-clamp(v,0.,1.),5.);}
fn brdf(n:vec3<f32>,v:vec3<f32>,l:vec3<f32>,t:vec3<f32>,m:Material)->vec3<f32> {
    let h=normalize(l+v);let nv=max(dot(n,v),0.001);let nl=max(dot(n,l),0.);
    let nh=max(dot(n,h),0.);let vh=max(dot(v,h),0.);
    let a=max(m.base.w*m.base.w,0.002);let aspect=sqrt(1.-0.9*m.surface.y);
    let ax=a/aspect;let ay=a*aspect;let b=cross(n,t);
    let q=pow(dot(t,h)/ax,2.)+pow(dot(b,h)/ay,2.)+nh*nh;
    let d=1./max(3.14159265*ax*ay*q*q,0.00001);
    let k=(m.base.w+1.)*(m.base.w+1.)/8.;
    let g=nv/(nv*(1.-k)+k)*nl/(nl*(1.-k)+k);
    let dielectric=pow((m.volume.y-1.)/(m.volume.y+1.),2.);
    let f=fresnel(mix(vec3<f32>(dielectric),m.base.rgb,m.surface.x),vh);
    let diffuse=(1.-f)*(1.-m.surface.x)*(1.-m.volume.x)*m.base.rgb/3.14159265;
    let coat_a=max(m.surface.w*m.surface.w,0.002);let ca2=coat_a*coat_a;
    let coat_d=ca2/(3.14159265*pow(nh*nh*(ca2-1.)+1.,2.));
    let coat=m.surface.z*coat_d*g*fresnel(vec3<f32>(0.04),vh)/(4.*nv*max(nl,0.001));
    return (diffuse+f*d*g/(4.*nv*max(nl,0.001))+coat)*nl;
}
@fragment fn fs_main(p:Surface,@builtin(front_facing) front:bool)->@location(0) vec4<f32> {
    // Read-only prepass supports screen-space AO without a texture feedback loop.
    if p.clip.z>textureLoad(scene_depth,vec2<i32>(p.clip.xy),0)+0.0000001 {discard;}
    var m:Material;
    let floor=p.material==0xffffffffu;
    if floor {
        m=Material(vec4<f32>(select(vec3<f32>(0.18,0.20,0.24),vec3<f32>(0.025,0.03,0.04),u.screen.w>0.5),0.7),vec4<f32>(0.,0.,0.,0.12),vec4<f32>(0.,1.5,0.,0.),vec4<f32>(1.),vec4<f32>(0.));
    }else {m=materials[p.material];}
    if m.flags.x>0.5 {return vec4<f32>(m.base.rgb,1.);}
    var n=normalize(p.normal);let v=select(normalize(u.eye.xyz-p.world),-u.forward.xyz,u.projection.z>0.5);
    n=select(-n,n,dot(n,v)>=0.);
    let axis=normalize(vec3<f32>(u.row0.z,u.row1.z,u.row2.z));
    let z=p.build.z-u.build.x;let first=z<u.layers.y;
    let height=select(u.layers.x,u.layers.y,first);
    let phase=select((z-u.layers.y)/u.layers.x,z/u.layers.y,first);
    let footprint=max(length(vec2<f32>(dpdx(z),dpdy(z)))*LAYER_FOOTPRINT_DIAGONAL,LAYER_FOOTPRINT_FLOOR_MM);
    let gain=select(1.,smoothstep(LAYER_START_PIXELS,LAYER_END_PIXELS,height/footprint),u.build.z>0.5);
    let mask=1.-pow(dot(n,axis),2.);
    let slope=3.14159265*u.layers.z/height*sin(6.28318531*phase)*mask*gain*u.layers.w;
    if !floor {n=normalize(n-(axis-n*dot(n,axis))*slope);}
    var tangent=cross(axis,n);
    if dot(tangent,tangent)<0.0001 {
        // The least-aligned cardinal axis cannot be parallel to a unit normal.
        // A fixed Y fallback is singular for legal tilted faces facing world Y.
        let a=abs(n);var basis=vec3<f32>(1.,0.,0.);
        if a.y<a.x&&a.y<=a.z {basis=vec3<f32>(0.,1.,0.);}
        else if a.z<a.x&&a.z<a.y {basis=vec3<f32>(0.,0.,1.);}
        tangent=cross(basis,n);
    }
    tangent*=inverseSqrt(max(dot(tangent,tangent),0.00000000000000000001));
    let key=normalize(vec3<f32>(-2.5,-3.,4.));let fill=normalize(vec3<f32>(3.,-1.,2.));let rim=normalize(vec3<f32>(0.,3.,3.));
    let visibility=shadow(p.world,normalize(p.normal));let ao=ambient_occlusion(p,n);
    var color=brdf(n,v,key,tangent,m)*vec3<f32>(3.6,3.3,3.)*visibility+brdf(n,v,fill,tangent,m)*vec3<f32>(0.9,1.1,1.4)+brdf(n,v,rim,tangent,m)*vec3<f32>(2.7,2.8,3.);
    let f0=mix(vec3<f32>(pow((m.volume.y-1.)/(m.volume.y+1.),2.)),m.base.rgb,m.surface.x);
    let f=fresnel(f0,max(dot(n,v),0.));
    // Anisotropic reflected lobe follows build-layer tangent for silk.
    let reflected=normalize(reflect(-v,n)+tangent*m.surface.y*m.base.w*0.3);
    color+=environment(reflected,m.base.w)*f*ao;
    color+=environment(n,1.)*m.base.rgb*(1.-m.surface.x)*(1.-m.volume.x)*ao;
    color+=environment(reflect(-v,n),m.surface.w)*m.surface.z*fresnel(vec3<f32>(0.04),max(dot(n,v),0.))*ao;
    // Raster transmission: one representative thickness, no refraction/volume tracing.
    let thickness=u.center.w*0.35;
    let absorb=exp(-max(vec3<f32>(1.)-m.absorption.rgb,vec3<f32>(0.))*m.volume.z*thickness);
    let transmitted=m.volume.x*absorb*exp(-m.volume.w*thickness);
    color+=environment(-v,m.base.w)*transmitted*(1.-f);
    let alpha=clamp(1.-dot(transmitted,vec3<f32>(0.2126,0.7152,0.0722))*(1.-max(f.r,max(f.g,f.b))),0.06,1.);
    return vec4<f32>(max(color,vec3<f32>(0.))*alpha,alpha);
}
