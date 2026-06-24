struct AshSceneUniform {
    view_projection: mat4x4<f32>,
    view: mat4x4<f32>,
    world_from_view: mat4x4<f32>,
    light_dir: vec4<f32>,
    light_color: vec4<f32>,
    camera_pos: vec4<f32>,
    mtoon_lighting: vec4<f32>,
};

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coord_0: vec2<f32>,
    @location(4) color_0: vec4<f32>,
    @location(5) normal: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coord_0: vec2<f32>,
    @location(1) color_0: vec4<f32>,
    @location(2) normal: vec3<f32>,
};

@group(0) @binding(0)
var<uniform> mtoon: MtoonGpuUniform;
@group(0) @binding(1)
var main_texture: texture_2d<f32>;
@group(0) @binding(2)
var main_sampler: sampler;
@group(0) @binding(9)
var<uniform> scene: AshSceneUniform;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = scene.view_projection * vec4<f32>(input.position, 1.0);
    output.tex_coord_0 = mtoon_uv_animation(mtoon, input.tex_coord_0, 0.0);
    output.color_0 = input.color_0 * mtoon_base_color_factor(mtoon);
    output.normal = normalize(input.normal);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(main_texture, main_sampler, input.tex_coord_0);
    let ndotl = clamp(dot(normalize(input.normal), normalize(-scene.light_dir.xyz)), -1.0, 1.0);
    let shade_rate = mtoon_lit_shade_rate(mtoon, ndotl, 0.0);
    let base = input.color_0.rgb * texel.rgb;
    let lit = mtoon_mix_shade(mtoon, base, shade_rate) * scene.light_color.rgb * scene.light_dir.w;
    let emissive = mtoon_emissive_color(mtoon);
    return vec4<f32>(lit + emissive, input.color_0.a * texel.a);
}
