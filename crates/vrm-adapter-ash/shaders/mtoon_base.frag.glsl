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

layout(location = 0) in vec2 in_tex_coord_0;
layout(location = 1) in vec4 in_color_0;
layout(location = 2) in vec3 in_normal;
layout(location = 3) in vec4 in_tangent;
layout(location = 4) in vec3 in_world_position;

layout(location = 0) out vec4 out_color;

float linearstep(float edge0, float edge1, float value) {
    return clamp((value - edge0) / max(edge1 - edge0, 0.00001), 0.0, 1.0);
}

vec2 mtoon_uv_animation(vec2 uv) {
    float mask = texture(uv_animation_mask_texture, uv).b;
    vec2 scroll = mtoon.uv_animation.xy * mask;
    float rotation = mtoon.uv_animation.z * mask;
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
    float shifted = clamp(ndotl + shift, 0.0, 1.0);
    return linearstep(1.0 - toony, 1.0, shifted);
}

vec3 mtoon_normal(vec2 uv) {
    vec3 geometric_normal = normalize(in_normal);
    vec3 tangent = normalize(in_tangent.xyz);
    vec3 bitangent = normalize(cross(geometric_normal, tangent) * in_tangent.w);
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

void main() {
    vec2 uv = mtoon_uv_animation(in_tex_coord_0);
    vec4 main_texel = texture(main_texture, uv);
    vec4 base = in_color_0 * main_texel * mtoon.base_color_factor;
    float alpha = base.a;
    uint alpha_mode = mtoon.flags.w;
    if (alpha_mode == 1u && alpha < mtoon.shade_color_factor_cutoff.a) {
        discard;
    }

    vec3 normal = mtoon_normal(uv);
    vec3 light_dir = normalize(scene.light_dir.xyz);
    float ndotl = clamp(dot(normal, light_dir), 0.0, 1.0);
    float shift_texel = texture(shading_shift_texture, uv).r;
    float shade_rate = mtoon_lit_shade_rate(ndotl, shift_texel);
    vec3 shade = mtoon.shade_color_factor_cutoff.rgb * texture(shade_multiply_texture, uv).rgb;
    vec3 direct = mix(shade, base.rgb, shade_rate) * scene.light_color.rgb * scene.light_dir.w;
    vec3 ambient = base.rgb * (scene.mtoon_lighting.y + scene.mtoon_lighting.z * mtoon.lighting.z);

    vec3 matcap = texture(matcap_texture, uv).rgb * mtoon.matcap_factor_debug.rgb;
    vec3 view_dir = normalize(scene.camera_pos.xyz - in_world_position);
    vec3 rim_base = mtoon.rim_color_lighting_mix.rgb * pow(
        clamp(1.0 - dot(view_dir, normal) + mtoon.rim_params.y, 0.0, 1.0),
        max(mtoon.rim_params.x, 0.0001)
    );
    vec3 rim_light = scene.light_color.rgb * scene.light_dir.w + vec3(scene.mtoon_lighting.w);
    vec3 rim_mix = mix(vec3(1.0), rim_light, mtoon.rim_color_lighting_mix.a);
    vec3 rim = texture(rim_multiply_texture, uv).rgb * rim_base * rim_mix;
    float outline_mask = texture(outline_width_texture, uv).r;
    float uv_mask = texture(uv_animation_mask_texture, in_tex_coord_0).b;
    vec3 emissive = mtoon.emissive_color_outline_width.rgb;
    vec3 color = (direct + ambient + matcap + rim + emissive) * scene.mtoon_lighting.x;

    if (mtoon.flags.z == 1u) {
        color = mix(color, mtoon.outline_color_lighting_mix.rgb, outline_mask);
    }
    if (mtoon.flags.x == 1u) {
        color = vec3(shade_rate);
    } else if (mtoon.flags.x == 2u) {
        color = vec3(uv_mask);
    }

    out_color = vec4(clamp(color, vec3(0.0), vec3(1.0)), alpha);
}
