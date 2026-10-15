// Shared fullscreen resolve: accumulate scene-linear premultiplied radiance,
// unpremultiply before AgX, then exactly one sRGB transfer to an UNORM target.
struct Output { values:vec4<f32> }
@group(0) @binding(0) var input_image:texture_2d<f32>;
@group(0) @binding(1) var<uniform> output:Output;
@vertex fn vs_fullscreen(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    return vec4<f32>(vec2<f32>(f32((i<<1u)&2u),f32(i&2u))*2.-1.,0.,1.);
}
@fragment fn fs_accumulate(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    return textureLoad(input_image,vec2<i32>(p.xy),0)*output.values.x;
}
@fragment fn fs_tonemap(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    let pixel=textureLoad(input_image,vec2<i32>(p.xy),0);
    if pixel.a<=0. {return vec4<f32>(0.);}
    return turninator_agx_srgb(vec4<f32>(pixel.rgb/pixel.a,clamp(pixel.a,0.,1.)),output.values.y);
}
