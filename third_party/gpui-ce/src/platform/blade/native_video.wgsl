// Exactly the shared sampling and input geometry contract. No CPU color loop.
struct NativeParams {
    target_rect: vec4<f32>, clip: vec4<f32>, uv0: vec4<f32>, uv1: vec4<f32>,
    visible_uv: vec4<f32>, red: vec4<f32>, green: vec4<f32>, blue: vec4<f32>, view: vec4<f32>,
}
var<uniform> native_params: NativeParams;
var native_y: texture_2d<f32>;
var native_uv: texture_2d<f32>;
var native_sampler: sampler;
// H.273 tables 2/3: primaries 5 are BT.470 B/G (D65); transfer 6 is
// functionally BT.709. Convert in linear light, then return BT.709 signal RGB.
// Matrix derived from the declared xy primaries, not inferred from resolution.
fn bt470bg_to_bt709(signal: vec3<f32>) -> vec3<f32> {
    let alpha = 1.099296826809442;
    let beta = 0.018053968510807;
    let linear = select(signal / 4.5, pow((signal + vec3<f32>(alpha - 1.0)) / alpha, vec3<f32>(1.0 / 0.45)), signal >= vec3<f32>(4.5 * beta));
    let converted = clamp(vec3<f32>(
        dot(vec3<f32>(1.044043209, -0.044043209, 0.0), linear),
        linear.g,
        dot(vec3<f32>(0.0, 0.011793378, 0.988206622), linear)
    ), vec3<f32>(0.0), vec3<f32>(1.0));
    return select(4.5 * converted, alpha * pow(converted, vec3<f32>(0.45)) - vec3<f32>(alpha - 1.0), converted >= vec3<f32>(beta));
}
struct NativeVarying { @builtin(position) position:vec4<f32>, @location(0) unit:vec2<f32>, @location(1) clipped:vec4<f32>, }
@vertex fn vs_native(@builtin(vertex_index) vertex:u32)->NativeVarying {
    let unit=vec2<f32>(f32(vertex & 1u),0.5*f32(vertex & 2u));
    let point=native_params.target_rect.xy+unit*native_params.target_rect.zw;
    var out:NativeVarying;
    out.position=vec4<f32>(point/native_params.view.xy*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),0.0,1.0);
    out.unit=unit;
    out.clipped=vec4<f32>(point.x-native_params.clip.x,native_params.clip.x+native_params.clip.z-point.x,point.y-native_params.clip.y,native_params.clip.y+native_params.clip.w-point.y);
    return out;
}
@fragment fn fs_native(input:NativeVarying)->@location(0) vec4<f32> {
    if any(input.clipped < vec4<f32>(0.0)){discard;}
    let u=vec3<f32>(input.unit,1.0);
    let uv=clamp(vec2<f32>(dot(native_params.uv0.xyz,u),dot(native_params.uv1.xyz,u)),native_params.visible_uv.xy,native_params.visible_uv.zw);
    let y=textureSampleLevel(native_y,native_sampler,uv,0.0).r;
    let chroma=textureSampleLevel(native_uv,native_sampler,uv+vec2<f32>(native_params.view.z,0.0),0.0).rg;
    let value=vec4<f32>(y,chroma,1.0);
    var rgb=clamp(vec3<f32>(dot(native_params.red,value),dot(native_params.green,value),dot(native_params.blue,value)),vec3<f32>(0.0),vec3<f32>(1.0));
    if native_params.view.w > 0.5 { rgb = bt470bg_to_bt709(rgb); }
    return vec4<f32>(rgb,1.0);
}
