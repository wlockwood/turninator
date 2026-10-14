struct Uniforms {
    right: vec4<f32>, up: vec4<f32>, back: vec4<f32>, camera_target: vec4<f32>,
    projection: vec4<f32>, clipping: vec4<f32>,
    row0: vec4<f32>, row1: vec4<f32>, row2: vec4<f32>, shift_spin: vec4<f32>,
    color_roughness: vec4<f32>, layers: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) build_z: f32,
}
fn spin(p: vec3<f32>) -> vec3<f32> {
    let s = sin(u.shift_spin.w); let c = cos(u.shift_spin.w);
    return vec3<f32>(c*p.x-s*p.y, s*p.x+c*p.y, p.z);
}
@vertex fn vs_main(@location(0) position: vec3<f32>) -> VertexOutput {
    var out: VertexOutput;
    let rotated = vec3<f32>(dot(u.row0.xyz, position), dot(u.row1.xyz, position), dot(u.row2.xyz, position));
    out.world = spin(rotated-u.shift_spin.xyz);
    out.build_z = position.z-u.clipping.z;
    let relative = out.world-u.camera_target.xyz;
    let view = vec3<f32>(dot(relative,u.right.xyz), dot(relative,u.up.xyz), u.camera_target.w-dot(relative,u.back.xyz));
    let n = u.clipping.x; let f = u.clipping.y;
    if u.projection.w > 0.5 {
        let half_span = u.projection.y*0.5;
        out.clip = vec4<f32>(view.x/(half_span*u.projection.z), view.y/half_span, (view.z-n)/(f-n), 1.);
    } else {
        out.clip = vec4<f32>(view.x/(u.projection.x*u.projection.z), view.y/u.projection.x, f/(f-n)*view.z-f*n/(f-n), view.z);
    }
    return out;
}
@fragment fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let camera = u.camera_target.xyz+u.back.xyz*u.camera_target.w;
    let view_dir = select(normalize(camera-input.world),u.back.xyz,u.projection.w>0.5);
    var normal = normalize(cross(dpdx(input.world), dpdy(input.world)));
    normal = select(-normal, normal, dot(normal,view_dir)>=0.);
    let build_axis = spin(vec3<f32>(u.row0.z,u.row1.z,u.row2.z));
    let build_alignment = dot(normal,build_axis);
    let cap_mask = clamp(1.-build_alignment*build_alignment,0.,1.);
    let first = input.build_z < u.layers.y;
    let height = select(u.layers.x,u.layers.y,first);
    let phase = select((input.build_z-u.layers.y)/u.layers.x,input.build_z/u.layers.y,first);
    let pixels_per_layer = height/max(fwidth(input.build_z),1e-8);
    let fade = smoothstep(2.,4.,pixels_per_layer);
    let derivative = 3.14159265359*u.layers.z/height*sin(6.28318530718*phase);
    let gradient = build_axis-normal*dot(normal,build_axis);
    normal = normalize(normal-gradient*derivative*cap_mask*fade*u.layers.w);
    let key = normalize(vec3<f32>(-0.4,-0.6,0.9));
    let fill = normalize(vec3<f32>(0.8,-0.2,0.45));
    let diffuse = 0.2+0.65*max(dot(normal,key),0.)+0.25*max(dot(normal,fill),0.);
    let half_vector = normalize(key+view_dir);
    let gloss = pow(max(dot(normal,half_vector),0.),mix(96.,8.,u.color_roughness.w))*mix(0.22,0.035,u.color_roughness.w);
    let linear = u.color_roughness.xyz*diffuse+vec3<f32>(gloss);
    return vec4<f32>(pow(clamp(linear,vec3<f32>(0.),vec3<f32>(1.)),vec3<f32>(1./2.2)),1.);
}
@fragment fn fs_edge() -> @location(0) vec4<f32> { return vec4<f32>(0.025,0.035,0.055,1.); }
