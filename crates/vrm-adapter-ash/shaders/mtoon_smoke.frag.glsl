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

layout(location = 0) in vec2 in_tex_coord_0;
layout(location = 1) in vec4 in_color_0;

layout(location = 0) out vec4 out_color;

void main() {
    vec4 main_texel = texture(main_texture, in_tex_coord_0);
    vec3 emissive = mtoon.emissive_color_outline_width.rgb;
    vec4 color = in_color_0 * main_texel * mtoon.base_color_factor;
    color.rgb += emissive;

    float cutoff = mtoon.shade_color_factor_cutoff.a;
    uint alpha_mode = mtoon.flags.w;
    if (alpha_mode == 1u && color.a < cutoff) {
        discard;
    }

    out_color = clamp(color, vec4(0.0), vec4(1.0));
}
