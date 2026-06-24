struct AshSceneUniform {
    view_projection: mat4x4<f32>,
    view: mat4x4<f32>,
    world_from_view: mat4x4<f32>,
    light_dir: vec4<f32>,
    light_color: vec4<f32>,
    camera_pos: vec4<f32>,
    mtoon_lighting: vec4<f32>,
};

struct AshMaterialUvUniform {
    base_transform: vec4<f32>,
    shade_transform: vec4<f32>,
    shading_shift_transform: vec4<f32>,
    normal_transform: vec4<f32>,
    matcap_transform: vec4<f32>,
    rim_transform: vec4<f32>,
    emissive_transform: vec4<f32>,
    occlusion_transform: vec4<f32>,
    uv_animation_mask_transform: vec4<f32>,
    rotation_a: vec4<f32>,
    rotation_b: vec4<f32>,
    uv_animation: vec4<f32>,
};

struct AshMaterialExtraUniform {
    flags: vec4<f32>,
    pbr_params: vec4<f32>,
    flags2: vec4<f32>,
    owner_color: vec4<f32>,
};

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coord_0: vec2<f32>,
    @location(2) tex_coord_0_dx: vec2<f32>,
    @location(3) tex_coord_0_dy: vec2<f32>,
    @location(4) color_0: vec4<f32>,
    @location(5) normal: vec3<f32>,
    @location(6) tangent: vec4<f32>,
    @location(7) normal_scale: f32,
    @location(8) double_sided: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coord_0: vec2<f32>,
    @location(1) tex_coord_0_dx: vec2<f32>,
    @location(2) tex_coord_0_dy: vec2<f32>,
    @location(3) color_0: vec4<f32>,
    @location(4) normal: vec3<f32>,
    @location(5) tangent: vec4<f32>,
    @location(6) world_position: vec3<f32>,
    @location(7) normal_scale: f32,
    @location(8) double_sided: f32,
};

struct FragmentInput {
    @builtin(front_facing) front_facing: bool,
    @location(0) tex_coord_0: vec2<f32>,
    @location(1) tex_coord_0_dx: vec2<f32>,
    @location(2) tex_coord_0_dy: vec2<f32>,
    @location(3) color_0: vec4<f32>,
    @location(4) normal: vec3<f32>,
    @location(5) tangent: vec4<f32>,
    @location(6) world_position: vec3<f32>,
    @location(7) normal_scale: f32,
    @location(8) double_sided: f32,
};

@group(0) @binding(0)
var<uniform> mtoon: MtoonGpuUniform;
@group(0) @binding(1)
var main_texture: texture_2d<f32>;
@group(0) @binding(2)
var main_sampler: sampler;
@group(0) @binding(3)
var shade_multiply_texture: texture_2d<f32>;
@group(0) @binding(4)
var shade_multiply_sampler: sampler;
@group(0) @binding(5)
var shading_shift_texture: texture_2d<f32>;
@group(0) @binding(6)
var shading_shift_sampler: sampler;
@group(0) @binding(7)
var normal_texture: texture_2d<f32>;
@group(0) @binding(8)
var normal_sampler: sampler;
@group(0) @binding(9)
var matcap_texture: texture_2d<f32>;
@group(0) @binding(10)
var matcap_sampler: sampler;
@group(0) @binding(11)
var rim_multiply_texture: texture_2d<f32>;
@group(0) @binding(12)
var rim_multiply_sampler: sampler;
@group(0) @binding(13)
var outline_width_texture: texture_2d<f32>;
@group(0) @binding(14)
var outline_width_sampler: sampler;
@group(0) @binding(15)
var uv_animation_mask_texture: texture_2d<f32>;
@group(0) @binding(16)
var uv_animation_mask_sampler: sampler;
@group(0) @binding(17)
var emissive_texture: texture_2d<f32>;
@group(0) @binding(18)
var emissive_sampler: sampler;
@group(0) @binding(19)
var occlusion_texture: texture_2d<f32>;
@group(0) @binding(20)
var occlusion_sampler: sampler;
@group(0) @binding(30)
var<uniform> scene: AshSceneUniform;
@group(0) @binding(31)
var<uniform> material_uv: AshMaterialUvUniform;
@group(0) @binding(32)
var<uniform> material_extra: AshMaterialExtraUniform;

fn ash_linearstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    return clamp((value - edge0) / max(edge1 - edge0, 0.00001), 0.0, 1.0);
}

fn ash_linear_to_srgb_channel(value: f32) -> f32 {
    let x = clamp(value, 0.0, 1.0);
    if (x <= 0.0031308) {
        return 12.92 * x;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

fn ash_srgb_to_linear_channel(value: f32) -> f32 {
    if (value <= 0.04045) {
        return value / 12.92;
    }
    return pow((value + 0.055) / 1.055, 2.4);
}

fn ash_srgb_to_linear_color(color: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        ash_srgb_to_linear_channel(color.r),
        ash_srgb_to_linear_channel(color.g),
        ash_srgb_to_linear_channel(color.b),
    );
}

fn ash_output_color(color: vec3<f32>, alpha: f32) -> vec4<f32> {
    return vec4<f32>(
        ash_linear_to_srgb_channel(color.r),
        ash_linear_to_srgb_channel(color.g),
        ash_linear_to_srgb_channel(color.b),
        alpha,
    );
}

fn ash_owner_id_output_color(color: vec3<f32>, alpha: f32) -> vec4<f32> {
    let rgb8 = round(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0) / 255.0;
    return vec4<f32>(rgb8, alpha);
}

fn ash_pbr_direct(
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
    let geometry_v = n_dot_l * sqrt(alpha2 + (1.0 - alpha2) * n_dot_v * n_dot_v);
    let geometry_l = n_dot_v * sqrt(alpha2 + (1.0 - alpha2) * n_dot_l * n_dot_l);
    let visibility = 0.5 / max(geometry_v + geometry_l, 0.0001);
    let f0 = mix(vec3<f32>(0.04), diffuse, metallic);
    let f90 = mix(clamp(dot(f0, vec3<f32>(50.0 * 0.33)), 0.0, 1.0), 1.0, metallic);
    let fresnel = f0 + (vec3<f32>(f90) - f0) * pow(1.0 - v_dot_h, 5.0);
    let specular = distribution * visibility * fresnel;
    let diffuse_lobe = diffuse * (1.0 - metallic) / pi;
    return (diffuse_lobe + specular) * pi * n_dot_l;
}

fn ash_transform_uv(uv: vec2<f32>, offset_scale: vec4<f32>, rotation: f32) -> vec2<f32> {
    let scaled = uv * offset_scale.zw;
    let c = cos(rotation);
    let s = sin(rotation);
    return vec2<f32>(
        c * scaled.x - s * scaled.y + offset_scale.x,
        s * scaled.x + c * scaled.y + offset_scale.y,
    );
}

fn ash_transform_uv_gradient(
    gradient: vec2<f32>,
    offset_scale: vec4<f32>,
    rotation: f32,
) -> vec2<f32> {
    let scaled = gradient * offset_scale.zw;
    let c = cos(rotation);
    let s = sin(rotation);
    return vec2<f32>(
        c * scaled.x - s * scaled.y,
        s * scaled.x + c * scaled.y,
    );
}

fn ash_flip_v_gradient(gradient: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(gradient.x, -gradient.y);
}

fn ash_texture_grad_or_implicit(
    source: texture_2d<f32>,
    source_sampler: sampler,
    uv: vec2<f32>,
    dx: vec2<f32>,
    dy: vec2<f32>,
    explicit_grad: bool,
) -> vec4<f32> {
    if (explicit_grad) {
        return textureSampleGrad(source, source_sampler, uv, dx, dy);
    }
    return textureSample(source, source_sampler, uv);
}

fn ash_mtoon_uv_animation(uv: vec2<f32>) -> vec2<f32> {
    let mask_uv = ash_transform_uv(
        uv,
        material_uv.uv_animation_mask_transform,
        material_uv.rotation_b.z,
    );
    let mask = textureSample(uv_animation_mask_texture, uv_animation_mask_sampler, mask_uv).b;
    let scroll = material_uv.uv_animation.xy * mask;
    let rotation = material_uv.uv_animation.z * mask;
    let centered = uv - vec2<f32>(0.5, 0.5);
    let c = cos(rotation);
    let s = sin(rotation);
    let rotated = vec2<f32>(
        centered.x * c + centered.y * s,
        -centered.x * s + centered.y * c,
    );
    return rotated + vec2<f32>(0.5, 0.5) + scroll;
}

fn ash_mtoon_lit_shade_rate(ndotl: f32, shift_texel: f32) -> f32 {
    let shift = mtoon.shading.z + shift_texel * mtoon.lighting.x;
    let toony = clamp(mtoon.shading.w, 0.0, 1.0);
    return ash_linearstep(-1.0 + toony, 1.0 - toony, ndotl + shift);
}

fn ash_mtoon_normal(
    input: FragmentInput,
    uv: vec2<f32>,
    uv_dx: vec2<f32>,
    uv_dy: vec2<f32>,
    explicit_grad: bool,
) -> vec3<f32> {
    var face_sign = -1.0;
    if (input.front_facing || input.double_sided < 0.5) {
        face_sign = 1.0;
    }
    let geometric_normal = normalize(input.normal) * face_sign;
    if (input.normal_scale == 0.0) {
        return geometric_normal;
    }
    let normal_scale = abs(input.normal_scale);
    let tangent = normalize(input.tangent.xyz) * face_sign;
    let bitangent = normalize(cross(geometric_normal, tangent) * input.tangent.w) * face_sign;
    let sampled = ash_texture_grad_or_implicit(
        normal_texture,
        normal_sampler,
        uv,
        uv_dx,
        uv_dy,
        explicit_grad,
    ).xyz;
    let tangent_normal = vec3<f32>(
        (sampled.x * 2.0 - 1.0) * normal_scale,
        (1.0 - sampled.y * 2.0) * normal_scale,
        sampled.z * 2.0 - 1.0,
    );
    if (input.normal_scale < 0.0) {
        let use_view_derivative = material_extra.flags2.y > 0.5;
        let view_position = (scene.view * vec4<f32>(input.world_position, 1.0)).xyz;
        let view_normal = normalize((scene.view * vec4<f32>(geometric_normal, 0.0)).xyz);
        let derivative_position = select(input.world_position, view_position, use_view_derivative);
        let derivative_normal = select(geometric_normal, view_normal, use_view_derivative);
        let q0 = dpdx(derivative_position);
        let q1 = dpdy(derivative_position);
        let st0 = select(dpdx(uv), uv_dx, explicit_grad);
        let st1 = select(dpdy(uv), uv_dy, explicit_grad);
        let q1perp = cross(q1, derivative_normal);
        let q0perp = cross(derivative_normal, q0);
        var derivative_tangent = q1perp * st0.x + q0perp * st1.x;
        var derivative_bitangent = q1perp * st0.y + q0perp * st1.y;
        let det = max(dot(derivative_tangent, derivative_tangent), dot(derivative_bitangent, derivative_bitangent));
        if (det <= 0.0) {
            return geometric_normal;
        }
        let derivative_scale = 1.0 / sqrt(det);
        derivative_tangent = derivative_tangent * derivative_scale * face_sign;
        derivative_bitangent = derivative_bitangent * derivative_scale * face_sign;
        let perturbed = normalize(
            derivative_tangent * tangent_normal.x +
            derivative_bitangent * tangent_normal.y +
            derivative_normal * tangent_normal.z,
        );
        if (use_view_derivative) {
            return normalize((scene.world_from_view * vec4<f32>(perturbed, 0.0)).xyz);
        }
        return perturbed;
    }
    return normalize(
        tangent * tangent_normal.x +
        bitangent * tangent_normal.y +
        geometric_normal * tangent_normal.z,
    );
}

fn ash_matcap_uv_from_view(input: FragmentInput, normal: vec3<f32>) -> vec2<f32> {
    let matcap_view_position = (scene.view * vec4<f32>(input.world_position, 1.0)).xyz;
    let matcap_view_dir = normalize(-matcap_view_position);
    let matcap_normal = normalize((scene.view * vec4<f32>(normal, 0.0)).xyz);
    let matcap_x = normalize(vec3<f32>(matcap_view_dir.z, 0.0, -matcap_view_dir.x));
    let matcap_y = cross(matcap_view_dir, matcap_x);
    let raw_matcap_uv = vec2<f32>(
        0.5 + 0.5 * dot(matcap_x, matcap_normal),
        0.5 - 0.5 * dot(matcap_y, matcap_normal),
    );
    return ash_transform_uv(raw_matcap_uv, material_uv.matcap_transform, material_uv.rotation_b.w);
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.tex_coord_0 = input.tex_coord_0;
    output.tex_coord_0_dx = input.tex_coord_0_dx;
    output.tex_coord_0_dy = input.tex_coord_0_dy;
    output.color_0 = input.color_0;
    output.normal = normalize(input.normal);
    output.tangent = vec4<f32>(normalize(input.tangent.xyz), input.tangent.w);
    output.world_position = input.position;
    output.normal_scale = input.normal_scale;
    output.double_sided = input.double_sided;
    output.clip_position = scene.view_projection * vec4<f32>(input.position, 1.0);
    if (mtoon.flags.z == 1u) {
        output.clip_position.z = output.clip_position.z + 0.000001 * output.clip_position.w;
    }
    return output;
}

@fragment
fn fs_main(input: FragmentInput) -> @location(0) vec4<f32> {
    let explicit_grad = dot(abs(input.tex_coord_0_dx) + abs(input.tex_coord_0_dy), vec2<f32>(1.0)) > 0.0;
    let animated_uv = ash_mtoon_uv_animation(input.tex_coord_0);
    let animated_uv_dx = input.tex_coord_0_dx;
    let animated_uv_dy = input.tex_coord_0_dy;
    let base_uv = ash_transform_uv(animated_uv, material_uv.base_transform, material_uv.rotation_a.x);
    let base_uv_dx = ash_transform_uv_gradient(animated_uv_dx, material_uv.base_transform, material_uv.rotation_a.x);
    let base_uv_dy = ash_transform_uv_gradient(animated_uv_dy, material_uv.base_transform, material_uv.rotation_a.x);
    let shade_uv = ash_transform_uv(animated_uv, material_uv.shade_transform, material_uv.rotation_a.y);
    let shade_uv_dx = ash_transform_uv_gradient(animated_uv_dx, material_uv.shade_transform, material_uv.rotation_a.y);
    let shade_uv_dy = ash_transform_uv_gradient(animated_uv_dy, material_uv.shade_transform, material_uv.rotation_a.y);
    let shading_shift_uv = ash_transform_uv(
        animated_uv,
        material_uv.shading_shift_transform,
        material_uv.rotation_a.z,
    );
    let shading_shift_uv_dx = ash_transform_uv_gradient(
        animated_uv_dx,
        material_uv.shading_shift_transform,
        material_uv.rotation_a.z,
    );
    let shading_shift_uv_dy = ash_transform_uv_gradient(
        animated_uv_dy,
        material_uv.shading_shift_transform,
        material_uv.rotation_a.z,
    );
    let normal_uv = ash_transform_uv(animated_uv, material_uv.normal_transform, material_uv.rotation_a.w);
    let normal_uv_dx = ash_transform_uv_gradient(animated_uv_dx, material_uv.normal_transform, material_uv.rotation_a.w);
    let normal_uv_dy = ash_transform_uv_gradient(animated_uv_dy, material_uv.normal_transform, material_uv.rotation_a.w);
    let rim_uv = ash_transform_uv(animated_uv, material_uv.rim_transform, material_uv.rotation_b.x);
    let rim_uv_dx = ash_transform_uv_gradient(animated_uv_dx, material_uv.rim_transform, material_uv.rotation_b.x);
    let rim_uv_dy = ash_transform_uv_gradient(animated_uv_dy, material_uv.rim_transform, material_uv.rotation_b.x);
    let emissive_uv = ash_transform_uv(animated_uv, material_uv.emissive_transform, material_uv.rotation_b.y);
    let emissive_uv_dx = ash_transform_uv_gradient(animated_uv_dx, material_uv.emissive_transform, material_uv.rotation_b.y);
    let emissive_uv_dy = ash_transform_uv_gradient(animated_uv_dy, material_uv.emissive_transform, material_uv.rotation_b.y);
    let occlusion_uv = ash_transform_uv(animated_uv, material_uv.occlusion_transform, material_uv.uv_animation.w);
    let occlusion_uv_dx = ash_transform_uv_gradient(animated_uv_dx, material_uv.occlusion_transform, material_uv.uv_animation.w);
    let occlusion_uv_dy = ash_transform_uv_gradient(animated_uv_dy, material_uv.occlusion_transform, material_uv.uv_animation.w);

    var base_sample_uv = base_uv;
    var base_sample_uv_dx = base_uv_dx;
    var base_sample_uv_dy = base_uv_dy;
    if (material_extra.flags2.w > 1.5 && material_extra.flags2.w < 2.5) {
        base_sample_uv = vec2<f32>(base_uv.x, 1.0 - base_uv.y);
        base_sample_uv_dx = ash_flip_v_gradient(base_uv_dx);
        base_sample_uv_dy = ash_flip_v_gradient(base_uv_dy);
    }
    let raw_main_texel = ash_texture_grad_or_implicit(
        main_texture,
        main_sampler,
        base_sample_uv,
        base_sample_uv_dx,
        base_sample_uv_dy,
        explicit_grad,
    );
    var main_texel_rgb = raw_main_texel.rgb;
    if (material_extra.flags2.w > 1.0 && material_extra.flags2.w < 1.5) {
        main_texel_rgb = ash_srgb_to_linear_color(raw_main_texel.rgb);
    }
    let diffuse = input.color_0.rgb * main_texel_rgb;
    let alpha = input.color_0.a * raw_main_texel.a;
    let alpha_mode = mtoon.flags.w;
    if (alpha_mode == 1u && alpha < mtoon_alpha_cutoff(mtoon)) {
        discard;
    }
    var opaque_alpha = alpha;
    if (alpha_mode < 2u) {
        opaque_alpha = 1.0;
    }
    if (material_extra.flags2.z > 0.5) {
        return vec4<f32>(vec3<f32>(1.0), opaque_alpha);
    }
    if (material_extra.flags2.w > 4.5 && material_extra.flags2.w < 5.5) {
        return ash_owner_id_output_color(input.color_0.rgb, opaque_alpha);
    }
    if (material_extra.flags2.w > 2.5) {
        if (material_extra.flags2.w > 3.5) {
            return ash_output_color(vec3<f32>(base_sample_uv, 0.0), opaque_alpha);
        }
        return ash_output_color(vec3<f32>(input.tex_coord_0, 0.0), opaque_alpha);
    }
    if (material_extra.flags2.w < -0.5) {
        return ash_output_color(input.color_0.rgb, opaque_alpha);
    }
    if (material_extra.flags2.w > 0.5) {
        return ash_output_color(diffuse, opaque_alpha);
    }

    let normal = ash_mtoon_normal(input, normal_uv, normal_uv_dx, normal_uv_dy, explicit_grad);
    let light_dir = normalize(scene.light_dir.xyz);
    let ndotl = clamp(dot(normal, light_dir), -1.0, 1.0);
    let view_dir = normalize(scene.camera_pos.xyz - input.world_position);
    let emissive = mtoon_emissive_color(mtoon) * ash_texture_grad_or_implicit(
        emissive_texture,
        emissive_sampler,
        emissive_uv,
        emissive_uv_dx,
        emissive_uv_dy,
        explicit_grad,
    ).rgb;

    if (material_extra.flags2.x > 0.5) {
        return ash_output_color(diffuse + emissive, opaque_alpha);
    }
    if (material_extra.flags.y > 0.5) {
        let direct = ash_pbr_direct(
            diffuse,
            normal,
            view_dir,
            light_dir,
            material_extra.pbr_params.x,
            material_extra.pbr_params.y,
        ) * scene.light_color.rgb * scene.light_dir.w;
        let occlusion = (ash_texture_grad_or_implicit(
            occlusion_texture,
            occlusion_sampler,
            occlusion_uv,
            occlusion_uv_dx,
            occlusion_uv_dy,
            explicit_grad,
        ).r - 1.0) * material_extra.pbr_params.z + 1.0;
        let ambient = diffuse * (1.0 - material_extra.pbr_params.x) * scene.mtoon_lighting.w * occlusion;
        var pbr_color = direct + ambient + emissive;
        if (mtoon.flags.z == 1u) {
            pbr_color = mtoon.outline_color_lighting_mix.rgb * mix(
                vec3<f32>(1.0),
                pbr_color,
                mtoon.outline_color_lighting_mix.a,
            );
        }
        return ash_output_color(pbr_color, opaque_alpha);
    }

    let shift_texel = ash_texture_grad_or_implicit(
        shading_shift_texture,
        shading_shift_sampler,
        shading_shift_uv,
        shading_shift_uv_dx,
        shading_shift_uv_dy,
        explicit_grad,
    ).r;
    let shade_rate = ash_mtoon_lit_shade_rate(ndotl, shift_texel);
    let shade = mtoon.shade_color_factor_cutoff.rgb * ash_texture_grad_or_implicit(
        shade_multiply_texture,
        shade_multiply_sampler,
        shade_uv,
        shade_uv_dx,
        shade_uv_dy,
        explicit_grad,
    ).rgb;
    var direct = mix(shade, diffuse, shade_rate) * scene.light_color.rgb * scene.light_dir.w;
    if (material_extra.flags.x > 0.5) {
        direct = min(direct, diffuse);
    }
    let sampled_occlusion = (ash_texture_grad_or_implicit(
        occlusion_texture,
        occlusion_sampler,
        occlusion_uv,
        occlusion_uv_dx,
        occlusion_uv_dy,
        explicit_grad,
    ).r - 1.0) * material_extra.pbr_params.z + 1.0;
    var occlusion = sampled_occlusion;
    if (material_extra.flags.z > 0.5) {
        occlusion = 1.0;
    }
    let ambient = diffuse * (scene.mtoon_lighting.y + scene.mtoon_lighting.z * mtoon.lighting.z) * occlusion;

    let matcap_uv = ash_matcap_uv_from_view(input, normal);
    let matcap = textureSample(matcap_texture, matcap_sampler, matcap_uv).rgb * mtoon.matcap_factor_debug.rgb;
    let rim_base = mtoon.rim_color_lighting_mix.rgb * pow(
        clamp(1.0 - dot(view_dir, normal) + mtoon.rim_params.y, 0.0, 1.0),
        max(mtoon.rim_params.x, 0.0001),
    );
    let rim_light = scene.light_color.rgb * scene.light_dir.w + vec3<f32>(scene.mtoon_lighting.w);
    let rim_mix = mix(vec3<f32>(1.0), rim_light, mtoon.rim_color_lighting_mix.a);
    let rim = (rim_base + matcap) * ash_texture_grad_or_implicit(
        rim_multiply_texture,
        rim_multiply_sampler,
        rim_uv,
        rim_uv_dx,
        rim_uv_dy,
        explicit_grad,
    ).rgb * rim_mix;
    let outline_mask = ash_texture_grad_or_implicit(
        outline_width_texture,
        outline_width_sampler,
        base_uv,
        base_uv_dx,
        base_uv_dy,
        explicit_grad,
    ).r;
    let uv_mask_uv = ash_transform_uv(
        input.tex_coord_0,
        material_uv.uv_animation_mask_transform,
        material_uv.rotation_b.z,
    );
    let uv_mask = textureSample(uv_animation_mask_texture, uv_animation_mask_sampler, uv_mask_uv).b;
    var color = (direct + ambient + rim + emissive) * scene.mtoon_lighting.x;

    if (mtoon.flags.z == 1u) {
        let outline_color = mtoon.outline_color_lighting_mix.rgb * mix(
            vec3<f32>(1.0),
            color,
            mtoon.outline_color_lighting_mix.a,
        );
        color = mix(color, outline_color, outline_mask);
    }
    if (mtoon.flags.x == 1u) {
        color = vec3<f32>(shade_rate);
    } else if (mtoon.flags.x == 2u) {
        color = vec3<f32>(uv_mask);
    }

    return ash_output_color(color, opaque_alpha);
}
