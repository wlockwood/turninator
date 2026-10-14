@group(0) @binding(0) var accumulation:texture_2d<f32>;
@group(0) @binding(1) var revealage:texture_2d<f32>;
@vertex fn vs_main(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    let x=f32((index<<1u)&2u);let y=f32(index&2u);return vec4<f32>(x*2.-1.,y*2.-1.,0.,1.);
}
@fragment fn fs_main(@builtin(position) position:vec4<f32>)->@location(0) vec4<f32> {
    let pixel=vec2<i32>(position.xy);let accumulated=textureLoad(accumulation,pixel,0);let reveal=clamp(textureLoad(revealage,pixel,0).r,0.,1.);
    let average=accumulated.rgb/max(accumulated.a,1e-8);
    let background=pow(vec3<f32>(0.095,0.11,0.14),vec3<f32>(2.2));
    let linear=average*(1.-reveal)+background*reveal;
    return vec4<f32>(pow(clamp(linear,vec3<f32>(0.),vec3<f32>(1.)),vec3<f32>(1./2.2)),1.);
}
