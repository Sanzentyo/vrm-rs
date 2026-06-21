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
    material_flags: vec4<f32>,
    material_flags2: vec4<f32>,
    pbr_params: vec4<f32>,
    owner_color: vec4<f32>,
    outline_color: vec4<f32>,
    pipeline: vec4<f32>,
    lighting: vec4<f32>,
    light_color: vec4<f32>,
    base_uv_transform: vec4<f32>,
    shade_uv_transform: vec4<f32>,
    shading_shift_uv_transform: vec4<f32>,
    normal_uv_transform: vec4<f32>,
    matcap_uv_transform: vec4<f32>,
    rim_uv_transform: vec4<f32>,
    emissive_uv_transform: vec4<f32>,
    occlusion_uv_transform: vec4<f32>,
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

@group(#{MATERIAL_BIND_GROUP}) @binding(17)
var occlusion_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(18)
var occlusion_sampler: sampler;

struct OwnerSampleOverrideRecord {
    pixel: vec2<u32>,
    sample: vec2<f32>,
    replacement_rgba: vec4<f32>,
    relation_to_expected: u32,
    geometry_flags: u32,
    sample_pass: u32,
    padding0: u32,
    geometry_ids: vec4<u32>,
    geometry_indices: vec4<u32>,
    barycentric_depth: vec4<f32>,
    geometry_uvs: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(20)
var<storage, read> owner_sample_overrides: array<OwnerSampleOverrideRecord>;

fn linearstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    return clamp((value - edge0) / max(edge1 - edge0, 0.00001), 0.0, 1.0);
}

fn linear_to_srgb_channel(value: f32) -> f32 {
    let x = clamp(value, 0.0, 1.0);
    return select(1.055 * pow(x, 1.0 / 2.4) - 0.055, 12.92 * x, x <= 0.0031308);
}

fn srgb_to_linear_channel(value: f32) -> f32 {
    let x = clamp(value, 0.0, 1.0);
    return select(pow((x + 0.055) / 1.055, 2.4), x / 12.92, x <= 0.04045);
}

fn srgb_to_linear_color(color: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        srgb_to_linear_channel(color.r),
        srgb_to_linear_channel(color.g),
        srgb_to_linear_channel(color.b),
    );
}

fn output_color(color: vec3<f32>, alpha: f32) -> vec4<f32> {
    return vec4<f32>(
        linear_to_srgb_channel(color.r),
        linear_to_srgb_channel(color.g),
        linear_to_srgb_channel(color.b),
        alpha,
    );
}

fn owner_id_output_color(color: vec3<f32>, alpha: f32) -> vec4<f32> {
    let rgb8 = round(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0) / 255.0;
    return vec4<f32>(rgb8, alpha);
}

const OWNER_SAMPLE_NO_OVERRIDE: u32 = 4294967295u;

fn owner_sample_override_index(fragment_position: vec4<f32>) -> u32 {
    let pixel = vec2<u32>(
        u32(floor(fragment_position.x)),
        u32(floor(fragment_position.y)),
    );
    for (var i = 0u; i < arrayLength(&owner_sample_overrides); i = i + 1u) {
        let record = owner_sample_overrides[i];
        if all(record.pixel == pixel) {
            return i;
        }
    }
    return OWNER_SAMPLE_NO_OVERRIDE;
}

fn owner_sample_has_geometry(index: u32) -> bool {
    return index != OWNER_SAMPLE_NO_OVERRIDE && owner_sample_overrides[index].geometry_flags != 0u;
}

fn owner_sample_raw_uv(index: u32, fallback: vec2<f32>) -> vec2<f32> {
    if owner_sample_has_geometry(index) {
        return owner_sample_overrides[index].geometry_uvs.xy;
    }
    return fallback;
}

fn owner_sample_base_uv(index: u32, fallback: vec2<f32>) -> vec2<f32> {
    if owner_sample_has_geometry(index) {
        return owner_sample_overrides[index].geometry_uvs.zw;
    }
    return fallback;
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

fn surface_normal(
    input: VertexOutput,
    front_facing: bool,
    normal_uv: vec2<f32>,
    normal_uv_dx: vec2<f32>,
    normal_uv_dy: vec2<f32>,
    use_explicit_texture_grad: bool,
) -> vec3<f32> {
    let face_sign = select(-1.0, 1.0, front_facing || material.pipeline.w < 0.5);
    let geometric_normal = normalize(input.world_normal) * face_sign;
    if material.pipeline.z <= 0.0 {
        return geometric_normal;
    }
    var sampled = textureSample(normal_texture, normal_sampler, normal_uv).xyz;
    if use_explicit_texture_grad {
        sampled = textureSampleGrad(normal_texture, normal_sampler, normal_uv, normal_uv_dx, normal_uv_dy).xyz;
    }
    let tangent_normal = vec3<f32>(
        (sampled.x * 2.0 - 1.0) * material.pipeline.z,
        (1.0 - sampled.y * 2.0) * material.pipeline.z,
        sampled.z * 2.0 - 1.0,
    );
    if material.material_flags.w > 0.5 {
        let use_view_derivative = material.material_flags2.y > 0.5;
        let view_position = (view.view_from_world * vec4<f32>(input.world_position.xyz, 1.0)).xyz;
        let view_normal = normalize((view.view_from_world * vec4<f32>(geometric_normal, 0.0)).xyz);
        let derivative_position = select(input.world_position.xyz, view_position, use_view_derivative);
        let derivative_normal = select(geometric_normal, view_normal, use_view_derivative);
        let q0 = dpdx(derivative_position);
        let q1 = dpdy(derivative_position);
        let st0 = select(dpdx(normal_uv), normal_uv_dx, use_explicit_texture_grad);
        let st1 = select(dpdy(normal_uv), normal_uv_dy, use_explicit_texture_grad);
        let q1perp = cross(q1, derivative_normal);
        let q0perp = cross(derivative_normal, q0);
        var tangent = q1perp * st0.x + q0perp * st1.x;
        var bitangent = q1perp * st0.y + q0perp * st1.y;
        let det = max(dot(tangent, tangent), dot(bitangent, bitangent));
        if det <= 0.0 {
            return geometric_normal;
        }
        let scale = 1.0 / sqrt(det);
        tangent = tangent * scale * face_sign;
        bitangent = bitangent * scale * face_sign;
        let perturbed = normalize(
            tangent * tangent_normal.x +
            bitangent * tangent_normal.y +
            derivative_normal * tangent_normal.z,
        );
        return select(
            perturbed,
            normalize((view.world_from_view * vec4<f32>(perturbed, 0.0)).xyz),
            use_view_derivative,
        );
    }
#ifdef VERTEX_TANGENTS
    let tangent = normalize(input.world_tangent.xyz) * face_sign;
    let bitangent = normalize(cross(geometric_normal, tangent) * input.world_tangent.w) * face_sign;
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
    let owner_sample_index = owner_sample_override_index(input.position);
    let use_owner_sample_geometry = owner_sample_has_geometry(owner_sample_index);
    let sampled_raw_uv = select(
        uv,
        owner_sample_raw_uv(owner_sample_index, uv),
        use_owner_sample_geometry,
    );

    let default_animated_uv = animate_uv(uv);
    let sampled_animated_uv = animate_uv(sampled_raw_uv);
    let default_base_uv = transform_uv(default_animated_uv, material.base_uv_transform, material.uv_rotation_a.x);
    let sampled_base_uv = transform_uv(sampled_animated_uv, material.base_uv_transform, material.uv_rotation_a.x);
    let base_uv = select(
        default_base_uv,
        owner_sample_base_uv(owner_sample_index, sampled_base_uv),
        use_owner_sample_geometry,
    );
    let default_shade_uv = transform_uv(default_animated_uv, material.shade_uv_transform, material.uv_rotation_a.y);
    let shade_uv = select(
        default_shade_uv,
        transform_uv(sampled_animated_uv, material.shade_uv_transform, material.uv_rotation_a.y),
        use_owner_sample_geometry,
    );
    let default_shading_shift_uv = transform_uv(default_animated_uv, material.shading_shift_uv_transform, material.uv_rotation_a.z);
    let shading_shift_uv = select(
        default_shading_shift_uv,
        transform_uv(sampled_animated_uv, material.shading_shift_uv_transform, material.uv_rotation_a.z),
        use_owner_sample_geometry,
    );
    let default_normal_uv = transform_uv(default_animated_uv, material.normal_uv_transform, material.uv_rotation_a.w);
    let normal_uv = select(
        default_normal_uv,
        transform_uv(sampled_animated_uv, material.normal_uv_transform, material.uv_rotation_a.w),
        use_owner_sample_geometry,
    );
    let default_rim_uv = transform_uv(default_animated_uv, material.rim_uv_transform, material.uv_rotation_b.x);
    let rim_uv = select(
        default_rim_uv,
        transform_uv(sampled_animated_uv, material.rim_uv_transform, material.uv_rotation_b.x),
        use_owner_sample_geometry,
    );
    let default_emissive_uv = transform_uv(default_animated_uv, material.emissive_uv_transform, material.uv_rotation_b.y);
    let emissive_uv = select(
        default_emissive_uv,
        transform_uv(sampled_animated_uv, material.emissive_uv_transform, material.uv_rotation_b.y),
        use_owner_sample_geometry,
    );
    let default_occlusion_uv = transform_uv(default_animated_uv, material.occlusion_uv_transform, material.uv_animation.w);
    let occlusion_uv = select(
        default_occlusion_uv,
        transform_uv(sampled_animated_uv, material.occlusion_uv_transform, material.uv_animation.w),
        use_owner_sample_geometry,
    );

    let normal = surface_normal(
        input,
        front_facing,
        normal_uv,
        dpdx(default_normal_uv),
        dpdy(default_normal_uv),
        use_owner_sample_geometry,
    );
    let light_dir = normalize(vec3<f32>(-1.0, 1.0, -1.0));
    let ndotl = clamp(dot(normal, light_dir), -1.0, 1.0);

    let default_base_sample_uv = select(
        default_base_uv,
        vec2<f32>(default_base_uv.x, 1.0 - default_base_uv.y),
        material.material_flags2.w > 1.5 && material.material_flags2.w < 2.5,
    );
    let base_sample_uv = select(
        base_uv,
        vec2<f32>(base_uv.x, 1.0 - base_uv.y),
        material.material_flags2.w > 1.5 && material.material_flags2.w < 2.5,
    );
    var raw_texel = textureSample(base_texture, base_sampler, base_sample_uv);
    if use_owner_sample_geometry {
        raw_texel = textureSampleGrad(
            base_texture,
            base_sampler,
            base_sample_uv,
            dpdx(default_base_sample_uv),
            dpdy(default_base_sample_uv),
        );
    }
    let texel_rgb = select(
        raw_texel.rgb,
        srgb_to_linear_color(raw_texel.rgb),
        material.material_flags2.w > 1.0 && material.material_flags2.w < 1.5,
    );
    let texel = vec4<f32>(texel_rgb, raw_texel.a);
    var emissive_texel = textureSample(emissive_texture, emissive_sampler, emissive_uv).rgb;
    if use_owner_sample_geometry {
        emissive_texel = textureSampleGrad(
            emissive_texture,
            emissive_sampler,
            emissive_uv,
            dpdx(default_emissive_uv),
            dpdy(default_emissive_uv),
        ).rgb;
    }
    let is_pbr_fallback = material.material_flags.y > 0.5;
    let alpha = material.base_color.a * raw_texel.a;
    if material.pipeline.x > 0.5 && material.pipeline.x < 1.5 && alpha < material.pipeline.y {
        discard;
    }
    let opaque_alpha = select(alpha, 1.0, material.pipeline.x < 1.5);
    if material.material_flags2.z > 0.5 {
        return vec4<f32>(vec3<f32>(1.0), opaque_alpha);
    }
    if material.material_flags2.w > 4.5 && material.material_flags2.w < 5.5 {
#ifdef VERTEX_COLORS
        return owner_id_output_color(input.color.rgb, 1.0);
#else
        return owner_id_output_color(material.owner_color.rgb, 1.0);
#endif
    }
    if material.material_flags2.w > 5.5 && material.material_flags2.w < 6.5 {
        return vec4<f32>(
            select(vec3<f32>(0.0), vec3<f32>(0.0, 1.0, 0.0), material.owner_color.a > 1.5),
            opaque_alpha,
        );
    }
    if material.material_flags2.w > 2.5 {
        if material.material_flags2.w > 3.5 {
            return output_color(vec3<f32>(base_sample_uv, 0.0), opaque_alpha);
        }
        return output_color(vec3<f32>(sampled_raw_uv, 0.0), opaque_alpha);
    }
    let diffuse = material.base_color.rgb * texel.rgb;
    if material.material_flags2.w < -0.5 {
        return output_color(material.base_color.rgb, opaque_alpha);
    }
    if material.material_flags2.w > 0.5 {
        return output_color(diffuse, opaque_alpha);
    }
    let view_dir = normalize(view.world_position.xyz - input.world_position.xyz);
    if material.material_flags2.x > 0.5 {
        return output_color(diffuse + material.emissive.rgb * emissive_texel, opaque_alpha);
    }

    if is_pbr_fallback {
        let direct = pbr_direct(
            diffuse,
            normal,
            view_dir,
            light_dir,
            material.pbr_params.x,
            material.pbr_params.y,
        ) * material.light_color.rgb * material.pbr_params.w;
        var occlusion_sample = textureSample(occlusion_texture, occlusion_sampler, occlusion_uv).r;
        if use_owner_sample_geometry {
            occlusion_sample = textureSampleGrad(
                occlusion_texture,
                occlusion_sampler,
                occlusion_uv,
                dpdx(default_occlusion_uv),
                dpdy(default_occlusion_uv),
            ).r;
        }
        let occlusion = (occlusion_sample - 1.0) * material.pbr_params.z + 1.0;
        let ambient = diffuse * (1.0 - material.pbr_params.x) * material.lighting.w * occlusion;
        var pbr_color = direct + ambient + material.emissive.rgb * emissive_texel;
        if material.outline_color.a >= 0.0 {
            pbr_color = material.outline_color.rgb * mix(vec3<f32>(1.0), pbr_color, material.outline_color.a);
        }
        return output_color(pbr_color, opaque_alpha);
    }

    var shade_texel = textureSample(shade_texture, shade_sampler, shade_uv);
    if use_owner_sample_geometry {
        shade_texel = textureSampleGrad(
            shade_texture,
            shade_sampler,
            shade_uv,
            dpdx(default_shade_uv),
            dpdy(default_shade_uv),
        );
    }
    let shade = material.shade_color.rgb * shade_texel.rgb;
    var shift_texel = textureSample(shading_shift_texture, shading_shift_sampler, shading_shift_uv).r;
    if use_owner_sample_geometry {
        shift_texel = textureSampleGrad(
            shading_shift_texture,
            shading_shift_sampler,
            shading_shift_uv,
            dpdx(default_shading_shift_uv),
            dpdy(default_shading_shift_uv),
        ).r;
    }
    let shift = material.shading.x + shift_texel * material.shading.w;
    let toon = linearstep(
        -1.0 + material.shading.y,
        1.0 - material.shading.y,
        ndotl + shift,
    );
    var direct = mix(shade, diffuse, toon) * material.light_color.rgb * material.pbr_params.w;
    if material.material_flags.x > 0.5 {
        direct = min(direct, diffuse);
    }
    var sampled_occlusion_texel = textureSample(occlusion_texture, occlusion_sampler, occlusion_uv).r;
    if use_owner_sample_geometry {
        sampled_occlusion_texel = textureSampleGrad(
            occlusion_texture,
            occlusion_sampler,
            occlusion_uv,
            dpdx(default_occlusion_uv),
            dpdy(default_occlusion_uv),
        ).r;
    }
    let sampled_occlusion = (sampled_occlusion_texel - 1.0) * material.pbr_params.z + 1.0;
    let occlusion = select(sampled_occlusion, 1.0, material.material_flags.z > 0.5);
    let ambient = diffuse * (material.lighting.y + material.lighting.z * material.shading.z) * occlusion;

    let matcap_view_position = (view.view_from_world * vec4<f32>(input.world_position.xyz, 1.0)).xyz;
    let matcap_view_dir = normalize(-matcap_view_position);
    let matcap_normal = normalize((view.view_from_world * vec4<f32>(normal, 0.0)).xyz);
    let matcap_x = normalize(vec3<f32>(matcap_view_dir.z, 0.0, -matcap_view_dir.x));
    let matcap_y = cross(matcap_view_dir, matcap_x);
    let raw_matcap_uv = vec2<f32>(
        0.5 + 0.5 * dot(matcap_x, matcap_normal),
        0.5 - 0.5 * dot(matcap_y, matcap_normal),
    );
    let matcap_uv = transform_uv(raw_matcap_uv, material.matcap_uv_transform, material.uv_rotation_b.w);
    let matcap = textureSample(matcap_texture, matcap_sampler, matcap_uv).rgb * material.matcap_factor.rgb;
    let rim_base = material.rim_color.rgb * pow(
        clamp(1.0 - dot(view_dir, normal) + material.rim_params.z, 0.0, 1.0),
        material.rim_params.y,
    );
    var rim_texel = textureSample(rim_texture, rim_sampler, rim_uv).rgb;
    if use_owner_sample_geometry {
        rim_texel = textureSampleGrad(
            rim_texture,
            rim_sampler,
            rim_uv,
            dpdx(default_rim_uv),
            dpdy(default_rim_uv),
        ).rgb;
    }
    let rim_light = material.light_color.rgb * material.pbr_params.w + vec3<f32>(material.lighting.w);
    let rim_mix = mix(vec3<f32>(1.0), rim_light, material.rim_params.x);
    let rim = (rim_base + matcap) * rim_texel * rim_mix;
    var color = (direct + ambient + rim + material.emissive.rgb * emissive_texel) * material.lighting.x;
    if material.outline_color.a >= 0.0 {
        color = material.outline_color.rgb * mix(vec3<f32>(1.0), color, material.outline_color.a);
    }
    return output_color(color, opaque_alpha);
}
