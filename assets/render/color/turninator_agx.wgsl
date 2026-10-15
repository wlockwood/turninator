// turninator-agx-v1. Adapted from Filament; see NOTICE.md (Apache-2.0/MIT).
// Host MUST validate finite straight RGBA, alpha 0..1, EV -32..32, and finite
// exposure-scaled RGB before dispatch. RGB is scene-linear Rec.709 D65.
// Output is display-linear Rec.709, or display-sRGB only via the explicit wrapper.
// Use the linear function with an sRGB render target; the sRGB function with an
// unorm target/readback. Never combine the sRGB wrapper with hardware sRGB encode.
const AGX_REC709_TO_REC2020: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.627507389, 0.0691077262, 0.0163964797),
    vec3<f32>(0.329277724, 0.919504464, 0.0880243331),
    vec3<f32>(0.0433038622, 0.0113590918, 0.895513475)
);
const AGX_INSET: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.856627153315983, 0.137318972929847, 0.11189821299995),
    vec3<f32>(0.0951212405381588, 0.761241990602591, 0.0767994186031903),
    vec3<f32>(0.0482516061458583, 0.101439036467562, 0.811302368396859)
);
const AGX_OUTSET: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(1.12710059, -0.141329765, -0.141329765),
    vec3<f32>(-0.110606641, 1.15782368, -0.110606641),
    vec3<f32>(-0.0164939389, -0.016493937, 1.25193644)
);
const AGX_REC2020_TO_REC709: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(1.66021347, -0.124552041, -0.0181550235),
    vec3<f32>(-0.58756429, 1.13294613, -0.100604787),
    vec3<f32>(-0.0728275776, -0.00834833086, 1.11883187)
);
fn turninator_agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2=x*x;
    let x4=x2*x2;
    let x6=x4*x2;
    return -17.86*x6*x + 78.01*x6 - 126.7*x4*x + 92.06*x4
         - 28.72*x2*x + 4.361*x2 - 0.1718*x + vec3<f32>(0.002857);
}
fn turninator_agx_linear(scene: vec4<f32>, exposure_ev: f32) -> vec4<f32> {
    var c=min(max(scene.rgb, vec3<f32>(0.0))*exp2(exposure_ev), vec3<f32>(65504.0));
    c=AGX_INSET*(AGX_REC709_TO_REC2020*c);
    c=clamp((log2(max(c, vec3<f32>(1e-10))) - vec3<f32>(-12.47393))
            /(4.026069 - (-12.47393)), vec3<f32>(0.0), vec3<f32>(1.0));
    c=AGX_OUTSET*turninator_agx_contrast(c);
    c=pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
    return vec4<f32>(clamp(AGX_REC2020_TO_REC709*c, vec3<f32>(0.0), vec3<f32>(1.0)), scene.a);
}
fn turninator_agx_srgb(scene: vec4<f32>, exposure_ev: f32) -> vec4<f32> {
    let linear=turninator_agx_linear(scene, exposure_ev);
    let encoded=select(1.055*pow(linear.rgb, vec3<f32>(1.0/2.4)) - vec3<f32>(0.055),
            12.92*linear.rgb, linear.rgb <= vec3<f32>(0.0031308));
    return vec4<f32>(encoded, linear.a);
}
// Projected layer filter constants; heights and positions stay in build mm.
const LAYER_START_PIXELS: f32=2.0;
const LAYER_END_PIXELS: f32=4.0;
const LAYER_FOOTPRINT_DIAGONAL: f32=1.4142135623730951;
const LAYER_FOOTPRINT_FLOOR_MM: f32=1e-12;
