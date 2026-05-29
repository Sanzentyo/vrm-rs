#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
}

struct BevyMtoonUniform {
    base_color: vec4<f32>,
    shade_color: vec4<f32>,
    shading: vec4<f32>,
    emissive: vec4<f32>,
    matcap_factor: vec4<f32>,
    rim_color: vec4<f32>,
    rim_params: vec4<f32>,
    outline_color: vec4<f32>,
    pipeline: vec4<f32>,
    lighting: vec4<f32>,
    base_uv_transform: vec4<f32>,
    shade_uv_transform: vec4<f32>,
    shading_shift_uv_transform: vec4<f32>,
    normal_uv_transform: vec4<f32>,
    matcap_uv_transform: vec4<f32>,
    rim_uv_transform: vec4<f32>,
    emissive_uv_transform: vec4<f32>,
    uv_animation_mask_uv_transform: vec4<f32>,
    uv_rotation_a: vec4<f32>,
    uv_rotation_b: vec4<f32>,
    uv_animation: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var<uniform> material: BevyMtoonUniform;

@group(#{MATERIAL_BIND_GROUP}) @binding(1)
var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2)
var base_sampler: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(3)
var shade_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4)
var shade_sampler: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(5)
var shading_shift_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6)
var shading_shift_sampler: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(7)
var matcap_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8)
var matcap_sampler: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(9)
var rim_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10)
var rim_sampler: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(11)
var normal_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(12)
var normal_sampler: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(13)
var emissive_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(14)
var emissive_sampler: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(15)
var uv_animation_mask_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(16)
var uv_animation_mask_sampler: sampler;

fn linearstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    return clamp((value - edge0) / max(edge1 - edge0, 0.00001), 0.0, 1.0);
}

fn pbr_direct(
    diffuse: vec3<f32>,
    normal: vec3<f32>,
    view_dir: vec3<f32>,
    light_dir: vec3<f32>,
    metallic: f32,
    roughness: f32,
) -> vec3<f32> {
    let pi = 3.141592653589793;
    let n_dot_l = max(dot(normal, light_dir), 0.0);
    let n_dot_v = max(dot(normal, view_dir), 0.0001);
    let half_dir = normalize(light_dir + view_dir);
    let n_dot_h = max(dot(normal, half_dir), 0.0001);
    let v_dot_h = max(dot(view_dir, half_dir), 0.0);
    let rough = clamp(roughness, 0.04, 1.0);
    let alpha = rough * rough;
    let alpha2 = alpha * alpha;
    let denom = n_dot_h * n_dot_h * (alpha2 - 1.0) + 1.0;
    let distribution = alpha2 / max(pi * denom * denom, 0.0001);
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let geometry_l = n_dot_l / (n_dot_l * (1.0 - k) + k);
    let geometry_v = n_dot_v / (n_dot_v * (1.0 - k) + k);
    let geometry = geometry_l * geometry_v;
    let f0 = mix(vec3<f32>(0.04), diffuse, metallic);
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - v_dot_h, 5.0);
    let specular = distribution * geometry * fresnel / max(4.0 * n_dot_l * n_dot_v, 0.0001);
    let diffuse_lobe = diffuse * (1.0 - metallic) / pi;
    return (diffuse_lobe + specular) * pi * n_dot_l;
}

fn transform_uv(uv: vec2<f32>, offset_scale: vec4<f32>, rotation: f32) -> vec2<f32> {
    let scaled = uv * offset_scale.zw;
    let c = cos(rotation);
    let s = sin(rotation);
    return vec2<f32>(
        c * scaled.x - s * scaled.y + offset_scale.x,
        s * scaled.x + c * scaled.y + offset_scale.y,
    );
}

fn animate_uv(uv: vec2<f32>) -> vec2<f32> {
    let mask_uv = transform_uv(
        uv,
        material.uv_animation_mask_uv_transform,
        material.uv_rotation_b.z,
    );
    let mask = textureSample(uv_animation_mask_texture, uv_animation_mask_sampler, mask_uv).b;
    let phase = material.uv_animation.z * mask;
    let c = cos(phase);
    let s = sin(phase);
    let centered = uv - vec2<f32>(0.5, 0.5);
    let rotated = vec2<f32>(
        c * centered.x + s * centered.y,
        -s * centered.x + c * centered.y,
    ) + vec2<f32>(0.5, 0.5);
    return rotated + material.uv_animation.xy * mask;
}

fn surface_normal(input: VertexOutput, front_facing: bool, normal_uv: vec2<f32>) -> vec3<f32> {
    let face_sign = select(-1.0, 1.0, front_facing || material.pipeline.w < 0.5);
    let geometric_normal = normalize(input.world_normal) * face_sign;
    if material.pipeline.z <= 0.0 {
        return geometric_normal;
    }
#ifdef VERTEX_TANGENTS
    let tangent = normalize(input.world_tangent.xyz) * face_sign;
    let bitangent = normalize(cross(geometric_normal, tangent) * input.world_tangent.w) * face_sign;
    let sampled = textureSample(normal_texture, normal_sampler, normal_uv).xyz;
    let tangent_normal = vec3<f32>(
        (sampled.x * 2.0 - 1.0) * material.pipeline.z,
        (sampled.y * 2.0 - 1.0) * material.pipeline.z,
        sampled.z * 2.0 - 1.0,
    );
    return normalize(
        tangent * tangent_normal.x +
        bitangent * tangent_normal.y +
        geometric_normal * tangent_normal.z,
    );
#else
    return geometric_normal;
#endif
}

@fragment
fn fragment(input: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
#ifdef VERTEX_UVS_A
    let uv = input.uv;
#else
    let uv = vec2<f32>(0.0, 0.0);
#endif

    let animated_uv = animate_uv(uv);
    let base_uv = transform_uv(animated_uv, material.base_uv_transform, material.uv_rotation_a.x);
    let shade_uv = transform_uv(animated_uv, material.shade_uv_transform, material.uv_rotation_a.y);
    let shading_shift_uv = transform_uv(animated_uv, material.shading_shift_uv_transform, material.uv_rotation_a.z);
    let normal_uv = transform_uv(animated_uv, material.normal_uv_transform, material.uv_rotation_a.w);
    let rim_uv = transform_uv(animated_uv, material.rim_uv_transform, material.uv_rotation_b.x);
    let emissive_uv = transform_uv(animated_uv, material.emissive_uv_transform, material.uv_rotation_b.y);

    let normal = surface_normal(input, front_facing, normal_uv);
    let light_dir = normalize(vec3<f32>(-1.0, -1.0, -1.0));
    let ndotl = clamp(dot(normal, light_dir), -1.0, 1.0);

    let texel = textureSample(base_texture, base_sampler, base_uv);
    let emissive_texel = textureSample(emissive_texture, emissive_sampler, emissive_uv).rgb;
    let alpha = material.base_color.a * texel.a;
    if material.pipeline.x > 0.5 && material.pipeline.x < 1.5 && alpha < material.pipeline.y {
        discard;
    }
    let opaque_alpha = select(alpha, 1.0, material.pipeline.x < 1.5);
    let diffuse = material.base_color.rgb * texel.rgb;
    let view_dir = normalize(view.world_position.xyz - input.world_position.xyz);

    if material.rim_params.w > 0.5 {
        let direct = pbr_direct(
            diffuse,
            normal,
            view_dir,
            light_dir,
            material.matcap_factor.w,
            material.rim_color.w,
        );
        let ambient = diffuse * (1.0 - material.matcap_factor.w) * material.lighting.w;
        var pbr_color = direct + ambient + material.emissive.rgb * emissive_texel;
        if material.outline_color.a >= 0.0 {
            pbr_color = material.outline_color.rgb * mix(vec3<f32>(1.0), pbr_color, material.outline_color.a);
        }
        return vec4<f32>(pbr_color, opaque_alpha);
    }

    let shade_texel = textureSample(shade_texture, shade_sampler, shade_uv);
    let shade = material.shade_color.rgb * shade_texel.rgb;
    let shift_texel = textureSample(shading_shift_texture, shading_shift_sampler, shading_shift_uv).r;
    let shift = material.shading.x + shift_texel * material.shading.w;
    let toon = linearstep(
        -1.0 + material.shading.y,
        1.0 - material.shading.y,
        ndotl + shift,
    );
    let direct = mix(shade, diffuse, toon);
    let ambient = diffuse * (material.lighting.y + material.lighting.z * material.shading.z);

    let matcap_x = normalize(vec3<f32>(view_dir.z, 0.0, -view_dir.x));
    let matcap_y = cross(view_dir, matcap_x);
    let raw_matcap_uv = vec2<f32>(
        0.5 + 0.5 * dot(matcap_x, normal),
        0.5 - 0.5 * dot(matcap_y, normal),
    );
    let matcap_uv = transform_uv(raw_matcap_uv, material.matcap_uv_transform, material.uv_rotation_b.w);
    let matcap = textureSample(matcap_texture, matcap_sampler, matcap_uv).rgb * material.matcap_factor.rgb;
    let rim_base = material.rim_color.rgb * pow(
        clamp(1.0 - dot(view_dir, normal) + material.rim_params.z, 0.0, 1.0),
        material.rim_params.y,
    );
    let rim_texel = textureSample(rim_texture, rim_sampler, rim_uv).rgb;
    let rim_mix = mix(vec3<f32>(1.0), vec3<f32>(1.03183099), material.rim_params.x);
    let rim = (rim_base + matcap) * rim_texel * rim_mix;
    var color = (direct + ambient + rim + material.emissive.rgb * emissive_texel) * material.lighting.x;
    if material.outline_color.a >= 0.0 {
        color = material.outline_color.rgb * mix(vec3<f32>(1.0), color, material.outline_color.a);
    }
    return vec4<f32>(color, opaque_alpha);
}
