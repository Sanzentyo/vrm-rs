#version 450

layout(set = 0, binding = 0, std140) uniform MtoonGpuUniform {
    vec4 base_color_factor;
    vec4 shade_color_factor_cutoff;
    vec4 emissive_color_outline_width;
    vec4 shading;
    vec4 lighting;
    vec4 matcap_factor_debug;
    vec4 rim_color_lighting_mix;
    vec4 rim_params;
    vec4 outline_color_lighting_mix;
    vec4 uv_animation;
    uvec4 flags;
} mtoon;

layout(set = 0, binding = 1) uniform sampler2D main_texture;
layout(set = 0, binding = 2) uniform sampler2D shade_multiply_texture;
layout(set = 0, binding = 3) uniform sampler2D shading_shift_texture;
layout(set = 0, binding = 4) uniform sampler2D normal_texture;
layout(set = 0, binding = 5) uniform sampler2D matcap_texture;
layout(set = 0, binding = 6) uniform sampler2D rim_multiply_texture;
layout(set = 0, binding = 7) uniform sampler2D outline_width_texture;
layout(set = 0, binding = 8) uniform sampler2D uv_animation_mask_texture;
layout(set = 0, binding = 12) uniform sampler2D emissive_texture;
layout(set = 0, binding = 13) uniform sampler2D occlusion_texture;

layout(set = 0, binding = 9, std140) uniform AshSceneUniform {
    mat4 view_projection;
    mat4 view;
    mat4 world_from_view;
    vec4 light_dir;
    vec4 light_color;
    vec4 camera_pos;
    vec4 mtoon_lighting;
} scene;

layout(set = 0, binding = 10, std140) uniform AshMaterialUvUniform {
    vec4 base_transform;
    vec4 shade_transform;
    vec4 shading_shift_transform;
    vec4 normal_transform;
    vec4 matcap_transform;
    vec4 rim_transform;
    vec4 emissive_transform;
    vec4 occlusion_transform;
    vec4 uv_animation_mask_transform;
    vec4 rotation_a;
    vec4 rotation_b;
    vec4 uv_animation;
} material_uv;

layout(set = 0, binding = 11, std140) uniform AshMaterialExtraUniform {
    vec4 flags;
    vec4 pbr_params;
    vec4 flags2;
    vec4 owner_color;
} material_extra;

layout(location = 0) in vec2 in_tex_coord_0;
layout(location = 1) in vec2 in_tex_coord_0_dx;
layout(location = 2) in vec2 in_tex_coord_0_dy;
layout(location = 3) in vec4 in_color_0;
layout(location = 4) in vec3 in_normal;
layout(location = 5) in vec4 in_tangent;
layout(location = 6) in vec3 in_world_position;
layout(location = 7) in float in_normal_scale;
layout(location = 8) in float in_double_sided;

layout(location = 0) out vec4 out_color;

float linearstep(float edge0, float edge1, float value) {
    return clamp((value - edge0) / max(edge1 - edge0, 0.00001), 0.0, 1.0);
}

float linear_to_srgb_channel(float value) {
    float x = clamp(value, 0.0, 1.0);
    if (x <= 0.0031308) {
        return 12.92 * x;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

float srgb_to_linear_channel(float value) {
    if (value <= 0.04045) {
        return value / 12.92;
    }
    return pow((value + 0.055) / 1.055, 2.4);
}

vec3 srgb_to_linear_color(vec3 color) {
    return vec3(
        srgb_to_linear_channel(color.r),
        srgb_to_linear_channel(color.g),
        srgb_to_linear_channel(color.b)
    );
}

vec4 output_color(vec3 color, float alpha) {
    return vec4(
        linear_to_srgb_channel(color.r),
        linear_to_srgb_channel(color.g),
        linear_to_srgb_channel(color.b),
        alpha
    );
}

vec4 owner_id_output_color(vec3 color, float alpha) {
    vec3 rgb8 = round(clamp(color, vec3(0.0), vec3(1.0)) * 255.0) / 255.0;
    return vec4(rgb8, alpha);
}

vec3 pbr_direct(
    vec3 diffuse,
    vec3 normal,
    vec3 view_dir,
    vec3 light_dir,
    float metallic,
    float roughness
) {
    const float pi = 3.141592653589793;
    float n_dot_l = max(dot(normal, light_dir), 0.0);
    float n_dot_v = max(dot(normal, view_dir), 0.0001);
    vec3 half_dir = normalize(light_dir + view_dir);
    float n_dot_h = max(dot(normal, half_dir), 0.0001);
    float v_dot_h = max(dot(view_dir, half_dir), 0.0);
    float rough = clamp(roughness, 0.04, 1.0);
    float alpha = rough * rough;
    float alpha2 = alpha * alpha;
    float denom = n_dot_h * n_dot_h * (alpha2 - 1.0) + 1.0;
    float distribution = alpha2 / max(pi * denom * denom, 0.0001);
    float k = (rough + 1.0) * (rough + 1.0) / 8.0;
    float geometry_l = n_dot_l / (n_dot_l * (1.0 - k) + k);
    float geometry_v = n_dot_v / (n_dot_v * (1.0 - k) + k);
    float geometry = geometry_l * geometry_v;
    vec3 f0 = mix(vec3(0.04), diffuse, metallic);
    vec3 fresnel = f0 + (vec3(1.0) - f0) * pow(1.0 - v_dot_h, 5.0);
    vec3 specular = distribution * geometry * fresnel / max(4.0 * n_dot_l * n_dot_v, 0.0001);
    vec3 diffuse_lobe = diffuse * (1.0 - metallic) / pi;
    return (diffuse_lobe + specular) * pi * n_dot_l;
}

vec2 transform_uv(vec2 uv, vec4 offset_scale, float rotation) {
    vec2 scaled = uv * offset_scale.zw;
    float c = cos(rotation);
    float s = sin(rotation);
    return vec2(
        c * scaled.x - s * scaled.y + offset_scale.x,
        s * scaled.x + c * scaled.y + offset_scale.y
    );
}

vec2 transform_uv_gradient(vec2 gradient, vec4 offset_scale, float rotation) {
    vec2 scaled = gradient * offset_scale.zw;
    float c = cos(rotation);
    float s = sin(rotation);
    return vec2(
        c * scaled.x - s * scaled.y,
        s * scaled.x + c * scaled.y
    );
}

vec2 flip_v_gradient(vec2 gradient) {
    return vec2(gradient.x, -gradient.y);
}

vec4 texture_grad_or_implicit(sampler2D source, vec2 uv, vec2 dx, vec2 dy, bool explicit_grad) {
    return explicit_grad ? textureGrad(source, uv, dx, dy) : texture(source, uv);
}

vec2 mtoon_uv_animation(vec2 uv) {
    vec2 mask_uv = transform_uv(
        uv,
        material_uv.uv_animation_mask_transform,
        material_uv.rotation_b.z
    );
    float mask = texture(uv_animation_mask_texture, mask_uv).b;
    vec2 scroll = material_uv.uv_animation.xy * mask;
    float rotation = material_uv.uv_animation.z * mask;
    vec2 centered = uv - vec2(0.5, 0.5);
    float c = cos(rotation);
    float s = sin(rotation);
    vec2 rotated = vec2(
        centered.x * c + centered.y * s,
        -centered.x * s + centered.y * c
    );
    return rotated + vec2(0.5, 0.5) + scroll;
}

float mtoon_lit_shade_rate(float ndotl, float shift_texel) {
    float shift = mtoon.shading.z + shift_texel * mtoon.lighting.x;
    float toony = clamp(mtoon.shading.w, 0.0, 1.0);
    return linearstep(-1.0 + toony, 1.0 - toony, ndotl + shift);
}

vec3 mtoon_normal(vec2 uv, vec2 uv_dx, vec2 uv_dy, bool explicit_grad, bool front_facing) {
    float face_sign = (front_facing || in_double_sided < 0.5) ? 1.0 : -1.0;
    vec3 geometric_normal = normalize(in_normal) * face_sign;
    if (in_normal_scale == 0.0) {
        return geometric_normal;
    }
    float normal_scale = abs(in_normal_scale);
    vec3 tangent = normalize(in_tangent.xyz) * face_sign;
    vec3 bitangent = normalize(cross(geometric_normal, tangent) * in_tangent.w) * face_sign;
    vec3 sampled = texture_grad_or_implicit(normal_texture, uv, uv_dx, uv_dy, explicit_grad).xyz;
    vec3 tangent_normal = vec3(
        (sampled.x * 2.0 - 1.0) * normal_scale,
        (1.0 - sampled.y * 2.0) * normal_scale,
        sampled.z * 2.0 - 1.0
    );
    if (in_normal_scale < 0.0) {
        bool use_view_derivative = material_extra.flags2.y > 0.5;
        vec3 view_position = (scene.view * vec4(in_world_position, 1.0)).xyz;
        vec3 view_normal = normalize((scene.view * vec4(geometric_normal, 0.0)).xyz);
        vec3 derivative_position = use_view_derivative ? view_position : in_world_position;
        vec3 derivative_normal = use_view_derivative ? view_normal : geometric_normal;
        vec3 q0 = dFdx(derivative_position);
        vec3 q1 = dFdy(derivative_position);
        vec2 st0 = explicit_grad ? uv_dx : dFdx(uv);
        vec2 st1 = explicit_grad ? uv_dy : dFdy(uv);
        vec3 q1perp = cross(q1, derivative_normal);
        vec3 q0perp = cross(derivative_normal, q0);
        vec3 derivative_tangent = q1perp * st0.x + q0perp * st1.x;
        vec3 derivative_bitangent = q1perp * st0.y + q0perp * st1.y;
        float det = max(dot(derivative_tangent, derivative_tangent), dot(derivative_bitangent, derivative_bitangent));
        if (det <= 0.0) {
            return geometric_normal;
        }
        float derivative_scale = 1.0 / sqrt(det);
        derivative_tangent *= derivative_scale * face_sign;
        derivative_bitangent *= derivative_scale * face_sign;
        vec3 perturbed = normalize(
            derivative_tangent * tangent_normal.x +
            derivative_bitangent * tangent_normal.y +
            derivative_normal * tangent_normal.z
        );
        return use_view_derivative
            ? normalize((scene.world_from_view * vec4(perturbed, 0.0)).xyz)
            : perturbed;
    }
    return normalize(
        tangent * tangent_normal.x +
        bitangent * tangent_normal.y +
        geometric_normal * tangent_normal.z
    );
}

vec2 matcap_uv_from_view(vec3 normal) {
    vec3 matcap_view_position = (scene.view * vec4(in_world_position, 1.0)).xyz;
    vec3 matcap_view_dir = normalize(-matcap_view_position);
    vec3 matcap_normal = normalize((scene.view * vec4(normal, 0.0)).xyz);
    vec3 matcap_x = normalize(vec3(matcap_view_dir.z, 0.0, -matcap_view_dir.x));
    vec3 matcap_y = cross(matcap_view_dir, matcap_x);
    vec2 raw_matcap_uv = vec2(
        0.5 + 0.5 * dot(matcap_x, matcap_normal),
        0.5 - 0.5 * dot(matcap_y, matcap_normal)
    );
    return transform_uv(raw_matcap_uv, material_uv.matcap_transform, material_uv.rotation_b.w);
}

void main() {
    bool explicit_grad = dot(abs(in_tex_coord_0_dx) + abs(in_tex_coord_0_dy), vec2(1.0)) > 0.0;
    vec2 animated_uv = mtoon_uv_animation(in_tex_coord_0);
    vec2 animated_uv_dx = in_tex_coord_0_dx;
    vec2 animated_uv_dy = in_tex_coord_0_dy;
    vec2 base_uv = transform_uv(animated_uv, material_uv.base_transform, material_uv.rotation_a.x);
    vec2 base_uv_dx = transform_uv_gradient(animated_uv_dx, material_uv.base_transform, material_uv.rotation_a.x);
    vec2 base_uv_dy = transform_uv_gradient(animated_uv_dy, material_uv.base_transform, material_uv.rotation_a.x);
    vec2 shade_uv = transform_uv(animated_uv, material_uv.shade_transform, material_uv.rotation_a.y);
    vec2 shade_uv_dx = transform_uv_gradient(animated_uv_dx, material_uv.shade_transform, material_uv.rotation_a.y);
    vec2 shade_uv_dy = transform_uv_gradient(animated_uv_dy, material_uv.shade_transform, material_uv.rotation_a.y);
    vec2 shading_shift_uv = transform_uv(
        animated_uv,
        material_uv.shading_shift_transform,
        material_uv.rotation_a.z
    );
    vec2 shading_shift_uv_dx = transform_uv_gradient(
        animated_uv_dx,
        material_uv.shading_shift_transform,
        material_uv.rotation_a.z
    );
    vec2 shading_shift_uv_dy = transform_uv_gradient(
        animated_uv_dy,
        material_uv.shading_shift_transform,
        material_uv.rotation_a.z
    );
    vec2 normal_uv = transform_uv(animated_uv, material_uv.normal_transform, material_uv.rotation_a.w);
    vec2 normal_uv_dx = transform_uv_gradient(animated_uv_dx, material_uv.normal_transform, material_uv.rotation_a.w);
    vec2 normal_uv_dy = transform_uv_gradient(animated_uv_dy, material_uv.normal_transform, material_uv.rotation_a.w);
    vec2 rim_uv = transform_uv(animated_uv, material_uv.rim_transform, material_uv.rotation_b.x);
    vec2 rim_uv_dx = transform_uv_gradient(animated_uv_dx, material_uv.rim_transform, material_uv.rotation_b.x);
    vec2 rim_uv_dy = transform_uv_gradient(animated_uv_dy, material_uv.rim_transform, material_uv.rotation_b.x);
    vec2 emissive_uv = transform_uv(animated_uv, material_uv.emissive_transform, material_uv.rotation_b.y);
    vec2 emissive_uv_dx = transform_uv_gradient(animated_uv_dx, material_uv.emissive_transform, material_uv.rotation_b.y);
    vec2 emissive_uv_dy = transform_uv_gradient(animated_uv_dy, material_uv.emissive_transform, material_uv.rotation_b.y);
    vec2 occlusion_uv = transform_uv(animated_uv, material_uv.occlusion_transform, material_uv.uv_animation.w);
    vec2 occlusion_uv_dx = transform_uv_gradient(animated_uv_dx, material_uv.occlusion_transform, material_uv.uv_animation.w);
    vec2 occlusion_uv_dy = transform_uv_gradient(animated_uv_dy, material_uv.occlusion_transform, material_uv.uv_animation.w);
    vec2 base_sample_uv = base_uv;
    vec2 base_sample_uv_dx = base_uv_dx;
    vec2 base_sample_uv_dy = base_uv_dy;
    if (material_extra.flags2.w > 1.5 && material_extra.flags2.w < 2.5) {
        base_sample_uv = vec2(base_uv.x, 1.0 - base_uv.y);
        base_sample_uv_dx = flip_v_gradient(base_uv_dx);
        base_sample_uv_dy = flip_v_gradient(base_uv_dy);
    }
    vec4 raw_main_texel = texture_grad_or_implicit(
        main_texture,
        base_sample_uv,
        base_sample_uv_dx,
        base_sample_uv_dy,
        explicit_grad
    );
    vec3 main_texel_rgb = raw_main_texel.rgb;
    if (material_extra.flags2.w > 1.0 && material_extra.flags2.w < 1.5) {
        main_texel_rgb = srgb_to_linear_color(raw_main_texel.rgb);
    }
    vec3 diffuse = in_color_0.rgb * main_texel_rgb;
    float alpha = in_color_0.a * raw_main_texel.a;
    uint alpha_mode = mtoon.flags.w;
    if (alpha_mode == 1u && alpha < mtoon.shade_color_factor_cutoff.a) {
        discard;
    }
    float opaque_alpha = alpha_mode < 2u ? 1.0 : alpha;
    if (material_extra.flags2.z > 0.5) {
        out_color = vec4(vec3(1.0), opaque_alpha);
        return;
    }
    if (material_extra.flags2.w > 4.5 && material_extra.flags2.w < 5.5) {
        out_color = owner_id_output_color(in_color_0.rgb, opaque_alpha);
        return;
    }
    if (material_extra.flags2.w > 2.5) {
        if (material_extra.flags2.w > 3.5) {
            out_color = output_color(vec3(base_sample_uv, 0.0), opaque_alpha);
            return;
        }
        out_color = output_color(vec3(in_tex_coord_0, 0.0), opaque_alpha);
        return;
    }
    if (material_extra.flags2.w < -0.5) {
        out_color = output_color(in_color_0.rgb, opaque_alpha);
        return;
    }
    if (material_extra.flags2.w > 0.5) {
        out_color = output_color(diffuse, opaque_alpha);
        return;
    }

    vec3 normal = mtoon_normal(normal_uv, normal_uv_dx, normal_uv_dy, explicit_grad, gl_FrontFacing);
    vec3 light_dir = normalize(scene.light_dir.xyz);
    float ndotl = clamp(dot(normal, light_dir), -1.0, 1.0);
    vec3 view_dir = normalize(scene.camera_pos.xyz - in_world_position);
    vec3 emissive = mtoon.emissive_color_outline_width.rgb * texture_grad_or_implicit(
        emissive_texture,
        emissive_uv,
        emissive_uv_dx,
        emissive_uv_dy,
        explicit_grad
    ).rgb;

    if (material_extra.flags2.x > 0.5) {
        out_color = output_color(diffuse + emissive, opaque_alpha);
        return;
    }
    if (material_extra.flags.y > 0.5) {
        vec3 direct = pbr_direct(
            diffuse,
            normal,
            view_dir,
            light_dir,
            material_extra.pbr_params.x,
            material_extra.pbr_params.y
        ) * scene.light_color.rgb * scene.light_dir.w;
        float occlusion = (texture_grad_or_implicit(
            occlusion_texture,
            occlusion_uv,
            occlusion_uv_dx,
            occlusion_uv_dy,
            explicit_grad
        ).r - 1.0) * material_extra.pbr_params.z + 1.0;
        vec3 ambient = diffuse * (1.0 - material_extra.pbr_params.x) * scene.mtoon_lighting.w * occlusion;
        vec3 pbr_color = direct + ambient + emissive;
        if (mtoon.flags.z == 1u) {
            pbr_color = mtoon.outline_color_lighting_mix.rgb * mix(
                vec3(1.0),
                pbr_color,
                mtoon.outline_color_lighting_mix.a
            );
        }
        out_color = output_color(pbr_color, opaque_alpha);
        return;
    }

    float shift_texel = texture_grad_or_implicit(
        shading_shift_texture,
        shading_shift_uv,
        shading_shift_uv_dx,
        shading_shift_uv_dy,
        explicit_grad
    ).r;
    float shade_rate = mtoon_lit_shade_rate(ndotl, shift_texel);
    vec3 shade = mtoon.shade_color_factor_cutoff.rgb * texture_grad_or_implicit(
        shade_multiply_texture,
        shade_uv,
        shade_uv_dx,
        shade_uv_dy,
        explicit_grad
    ).rgb;
    vec3 direct = mix(shade, diffuse, shade_rate) * scene.light_color.rgb * scene.light_dir.w;
    if (material_extra.flags.x > 0.5) {
        direct = min(direct, diffuse);
    }
    float sampled_occlusion = (texture_grad_or_implicit(
        occlusion_texture,
        occlusion_uv,
        occlusion_uv_dx,
        occlusion_uv_dy,
        explicit_grad
    ).r - 1.0) * material_extra.pbr_params.z + 1.0;
    float occlusion = material_extra.flags.z > 0.5 ? 1.0 : sampled_occlusion;
    vec3 ambient = diffuse * (scene.mtoon_lighting.y + scene.mtoon_lighting.z * mtoon.lighting.z) * occlusion;

    vec2 matcap_uv = matcap_uv_from_view(normal);
    vec3 matcap = texture(matcap_texture, matcap_uv).rgb * mtoon.matcap_factor_debug.rgb;
    vec3 rim_base = mtoon.rim_color_lighting_mix.rgb * pow(
        clamp(1.0 - dot(view_dir, normal) + mtoon.rim_params.y, 0.0, 1.0),
        max(mtoon.rim_params.x, 0.0001)
    );
    vec3 rim_light = scene.light_color.rgb * scene.light_dir.w + vec3(scene.mtoon_lighting.w);
    vec3 rim_mix = mix(vec3(1.0), rim_light, mtoon.rim_color_lighting_mix.a);
    vec3 rim = (rim_base + matcap) * texture_grad_or_implicit(
        rim_multiply_texture,
        rim_uv,
        rim_uv_dx,
        rim_uv_dy,
        explicit_grad
    ).rgb * rim_mix;
    float outline_mask = texture_grad_or_implicit(
        outline_width_texture,
        base_uv,
        base_uv_dx,
        base_uv_dy,
        explicit_grad
    ).r;
    vec2 uv_mask_uv = transform_uv(
        in_tex_coord_0,
        material_uv.uv_animation_mask_transform,
        material_uv.rotation_b.z
    );
    float uv_mask = texture(uv_animation_mask_texture, uv_mask_uv).b;
    vec3 color = (direct + ambient + rim + emissive) * scene.mtoon_lighting.x;

    if (mtoon.flags.z == 1u) {
        vec3 outline_color = mtoon.outline_color_lighting_mix.rgb * mix(
            vec3(1.0),
            color,
            mtoon.outline_color_lighting_mix.a
        );
        color = mix(color, outline_color, outline_mask);
    }
    if (mtoon.flags.x == 1u) {
        color = vec3(shade_rate);
    } else if (mtoon.flags.x == 2u) {
        color = vec3(uv_mask);
    }

    out_color = output_color(color, opaque_alpha);
}
