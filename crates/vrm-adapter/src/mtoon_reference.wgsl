struct MtoonGpuUniform {
    base_color_factor: vec4<f32>,
    shade_color_factor_cutoff: vec4<f32>,
    emissive_color_outline_width: vec4<f32>,
    shading: vec4<f32>,
    lighting: vec4<f32>,
    matcap_factor_debug: vec4<f32>,
    rim_color_lighting_mix: vec4<f32>,
    rim_params: vec4<f32>,
    outline_color_lighting_mix: vec4<f32>,
    uv_animation: vec4<f32>,
    flags: vec4<u32>,
};

fn mtoon_base_color_factor(uniform: MtoonGpuUniform) -> vec4<f32> {
    return uniform.base_color_factor;
}

fn mtoon_alpha_cutoff(uniform: MtoonGpuUniform) -> f32 {
    return uniform.shade_color_factor_cutoff.w;
}

fn mtoon_emissive_color(uniform: MtoonGpuUniform) -> vec3<f32> {
    return uniform.emissive_color_outline_width.xyz;
}

fn mtoon_outline_width(uniform: MtoonGpuUniform) -> f32 {
    return uniform.emissive_color_outline_width.w;
}

fn mtoon_uv_animation(uniform: MtoonGpuUniform, uv: vec2<f32>, time_seconds: f32) -> vec2<f32> {
    let scroll = uniform.uv_animation.xy * time_seconds;
    let rotation = uniform.uv_animation.z * time_seconds;
    let centered = uv - vec2<f32>(0.5, 0.5);
    let c = cos(rotation);
    let s = sin(rotation);
    let rotated = vec2<f32>(
        centered.x * c - centered.y * s,
        centered.x * s + centered.y * c,
    );
    return rotated + vec2<f32>(0.5, 0.5) + scroll;
}

fn mtoon_lit_shade_rate(uniform: MtoonGpuUniform, ndotl: f32) -> f32 {
    let shift = uniform.shading.z;
    let toony = clamp(uniform.shading.w, 0.0, 1.0);
    let shifted = clamp(ndotl + shift, 0.0, 1.0);
    return smoothstep(1.0 - toony, 1.0, shifted);
}

fn mtoon_mix_shade(uniform: MtoonGpuUniform, base_color: vec3<f32>, shade_rate: f32) -> vec3<f32> {
    let shade = uniform.shade_color_factor_cutoff.xyz;
    return mix(shade, base_color, shade_rate);
}
