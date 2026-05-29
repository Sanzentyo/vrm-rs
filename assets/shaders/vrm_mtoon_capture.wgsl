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

fn linearstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    return clamp((value - edge0) / max(edge1 - edge0, 0.00001), 0.0, 1.0);
}

fn surface_normal(input: VertexOutput) -> vec3<f32> {
    let geometric_normal = normalize(input.world_normal);
    if material.pipeline.z <= 0.0 {
        return geometric_normal;
    }
#ifdef VERTEX_TANGENTS
    let tangent = normalize(input.world_tangent.xyz);
    let bitangent = normalize(cross(geometric_normal, tangent) * input.world_tangent.w);
    let sampled = textureSample(normal_texture, normal_sampler, input.uv).xyz;
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
fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    let normal = surface_normal(input);
    let light_dir = normalize(vec3<f32>(-1.0, -1.0, -1.0));
    let ndotl = clamp(dot(normal, light_dir), -1.0, 1.0);

#ifdef VERTEX_UVS_A
    let uv = input.uv;
#else
    let uv = vec2<f32>(0.0, 0.0);
#endif

    let texel = textureSample(base_texture, base_sampler, uv);
    let alpha = material.base_color.a * texel.a;
    if material.pipeline.x > 0.5 && material.pipeline.x < 1.5 && alpha < material.pipeline.y {
        discard;
    }
    let opaque_alpha = select(alpha, 1.0, material.pipeline.x < 0.5);
    let diffuse = material.base_color.rgb * texel.rgb;

    if material.rim_params.w > 0.5 {
        let direct = diffuse * max(ndotl, 0.0);
        let ambient = diffuse * material.lighting.w;
        var pbr_color = direct + ambient + material.emissive.rgb;
        if material.outline_color.a >= 0.0 {
            pbr_color = material.outline_color.rgb * mix(vec3<f32>(1.0), pbr_color, material.outline_color.a);
        }
        return vec4<f32>(pbr_color, opaque_alpha);
    }

    let shade_texel = textureSample(shade_texture, shade_sampler, uv);
    let shade = material.shade_color.rgb * shade_texel.rgb;
    let shift_texel = textureSample(shading_shift_texture, shading_shift_sampler, uv).r;
    let shift = material.shading.x + shift_texel * material.shading.w;
    let toon = linearstep(
        -1.0 + material.shading.y,
        1.0 - material.shading.y,
        ndotl + shift,
    );
    let direct = mix(shade, diffuse, toon);
    let ambient = diffuse * (material.lighting.y + material.lighting.z * material.shading.z);

    let view_dir = normalize(view.world_position.xyz - input.world_position.xyz);
    let matcap_x = normalize(vec3<f32>(view_dir.z, 0.0, -view_dir.x));
    let matcap_y = cross(view_dir, matcap_x);
    let matcap_uv = vec2<f32>(
        0.5 + 0.5 * dot(matcap_x, normal),
        0.5 - 0.5 * dot(matcap_y, normal),
    );
    let matcap = textureSample(matcap_texture, matcap_sampler, matcap_uv).rgb * material.matcap_factor.rgb;
    let rim_base = material.rim_color.rgb * pow(
        clamp(1.0 - dot(view_dir, normal) + material.rim_params.z, 0.0, 1.0),
        material.rim_params.y,
    );
    let rim_texel = textureSample(rim_texture, rim_sampler, uv).rgb;
    let rim_mix = mix(vec3<f32>(1.0), vec3<f32>(1.03183099), material.rim_params.x);
    let rim = (rim_base + matcap) * rim_texel * rim_mix;
    var color = (direct + ambient + rim + material.emissive.rgb) * material.lighting.x;
    if material.outline_color.a >= 0.0 {
        color = material.outline_color.rgb * mix(vec3<f32>(1.0), color, material.outline_color.a);
    }
    return vec4<f32>(color, opaque_alpha);
}
