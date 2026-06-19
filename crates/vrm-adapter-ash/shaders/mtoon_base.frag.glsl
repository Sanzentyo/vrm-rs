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
layout(location = 1) in vec4 in_color_0;
layout(location = 2) in vec3 in_normal;
layout(location = 3) in vec4 in_tangent;
layout(location = 4) in vec3 in_world_position;

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

vec4 output_color(vec3 color, float alpha) {
    return vec4(
        linear_to_srgb_channel(color.r),
        linear_to_srgb_channel(color.g),
        linear_to_srgb_channel(color.b),
        alpha
    );
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
        centered.x * c - centered.y * s,
        centered.x * s + centered.y * c
    );
    return rotated + vec2(0.5, 0.5) + scroll;
}

float mtoon_lit_shade_rate(float ndotl, float shift_texel) {
    float shift = mtoon.shading.z + shift_texel * mtoon.lighting.x;
    float toony = clamp(mtoon.shading.w, 0.0, 1.0);
    return linearstep(-1.0 + toony, 1.0 - toony, ndotl + shift);
}

vec3 mtoon_normal(vec2 uv, bool front_facing) {
    float face_sign = front_facing ? 1.0 : -1.0;
    vec3 geometric_normal = normalize(in_normal) * face_sign;
    vec3 tangent = normalize(in_tangent.xyz) * face_sign;
    vec3 bitangent = normalize(cross(geometric_normal, tangent) * in_tangent.w) * face_sign;
    vec3 sampled = texture(normal_texture, uv).xyz;
    vec3 tangent_normal = vec3(
        sampled.x * 2.0 - 1.0,
        1.0 - sampled.y * 2.0,
        sampled.z * 2.0 - 1.0
    );
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
    vec2 animated_uv = mtoon_uv_animation(in_tex_coord_0);
    vec2 base_uv = transform_uv(animated_uv, material_uv.base_transform, material_uv.rotation_a.x);
    vec2 shade_uv = transform_uv(animated_uv, material_uv.shade_transform, material_uv.rotation_a.y);
    vec2 shading_shift_uv = transform_uv(
        animated_uv,
        material_uv.shading_shift_transform,
        material_uv.rotation_a.z
    );
    vec2 normal_uv = transform_uv(animated_uv, material_uv.normal_transform, material_uv.rotation_a.w);
    vec2 rim_uv = transform_uv(animated_uv, material_uv.rim_transform, material_uv.rotation_b.x);
    vec4 main_texel = texture(main_texture, base_uv);
    vec3 diffuse = in_color_0.rgb * main_texel.rgb * mtoon.base_color_factor.rgb;
    float alpha = in_color_0.a * main_texel.a * mtoon.base_color_factor.a;
    uint alpha_mode = mtoon.flags.w;
    if (alpha_mode == 1u && alpha < mtoon.shade_color_factor_cutoff.a) {
        discard;
    }
    float opaque_alpha = alpha_mode == 0u ? 1.0 : alpha;

    vec3 normal = mtoon_normal(normal_uv, gl_FrontFacing);
    vec3 light_dir = normalize(scene.light_dir.xyz);
    float ndotl = clamp(dot(normal, light_dir), -1.0, 1.0);
    vec3 view_dir = normalize(scene.camera_pos.xyz - in_world_position);
    vec3 emissive = mtoon.emissive_color_outline_width.rgb;

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
        vec3 ambient = diffuse * (1.0 - material_extra.pbr_params.x) * scene.mtoon_lighting.w;
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

    float shift_texel = texture(shading_shift_texture, shading_shift_uv).r;
    float shade_rate = mtoon_lit_shade_rate(ndotl, shift_texel);
    vec3 shade = mtoon.shade_color_factor_cutoff.rgb * texture(shade_multiply_texture, shade_uv).rgb;
    vec3 direct = mix(shade, diffuse, shade_rate) * scene.light_color.rgb * scene.light_dir.w * material_extra.pbr_params.w;
    if (material_extra.flags.x > 0.5) {
        direct = min(direct, diffuse);
    }
    vec3 ambient = diffuse * (scene.mtoon_lighting.y + scene.mtoon_lighting.z * mtoon.lighting.z);

    vec2 matcap_uv = matcap_uv_from_view(normal);
    vec3 matcap = texture(matcap_texture, matcap_uv).rgb * mtoon.matcap_factor_debug.rgb;
    vec3 rim_base = mtoon.rim_color_lighting_mix.rgb * pow(
        clamp(1.0 - dot(view_dir, normal) + mtoon.rim_params.y, 0.0, 1.0),
        max(mtoon.rim_params.x, 0.0001)
    );
    vec3 rim_light = scene.light_color.rgb * scene.light_dir.w + vec3(scene.mtoon_lighting.w);
    vec3 rim_mix = mix(vec3(1.0), rim_light, mtoon.rim_color_lighting_mix.a);
    vec3 rim = (rim_base + matcap) * texture(rim_multiply_texture, rim_uv).rgb * rim_mix;
    float outline_mask = texture(outline_width_texture, base_uv).r;
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
